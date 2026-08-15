// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryTrove.sol";

/// Deploy the shared pot's wrapper (`ScryTrove.sol`'s header is the design;
/// `contracts/RUNBOOK.md` is the deploy order).
///
/// **ONE deployment, ever — and this is the opposite of `DeployScryItem`.**
/// That script deploys once per catalog, because one ERC-721 holding five
/// games' skins renders as one incoherent marketplace collection. This one
/// deploys once for the whole platform, because **being a single address is its
/// entire function**: `ScryGacha` keys pools by collection, so everything that
/// shares this address shares one pot. A second trove would be a second pot,
/// which is the thing it exists to stop.
///
/// ⚠ **If a trove is already in `deployments.json`, do not run this.** Nothing
/// on chain stops you and the second one will look fine.
///
///   export PRIVATE_KEY=0x...
///   # optional:
///   # export TROVE_BASE_URI=https://scry.moreright.xyz/api/trove/
///
///   forge script script/DeployScryTrove.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// **On `TROVE_BASE_URI`.** It is **welded at deploy** — there is no
/// `setBaseURI`, deliberately, because a repointable base on a contract holding
/// other people's tokens is an admin door onto what a buyer thinks they are
/// drawing. An empty string is a legitimate choice: `tokenURI` then returns ""
/// and wallets fall back to the address, while `contentsOf` — which is the
/// truth — keeps answering. **The gacha never asks a collection for metadata**
/// (`IERC721Minimal` is `transferFrom` + `ownerOf`), so nothing about a draw
/// depends on this string.
///
/// ⚠ **On chain is welded.** Deploying is an operator act. There is no admin
/// key on this contract to fix a mistake with — no owner, no pause, no rescue —
/// which is the property that makes it safe to escrow into and also the reason
/// the address you get is the address you keep.
contract DeployScryTrove is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        string memory baseURI = vm.envOr("TROVE_BASE_URI", string(""));

        vm.startBroadcast(pk);
        ScryTrove trove = new ScryTrove(baseURI);
        vm.stopBroadcast();

        console2.log("ScryTrove      ", address(trove));
        console2.log("baseURI (welded)", baseURI);
        console2.log("");
        console2.log("Record it in contracts/deployments.json - broadcast/ is gitignored,");
        console2.log("so that file is the durable record.");
        console2.log("");
        console2.log("Then, and NOT before, open its pool:");
        console2.log("  cast send $SCRY_GACHA \\");
        console2.log("    'openPool(address,uint256,uint256,uint256)' \\");
        console2.log("    <trove> 10000000000000000 1000 8500");
        console2.log("");
        console2.log("Check these before believing the deploy - each is a property");
        console2.log("meter/gacha_fit.py will assert about it from outside:");
        console2.log("  supportsInterface(0x80ac58cd) -> true   (ERC-721)");
        console2.log("  supportsInterface(0xd9b67a26) -> false  (NOT ERC-1155)");
        console2.log("  owner()                       -> REVERTS (there is no owner)");
        console2.log("  paused()                      -> REVERTS (there is no pause)");
    }
}
