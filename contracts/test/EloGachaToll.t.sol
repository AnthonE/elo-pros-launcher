// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, Vm} from "forge-std/Test.sol";
import {EloGacha} from "../src/EloGacha.sol";
import {MockArbSys} from "./MockArbSys.sol";

/// The toll rail (decided 2026-07-27, `SENTENCES.md`: the toll is `SCRY`).
///
/// `EloGacha.t.sol` covers the ETH rail and every one of its 36 tests still
/// passes untouched — which is the first thing this suite is for. The toll is an
/// **armable knob that is off by default**, so "arming nothing changes nothing"
/// is not a nice property, it is the deploy plan: the pool ships ETH-only and
/// the coin leg is switched on later, against a live pool, without a migration.
///
/// What this file asserts that nothing else can:
///
///   1. the two rails are genuinely independent — a position that earned on both
///      collects both, and neither one's failure can reach the other;
///   2. every unit of a toll becomes a NAMED liability, the same conservation
///      claim `_assertSolvent` makes in ETH;
///   3. **an exit never depends on the toll coin.** This is the one that would
///      not show up in ordinary use and would be catastrophic once: a paused,
///      blacklisting or self-destructed toll coin must not be able to hold a
///      depositor's backing or their NFT hostage. `withdraw` is tested against a
///      token that reverts on every transfer.
contract GachaTollNft {
    mapping(uint256 => address) public ownerOf;
    mapping(address => mapping(address => bool)) public isApprovedForAll;

    function mint(address to, uint256 id) external {
        ownerOf[id] = to;
    }

    function setApprovalForAll(address who, bool v) external {
        isApprovedForAll[msg.sender][who] = v;
    }

    function transferFrom(address from, address to, uint256 id) public {
        require(ownerOf[id] == from, "wrong from");
        require(msg.sender == from || isApprovedForAll[from][msg.sender], "not authorized");
        ownerOf[id] = to;
    }
}

/// @dev The toll coin, with the two failure modes a posted third-party ERC-20
///      can actually have. `paused` is a token that stops moving after the pool
///      already holds some of it; `feeBps` is the fee-on-transfer family, which
///      delivers less than it was asked to and would silently make every
///      downstream split pay out of somebody else's credits. Written out rather
///      than extended from `MockToken`, because making a fixture five other
///      suites depend on `virtual` for one suite's benefit is the wrong trade.
contract TollCoin {
    string public constant name = "Scry";
    string public constant symbol = "SCRY";
    uint8 public constant decimals = 18;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    bool public paused;
    uint256 public feeBps;

    event Transfer(address indexed from, address indexed to, uint256 value);

    function setPaused(bool v) external {
        paused = v;
    }

    function setFeeBps(uint256 v) external {
        feeBps = v;
    }

    function mint(address to, uint256 value) external {
        totalSupply += value;
        balanceOf[to] += value;
        emit Transfer(address(0), to, value);
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        return true;
    }

    function transfer(address to, uint256 value) external returns (bool) {
        _move(msg.sender, to, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= value, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - value;
        _move(from, to, value);
        return true;
    }

    function _move(address from, address to, uint256 value) internal {
        require(!paused, "token paused");
        require(balanceOf[from] >= value, "balance");
        uint256 skimmed = value * feeBps / 10_000;
        unchecked {
            balanceOf[from] -= value;
            balanceOf[to] += value - skimmed;
            balanceOf[address(0xFEE)] += skimmed;
        }
        emit Transfer(from, to, value - skimmed);
    }
}

contract EloGachaTollTest is Test {
    EloGacha internal g;
    GachaTollNft internal nft;
    TollCoin internal coin;
    MockArbSys internal arb;

    address internal alice = address(0xA11CE); // depositor
    address internal bob = address(0xB0B); // depositor
    address internal cody = address(0xC0DE); // buyer
    address internal sink = address(0x5EEE);
    address internal constant ARBSYS = 0x0000000000000000000000000000000000000064;
    address internal constant DEAD = 0x000000000000000000000000000000000000dEaD;

    uint256 internal constant BACKING = 0.01 ether;
    uint256 internal constant DELAY = 100;
    /// 1,000 coin per ETH of quoted price, so the 0.011 ETH fee tolls at exactly
    /// 11 coin and every share below divides without a carry. Chosen so a wrong
    /// split shows up as a wrong number, not as a rounding argument.
    uint256 internal constant RATE = 1_000e18;
    uint256 internal constant TOLL = 11e18;

    function setUp() public {
        vm.etch(ARBSYS, address(new MockArbSys()).code);
        arb = MockArbSys(ARBSYS);
        arb.setL2Block(20_000_000);
        arb.setWindow(256);

        g = new EloGacha(DELAY, sink);
        nft = new GachaTollNft();
        coin = new TollCoin();

        vm.deal(alice, 100 ether);
        vm.deal(bob, 100 ether);
        vm.deal(cody, 100 ether);
        g.openPool(address(nft), BACKING, 1_000, 8_500);

        nft.mint(alice, 1);
        nft.mint(bob, 2);
        nft.mint(alice, 3);
        vm.prank(alice);
        nft.setApprovalForAll(address(g), true);
        vm.prank(bob);
        nft.setApprovalForAll(address(g), true);

        coin.mint(cody, 10_000e18);
        vm.prank(cody);
        coin.approve(address(g), type(uint256).max);
    }

    // ---------------------------------------------------------------- helpers

    function _arm() internal {
        g.setToll(address(coin), RATE);
        g.setTollBurnBps(500);
    }

    function _twoPositions() internal returns (uint256 a, uint256 b) {
        vm.prank(alice);
        a = g.deposit{value: BACKING}(address(nft), 1);
        vm.prank(bob);
        b = g.deposit{value: BACKING}(address(nft), 2);
    }

    function _requestToll() internal returns (uint256 drawId) {
        vm.prank(cody);
        drawId = g.request(address(nft), bytes32("s"), 0, 0, 0);
    }

    function _settle(uint256 drawId) internal returns (uint256 positionId) {
        (,,,,, uint64 revealBlock,,,) = g.draws(drawId);
        arb.setL2Block(uint256(revealBlock) + 1);
        arb.setHash(uint256(revealBlock), keccak256("bh"));
        positionId = g.settle(drawId);
    }

    /// The toll rail's conservation claim, and the twin of `_assertSolvent`.
    /// Every unit the contract holds is either a named liability or a donation —
    /// never a shortfall.
    function _assertTollSolvent() internal view {
        assertGe(coin.balanceOf(address(g)), g.accountedToll(), "toll insolvent: balance below liabilities");
    }

    function _tollLiabilities() internal view returns (uint256) {
        (uint256 esc, uint256 owedOut, uint256 house, uint256 pots, uint256 credits, uint256 burn) = g.tollLedger();
        return esc + owedOut + house + pots + credits + burn;
    }

    // ------------------------------------------------------------------ tests

    /// Off by default, and that is the deploy plan rather than a default. Until
    /// the operator posts a coin AND a rate, this contract is exactly what
    /// `EloGacha.t.sol` tests: native ETH, ten `msg.value` reads, no allowance.
    function test_theTollIsOffUntilBothKnobsArePosted() public {
        _twoPositions();
        assertEq(g.tollCoin(), address(0), "no coin posted");
        assertEq(g.toll(address(nft)), 0, "no toll");
        (address asset, uint256 amount) = g.quote(address(nft));
        assertEq(asset, address(0), "quote says native");
        assertEq(amount, g.fee(address(nft)), "quote is the ETH fee");

        // A coin with no rate is still the ETH rail — arming takes two posts.
        g.setToll(address(coin), 0);
        assertEq(g.toll(address(nft)), 0, "a coin without a rate charges nothing");
        (asset,) = g.quote(address(nft));
        assertEq(asset, address(0), "still native");
    }

    /// The price is the SAME question carried across a posted rate. Not an
    /// oracle, not a swap: `fee` still quotes the pool's EV times its surcharge,
    /// and the rate is the only thing that turns it into coin.
    function test_theTollIsTheEthQuoteCarriedAcrossThePostedRate() public {
        _twoPositions();
        _arm();
        uint256 f = g.fee(address(nft));
        assertEq(f, 0.011 ether, "0.01 harmonic mean at a 10% surcharge");
        assertEq(g.toll(address(nft)), TOLL, "the fee at 1,000 coin per ETH");
        (address asset, uint256 amount) = g.quote(address(nft));
        assertEq(asset, address(coin), "quote names the coin");
        assertEq(amount, TOLL, "quote is the toll, not the ETH fee");

        // Retuning is a posted act and it re-prices the next pull immediately.
        g.setToll(address(coin), RATE * 2);
        assertEq(g.toll(address(nft)), TOLL * 2, "the rate is the price");
    }

    /// The pull moves coin and NOT ether — including refusing ether outright.
    /// A buyer who read `fee` instead of `quote` gets a failed transaction, not
    /// a silent claim on ETH they will never think to collect.
    function test_theRequestPullsTheCoinAndRefusesEther() public {
        _twoPositions();
        _arm();

        vm.prank(cody);
        vm.expectRevert(EloGacha.TollRailTakesNoEth.selector);
        g.request{value: 0.011 ether}(address(nft), bytes32("s"), 0, 0, 0);

        uint256 before = coin.balanceOf(cody);
        uint256 drawId = _requestToll();

        assertEq(before - coin.balanceOf(cody), TOLL, "the buyer paid the toll");
        assertEq(coin.balanceOf(address(g)), TOLL, "the pool escrowed it");
        assertEq(address(g).balance, BACKING * 2, "no ether moved: only the two backings");
        (,,,,,,,, uint256 escrowedToll) = g.draws(drawId);
        assertEq(escrowedToll, TOLL, "the draw records the toll it holds");
        (,, uint256 ethFee,,,,,,) = g.draws(drawId);
        assertEq(ethFee, 0, "and no ETH fee");
        assertEq(g.accountedToll(), TOLL, "escrow is a named liability");
        _assertTollSolvent();
    }

    /// **No event field ever carries two assets.** `DrawRequested.fee` and
    /// `DrawForfeited.fee` are the ETH leg and only the ETH leg; the toll rides
    /// in `TollPaid`/`TollForfeited`, which name the coin. An indexer summing
    /// `fee` across both eras would otherwise add 11e18 of a coin to a column of
    /// ether and report an eleven-ETH draw — a plausible wrong unit, which is
    /// strictly worse than a zero sitting beside a `TollPaid` in the same
    /// transaction.
    function test_noEventFieldEverCarriesTwoAssets() public {
        _twoPositions();
        _arm();

        vm.recordLogs();
        uint256 drawId = _requestToll();
        Vm.Log[] memory logs = vm.getRecordedLogs();
        bytes32 requested =
            keccak256("DrawRequested(uint256,address,address,uint256,uint256,uint256,uint256)");
        bytes32 tollPaid = keccak256("TollPaid(uint256,address,address,uint256)");
        bool sawRequested;
        bool sawToll;
        for (uint256 i = 0; i < logs.length; i++) {
            if (logs[i].topics[0] == requested) {
                sawRequested = true;
                (uint256 fee,,,) = abi.decode(logs[i].data, (uint256, uint256, uint256, uint256));
                assertEq(fee, 0, "DrawRequested.fee must be the ETH leg, and there was none");
            }
            if (logs[i].topics[0] == tollPaid) {
                sawToll = true;
                assertEq(abi.decode(logs[i].data, (uint256)), TOLL, "TollPaid carries the amount");
                assertEq(address(uint160(uint256(logs[i].topics[3]))), address(coin), "and names the coin");
            }
        }
        assertTrue(sawRequested && sawToll, "both events fired");

        // The forfeit path keeps the same split.
        (,,,,, uint64 revealBlock,,,) = g.draws(drawId);
        arb.setL2Block(uint256(revealBlock) + 257);
        vm.recordLogs();
        g.expire(drawId);
        logs = vm.getRecordedLogs();
        bytes32 forfeited = keccak256("DrawForfeited(uint256,address,uint256)");
        bytes32 tollForfeited = keccak256("TollForfeited(uint256,address,address,uint256)");
        bool sawForfeit;
        bool sawTollForfeit;
        for (uint256 i = 0; i < logs.length; i++) {
            if (logs[i].topics[0] == forfeited) {
                sawForfeit = true;
                assertEq(abi.decode(logs[i].data, (uint256)), 0, "DrawForfeited.fee is the ETH leg");
            }
            if (logs[i].topics[0] == tollForfeited) {
                sawTollForfeit = true;
                assertEq(abi.decode(logs[i].data, (uint256)), TOLL, "and the toll rides in its own event");
            }
        }
        assertTrue(sawForfeit && sawTollForfeit, "both forfeit events fired");
    }

    /// `maxFee` is denominated in whatever `quote` returns. A buyer capping a
    /// `SCRY` charge with an ETH number would otherwise be capping nothing.
    function test_maxFeeCapsTheTollNotTheEthFee() public {
        _twoPositions();
        _arm();

        // 0.011 ether as a cap is astronomically ABOVE an 11-coin toll, so a
        // contract that compared against the ETH fee would wave this through.
        vm.prank(cody);
        vm.expectRevert(EloGacha.FeeAboveCap.selector);
        g.request(address(nft), bytes32("s"), TOLL - 1, 0, 0);

        vm.prank(cody);
        g.request(address(nft), bytes32("s"), TOLL, 0, 0);
    }

    /// A rate change re-prices the NEXT pull, never an open one. The same
    /// promise `pos.keepBps` keeps for a resolution: a knob may change what the
    /// next draw costs, never what a draw that already happened cost.
    function test_aRateChangeCannotRepriceAnOpenDraw() public {
        _twoPositions();
        _arm();
        uint256 drawId = _requestToll();

        g.setToll(address(coin), RATE * 5);
        (,,,,,,,, uint256 escrowedToll) = g.draws(drawId);
        assertEq(escrowedToll, TOLL, "the open draw kept the toll it paid");

        _settle(drawId);
        assertEq(_tollLiabilities(), TOLL, "and settled against that amount");
    }

    /// The split, to the wei, and the ETH rail untouched beside it.
    function test_everyUnitOfATollBecomesANamedLiability() public {
        (uint256 a,) = _twoPositions();
        _arm();
        uint256 drawId = _requestToll();
        uint256 houseEthBefore = g.houseOwed();

        uint256 drawn = _settle(drawId);

        uint256 burn = TOLL * 500 / 10_000; // 0.55
        uint256 house = TOLL * 100 / 10_000; // 0.11
        uint256 distributable = TOLL - burn - house; // 10.34
        uint256 topShare = distributable * 500 / 10_000; // 0.517
        uint256 each = (distributable - topShare) / 2; // 4.9115

        (uint256 esc, uint256 owedOut, uint256 houseToll, uint256 pots, uint256 credits, uint256 burnPending) =
            g.tollLedger();
        assertEq(esc, 0, "escrow released");
        assertEq(burnPending, burn, "the burn is set aside, not sent");
        assertEq(houseToll, house, "the house cut accrued on the toll rail");
        // Position `a` took the vacant top seat on deposit, so if it was drawn
        // its pot settled out with it; otherwise the pot is still standing.
        assertEq(pots + (drawn == a ? topShare : 0), topShare, "the top slice landed once");
        assertEq(esc + owedOut + houseToll + pots + credits + burnPending, TOLL, "nothing invented, nothing dropped");
        assertEq(g.accountedToll(), TOLL, "and accountedToll agrees");

        assertEq(g.houseOwed(), houseEthBefore, "the ETH rail took no cut from a toll");
        assertEq(g.escrowTotal(), 0, "and escrowed no ether");
        assertEq(g.owedToll(alice) + g.owedToll(bob), each + (drawn == a ? topShare : 0), "the drawn side was credited");
        _assertTollSolvent();
    }

    /// Two claims, two assets, and neither can be paid on the other's call.
    function test_claimAndClaimTollAreSeparateAndPayTheirOwnAsset() public {
        _twoPositions();
        _arm();
        uint256 drawId = _requestToll();
        uint256 drawn = _settle(drawId);
        address survivor = drawn == 1 ? bob : alice;

        // The survivor's toll share is waiting; there is no ETH waiting at all,
        // because no ether was ever charged on this pull.
        vm.prank(survivor);
        vm.expectRevert(EloGacha.NothingOwed.selector);
        g.claim();

        uint256[] memory ids = new uint256[](1);
        ids[0] = drawn == 1 ? 2 : 1;
        vm.prank(survivor);
        (uint256 eth, uint256 tol) = g.harvest(ids);
        assertEq(eth, 0, "nothing on the ETH rail");
        assertGt(tol, 0, "and a real share on the toll rail");
        assertEq(g.owedToll(survivor), tol, "harvest ACCRUES the toll; it does not push it");

        uint256 before = coin.balanceOf(survivor);
        vm.prank(survivor);
        uint256 paid = g.claimToll();
        assertEq(paid, tol, "claimToll paid exactly what was accrued");
        assertEq(coin.balanceOf(survivor) - before, tol, "in coin");
        assertEq(g.owedToll(survivor), 0, "and cleared");
        _assertTollSolvent();
    }

    /// A position that earned on both rails collects both. The rails are
    /// independent, not alternating: arming the toll does not orphan what the
    /// ETH era already credited.
    function test_aPositionThatEarnedOnBothRailsCollectsBoth() public {
        _twoPositions();

        // Era one: an ETH pull.
        uint256 f = g.fee(address(nft));
        vm.prank(cody);
        uint256 ethDraw = g.request{value: f}(address(nft), bytes32("eth"), 0, 0, 0);
        _settle(ethDraw);
        // Put the pool back to two positions so the second pull has a pool.
        vm.prank(alice);
        g.deposit{value: BACKING}(address(nft), 3);

        // Era two: arm the toll and pull again.
        _arm();
        uint256 tollDraw = _requestToll();
        _settle(tollDraw);

        // Whoever still holds a position is owed on both rails at once.
        uint256 ethOwed = g.owed(alice) + g.owed(bob);
        uint256 tollOwed = g.owedToll(alice) + g.owedToll(bob);
        assertGt(ethOwed, 0, "the ETH era's credits survived the arming");
        assertGt(tollOwed, 0, "and the toll era's exist beside them");

        _assertTollSolvent();
        assertGe(address(g).balance, g.accountedEth(), "the ETH rail is still solvent too");
    }

    /// **The property that would only ever fail once.** A toll coin that stops
    /// moving must not be able to hold a depositor's backing or their NFT.
    /// `withdraw` pushes ETH and accrues the toll; only `claimToll` — the call
    /// that actually needs the token — is allowed to fail.
    function test_aDeadTollCoinCannotBlockAnExit() public {
        (uint256 a,) = _twoPositions();
        _arm();
        uint256 drawId = _requestToll();
        uint256 drawn = _settle(drawId);
        uint256 survivorId = drawn == a ? 2 : a; // positions 1 and 2 hold tokens 1 and 2
        address survivor = drawn == a ? bob : alice;

        coin.setPaused(true);

        uint256 ethBefore = survivor.balance;
        vm.prank(survivor);
        g.withdraw(survivorId);
        assertEq(survivor.balance - ethBefore, BACKING, "the backing came home");
        assertEq(nft.ownerOf(survivorId), survivor, "and so did the token");
        assertGt(g.owedToll(survivor), 0, "the toll is still theirs, waiting");

        vm.prank(survivor);
        vm.expectRevert("token paused");
        g.claimToll();

        // ...and it is still there once the token recovers.
        coin.setPaused(false);
        vm.prank(survivor);
        assertGt(g.claimToll(), 0, "paid after the token came back");
    }

    /// A fee-on-transfer toll coin delivers less than it promised, and every
    /// downstream split would then be paying out of somebody else's credits.
    /// The balance delta is the only thing that catches it.
    function test_aShortDeliveringTollCoinIsRefused() public {
        _twoPositions();
        _arm();
        coin.setFeeBps(100);

        vm.prank(cody);
        vm.expectRevert(EloGacha.TollShortDelivered.selector);
        g.request(address(nft), bytes32("s"), 0, 0, 0);
    }

    /// The burn is accrued in `settle` and only ever counted when it lands.
    /// ⚠ It is not a supply reduction: the coins sit at a dead address and
    /// `totalSupply()` does not move.
    function test_theBurnIsAccruedInSettleAndCountedOnlyWhenItLands() public {
        _twoPositions();
        _arm();
        uint256 supplyBefore = coin.totalSupply();
        _settle(_requestToll());

        uint256 burn = TOLL * 500 / 10_000;
        (,,,,, uint256 burnPending) = g.tollLedger();
        assertEq(burnPending, burn, "set aside");
        assertEq(g.totalBurned(), 0, "and NOT yet counted as burned");
        assertEq(coin.balanceOf(DEAD), 0, "nothing has left");

        // Permissionless: the destination is a constant, so there is nothing to
        // choose and no reason to gate it.
        vm.prank(cody);
        assertEq(g.sweepBurn(), burn, "swept");
        assertEq(coin.balanceOf(DEAD), burn, "to 0xdEaD");
        assertEq(g.totalBurned(), burn, "counted when it left");
        assertEq(coin.totalSupply(), supplyBefore, "totalSupply did NOT fall");
        (,,,,, burnPending) = g.tollLedger();
        assertEq(burnPending, 0, "the liability is gone");

        vm.expectRevert(EloGacha.NothingToBurn.selector);
        g.sweepBurn();
    }

    /// The house's toll goes to the same posted sink as its ether, by the same
    /// permissionless push. When that sink is `EloFeeSplitter` this is the
    /// whole handoff, and it needed zero new machinery.
    function test_theHouseTollSweepsToThePostedSink() public {
        _twoPositions();
        _arm();
        _settle(_requestToll());

        uint256 house = TOLL * 100 / 10_000;
        vm.prank(cody);
        assertEq(g.sweepHouseToll(), house, "swept");
        assertEq(coin.balanceOf(sink), house, "to the sink, not to the caller");

        vm.expectRevert(EloGacha.NothingAccrued.selector);
        g.sweepHouseToll();
    }

    /// An expired draw forfeits the toll to the depositors exactly as a
    /// settlement would have distributed it — the port's sharpest edit, and it
    /// has to hold on both rails or the toll rail is a free option.
    function test_anExpiredTollDrawForfeitsToTheDepositors() public {
        _twoPositions();
        _arm();
        uint256 drawId = _requestToll();
        uint256 buyerBefore = coin.balanceOf(cody);

        (,,,,, uint64 revealBlock,,,) = g.draws(drawId);
        arb.setL2Block(uint256(revealBlock) + 257); // past the arbBlockHash window
        g.expire(drawId);

        assertEq(coin.balanceOf(cody), buyerBefore, "the buyer got nothing back");
        assertEq(_tollLiabilities(), TOLL, "and all of it is owed to somebody else");
        (uint256 esc,,,,,) = g.tollLedger();
        assertEq(esc, 0, "escrow released");
        _assertTollSolvent();
    }

    /// The top seat pays on both rails when it moves — a tenure that earned in
    /// coin and in ether is owed both, once. **The pot's exact size is asserted,
    /// not merely that it is nonzero**: `_vacateTop` zeroes both pots in one
    /// write, so a version that cleared the toll pot without crediting it would
    /// still leave the ledger self-consistent and simply lose the coins.
    function test_theTopSeatSettlesBothPotsWhenItMoves() public {
        // The rich position takes the seat and is rarely drawn (weight is
        // inverse to backing), so the tenure survives to accrue on both rails.
        vm.prank(bob);
        uint256 rich = g.deposit{value: BACKING * 8}(address(nft), 2);
        vm.prank(alice);
        g.deposit{value: BACKING}(address(nft), 1);
        assertEq(_topPositionId(), rich, "the rich position holds the seat");

        uint256 f = g.fee(address(nft));
        vm.prank(cody);
        _settle(g.request{value: f}(address(nft), bytes32("e"), 0, 0, 0));
        // Restore the pool to two positions for the second pull.
        vm.prank(alice);
        g.deposit{value: BACKING}(address(nft), 3);
        _arm();
        _settle(_requestToll());

        // The seat must still be held for this test to be about anything.
        assertEq(_topPositionId(), rich, "the seat survived both draws");
        (,, uint256 potEth) = _pots();
        uint256 potToll = _tollPot();
        assertGt(potEth, 0, "the tenure earned in ether");
        assertGt(potToll, 0, "and in coin");

        // A lower re-price forfeits the seat, which settles the tenure. Both
        // pots land on the holder, each on its own rail.
        uint256 ethBefore = bob.balance;
        uint256 tollBefore = g.owedToll(bob);
        // `reprice` settles the position's own fee share on both rails before it
        // touches the seat, so both legs are in the expected totals below.
        (uint256 pendEth, uint256 pendToll) = g.pendingFees(rich);
        vm.prank(bob);
        g.reprice(rich, BACKING);
        assertEq(_topPositionId(), 0, "the seat is vacant");
        // `reprice` pushes ETH, so the ether arrives as balance: the released
        // backing, plus the position's earned share, plus the pot, to the wei.
        assertEq(bob.balance - ethBefore, BACKING * 7 + pendEth + potEth, "the ETH pot banked with the refund");
        assertEq(g.owedToll(bob) - tollBefore, pendToll + potToll, "and the toll pot banked in coin");
        uint256 potTollAfter = _tollPot();
        assertEq(potTollAfter, 0, "the pot is spent, not stranded");
        _assertTollSolvent();
    }

    function _topPositionId() internal view returns (uint256 id) {
        (,,,,,,,,,, id,,,) = g.poolCard(address(nft));
    }

    function _pots() internal view returns (uint256 pending, uint256 topId, uint256 potEth) {
        (,,,,,,,,, pending, topId, potEth,,) = g.poolCard(address(nft));
    }

    function _tollPot() internal view returns (uint256 potToll) {
        (,,,, potToll,) = g.tollCard(address(nft));
    }

    /// `tollCard` is the toll rail's side of `poolCard`, kept out of that tuple
    /// on purpose — the meter and the page decode `poolCard` positionally, and a
    /// field inserted there would shift every one of them onto a wrong number.
    function test_tollCardReportsTheRailWithoutMovingPoolCard() public {
        _twoPositions();
        _arm();
        (address c, uint256 rate, uint256 burnBps, uint256 price, uint256 pot, uint256 vol) = g.tollCard(address(nft));
        assertEq(c, address(coin));
        assertEq(rate, RATE);
        assertEq(burnBps, 500);
        assertEq(price, TOLL);
        assertEq(pot, 0);
        assertEq(vol, 0);

        _settle(_requestToll());
        (,,,,, vol) = g.tollCard(address(nft));
        assertEq(vol, TOLL, "lifetime toll volume");

        // poolCard still answers 14 fields, and its `price` is still the ETH
        // quote the rate is applied to.
        (,,,,,,, uint256 ev, uint256 ethPrice,,,,,) = g.poolCard(address(nft));
        assertEq(ethPrice, g.fee(address(nft)), "poolCard.price is unchanged and still ETH");
        assertGt(ev, 0);
    }

    // ------------------------------------------------------------- the knobs

    /// **The coin may only move while the rail is empty**, because every credit
    /// on it is denominated in whatever this address says at the moment it is
    /// paid. The rate may move whenever — that is what makes "stop selling in
    /// `SCRY`" a different act from "change the ledger's denomination".
    function test_theCoinCannotMoveWhileAnythingIsOutstanding() public {
        _twoPositions();
        _arm();
        TollCoin other = new TollCoin();

        // Empty rail: free to change.
        g.setToll(address(other), RATE);
        g.setToll(address(coin), RATE);

        _settle(_requestToll());
        assertGt(g.accountedToll(), 0, "the rail now holds credits");

        vm.expectRevert(EloGacha.TollRailNotEmpty.selector);
        g.setToll(address(other), RATE);
        vm.expectRevert(EloGacha.TollRailNotEmpty.selector);
        g.setToll(address(0), 0);

        // But disarming by RATE is always legal and strands nothing.
        g.setToll(address(coin), 0);
        assertEq(g.toll(address(nft)), 0, "the next pull is back on ETH");
        assertEq(g.tollCoin(), address(coin), "and the ledger still knows its coin");
        (address asset,) = g.quote(address(nft));
        assertEq(asset, address(0), "quote agrees");

        // Everything already earned is still collectable.
        vm.prank(cody);
        g.sweepBurn();
        vm.prank(cody);
        g.sweepHouseToll();
        // Read BEFORE the prank: `vm.prank` applies to the next call of any
        // kind, and a view read in between spends it on the view.
        bool aliceOwed = g.owedToll(alice) > 0;
        bool bobOwed = g.owedToll(bob) > 0;
        if (aliceOwed) {
            vm.prank(alice);
            g.claimToll();
        }
        if (bobOwed) {
            vm.prank(bob);
            g.claimToll();
        }
    }

    function test_theTollKnobsAreBoundedAndOwnerOnly() public {
        vm.expectRevert(EloGacha.BurnTooHigh.selector);
        g.setTollBurnBps(2_001);
        g.setTollBurnBps(2_000);

        vm.expectRevert(EloGacha.RateWithoutCoin.selector);
        g.setToll(address(0), 1);

        vm.expectRevert(EloGacha.NotAContract.selector);
        g.setToll(address(0xDEAD01), RATE);

        vm.prank(alice);
        vm.expectRevert(EloGacha.OwnerOnly.selector);
        g.setToll(address(coin), RATE);
        vm.prank(alice);
        vm.expectRevert(EloGacha.OwnerOnly.selector);
        g.setTollBurnBps(1);
    }

    /// A one-position pool still conserves the toll: the fee is spread while
    /// the drawn position is STILL ACTIVE, so the sole depositor collects the
    /// whole distributable and the count never reaches zero on this path.
    /// (`_distribute`'s empty-pool branch is defensive: `request` refuses a pool
    /// with no positions, and deposits are frozen for the length of a draw, so
    /// nothing can empty a pool between the request and the split.)
    function test_aOnePositionPoolStillConservesTheToll() public {
        vm.prank(alice);
        g.deposit{value: BACKING}(address(nft), 1);
        _arm();
        _settle(_requestToll()); // the sole position is drawn out

        uint256 burn = TOLL * 500 / 10_000;
        uint256 house = TOLL * 100 / 10_000;
        (uint256 esc, uint256 owedOut, uint256 houseToll, uint256 pots, uint256 credits, uint256 burnPending) =
            g.tollLedger();
        assertEq(esc, 0, "escrow released");
        assertEq(burnPending, burn, "the burn slice");
        assertEq(houseToll, house, "the house cut");
        assertEq(owedOut, TOLL - burn - house, "and every remaining unit went to the sole depositor");
        assertEq(g.owedToll(alice), owedOut, "who is alice");
        assertEq(esc + owedOut + houseToll + pots + credits + burnPending, TOLL, "nothing invented, nothing dropped");
        _assertTollSolvent();
    }
}
