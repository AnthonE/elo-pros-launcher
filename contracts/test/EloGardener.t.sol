// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "./MockToken.sol";
import "../src/EloGarden.sol";
import "../src/EloGranary.sol";
import "../src/EloGardener.sol";

/// The Gardener farm + the Granary mint hub (FARMING.md): stake SEED, earn the
/// reward coin by the posted schedule. **In production that is the MYRRH/OBOL
/// Garden's SEED paying MYRRH** — one Garden, both sides house-minted, because
/// a `EloGarden` never holds canonical SCRY (audit F5), and MYRRH because the
/// Garden is its only source since 2026-07-26 (§3a; era-0 6,000/day). This harness
/// stakes a stand-in `EloGarden(OBOL, mock-SCRY)` on purpose: the Gardener
/// only ever sees an ERC-20 LP and an ERC-20 reward, so which pair and which
/// coin is a deploy-time choice these tests must not weld. (The header used to
/// state OBOL/SCRY as the production pair — that is the pre-F5 farm, and it
/// never shipped.) 67% of harvests cliff-lock; the
/// DFK withdrawal-slash ladder pays permanent liquidity to 0xdEaD; the
/// granary clamps every mint to a posted daily cap and the shortfall is
/// carried, never lost. `forge test -vv` before any broadcast.
contract EloGardenerTest is Test {
    SpoilsToken obol;
    SpoilsToken myrrh;
    MockToken tscry; // stands in for SCRY, faucet-funded for tests only
    EloGarden garden;
    EloGranary granary;
    EloGardener gardener;

    address op = address(0xC0FFEE); // spoils distributor pre-rotation
    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    address constant DEAD = 0x000000000000000000000000000000000000dEaD;

    uint256 constant RPS = 1e18; // 1 MYRRH/sec: easy math in tests
    uint256 constant LOCK_BPS = 6700; // the FINAL posted number
    uint256 unlockAt;
    uint256 seedBal; // alice's SEED after addLiquidity

    function setUp() public {
        obol = new SpoilsToken("obol", "OBOL", 0, op);
        myrrh = new SpoilsToken("myrrh", "MYRRH", 0, op);
        tscry = new MockToken("stand-in reserve", "tSCRY");
        garden = new EloGarden(IERC20(address(obol)), IERC20(address(tscry)));

        unlockAt = vm.getBlockTimestamp() + 90 days;
        granary = new EloGranary(myrrh); // steward = this
        gardener = new EloGardener(granary, RPS, LOCK_BPS, unlockAt); // owner = this
        granary.setGrant(address(gardener), 1000e18);
        vm.prank(op);
        myrrh.setMinter(address(granary));
        gardener.addPool(IERC20(address(garden)), 100); // pid 0

        // alice LPs the OBOL/SCRY garden to get SEED shares
        vm.prank(op);
        obol.mint(alice, 500e18);
        vm.startPrank(alice);
        tscry.faucet(); // 1000e18
        obol.approve(address(garden), type(uint256).max);
        tscry.approve(address(garden), type(uint256).max);
        garden.addLiquidity(400e18, 400e18, 0, type(uint256).max);
        garden.approve(address(gardener), type(uint256).max);
        vm.stopPrank();
        seedBal = garden.balanceOf(alice); // sqrt(400e18*400e18) - 1000
    }

    function _stake(address who, uint256 amt) internal {
        vm.prank(who);
        gardener.deposit(0, amt);
    }

    // -- emission + the lock split ------------------------------------------

    function test_accrual_splits_67_locked_33_paid() public {
        _stake(alice, 100e18); // round stake: accPerShare math lands exact
        vm.warp(vm.getBlockTimestamp() + 100); // 100 MYRRH accrued
        assertEq(gardener.pendingMyrrh(0, alice), 100e18);
        vm.prank(alice);
        gardener.deposit(0, 0); // harvest
        assertEq(myrrh.balanceOf(alice), 33e18);
        assertEq(gardener.lockedOf(alice), 67e18);
        assertEq(gardener.pendingMyrrh(0, alice), 0);
    }

    function test_granary_clamp_carries_stash_and_never_resplits_it() public {
        granary.setGrant(address(gardener), 10e18); // tight daily cap
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100);
        vm.prank(alice);
        gardener.deposit(0, 0); // immediate 33, cap 10 -> paid 10, stash 23
        assertEq(myrrh.balanceOf(alice), 10e18);
        (,, uint256 stash,) = gardener.userInfo(0, alice);
        assertEq(stash, 23e18);
        assertEq(gardener.lockedOf(alice), 67e18);

        gardener.setRewardPerSecond(0); // freeze fresh accrual
        vm.warp(vm.getBlockTimestamp() + 1 days); // granary day rolls
        granary.setGrant(address(gardener), 1000e18);
        vm.prank(alice);
        gardener.deposit(0, 0); // stash pays out WHOLE: no second lock cut
        assertEq(myrrh.balanceOf(alice), 33e18);
        (,, stash,) = gardener.userInfo(0, alice);
        assertEq(stash, 0);
        assertEq(gardener.lockedOf(alice), 67e18); // unchanged
    }

    function test_revoked_grant_defers_rewards_never_traps_principal() public {
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 5 weeks); // 3,024,000 MYRRH accrued
        granary.revokeGrant(address(gardener)); // granary.mint now reverts "no grant"
        uint256 slash = (100e18 * 1) / 10_000; // patient tier still applies
        vm.prank(alice);
        gardener.withdraw(0, 100e18); // settle catches the revert; principal exits anyway
        assertEq(garden.balanceOf(alice), seedBal - slash);
        assertEq(garden.balanceOf(DEAD), slash);
        assertEq(myrrh.balanceOf(alice), 0); // nothing minted while revoked
        (,, uint256 stash,) = gardener.userInfo(0, alice);
        assertEq(stash, 997_920e18); // the 33% immediate, carried whole
        assertEq(gardener.lockedOf(alice), 2_026_080e18); // the 67% cliff-locks as usual

        // the carry pays the moment a grant returns - nothing was lost
        granary.setGrant(address(gardener), type(uint256).max);
        vm.prank(alice);
        gardener.deposit(0, 0);
        assertEq(myrrh.balanceOf(alice), 997_920e18);
        (,, stash,) = gardener.userInfo(0, alice);
        assertEq(stash, 0);
    }

    function test_rotated_minter_defers_rewards_never_traps_principal() public {
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 MYRRH accrued
        granary.rotateMinterAway(bob); // granary can mint nothing now
        vm.prank(alice);
        gardener.withdraw(0, 100e18); // "minter only" is caught the same way
        assertEq(myrrh.balanceOf(alice), 0);
        (,, uint256 stash,) = gardener.userInfo(0, alice);
        assertEq(stash, 33e18);
        assertEq(gardener.lockedOf(alice), 67e18);
    }

    // -- the DFK slash ladder ----------------------------------------------

    function test_slash_ladder_matches_the_older_harmony_schedule() public view {
        assertEq(gardener.withdrawFeeBps(0), 2500); // same block 25%
        assertEq(gardener.withdrawFeeBps(30 minutes), 800);
        assertEq(gardener.withdrawFeeBps(2 hours), 400);
        assertEq(gardener.withdrawFeeBps(2 days), 200);
        assertEq(gardener.withdrawFeeBps(4 days), 100);
        assertEq(gardener.withdrawFeeBps(1 weeks), 50);
        assertEq(gardener.withdrawFeeBps(3 weeks), 25);
        assertEq(gardener.withdrawFeeBps(5 weeks), 1); // 0.01% floor
    }

    function test_same_block_exit_donates_25pct_permanent_liquidity() public {
        _stake(alice, seedBal);
        uint256 slash = (seedBal * 2500) / 10_000;
        vm.prank(alice);
        gardener.withdraw(0, seedBal);
        assertEq(garden.balanceOf(DEAD), slash);
        assertEq(garden.balanceOf(alice), seedBal - slash);
    }

    function test_patient_exit_pays_one_bp() public {
        _stake(alice, seedBal);
        vm.warp(vm.getBlockTimestamp() + 5 weeks);
        uint256 slash = (seedBal * 1) / 10_000;
        vm.prank(alice);
        gardener.withdraw(0, seedBal);
        assertEq(garden.balanceOf(DEAD), slash);
        assertEq(garden.balanceOf(alice), seedBal - slash);
    }

    function test_new_deposit_resets_the_slash_clock() public {
        _stake(alice, 300e18);
        vm.warp(vm.getBlockTimestamp() + 5 weeks);
        _stake(alice, 1e18); // DFK rule: the clock restarts
        uint256 staked = 301e18;
        uint256 slash = (staked * 2500) / 10_000;
        vm.prank(alice);
        gardener.withdraw(0, staked); // same block as the top-up
        assertEq(garden.balanceOf(DEAD), slash);
        assertEq(garden.balanceOf(alice), seedBal - slash); // user side of the same split (TEST-AUDIT.md fix)
    }

    // -- the cliff ----------------------------------------------------------

    function test_locked_rewards_cliff_then_pay() public {
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100);
        vm.prank(alice);
        gardener.deposit(0, 0);
        assertEq(gardener.lockedOf(alice), 67e18);
        vm.prank(alice);
        vm.expectRevert("still locked");
        gardener.claimLocked();
        vm.warp(unlockAt);
        vm.prank(alice);
        uint256 paid = gardener.claimLocked();
        assertEq(paid, 67e18);
        assertEq(gardener.lockedOf(alice), 0);
        assertEq(myrrh.balanceOf(alice), 100e18);
        vm.prank(alice);
        vm.expectRevert("nothing locked");
        gardener.claimLocked();
    }

    // -- emergency exit ------------------------------------------------------

    function test_emergency_forfeits_pending_keeps_locked_pays_slash() public {
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100);
        vm.prank(alice);
        gardener.deposit(0, 0); // locks 67, pays 33
        vm.warp(vm.getBlockTimestamp() + 50); // 50 more accrue, unharvested
        uint256 slash = (100e18 * 800) / 10_000; // 150s since deposit: 8%
        vm.prank(alice);
        gardener.emergencyWithdraw(0);
        assertEq(myrrh.balanceOf(alice), 33e18); // forfeited the 50
        assertEq(gardener.lockedOf(alice), 67e18); // earned lock survives
        (uint256 amt,, uint256 stash,) = gardener.userInfo(0, alice);
        assertEq(amt, 0);
        assertEq(stash, 0);
        assertEq(garden.balanceOf(DEAD), slash);
        assertEq(garden.balanceOf(alice), seedBal - slash);
    }

    // -- allocation points across pools --------------------------------------

    function test_two_pools_split_emission_by_alloc() public {
        // pid 1: tSCRY itself as a stand-in LP token, alloc 50 (1/3 of stream)
        gardener.addPool(IERC20(address(tscry)), 50);
        vm.startPrank(bob);
        tscry.faucet();
        tscry.approve(address(gardener), type(uint256).max);
        gardener.deposit(1, 100e18);
        vm.stopPrank();
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 300);
        assertEq(gardener.pendingMyrrh(0, alice), 200e18); // 300 * 100/150
        assertEq(gardener.pendingMyrrh(1, bob), 100e18); // 300 *  50/150
    }

    function test_duplicate_pool_refused() public {
        vm.expectRevert("pool exists");
        gardener.addPool(IERC20(address(garden)), 10);
    }

    // -- granary ------------------------------------------------------------

    function test_granary_grants_clamp_and_roll_daily() public {
        granary.setGrant(alice, 10e18);
        vm.prank(alice);
        uint256 minted = granary.mint(alice, 4e18);
        assertEq(minted, 4e18);
        assertEq(granary.availableToday(alice), 6e18);
        vm.prank(alice);
        minted = granary.mint(alice, 100e18); // clamped, not reverted
        assertEq(minted, 6e18);
        assertEq(granary.availableToday(alice), 0);
        vm.warp(vm.getBlockTimestamp() + 1 days); // the day rolls
        assertEq(granary.availableToday(alice), 10e18);
    }

    function test_granary_refuses_the_ungranted_and_survives_revoke() public {
        vm.prank(bob);
        vm.expectRevert("no grant");
        granary.mint(bob, 1e18);
        granary.setGrant(bob, 5e18);
        granary.revokeGrant(bob);
        vm.prank(bob);
        vm.expectRevert("no grant");
        granary.mint(bob, 1e18);
    }

    function test_granary_steward_mint_and_rotation_away() public {
        granary.stewardMint(alice, 7e18); // ledger mints, unbudgeted
        assertEq(myrrh.balanceOf(alice), 7e18);
        vm.prank(bob);
        vm.expectRevert("steward only");
        granary.stewardMint(bob, 1e18);
        granary.rotateMinterAway(bob); // escape hatch
        assertEq(myrrh.minter(), bob);
        granary.setGrant(alice, 10e18);
        vm.prank(alice);
        vm.expectRevert("minter only"); // granary can mint nothing now
        granary.mint(alice, 1e18);
    }

    // -- operator gates ------------------------------------------------------

    function test_owner_gates_hold() public {
        vm.startPrank(bob);
        vm.expectRevert("owner only");
        gardener.addPool(IERC20(address(tscry)), 1);
        vm.expectRevert("owner only");
        gardener.setRewardPerSecond(1);
        vm.expectRevert("owner only");
        gardener.setLockBps(1);
        vm.stopPrank();
        vm.expectRevert("lock too high");
        gardener.setLockBps(9001);
    }

    function test_constructor_guards() public {
        vm.expectRevert("lock too high");
        new EloGardener(granary, RPS, 9001, vm.getBlockTimestamp() + 1 days);
        vm.expectRevert("unlock in past");
        new EloGardener(granary, RPS, 100, vm.getBlockTimestamp());
    }

    // -- proportional sharing (TEST-AUDIT.md HIGH: the split math that ------
    // -- divides money between concurrent stakers in ONE pool) --------------

    /// bob LPs the same 1:1 garden so he holds SEED for pool 0. Reserves are
    /// 400/400 after setUp, so addLiquidity(100,100) mints exactly 100e18.
    function _seedBob() internal {
        vm.prank(op);
        obol.mint(bob, 100e18);
        vm.startPrank(bob);
        tscry.faucet(); // 1000e18
        obol.approve(address(garden), type(uint256).max);
        tscry.approve(address(garden), type(uint256).max);
        garden.addLiquidity(100e18, 100e18, 0, type(uint256).max);
        garden.approve(address(gardener), type(uint256).max);
        vm.stopPrank();
        assertEq(garden.balanceOf(bob), 100e18);
    }

    /// A stakes 100 alone for 100s (the whole stream), then B joins with an
    /// equal stake for the next 100s (they halve it). Exact values at RPS 1/s:
    ///   interval 1: A = T * RPS               = 100e18
    ///   interval 2: A = B = T * RPS * X/(2X)  =  50e18 each
    /// The late joiner's rewardDebt is captured at entry (deposit runs
    /// updatePool first), so B's pending is 0 the moment it joins - it can
    /// never farm the interval it was not staked for.
    function test_two_stakers_one_pool_exact_proportional_split() public {
        _seedBob();
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // interval 1: alice alone
        _stake(bob, 100e18); // join settles the pool first
        assertEq(gardener.pendingMyrrh(0, bob), 0); // rewardDebt captured at join
        assertEq(gardener.pendingMyrrh(0, alice), 100e18); // interval 1 is all alice's
        vm.warp(vm.getBlockTimestamp() + 100); // interval 2: 200e18 staked total
        assertEq(gardener.pendingMyrrh(0, alice), 150e18); // 100 + 100*100/200
        assertEq(gardener.pendingMyrrh(0, bob), 50e18); //       100*100/200

        // and the split survives an actual settle: harvest both, exact mints
        vm.prank(alice);
        gardener.deposit(0, 0);
        vm.prank(bob);
        gardener.deposit(0, 0);
        assertEq(myrrh.balanceOf(alice), 49.5e18); // 33% of 150 immediate
        assertEq(gardener.lockedOf(alice), 100.5e18); // 67% of 150 locked
        assertEq(myrrh.balanceOf(bob), 16.5e18); // 33% of 50
        assertEq(gardener.lockedOf(bob), 33.5e18); // 67% of 50
    }

    /// Partial withdraw settles rewards for the WHOLE prior stake, then the
    /// remainder accrues at the reduced share. Exact: A and B stake 100 each
    /// from t0; after 100s A withdraws 50 (both are owed 50); for the next
    /// 300s A holds 50 of 150 staked -> +100, B holds 100 of 150 -> +200.
    /// (300s chosen so 300e18 * 1e12 / 150e18 divides with no remainder.)
    function test_partial_withdraw_settles_then_accrues_at_reduced_share() public {
        _seedBob();
        _stake(alice, 100e18);
        _stake(bob, 100e18); // same block: both from t0
        vm.warp(vm.getBlockTimestamp() + 100); // 100 emitted: 50 each
        uint256 slash = (50e18 * 800) / 10_000; // 100s since deposit: the 8% rung
        vm.prank(alice);
        gardener.withdraw(0, 50e18);
        // rewards settled for the full pre-withdraw stake: 50 fresh -> 33/67
        assertEq(myrrh.balanceOf(alice), 16.5e18);
        assertEq(gardener.lockedOf(alice), 33.5e18);
        assertEq(gardener.pendingMyrrh(0, alice), 0);
        // principal: 50 out, 4 slashed to DEAD, 46 back
        assertEq(garden.balanceOf(DEAD), slash);
        assertEq(garden.balanceOf(alice), seedBal - 100e18 + 46e18);
        (uint256 amt,,,) = gardener.userInfo(0, alice);
        assertEq(amt, 50e18);
        // reduced share going forward, exactly
        vm.warp(vm.getBlockTimestamp() + 300);
        assertEq(gardener.pendingMyrrh(0, alice), 100e18); // 300 * 50/150
        assertEq(gardener.pendingMyrrh(0, bob), 250e18); // 50 + 300 * 100/150
    }

    /// claimLocked is clamped by the granary like every other mint: a locked
    /// balance bigger than the day's budget drains across days, exactly, and
    /// nothing is ever lost. 67 locked vs a 30/day cap: 30 + 30 + 7.
    function test_claimLocked_partial_under_tight_granary_cap_drains_daily() public {
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100);
        vm.prank(alice);
        gardener.deposit(0, 0); // locks 67, pays 33
        assertEq(gardener.lockedOf(alice), 67e18);
        vm.warp(unlockAt); // past the cliff (a later UTC day)
        granary.setGrant(address(gardener), 30e18); // tighter than the locked balance
        vm.prank(alice);
        uint256 paid = gardener.claimLocked();
        assertEq(paid, 30e18); // the cap's worth
        assertEq(gardener.lockedOf(alice), 37e18); // remainder retained, not lost
        vm.warp(vm.getBlockTimestamp() + 1 days); // the day rolls: fresh budget
        vm.prank(alice);
        paid = gardener.claimLocked();
        assertEq(paid, 30e18);
        assertEq(gardener.lockedOf(alice), 7e18);
        vm.warp(vm.getBlockTimestamp() + 1 days);
        vm.prank(alice);
        paid = gardener.claimLocked();
        assertEq(paid, 7e18); // drained to zero
        assertEq(gardener.lockedOf(alice), 0);
        assertEq(myrrh.balanceOf(alice), 100e18); // 33 + 30 + 30 + 7: conserved
    }

    /// Port of EloSilo's rate-change shape: setRewardPerSecond runs
    /// massUpdatePools first, so the elapsed interval settles at the OLD rate
    /// and only the future accrues at the new one. 100s @ 1/s + 100s @ 2/s.
    function test_rate_change_settles_first_no_retroactive_repricing() public {
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 @ 1 MYRRH/s
        gardener.setRewardPerSecond(2e18); // massUpdatePools runs inside
        vm.warp(vm.getBlockTimestamp() + 100); // 200 @ 2 MYRRH/s
        assertEq(gardener.pendingMyrrh(0, alice), 300e18); // never 400
    }

    /// setAllocPoint massUpdates first too: the elapsed interval keeps the
    /// old alloc split; only the future shifts. Exact per-interval values.
    function test_setAllocPoint_rebalances_forward_only() public {
        gardener.addPool(IERC20(address(tscry)), 100); // pid 1: even split
        vm.startPrank(bob);
        tscry.faucet();
        tscry.approve(address(gardener), type(uint256).max);
        gardener.deposit(1, 100e18);
        vm.stopPrank();
        _stake(alice, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 emitted at 100/100 alloc
        assertEq(gardener.pendingMyrrh(0, alice), 50e18);
        assertEq(gardener.pendingMyrrh(1, bob), 50e18);
        gardener.setAllocPoint(1, 300); // total 400: pool 0 1/4, pool 1 3/4
        vm.warp(vm.getBlockTimestamp() + 100); // 100 emitted at the NEW split
        assertEq(gardener.pendingMyrrh(0, alice), 75e18); // 50 + 100*100/400
        assertEq(gardener.pendingMyrrh(1, bob), 125e18); // 50 + 100*300/400
    }

    // -- production parameters (TEST-AUDIT.md: the precision floor) ---------

    /// DECIDED + FIXED 2026-07-22 (was PINNED as a permissionless permanent
    /// grief — see contracts/TEST-AUDIT.md, Gardener precision floor). At the
    /// posted production rate a 1-second interval floors the per-share
    /// increment to zero whenever lpSupply > rps * ACC; updatePool is public,
    /// so per-second callers used to zero emission FOREVER. The fix:
    /// carryScaled[pid] accumulates the scaled remainder, so per-second and
    /// patient callers arrive at identical state and nothing is ever lost.
    /// This test proves exact conservation at production scale.
    ///
    /// ⚠ BOTH NUMBERS BELOW MOVE WITH THE ERA-0 RATE, and the stake is the one
    /// that matters: the threshold is `rps * ACC`, so the 25x rate rise on
    /// 2026-07-28 took it from 2.78e27 to 6.94e28 — and the old 3e27 stake,
    /// left alone, would have sat BELOW the floor it exists to cross. The test
    /// would still pass, having stopped testing anything.
    function test_production_rps_per_second_updates_lose_nothing_carry_conserves() public {
        uint256 prodRps = 69_444_444_444_444_444; // DeployGardener.s.sol default, era 0
        uint256 bigStake = 8e28; // > rps * ACC = 6.94e28: the floor threshold
        SpoilsToken bigLp = new SpoilsToken("big lp", "BLP", 0, op);
        uint256 cliff = vm.getBlockTimestamp() + 90 days;
        EloGardener prod = new EloGardener(granary, prodRps, 6700, cliff);
        prod.addPool(IERC20(address(bigLp)), 100); // sole pool: the whole stream
        vm.prank(op);
        bigLp.mint(alice, bigStake);
        vm.startPrank(alice);
        bigLp.approve(address(prod), type(uint256).max);
        prod.deposit(0, bigStake);
        vm.stopPrank();

        uint256 t0 = vm.getBlockTimestamp();
        vm.warp(t0 + 1);
        prod.updatePool(0);
        (,, uint256 lastTime, uint256 acc) = prod.pools(0);
        assertEq(acc, 0); // floor(1 * 2.77e14 * 1e12 / 3e26) == 0 still...
        assertEq(lastTime, t0 + 1);
        assertEq(prod.carryScaled(0), prodRps * 1e12); // ...but NOTHING is lost: the carry holds it
        vm.warp(t0 + 2);
        prod.updatePool(0);
        (,, lastTime, acc) = prod.pools(0);
        assertEq(acc, 1); // the carried remainder tips the second update over
        assertEq(lastTime, t0 + 2);
        assertEq(prod.carryScaled(0), 2 * prodRps * 1e12 - bigStake); // exact remainder retained
        assertEq(prod.pendingMyrrh(0, alice), (bigStake * 1) / 1e12); // 3e14 wei MYRRH credited

        // conservation: credited + carried == everything emitted, to the wei.
        // Per-second callers and patient callers now arrive at identical state
        // (a patient single 2s update yields the same acc=1, same carry) —
        // the permissionless zero-emission grief is closed. 2026-07-22.
        assertEq(acc * bigStake + prod.carryScaled(0), 2 * prodRps * 1e12);
    }

    // -- ladder boundaries + governance (TEST-AUDIT.md: operator knobs) -----

    /// The six exact rung boundaries of the DFK ladder. Every comparison in
    /// withdrawFeeBps is strict `<`, so AT each boundary the next (cheaper)
    /// rung already applies. Pure calls; complements the mid-rung sweep above.
    function test_slash_ladder_exact_boundaries() public view {
        assertEq(gardener.withdrawFeeBps(1 hours), 400);
        assertEq(gardener.withdrawFeeBps(1 days), 200);
        assertEq(gardener.withdrawFeeBps(3 days), 100);
        assertEq(gardener.withdrawFeeBps(5 days), 50);
        assertEq(gardener.withdrawFeeBps(2 weeks), 25);
        assertEq(gardener.withdrawFeeBps(4 weeks), 1);
    }

    function test_transferOwner_old_refused_new_works() public {
        vm.expectRevert("zero owner");
        gardener.transferOwner(address(0));
        vm.prank(bob);
        vm.expectRevert("owner only");
        gardener.transferOwner(bob); // a stranger cannot take the keys
        gardener.transferOwner(alice);
        assertEq(gardener.owner(), alice);
        vm.expectRevert("owner only");
        gardener.setRewardPerSecond(2e18); // the old owner (this) is refused
        vm.prank(alice);
        gardener.setRewardPerSecond(2e18); // the new owner works
        assertEq(gardener.rewardPerSecond(), 2e18);
    }

    function test_granary_transferSteward_and_stranger_gates() public {
        vm.startPrank(bob); // a stranger hits every steward knob
        vm.expectRevert("steward only");
        granary.setGrant(bob, 1e18);
        vm.expectRevert("steward only");
        granary.revokeGrant(alice);
        vm.expectRevert("steward only");
        granary.rotateMinterAway(bob);
        vm.expectRevert("steward only");
        granary.transferSteward(bob);
        vm.stopPrank();
        vm.expectRevert("zero steward");
        granary.transferSteward(address(0));
        vm.expectRevert("zero minter");
        granary.setGrant(address(0), 1e18);

        // TWO-STEP since 2026-07-28. Proposing does not move the role: the
        // granary holds the SpoilsToken minter slot and `setMinter` is
        // `onlyMinter`, so a one-step handoff to an address nobody controls
        // welded the coin's mint authority to a granary nobody could drive —
        // no new distributor, no stewardMint to fund a harvest root, ever.
        // The successor has to transact to prove it exists.
        granary.transferSteward(alice);
        assertEq(granary.steward(), address(this), "proposing does not hand over");
        assertEq(granary.pendingSteward(), alice);
        granary.setGrant(bob, 1e18); // the incumbent still holds the role

        vm.prank(bob);
        vm.expectRevert("not pending steward");
        granary.acceptSteward();

        vm.prank(alice);
        granary.acceptSteward();
        assertEq(granary.steward(), alice);
        assertEq(granary.pendingSteward(), address(0), "the proposal is consumed");

        vm.expectRevert("steward only");
        granary.setGrant(bob, 1e18); // the old steward (this) is refused
        vm.prank(alice);
        granary.setGrant(bob, 1e18); // the new steward works
        assertEq(granary.availableToday(bob), 1e18);
    }

    // ── THE HALVING SCHEDULE (operator 2026-07-28: "farm MYRRH like bitcoin
    //    ... early users get way more than people later, max 40 year run") ──
    //
    // The shape is welded as constants, so these assert the CURVE, not a dial.
    // Every other test in this file lives inside era 0 (the longest warp is 5
    // weeks), where emittedBetween is exactly elapsed * rewardPerSecond — which
    // is why none of them changed when the schedule landed.

    function test_schedule_halves_every_four_years() public view {
        uint256 t0 = gardener.startTime();
        uint256 period = gardener.HALVING_PERIOD();
        assertEq(period, 4 * 365 days, "the period is four years");
        // a day in each of the first four eras, each half the last
        for (uint256 era = 0; era < 4; era++) {
            uint256 at = t0 + era * period;
            assertEq(
                gardener.emittedBetween(at, at + 1 days), (RPS >> era) * 1 days, "era rate is base >> era"
            );
        }
    }

    /// The bug this whole function exists to prevent: a pool left un-updated
    /// across a halving must NOT be paid the old rate for the whole span.
    function test_schedule_splits_an_interval_spanning_a_halving() public view {
        uint256 t0 = gardener.startTime();
        uint256 boundary = t0 + gardener.HALVING_PERIOD();
        uint256 got = gardener.emittedBetween(boundary - 1 days, boundary + 1 days);
        assertEq(got, RPS * 1 days + (RPS / 2) * 1 days, "one day each side, at its own rate");
        // and it is strictly less than the naive flat-rate answer
        assertLt(got, RPS * 2 days, "naive elapsed*rate would overpay across the boundary");
    }

    /// "max 40 year run" is a fact about the contract, not a projection.
    function test_schedule_stops_dead_at_forty_years() public {
        uint256 t0 = gardener.startTime();
        uint256 end = gardener.emissionEnd();
        assertEq(end, t0 + 4 * 365 days * 10, "ten halvings of four years");
        assertEq(gardener.emittedBetween(end, end + 3650 days), 0, "nothing after the end");
        // a call spanning the end is clamped to it, not extrapolated
        assertEq(
            gardener.emittedBetween(t0, end + 3650 days),
            gardener.emittedBetween(t0, end),
            "past the terminal is clamped, never extrapolated"
        );
        vm.warp(end);
        assertEq(gardener.currentRewardPerSecond(), 0, "the rate is zero once the run is over");
    }

    /// Early >> late, which is the whole point of the operator's sentence.
    function test_schedule_front_loads_half_into_the_first_era() public view {
        uint256 t0 = gardener.startTime();
        uint256 period = gardener.HALVING_PERIOD();
        uint256 lifetime = gardener.emittedBetween(t0, gardener.emissionEnd());
        uint256 firstEra = gardener.emittedBetween(t0, t0 + period);
        // geometric: era 0 is ~50% of everything the farm will ever emit
        // the true delta is 0.0978% (the run is 2 - 2^-9 era-widths, not 2), so
        // 0.1% would pass by 0.002 percentage points — a tolerance that tight is
        // an accident waiting to look like a regression
        assertApproxEqRel(firstEra, lifetime / 2, 0.002e18, "first four years are ~half the run");
        uint256 lastEra = gardener.emittedBetween(gardener.emissionEnd() - period, gardener.emissionEnd());
        assertEq(lastEra * 512, firstEra, "the final era pays 1/512th of the first");
    }

    /// The schedule must not be able to breach SpoilsToken's IMMUTABLE cap.
    /// Checked at the real posted numbers rather than the test's 1 MYRRH/sec.
    function test_schedule_lifetime_fits_under_the_myrrh_cap() public {
        uint256 posted = 69_444_444_444_444_444; // 6,000/day era 0 (DeployGardener)
        EloGranary g2 = new EloGranary(myrrh);
        EloGardener prod = new EloGardener(g2, posted, 6700, vm.getBlockTimestamp() + 90 days);
        uint256 lifetime = prod.emittedBetween(prod.startTime(), prod.emissionEnd());
        // ~2.42M genesis float (pools + Garden seed + drop + powder) + the run
        uint256 genesisFloat = 2_422_640e18;
        assertLt(lifetime + genesisFloat, 21_000_000e18, "the whole run fits under the cap");
        // and the margin is real headroom, not a rounding sliver
        assertGt(21_000_000e18 - (lifetime + genesisFloat), 1_000_000e18, ">=1M MYRRH never minted");
    }

    /// The owner rescales the curve's HEIGHT and cannot touch its SHAPE.
    function test_owner_moves_the_base_but_not_the_schedule() public {
        uint256 end = gardener.emissionEnd();
        uint256 period = gardener.HALVING_PERIOD();
        gardener.setRewardPerSecond(RPS * 2);
        assertEq(gardener.emissionEnd(), end, "the terminal did not move");
        assertEq(gardener.HALVING_PERIOD(), period, "the period did not move");
        uint256 t0 = gardener.startTime();
        // still halves, just from the new base
        assertEq(gardener.emittedBetween(t0 + period, t0 + period + 1 days), RPS * 1 days);
    }
}
