// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Script, console2} from "forge-std/Script.sol";
import {IPoolManager} from "v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "v4-core/src/types/PoolKey.sol";
import {Currency} from "v4-core/src/types/Currency.sol";
import {IHooks} from "v4-core/src/interfaces/IHooks.sol";
import {Hooks} from "v4-core/src/libraries/Hooks.sol";
import {HookMiner} from "v4-periphery/src/utils/HookMiner.sol";
import {ScryClock} from "../src/ScryClock.sol";

/// @title DeployScryClock — mine the address, deploy, and bind the pool in ONE run
///
/// @notice ⚠ **Deploy and initialize must happen in the same script run.**
///         The hook address is part of `PoolKey`, so anyone may open a pool
///         against our hook — including one full of worthless tokens, purely to
///         drive our clock. `ScryClock` binds to the first pool that
///         initializes and reverts on every later one, which turns that attack
///         into a race rather than a certainty. This script closes the race.
///         Splitting the two steps re-opens it.
///
/// @dev    ## The price is not a parameter, it is a commitment
///
///         `CLAUDE.md`: **a v3/v4 pool's first mint IS its price.** The
///         `SQRT_PRICE_X96` below must be derived from the live v3 MYRRH/OBOL
///         pool at deploy time, not inherited from this comment. The value
///         checked in here corresponds to **5 OBOL per MYRRH**, which is where
///         `ScryGarden` opened and where the v3 pair sat on 2026-07-31 —
///         10,010,411 OBOL against 2,001,922.60 MYRRH. If that has moved, this
///         constant is wrong and opening the pool at it is a gift to the first
///         arbitrageur.
///
///         Re-derive:
///           price       = amount1 / amount0   (raw units, currency1 per currency0)
///           sqrtPriceX96 = sqrt(price) * 2**96
///
///         With OBOL `0xa003af4a…` < MYRRH `0xde967108…`, currency0 is OBOL and
///         currency1 is MYRRH, so price = MYRRH/OBOL = 1/5 = 0.2.
contract DeployScryClock is Script {
    // ---- chain 4663, verified by eth_getCode 2026-07-31 (docs/onchain/HOOKS.md §0a) --
    IPoolManager constant POOL_MANAGER = IPoolManager(0x8366a39CC670B4001A1121B8F6A443A643e40951);

    // ---- the coins (contracts/deployments.json) -----------------------------
    address constant OBOL = 0xA003af4A6c38629A986545Afc8f9312C7EB76220;
    address constant MYRRH = 0xde967108cC27dB651e8cFeC7dd18db814508b893;

    /// @dev Foundry routes salted `new` in a broadcast through this canonical
    ///      factory, so the miner must grind against it and not against the
    ///      script's own address.
    address constant CREATE2_DEPLOYER = 0x4e59b44847b379578588920cA78FbF26c0B4956C;

    // ---- the pool -----------------------------------------------------------
    /// @dev A game venue wants a normal fee. The skim is separate from this and
    ///      only touches players; LPs earn this on every swap either way.
    uint24 constant LP_FEE = 10_000; // 1%, matching every other scry pool
    int24 constant TICK_SPACING = 200;

    /// @dev sqrt(0.2) * 2**96, computed at 60 digits of precision. RE-DERIVE —
    ///      see the note above; the live v3 pair read 5.0004 OBOL per MYRRH on
    ///      2026-07-31, and 5 is where `ScryGarden` opened.
    uint160 constant SQRT_PRICE_X96 = 35431911422859142059220343232;

    // ---- the game's immutables ---------------------------------------------
    // Not one of these can be changed after deploy. `docs/onchain/HOOKS.md` §5 records
    // which are still open; anything unconfirmed should stop this script.
    uint256 constant POT_BPS = 100; // 1% of the MYRRH received
    uint64 constant EXTENSION = 30 minutes; // minutes, never seconds (§2.1 fn 3)
    uint64 constant MAX_HORIZON = 24 hours;
    uint256 constant ROLLOVER_BPS = 1_000; // 10% seeds the next round

    function run() external {
        // OBOL sorts below MYRRH, so currency0 = OBOL and currency1 = MYRRH.
        // Buying MYRRH is therefore zeroForOne, and the pot accrues in MYRRH:
        // the prize is the capped coin and the ticket is the elastic one.
        require(OBOL < MYRRH, "currency order changed - re-sort the pool key");
        Currency currency0 = Currency.wrap(OBOL);
        Currency currency1 = Currency.wrap(MYRRH);
        Currency potCurrency = currency1; // MYRRH
        bool buyZeroForOne = true;

        uint160 flags = uint160(
            Hooks.BEFORE_INITIALIZE_FLAG | Hooks.AFTER_SWAP_FLAG | Hooks.AFTER_SWAP_RETURNS_DELTA_FLAG
        );

        bytes memory constructorArgs = abi.encode(
            POOL_MANAGER, potCurrency, buyZeroForOne, POT_BPS, EXTENSION, MAX_HORIZON, ROLLOVER_BPS
        );

        (address hookAddress, bytes32 salt) =
            HookMiner.find(CREATE2_DEPLOYER, flags, type(ScryClock).creationCode, constructorArgs);

        console2.log("mined hook address:", hookAddress);
        console2.logBytes32(salt);

        vm.startBroadcast();

        ScryClock clock = new ScryClock{salt: salt}(
            POOL_MANAGER, potCurrency, buyZeroForOne, POT_BPS, EXTENSION, MAX_HORIZON, ROLLOVER_BPS
        );
        require(address(clock) == hookAddress, "mined address mismatch");

        PoolKey memory key = PoolKey({
            currency0: currency0,
            currency1: currency1,
            fee: LP_FEE,
            tickSpacing: TICK_SPACING,
            hooks: IHooks(address(clock))
        });

        // Same run, same broadcast — this is the race closure, not a formality.
        POOL_MANAGER.initialize(key, SQRT_PRICE_X96);

        vm.stopBroadcast();

        console2.log("ScryClock:", address(clock));
        console2.log("bound:", clock.bound());
        console2.log("--- add this to contracts/deployments.json in THIS commit ---");
    }
}
