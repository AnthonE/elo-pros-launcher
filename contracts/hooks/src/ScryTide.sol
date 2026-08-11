// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {BaseHook} from "@openzeppelin/uniswap-hooks/src/base/BaseHook.sol";
import {IPoolManager} from "v4-core/src/interfaces/IPoolManager.sol";
import {Hooks} from "v4-core/src/libraries/Hooks.sol";
import {LPFeeLibrary} from "v4-core/src/libraries/LPFeeLibrary.sol";
import {PoolKey} from "v4-core/src/types/PoolKey.sol";
import {BeforeSwapDelta, BeforeSwapDeltaLibrary} from "v4-core/src/types/BeforeSwapDelta.sol";
import {SwapParams} from "v4-core/src/types/PoolOperation.sol";

/// @title ScryTide — the fee is a wave, and the timetable is public
///
/// @notice **The game.** This pool's fee rises and falls on a fixed cycle,
///         forever. Cheap at low tide, dear at high tide, on a schedule anyone
///         can compute years ahead. That is the entire mechanism.
///
///         It tracks nothing. It predicts nothing. It is not a volatility
///         model and it is not trying to protect anyone from anything — those
///         hooks exist and this is not one. It turns a fee schedule into a tide
///         table people set alarms to, and that is the whole product.
///
/// @dev    ## Why a deterministic fee is safe and a random one is not
///
///         `docs/onchain/HOOKS.md` R7 kills every hook that rolls a die inside a swap:
///         the trader controls the transaction, so they wrap it and revert on a
///         loss, and a "random fee" is really a discount for whoever polls.
///
///         The corollary is what makes this contract work. **A fully public,
///         fully deterministic schedule has nothing to game, because nobody can
///         have an edge on it.** Waiting for low tide is not an exploit — it is
///         the intended behaviour and the joke. A predictable mechanism is a
///         feature; a random one is an option you granted for free.
///
///         So the fee here is a pure function of `block.timestamp` and three
///         immutables. No storage is read. No oracle is called. Two traders in
///         the same block always pay the same fee.
///
///         ## A triangle, not a sine, and that is a design decision
///
///         Integer sine on-chain needs a lookup table or an approximation, and
///         both cost gas to be wrong in a way nobody can check in their head.
///         A triangle wave is exact in integer math, costs a modulo and a
///         divide, and a trader can compute the next low tide mentally.
///
///         The point of this pool is legibility. Pick the waveform a person can
///         predict without a calculator.
///
///         ## R3 — nothing here is a discretionary power
///
///         `FEE_FLOOR`, `FEE_AMP` and `PERIOD` are `immutable` and public. There
///         is no owner, no setter, and no path by which the deployer can charge
///         you a different fee than the one this file computes. A dynamic fee
///         set by a hook is exactly the shape that WOULD be a discretionary
///         power in the swap path — welding the bounds at construction is what
///         makes it not one.
///
///         ## R6 — the clock
///
///         `block.timestamp` throughout. On Arbitrum Nitro, Solidity's
///         `block.number` is the PARENT chain's and advances once per ~12s while
///         the real chain runs at 101ms — a period written in blocks would be
///         ~119x longer than intended and nothing would fail. `contracts/RUNBOOK.md`
///         §0c. Timestamps are accurate on Nitro, so no `ArbSys` call is needed.
///
///         ## What LPs are owed, and it is not an APR
///
///         LPs earn less at low tide by construction. The honest disclosure for
///         this pool is **the tide table itself**, which `feeAt` and
///         `nextLowTide` exist to render — never a yield figure, which would be
///         a number we cannot honour.
contract ScryTide is BaseHook {
    using LPFeeLibrary for uint24;

    /// @notice The cheapest the fee ever gets, in hundredths of a bip.
    /// @dev    v4 fee units: 1_000_000 = 100%, so 3_000 = 0.30%.
    uint24 public immutable FEE_FLOOR;

    /// @notice How far above the floor high tide sits, same units.
    uint24 public immutable FEE_AMP;

    /// @notice A full cycle, in seconds — floor to peak and back to floor.
    uint256 public immutable PERIOD;

    /// @notice Ceiling on `FEE_FLOOR + FEE_AMP`. 10%.
    /// @dev    Not a protocol limit — v4 permits up to 100% — but a fee above
    ///         this is not a tide, it is a closed pool with extra steps.
    uint24 public constant MAX_TIDE_FEE = 100_000;

    event Charted(uint24 feeFloor, uint24 feeAmp, uint256 period);

    error NotDynamicFee();
    error BadParameter();

    constructor(IPoolManager _poolManager, uint24 _feeFloor, uint24 _feeAmp, uint256 _period)
        BaseHook(_poolManager)
    {
        // An odd period would make the two halves of the wave different
        // lengths, so low tide would drift against the wall clock and the
        // timetable would stop being memorable. Even, and at least an hour.
        if (_period < 1 hours || _period % 2 != 0) revert BadParameter();
        // Amplitude zero is a fixed-fee pool wearing a costume.
        if (_feeAmp == 0) revert BadParameter();
        if (uint256(_feeFloor) + uint256(_feeAmp) > MAX_TIDE_FEE) revert BadParameter();

        FEE_FLOOR = _feeFloor;
        FEE_AMP = _feeAmp;
        PERIOD = _period;
        emit Charted(_feeFloor, _feeAmp, _period);
    }

    /// @dev Flags: BEFORE_INITIALIZE (1<<13) | BEFORE_SWAP (1<<7) = 0x2080.
    ///      No return-delta flag: this hook overrides the FEE, which is a
    ///      separate channel from the swap delta and needs no permission bit.
    function getHookPermissions() public pure override returns (Hooks.Permissions memory) {
        return Hooks.Permissions({
            beforeInitialize: true,
            afterInitialize: false,
            beforeAddLiquidity: false,
            afterAddLiquidity: false,
            beforeRemoveLiquidity: false,
            afterRemoveLiquidity: false,
            beforeSwap: true,
            afterSwap: false,
            beforeDonate: false,
            afterDonate: false,
            beforeSwapReturnDelta: false,
            afterSwapReturnDelta: false,
            afterAddLiquidityReturnDelta: false,
            afterRemoveLiquidityReturnDelta: false
        });
    }

    /// @dev Refuse a pool that was not opened with the dynamic-fee flag.
    ///
    ///      Without this the hook would compute a fee on every swap and the
    ///      pool would silently ignore it, charging the static fee forever.
    ///      Nothing would revert and no test would fail — the pool would just
    ///      not be a tide. That is the failure mode this reverts to prevent,
    ///      and it is the same class of silent-wrongness as the Nitro clock.
    function _beforeInitialize(address, PoolKey calldata key, uint160) internal view override returns (bytes4) {
        if (!key.fee.isDynamicFee()) revert NotDynamicFee();
        return BaseHook.beforeInitialize.selector;
    }

    /// @dev The whole mechanism. A pure function of the clock.
    function _beforeSwap(address, PoolKey calldata, SwapParams calldata, bytes calldata)
        internal
        view
        override
        returns (bytes4, BeforeSwapDelta, uint24)
    {
        // ⚠ The fee must carry OVERRIDE_FEE_FLAG or the PoolManager ignores it
        //    and charges the pool's stored fee. That is a silent failure, not a
        //    revert — the same shape as the missing dynamic-fee flag above.
        uint24 fee = _feeAt(block.timestamp) | LPFeeLibrary.OVERRIDE_FEE_FLAG;
        return (BaseHook.beforeSwap.selector, BeforeSwapDeltaLibrary.ZERO_DELTA, fee);
    }

    // ------------------------------------------------------------- the timetable

    /// @notice The fee right now.
    function currentFee() external view returns (uint24) {
        return _feeAt(block.timestamp);
    }

    /// @notice The fee at any past or future moment. This is the tide table.
    /// @dev    Public so a surface can render the schedule instead of an APR,
    ///         and so a trader can verify the next low tide themselves rather
    ///         than trusting our page.
    function feeAt(uint256 timestamp) external view returns (uint24) {
        return _feeAt(timestamp);
    }

    /// @notice When the fee is next at its floor.
    function nextLowTide() external view returns (uint256) {
        uint256 phase = block.timestamp % PERIOD;
        return phase == 0 ? block.timestamp : block.timestamp + (PERIOD - phase);
    }

    /// @notice When the fee is next at its peak.
    function nextHighTide() external view returns (uint256) {
        uint256 half = PERIOD / 2;
        uint256 phase = block.timestamp % PERIOD;
        if (phase == half) return block.timestamp;
        return phase < half
            ? block.timestamp + (half - phase)
            : block.timestamp + (PERIOD - phase) + half;
    }

    /// @dev Triangle wave. Rises from `FEE_FLOOR` at phase 0 to
    ///      `FEE_FLOOR + FEE_AMP` at the half-period, then falls back.
    ///
    ///      No overflow risk: `tri <= half`, so the product is at most
    ///      `FEE_AMP * half` and `FEE_AMP` is bounded by MAX_TIDE_FEE.
    function _feeAt(uint256 timestamp) private view returns (uint24) {
        uint256 half = PERIOD / 2;
        uint256 phase = timestamp % PERIOD;
        uint256 tri = phase < half ? phase : PERIOD - phase;
        return FEE_FLOOR + uint24((uint256(FEE_AMP) * tri) / half);
    }
}
