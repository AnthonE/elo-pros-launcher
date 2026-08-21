// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "../src/EloHarvest.sol";
import "../src/EloGardener.sol";
import "../src/EloGranary.sol";
import "../src/EloSilo.sol";
import "../src/IERC20.sol";
import "../src/EloGacha.sol";
import {MockArbSys} from "./MockArbSys.sol";
import {MockToken} from "./MockToken.sol";

/// The invariant suite `AUDIT-2026-07-25.md` §4 said this repo did not have:
///
///   "Zero invariant tests, two fuzz tests repo-wide. Every numeric law is
///    pinned at hand-picked points. F1 and F2 are both conservation properties
///    an invariant test would have caught (`sum(user rewards) == emission over
///    window`; `lpSupply == sum(UserInfo.amount)`)."
///
/// Those two are §GARDENER and §SILO below, stated as the audit stated them.
/// The other two cover the Phase-1 contracts that hold money — the token the
/// whole economy is denominated in, and the contract the drop is paid from.
///
/// A handler-per-suite bounds the fuzzer's inputs so runs do all-revert into a
/// vacuous green; every handler keeps GHOST totals in plain Solidity, and the
/// invariant compares the contract's own accounting against them. Where a
/// handler swallows a revert it is because that call is legitimately refusable
/// (a cap, a clock, an empty vault), never to hide one.

// ═══════════════════════════════════════════════════════════════════════════
// SPOILS — supply conservation on the Phase-1 keystone
// ═══════════════════════════════════════════════════════════════════════════

contract SpoilsHandler is Test {
    SpoilsToken public coin;
    address public minter;
    address[3] public actors = [address(0xA1), address(0xA2), address(0xA3)];

    uint256 public ghostMinted;
    uint256 public ghostBurned;

    constructor(SpoilsToken c, address m) {
        coin = c;
        minter = m;
    }

    function mint(uint256 who, uint256 amount) external {
        address to = actors[who % 3];
        amount = bound(amount, 0, 1_000_000e18);
        vm.prank(minter);
        try coin.mint(to, amount) {
            ghostMinted += amount;
        } catch {
            // the cap is a legitimate refusal; nothing moved
        }
    }

    function burn(uint256 who, uint256 amount) external {
        address from = actors[who % 3];
        amount = bound(amount, 0, coin.balanceOf(from));
        vm.prank(from);
        coin.burn(amount);
        ghostBurned += amount;
    }

    function transfer(uint256 from_, uint256 to_, uint256 amount) external {
        address from = actors[from_ % 3];
        address to = actors[to_ % 3];
        amount = bound(amount, 0, coin.balanceOf(from));
        if (from == to) return;
        vm.prank(from);
        coin.transfer(to, amount);
    }
}

contract SpoilsTokenInvariants is Test {
    SpoilsToken coin;
    SpoilsHandler handler;
    address minter = address(0xD157);
    uint256 constant CAP = 5_000_000e18;

    function setUp() public {
        coin = new SpoilsToken("obol", "OBOL", CAP, minter);
        handler = new SpoilsHandler(coin, minter);
        targetContract(address(handler));
    }

    /// The harness, pinned before the invariants are worth reading — see the
    /// note on `HarvestInvariants.test_the_handler_can_actually_fund...`.
    function test_the_handler_actually_mints_burns_and_moves() public {
        handler.mint(0, 100e18);
        assertEq(handler.ghostMinted(), 100e18, "minting works");
        handler.transfer(0, 1, 40e18);
        assertEq(coin.balanceOf(handler.actors(1)), 40e18, "transfer works");
        handler.burn(1, 40e18);
        assertEq(handler.ghostBurned(), 40e18, "burning works");
    }

    /// The whole ledger in one line. `totalSupply` is what circulates,
    /// `totalMinted` is what was ever created, `totalBurned` is what was
    /// retired — and burning must NEVER reopen mint room (the cap is forever).
    function invariant_supply_is_minted_minus_burned() public view {
        assertEq(coin.totalSupply(), coin.totalMinted() - coin.totalBurned(), "supply identity");
    }

    /// A cap set at deploy is enforced forever, against any sequence.
    function invariant_the_cap_is_never_exceeded() public view {
        assertLe(coin.totalMinted(), CAP, "cap held");
    }

    /// The handler's own books and the token's must not drift: nothing is
    /// created or destroyed outside `mint`/`burn`.
    function invariant_no_supply_appears_from_nowhere() public view {
        assertEq(coin.totalMinted(), handler.ghostMinted(), "minted matches the handler");
        assertEq(coin.totalBurned(), handler.ghostBurned(), "burned matches the handler");
        uint256 held;
        for (uint256 i; i < 3; i++) {
            held += coin.balanceOf(handler.actors(i));
        }
        assertEq(held, coin.totalSupply(), "every coin is in someone's hands");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HARVEST — money conservation on the contract the drop is paid from
// ═══════════════════════════════════════════════════════════════════════════

contract HarvestHandler is Test {
    SpoilsToken public coin;
    EloHarvest public harvest;
    address public minter;
    address public sink = address(0x5153);
    address[3] public actors = [address(0xC1), address(0xC2), address(0xC3)];

    uint256 public ghostFunded;
    uint256 public ghostPaid;
    uint256 public ghostSwept;
    // the single-leaf root currently posted, so `claim` has something to prove
    address public rootWallet;
    uint256 public rootAmount;

    /// §M4's guarantee, tracked independently of the contract that makes it:
    /// the HIGHEST obligation stated since the SWEEP_DELAY window last lapsed.
    /// `sweepFloor()` must never sit below this while that window is open.
    ///
    /// It is a high-water mark and NOTHING draws it down but the window
    /// lapsing. It used to be drawn down by claims, mirroring a decrement the
    /// contract made in `claim` — and that decrement was the bug (2026-07-28):
    /// a payment under the CURRENT root discharged an obligation carried from
    /// the PREVIOUS one, which the contract has no way to know the claimant
    /// shared. Both sides of the mirror are gone.
    ///
    /// The suite had no invariant for this, and the two unit tests that pinned
    /// it both stopped after ONE retraction — which is exactly where the guard
    /// held. `postRoot` overwrote its carried floor with the retracted value, so
    /// a SECOND consecutive re-post walked the obligation to zero inside the
    /// window (audit 2026-07-27). A ghost is the shape that catches it, because
    /// the property is about a HISTORY the contract only keeps one number of.
    uint256 public ghostWindowFloor;
    uint256 public ghostWindowOpenedAt;

    constructor(SpoilsToken c, EloHarvest h, address m) {
        coin = c;
        harvest = h;
        minter = m;
    }

    function fund(uint256 amount) external {
        amount = bound(amount, 0, 100_000e18);
        vm.prank(minter);
        coin.mint(address(harvest), amount);
        ghostFunded += amount;
    }

    function postRoot(uint256 who, uint256 cumulative) external {
        rootWallet = actors[who % 3];
        rootAmount = bound(cumulative, 0, 50_000e18);
        // Roll the ghost window on the same boundary `sweepFloor` uses, then
        // carry the OUTGOING obligation into it — the value the contract is
        // about to move into `priorOwed`.
        if (block.timestamp >= ghostWindowOpenedAt + harvest.SWEEP_DELAY()) ghostWindowFloor = 0;
        uint256 outgoing = harvest.owedUnderRoot();
        if (outgoing > ghostWindowFloor) ghostWindowFloor = outgoing;
        harvest.postRoot(keccak256(abi.encodePacked(rootWallet, rootAmount)), rootAmount, "inv");
        ghostWindowOpenedAt = block.timestamp;
    }

    function claim() external {
        if (rootWallet == address(0)) return;
        uint256 before = harvest.claimed(rootWallet);
        bytes32[] memory proof = new bytes32[](0);
        try harvest.claim(rootWallet, rootAmount, proof) returns (uint256 paid) {
            ghostPaid += paid;
            // `ghostWindowFloor` is deliberately NOT touched here. A claim pays
            // out of the CURRENT root; the carried floor is the PREVIOUS root's
            // obligation, and discharging one with the other is what the
            // 2026-07-28 fix removed from `claim`. The floor releases when the
            // window lapses, which `postRoot` above is what rolls.
            // monotone by construction: `claimed` is assigned the cumulative,
            // and `claim` refuses a cumulative that is not strictly greater.
            assertGe(harvest.claimed(rootWallet), before, "claimed went backwards");
        } catch {
            // "nothing to claim" (already at this cumulative) or an empty vault
        }
    }

    function sweep(uint256 amount) external {
        amount = bound(amount, 0, coin.balanceOf(address(harvest)));
        try harvest.sweep(sink, amount) {
            ghostSwept += amount;
        } catch {
            // the posted floor refused it, which is the contract's whole job
        }
    }

    function warp(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 10 days));
    }
}

contract HarvestInvariants is Test {
    SpoilsToken coin;
    EloHarvest harvest;
    HarvestHandler handler;
    address minter = address(0xD157);

    function setUp() public {
        coin = new SpoilsToken("obol", "OBOL", 0, minter);
        harvest = new EloHarvest(IERC20(address(coin)));
        handler = new HarvestHandler(coin, harvest, minter);
        harvest.transferOperator(address(handler));
        targetContract(address(handler));
    }

    /// The harness itself, pinned. The first version of this suite moved the
    /// minter role to the handler while the handler still pranked as the OLD
    /// minter, so `fund` reverted 401 times out of 401 and
    /// `invariant_every_wei_is_accounted_for` passed on `0 == 0`. An invariant
    /// suite whose handler cannot act is the most convincing vacuous green
    /// there is — it prints call counts and a PASS. So every suite here proves
    /// its own handler moves state before the invariants are worth reading.
    function test_the_handler_can_actually_fund_claim_and_sweep() public {
        handler.fund(10_000e18);
        assertEq(handler.ghostFunded(), 10_000e18, "funding works");
        handler.postRoot(0, 4_000e18);
        handler.claim();
        assertEq(handler.ghostPaid(), 4_000e18, "claiming works");
        handler.sweep(1_000e18);
        assertGt(handler.ghostSwept(), 0, "sweeping works");
    }

    /// Nothing appears and nothing vanishes: every wei that was ever funded is
    /// either still in the contract, was claimed by a wallet, or was swept.
    function invariant_every_wei_is_accounted_for() public view {
        assertEq(
            coin.balanceOf(address(harvest)) + handler.ghostPaid() + handler.ghostSwept(),
            handler.ghostFunded(),
            "funded == held + paid + swept"
        );
    }

    /// The contract's own tally of what it paid matches what actually left.
    function invariant_totalClaimed_matches_what_was_paid() public view {
        assertEq(harvest.totalClaimed(), handler.ghostPaid(), "totalClaimed is honest");
        uint256 perWallet;
        for (uint256 i; i < 3; i++) {
            perWallet += harvest.claimed(handler.actors(i));
        }
        assertEq(perWallet, harvest.totalClaimed(), "per-wallet sums to the total");
    }

    /// §M4. The obligation is re-armed per root and only ever drawn DOWN by a
    /// payment — it can never exceed the total that was posted for it, which
    /// is the property `sweep` rests on.
    function invariant_owed_never_exceeds_the_posted_total() public view {
        assertLe(harvest.owedUnderRoot(), harvest.totalCommitted(), "owed <= posted");
        assertGe(harvest.sweepFloor(), harvest.owedUnderRoot(), "the floor is never below what is owed");
    }

    /// §M4's one UNCONDITIONAL promise, as an invariant rather than as two unit
    /// tests that both stop one call short of the bypass. While the delay window
    /// is open, the floor must cover the highest obligation stated inside it —
    /// however many roots have passed through since, and whatever they claimed
    /// to owe. Anything less means an obligation was walked down and swept.
    ///
    /// Note the pairing with the assertion above: that one says the floor covers
    /// the CURRENT root, which stayed true throughout the bug (0 >= 0). This one
    /// says it covers the window's HISTORY, which is where the guarantee lives.
    function invariant_the_window_holds_the_highest_stated_obligation() public view {
        if (block.timestamp < uint256(harvest.lastRootTime()) + harvest.SWEEP_DELAY()) {
            assertGe(
                harvest.sweepFloor(), handler.ghostWindowFloor(), "an obligation was walked down inside the window"
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GARDENER — F2, in the audit's own words: lpSupply == sum(UserInfo.amount)
// ═══════════════════════════════════════════════════════════════════════════

contract GardenerHandler is Test {
    SpoilsToken public lp;
    EloGardener public gardener;
    address public minter;
    address[3] public actors = [address(0xB1), address(0xB2), address(0xB3)];

    uint256 public ghostDonated;

    constructor(SpoilsToken l, EloGardener g, address m) {
        lp = l;
        gardener = g;
        minter = m;
        for (uint256 i; i < 3; i++) {
            vm.prank(m);
            l.mint(actors[i], 1_000e18);
            vm.prank(actors[i]);
            l.approve(address(g), type(uint256).max);
        }
    }

    function deposit(uint256 who, uint256 amount) external {
        address a = actors[who % 3];
        amount = bound(amount, 0, lp.balanceOf(a));
        vm.prank(a);
        gardener.deposit(0, amount);
    }

    function withdraw(uint256 who, uint256 amount) external {
        address a = actors[who % 3];
        (uint256 staked,,,) = gardener.userInfo(0, a);
        amount = bound(amount, 0, staked);
        vm.prank(a);
        gardener.withdraw(0, amount);
    }

    function emergencyWithdraw(uint256 who) external {
        address a = actors[who % 3];
        (uint256 staked,,,) = gardener.userInfo(0, a);
        if (staked == 0) return;
        vm.prank(a);
        gardener.emergencyWithdraw(0);
    }

    /// F2's actual attack: LP sent in by a PLAIN TRANSFER has no `UserInfo`
    /// row, so it can never be claimed — but under the pre-fix accounting it
    /// enlarged the denominator and diluted every real staker forever.
    function donate(uint256 who, uint256 amount) external {
        address a = actors[who % 3];
        amount = bound(amount, 0, lp.balanceOf(a));
        vm.prank(a);
        lp.transfer(address(gardener), amount);
        ghostDonated += amount;
    }

    /// `updatePool` is public, so the fuzzer gets to drive it too — that is F1's
    /// grief vector and the Gardener is the arm that survives it.
    function poke(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 2 days));
        gardener.updatePool(0);
    }
}

contract GardenerInvariants is Test {
    SpoilsToken lp;
    SpoilsToken reward;
    EloGranary granary;
    EloGardener gardener;
    GardenerHandler handler;
    address minter = address(0xD157);

    function setUp() public {
        lp = new SpoilsToken("seed", "SEED", 0, minter);
        reward = new SpoilsToken("obol", "OBOL", 0, minter);
        granary = new EloGranary(reward);
        gardener = new EloGardener(granary, 1e18, 6_700, block.timestamp + 90 days);
        granary.setGrant(address(gardener), 1_000_000e18);
        vm.prank(minter);
        reward.setMinter(address(granary));
        gardener.addPool(IERC20(address(lp)), 100);

        handler = new GardenerHandler(lp, gardener, minter);
        targetContract(address(handler));
    }

    /// The harness, pinned. A `stakedSupply == sum(amounts)` invariant over a
    /// handler that never successfully stakes is `0 == 0`.
    function test_the_handler_actually_stakes_withdraws_and_donates() public {
        handler.deposit(0, 100e18);
        assertEq(gardener.stakedSupply(0), 100e18, "deposit works");
        handler.donate(1, 50e18);
        assertEq(handler.ghostDonated(), 50e18, "donation works");
        assertEq(gardener.stakedSupply(0), 100e18, "and it changed no accounting - F2");
        handler.withdraw(0, 100e18);
        assertEq(gardener.stakedSupply(0), 0, "withdraw works");
    }

    /// THE F2 INVARIANT, as `AUDIT-2026-07-25.md` §4 wrote it. Before the fix
    /// the denominator was `lp.balanceOf(address(this))`, so a donation moved
    /// it and stranded 90% of a sole staker's rewards in the measured case.
    function invariant_staked_supply_equals_the_sum_of_user_amounts() public view {
        uint256 sum;
        for (uint256 i; i < 3; i++) {
            (uint256 amount,,,) = gardener.userInfo(0, handler.actors(i));
            sum += amount;
        }
        assertEq(gardener.stakedSupply(0), sum, "F2: internal accounting == the sum of the rows");
    }

    /// And the donation really is inert: it sits in the contract, outside the
    /// accounting, changing nothing. (A held balance BELOW stakedSupply would
    /// be the opposite failure — shares the contract cannot honour.)
    function invariant_donations_sit_outside_the_accounting() public view {
        assertGe(lp.balanceOf(address(gardener)), gardener.stakedSupply(0), "never short of what it owes");
    }
}

/// The invariant above proves the FIX's bookkeeping is self-consistent. It
/// would NOT have caught F2, and saying so is the point: pre-fix there was no
/// `stakedSupply` to be consistent with — `updatePool` read
/// `p.lp.balanceOf(address(this))` as the denominator, so a donation shrank
/// every staker's share of the emission stream permanently.
///
/// This is the shape that catches it, and it is the audit's other phrasing:
/// `sum(user rewards) == emission over window`. One stake, made once and never
/// moved, so `stakedSupply` is a constant and `accPerShare * stakedSupply` is
/// genuinely cumulative — then let the fuzzer donate and poke at will.
contract GardenerDonationHandler is Test {
    SpoilsToken public lp;
    EloGardener public gardener;
    address public donor = address(0xD00);

    uint256 public ghostDonated;

    constructor(SpoilsToken l, EloGardener g) {
        lp = l;
        gardener = g;
    }

    function donate(uint256 amount) external {
        amount = bound(amount, 0, lp.balanceOf(donor));
        vm.prank(donor);
        lp.transfer(address(gardener), amount);
        ghostDonated += amount;
    }

    function poke(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 3 days));
        gardener.updatePool(0);
    }

    function drift(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 3 days));
    }
}

contract GardenerDonationInvariants is Test {
    SpoilsToken lp;
    SpoilsToken reward;
    EloGranary granary;
    EloGardener gardener;
    GardenerDonationHandler handler;
    address minter = address(0xD157);
    address staker = address(0x57A);

    uint256 constant ACC = 1e12;
    uint256 constant RPS = 1e18;
    uint256 constant STAKE = 100e18;

    uint256 startTime;

    function setUp() public {
        lp = new SpoilsToken("seed", "SEED", 0, minter);
        reward = new SpoilsToken("obol", "OBOL", 0, minter);
        granary = new EloGranary(reward);
        gardener = new EloGardener(granary, RPS, 6_700, block.timestamp + 90 days);
        granary.setGrant(address(gardener), type(uint128).max);
        vm.prank(minter);
        reward.setMinter(address(granary));
        gardener.addPool(IERC20(address(lp)), 100);

        vm.prank(minter);
        lp.mint(staker, STAKE);
        vm.startPrank(staker);
        lp.approve(address(gardener), type(uint256).max);
        gardener.deposit(0, STAKE);
        vm.stopPrank();

        (,, uint256 lastRewardTime,) = gardener.pools(0);
        startTime = lastRewardTime;

        handler = new GardenerDonationHandler(lp, gardener);
        // hoisted deliberately: `vm.prank` arms the NEXT call, and Solidity
        // evaluates arguments first — so `lp.mint(handler.donor(), …)` spends
        // the prank on `handler.donor()` and mints as this contract. Same
        // eagerness the F6 ban is about; it cost a red here first.
        address d = handler.donor();
        vm.prank(minter);
        lp.mint(d, 10_000e18); // far more than the stake
        targetContract(address(handler));
    }

    /// THE F2 INVARIANT. Every second of emission since the stake is either
    /// attributed per share or held in the carry — and a donation, which no
    /// `UserInfo` row can ever claim, must not move that arithmetic at all.
    /// Pre-fix the denominator was the held BALANCE, so donating 900 against a
    /// 100 stake stranded 90% of the emission forever.
    function invariant_donations_cannot_dilute_the_emission_stream() public view {
        (,, uint256 lastRewardTime, uint256 accPerShare) = gardener.pools(0);
        assertEq(gardener.stakedSupply(0), STAKE, "the stake never moved");
        assertEq(
            accPerShare * gardener.stakedSupply(0) + gardener.carryScaled(0),
            ACC * RPS * (lastRewardTime - startTime),
            "F2: attributed + carried == emitted, whatever was donated"
        );
    }

    /// And the donor really did donate — a green above is worthless if the
    /// fuzzer never moved a token in.
    function test_the_handler_actually_donates_and_pokes() public {
        handler.poke(1 days);
        handler.donate(5_000e18);
        assertGt(handler.ghostDonated(), 0, "donation landed");
        assertGt(lp.balanceOf(address(gardener)), STAKE, "and the contract is holding it");
        handler.poke(1 days);
        (,, uint256 lastRewardTime, uint256 accPerShare) = gardener.pools(0);
        assertEq(
            accPerShare * STAKE + gardener.carryScaled(0),
            ACC * RPS * (lastRewardTime - startTime),
            "the donation changed nothing"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SILO — F1, in the audit's own words: sum(user rewards) == emission over window
// ═══════════════════════════════════════════════════════════════════════════

contract SiloHandler is Test {
    EloSilo public silo;

    constructor(EloSilo s) {
        silo = s;
    }

    /// F1 exactly: `updateBin` is PUBLIC, so anyone may drive it as often as
    /// they like. Without the scaled carry, every interval whose per-unit
    /// increment floors to zero advanced `lastRewardTime` and threw that
    /// second's emission away — permanently, for the cost of gas.
    function pokeFast(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 3));
        silo.updateBin(0);
    }

    function pokeSlow(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1 hours, 5 days));
        silo.updateBin(0);
    }

    /// Time passing with nobody poking must be indistinguishable from time
    /// passing with everybody poking — that is the whole content of F1.
    function drift(uint256 secs) external {
        vm.warp(block.timestamp + bound(secs, 1, 7 days));
    }
}

contract SiloEmissionInvariants is Test {
    SpoilsToken obol;
    SpoilsToken reward;
    EloGranary granary;
    EloSilo silo;
    SiloHandler handler;
    address minter = address(0xD157);
    address alice = address(0xA11CE);

    uint256 constant ACC = 1e12;
    // Deliberately large enough that (reward * ACC) / totalWeighted FLOORS TO
    // ZERO on a one-second interval — the operating point F1 lives at. At
    // 240 OBOL/day the audit put the threshold near 2.8e27 weighted units.
    uint256 constant RPS = 240e18 / uint256(1 days);
    uint256 constant SEALED = 3_000_000_000e18;

    uint256 startTime;
    uint256 weightedAtStart;

    function setUp() public {
        obol = new SpoilsToken("obol", "OBOL", 0, minter);
        reward = new SpoilsToken("myrrh", "MYRRH", 0, minter);
        granary = new EloGranary(reward);
        silo = new EloSilo(granary, RPS, 5_000);
        granary.setGrant(address(silo), type(uint128).max);
        vm.prank(minter);
        reward.setMinter(address(granary));

        silo.addTier(7 days, 10_000);
        silo.addBin(obol, 100);

        // ONE seal, never broken, so `totalWeighted` is a constant for the run
        // and `accPerShare * totalWeighted` really is cumulative emission.
        vm.prank(minter);
        obol.mint(alice, SEALED);
        vm.startPrank(alice);
        obol.approve(address(silo), type(uint256).max);
        silo.seal(0, 0, SEALED);
        vm.stopPrank();

        (,, uint256 lastRewardTime,, uint256 totalWeighted) = silo.bins(0);
        startTime = lastRewardTime;
        weightedAtStart = totalWeighted;
        assertGt(weightedAtStart, RPS * ACC, "the fixture must sit ABOVE the flooring threshold, or F1 cannot occur");

        handler = new SiloHandler(silo);
        targetContract(address(handler));
    }

    /// THE F1 INVARIANT. Every second of emission between `startTime` and the
    /// bin's last update is either attributed per weighted unit (`accPerShare`)
    /// or held in the scaled remainder (`carryScaled`) — never dropped. This is
    /// an EXACT equality, not a bound, because `totalWeighted` is constant for
    /// the run and the carry is what makes the arithmetic lossless.
    ///
    /// Pre-fix, `updateBin` computed `inc = scaled / totalWeighted`, wrote
    /// `accPerShare += inc`, and discarded `scaled - inc * totalWeighted`. At
    /// this fixture's size `inc` is 0 for short intervals, so the griefed arm
    /// accrued NOTHING while `lastRewardTime` marched forward.
    function invariant_no_second_of_emission_is_ever_destroyed() public view {
        (,, uint256 lastRewardTime, uint256 accPerShare, uint256 totalWeighted) = silo.bins(0);
        assertEq(totalWeighted, weightedAtStart, "the fixture kept totalWeighted constant");
        assertEq(
            accPerShare * totalWeighted + silo.carryScaled(0),
            ACC * RPS * (lastRewardTime - startTime),
            "F1: attributed + carried == emitted"
        );
    }

    /// The same fact from the staker's side, which is the one that pays: what
    /// alice is owed tracks wall-clock emission, whatever cadence the bin was
    /// poked at. Measured against `block.timestamp`, not `lastRewardTime` —
    /// `pendingReward` projects the unposted tail live, which is what a UI shows
    /// and therefore what a staker believes they are owed.
    ///
    /// Bounded rather than exact, in ONE direction only: the live projection
    /// recomputes its increment WITHOUT the carry, so it can trail by the
    /// un-attributable remainder. It must never run ahead.
    function invariant_the_sole_staker_is_owed_the_whole_window() public view {
        uint256 emitted = RPS * (block.timestamp - startTime);
        uint256 owed = silo.pendingReward(alice, 0);
        assertLe(owed, emitted, "never more than was emitted");
        // TWO flooring sources, both bounded by one weighted unit's worth, and
        // naming them is the point of the bound: (1) `carryScaled`, the posted
        // remainder the bin is holding, and (2) the view's OWN
        // `(reward * ACC) / totalWeighted`, which recomputes the unposted tail
        // without a carry of its own. On a 7-day window this slack is ~4e-6 of
        // the emission, so the assertion still says something.
        uint256 slack = (silo.carryScaled(0) + weightedAtStart) / ACC + 1;
        assertGe(owed + slack, emitted, "and never meaningfully less");
    }

    /// The Silo harness, pinned the same way the Harvest one is: if the bin is
    /// never actually updated, `lastRewardTime == startTime` and the emission
    /// identity above is `0 == 0` — a green that means nothing.
    function test_the_handler_actually_advances_the_bin() public {
        // FIRST, from a zero carry: one second at this fixture's size attributes
        // NOTHING per weighted unit. That is F1's operating point, and it is why
        // the carry has to exist — pre-fix, this second was simply gone.
        assertEq(silo.carryScaled(0), 0, "the fixture starts with no carry");
        (,, uint256 t0, uint256 acc0,) = silo.bins(0);
        handler.pokeFast(1);
        (,, uint256 t1, uint256 acc1,) = silo.bins(0);
        assertGt(t1, t0, "the clock advanced");
        assertEq(acc1, acc0, "and nothing was attributable per unit (this IS the flooring)");
        assertEq(silo.carryScaled(0), ACC * RPS * (t1 - t0), "so the whole second landed in the carry");

        // THEN a long interval, which does attribute — and the carry is spent
        // into it rather than discarded.
        handler.pokeSlow(1 days);
        (,, uint256 t2, uint256 acc2,) = silo.bins(0);
        assertGt(acc2, acc1, "a real interval attributes per unit");
        assertEq(
            acc2 * weightedAtStart + silo.carryScaled(0),
            ACC * RPS * (t2 - startTime),
            "and across BOTH pokes, every second is still accounted for"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GACHA — solvency, ETH conservation, and a segment tree that cannot drift
// ═══════════════════════════════════════════════════════════════════════════

/// The pool's own suite asserts solvency after each hand-written action. That
/// catches what I thought to write down. This drives the whole state machine —
/// deposit, re-price, withdraw, request, settle, expire, both resolutions, claim,
/// sweep — in whatever order the fuzzer picks, and asserts three properties that
/// hold no matter the order.
///
/// **No ghost totals are needed for the money one, which is why it is the strong
/// version:** the handler funds every actor once and nothing else ever mints
/// ETH, so the SUM of every balance in the system is a constant. Any wei the pool
/// created or destroyed shows up as a broken sum, with no bookkeeping for a bug
/// to hide behind.
contract InvNft {
    mapping(uint256 => address) public ownerOf;
    mapping(address => mapping(address => bool)) public isApprovedForAll;

    function mint(address to, uint256 id) external {
        ownerOf[id] = to;
    }

    function setApprovalForAll(address who, bool v) external {
        isApprovedForAll[msg.sender][who] = v;
    }

    function transferFrom(address from, address to, uint256 id) external {
        require(ownerOf[id] == from, "wrong from");
        require(msg.sender == from || isApprovedForAll[from][msg.sender], "not authorized");
        ownerOf[id] = to;
    }
}

contract GachaHandler is Test {
    EloGacha public g;
    InvNft public nft;
    MockArbSys public arb;
    /// The toll coin. The handler ARMS AND DISARMS it mid-run
    /// rather than picking a rail once, because the interesting states are the
    /// transitions: a pool that earned in ether, then in coin, then in ether
    /// again, with credits outstanding on both the whole time.
    MockToken public coin;
    uint256 public didTollDraw;

    address[2] public actors = [address(0xE1), address(0xE2)];
    address public sink;
    uint256 public nextToken = 1;
    uint256[] public positionIds;

    // Counted on SUCCESS only. `fail_on_revert = false` plus a handler that
    // try/catches every call means "reverts: 0" proves nothing about coverage —
    // a suite whose handler refuses everything reports a serene green. These are
    // logged by `afterInvariant` so vacuity is visible instead of assumed.
    uint256 public didDeposit;
    uint256 public didSettle;
    uint256 public didExpire;
    uint256 public didResolve;
    uint256 public didWithdraw;
    /// Why the last settle was refused. A zero in the coverage print needs a
    /// reason, not a shrug.
    string public lastSettleError;

    constructor(EloGacha g_, InvNft n_, MockArbSys a_, address sink_, MockToken coin_) {
        g = g_;
        nft = n_;
        arb = a_;
        sink = sink_;
        coin = coin_;
        for (uint256 i = 0; i < 2; i++) {
            vm.prank(actors[i]);
            nft.setApprovalForAll(address(g), true);
        }
    }

    function positionCount() external view returns (uint256) {
        return positionIds.length;
    }

    function deposit(uint256 who, uint256 backing) external {
        address a = actors[who % 2];
        backing = bound(backing, 0.01 ether, 2 ether);
        uint256 id = nextToken++;
        nft.mint(a, id);
        vm.prank(a);
        try g.deposit{value: backing}(address(nft), id) returns (uint256 posId) {
            positionIds.push(posId);
            didDeposit++;
        } catch {
            // a pending draw freezes the set; refusing is the design
        }
    }

    function reprice(uint256 idx, uint256 newBacking) external {
        if (positionIds.length == 0) return;
        uint256 posId = positionIds[idx % positionIds.length];
        (, address dep,,, uint256 old,,,,,,,,,) = g.positions(posId);
        newBacking = bound(newBacking, 0.01 ether, 2 ether);
        uint256 top = newBacking > old ? newBacking - old : 0;
        vm.prank(dep);
        try g.reprice{value: top}(posId, newBacking) {} catch {}
    }

    function withdraw(uint256 idx) external {
        if (positionIds.length == 0) return;
        uint256 posId = positionIds[idx % positionIds.length];
        (, address dep,,,,,,,,,,,,) = g.positions(posId);
        vm.prank(dep);
        try g.withdraw(posId) {
            didWithdraw++;
        } catch {}
    }

    /// Establishes its own precondition on purpose. An empty pool quotes no
    /// price, so a bare `if (fee == 0) return` made the whole draw path depend on
    /// the fuzzer happening to order deposit -> request -> settle inside one
    /// 32-call run. It mostly did not, and five invariants passed over ZERO
    /// draws. A handler that cannot reach the interesting state is not a bounded
    /// handler, it is a broken one.
    function requestDraw(uint256 who, uint256 salt, uint256 autoBid) external {
        address a = actors[who % 2];
        if (g.fee(address(nft)) == 0) {
            uint256 seedId = nextToken++;
            nft.mint(a, seedId);
            vm.prank(a);
            try g.deposit{value: 1 ether}(address(nft), seedId) returns (uint256 posId) {
                positionIds.push(posId);
                didDeposit++;
            } catch {
                return;
            }
        }
        // `quote`, never `fee`: the price and the ASSET are one question, and a
        // handler that asked only for the ETH number would send ether at a pool
        // charging coin and count the refusal as a legitimate revert.
        (address asset, uint256 price) = g.quote(address(nft));
        if (price == 0) return;
        uint256 bid = autoBid % 2 == 0 ? 0 : 1 ether;
        if (asset == address(0)) {
            vm.prank(a);
            try g.request{value: price}(address(nft), bytes32(salt), 0, 0, bid) {} catch {}
        } else {
            vm.prank(a);
            try g.request(address(nft), bytes32(salt), 0, 0, bid) {
                didTollDraw++;
            } catch {}
        }
    }

    /// Arm the toll at a bounded rate. The rate is kept small enough that a
    /// funded actor can afford a pull, and nonzero so arming actually arms.
    function armToll(uint256 rate, uint256 burnBps) external {
        rate = bound(rate, 1e15, 10e18);
        burnBps = bound(burnBps, 0, 2_000);
        try g.setToll(address(coin), rate) {} catch {}
        try g.setTollBurnBps(burnBps) {} catch {}
    }

    /// Disarm by RATE, which is the always-legal direction — the coin cannot
    /// move while the rail holds anything, and that guard is itself under test.
    function disarmToll() external {
        try g.setToll(address(coin), 0) {} catch {}
    }

    function claimToll(uint256 who) external {
        address a = actors[who % 2];
        vm.prank(a);
        try g.claimToll() {} catch {}
    }

    function sweepToll() external {
        try g.sweepHouseToll() {} catch {}
        try g.sweepBurn() {} catch {}
    }

    /// Advance the etched L2 clock to the reveal and settle. The clock only ever
    /// moves forward, as a chain's does.
    function settle() external {
        (,,,,,,,,, uint256 pending,,,,) = g.poolCard(address(nft));
        if (pending == 0) return;
        (,,,,, uint64 revealBlock,,,) = g.draws(pending);
        if (arb.l2() <= uint256(revealBlock)) arb.setL2Block(uint256(revealBlock) + 1);
        try g.settle(pending) {
            didSettle++;
        } catch Error(string memory why) {
            lastSettleError = why;
        } catch {
            lastSettleError = "non-string revert";
        }
    }

    /// The other branch: let the reveal age out of the window and forfeit.
    function expire() external {
        (,,,,,,,,, uint256 pending,,,,) = g.poolCard(address(nft));
        if (pending == 0) return;
        (,,,,, uint64 revealBlock,,,) = g.draws(pending);
        arb.setL2Block(uint256(revealBlock) + 300);
        try g.expire(pending) {
            didExpire++;
        } catch {}
    }

    function resolve(uint256 idx, bool takeTheBid) external {
        uint256 n = positionIds.length;
        if (n == 0) return;
        // Scan from a fuzzed offset for a position that is actually awaiting a
        // resolution. Blind indexing only worked when the pool held exactly two
        // positions; with more, the drawn one usually sat at an index nobody
        // looked at, and both settlement branches went untested.
        uint256 posId;
        address buyer;
        for (uint256 i = 0; i < n; i++) {
            uint256 candidate = positionIds[(idx + i) % n];
            (,, address b,,,,,,,,,, EloGacha.PositionStatus st,) = g.positions(candidate);
            if (st == EloGacha.PositionStatus.Drawn && b != address(0)) {
                posId = candidate;
                buyer = b;
                break;
            }
        }
        if (buyer == address(0)) return;
        vm.prank(buyer);
        if (takeTheBid) {
            try g.takeBid(posId) {
                didResolve++;
            } catch {}
        } else {
            try g.keepNft(posId) {
                didResolve++;
            } catch {}
        }
    }

    function harvest(uint256 idx) external {
        if (positionIds.length == 0) return;
        uint256[] memory one = new uint256[](1);
        one[0] = positionIds[idx % positionIds.length];
        (, address dep,,,,,,,,,,,,) = g.positions(one[0]);
        vm.prank(dep);
        try g.harvest(one) {} catch {}
    }

    function claim(uint256 who) external {
        address a = actors[who % 2];
        vm.prank(a);
        try g.claim() {} catch {}
    }

    function sweep() external {
        try g.sweepHouse() {} catch {}
    }
}

contract GachaInvariants is Test {
    EloGacha internal g;
    InvNft internal nft;
    MockArbSys internal arb;
    GachaHandler internal handler;
    MockToken internal coin;

    address internal constant ARBSYS = 0x0000000000000000000000000000000000000064;
    address internal sink = address(0x51C0);
    address internal constant DEAD = 0x000000000000000000000000000000000000dEaD;
    uint256 internal constant FUNDING = 1_000 ether;
    uint256 internal constant COIN_FUNDING = 1_000_000e18;
    uint256 internal startTotal;
    uint256 internal startCoinTotal;

    function setUp() public {
        // The clock, etched — and its window set EXPLICITLY, because vm.etch runs
        // no constructor (a field initializer reads 0 here, and every reveal then
        // looks expired while the expiry path passes for the wrong reason).
        vm.etch(ARBSYS, address(new MockArbSys()).code);
        arb = MockArbSys(ARBSYS);
        arb.setL2Block(20_000_000);
        arb.setWindow(256);

        g = new EloGacha(100, sink);
        nft = new InvNft();
        g.openPool(address(nft), 0.01 ether, 1_000, 8_500);

        coin = new MockToken("Scry", "SCRY");
        handler = new GachaHandler(g, nft, arb, sink, coin);
        vm.deal(address(handler), FUNDING);
        for (uint256 i = 0; i < 2; i++) {
            vm.deal(handler.actors(i), FUNDING);
            coin.mint(handler.actors(i), COIN_FUNDING);
            vm.prank(handler.actors(i));
            coin.approve(address(g), type(uint256).max);
        }
        // The handler owns the knobs so the fuzzer can arm and disarm the toll
        // mid-run. It is the LAST thing setUp does: `openPool` above needs the
        // owner seat, and nothing after this point does.
        g.transferOwner(address(handler));
        // Seed two positions so a draw is reachable from the fuzzer's first call.
        handler.deposit(0, 1 ether);
        handler.deposit(1, 1 ether);
        // Measured AFTER seeding, never assumed from FUNDING * 3: the baseline of
        // a conservation invariant must be observed, or it is just another claim.
        startTotal = address(g).balance + address(handler).balance + sink.balance
            + handler.actors(0).balance + handler.actors(1).balance;
        startCoinTotal = _coinEverywhere();

        targetContract(address(handler));
    }

    /// Every wei that exists still exists somewhere. The handler funds the actors
    /// once and nothing mints ETH, so this sum cannot move — and any wei the pool
    /// invented or destroyed breaks it with no ghost accounting to argue with.
    function invariant_no_wei_is_created_or_destroyed() public view {
        uint256 total = address(g).balance + address(handler).balance + sink.balance;
        for (uint256 i = 0; i < 2; i++) {
            total += handler.actors(i).balance;
        }
        assertEq(total, startTotal, "ETH appeared or vanished");
    }

    /// Every unit of the toll coin still exists somewhere — including at the
    /// BURN address, which is why it is summed here rather than treated as gone.
    /// The burn makes coins unspendable; it does not destroy them, and an
    /// invariant that pretended otherwise would be asserting the overstatement
    /// the toll rail's own notes warn against.
    function invariant_no_toll_unit_is_created_or_destroyed() public view {
        assertEq(_coinEverywhere(), startCoinTotal, "toll coin appeared or vanished");
    }

    /// The toll rail's own solvency, and the exact twin of the ETH one below. It
    /// is a separate assertion because the rails are separately fundable: a bug
    /// that paid a toll credit out of escrow would leave ETH solvency untouched
    /// and every ETH invariant green.
    function invariant_the_toll_rail_can_always_pay_what_it_owes() public view {
        assertGe(coin.balanceOf(address(g)), g.accountedToll(), "toll insolvent: liabilities exceed balance");
    }

    function _coinEverywhere() internal view returns (uint256 total) {
        total = coin.balanceOf(address(g)) + coin.balanceOf(address(handler)) + coin.balanceOf(sink)
            + coin.balanceOf(DEAD);
        for (uint256 i = 0; i < 2; i++) {
            total += coin.balanceOf(handler.actors(i));
        }
    }

    /// The claim the whole design rests on: the pool can always pay what it owes.
    /// `accountedEth` is the sum of its named liabilities — backings, escrowed
    /// fees, credits, the house's cut, the top pot, undistributed fee credits.
    function invariant_the_pool_can_always_pay_what_it_owes() public view {
        assertGe(address(g).balance, g.accountedEth(), "insolvent: liabilities exceed balance");
    }

    /// The segment tree is a second copy of the weight sum, and a draw reads the
    /// TREE while the price reads the sum. If they ever disagree the pool prices
    /// one pool and draws from another, which no per-action test would notice.
    function invariant_the_tree_never_drifts_from_the_weight_sum() public view {
        (,,,,,, uint256 totalWeight,,,,,,,) = g.poolCard(address(nft));
        assertEq(g.treeRoot(address(nft)), totalWeight, "tree root != totalWeight");
    }

    /// The harness, pinned before the invariants above are worth reading — the
    /// same guard `SpoilsTokenInvariants` and `HarvestInvariants` keep. Written
    /// because the first version of this suite reported five serene greens while
    /// the fuzzer had performed **zero** draws, settles and resolutions: the
    /// handler try/catches everything, so a handler that refuses every call still
    /// prints `reverts: 0`. This test fails the moment a path stops working.
    function test_the_handler_can_actually_run_a_whole_draw() public {
        uint256 before = handler.didDeposit();
        handler.deposit(0, 1 ether);
        handler.deposit(1, 2 ether);
        assertEq(handler.didDeposit() - before, 2, "deposits do not work");

        handler.requestDraw(0, 42, 0);
        (,,,,,,,,, uint256 pending,,,,) = g.poolCard(address(nft));
        assertGt(pending, 0, "a draw was never opened");

        handler.settle();
        assertEq(handler.didSettle(), 1, "settle does not work");

        handler.resolve(0, false);
        handler.resolve(1, true);
        assertGt(handler.didResolve(), 0, "neither resolution branch works");

        handler.deposit(0, 1 ether);
        handler.requestDraw(1, 7, 1);
        handler.expire();
        assertEq(handler.didExpire(), 1, "expire does not work");

        handler.withdraw(0);
        handler.claim(0);
        handler.sweep();
    }

    /// Coverage, printed rather than asserted. Asserting it would be flaky (a run
    /// that happens never to draw is legitimate), but a reader with `-vv` can see
    /// whether the fuzzer actually reached the draw and both resolution branches —
    /// which is the difference between these invariants holding and them being
    /// vacuous. Verified by hand on a 64x32 run: deposits, settles, expiries,
    /// resolutions and withdrawals were all non-zero.
    function afterInvariant() public view {
        console.log("gacha handler coverage over this run:");
        console.log("  deposits ", handler.didDeposit());
        console.log("  settles  ", handler.didSettle());
        console.log("  expiries ", handler.didExpire());
        console.log("  resolves ", handler.didResolve());
        console.log("  withdraws", handler.didWithdraw());
        console.log("  toll draws", handler.didTollDraw());
        if (handler.didSettle() == 0) {
            console.log("  no settle happened; last refusal:", handler.lastSettleError());
        }
    }

    /// An empty POOL is empty in every representation of itself. Note what this
    /// deliberately does not say: `backingTotal` may be non-zero here, because a
    /// DRAWN position has left the pool while its backing stays escrowed until
    /// someone resolves it. Asserting otherwise was this suite's first red, and
    /// the contract was right.
    function invariant_an_empty_pool_has_no_weight() public view {
        (,,,,, uint256 count,,,,,,,,) = g.poolCard(address(nft));
        if (count == 0) {
            assertEq(g.treeRoot(address(nft)), 0, "no positions but the tree holds weight");
        }
    }

    /// The escrow counter against the positions themselves: every wei of
    /// `backingTotal` belongs to a position that is still Active or Drawn. This is
    /// the one that would catch a missed or doubled decrement on any of the five
    /// paths that move backing (withdraw, keep, bid, reprice, deposit).
    function invariant_backing_matches_the_positions_holding_it() public view {
        uint256 sum;
        uint256 n = handler.positionCount();
        for (uint256 i = 0; i < n; i++) {
            (,,,, uint256 backing,,,,,,,, EloGacha.PositionStatus st,) = g.positions(handler.positionIds(i));
            if (st == EloGacha.PositionStatus.Active || st == EloGacha.PositionStatus.Drawn) {
                sum += backing;
            }
        }
        assertEq(g.backingTotal(), sum, "escrowed backing does not match the positions holding it");
    }
}
