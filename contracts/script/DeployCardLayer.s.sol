// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloItem.sol";
import "../src/EloKiln.sol";

/// Deploy one title's card layer: the cards contract, the
/// crest contract, and the kiln between them — three deploys and one wire in a
/// single broadcast, because the wiring is where a hand ceremony goes wrong.
///
/// **Cards and crests are separate `EloItem` deploys and that is load-bearing**:
/// `EloGacha` keys pools by collection address with no
/// per-catalog filter, so the address is the unit of "what may be in a pool".
/// One contract for both would put penny cards in the crest pool. `EloKiln`'s
/// constructor refuses one address for both, so this script cannot make that
/// mistake even if edited to.
///
/// **What gets wired, and what deliberately does not.**
///   - the kiln becomes a minter on the CRESTS — that is what lets `fire` mint
///   - the deployer becomes a minter on the CARDS — drops, packs and store
///     copies all start here until a drop root replaces hand minting
///   - the deployer does NOT become a minter on the crests. Every crest's
///     provenance should be a `fire` — a set burned, on chain, checkable. The
///     door stays posted (`setMinter` is an owner call away, and the card
///     discloses `minterCount`), but this script does not open it.
///
///   export PRIVATE_KEY=0x...
///   export CARD_NAME="Gates Cards"       CARD_SYMBOL="GCARD"
///   export CREST_NAME="Gates Crests"     CREST_SYMBOL="GCREST"
///   # optional:
///   # ⚠ ROYALTY IS SPOKEN AND IT IS ZERO — operator 2026-08-09 ("0 fee is
///   #   good"), closing for the card layer what 2026-08-03 closed for the
///   #   pass. Both vars default to 0, so LEAVING THESE UNSET IS THE DECISION
///   #   rather than a shrug. Unlike the pass, `EloItem` welds BOTH the
///   #   receiver and the bps in its constructor and ships no setter, so a
///   #   later change is a redeploy and a new address, not a number edit.
///   # export CARD_ROYALTY_RECEIVER=0x... CARD_ROYALTY_BPS=0    # ERC-2981, IMMUTABLE; cap 1000
///   # export CREST_ROYALTY_RECEIVER=0x... CREST_ROYALTY_BPS=0
///   # export CARD_BASE_URI=https://elopros.com/api/items/cards
///   # export CREST_BASE_URI=https://elopros.com/api/items/crests
///   # a first set, defined in the same broadcast (skipped when unset).
///   # ⚠ These must MATCH data/cards/catalogs.json — the chain is what
///   #   EloKiln freezes, the catalog is only what names it. Gates set one:
///   # export SET_ID=1
///   # export SET_CARD_CATALOGS=1,2,3,4,5,6,7,8   # rows on the CARDS contract, no dupes
///   # export SET_CREST_CATALOG=1                 # row on the CREST contract
///   # export SET_MAX_LEVEL=5                    # Steam's 5
///
///   forge script script/DeployCardLayer.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// **A set defined here is still soft until it first fires.** `defineSet` may
/// correct composition until the first `fire` freezes it, so a
/// wrong catalog id at deploy time is recoverable — unlike everything else in
/// this script.
///
/// ⚠ **On chain is welded.** Three permanent addresses come out of this. Record
/// all three in `contracts/deployments.json` in the same sitting — foundry's
/// `broadcast/` is gitignored, so that file is the durable record. Then the
/// service half: `SCRY_CARDS`, `SCRY_CRESTS`, `SCRY_KILN` for `meter/cards.py`,
/// and the set's declaration in `data/cards/catalogs.json` by commit.
contract DeployCardLayer is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(pk);

        string memory cardName = vm.envString("CARD_NAME");
        string memory cardSymbol = vm.envString("CARD_SYMBOL");
        string memory crestName = vm.envString("CREST_NAME");
        string memory crestSymbol = vm.envString("CREST_SYMBOL");
        address cardReceiver = vm.envOr("CARD_ROYALTY_RECEIVER", address(0));
        uint256 cardBps = vm.envOr("CARD_ROYALTY_BPS", uint256(0));
        address crestReceiver = vm.envOr("CREST_ROYALTY_RECEIVER", address(0));
        uint256 crestBps = vm.envOr("CREST_ROYALTY_BPS", uint256(0));
        // The path shape `tokenURI` appends is welded (`/item/{id}/metadata`),
        // so WHICH collection a URL means has to live in the baseURI — and the
        // two contracts must not share one, or a wallet cannot tell card 5
        // from crest 5. These defaults resolve to the meter's routes.
        string memory cardBaseURI =
            vm.envOr("CARD_BASE_URI", string("https://elopros.com/api/items/cards"));
        string memory crestBaseURI =
            vm.envOr("CREST_BASE_URI", string("https://elopros.com/api/items/crests"));

        uint256 setId = vm.envOr("SET_ID", uint256(0));
        uint256[] memory setCatalogs = vm.envOr("SET_CARD_CATALOGS", ",", new uint256[](0));
        uint256 setCrestCatalog = vm.envOr("SET_CREST_CATALOG", uint256(0));
        uint256 setMaxLevel = vm.envOr("SET_MAX_LEVEL", uint256(5));

        require(cardBps <= 1000 && crestBps <= 1000, "ROYALTY_BPS > 10%");

        vm.startBroadcast(pk);

        EloItem cards = new EloItem(cardName, cardSymbol, cardReceiver, uint96(cardBps), cardBaseURI);
        EloItem crests = new EloItem(crestName, crestSymbol, crestReceiver, uint96(crestBps), crestBaseURI);
        EloKiln kiln = new EloKiln(address(cards), address(crests));

        crests.setMinter(address(kiln), true);
        cards.setMinter(deployer, true);

        if (setId != 0 && setCatalogs.length > 0) {
            uint32[] memory ids = new uint32[](setCatalogs.length);
            for (uint256 i = 0; i < setCatalogs.length; i++) {
                ids[i] = uint32(setCatalogs[i]);
            }
            kiln.defineSet(uint32(setId), ids, uint32(setCrestCatalog), uint8(setMaxLevel));
        }

        vm.stopBroadcast();

        console2.log("EloItem (cards) ", address(cards));
        console2.log("EloItem (crests)", address(crests));
        console2.log("EloKiln         ", address(kiln));
        console2.log("  owner/creator  ", deployer);
        console2.log("  crest minters  ", crests.minterCount()); // the kiln, and only the kiln
        console2.log("  card minters   ", cards.minterCount());
        if (setId != 0 && setCatalogs.length > 0) {
            console2.log("  set defined    ", setId);
            console2.log("    cards in set ", setCatalogs.length);
            console2.log("    max level    ", setMaxLevel);
        } else {
            console2.log("  no set defined - kiln.defineSet(...) when the catalog is authored");
        }
        console2.log("");
        console2.log("next: record ALL THREE addresses in contracts/deployments.json (broadcast/ is");
        console2.log("gitignored), then SCRY_CARDS / SCRY_CRESTS / SCRY_KILN for meter/cards.py, then");
        console2.log("the set's entry in data/cards/catalogs.json by commit. A gacha pool, if ever,");
        console2.log("opens over the CREST contract - never the cards: only a crest clears MIN_BACKING_FLOOR.");
    }
}
