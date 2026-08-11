// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Script, console2} from "forge-std/Script.sol";
import {IPoolManager} from "v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "v4-core/src/types/PoolKey.sol";
import {Currency} from "v4-core/src/types/Currency.sol";
import {IHooks} from "v4-core/src/interfaces/IHooks.sol";
import {Hooks} from "v4-core/src/libraries/Hooks.sol";
import {LPFeeLibrary} from "v4-core/src/libraries/LPFeeLibrary.sol";
import {HookMiner} from "v4-periphery/src/utils/HookMiner.sol";
import {ScryTide} from "../src/ScryTide.sol";

/// @title DeployScryTide — mine, deploy, and open a dynamic-fee pool
///
/// @notice ⚠ **There is no pool for this yet, and that is the blocker — not the
///         code.** Every hook is a NEW pool starting at zero depth (`HOOKS.md`
///         R1), and as of 2026-07-31 the house's uncommitted float is **94.78
///         OBOL and 178.13 MYRRH** — everything else is already pooled. The one
///         raidable source is halving the v3 MYRRH/OBOL pair (~$2,002) and the
///         Clock has first claim on it.
///
///         So `CURRENCY0`/`CURRENCY1` below are deliberately left as zero
///         addresses. Fill them when a pair and its depth actually exist, and
///         re-derive `SQRT_PRICE_X96` at that moment — a pool's first mint IS
///         its price.
///
/// @dev    Two things here are easy to get wrong and both fail SILENTLY:
///
///         1. **The pool's fee must be `LPFeeLibrary.DYNAMIC_FEE_FLAG`**, not a
///            number. Open it with a static fee and the hook computes a tide
///            the pool ignores forever. `ScryTide._beforeInitialize` reverts on
///            this, which is the only reason it is a loud failure.
///         2. **`beforeSwap` must OR the returned fee with
///            `OVERRIDE_FEE_FLAG`.** Without it the PoolManager discards the
///            hook's fee and charges the stored one. Nothing reverts.
///            `test_a_swap_actually_costs_more_at_high_tide` is what catches it.
contract DeployScryTide is Script {
    IPoolManager constant POOL_MANAGER = IPoolManager(0x8366a39CC670B4001A1121B8F6A443A643e40951);
    address constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    // ---- the pair — UNSET ON PURPOSE, see the notice above -----------------
    address constant CURRENCY0 = address(0);
    address constant CURRENCY1 = address(0);
    uint160 constant SQRT_PRICE_X96 = 0;

    int24 constant TICK_SPACING = 200;

    // ---- the wave, welded at construction ----------------------------------
    // v4 fee units: 1_000_000 = 100%. Low tide 0.30%, high tide 1.00%, over a
    // 24h cycle so the timetable is the same hour every day and a player can
    // hold it in their head. All three are immutable and cannot be retuned.
    uint24 constant FEE_FLOOR = 3_000;
    uint24 constant FEE_AMP = 7_000;
    uint256 constant PERIOD = 24 hours;

    function run() external {
        require(CURRENCY0 != address(0) && CURRENCY1 != address(0), "no pair chosen - see the notice");
        require(CURRENCY0 < CURRENCY1, "currencies must be sorted");
        require(SQRT_PRICE_X96 != 0, "re-derive the opening price - the first mint IS the price");

        uint160 flags = uint160(Hooks.BEFORE_INITIALIZE_FLAG | Hooks.BEFORE_SWAP_FLAG);
        bytes memory args = abi.encode(POOL_MANAGER, FEE_FLOOR, FEE_AMP, PERIOD);

        (address hookAddress, bytes32 salt) =
            HookMiner.find(CREATE2_DEPLOYER, flags, type(ScryTide).creationCode, args);
        console2.log("mined hook address:", hookAddress);

        vm.startBroadcast();

        ScryTide tide = new ScryTide{salt: salt}(POOL_MANAGER, FEE_FLOOR, FEE_AMP, PERIOD);
        require(address(tide) == hookAddress, "mined address mismatch");

        PoolKey memory key = PoolKey({
            currency0: Currency.wrap(CURRENCY0),
            currency1: Currency.wrap(CURRENCY1),
            fee: LPFeeLibrary.DYNAMIC_FEE_FLAG, // NOT a number. See §2 above.
            tickSpacing: TICK_SPACING,
            hooks: IHooks(address(tide))
        });
        POOL_MANAGER.initialize(key, SQRT_PRICE_X96);

        vm.stopBroadcast();

        console2.log("ScryTide:", address(tide));
        console2.log("low tide / high tide (hundredths of a bip):", FEE_FLOOR, FEE_FLOOR + FEE_AMP);
        console2.log("--- add this to contracts/deployments.json in THIS commit ---");
    }
}
