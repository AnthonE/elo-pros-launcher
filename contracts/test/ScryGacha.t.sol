// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {ScryGacha} from "../src/ScryGacha.sol";
import {MockArbSys} from "./MockArbSys.sol";

interface IERC721Receiver {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

/// @dev The minimum ERC-721 the pool actually touches, plus two switches the
///      suite needs: `paused` makes a transfer revert (a collection that stops
///      working AFTER a deposit — the stuck-token path), and `safeTransferFrom`
///      is real, so the test that pushes a token in with a safe transfer proves
///      the pool refuses it rather than merely asserting a comment.
contract GachaNft {
    mapping(uint256 => address) public ownerOf;
    mapping(uint256 => address) public approved;
    mapping(address => mapping(address => bool)) public isApprovedForAll;
    bool public paused;

    function setPaused(bool v) external {
        paused = v;
    }

    function mint(address to, uint256 id) external {
        ownerOf[id] = to;
    }

    function setApprovalForAll(address who, bool v) external {
        isApprovedForAll[msg.sender][who] = v;
    }

    function transferFrom(address from, address to, uint256 id) public {
        require(!paused, "paused");
        require(ownerOf[id] == from, "wrong from");
        require(
            msg.sender == from || approved[id] == msg.sender || isApprovedForAll[from][msg.sender], "not authorized"
        );
        delete approved[id];
        ownerOf[id] = to;
    }

    function safeTransferFrom(address from, address to, uint256 id) external {
        transferFrom(from, to, id);
        if (to.code.length > 0) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, from, id, "")
                    == IERC721Receiver.onERC721Received.selector,
                "receiver refused"
            );
        }
    }
}

/// @dev A collection whose `transferFrom` returns without moving anything. The
///      `ownerOf` check in `_pullNft` is the only thing that catches it.
contract GachaNftLiar {
    mapping(uint256 => address) public ownerOf;

    function mint(address to, uint256 id) external {
        ownerOf[id] = to;
    }

    function transferFrom(address, address, uint256) external {}
}

contract ScryGachaTest is Test {
    ScryGacha internal g;
    GachaNft internal nft;

    address internal alice = address(0xA11CE); // depositor, holds the odd tokens
    address internal bob = address(0xB0B); // depositor, holds the even tokens
    address internal cody = address(0xC0DE); // buyer
    address internal stranger = address(0x57A);
    address internal sink = address(0x5EEE);

    MockArbSys internal arb;
    address internal constant ARBSYS = 0x0000000000000000000000000000000000000064;

    uint256 internal constant NUM = 1e36;
    uint256 internal constant CHEAP = 0.01 ether;
    uint256 internal constant RICH = 0.09 ether;
    uint256 internal constant DELAY = 100;

    function setUp() public {
        // The clock first: the contract reads ArbSys, so nothing works without
        // it. `etch` copies the runtime code to 0x64 and leaves storage there
        // fresh, which the setters then fill.
        vm.etch(ARBSYS, address(new MockArbSys()).code);
        arb = MockArbSys(ARBSYS);
        // BOTH are set explicitly because `vm.etch` copies runtime code and
        // NOTHING else: no constructor runs and no storage comes with it, so the
        // mock's `window = 256` initializer never executed at this address. Left
        // implicit, window was 0, every reveal read as expired — and the expiry
        // test passed for the wrong reason, which is the failure mode worth
        // remembering here.
        arb.setL2Block(20_000_000); // a plausible 4663 height
        arb.setWindow(256);

        g = new ScryGacha(DELAY, sink);
        nft = new GachaNft();
        vm.deal(alice, 200 ether);
        vm.deal(bob, 200 ether);
        vm.deal(cody, 100 ether);
        vm.deal(stranger, 10 ether);
        g.openPool(address(nft), CHEAP, 1_000, 8_500);
        for (uint256 i = 1; i <= 60; i++) {
            nft.mint(_holder(i), i);
        }
        vm.prank(alice);
        nft.setApprovalForAll(address(g), true);
        vm.prank(bob);
        nft.setApprovalForAll(address(g), true);
        vm.prank(cody);
        nft.setApprovalForAll(address(g), true);
    }

    // ---------------------------------------------------------------- helpers

    function _holder(uint256 tokenId) internal view returns (address) {
        return tokenId % 2 == 0 ? bob : alice;
    }

    /// Every wei in the contract belongs to a named liability. Asserted after
    /// each action in the lifecycle tests: it is what makes "backing only ever
    /// pays its own standing bid" checkable rather than merely commented.
    function _assertSolvent() internal view {
        assertGe(address(g).balance, g.accountedEth(), "insolvent: balance below liabilities");
    }

    function _deposit(uint256 tokenId, uint256 backing) internal returns (uint256 id) {
        vm.prank(_holder(tokenId));
        id = g.deposit{value: backing}(address(nft), tokenId);
    }

    function _request(bytes32 salt, uint256 autoBidAbove) internal returns (uint256 drawId) {
        uint256 f = g.fee(address(nft));
        vm.prank(cody);
        drawId = g.request{value: f}(address(nft), salt, 0, 0, autoBidAbove);
    }

    /// Advance the L2 clock past the reveal and pin that block's hash. Pinned
    /// rather than left to the mock's default, so a draw's outcome is a function
    /// the test computes independently — see `_predict`.
    function _reveal(uint256 drawId, bytes32 bh) internal returns (bytes32) {
        (,,,,, uint64 revealBlock,,,) = g.draws(drawId);
        if (arb.l2() <= uint256(revealBlock)) arb.setL2Block(uint256(revealBlock) + 1);
        arb.setHash(uint256(revealBlock), bh);
        return bh;
    }

    /// Search for a reveal hash that lands on `wanted`. The contract is not
    /// touched — `_predict` is a pure walk of public state — so this only makes
    /// a test deterministic about WHICH position it exercises, which beats
    /// branching every assertion on the draw.
    function _revealHitting(uint256 drawId, uint256 wanted) internal returns (bytes32 bh) {
        for (uint256 i = 0; i < 200; i++) {
            bh = _reveal(drawId, keccak256(abi.encode("hit", drawId, i)));
            if (_predict(drawId, bh) == wanted) return bh;
        }
        revert("no reveal hash hit the wanted position");
    }

    function _backingOf(uint256 id) internal view returns (uint256 backing) {
        (,,,, backing,,,,,,,,,) = g.positions(id);
    }

    function _statusOf(uint256 id) internal view returns (ScryGacha.PositionStatus s) {
        (,,,,,,,,,,,, s,) = g.positions(id);
    }

    function _slotOf(uint256 id) internal view returns (uint256 slot) {
        (,,,,,, slot,,,,,,,) = g.positions(id);
    }

    function _count() internal view returns (uint256 count) {
        (,,,,, count,,,,,,,,) = g.poolCard(address(nft));
    }

    function _totalWeight() internal view returns (uint256 w) {
        (,,,,,, w,,,,,,,) = g.poolCard(address(nft));
    }

    function _pendingDraw() internal view returns (uint256 d) {
        (,,,,,,,,, d,,,,) = g.poolCard(address(nft));
    }

    function _topId() internal view returns (uint256 id) {
        (,,,,,,,,,, id,,,) = g.poolCard(address(nft));
    }

    function _pot() internal view returns (uint256 pot) {
        (,,,,,,,,,,, pot,,) = g.poolCard(address(nft));
    }

    /// The ETH leg of `pendingFees`, which returns `(eth, toll)` since the toll
    /// rail landed 2026-07-27. Every caller below predates that and is asserting
    /// on the ETH rail, so naming the leg here beats destructuring at each site.
    function _pendingEth(uint256 positionId) internal view returns (uint256 eth) {
        (eth,) = g.pendingFees(positionId);
    }

    /// The same selection the contract runs, recomputed from public state: the
    /// seed, modulo the total weight, walked over the slots in order. A test
    /// that asks the contract for its own answer proves nothing; this walks the
    /// cumulative weights independently.
    function _predict(uint256 drawId, bytes32 bh) internal view returns (uint256) {
        (address collection, address buyer,,,,, bytes32 salt,,) = g.draws(drawId);
        uint256 target = uint256(keccak256(abi.encode(bh, salt, drawId, collection, buyer))) % _totalWeight();
        uint256 cumulative;
        for (uint256 slot = 1; slot <= 64; slot++) {
            uint256 id = g.positionAtSlot(collection, slot);
            if (id == 0) continue;
            cumulative += NUM / _backingOf(id);
            if (target < cumulative) return id;
        }
        revert("prediction fell off the end");
    }

    // --------------------------------------------------------------- curation

    function test_openPoolGatesTheFloorTheBidAndTheSurcharge() public {
        GachaNft other = new GachaNft();
        vm.expectRevert(ScryGacha.BelowFloor.selector);
        g.openPool(address(other), CHEAP - 1, 1_000, 8_500);
        vm.expectRevert(ScryGacha.BidOutOfRange.selector);
        g.openPool(address(other), CHEAP, 1_000, 9_501);
        vm.expectRevert(ScryGacha.BidOutOfRange.selector);
        g.openPool(address(other), CHEAP, 1_000, 4_999);
        vm.expectRevert(ScryGacha.SurchargeTooHigh.selector);
        g.openPool(address(other), CHEAP, 5_001, 8_500);
        vm.expectRevert(ScryGacha.NotAContract.selector);
        g.openPool(address(0xDEAD), CHEAP, 1_000, 8_500);
        vm.expectRevert(ScryGacha.PoolExists.selector);
        g.openPool(address(nft), CHEAP, 1_000, 8_500);
    }

    function test_curatorMayOpenPoolsAndBlockingIsSticky() public {
        GachaNft other = new GachaNft();
        vm.prank(stranger);
        vm.expectRevert(ScryGacha.OwnerOrCurator.selector);
        g.openPool(address(other), CHEAP, 1_000, 8_500);

        g.setCurator(stranger);
        vm.prank(stranger);
        g.openPool(address(other), CHEAP, 1_000, 8_500);
        (bool exists, bool open,,,,,,,,,,,,) = g.poolCard(address(other));
        assertTrue(exists && open);

        // Sticky: blocking closes the pool and can never be undone, so removing
        // a malicious collection is not reversible by a later curator.
        g.blockCollection(address(other));
        (, bool openAfter,,,,,,,,,,,,) = g.poolCard(address(other));
        assertFalse(openAfter);
        GachaNft third = new GachaNft();
        g.blockCollection(address(third));
        vm.expectRevert(ScryGacha.CollectionIsBlocked.selector);
        g.openPool(address(third), CHEAP, 1_000, 8_500);
    }

    // ---------------------------------------------------------------- deposit

    function test_depositEscrowsTheTokenAndWeighsInverseToBacking() public {
        uint256 id = _deposit(1, CHEAP);
        assertEq(nft.ownerOf(1), address(g), "token not escrowed");
        assertEq(g.positionOfToken(address(nft), 1), id);
        assertEq(_backingOf(id), CHEAP);
        assertEq(g.backingTotal(), CHEAP);
        assertEq(g.treeRoot(address(nft)), NUM / CHEAP, "tree root != weight");
        _assertSolvent();

        uint256 rich = _deposit(2, RICH);
        // Inverse: nine times the backing, one ninth of the weight.
        assertEq(g.treeRoot(address(nft)), NUM / CHEAP + NUM / RICH);
        assertGt(g.drawChancePpm(id), g.drawChancePpm(rich) * 8);
        _assertSolvent();
    }

    function test_depositBelowTheFloorIsRefused() public {
        vm.prank(alice);
        vm.expectRevert(ScryGacha.BelowMinBacking.selector);
        g.deposit{value: CHEAP - 1}(address(nft), 1);
    }

    function test_aSafeTransferIntoThePoolReverts() public {
        // No onERC721Received, on purpose: a token pushed in that way would be
        // an untracked deposit with no backing behind it.
        vm.prank(alice);
        vm.expectRevert();
        nft.safeTransferFrom(alice, address(g), 1);
        assertEq(nft.ownerOf(1), alice);
    }

    function test_aCollectionThatLiesAboutTransferIsCaught() public {
        GachaNftLiar liar = new GachaNftLiar();
        liar.mint(alice, 7);
        g.openPool(address(liar), CHEAP, 1_000, 8_500);
        vm.prank(alice);
        vm.expectRevert(ScryGacha.NftTransferFailed.selector);
        g.deposit{value: CHEAP}(address(liar), 7);
    }

    // ------------------------------------------------------------------ price

    function test_thePriceIsTheHarmonicMeanTimesTheSurcharge() public {
        _deposit(1, CHEAP);
        assertEq(g.harmonicMean(address(nft)), CHEAP, "one position: EV is its backing");
        assertEq(g.fee(address(nft)), CHEAP * 11_000 / 10_000);

        _deposit(2, RICH);
        // Harmonic mean of 0.01 and 0.09 is 0.018 — NOT the 0.05 arithmetic
        // mean. That gap is the whole reason a rich pool prices cheaply.
        assertApproxEqAbs(g.harmonicMean(address(nft)), 0.018 ether, 1e6);
        assertLt(g.harmonicMean(address(nft)), (CHEAP + RICH) / 2);
        assertEq(g.fee(address(nft)), g.harmonicMean(address(nft)) * 11_000 / 10_000);
    }

    function testFuzz_thePriceSitsBetweenTheBackingsAndBelowTheirMean(uint256 a, uint256 b) public {
        a = bound(a, CHEAP, 50 ether);
        b = bound(b, CHEAP, 50 ether);
        _deposit(1, a);
        _deposit(2, b);
        uint256 ev = g.harmonicMean(address(nft));
        assertLe(ev, (a + b) / 2 + 1, "harmonic mean above the arithmetic mean");
        assertGe(ev + 1, a < b ? a : b, "harmonic mean below the cheapest backing");
        assertLe(ev, a > b ? a : b, "harmonic mean above the richest backing");
    }

    function test_drawChanceIsExactlyTheWeightShare() public {
        _deposit(1, CHEAP);
        _deposit(2, RICH);
        // w1 = 1e20, w2 = 1e36/9e16: the cheap position takes ~90% of draws.
        assertEq(g.drawChancePpm(1), 900_000);
        assertEq(g.drawChancePpm(2), 99_999);
    }

    // ------------------------------------------------------------------- draw

    function test_settlePicksThePositionTheSeedPointsAt() public {
        _deposit(1, CHEAP);
        _deposit(2, RICH);
        uint256 drawId = _request(bytes32("salt-1"), 0);

        vm.expectRevert(ScryGacha.TooEarly.selector);
        g.settle(drawId);

        bytes32 bh = _reveal(drawId, keccak256("block"));
        uint256 expected = _predict(drawId, bh);
        uint256 got = g.settle(drawId);
        assertEq(got, expected, "selection disagreed with the independent walk");
        assertEq(uint256(_statusOf(got)), uint256(ScryGacha.PositionStatus.Drawn));
        assertEq(_count(), 1, "the drawn position did not leave the pool");
        assertEq(_pendingDraw(), 0, "pool still locked after settlement");
        _assertSolvent();
    }

    function test_theFeeSplitsEquallyPaysTheHouseAndFundsTheTop() public {
        _deposit(1, CHEAP);
        uint256 rich = _deposit(2, RICH);
        assertEq(_topId(), rich, "the richest position holds the top");
        uint256 f = g.fee(address(nft));
        uint256 drawId = _request(bytes32("salt-2"), 0);

        uint256 house = f * 100 / 10_000;
        uint256 distributable = f - house;
        uint256 topShare = distributable * 500 / 10_000;
        uint256 each = (distributable - topShare) / 2;

        bytes32 bh = _revealHitting(drawId, 1); // land on the cheap position
        g.settle(drawId);

        assertEq(g.houseOwed(), house, "house cut");
        assertEq(_pot(), topShare, "top pot funded");
        assertEq(g.owed(alice), each, "the drawn depositor was paid its share");
        assertEq(_pendingEth(rich), each, "the survivor's share is waiting");
        // Nothing was invented and nothing dropped: the fee is now entirely a
        // set of liabilities.
        assertEq(house + topShare + each * 2 + (distributable - topShare) % 2, f);
        _assertSolvent();
        bh; // the hash is the fixture, not an assertion
    }

    function test_theDrawnPositionSharesInTheDrawThatTookIt() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP); // equal backing, identical weights
        uint256 f = g.fee(address(nft));
        uint256 drawId = _request(bytes32("salt-3"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);

        // Position 1 was still active when the fee was routed, because it bore
        // the selection weight for this very draw. It also held the top (equal
        // backings, and the first deposit takes a vacant top), so its pot
        // settled in the same act — both halves land on its depositor.
        uint256 house = f * 100 / 10_000;
        uint256 distributable = f - house;
        uint256 topShare = distributable * 500 / 10_000;
        uint256 each = (distributable - topShare) / 2;
        assertEq(g.owed(alice), each + topShare, "the drawn position earned nothing from its own draw");
        assertEq(_pendingEth(2), each, "the survivor's equal share");
        assertEq(_pot(), 0, "the drawn top's pot did not settle");
    }

    function test_cheapPositionsAreDrawnFarMoreOften() public {
        // The claim under the whole design: weight is inverse, so the cheap side
        // of a 9:1 backing split takes the overwhelming majority of draws.
        uint256 cheapWins;
        uint256 richWins;
        uint256 token = 10;
        _deposit(token++, CHEAP);
        _deposit(token++, RICH);
        for (uint256 i = 0; i < 16; i++) {
            uint256 drawId = _request(keccak256(abi.encode("loop", i)), 0);
            bytes32 bh = _reveal(drawId, keccak256(abi.encode("bh", i)));
            uint256 drawn = _predict(drawId, bh);
            uint256 backing = _backingOf(drawn);
            g.settle(drawId);
            vm.prank(cody);
            g.keepNft(drawn);
            if (backing == CHEAP) {
                cheapWins++;
                _deposit(token++, CHEAP);
            } else {
                richWins++;
                _deposit(token++, RICH);
            }
            _assertSolvent();
        }
        assertGt(cheapWins, richWins * 3, "inverse weighting did not dominate the draw");
    }

    // ------------------------------------------------------------- resolution

    function test_keepNftMovesTheTokenAndReturnsTheBackingLessTheCut() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 drawId = _request(bytes32("keep"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);

        uint256 before = g.owed(alice);
        vm.prank(cody);
        g.keepNft(1);

        assertEq(nft.ownerOf(1), cody, "buyer did not get the token");
        uint256 cut = CHEAP * 100 / 10_000;
        assertEq(g.owed(alice) - before, CHEAP - cut, "depositor's backing return");
        assertEq(uint256(_statusOf(1)), uint256(ScryGacha.PositionStatus.Settled));
        assertEq(g.positionOfToken(address(nft), 1), 0, "escrow record not cleared");
        _assertSolvent();
    }

    function test_takeBidPaysFromItsOwnBackingAndSendsTheTokenHome() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 drawId = _request(bytes32("bid"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);

        uint256 houseBefore = g.houseOwed();
        vm.prank(cody);
        g.takeBid(1);

        uint256 payout = CHEAP * 8_500 / 10_000;
        assertEq(g.owed(cody), payout, "buyer's standing-bid payout");
        assertEq(nft.ownerOf(1), alice, "token did not return to its depositor");
        // The retained slice is the whole house take on this outcome.
        assertEq(g.houseOwed() - houseBefore, CHEAP - payout);
        _assertSolvent();
    }

    function test_theBidIsFundedOnlyByItsOwnBackingNeverTheOtherPositions() public {
        // A jackpot draw: the rich position's bid is many times the fee paid,
        // and every wei of it comes out of that position's own escrow.
        _deposit(1, CHEAP);
        uint256 rich = _deposit(2, RICH);
        uint256 f = g.fee(address(nft));
        uint256 drawId = _request(bytes32("jackpot"), 0);
        _revealHitting(drawId, rich);
        g.settle(drawId);

        uint256 cheapBackingBefore = _backingOf(1);
        vm.prank(cody);
        g.takeBid(rich);

        uint256 payout = RICH * 8_500 / 10_000;
        assertGt(payout, f * 3, "the jackpot should dwarf the fee");
        assertEq(g.owed(cody), payout);
        assertEq(_backingOf(1), cheapBackingBefore, "another position's backing moved");
        assertEq(nft.ownerOf(2), bob, "token did not return to its depositor");
        _assertSolvent();
    }

    function test_autoBidAboveResolvesInTheSameTransaction() public {
        _deposit(1, CHEAP);
        uint256 rich = _deposit(2, RICH);

        // Rule: take the bid on anything backed above the floor.
        uint256 drawId = _request(bytes32("auto-rich"), CHEAP);
        _revealHitting(drawId, rich);
        g.settle(drawId);
        assertEq(uint256(_statusOf(rich)), uint256(ScryGacha.PositionStatus.Settled), "not resolved atomically");
        assertEq(g.owed(cody), RICH * 8_500 / 10_000, "rich draw should have taken the bid");
        assertEq(nft.ownerOf(2), bob);

        // Same rule, cheap draw: at or below the threshold, the token is handed
        // over in the same transaction.
        _deposit(4, RICH);
        uint256 second = _request(bytes32("auto-cheap"), CHEAP);
        _revealHitting(second, 1);
        g.settle(second);
        assertEq(nft.ownerOf(1), cody, "cheap draw should have handed over the token");
        _assertSolvent();
    }

    function test_keepNftIsStrictSoTheBuyerCanFallBackToTheBid() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 drawId = _request(bytes32("stuck"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);

        nft.setPaused(true);
        vm.prank(cody);
        vm.expectRevert("paused");
        g.keepNft(1);

        // The fallback still works, and it delivers best-effort so the ETH legs
        // can never be held hostage by the collection.
        vm.prank(cody);
        g.takeBid(1);
        assertEq(g.owed(cody), CHEAP * 8_500 / 10_000);
        assertEq(g.stuckNftRecipient(1), alice, "no pull claim recorded");

        nft.setPaused(false);
        vm.prank(stranger);
        vm.expectRevert(ScryGacha.NotTheRecipient.selector);
        g.recoverStuckNft(1);
        vm.prank(alice);
        g.recoverStuckNft(1);
        assertEq(nft.ownerOf(1), alice);
        assertEq(g.stuckNftRecipient(1), address(0));
    }

    function test_theDepositorMayResolveOnlyAfterTheBuyersWindow() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 drawId = _request(bytes32("win"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);

        vm.prank(alice);
        vm.expectRevert(ScryGacha.BuyersWindow.selector);
        g.depositorReclaimNft(1);

        // via_ir may fold consecutive block.timestamp reads across a warp, so
        // the clock is read through the cheatcode rather than the opcode.
        vm.warp(vm.getBlockTimestamp() + 1 hours + 1);
        vm.prank(alice);
        g.depositorReclaimNft(1);
        assertEq(nft.ownerOf(1), alice);
        assertEq(g.owed(cody), CHEAP * 8_500 / 10_000);
        _assertSolvent();
    }

    function test_theDepositorMayAlsoKeepTheEthAfterTheWindow() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 drawId = _request(bytes32("win2"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);

        vm.warp(vm.getBlockTimestamp() + 1 hours + 1);
        uint256 before = g.owed(alice);
        vm.prank(alice);
        g.depositorReclaimBacking(1);
        assertEq(nft.ownerOf(1), cody, "buyer should get the token on this branch");
        assertEq(g.owed(alice) - before, CHEAP - CHEAP * 100 / 10_000);
        _assertSolvent();
    }

    function test_anyoneMayFinalizeAfterTheFinalizeWindow() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 drawId = _request(bytes32("fin"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);

        vm.prank(stranger);
        vm.expectRevert(ScryGacha.TooEarlyToFinalize.selector);
        g.finalize(1);

        vm.warp(vm.getBlockTimestamp() + 24 hours + 1);
        vm.prank(stranger);
        g.finalize(1);
        // The default is the completed sale: token to the buyer, backing home.
        assertEq(nft.ownerOf(1), cody);
        assertEq(uint256(_statusOf(1)), uint256(ScryGacha.PositionStatus.Settled));
        _assertSolvent();
    }

    // ----------------------------------------------------------------- expiry

    function test_anExpiredDrawForfeitsTheFeeAndNeverRefundsTheBuyer() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 f = g.fee(address(nft));
        uint256 codyBefore = cody.balance;
        uint256 drawId = _request(bytes32("exp"), 0);
        (,,,,, uint64 revealBlock,,,) = g.draws(drawId);

        // Past the reveal window: `arbBlockHash` refuses, so nobody can settle.
        arb.setL2Block(uint256(revealBlock) + 257);
        vm.expectRevert(ScryGacha.RevealExpired.selector);
        g.settle(drawId);

        vm.prank(stranger);
        g.expire(drawId);

        // The buyer is NOT made whole. This is the port's sharpest edit: a
        // refund here would price a free option on every draw — read the
        // outcome at the reveal block, settle the good ones, walk from the bad.
        assertEq(cody.balance, codyBefore - f, "buyer was refunded");
        assertEq(g.owed(cody), 0, "buyer was credited");
        uint256 house = f * 100 / 10_000;
        uint256 topShare = (f - house) * 500 / 10_000;
        assertEq(g.houseOwed(), house, "house still takes its posted cut");
        assertEq(_pot(), topShare, "the top still earns");
        assertEq(_pendingEth(1) + _pendingEth(2), (f - house - topShare) / 2 * 2, "depositors were not paid");
        _assertSolvent();
    }

    function test_expiryIsTheEscapeThatUnlocksAJammedPool() public {
        _deposit(1, CHEAP);
        uint256 drawId = _request(bytes32("jam"), 0);
        (,,,,, uint64 revealBlock,,,) = g.draws(drawId);

        // While a draw is open the set is frozen — that freeze is what removes
        // the need for FWA's staging queue, and expiry is what guarantees the
        // freeze can never be permanent.
        vm.prank(alice);
        vm.expectRevert(ScryGacha.DrawPending.selector);
        g.deposit{value: CHEAP}(address(nft), 3);

        // Before the reveal there is nothing to expire...
        vm.expectRevert(ScryGacha.TooEarly.selector);
        g.expire(drawId);
        // ...and inside the window the hash still answers, so expiry is refused
        // in favour of settling. The two branches are complementary by
        // construction: both read `arbBlockHash`'s own success.
        arb.setL2Block(uint256(revealBlock) + 1);
        vm.expectRevert(ScryGacha.StillSettleable.selector);
        g.expire(drawId);

        arb.setL2Block(uint256(revealBlock) + 300);
        g.expire(drawId);
        assertEq(_pendingDraw(), 0, "pool still locked after expiry");
        _deposit(3, CHEAP);
    }

    function test_aPendingDrawFreezesEveryPoolMutation() public {
        uint256 id = _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        _request(bytes32("freeze"), 0);

        vm.prank(alice);
        vm.expectRevert(ScryGacha.DrawPending.selector);
        g.deposit{value: CHEAP}(address(nft), 3);
        vm.prank(alice);
        vm.expectRevert(ScryGacha.DrawPending.selector);
        g.withdraw(id);
        vm.prank(alice);
        vm.expectRevert(ScryGacha.DrawPending.selector);
        g.reprice{value: CHEAP}(id, CHEAP * 2);
        vm.prank(cody);
        vm.expectRevert(ScryGacha.DrawPending.selector);
        g.request{value: 1 ether}(address(nft), bytes32("second"), 0, 0, 0);
    }

    function test_harvestIsAllowedMidDrawBecauseItMovesNoWeight() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 first = _request(bytes32("h1"), 0);
        _revealHitting(first, 1);
        g.settle(first);
        vm.prank(cody);
        g.keepNft(1);

        _deposit(3, CHEAP); // back to two positions
        _request(bytes32("h2"), 0); // and a draw in flight

        uint256[] memory ids = new uint256[](1);
        ids[0] = 2; // bob's surviving position
        uint256 balBefore = bob.balance;
        uint256 earned = _pendingEth(2);
        assertGt(earned, 0, "the survivor earned nothing");
        vm.prank(bob);
        // harvest returns (eth, toll) since the toll rail landed 2026-07-27;
        // this is an ETH-rail pool, so the toll leg must be zero, not ignored.
        (uint256 harvestedEth, uint256 harvestedToll) = g.harvest(ids);
        assertEq(harvestedEth, earned);
        assertEq(harvestedToll, 0, "an ETH-rail pool must harvest no toll");
        assertEq(bob.balance, balBefore + earned, "harvest paid the wrong amount");
        _assertSolvent();
    }

    // ------------------------------------------------------------- exit paths

    function test_withdrawReturnsTheTokenTheBackingAndTheFees() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 drawId = _request(bytes32("w"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);
        vm.prank(cody);
        g.keepNft(1);

        uint256 earned = _pendingEth(2);
        assertGt(earned, 0, "survivor earned nothing");
        uint256 balBefore = bob.balance;

        vm.prank(bob);
        g.withdraw(2);

        assertEq(bob.balance, balBefore + CHEAP + earned, "backing + fees not paid out");
        assertEq(nft.ownerOf(2), bob, "token not returned");
        assertEq(uint256(_statusOf(2)), uint256(ScryGacha.PositionStatus.Withdrawn));
        assertEq(_count(), 0);
        assertEq(g.treeRoot(address(nft)), 0, "tree not emptied");
        _assertSolvent();
    }

    function test_theSlotIsRecycledAfterAnExit() public {
        uint256 a = _deposit(1, CHEAP);
        uint256 slotA = _slotOf(a);
        vm.prank(alice);
        g.withdraw(a);
        uint256 b = _deposit(3, CHEAP);
        assertEq(_slotOf(b), slotA, "slot was not reused");
        assertEq(g.treeRoot(address(nft)), NUM / CHEAP, "a recycled slot left stale weight");
    }

    function test_repriceMovesTheWeightAndSettlesTheDifference() public {
        uint256 id = _deposit(1, CHEAP);
        assertEq(g.treeRoot(address(nft)), NUM / CHEAP);

        vm.prank(alice);
        g.reprice{value: RICH - CHEAP}(id, RICH);
        assertEq(_backingOf(id), RICH);
        assertEq(g.treeRoot(address(nft)), NUM / RICH, "weight not re-priced");
        assertEq(g.backingTotal(), RICH);
        _assertSolvent();

        uint256 balBefore = alice.balance;
        vm.prank(alice);
        g.reprice(id, CHEAP);
        assertEq(alice.balance, balBefore + (RICH - CHEAP), "reduction not refunded");
        assertEq(g.treeRoot(address(nft)), NUM / CHEAP);
        assertEq(g.backingTotal(), CHEAP);
        _assertSolvent();

        vm.prank(alice);
        vm.expectRevert(ScryGacha.BelowMinBacking.selector);
        g.reprice(id, CHEAP - 1);
    }

    function test_aLowerRepriceForfeitsTheTopSlot() public {
        _deposit(1, CHEAP);
        uint256 rich = _deposit(2, RICH);
        assertEq(_topId(), rich, "the richest position should hold the top");

        vm.prank(bob);
        g.reprice(rich, CHEAP); // a retreat gives up the title
        assertEq(_topId(), 0, "the top survived a reduction");
    }

    // ---------------------------------------------------------------- the top

    function test_claimTopNeedsToBeatTheStandingTopByTheThreshold() public {
        uint256 a = _deposit(1, 1 ether);
        assertEq(_topId(), a);
        // 1.05x does not clear a 10% bar; 1.10x does.
        uint256 b = _deposit(2, 1.05 ether);
        assertEq(_topId(), a, "the top flipped on a dust overtake");
        vm.prank(bob);
        vm.expectRevert(ScryGacha.TopNotBeaten.selector);
        g.claimTop(b);

        vm.prank(bob);
        g.reprice{value: 0.05 ether}(b, 1.1 ether);
        assertEq(_topId(), b, "clearing the bar did not seize the top");
    }

    function test_theTopPotSettlesToItsHolderWhenTheTopMoves() public {
        _deposit(1, CHEAP);
        _deposit(2, 1 ether);
        uint256 f = g.fee(address(nft));
        uint256 drawId = _request(bytes32("top"), 0);
        _revealHitting(drawId, 1); // do not draw the top itself
        g.settle(drawId);

        uint256 topShare = (f - f * 100 / 10_000) * 500 / 10_000;
        assertEq(_pot(), topShare, "pot not funded");

        // A takeover settles the outgoing holder's tenure and resets the pot.
        uint256 owedBefore = g.owed(bob);
        _deposit(3, 2 ether);
        assertEq(g.owed(bob) - owedBefore, topShare, "outgoing holder was not paid");
        assertEq(_pot(), 0, "pot did not reset for the new holder");
        _assertSolvent();
    }

    // ------------------------------------------------------------ the plumbing

    function test_everyWeiOfAFeeBecomesANamedLiability() public {
        // Three positions makes the equal split inexact, which is exactly where
        // a dropped remainder would hide: ScryGardener and ScrySilo both lost
        // emission this way before `carryScaled` existed.
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        _deposit(3, CHEAP);
        uint256 f = g.fee(address(nft));
        uint256 drawId = _request(bytes32("c1"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);
        vm.prank(cody);
        g.keepNft(1);

        uint256 liabilities =
            g.owed(alice) + g.owed(bob) + g.owed(cody) + g.houseOwed() + _pot() + g.feeCreditsOutstanding();
        // The fee plus the drawn position's returned backing are the only ETH
        // that became a liability in this window.
        assertEq(liabilities, f + CHEAP, "a wei went missing");
        _assertSolvent();
    }

    function test_claimPaysAndTheHouseSweepGoesToTheSink() public {
        _deposit(1, CHEAP);
        _deposit(2, CHEAP);
        uint256 drawId = _request(bytes32("sw"), 0);
        _revealHitting(drawId, 1);
        g.settle(drawId);
        vm.prank(cody);
        g.takeBid(1);

        uint256 codyOwed = g.owed(cody);
        uint256 balBefore = cody.balance;
        vm.prank(cody);
        assertEq(g.claim(), codyOwed);
        assertEq(cody.balance, balBefore + codyOwed);
        vm.prank(cody);
        vm.expectRevert(ScryGacha.NothingOwed.selector);
        g.claim();

        // Permissionless, but the destination is posted: the caller cannot point
        // the house's share at themselves.
        uint256 accrued = g.houseOwed();
        assertGt(accrued, 0);
        vm.prank(stranger);
        g.sweepHouse();
        assertEq(sink.balance, accrued);
        assertEq(g.houseOwed(), 0);
        vm.expectRevert(ScryGacha.NothingAccrued.selector);
        g.sweepHouse();
        _assertSolvent();
    }

    function test_rescueStrayRefusesAnEscrowedTokenAndFreesAPushedOne() public {
        _deposit(1, CHEAP);
        vm.expectRevert(ScryGacha.TokenIsEscrowed.selector);
        g.rescueStray(address(nft), 1, address(this));

        // A raw transferFrom bypasses every path here, so the token is loose:
        // owed to nobody, tracked by nothing.
        vm.prank(bob);
        nft.transferFrom(bob, address(g), 2);
        assertEq(g.positionOfToken(address(nft), 2), 0);
        vm.prank(stranger);
        vm.expectRevert(ScryGacha.OwnerOnly.selector);
        g.rescueStray(address(nft), 2, stranger);
        g.rescueStray(address(nft), 2, bob);
        assertEq(nft.ownerOf(2), bob);
    }

    function test_knobBoundsHold() public {
        vm.expectRevert(ScryGacha.HouseCutTooHigh.selector);
        g.setHouseBps(501, 100);
        vm.expectRevert(ScryGacha.TopShareTooHigh.selector);
        g.setTopKnobs(2_001, 1_000);
        vm.expectRevert(ScryGacha.ChoiceExceedsFinalize.selector);
        g.setWindows(2 days, 1 days);
        vm.expectRevert(ScryGacha.FinalizeTooLong.selector);
        g.setWindows(1 days, 31 days);
        vm.prank(stranger);
        vm.expectRevert(ScryGacha.OwnerOnly.selector);
        g.setHouseBps(0, 0);
        g.transferOwner(stranger);
        vm.expectRevert(ScryGacha.OwnerOnly.selector);
        g.setHouseBps(0, 0);
    }

    function test_theDeployBoundsTheDrawDelay() public {
        vm.expectRevert(ScryGacha.DelayOutOfRange.selector);
        new ScryGacha(1, sink);
        vm.expectRevert(ScryGacha.DelayOutOfRange.selector);
        new ScryGacha(129, sink);
        vm.expectRevert(ScryGacha.ZeroSink.selector);
        new ScryGacha(100, address(0));
    }
}
