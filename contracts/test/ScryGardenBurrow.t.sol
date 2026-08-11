// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "./MockToken.sol";
import "../src/ScryGarden.sol";
import "../src/ScryBurrow.sol";

contract ScryGardenBurrowTest is Test {
    MockToken gold; // collateral / token0
    MockToken tears; // debt / token1
    ScryGarden garden;
    ScryBurrow burrow;

    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    address whale = address(0x5A1E);

    function setUp() public {
        gold = new MockToken("garden token0", "TK0");
        tears = new MockToken("garden token1", "TK1");
        garden = new ScryGarden(IERC20(address(gold)), IERC20(address(tears)));
        burrow = new ScryBurrow(IERC20(address(gold)), IERC20(address(tears)), IGardenOracle(address(garden)));
        // Same balances the old three faucet rounds produced (3 × 1000e18 each),
        // so every assertion below keeps its original arithmetic.
        for (uint256 i = 0; i < 3; i++) {
            vm.warp(vm.getBlockTimestamp() + 1 days);
            gold.mint(whale, 1000e18);
            tears.mint(whale, 1000e18);
            gold.mint(alice, 1000e18);
            tears.mint(alice, 1000e18);
            gold.mint(bob, 1000e18);
            tears.mint(bob, 1000e18);
        }
        // whale seeds the garden 1:1 and donates lendable tears to the burrow
        vm.startPrank(whale);
        gold.approve(address(garden), type(uint256).max);
        tears.approve(address(garden), type(uint256).max);
        garden.addLiquidity(2000e18, 2000e18, 0, type(uint256).max);
        tears.transfer(address(burrow), 1000e18);
        vm.stopPrank();
        _approveAll(alice);
        _approveAll(bob);
    }

    function _approveAll(address who) internal {
        vm.startPrank(who);
        gold.approve(address(garden), type(uint256).max);
        tears.approve(address(garden), type(uint256).max);
        gold.approve(address(burrow), type(uint256).max);
        tears.approve(address(burrow), type(uint256).max);
        vm.stopPrank();
    }

    // ── garden ──────────────────────────────────────────────────────────────
    function test_spotPriceAndSwapMovesIt() public {
        assertEq(garden.spotPrice(), 1e18); // 1:1 seed
        vm.prank(alice);
        uint256 out = garden.swap(100e18, true, 0, type(uint256).max); // gold in → tears out
        assertGt(out, 0);
        assertLt(out, 100e18); // fee + slippage
        assertLt(garden.spotPrice(), 1e18); // gold got cheaper in tears
    }

    function test_lpRoundTripEarnsFees() public {
        vm.startPrank(alice);
        uint256 seeds = garden.addLiquidity(500e18, 500e18, 0, type(uint256).max);
        vm.stopPrank();
        // bob churns swaps — fees accrue to the pool
        vm.startPrank(bob);
        for (uint256 i = 0; i < 5; i++) {
            uint256 got = garden.swap(100e18, true, 0, type(uint256).max);
            garden.swap(got, false, 0, type(uint256).max);
        }
        vm.stopPrank();
        vm.startPrank(alice);
        (uint256 a0, uint256 a1) = garden.removeLiquidity(seeds, 0, 0, type(uint256).max);
        vm.stopPrank();
        // value out (at ~1:1) exceeds value in: the 0.3% fee is the yield
        assertGt(a0 + a1, 1000e18);
    }

    function test_slippageGuard() public {
        vm.prank(alice);
        vm.expectRevert(bytes("slippage"));
        garden.swap(100e18, true, 99e18, type(uint256).max); // demands more than x*y=k allows
    }

    // ── audit additions (TEST-AUDIT.md 2026-07-22) ──────────────────────────

    /// The constant-product law itself, asserted from exact reserve reads:
    /// k never decreases across a swap, and the 0.3% fee (which stays in the
    /// reserves) leaves it strictly larger. Matters beyond the playground:
    /// this same Garden code is what `DeploySpoils.s.sol` deploys as the ONE
    /// MYRRH/OBOL Garden the Gardener farms and the Burrow prices off.
    /// (It is NOT deployed against canonical SCRY — audit F5, 2026-07-25 —
    /// which is what this comment claimed until 2026-07-27.)
    function test_swapPreservesXykInvariant() public {
        (uint112 r0b, uint112 r1b) = garden.getReserves();
        uint256 kBefore = uint256(r0b) * uint256(r1b);
        vm.prank(alice);
        uint256 out = garden.swap(100e18, true, 0, type(uint256).max);
        (uint112 r0a, uint112 r1a) = garden.getReserves();
        uint256 kAfter = uint256(r0a) * uint256(r1a);
        assertGt(out, 0, "swap paid out");
        assertGe(kAfter, kBefore, "x*y=k violated: k decreased across a swap");
        assertGt(kAfter, kBefore, "the 0.3% fee must leave k strictly larger");
        // and the reverse direction holds the law too
        kBefore = kAfter;
        vm.prank(bob);
        garden.swap(50e18, false, 0, type(uint256).max);
        (r0a, r1a) = garden.getReserves();
        kAfter = uint256(r0a) * uint256(r1a);
        assertGe(kAfter, kBefore, "x*y=k violated on the reverse direction");
        assertGt(kAfter, kBefore, "fee grows k in the reverse direction too");
    }

    /// getAmountOut pinned against the fee formula recomputed here from exact
    /// reserve reads and the posted FEE_BPS = 30 (ScryGarden.sol):
    ///   out = in*(10000-30)*rOut / (rIn*10000 + in*(10000-30))
    /// and the executed swap must pay exactly that quote.
    ///
    /// The literal 30 is deliberate and must NOT be replaced by
    /// `garden.FEE_BPS()`: this test exists to catch the constant MOVING, and a
    /// formula that reads the constant it is checking cannot. It went 30 -> 100
    /// and back on 2026-07-27; ScryGarden.sol's notice carries why 0.3% is the
    /// number, and it is load-bearing now rather than an inherited default.
    function test_getAmountOutMatchesFeeFormula() public {
        assertEq(garden.FEE_BPS(), 30, "posted fee is 0.3%");
        (uint112 r0, uint112 r1) = garden.getReserves();
        uint256 amountIn = 123e18;
        uint256 inWithFee = amountIn * (10_000 - 30);
        uint256 expected = (inWithFee * uint256(r1)) / (uint256(r0) * 10_000 + inWithFee);
        uint256 quoted = garden.getAmountOut(amountIn, true);
        assertEq(quoted, expected, "quote must equal the recomputed 0.3% x*y=k formula");
        uint256 balBefore = tears.balanceOf(alice);
        vm.prank(alice);
        uint256 out = garden.swap(amountIn, true, expected, type(uint256).max); // minOut = the exact quote
        assertEq(out, expected, "swap pays the quote exactly");
        assertEq(tears.balanceOf(alice) - balBefore, expected, "payout landed in full");
    }

    // ── burrow ──────────────────────────────────────────────────────────────
    function _openPosition(address who, uint256 coll, uint256 debt) internal {
        vm.startPrank(who);
        burrow.depositCollateral(coll);
        burrow.borrow(debt);
        vm.stopPrank();
    }

    function test_borrowWithinLtv() public {
        _openPosition(alice, 100e18, 60e18); // exactly 60% at price 1
        // ±1 wei: borrowIndex > 1e18 by borrow time (setUp warps past mint
        // cooldowns), so the share round-trip floors — documented toy dust.
        assertApproxEqAbs(burrow.debtOf(alice), 60e18, 1);
        vm.prank(alice);
        vm.expectRevert(bytes("exceeds borrow LTV"));
        burrow.borrow(1e18);
    }

    function test_withdrawGuardHoldsLtv() public {
        _openPosition(alice, 100e18, 30e18);
        vm.prank(alice);
        burrow.withdrawCollateral(50e18); // still exactly 60% LTV
        vm.prank(alice);
        vm.expectRevert(bytes("would exceed borrow LTV"));
        burrow.withdrawCollateral(1e18);
    }

    function test_interestAccrues() public {
        _openPosition(alice, 100e18, 50e18);
        uint256 before = burrow.debtOf(alice);
        vm.warp(vm.getBlockTimestamp() + 365 days);
        uint256 debt = burrow.debtOf(alice);
        // ~10% simple APR (accrued lazily in one step here)
        assertApproxEqRel(debt, (before * 110) / 100, 0.01e18);
    }

    function test_repayClearsDebt() public {
        _openPosition(alice, 100e18, 50e18);
        vm.prank(alice);
        burrow.repay(type(uint256).max); // pays exactly current debt
        assertEq(burrow.debtOf(alice), 0);
        assertEq(burrow.healthFactor(alice), type(uint256).max);
    }

    function test_priceDumpLiquidation() public {
        _openPosition(alice, 100e18, 60e18); // healthy: 60 < 75
        assertGe(burrow.healthFactor(alice), 1e18);
        vm.prank(bob);
        vm.expectRevert(bytes("healthy"));
        burrow.liquidate(alice);
        // whale shoves the garden: dumps gold, gold price in tears collapses
        vm.startPrank(whale);
        gold.approve(address(garden), type(uint256).max);
        garden.swap(500e18, true, 0, type(uint256).max);
        vm.stopPrank();
        assertLt(burrow.healthFactor(alice), 1e18); // now underwater
        uint256 debt = burrow.debtOf(alice);
        uint256 bobGoldBefore = gold.balanceOf(bob);
        vm.prank(bob);
        burrow.liquidate(alice);
        assertEq(burrow.debtOf(alice), 0);
        uint256 seized = gold.balanceOf(bob) - bobGoldBefore;
        // seized = debt × 1.1 / price, CAPPED at the collateral that's there —
        // this dump pushes the position past the bonus, so the cap binds and
        // the liquidator eats the shortfall (the documented bad-debt zone).
        uint256 price = garden.spotPrice();
        uint256 uncapped = (debt * 11_000) / 10_000 * 1e18 / price;
        assertGt(uncapped, 100e18); // the cap really binds here
        assertEq(seized, 100e18); // all of alice's collateral
        assertEq(burrow.collateral(alice), 0);
        // the manipulable-oracle liquidation hunt is the game, at zero stakes
    }
}
