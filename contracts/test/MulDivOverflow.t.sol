// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/Math.sol";
import "../src/SpoilsToken.sol";
import "../src/EloGranary.sol";
import "../src/EloGardener.sol";
import "../src/EloSilo.sol";
import "./MockToken.sol";

/// Regression for the 2026-07-29 review finding: `(a * b) / c` in the two
/// MasterChef update paths overflowed on the INTERMEDIATE product while the
/// quotient itself was always in range. It is not a rounding nicety — every
/// exit from both contracts routes through the update, and both `setAllocPoint`s
/// mass-update BEFORE mutating, so the call that would shrink the offending
/// `allocPoint` reverts too. No path home, principal trapped.
///
/// Each test here REINTRODUCES the bug first (asserting the old arithmetic
/// reverts) and then proves the shipped path does not, per the house rule that
/// a regression which never fails is not a regression.
contract MulDivOverflowTest is Test {
    SpoilsToken obol;
    SpoilsToken myrrh;
    MockToken lp;
    EloGranary granaryM;
    EloGranary granaryO;
    EloGardener gardener;
    EloSilo silo;

    address op = address(0xC0FFEE);
    address alice = address(0xA11CE);

    function setUp() public {
        obol = new SpoilsToken("obol", "OBOL", 0, op);
        myrrh = new SpoilsToken("myrrh", "MYRRH", 0, op);
        lp = new MockToken("lp", "LP");

        granaryM = new EloGranary(myrrh);
        gardener = new EloGardener(granaryM, 1e18, 6700, vm.getBlockTimestamp() + 90 days);
        granaryM.setGrant(address(gardener), type(uint128).max);
        vm.prank(op);
        myrrh.setMinter(address(granaryM));

        granaryO = new EloGranary(obol);
        silo = new EloSilo(granaryO, 1e18, 5000);
        granaryO.setGrant(address(silo), type(uint128).max);
        // mint alice her principal while `op` is still the minter - handing the
        // role to the granary first is what made this revert "minter only"
        vm.prank(op);
        obol.mint(alice, 10e18);
        vm.prank(op);
        obol.setMinter(address(granaryO));
    }

    // -- the library itself ---------------------------------------------------

    /// Agrees with plain arithmetic everywhere plain arithmetic works.
    function testFuzz_mulDiv_matches_plain_math_when_it_fits(uint128 a, uint128 b, uint128 d) public pure {
        vm.assume(d != 0);
        assertEq(Math.mulDiv(a, b, d), (uint256(a) * uint256(b)) / uint256(d), "mulDiv disagrees with a*b/c");
    }

    /// The whole point: the quotient fits even when the product does not.
    function test_mulDiv_survives_an_intermediate_that_overflows_256_bits() public pure {
        uint256 huge = type(uint256).max / 2;
        // huge * 4 overflows; (huge * 4) / 4 == huge does not.
        assertEq(Math.mulDiv(huge, 4, 4), huge, "mulDiv lost a value the quotient could hold");
        assertEq(Math.mulDiv(type(uint256).max, 1, 1), type(uint256).max);
    }

    /// It still refuses when the QUOTIENT genuinely does not fit.
    /// Through `this.` because Math is an internal library: inlined into the
    /// test, the revert happens at the cheatcode's own depth and expectRevert
    /// cannot see it.
    function test_mulDiv_reverts_when_the_result_itself_overflows() public {
        vm.expectRevert(bytes("mulDiv: overflow"));
        this.mulDivExt(type(uint256).max, type(uint256).max, 1);
    }

    function test_mulDiv_reverts_on_a_zero_denominator() public {
        vm.expectRevert();
        this.mulDivExt(1, 1, 0);
    }

    function mulDivExt(uint256 a, uint256 b, uint256 c) external pure returns (uint256) {
        return Math.mulDiv(a, b, c);
    }

    // -- the two contracts ----------------------------------------------------

    /// THE BUG, REINTRODUCED: the old expression, on the state this test puts
    /// the Gardener into, reverts. The shipped path must not.
    function test_gardener_survives_an_allocPoint_that_broke_the_old_math() public {
        // An allocPoint big enough that `emitted * allocPoint` leaves 256 bits.
        uint256 monstrous = type(uint256).max / 1e20;
        gardener.addPool(IERC20(address(lp)), monstrous);

        lp.faucet();
        lp.approve(address(gardener), type(uint256).max);
        gardener.deposit(0, 1e18);
        vm.warp(vm.getBlockTimestamp() + 30 days);

        uint256 emitted = gardener.emittedBetween(vm.getBlockTimestamp() - 30 days, vm.getBlockTimestamp());
        assertGt(emitted, 0, "nothing emitted - the test proves nothing");
        // the harness must actually reach the overflow, or the expectRevert
        // below would pass for the wrong reason
        assertGt(emitted, type(uint256).max / monstrous, "setup does not overflow the old math");

        // the old arithmetic, verbatim. `tap` is read FIRST on purpose: an
        // external call inside the arguments would consume the expectRevert.
        uint256 tap = gardener.totalAllocPoint();
        vm.expectRevert();
        this.oldMath(emitted, monstrous, tap);

        // the shipped arithmetic: no revert, and every exit still works
        gardener.updatePool(0);
        gardener.pendingMyrrh(0, address(this));
        gardener.setAllocPoint(0, 100); // the call that used to be unreachable
        gardener.withdraw(0, 1e18);
        assertEq(gardener.totalAllocPoint(), 100, "allocPoint could not be brought back down");
    }

    function test_silo_survives_an_allocPoint_that_broke_the_old_math() public {
        silo.addTier(30 days, 10_000);
        uint256 monstrous = type(uint256).max / 1e20;
        silo.addBin(obol, monstrous);

        vm.startPrank(alice);
        obol.approve(address(silo), type(uint256).max);
        silo.seal(0, 0, 1e18);
        vm.stopPrank();

        vm.warp(vm.getBlockTimestamp() + 30 days);

        uint256 elapsedRate = 30 days * silo.rewardPerSecond();
        assertGt(elapsedRate, type(uint256).max / monstrous, "setup does not overflow the old math");
        uint256 tap = silo.totalAllocPoint();
        vm.expectRevert();
        this.oldMath(elapsedRate, monstrous, tap);

        silo.updateBin(0);
        silo.setAllocPoint(0, 25); // used to be unreachable
        vm.prank(alice);
        silo.reap(0);
        assertEq(silo.totalAllocPoint(), 25, "allocPoint could not be brought back down");
    }

    /// `external` so `vm.expectRevert` can catch the arithmetic panic.
    function oldMath(uint256 a, uint256 b, uint256 c) external pure returns (uint256) {
        return (a * b) / c;
    }
}
