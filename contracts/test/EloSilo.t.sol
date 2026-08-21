// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "../src/EloGranary.sol";
import "../src/EloSilo.sol";

/// The Silo spoils lockers (FARMING.md): seal OBOL for a posted season on
/// the cJEWEL weight ladder, earn MYRRH through the granary's daily cap;
/// breaking a seal early burns principal on a straight line from 50% to 0
/// and forfeits unpaid rewards. `forge test -vv` before any broadcast.
contract EloSiloTest is Test {
    SpoilsToken obol;
    SpoilsToken myrrh;
    EloGranary granary;
    EloSilo silo;

    address op = address(0xC0FFEE); // spoils distributor pre-rotation
    address alice = address(0xA11CE);
    address bob = address(0xB0B);

    uint256 constant RPS = 1e18; // 1 MYRRH/sec: easy math in tests
    uint256 constant MAX_BREAK = 5000; // 50% same-second break burn

    // tier menu posted in setUp
    uint256 constant T_WEEK = 0; // 7 days, 1x
    uint256 constant T_MONTH = 1; // 28 days, 2x

    function setUp() public {
        obol = new SpoilsToken("obol", "OBOL", 0, op);
        myrrh = new SpoilsToken("myrrh", "MYRRH", 0, op);
        granary = new EloGranary(myrrh); // steward = this
        silo = new EloSilo(granary, RPS, MAX_BREAK); // owner = this
        granary.setGrant(address(silo), 1_000_000e18);
        vm.prank(op);
        myrrh.setMinter(address(granary));

        silo.addTier(7 days, 10_000); // T_WEEK: 1x
        silo.addTier(28 days, 20_000); // T_MONTH: 2x
        silo.addBin(obol, 100); // bin 0: OBOL

        vm.prank(op);
        obol.mint(alice, 1000e18);
        vm.prank(op);
        obol.mint(bob, 1000e18);
        vm.prank(alice);
        obol.approve(address(silo), type(uint256).max);
        vm.prank(bob);
        obol.approve(address(silo), type(uint256).max);
    }

    function _seal(address who, uint256 tier, uint256 amt) internal returns (uint256 id) {
        vm.prank(who);
        id = silo.seal(0, tier, amt);
    }

    // -- sealing + accrual --------------------------------------------------

    function test_seal_parks_supply_and_accrues_by_posted_rate() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        assertEq(obol.balanceOf(alice), 900e18);
        assertEq(obol.balanceOf(address(silo)), 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 MYRRH accrued, sole sealer
        assertEq(silo.pendingReward(alice, id), 100e18);
        vm.prank(alice);
        silo.reap(id);
        assertEq(myrrh.balanceOf(alice), 100e18); // no lock-split in the silo
        assertEq(silo.pendingReward(alice, id), 0);
    }

    function test_weight_ladder_2x_tier_earns_double() public {
        // alice: 100 at 1x (weighted 100); bob: 50 at 2x (weighted 100)
        uint256 a = _seal(alice, T_WEEK, 100e18);
        uint256 b = _seal(bob, T_MONTH, 50e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 MYRRH to split evenly
        assertEq(silo.pendingReward(alice, a), 50e18);
        assertEq(silo.pendingReward(bob, b), 50e18);
    }

    function test_tier_weight_is_captured_menu_changes_never_reprice() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        silo.setTierEnabled(T_WEEK, false); // close the tier to new seals
        vm.expectRevert("tier closed");
        vm.prank(bob);
        silo.seal(0, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100);
        assertEq(silo.pendingReward(alice, id), 100e18); // still earning as posted
        vm.warp(vm.getBlockTimestamp() + 7 days);
        vm.prank(alice);
        silo.unseal(id); // and still exits fine
        assertEq(obol.balanceOf(alice), 1000e18);
    }

    function test_rate_change_settles_first_no_retroactive_repricing() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 @ 1/s
        silo.setRewardPerSecond(2e18); // massUpdateBins runs inside
        vm.warp(vm.getBlockTimestamp() + 100); // 200 @ 2/s
        assertEq(silo.pendingReward(alice, id), 300e18);
    }

    function test_empty_bin_accrues_nothing_and_never_bricks() public {
        vm.warp(vm.getBlockTimestamp() + 1000); // emission with zero sealed: no-op
        silo.massUpdateBins();
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 10);
        assertEq(silo.pendingReward(alice, id), 10e18); // clock starts at seal
    }

    function test_alloc_points_split_the_stream_across_bins() public {
        SpoilsToken salt = new SpoilsToken("salt", "SALT", 0, op);
        silo.addBin(salt, 100); // bin 1: even split with bin 0
        vm.prank(op);
        salt.mint(bob, 100e18);
        vm.prank(bob);
        salt.approve(address(silo), type(uint256).max);
        uint256 a = _seal(alice, T_WEEK, 100e18); // bin 0
        vm.prank(bob);
        uint256 b = silo.seal(1, T_WEEK, 100e18); // bin 1
        vm.warp(vm.getBlockTimestamp() + 100);
        assertEq(silo.pendingReward(alice, a), 50e18);
        assertEq(silo.pendingReward(bob, b), 50e18);
    }

    // -- maturity + unseal --------------------------------------------------

    function test_unseal_before_maturity_reverts() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 7 days - 1);
        vm.expectRevert("still sealed");
        vm.prank(alice);
        silo.unseal(id);
    }

    function test_unseal_at_maturity_pays_principal_and_rewards() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 7 days);
        vm.prank(alice);
        silo.unseal(id);
        assertEq(obol.balanceOf(alice), 1000e18); // principal whole
        assertEq(myrrh.balanceOf(alice), 7 days * 1e18); // full accrual
        vm.expectRevert("seal closed");
        vm.prank(alice);
        silo.unseal(id); // no double exit
    }

    function test_accrual_continues_past_maturity_until_unseal() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 7 days + 100); // still parked, still earning
        assertEq(silo.pendingReward(alice, id), (7 days + 100) * 1e18);
    }

    // -- the break ladder ---------------------------------------------------

    function test_break_at_seal_time_burns_half() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        assertEq(silo.breakFeeBps(alice, id), 5000);
        uint256 burnedBefore = obol.totalBurned();
        vm.prank(alice);
        silo.breakSeal(id);
        assertEq(obol.balanceOf(alice), 950e18); // half of 100 gone
        assertEq(obol.totalBurned(), burnedBefore + 50e18); // retired forever
        assertEq(obol.balanceOf(address(silo)), 0);
    }

    function test_break_fee_decays_linearly_to_zero() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 7 days / 2);
        assertEq(silo.breakFeeBps(alice, id), 2500); // halfway: 25%
        vm.prank(alice);
        silo.breakSeal(id);
        assertEq(obol.balanceOf(alice), 975e18);
    }

    function test_break_forfeits_rewards_never_minted() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100);
        assertEq(silo.pendingReward(alice, id), 100e18);
        vm.prank(alice);
        silo.breakSeal(id);
        assertEq(myrrh.balanceOf(alice), 0); // forfeited = less inflation
        assertEq(myrrh.totalMinted(), 0);
        assertEq(silo.pendingReward(alice, id), 0);
    }

    function test_break_after_maturity_refused_use_unseal() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 7 days);
        vm.expectRevert("matured - unseal");
        vm.prank(alice);
        silo.breakSeal(id);
    }

    function test_broken_seal_stops_earning_others_unaffected() public {
        uint256 a = _seal(alice, T_WEEK, 100e18);
        uint256 b = _seal(bob, T_WEEK, 100e18);
        vm.prank(alice);
        silo.breakSeal(a);
        vm.warp(vm.getBlockTimestamp() + 100);
        assertEq(silo.pendingReward(alice, a), 0);
        assertEq(silo.pendingReward(bob, b), 100e18); // bob now has the stream
    }

    // -- the granary clamp --------------------------------------------------

    function test_granary_clamp_carries_stash_then_pays() public {
        granary.setGrant(address(silo), 10e18); // tight daily cap
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 due, 10 mintable today
        vm.prank(alice);
        silo.reap(id);
        assertEq(myrrh.balanceOf(alice), 10e18);
        assertEq(silo.pendingReward(alice, id), 90e18); // stashed, not lost
        vm.warp(((vm.getBlockTimestamp() / 1 days) + 1) * 1 days); // next UTC day
        uint256 pending = silo.pendingReward(alice, id); // stash + fresh accrual
        vm.prank(alice);
        silo.reap(id);
        assertEq(myrrh.balanceOf(alice), 20e18); // another day's cap
        assertEq(silo.pendingReward(alice, id), pending - 10e18);
    }

    function test_unseal_under_clamp_credits_owed_then_claims() public {
        granary.setGrant(address(silo), 10e18);
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 7 days); // far more accrued than one day's cap
        uint256 due = silo.pendingReward(alice, id);
        vm.prank(alice);
        silo.unseal(id);
        assertEq(obol.balanceOf(alice), 1000e18); // principal never clamped
        assertEq(myrrh.balanceOf(alice), 10e18); // today's budget
        assertEq(silo.owedOf(alice), due - 10e18); // survives the closed seal
        vm.warp(((vm.getBlockTimestamp() / 1 days) + 1) * 1 days);
        vm.prank(alice);
        silo.claimOwed();
        assertEq(myrrh.balanceOf(alice), 20e18);
        assertEq(silo.owedOf(alice), due - 20e18); // keeps draining daily
    }

    function test_revoked_grant_never_bricks_exit() public {
        uint256 id = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 7 days);
        granary.revokeGrant(address(silo)); // granary.mint now reverts "no grant"
        uint256 due = silo.pendingReward(alice, id);
        vm.prank(alice);
        silo.unseal(id); // settle catches the revert; principal exits anyway
        assertEq(obol.balanceOf(alice), 1000e18);
        assertEq(myrrh.balanceOf(alice), 0);
        assertEq(silo.owedOf(alice), due); // rewards wait for a budget
        granary.setGrant(address(silo), 1_000_000e18);
        vm.prank(alice);
        silo.claimOwed();
        assertEq(myrrh.balanceOf(alice), due); // paid whole once re-granted
    }

    // -- the MYRRH self-bin (xJEWEL stakes the coin it emits) ---------------

    function test_myrrh_self_bin_parks_the_reward_coin() public {
        silo.addBin(myrrh, 25); // bin 1, a quarter of OBOL's weight
        vm.prank(op);
        vm.expectRevert("minter only"); // op rotated mint to the granary in setUp
        myrrh.mint(bob, 1e18);
        // bob earns MYRRH the only way anyone does: a granted distributor.
        granary.setGrant(address(this), 1000e18);
        granary.mint(bob, 100e18);
        uint256 a = _seal(alice, T_WEEK, 100e18); // alice: OBOL bin (alloc 100)
        vm.startPrank(bob);
        myrrh.approve(address(silo), type(uint256).max);
        uint256 b = silo.seal(1, T_WEEK, 100e18); // bob: MYRRH bin (alloc 25)
        vm.stopPrank();
        assertEq(myrrh.balanceOf(bob), 0); // parked out of circulation
        vm.warp(vm.getBlockTimestamp() + 125);
        assertEq(silo.pendingReward(alice, a), 100e18); // 100/125 of the stream
        assertEq(silo.pendingReward(bob, b), 25e18); // 25/125
        // breaking a MYRRH seal burns MYRRH - the self-bin sinks its own coin.
        // 125s into the 7-day seal the linear ladder reads
        // floor(5000 * (604800-125) / 604800) = 4998 bps, not the t=0 5000.
        assertEq(silo.breakFeeBps(bob, b), 4998);
        uint256 burnedBefore = myrrh.totalBurned();
        vm.prank(bob);
        silo.breakSeal(b);
        assertEq(myrrh.totalBurned(), burnedBefore + 49.98e18);
        assertEq(myrrh.balanceOf(bob), 50.02e18);
    }

    // -- positions + guardrails --------------------------------------------

    function test_multiple_seals_are_independent_positions() public {
        uint256 a = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 50);
        uint256 b = _seal(alice, T_MONTH, 100e18);
        assertEq(silo.sealCount(alice), 2);
        vm.warp(vm.getBlockTimestamp() + 7 days);
        vm.prank(alice);
        silo.unseal(a); // week seal matured
        vm.expectRevert("still sealed");
        vm.prank(alice);
        silo.unseal(b); // month seal still going
    }

    function test_zero_amount_and_unknown_tier_revert() public {
        vm.expectRevert(bytes("zero")); // 4-char literal needs the explicit type
        vm.prank(alice);
        silo.seal(0, T_WEEK, 0);
        vm.expectRevert(stdError.indexOOBError); // exact panic 0x32, not any-revert (TEST-AUDIT.md fix)
        vm.prank(alice);
        silo.seal(0, 99, 100e18); // out-of-range tier index
    }

    function test_knobs_are_owner_only_and_bins_unique() public {
        vm.startPrank(alice);
        vm.expectRevert("owner only");
        silo.addTier(1 days, 10_000);
        vm.expectRevert("owner only");
        silo.setRewardPerSecond(0);
        vm.stopPrank();
        vm.expectRevert("bin exists");
        silo.addBin(obol, 100);
        vm.expectRevert("break too high");
        new EloSilo(granary, RPS, 5001);
    }

    // -- the DeploySilo.s.sol production menu (TEST-AUDIT.md: "the test -----
    // -- tier menu != DeploySilo's menu - align it") ------------------------

    /// Post the EXACT tier menu DeploySilo.s.sol ships - 7d/1x, 30d/1.5x,
    /// 90d/2x, 180d/3x - alongside the suite's local menu, then seal one
    /// deposit in each posted tier and mature every one of them. Proves the
    /// production weights pass addTier's range gate, the durations mature,
    /// rewards pay, and principal exits whole - the menu that will actually
    /// exist on chain, not just the suite's stand-in.
    function test_deploy_silo_production_tier_menu_seals_and_matures() public {
        // the script's addTier calls, verbatim (DeploySilo.s.sol):
        silo.addTier(7 days, 10_000); // a week,      1x   -> tier 2
        silo.addTier(30 days, 15_000); // a month,     1.5x -> tier 3
        silo.addTier(90 days, 20_000); // a quarter,   2x   -> tier 4
        silo.addTier(180 days, 30_000); // half a year, 3x   -> tier 5
        assertEq(silo.tierCount(), 6); // appended alongside the suite's two

        uint256[4] memory durs = [uint256(7 days), 30 days, 90 days, 180 days];
        uint256[4] memory weights = [uint256(10_000), 15_000, 20_000, 30_000];
        uint256[4] memory ids;
        for (uint256 i = 0; i < 4; i++) {
            (uint256 dur, uint256 w, bool enabled) = silo.tiers(2 + i);
            assertEq(dur, durs[i]); // the posted season, byte for byte
            assertEq(w, weights[i]);
            assertTrue(enabled);
            ids[i] = _seal(alice, 2 + i, 100e18);
        }
        assertEq(obol.balanceOf(alice), 600e18); // 4 x 100 parked

        // headroom so the CLAMP is not what's under test here (the clamp has
        // its own suite section); 180 days of accrual outruns 1M/day.
        granary.setGrant(address(silo), type(uint256).max);
        uint256 t0 = vm.getBlockTimestamp();
        for (uint256 i = 0; i < 4; i++) {
            vm.warp(t0 + durs[i]); // each tier's exact maturity
            uint256 before = myrrh.balanceOf(alice);
            uint256 id = ids[i];
            vm.prank(alice);
            silo.unseal(id);
            assertGt(myrrh.balanceOf(alice), before); // the tier accrued and paid
        }
        assertEq(obol.balanceOf(alice), 1000e18); // all four principals whole
    }

    // -- late joiners + rebalance (TEST-AUDIT.md: mid-interval join, --------
    // -- setAllocPoint) -----------------------------------------------------

    /// A second sealer joining after accPerShare > 0 captures rewardDebt at
    /// entry (seal runs updateBin first): pending is 0 at the join and
    /// accrues only forward. Exact values at RPS 1/s, equal weighted stakes.
    function test_mid_interval_join_pending_zero_then_forward_only() public {
        uint256 a = _seal(alice, T_WEEK, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // alice alone: 100 accrued
        uint256 b = _seal(bob, T_WEEK, 100e18);
        assertEq(silo.pendingReward(bob, b), 0); // nothing retroactive
        assertEq(silo.pendingReward(alice, a), 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // now split evenly
        assertEq(silo.pendingReward(alice, a), 150e18); // 100 + 100*100/200
        assertEq(silo.pendingReward(bob, b), 50e18); //       100*100/200
    }

    /// setAllocPoint massUpdates first: the elapsed interval keeps the old
    /// split; only the future shifts. Exact per-interval values.
    function test_setAllocPoint_rebalances_forward_only() public {
        SpoilsToken salt = new SpoilsToken("salt", "SALT", 0, op);
        silo.addBin(salt, 100); // bin 1: even split at first
        vm.prank(op);
        salt.mint(bob, 100e18);
        vm.prank(bob);
        salt.approve(address(silo), type(uint256).max);
        uint256 a = _seal(alice, T_WEEK, 100e18); // bin 0
        vm.prank(bob);
        uint256 b = silo.seal(1, T_WEEK, 100e18); // bin 1
        vm.warp(vm.getBlockTimestamp() + 100); // 100 emitted at 100/100 alloc
        assertEq(silo.pendingReward(alice, a), 50e18);
        assertEq(silo.pendingReward(bob, b), 50e18);
        silo.setAllocPoint(1, 300); // total 400: bin 0 1/4, bin 1 3/4
        vm.warp(vm.getBlockTimestamp() + 100); // 100 emitted at the NEW split
        assertEq(silo.pendingReward(alice, a), 75e18); // 50 + 100*100/400
        assertEq(silo.pendingReward(bob, b), 125e18); // 50 + 100*300/400
    }

    // -- governance (TEST-AUDIT.md: every transferOwner, one test) ----------

    function test_transferOwner_old_refused_new_works() public {
        vm.expectRevert("zero owner");
        silo.transferOwner(address(0));
        vm.prank(bob);
        vm.expectRevert("owner only");
        silo.transferOwner(bob); // a stranger cannot take the keys
        silo.transferOwner(alice);
        assertEq(silo.owner(), alice);
        vm.expectRevert("owner only");
        silo.setRewardPerSecond(2e18); // the old owner (this) is refused
        vm.prank(alice);
        silo.setRewardPerSecond(2e18); // the new owner works
        assertEq(silo.rewardPerSecond(), 2e18);
    }
}
