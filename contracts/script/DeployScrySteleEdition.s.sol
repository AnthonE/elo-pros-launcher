// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScrySteleEdition.sol";

/// Deploy the stele-edition print shop to Robinhood Chain.
///   export PRIVATE_KEY=0x…
///   export SCRY_TOKEN=0xDa2a4b23459e9ca88183e990802be644AcA7C4B0   # SCRY
///   export SCRY_FEE_SPLITTER=0x…    # deployed ScryFeeSplitter
///   export STELE_PRICE_WEI=5000000000000000000   # flat SCRY per print (18-dec)
///   export SCRY_API_BASE=https://scry.moreright.xyz/api
///   # the LIVE vow register — prints of vows nobody swore are refused:
///   export SCRY_VOW_REGISTRY=0x08131e7660639bbd086dffa9375c2a563f1d3590
///   forge script script/DeployScrySteleEdition.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
/// Flat price, immutable at deploy. The vow stays soulbound; the edition is a
/// transferable cosmetic print, SCRY-in straight to the splitter.
///
/// SCRY_VOW_REGISTRY is immutable after deploy (audit 2026-07-25). Point it at
/// the register the prints are meant to refer to and check the address twice —
/// `mintEdition` asks it whether a vow exists, and a wrong register either
/// refuses every real vow or validates none of them. VERIFY IT AGAINST
/// `contracts/deployments.json` / the live deployment before broadcasting; it
/// is not guessable from the code.
contract DeployScrySteleEdition is Script {
    function run() external {
        address registry = vm.envAddress("SCRY_VOW_REGISTRY");
        require(registry.code.length > 0, "SCRY_VOW_REGISTRY: not a contract on this chain");
        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        ScrySteleEdition ed = new ScrySteleEdition(
            IERC20(vm.envAddress("SCRY_TOKEN")),
            vm.envAddress("SCRY_FEE_SPLITTER"),
            vm.envUint("STELE_PRICE_WEI"),
            vm.envString("SCRY_API_BASE"),
            IScryVowRegistryView(registry)
        );
        vm.stopBroadcast();
        console2.log("ScrySteleEdition", address(ed));
        console2.log("  vow registry (IMMUTABLE)", registry);
    }
}
