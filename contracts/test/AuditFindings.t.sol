// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryGranary.sol";
import "../src/ScryGardener.sol";
import "../src/ScrySilo.sol";
import "../src/ScryGarden.sol";
import "../src/ScryHarvest.sol";
import "../src/ScryJobBoard.sol";
import "../src/ScryReputation.sol";
import "../src/ScryInsurancePool.sol";
import "../src/ScryFeeSplitter.sol";
import "../src/IScryArbiter.sol";
import "../src/IERC20.sol";
import "../script/DeployGardener.s.sol";
import "../src/ScryGacha.sol";
import {MockArbSys} from "./MockArbSys.sol";

/// @title AuditFindings — the 2026-07-25 audit's findings, now as regressions
/// @notice These started life as DEFECT REPRODUCTIONS: eight tests that passed
///         by asserting broken behaviour. The fixes landed the same day, so
///         every assertion here has been INVERTED in place rather than deleted
///         — each one now fails if its bug is ever reintroduced, and the
///         comment above it still records what the bug was. That history is the
///         point: a regression test whose defect is named is worth two whose
///         aren't. Full write-up: contracts/AUDIT-2026-07-25.md.
contract AuditFindingsTest is Test {
    // ─────────────────────────────────────────────────────────────────────────
    // F1 · ScrySilo.updateBin USED TO lose emission permanently — the
    //      ScryGardener `carryScaled` fix (2026-07-22) was never ported to the
    //      Silo, and updateBin() is public, so ANY address could drive the loss.
    //      FIXED 2026-07-25: carryScaled ported verbatim.
    // ─────────────────────────────────────────────────────────────────────────
    function _silo() internal returns (SpoilsToken obol, ScryGranary gran, ScrySilo silo) {
        obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        gran = new ScryGranary(obol);
        obol.setMinter(address(gran));
        // production numbers: era-0 emission (DeployGardener.s.sol default)
        silo = new ScrySilo(gran, 2_777_777_777_777_777, 2500);
        gran.setGrant(address(silo), type(uint256).max);
        silo.addTier(30 days, 10_000); // 1x
        silo.addBin(obol, 100);
    }

    function _sealBig(SpoilsToken obol, ScryGranary gran, ScrySilo silo, address who, uint256 amt) internal {
        gran.stewardMint(who, amt);
        vm.startPrank(who);
        obol.approve(address(silo), amt);
        silo.seal(0, 0, amt);
        vm.stopPrank();
    }

    /// The whole point of `carryScaled` in ScryGardener, absent here: when
    /// (reward * 1e12) / totalWeighted floors to zero, `lastRewardTime` still
    /// advances, so that second's emission is gone forever.
    function test_F1_silo_carry_makes_per_second_and_patient_updateBin_agree() public {
        // 3e27 wei sealed == 3 billion OBOL. reward*ACC for one second is
        // 2.777e15 * 1e12 = 2.777e27 < 3e27, so the per-share increment floors to 0.
        uint256 sealAmt = 3e27;
        address alice = address(0xA11CE);

        // ── arm A: one patient update after 1000 seconds ────────────────────
        (SpoilsToken obolA, ScryGranary granA, ScrySilo siloA) = _silo();
        _sealBig(obolA, granA, siloA, alice, sealAmt);
        vm.warp(vm.getBlockTimestamp() + 1000);
        siloA.updateBin(0);
        uint256 patient = siloA.pendingReward(alice, 0);

        // ── arm B: same 1000 seconds, but someone calls updateBin every second ──
        (SpoilsToken obolB, ScryGranary granB, ScrySilo siloB) = _silo();
        _sealBig(obolB, granB, siloB, alice, sealAmt);
        address griefer = address(0x6217);
        for (uint256 i = 0; i < 1000; i++) {
            vm.warp(vm.getBlockTimestamp() + 1);
            vm.prank(griefer); // permissionless: updateBin is public
            siloB.updateBin(0);
        }
        uint256 griefed = siloB.pendingReward(alice, 0);

        assertGt(patient, 0, "the patient arm accrues");
        // WAS: griefed == 0 — the identical stake over the identical window
        // accrued NOTHING, because each 1s interval floored to a zero per-share
        // increment while lastRewardTime advanced anyway.
        // NOW: carryScaled holds the remainder, so both callers land in the same
        // place. Equal to the wei; the residual carry is sub-wei of reward.
        assertEq(griefed, patient, "F1: per-second and patient callers must agree");
    }

    /// Control: the same shape in ScryGardener is already fixed by carryScaled,
    /// which is what makes F1 an un-ported fix rather than an accepted design.
    function test_F1_control_gardener_carry_survives_the_same_grief() public {
        SpoilsToken obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        ScryGranary gran = new ScryGranary(obol);
        obol.setMinter(address(gran));
        ScryGardener farm = new ScryGardener(gran, 2_777_777_777_777_777, 6700, block.timestamp + 90 days);
        gran.setGrant(address(farm), type(uint256).max);

        SpoilsToken lp = new SpoilsToken("lp", "LP", 0, address(this));
        farm.addPool(IERC20(address(lp)), 100);

        address alice = address(0xA11CE);
        uint256 amt = 3e27;
        lp.mint(alice, amt);
        vm.startPrank(alice);
        lp.approve(address(farm), amt);
        farm.deposit(0, amt);
        vm.stopPrank();

        address griefer = address(0x6217);
        for (uint256 i = 0; i < 1000; i++) {
            vm.warp(vm.getBlockTimestamp() + 1);
            vm.prank(griefer);
            farm.updatePool(0);
        }
        assertGt(farm.pendingMyrrh(0, alice), 0, "carryScaled keeps the Gardener whole");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // F2 · ScryGardener USED TO treat `lp.balanceOf(address(this))` as the
    //      staked supply, so an unsolicited LP donation permanently diluted
    //      every staker and stranded the donated share's emission.
    //      FIXED 2026-07-25: an internal `stakedSupply` is the denominator.
    // ─────────────────────────────────────────────────────────────────────────
    function test_F2_gardener_lp_donation_cannot_touch_the_stream() public {
        SpoilsToken obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        ScryGranary gran = new ScryGranary(obol);
        obol.setMinter(address(gran));
        ScryGardener farm = new ScryGardener(gran, 1e18, 0, block.timestamp + 90 days);
        gran.setGrant(address(farm), type(uint256).max);
        SpoilsToken lp = new SpoilsToken("lp", "LP", 0, address(this));
        farm.addPool(IERC20(address(lp)), 100);

        address alice = address(0xA11CE);
        lp.mint(alice, 100e18);
        vm.startPrank(alice);
        lp.approve(address(farm), 100e18);
        farm.deposit(0, 100e18);
        vm.stopPrank();

        // a griefer plainly TRANSFERS lp shares in — no deposit, no stake, no
        // UserInfo row, therefore no claim on the emission it now absorbs.
        address griefer = address(0x6217);
        lp.mint(griefer, 900e18);
        vm.prank(griefer);
        lp.transfer(address(farm), 900e18);

        vm.warp(vm.getBlockTimestamp() + 1000);
        // 1000 seconds x 1e18/s = 1000e18 emitted; alice is the only staker.
        // WAS: 100e18 — the denominator was lp.balanceOf(this), so the donation
        // absorbed 90% of the stream and stranded it forever.
        // NOW: stakedSupply counts only real deposits; the donation is inert.
        assertEq(farm.pendingMyrrh(0, alice), 1000e18, "F2: a donation must not touch the stream");

        // and it stays inert through a full exit — the donation is not
        // withdrawable, but it never blocks or inflates one either.
        vm.prank(alice);
        farm.withdraw(0, 100e18);
        assertEq(farm.stakedSupply(0), 0, "F2: staked supply drains to zero, donation aside");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // F3 · ScryHarvest.sweep's solvency check USED TO derive the obligation as
    //      the GLOBAL totalClaimed against the CURRENT root's totalCommitted.
    //      Drop a wallet that had already claimed and the operator could sweep
    //      funds a live claimant was still owed under the posted root.
    //      FIXED 2026-07-25: `owedUnderRoot` is tracked explicitly.
    // ─────────────────────────────────────────────────────────────────────────
    function test_F3_harvest_sweep_cannot_take_a_live_claimants_money() public {
        SpoilsToken obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        ScryHarvest h = new ScryHarvest(IERC20(address(obol)));
        obol.mint(address(h), 150e18);

        address a = address(0xA);
        address b = address(0xB);
        bytes32 leafA = keccak256(abi.encodePacked(a, uint256(100e18)));
        bytes32 leafB = keccak256(abi.encodePacked(b, uint256(50e18)));
        bytes32 root1 = leafA < leafB ? keccak256(abi.encodePacked(leafA, leafB)) : keccak256(abi.encodePacked(leafB, leafA));

        h.postRoot(root1, 150e18, "ledger/1");
        bytes32[] memory pa = new bytes32[](1);
        pa[0] = leafB;
        h.claim(a, 100e18, pa);
        assertEq(obol.balanceOf(a), 100e18);

        // The operator posts the next epoch's root over the wallets that still
        // have a balance — the natural thing to do, and nothing on-chain warns.
        h.postRoot(leafB, 50e18, "ledger/2");

        // WAS: owedMax = totalCommitted(50) - totalClaimed(100) clamped to 0, so
        // this swept the contract empty and B's posted claim then reverted.
        // NOW: owedUnderRoot is re-armed to 50 by postRoot, so the sweep that
        // would break the posted root is refused.
        assertEq(h.owedUnderRoot(), 50e18, "F3: the posted root's obligation is tracked");
        vm.expectRevert(bytes("would break the posted root"));
        h.sweep(address(this), 50e18);

        // b's posted, still-valid claim is still payable.
        bytes32[] memory pb = new bytes32[](0);
        h.claim(b, 50e18, pb);
        assertEq(obol.balanceOf(b), 50e18, "F3: the promise is kept");

        // The current root is fully paid — and that is NOT yet enough. Posting
        // `ledger/2` opened a SWEEP_DELAY window, and inside it `sweepFloor()`
        // is the HIGHER of what this root owes and what the outgoing one still
        // owed (`priorOwed` = 50e18, root1's residual). The contract cannot know
        // that b's 50 under root2 IS root1's residual rather than a second debt,
        // so it holds the larger number for the whole window. Over-stating the
        // floor only ever sweeps LESS, which is the sole safe direction.
        assertEq(h.owedUnderRoot(), 0, "the root is fully paid");
        assertEq(h.sweepFloor(), 50e18, "and the window still holds the prior root's floor");
        obol.mint(address(h), 7e18); // a surplus that was never committed
        vm.expectRevert(bytes("would break the posted root"));
        h.sweep(address(this), 7e18);

        // Only when the window closes does the floor drop to what is actually
        // outstanding, and the uncommitted surplus becomes sweepable.
        vm.warp(block.timestamp + h.SWEEP_DELAY());
        assertEq(h.sweepFloor(), 0, "the window closed, the floor is the real obligation");
        h.sweep(address(this), 7e18);
        assertEq(obol.balanceOf(address(h)), 0, "F3: surplus only");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // F4 · ScryJobBoard.dispute() USED TO call the arbiter BEFORE closing the
    //      job. The arbiter is an operator-swappable address CALLed (not
    //      staticcall), so it could reenter close() and make one job pay out
    //      twice — taking a different buyer's escrow out of the pooled balance.
    //      FIXED 2026-07-25: checks-effects-interactions, plus an immutable
    //      arbitration-fee ceiling for F4b.
    // ─────────────────────────────────────────────────────────────────────────
    function test_F4_jobboard_dispute_reentrancy_cannot_pay_one_job_twice() public {
        MockERC20 scry = new MockERC20();
        ScryReputation rep = new ScryReputation(0);
        ScryInsurancePool pool = new ScryInsurancePool(IERC20(address(scry)), 0);
        address splitter = address(0x5719);
        EvilArbiter evil = new EvilArbiter();
        ScryJobBoard board = new ScryJobBoard(
            IERC20(address(scry)), IReputation(address(rep)), IInsurancePool(address(pool)), splitter, evil, 10e18
        );
        rep.setAuthority(address(board), true);
        evil.point(board);

        address buyer1 = address(0xB1);
        address buyer2 = address(0xB2);
        address seller = address(0x5E11E);
        scry.mint(buyer1, 100e18);
        scry.mint(buyer2, 100e18);

        vm.startPrank(buyer1);
        scry.approve(address(board), 100e18);
        uint256 jobA = board.post(seller, 100e18, ScryJobBoard.Mode.Escrow, "specA", uint64(block.timestamp + 1 days), 0);
        vm.stopPrank();

        vm.startPrank(buyer2);
        scry.approve(address(board), 100e18);
        board.post(seller, 100e18, ScryJobBoard.Mode.Escrow, "specB", uint64(block.timestamp + 30 days), 0);
        vm.stopPrank();

        assertEq(scry.balanceOf(address(board)), 200e18, "both escrows held in one pot");

        vm.prank(seller);
        board.deliver(jobA, "delivered");
        vm.warp(vm.getBlockTimestamp() + 2 days); // jobA past its deadline, jobB is not

        evil.arm(jobA); // reenter close(jobA) from inside rule()
        vm.prank(seller);
        // WAS: the reentrant close() settled jobA a second time out of the
        // pooled escrow — seller 190e18, splitter 10e18, board drained to 0,
        // buyer2's money gone.
        //
        // NOW: TWO independent bounds, and the outer one answers first.
        // §M6 added `nonReentrant`, so the reentrant call is refused before it
        // reads any state at all — that is the message asserted here. The
        // ordering fix is still underneath it and still load-bearing:
        // dispute() closes the job BEFORE calling the arbiter, so even with the
        // guard removed the reentrant close() would hit "closed". That second
        // bound is pinned on its own below, since a test can only observe the
        // first guard to fire.
        vm.expectRevert(bytes("reentrant"));
        board.dispute(jobA);

        // nothing moved, and buyer2's escrow is untouched.
        assertEq(scry.balanceOf(seller), 0, "F4: no double payout");
        assertEq(scry.balanceOf(address(board)), 200e18, "F4: both escrows intact");

        // with a well-behaved arbiter the same dispute settles exactly once.
        evil.disarm();
        vm.prank(seller);
        board.dispute(jobA);
        assertEq(scry.balanceOf(seller), 95e18, "F4: one net payout");
        assertEq(scry.balanceOf(splitter), 5e18, "F4: one fee");
        assertEq(scry.balanceOf(address(board)), 100e18, "F4: buyer2's escrow still held");

        // the SECOND bound, on its own: a settled job is closed state, so a
        // plain (non-reentrant) second settlement is refused by the ordering
        // fix rather than by the guard. Remove `nonReentrant` and this still
        // holds — which is what "two independent bounds" has to mean.
        vm.expectRevert(bytes("closed"));
        board.close(jobA);
        assertEq(scry.balanceOf(seller), 95e18, "F4: still one net payout");
    }

    /// Even without reentrancy, an operator-set arbiter names its own fee and
    /// its own recipient, and the board pulls it from the disputing party's
    /// allowance with no ceiling.
    function test_F4b_jobboard_arbiter_fee_is_bounded_by_the_immutable_ceiling() public {
        MockERC20 scry = new MockERC20();
        ScryReputation rep = new ScryReputation(0);
        ScryInsurancePool pool = new ScryInsurancePool(IERC20(address(scry)), 0);
        GreedyArbiter greedy = new GreedyArbiter();
        ScryJobBoard board = new ScryJobBoard(
            IERC20(address(scry)),
            IReputation(address(rep)),
            IInsurancePool(address(pool)),
            address(0x5719),
            greedy,
            10e18 // the immutable ceiling
        );
        rep.setAuthority(address(board), true);
        board.setInsuredOpen(true); // ships closed since 2026-07-25; F4b is about the FEE, so open it

        address buyer = address(0xB1);
        address seller = address(0x5E11E);
        scry.mint(buyer, 1_000e18);
        vm.startPrank(buyer);
        scry.approve(address(board), type(uint256).max); // the Insured / RepOnly flow asks for exactly this
        uint256 id = board.post(seller, 1e18, ScryJobBoard.Mode.Insured, "spec", uint64(block.timestamp + 1 days), 1e18);
        // WAS: the arbiter's self-declared 998e18 "flat fee" was pulled straight
        // out of the standing approval. NOW: it is refused against maxArbFee.
        vm.expectRevert(bytes("arb fee over the posted ceiling"));
        board.dispute(id);
        vm.stopPrank();

        assertEq(scry.balanceOf(greedy.pocket()), 0, "F4b: an arbiter cannot name its own price");
        assertEq(board.maxArbFee(), 10e18, "the ceiling is immutable state, not a knob");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // F5 · ScryGarden.addLiquidity USED TO silently keep the over-supplied
    //      side: no router, no refund, so a mis-ratioed add was an
    //      uncompensated donation to existing LPs — and DeploySpoils.s.sol
    //      points this pair at real SCRY. FIXED 2026-07-25: the amounts are
    //      maximums and only the in-ratio side is pulled.
    // ─────────────────────────────────────────────────────────────────────────
    function test_F5_garden_addLiquidity_never_pulls_the_excess() public {
        MockERC20 t0 = new MockERC20();
        MockERC20 t1 = new MockERC20();
        ScryGarden g = new ScryGarden(IERC20(address(t0)), IERC20(address(t1)));

        t0.mint(address(this), 1_000e18);
        t1.mint(address(this), 1_000e18);
        t0.approve(address(g), type(uint256).max);
        t1.approve(address(g), type(uint256).max);
        g.addLiquidity(100e18, 100e18, 0, type(uint256).max); // pool opens at 1:1

        address lp = address(0x11D);
        t0.mint(lp, 100e18);
        t1.mint(lp, 100e18);
        vm.startPrank(lp);
        t0.approve(address(g), type(uint256).max);
        t1.approve(address(g), type(uint256).max);
        // a naive add at the wrong ratio: 10 of token0, 100 of token1
        uint256 seeds = g.addLiquidity(10e18, 100e18, 0, type(uint256).max);
        vm.stopPrank();

        // seeds are still priced off the short side...
        assertEq(seeds, 10e18, "seeds priced off min(...)");
        // WAS: t1.balanceOf(lp) == 0 — the extra 90e18 was pulled in, kept by
        // the pool, and never refunded. NOW: the amounts are MAXIMUMS and only
        // the matching side is pulled.
        assertEq(t1.balanceOf(lp), 90e18, "F5: the excess is never pulled");
        assertEq(t0.balanceOf(lp), 90e18, "F5: the short side was used in full");
        (uint112 r0, uint112 r1) = g.getReserves();
        assertEq(uint256(r0), 110e18, "reserves stay in ratio");
        assertEq(uint256(r1), 110e18, "reserves stay in ratio");

        // and the round trip cannot mint value: burning the seeds returns no
        // more than went in.
        //
        // ⚠ This pair used to be `assertLe(out, 10e18)`, which
        // `AUDIT-2026-07-25.md` §3 correctly listed as one of three tests that
        // PASS VACUOUSLY — `out == 0` satisfies it, so a Garden that ate the
        // whole position would have been graded green by the very test named
        // "no value minted". Asserted exactly instead: at these reserves the
        // round trip returns precisely what it put in, so BOTH failure
        // directions (minting value, and swallowing it) are now caught.
        vm.startPrank(lp);
        (uint256 out0, uint256 out1) = g.removeLiquidity(seeds, 0, 0, type(uint256).max);
        vm.stopPrank();
        assertEq(out0, 10e18, "F5: the round trip returns exactly what went in - token0");
        assertEq(out1, 10e18, "F5: the round trip returns exactly what went in - token1");
        // the conservation law the assertion above is a proxy for, stated where
        // a zero cannot satisfy it: the LP ends holding what they started with.
        assertEq(t0.balanceOf(lp), 100e18, "F5: LP is made whole, token0");
        assertEq(t1.balanceOf(lp), 100e18, "F5: LP is made whole, token1");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // F5b · The Garden had NO deadline on any call and NO minimum-out on
    //       add/remove, so a signed transaction could sit in the mempool and
    //       execute later at a price the sender never saw. Added 2026-07-25
    //       when the Garden became the farmed MYRRH/OBOL LP.
    //
    //       These close SANDWICHING, not arbitrage. Nothing can close
    //       arbitrage — that is what LPing is (LAUNCH-DECISIONS.md §the house
    //       as market maker says so out loud).
    // ─────────────────────────────────────────────────────────────────────────
    function test_F5b_garden_calls_all_honour_a_deadline() public {
        MockERC20 t0 = new MockERC20();
        MockERC20 t1 = new MockERC20();
        ScryGarden g = new ScryGarden(IERC20(address(t0)), IERC20(address(t1)));
        t0.mint(address(this), 1_000e18);
        t1.mint(address(this), 1_000e18);
        t0.approve(address(g), type(uint256).max);
        t1.approve(address(g), type(uint256).max);

        uint256 past = block.timestamp - 1;
        vm.expectRevert(bytes("deadline passed"));
        g.addLiquidity(100e18, 100e18, 0, past);

        uint256 seeds = g.addLiquidity(100e18, 100e18, 0, block.timestamp);
        vm.expectRevert(bytes("deadline passed"));
        g.removeLiquidity(seeds, 0, 0, past);
        vm.expectRevert(bytes("deadline passed"));
        g.swap(1e18, true, 0, past);

        // and the current block is still inside the window (<=, not <)
        g.swap(1e18, true, 0, block.timestamp);
    }

    function test_F5b_garden_add_and_remove_honour_their_minimums() public {
        MockERC20 t0 = new MockERC20();
        MockERC20 t1 = new MockERC20();
        ScryGarden g = new ScryGarden(IERC20(address(t0)), IERC20(address(t1)));
        t0.mint(address(this), 1_000e18);
        t1.mint(address(this), 1_000e18);
        t0.approve(address(g), type(uint256).max);
        t1.approve(address(g), type(uint256).max);
        g.addLiquidity(100e18, 100e18, 0, type(uint256).max);

        // asking for more SEED than the deposit can mint is refused, not filled
        vm.expectRevert(bytes("slippage: fewer SEED than minSeeds"));
        g.addLiquidity(10e18, 10e18, 11e18, type(uint256).max);
        uint256 seeds = g.addLiquidity(10e18, 10e18, 10e18, type(uint256).max);
        assertEq(seeds, 10e18, "exactly the minimum is acceptable");

        // and the exit refuses to hand back less than the caller demanded
        vm.expectRevert(bytes("slippage: less out than the minimums"));
        g.removeLiquidity(seeds, 11e18, 0, type(uint256).max);
        vm.expectRevert(bytes("slippage: less out than the minimums"));
        g.removeLiquidity(seeds, 0, 11e18, type(uint256).max);
        (uint256 a0, uint256 a1) = g.removeLiquidity(seeds, 10e18, 10e18, type(uint256).max);
        assertEq(a0, 10e18);
        assertEq(a1, 10e18);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // F6 · vm.envOr's fallback argument is evaluated EAGERLY. DeployGardener's
    //      fallbacks USED TO be vm.envAddress calls, making MYRRH_TOKEN and
    //      SEED_OBOL_SCRY mandatory — the opposite of what its runbook
    //      promised. FIXED 2026-07-25: `envAddrOr` resolves sequentially, and
    //      preflight.py now refuses the banned shape in ANY deploy script.
    // ─────────────────────────────────────────────────────────────────────────
    /// The language fact itself, pinned so a foundry change cannot quietly make
    /// the banned pattern look safe again. WAS the shape DeployGardener used.
    ///
    /// ⚠ THIS TEST USED TO PASS VACUOUSLY, and `AUDIT-2026-07-25.md` §3 listed
    /// it as one of three that did: with BOTH keys unset, an eager default and
    /// a lazy one are indistinguishable — the primary is missing either way, so
    /// the fallback runs under either semantics and reverts — and a bare
    /// `vm.expectRevert()` cannot say which call reverted. The audit noted the
    /// discriminating run had been done BY HAND and never written down. It is
    /// written down now, twice, in both directions.
    ///
    /// The discriminator is: SET THE PRIMARY. A lazy default would never touch
    /// the fallback; an eager one evaluates it regardless.
    ///
    /// Env note: `vm.setEnv` is process-global and forge's workers share one
    /// process, so these keys are named uniquely on purpose — a race needs two
    /// tests touching the SAME key, and nothing else in the repo touches these.
    uint256 public envDefaultEvals;

    function bumpAndReturn() public returns (address) {
        envDefaultEvals++;
        return address(0xBEEF);
    }

    /// Direction 1, no revert involved: the primary WINS and the default STILL
    /// RAN. A counter cannot be satisfied by the wrong call reverting.
    function test_F6_the_default_is_evaluated_even_when_the_primary_is_set() public {
        vm.setEnv("SCRY_AUDIT_F6_PRIMARY_SET", "0x00000000000000000000000000000000000000A1");
        uint256 before = envDefaultEvals;

        address got = vm.envOr("SCRY_AUDIT_F6_PRIMARY_SET", bumpAndReturn());

        assertEq(got, address(0xA1), "the primary won, exactly as envOr documents");
        assertEq(envDefaultEvals, before + 1, "and the default ran anyway - THAT is the bug");
    }

    /// Direction 2, the production symptom: with the primary SET, the banned
    /// shape still reverts — because the eagerly-evaluated fallback reads a key
    /// that is not there. This is the run that proves DeployGardener's
    /// documented runbook (`export SEED_MYRRH_SCRY=…`) genuinely reverted.
    function test_F6_envOr_fallback_really_is_eager_which_is_why_it_is_banned() public {
        vm.setEnv("SCRY_AUDIT_F6_PRIMARY_SET", "0x00000000000000000000000000000000000000A1");

        // sanity: the primary really IS readable, so a revert below can only
        // come from the fallback. Without this the expectRevert is guesswork.
        assertEq(vm.envAddress("SCRY_AUDIT_F6_PRIMARY_SET"), address(0xA1));

        vm.expectRevert(); // from the FALLBACK, which a lazy default never runs
        this.envOrWithEnvAddressFallback();
    }

    function envOrWithEnvAddressFallback() external view returns (address) {
        // the banned shape: the fallback argument is evaluated unconditionally,
        // even though the primary key resolves
        return vm.envOr("SCRY_AUDIT_F6_PRIMARY_SET", vm.envAddress("SCRY_AUDIT_F6_FALLBACK_UNSET"));
    }

    /// And the replacement, exercised on the real script. Reaching a require()
    /// that names BOTH keys proves resolution got as far as the second key
    /// under its own control, rather than dying eagerly inside the first call.
    function test_F6_deploy_gardener_resolves_env_keys_sequentially() public {
        DeployGardener s = new DeployGardener();
        vm.expectRevert(bytes("set SCRY_AUDIT_PRIMARY_UNSET (or SCRY_AUDIT_FALLBACK_UNSET)"));
        s.envAddrOr("SCRY_AUDIT_PRIMARY_UNSET", "SCRY_AUDIT_FALLBACK_UNSET");
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

contract MockERC20 {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function approve(address s, uint256 v) external returns (bool) {
        allowance[msg.sender][s] = v;
        return true;
    }

    function transfer(address to, uint256 v) external returns (bool) {
        require(balanceOf[msg.sender] >= v, "balance");
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v;
        return true;
    }

    function transferFrom(address f, address t, uint256 v) external returns (bool) {
        require(balanceOf[f] >= v, "balance");
        uint256 a = allowance[f][msg.sender];
        require(a >= v, "allowance");
        if (a != type(uint256).max) allowance[f][msg.sender] = a - v;
        balanceOf[f] -= v;
        balanceOf[t] += v;
        return true;
    }
}

/// An arbiter that reenters the board while the job it is ruling on is still open.
contract EvilArbiter is IScryArbiter {
    ScryJobBoard public board;
    uint256 public target;
    bool public armed;

    function point(ScryJobBoard b) external {
        board = b;
    }

    function arm(uint256 id) external {
        target = id;
        armed = true;
    }

    function disarm() external {
        armed = false;
    }

    function feeRecipient() external view returns (address) {
        return address(this);
    }

    function flatFee() external pure returns (uint256) {
        return 0;
    }

    function rule(uint256, bytes32, bytes32) external returns (Verdict) {
        if (armed) {
            armed = false;
            board.close(target); // job status is still Delivered here
        }
        return Verdict.ForSeller;
    }
}

/// An arbiter that names its own fee and its own recipient.
contract GreedyArbiter is IScryArbiter {
    address public pocket = address(0xF00D);

    function feeRecipient() external view returns (address) {
        return pocket;
    }

    function flatFee() external pure returns (uint256) {
        return 998e18;
    }

    function rule(uint256, bytes32, bytes32) external pure returns (Verdict) {
        return Verdict.Undecided;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// The 2026-07-27 review of ScryGacha — three findings, same discipline.
//
// Written as DEFECT REPRODUCTIONS first (each passed by asserting the broken
// behaviour), then inverted in place once the fixes landed. The contract had
// never been read by anything but its own suite; all three came out of that
// first read, and none of them is reachable by an unprivileged caller alone —
// which is exactly why they were worth finding before a deploy rather than
// after.
// ═══════════════════════════════════════════════════════════════════════════

/// The minimum ERC-721 these three need, plus the pause switch that turns a
/// delivery into a stuck-token claim.
contract AuditNft {
    mapping(uint256 => address) public ownerOf;
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

    function transferFrom(address from, address to, uint256 id) external {
        require(!paused, "paused");
        require(ownerOf[id] == from, "wrong from");
        require(msg.sender == from || isApprovedForAll[from][msg.sender], "not authorized");
        ownerOf[id] = to;
    }
}

contract ScryGachaAuditTest is Test {
    ScryGacha internal g;
    AuditNft internal nft;
    MockArbSys internal arb;

    address internal constant ARBSYS = 0x0000000000000000000000000000000000000064;
    address internal alice = address(0xA11CE); // depositor
    address internal cody = address(0xC0DE); // buyer
    address internal attacker = address(0xBAD);
    address internal sink = address(0x51C0);

    function setUp() public {
        vm.etch(ARBSYS, address(new MockArbSys()).code);
        arb = MockArbSys(ARBSYS);
        arb.setL2Block(20_000_000);
        arb.setWindow(256);

        g = new ScryGacha(100, sink);
        nft = new AuditNft();
        g.openPool(address(nft), 0.01 ether, 1_000, 8_500);
        vm.deal(alice, 100 ether);
        vm.deal(cody, 100 ether);
        vm.prank(alice);
        nft.setApprovalForAll(address(g), true);
    }

    /// One position, so the draw is deterministic: whatever the seed, `_select`
    /// can only return that slot.
    function _depositAndDraw(uint256 tokenId, uint256 backing) internal returns (uint256 posId) {
        nft.mint(alice, tokenId);
        vm.prank(alice);
        posId = g.deposit{value: backing}(address(nft), tokenId);

        uint256 fee = g.fee(address(nft));
        vm.prank(cody);
        uint256 drawId = g.request{value: fee}(address(nft), bytes32("audit"), 0, 0, 0);
        (,,,,, uint64 revealBlock,,,) = g.draws(drawId); // 9th = toll, appended 2026-07-27
        arb.setL2Block(uint256(revealBlock) + 1);
        assertEq(g.settle(drawId), posId, "the only position must be the drawn one");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // G1 · `rescueStray` USED TO be able to take an NFT that was owed to a user.
    //      Every resolution path deletes `positionOfToken` BEFORE attempting
    //      delivery, and `_deliverNft` is best-effort — so a token whose
    //      transfer failed sat in the pool with `positionOfToken == 0` and a
    //      live `stuckNftRecipient`, which is precisely the state the rescue
    //      guard read as "loose". Its own comment claimed it "can never touch
    //      escrow"; that was false for exactly the case the stuck path exists
    //      for. FIXED 2026-07-27: a token->position claim is recorded on a
    //      failed delivery and the rescue refuses it.
    // ─────────────────────────────────────────────────────────────────────────
    function test_G1_rescueStrayCannotTakeATokenThatIsOwedToSomeone() public {
        uint256 posId = _depositAndDraw(1, 1 ether);

        // The collection stops working after the draw — the case `_deliverNft`
        // and `recoverStuckNft` exist for.
        nft.setPaused(true);
        vm.prank(cody);
        g.takeBid(posId); // ETH legs settle; the NFT cannot move
        assertEq(g.stuckNftRecipient(posId), alice, "alice is owed the token");
        assertEq(nft.ownerOf(1), address(g), "the pool still holds it");
        // The escrow record is RESTORED on a failed delivery, which is what makes
        // the rescue guard honest: the pool is still holding this token on
        // someone's behalf, so it must read as escrowed until they take it.
        assertEq(g.positionOfToken(address(nft), 1), posId, "the escrow record was restored");

        nft.setPaused(false);
        // The owner must not be able to take it out from under her.
        vm.expectRevert(ScryGacha.TokenIsEscrowed.selector);
        g.rescueStray(address(nft), 1, attacker);

        // And she can still collect it — after which it really is loose.
        vm.prank(alice);
        g.recoverStuckNft(posId);
        assertEq(nft.ownerOf(1), alice, "the recipient got her token");
        assertEq(g.positionOfToken(address(nft), 1), 0, "and the record is cleared on the way out");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // G2 · `setWindows` USED TO re-date positions that were already drawn.
    //      `choiceWindow`/`finalizeWindow` were global mutable storage read at
    //      resolution time against `pos.drawnAt`, and `setWindows(0, 0)` passed
    //      both bounds — so the owner could collapse a buyer's exclusive window
    //      to nothing AFTER the buyer had paid for it, letting the depositor
    //      immediately take the backing and push the token onto the buyer. The
    //      contract already froze `bidBps` at pool open for exactly this reason;
    //      the windows were the hole in the same promise. FIXED 2026-07-27: both
    //      deadlines are snapshotted into the position at settlement.
    // ─────────────────────────────────────────────────────────────────────────
    function test_G2_theWindowsCannotBeReDatedUnderAnAlreadyDrawnPosition() public {
        uint256 posId = _depositAndDraw(1, 10 ether);

        // Inside the buyer's window, the depositor cannot resolve.
        vm.prank(alice);
        vm.expectRevert(ScryGacha.BuyersWindow.selector);
        g.depositorReclaimNft(posId);

        // Zero is now refused outright — a window of nothing is not a window.
        vm.expectRevert(ScryGacha.ChoiceWindowTooShort.selector);
        g.setWindows(0, 0);

        // And shortening as far as the knob legally allows still must not reach a
        // position that was drawn under the old terms. This is the snapshot doing
        // the work, not the new minimum.
        g.setWindows(10 minutes, 10 minutes);
        skip(11 minutes);

        vm.prank(alice);
        vm.expectRevert(ScryGacha.BuyersWindow.selector);
        g.depositorReclaimBacking(posId);
        vm.prank(attacker);
        vm.expectRevert(ScryGacha.TooEarlyToFinalize.selector);
        g.finalize(posId);

        // The buyer's paid-for choice survives.
        vm.prank(cody);
        g.takeBid(posId);
        assertEq(g.owed(cody), 10 ether * 8_500 / 10_000, "the buyer kept the bid he paid for");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // G3 · `setPoolOpen` USED TO re-open a BLOCKED collection. `blockCollection`
    //      is owner-only and documented as one-way, but it only set
    //      `p.open = false` and left `p.exists` true, while `setPoolOpen` — open
    //      to the CURATOR, a seat documented as unable to touch money — checked
    //      nothing. So a curator could reverse the owner's emergency block, a
    //      strict escalation past an `onlyOwner` gate. FIXED 2026-07-27:
    //      re-opening checks the sticky block.
    // ─────────────────────────────────────────────────────────────────────────
    function test_G3_aBlockedCollectionStaysBlockedEvenForTheCurator() public {
        g.setCurator(attacker); // the curation seat, however it is held
        g.blockCollection(address(nft));

        vm.prank(attacker);
        vm.expectRevert(ScryGacha.CollectionIsBlocked.selector);
        g.setPoolOpen(address(nft), true);
        vm.expectRevert(ScryGacha.CollectionIsBlocked.selector);
        g.setPoolOpen(address(nft), true); // not even the owner

        // Closing a pool must still always be possible.
        vm.prank(attacker);
        g.setPoolOpen(address(nft), false);

        // And the block still holds against deposits.
        nft.mint(alice, 2);
        vm.prank(alice);
        vm.expectRevert(ScryGacha.PoolNotOpen.selector);
        g.deposit{value: 1 ether}(address(nft), 2);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // G4 · `_accrue` USED TO walk a position's fee checkpoint BACKWARDS. `_activate`
    //      deliberately CEILS the checkpoint at join so a joiner can never be paid
    //      for a distribution that preceded it; `_accrue` then assigned the FLOOR
    //      unconditionally, undoing that protection permanently. A depositor could
    //      trigger it on themselves for free with `reprice(id, itsOwnBacking)` and
    //      no value — a legal no-op. Nothing looked wrong at the time, because
    //      `_pending` saturates and paid nothing on that call; the position simply
    //      collected extra wei of OTHER depositors' fees on every later accrual.
    //      The second-order effect was the dangerous one: those extra wei come out
    //      of `feeCreditsOutstanding`, whose only cushion is truncation dust, and
    //      `-=` on a uint256 PANICS when it runs out — on the `withdraw`, `harvest`,
    //      `reprice` and `settle` paths. In a pool closed to new draws no fee can
    //      ever refill it, so backing and an escrowed NFT would be locked for good.
    //      FIXED 2026-07-27: the checkpoint only moves forward, and the counter
    //      decrements saturatingly so a wrong counter can never outrank a real exit.
    // ─────────────────────────────────────────────────────────────────────────
    function test_G4_aJoinCheckpointNeverMovesBackwards() public {
        // Two incumbents, then a draw so the accumulator holds a real value.
        nft.mint(alice, 10);
        nft.mint(alice, 11);
        vm.startPrank(alice);
        uint256 p1 = g.deposit{value: 1 ether}(address(nft), 10);
        uint256 p2 = g.deposit{value: 3 ether}(address(nft), 11);
        vm.stopPrank();

        uint256 fee = g.fee(address(nft));
        vm.prank(cody);
        uint256 drawId = g.request{value: fee}(address(nft), bytes32("g4"), 0, 0, 0);
        (,,,,, uint64 revealBlock,,,) = g.draws(drawId); // 9th = toll, appended 2026-07-27
        arb.setL2Block(uint256(revealBlock) + 1);
        uint256 drawn = g.settle(drawId);

        // A joiner arrives after that distribution and is owed nothing by it.
        nft.mint(alice, 12);
        vm.prank(alice);
        uint256 p3 = g.deposit{value: 1 ether}(address(nft), 12);
        // pendingFees returns (eth, toll) since the toll rail landed 2026-07-27.
        // Owed nothing means owed nothing on BOTH rails, so sum them.
        (uint256 joinEth, uint256 joinToll) = g.pendingFees(p3);
        assertEq(joinEth + joinToll, 0, "a joiner must be owed nothing at join");
        (,,,,,,, uint256 debtAtJoin,,,,,,) = g.positions(p3);

        // The free no-op that used to undo the ceiling.
        vm.prank(alice);
        g.reprice(p3, 1 ether);
        (,,,,,,, uint256 debtAfter,,,,,,) = g.positions(p3);
        assertGe(debtAfter, debtAtJoin, "the join checkpoint moved backwards");
        (uint256 stillEth, uint256 stillToll) = g.pendingFees(p3);
        assertEq(stillEth + stillToll, 0, "and the joiner is still owed nothing");

        // Harvesting an incumbent must still work, and the exit must stay open —
        // the counter is what used to brick this once the cushion was gone.
        uint256[] memory ids = new uint256[](1);
        ids[0] = drawn == p1 ? p2 : p1; // the survivor — the other one was drawn out
        vm.prank(alice);
        g.harvest(ids);
        vm.prank(alice);
        g.withdraw(p3);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // G5 · A VACANT top slot USED TO be free. `claimTop` demands 110% of the
    //      standing top, but `_activate` and `claimTop` both took an empty seat
    //      with no comparison at all — and the seat empties every time the
    //      top-backed position is drawn, withdrawn, or re-priced downward. So a
    //      position could wait for that, take the seat for the minimum backing,
    //      and collect `topShareBps` of every fee until somebody displaced it,
    //      while `topPositionId` no longer meant "the top-backed position".
    //      FIXED 2026-07-27: the seat carries the bar it was last held at.
    //      Nothing strands, because a vacant seat's slice folds back into the
    //      equal split.
    // ─────────────────────────────────────────────────────────────────────────
    function test_G5_aVacantTopSlotIsNotFree() public {
        uint256 posId = _depositAndDraw(20, 10 ether); // takes the seat, then is drawn out
        assertEq(g.topBar(address(nft)), 10 ether, "the bar is what the seat was held at");
        (,,,,,,,,,, uint256 top,,,) = g.poolCard(address(nft));
        assertEq(top, 0, "the seat empties when its position is drawn");
        assertEq(posId, 1);

        // The minimum backing must not inherit the whale's seat.
        nft.mint(alice, 21);
        vm.prank(alice);
        uint256 small = g.deposit{value: 0.01 ether}(address(nft), 21);
        (,,,,,,,,,, top,,,) = g.poolCard(address(nft));
        assertEq(top, 0, "a dust position took a whale's vacant seat");
        vm.prank(alice);
        vm.expectRevert(ScryGacha.TopNotBeaten.selector);
        g.claimTop(small);

        // Clearing the bar does take it.
        nft.mint(alice, 22);
        vm.prank(alice);
        uint256 bigger = g.deposit{value: 11 ether}(address(nft), 22);
        (,,,,,,,,,, top,,,) = g.poolCard(address(nft));
        assertEq(top, bigger, "clearing the bar takes the seat");
    }
}
