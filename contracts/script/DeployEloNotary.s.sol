// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloNotary.sol";

/// Deploy the Notary to Robinhood Chain. Permissionless, ownerless, free —
/// one broadcast and it belongs to everyone.
///   export PRIVATE_KEY=0x…
///   forge script script/DeployEloNotary.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
/// After deploy: set SCRY_NOTARY in keys.env so the anchor worker posts the
/// daily augury seed commit here (label "augury seed commit YYYY-MM-DD") —
/// the commit-reveal stream becomes an explorer-readable public beacon.
contract DeployEloNotary is Script {
    function run() external {
        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        EloNotary notary = new EloNotary();
        vm.stopBroadcast();
        console2.log("EloNotary", address(notary));
    }
}
