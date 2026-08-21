// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloCovenant.sol";

/// Deploy the Covenant to Robinhood Chain. Permissionless, ownerless, free —
/// one broadcast and it belongs to every fleet.
///   export PRIVATE_KEY=0x…
///   forge script script/DeployEloCovenant.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
/// After deploy: set SCRY_COVENANT in keys.env so the meter's /covenant surface
/// links each cohort to its on-chain register (CovenantOpened carries the full
/// oath text; Sworn/Renounced carry the roster — all explorer-readable).
contract DeployEloCovenant is Script {
    function run() external {
        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        EloCovenant covenant = new EloCovenant();
        vm.stopBroadcast();
        console2.log("EloCovenant", address(covenant));
    }
}
