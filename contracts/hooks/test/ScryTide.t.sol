// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Test} from "forge-std/Test.sol";
import {Deployers} from "v4-core/test/utils/Deployers.sol";
import {PoolKey} from "v4-core/src/types/PoolKey.sol";
import {IHooks} from "v4-core/src/interfaces/IHooks.sol";
import {Hooks} from "v4-core/src/libraries/Hooks.sol";
import {LPFeeLibrary} from "v4-core/src/libraries/LPFeeLibrary.sol";
import {TickMath} from "v4-core/src/libraries/TickMath.sol";
import {SwapParams, ModifyLiquidityParams} from "v4-core/src/types/PoolOperation.sol";
import {PoolSwapTest} from "v4-core/src/test/PoolSwapTest.sol";
import {ScryTide} from "../src/ScryTide.sol";

/// @title ScryTide tests — the spec, written before anything compiled
///
/// @dev ⚠ **These have never run.** See `../BRINGUP.md`.
///
///      The wave arithmetic itself WAS verified independently — modelled in
///      Python over a full cycle for bounds, symmetry, monotonicity and the
///      timetable landing on the extremes. So the likely failures here are v4
///      plumbing (the dynamic-fee flag, the override flag), not the maths.
contract ScryTideTest is Test, Deployers {
    ScryTide tide;
    // `key` is Deployers', inherited — see ScryClock.t.sol.

    uint24 constant FEE_FLOOR = 3_000; // 0.30%
    uint24 constant FEE_AMP = 7_000; // peaks at 1.00%
    uint256 constant PERIOD = 24 hours;

    function setUp() public {
        deployFreshManagerAndRouters();
        deployMintAndApprove2Currencies();

        uint160 flags = uint160(Hooks.BEFORE_INITIALIZE_FLAG | Hooks.BEFORE_SWAP_FLAG);
        deployCodeTo("ScryTide.sol:ScryTide", abi.encode(manager, FEE_FLOOR, FEE_AMP, PERIOD), address(flags));
        tide = ScryTide(address(flags));

        // ⚠ The pool MUST be opened with the dynamic-fee flag, not a number.
        key = PoolKey(currency0, currency1, LPFeeLibrary.DYNAMIC_FEE_FLAG, 200, IHooks(address(tide)));
        manager.initialize(key, TickMath.getSqrtPriceAtTick(0));
        // Full range — see ScryClock.t.sol. Ticks must be multiples of the
        // key's tickSpacing (200) or v4 reverts TickMisaligned; ±120 is
        // Uniswap's stock LIQUIDITY_PARAMS, written for spacing 60.
        modifyLiquidityRouter.modifyLiquidity(
            key,
            ModifyLiquidityParams(
                TickMath.minUsableTick(key.tickSpacing), TickMath.maxUsableTick(key.tickSpacing), 100 ether, 0
            ),
            ""
        );
    }

    // ------------------------------------------------------------ the refusal

    /// @dev Without this guard the hook computes a fee, the pool ignores it,
    ///      and the pool charges its static fee forever. Nothing reverts and no
    ///      test fails — it simply is not a tide. Same class of silent
    ///      wrongness as writing the period in block numbers.
    function test_refuses_a_pool_that_is_not_dynamic_fee() public {
        PoolKey memory staticFee = PoolKey(currency0, currency1, 3000, 60, IHooks(address(tide)));
        vm.expectRevert();
        manager.initialize(staticFee, TickMath.getSqrtPriceAtTick(0));
    }

    // -------------------------------------------------------------- the wave

    function test_low_tide_is_the_floor_and_high_tide_is_the_peak() public {
        vm.warp(_atPhase(0));
        assertEq(tide.currentFee(), FEE_FLOOR, "phase 0 is low tide");

        vm.warp(_atPhase(PERIOD / 2));
        assertEq(tide.currentFee(), FEE_FLOOR + FEE_AMP, "the half-period is high tide");
    }

    function test_the_wave_is_symmetric() public view {
        for (uint256 p = 0; p < PERIOD; p += 977) {
            if (p == 0) continue;
            assertEq(tide.feeAt(p), tide.feeAt(PERIOD - p), "the two halves must mirror");
        }
    }

    function test_the_fee_never_leaves_its_posted_bounds() public view {
        for (uint256 p = 0; p < PERIOD * 3; p += 601) {
            uint24 f = tide.feeAt(p);
            assertGe(f, FEE_FLOOR, "never below the floor");
            assertLe(f, FEE_FLOOR + FEE_AMP, "never above the peak");
        }
    }

    /// @dev The property the whole product rests on: two traders in the same
    ///      block pay the same fee, and it depends on nothing but the clock.
    function testFuzz_the_fee_is_a_pure_function_of_time(uint32 t) public view {
        assertEq(tide.feeAt(t), tide.feeAt(t), "same input, same fee");
        assertEq(tide.feeAt(t), tide.feeAt(uint256(t) + PERIOD), "and it repeats every period");
    }

    // ---------------------------------------------------------- the timetable

    function test_the_timetable_lands_on_the_extremes() public {
        for (uint256 p = 0; p < PERIOD; p += 1319) {
            vm.warp(_atPhase(p));
            assertEq(tide.feeAt(tide.nextLowTide()), FEE_FLOOR, "nextLowTide must be low tide");
            assertEq(tide.feeAt(tide.nextHighTide()), FEE_FLOOR + FEE_AMP, "nextHighTide must be high tide");
            assertGe(tide.nextLowTide(), block.timestamp, "never in the past");
            assertGe(tide.nextHighTide(), block.timestamp, "never in the past");
            assertLe(tide.nextLowTide() - block.timestamp, PERIOD, "always within one period");
            assertLe(tide.nextHighTide() - block.timestamp, PERIOD, "always within one period");
        }
    }

    // -------------------------------------------------------------- the swap

    /// @dev ⚠ The one most likely to be wrong. A returned fee without
    ///      OVERRIDE_FEE_FLAG is silently discarded by the PoolManager, and the
    ///      pool charges its stored fee instead. That failure is invisible
    ///      except by comparing what a swap actually costs at two phases.
    function test_a_swap_actually_costs_more_at_high_tide() public {
        vm.warp(_atPhase(0));
        uint256 cheap = _swapCost();

        vm.warp(_atPhase(PERIOD / 2));
        uint256 dear = _swapCost();

        assertGt(dear, cheap, "high tide must actually cost more - if equal, the override flag is missing");
    }

    /// @dev R6. On Nitro `block.number` is the parent chain's. The tide must
    ///      not move when only the block height does.
    function test_the_tide_follows_wall_time_not_block_height() public {
        vm.warp(_atPhase(0));
        uint24 before = tide.currentFee();
        vm.roll(block.number + 500_000);
        assertEq(tide.currentFee(), before, "block height must not move the tide");
    }

    // ------------------------------------------------------------- no admin

    function test_nothing_can_move_the_bounds() public view {
        assertEq(tide.FEE_FLOOR(), FEE_FLOOR);
        assertEq(tide.FEE_AMP(), FEE_AMP);
        assertEq(tide.PERIOD(), PERIOD);
        // No owner, no setter. Adding one should break the build.
    }

    /// See ScryClock.t.sol for why this cannot be `vm.expectRevert(); new
    /// ScryTide(...)`: BaseHook validates the address bits before our
    /// constructor body runs, so a `new` always reverts HookAddressNotValid
    /// and the parameter guards are never reached.
    function _rejects(bytes memory args, string memory why) internal {
        uint160 flags = uint160(Hooks.BEFORE_INITIALIZE_FLAG | Hooks.BEFORE_SWAP_FLAG);
        address where = address(flags | (uint160(1) << 40));
        vm.etch(where, abi.encodePacked(vm.getCode("ScryTide.sol:ScryTide"), args));
        (bool ok, bytes memory ret) = where.call("");
        assertFalse(ok, why);
        assertEq(bytes4(ret), ScryTide.BadParameter.selector, "refused, but not for the parameter");
    }

    function test_constructor_rejects_a_pool_that_is_not_a_tide() public {
        // Amplitude zero is a fixed-fee pool wearing a costume.
        _rejects(abi.encode(manager, FEE_FLOOR, uint24(0), PERIOD), "zero amplitude must be refused");

        // An odd period makes the two halves different lengths, so low tide
        // drifts against the wall clock and the timetable stops being memorable.
        _rejects(abi.encode(manager, FEE_FLOOR, FEE_AMP, PERIOD + 1), "an odd period must be refused");

        // Above 10% it is not a tide, it is a closed pool with extra steps.
        _rejects(abi.encode(manager, uint24(50_000), uint24(60_000), PERIOD), "a fee above the cap must be refused");
    }

    // ------------------------------------------------------------- internals

    /// @dev Warp to a timestamp sitting at exactly `phase` within the cycle.
    function _atPhase(uint256 phase) internal view returns (uint256) {
        uint256 base = block.timestamp + PERIOD * 2;
        return base - (base % PERIOD) + phase;
    }

    /// @dev What a fixed-size swap actually costs, in input token.
    ///
    ///      `amountSpecified` is POSITIVE here on purpose — in v4 that means
    ///      exact-OUTPUT, which is what this measurement needs: pin the output
    ///      and let the input float, so a higher fee shows up as more spent.
    ///      (Note this is the opposite sign convention to `ScryClock.t.sol`'s
    ///      `_buy` helper, which flips it for readability.)
    function _swapCost() internal returns (uint256) {
        uint256 before = currency0.balanceOfSelf();
        swapRouter.swap(
            key,
            SwapParams({zeroForOne: true, amountSpecified: 1 ether, sqrtPriceLimitX96: TickMath.MIN_SQRT_PRICE + 1}),
            PoolSwapTest.TestSettings({takeClaims: false, settleUsingBurn: false}),
            ""
        );
        return before - currency0.balanceOfSelf();
    }
}
