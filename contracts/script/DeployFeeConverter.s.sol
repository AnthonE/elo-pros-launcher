// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryFeeConverter.sol";
import "../src/ScryLaunch.sol";

/// Deploy a title's fee converter — OPTIONAL, and it changes nothing about the
/// launch vault.
///
/// WHY. `ScryLaunch.collectFees()` pays whatever the v3 position owes, which is
/// BOTH legs: SCRY and the game's own coin. A studio paid in its own coin
/// accumulates a bag of the thing its players bought, and the only way to
/// realise it is to sell into the pool those players are standing in. pons V2
/// pays creators in ETH for exactly this reason. This is our version.
///
/// ⚠ IT DOES NOT REMOVE SELL PRESSURE. Converting fee coin into SCRY is still a
/// sale of that coin. What it removes is discretion and inventory — small,
/// continuous, visible sells instead of a bag somebody chooses a moment to
/// exit. Say the smaller claim.
///
/// HOW IT IS WIRED. Deploy this FIRST, then pass its address as
/// `LAUNCH_FEE_RECIPIENT` when deploying `ScryLaunch` — that field is immutable,
/// so this is a decision made once. Pointing a vault at a plain wallet instead
/// is a legitimate choice; it is just the one with the bag.
///
/// ⚠ NEVER point a ticket's `proceeds` here. This handles FEES. The raise goes
/// to `ScryLaunch`, which has no way to pay a person at all — that is the whole
/// design, and routing the raise through this contract would undo it.
///
///   export PRIVATE_KEY=0x...
///   export FEE_SCRY=0x...        # chains.4663.contracts.SCRY
///   export FEE_COIN=0x...        # the game's own coin
///   export FEE_ROUTER=0x...      # canonical v3 SwapRouter
///   export FEE_PAYEE=0x...       # the studio's wallet. IMMUTABLE
///   export FEE_STEWARD=0x...     # may choose the swap floor. Never may withdraw
///   # optional:
///   # export FEE_CONVERT_FEE=10000
///
///   forge script script/DeployFeeConverter.s.sol --rpc-url $RPC --broadcast
///
/// ⚠ NOT FORK-TESTED. Its suite runs against a mock router.
contract DeployFeeConverter is Script {
    function run() external {
        address scry = vm.envAddress("FEE_SCRY");
        address coin = vm.envAddress("FEE_COIN");
        address router = vm.envAddress("FEE_ROUTER");
        address payee = vm.envAddress("FEE_PAYEE");
        address steward = vm.envAddress("FEE_STEWARD");
        uint256 convertFee = vm.envOr("FEE_CONVERT_FEE", uint256(10000));

        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        ScryFeeConverter conv = new ScryFeeConverter(
            IERC20(scry), IERC20(coin), ISwapRouter(router), uint24(convertFee), payee, steward
        );
        vm.stopBroadcast();

        console2.log("ScryFeeConverter", address(conv));
        console2.log("  pass this as LAUNCH_FEE_RECIPIENT - that field is IMMUTABLE");
        console2.log("  payee IMMUTABLE", payee);
        console2.log("  steward (chooses the floor, may never withdraw)", steward);
    }
}
