// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryItem.sol";

/// Deploy one catalog's item contract (`docs/items/ITEM-CONTRACT.md`).
///
/// **One deployment per catalog.** One ERC-721 holding five
/// games' skins renders as one incoherent marketplace collection with a
/// meaningless floor, and a shared collection makes a `ScryGacha` pool's
/// harmonic-mean price mean nothing.
///
/// **The deployer becomes the creator-owner**, holds the mint, and can hand it
/// on with `transferOwnership` or close it forever with `renounceMint`. The
/// house does not hold a key on a third party's catalog — but for the first
/// catalog the house IS the creator, because Gates is ours.
///
///   export PRIVATE_KEY=0x...
///   export ITEM_NAME="Gates Items"
///   export ITEM_SYMBOL="GATE"
///   # optional:
///   # export ITEM_ROYALTY_RECEIVER=0x...   # ERC-2981, IMMUTABLE once deployed
///   # export ITEM_ROYALTY_BPS=0            # house catalogs deploy 0 (SENTENCES 2026-08-03); cap 1000
///   # export ITEM_BASE_URI=https://scry.moreright.xyz
///   # export ITEM_MINTER=0x...             # a second minter besides the deployer
///   # export ITEM_SEED_PLACEHOLDERS=8      # mint N placeholders to the deployer
///   # export ITEM_SEED_CATALOG=1           # which catalogId the placeholders claim
///
///   forge script script/DeployScryItem.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// **On the placeholders.** `ITEM_SEED_PLACEHOLDERS` exists to wire the
/// pipeline end to end before a catalog is real — mint a handful, watch them
/// land in a wallet, open a gacha pool over the collection, render them on
/// `inventory.html` and `market.html`. Their `mintRef` is
/// `keccak("placeholder" ‖ catalogId ‖ i)`, which is **deliberately not a WAL
/// reference**: a placeholder must never be mistakable for something that
/// dropped from play. Seed on a testnet, or seed a catalogId the content file
/// does not claim, and burn them before the catalog arms.
///
/// ⚠ **On chain is welded.** Deploying is an operator act and the address is
/// permanent. Record it in `contracts/deployments.json` in the same sitting —
/// foundry's `broadcast/` is gitignored, so that file is the durable record.
contract DeployScryItem is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(pk);

        string memory itemName = vm.envString("ITEM_NAME");
        string memory itemSymbol = vm.envString("ITEM_SYMBOL");
        address receiver = vm.envOr("ITEM_ROYALTY_RECEIVER", address(0));
        uint256 bps = vm.envOr("ITEM_ROYALTY_BPS", uint256(0));
        // `/api` is load-bearing: nginx only routes the meter under it, so a
        // baseURI without it mints tokens whose tokenURI resolves to nothing.
        string memory baseURI = vm.envOr("ITEM_BASE_URI", string("https://scry.moreright.xyz/api"));
        address extraMinter = vm.envOr("ITEM_MINTER", address(0));
        uint256 seedCount = vm.envOr("ITEM_SEED_PLACEHOLDERS", uint256(0));
        uint32 seedCatalog = uint32(vm.envOr("ITEM_SEED_CATALOG", uint256(1)));

        require(bps <= 1000, "ITEM_ROYALTY_BPS > 10%");

        vm.startBroadcast(pk);

        ScryItem item = new ScryItem(itemName, itemSymbol, receiver, uint96(bps), baseURI);
        item.setMinter(deployer, true);
        if (extraMinter != address(0)) item.setMinter(extraMinter, true);

        if (seedCount > 0) {
            bytes32[] memory refs = new bytes32[](seedCount);
            for (uint256 i = 0; i < seedCount; i++) {
                refs[i] = keccak256(abi.encodePacked("placeholder", seedCatalog, i));
            }
            item.mintBatch(deployer, seedCatalog, item.KIND_STORE(), refs);
        }

        vm.stopBroadcast();

        console2.log("ScryItem       ", address(item));
        console2.log("  name         ", itemName);
        console2.log("  symbol       ", itemSymbol);
        console2.log("  owner/creator", deployer);
        console2.log("  royalty bps  ", bps);
        console2.log("  royalty to   ", receiver);
        console2.log("  minters      ", item.minterCount());
        console2.log("  placeholders ", seedCount);
        console2.log("");
        console2.log("next: record the address in contracts/deployments.json (broadcast/ is gitignored),");
        console2.log("then SCRY_ITEM + the catalog's page config, then a gacha pool over this collection.");
    }
}
