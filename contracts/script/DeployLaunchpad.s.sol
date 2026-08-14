// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryLaunchpad.sol";

/// Deploy a title's launch vault — the contract the copy sale's money goes to.
///
/// ⚠ ORDER MATTERS AND IT IS NOT RECOVERABLE. The ticket's `proceeds` is
/// IMMUTABLE, so the vault must exist BEFORE the ticket and be passed as
/// `TICKET_PROCEEDS`. Deploy a ticket first and its raise is pointed at
/// whatever address it was given, forever; there is no setter and adding one
/// would break the promise `proceeds` is immutable to keep.
///
///   1. this script                                  -> ScryLaunchpad
///   2. DeployGameTicket.s.sol with
///      TICKET_PROCEEDS=<the vault>                  -> ScryGameTicket
///      (and TICKET_SINK=<the vault> only if the SCRY leg is meant to be
///       raise rather than burn — see the warning below)
///   3. transfer the game's coin to the vault        (FUNDED, never minted)
///   4. steward calls seed(...) with the opening price
///
/// ⚠ THE SCRY SINK IS A DIFFERENT PROMISE. Point the ticket's `scrySink` at the
/// fee splitter and half of every SCRY buy burns. Point it here and the SCRY
/// leg is raise instead. Both are legitimate; they are not the same thing, and
/// a card that says "half burns" over the second is false. `ScryGameTicket`'s
/// NOTICE no longer claims either, on purpose.
///
///   export PRIVATE_KEY=0x...
///   export LAUNCHPAD_SCRY=0x...          # chains.4663.contracts.SCRY
///   export LAUNCHPAD_COIN=0x...          # the game's own coin
///   export LAUNCHPAD_WETH=0x...          # canonical WETH on 4663
///   export LAUNCHPAD_NPM=0x...           # canonical v3 NonfungiblePositionManager
///   export LAUNCHPAD_ROUTER=0x...        # canonical v3 SwapRouter
///   export LAUNCHPAD_SCRY_FEES=0x...     # where the SCRY fee leg goes. IMMUTABLE
///   export LAUNCHPAD_COIN_FEES=0x...     # where the COIN fee leg goes. IMMUTABLE
///   export LAUNCHPAD_STEWARD=0x...       # may seed and convert. Never may withdraw
///   # optional:
///   # export LAUNCHPAD_POOL_FEE=10000    # the SCRY/coin tier. Default 1%
///   # export LAUNCHPAD_CONVERT_FEE=10000 # the tier `convert` swaps through
///
///   forge script script/DeployLaunchpad.s.sol --rpc-url $RPC --broadcast
///
/// TWO FEE LEGS, because a v3 position pays two tokens and a `ScryFeeSplitter`
/// handles one. The coin leg is where a game's own economy hangs: point it at a
/// splitter the studio deploys for its own coin, with `SCRY_BURN` as one
/// recipient and its own `ScryBank` as another, and fee income burns and banks
/// itself without anybody selling. Both instruments already exist here and both
/// take their token as a constructor argument. Point either leg at a plain
/// wallet instead if that is what the studio wants.
///
/// ⚠ NOT FORK-TESTED. The suite for this runs against MOCK v3 — it pins who may
/// call what and that no exit exists, and it proves nothing about tick math,
/// the ratio a real mint consumes, or router pathing. Run a fork test against
/// the canonical addresses before this moves any money.
contract DeployLaunchpad is Script {
    function run() external {
        address scry = vm.envAddress("LAUNCHPAD_SCRY");
        address coin = vm.envAddress("LAUNCHPAD_COIN");
        address weth = vm.envAddress("LAUNCHPAD_WETH");
        address npm = vm.envAddress("LAUNCHPAD_NPM");
        address router = vm.envAddress("LAUNCHPAD_ROUTER");
        address scryFees = vm.envAddress("LAUNCHPAD_SCRY_FEES");
        address coinFees = vm.envAddress("LAUNCHPAD_COIN_FEES");
        address steward = vm.envAddress("LAUNCHPAD_STEWARD");
        uint256 poolFee = vm.envOr("LAUNCHPAD_POOL_FEE", uint256(10000));
        uint256 convertFee = vm.envOr("LAUNCHPAD_CONVERT_FEE", uint256(10000));

        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        ScryLaunchpad launch = new ScryLaunchpad(
            IERC20(scry),
            IERC20(coin),
            IWETH9(weth),
            ILaunchPositionManager(npm),
            ISwapRouter(router),
            uint24(poolFee),
            uint24(convertFee),
            scryFees,
            coinFees,
            steward
        );
        vm.stopBroadcast();

        console2.log("ScryLaunchpad", address(launch));
        console2.log("  pass this as TICKET_PROCEEDS - the ticket's proceeds is IMMUTABLE");
        console2.log("  scry fee leg IMMUTABLE", scryFees);
        console2.log("  coin fee leg IMMUTABLE", coinFees);
        console2.log("  steward (may seed and convert, may never withdraw)", steward);
        console2.log("  tickLower", vm.toString(launch.tickLower()));
        console2.log("  tickUpper", vm.toString(launch.tickUpper()));
        console2.log("  NEXT: fund the coin side by transfer, then steward calls seed()");
    }
}
