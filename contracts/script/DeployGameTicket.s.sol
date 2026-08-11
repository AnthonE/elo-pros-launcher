// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryGameTicket.sol";

/// Deploy one title's ticket — the copy, the licence, the buy step
/// (`GATES.md` §4 rev. 2026-08-08; `SENTENCES.md` same date: $10 to start,
/// paid in SCRY / ETH / USDG, half of every SCRY buy burns via the splitter).
///
/// **One deployment per title.** The SCRY sink is the DEPLOYED ScryFeeSplitter
/// — that is what makes "half burns" a posted split rather than a promise —
/// and the proceeds default to the dev wallet per the standing 2026-07-29
/// sentence for this class.
///
///   export PRIVATE_KEY=0x...
///   export TICKET_NAME="Gates"                # the title, as the token name
///   export TICKET_SYMBOL="GATES-COPY"
///   export TICKET_GAME="gates"                # the slug the depot serves
///   # addresses — read the real ones out of deployments.json, never retype:
///   export TICKET_SCRY=0x...                  # chains.4663.contracts.SCRY
///   export TICKET_USDG=0x...                  # the USDG the x402 rail uses
///   export TICKET_SINK=0x...                  # chains.4663.contracts.ScryFeeSplitter
///   # optional:
///   # export TICKET_PROCEEDS=0x...            # default: the dev wallet
///   # export TICKET_SUPPLY_CAP=0              # 0 = uncapped (a copy sale has no thousand)
///   # export TICKET_OWNER=0x...               # default: the broadcasting key
///   # ⚠ TICKET_BASE_URI is the SITE base, not an api base, and it is used for
///   # ONE field — external_url. The metadata itself is built on chain, so a
///   # copy renders with the meter offline. Post the name, blurb and art with
///   # `setMetadata(blurb, image)` after the broadcast; `image` takes any URI a
///   # wallet can resolve (ipfs://, data:, https://).
///   # export TICKET_BASE_URI=https://scry.moreright.xyz
///   # the opening prices — post them in the same broadcast so the sale is
///   # never live unpriced. Amounts are per-asset at the posted dollar figure;
///   # derive them from the tape the day you broadcast, never from a doc:
///   # export TICKET_PRICE_USD_CENTS=1000      # $10.00
///   # export TICKET_PRICE_WEI=...             # $10 of ETH, in wei
///   # export TICKET_PRICE_SCRY=...            # $10 of SCRY, 18 decimals
///   # export TICKET_PRICE_USDG=10000000       # $10 of USDG (6 decimals)
///
///   forge script script/DeployGameTicket.s.sol --rpc-url $RPC --broadcast
///
/// After the broadcast: record the address in deployments.json as
/// `ScryGameTicket:GATES`, set `SCRY_TICKET_GATES=0x...` on the meter, and
/// the card, the entitlement check and the depot gate light up with no code
/// change (`meter/tickets.py`).
contract DeployGameTicket is Script {
    address constant DEV_WALLET = 0xb474a95200bC5de8950E445B1E9d524f4de0f18D;

    function run() external {
        string memory name_ = vm.envString("TICKET_NAME");
        string memory symbol_ = vm.envString("TICKET_SYMBOL");
        string memory game_ = vm.envString("TICKET_GAME");
        address scry = vm.envAddress("TICKET_SCRY");
        address usdg = vm.envAddress("TICKET_USDG");
        address sink = vm.envAddress("TICKET_SINK");
        address proceeds = vm.envOr("TICKET_PROCEEDS", DEV_WALLET);
        uint256 cap = vm.envOr("TICKET_SUPPLY_CAP", uint256(0));
        // Sequential, not an inline default: a url literal inside envOr's
        // argument list reads as a comment to the preflight scanner and
        // unbalances its walk (the DeployGardener.envAddrOr lesson, one
        // shape over).
        string memory base = vm.envOr("TICKET_BASE_URI", string(""));
        if (bytes(base).length == 0) {
            base = "https://scry.moreright.xyz";
        }

        uint256 usdCents = vm.envOr("TICKET_PRICE_USD_CENTS", uint256(0));
        uint256 priceWei = vm.envOr("TICKET_PRICE_WEI", uint256(0));
        uint256 priceScry = vm.envOr("TICKET_PRICE_SCRY", uint256(0));
        uint256 priceUsdg = vm.envOr("TICKET_PRICE_USDG", uint256(0));

        // address(0) => the broadcasting key owns it, which is what a hand
        // deploy has always meant. The parameter exists for the factory, which
        // must name an owner at birth rather than hand over afterwards.
        address ticketOwner = vm.envOr("TICKET_OWNER", address(0));

        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        ScryGameTicket t = new ScryGameTicket(
            name_, symbol_, game_, IERC20(scry), IERC20(usdg), sink, proceeds, cap, base, ticketOwner
        );
        if (usdCents != 0 || priceWei != 0 || priceScry != 0 || priceUsdg != 0) {
            t.setPrices(usdCents, priceWei, priceScry, priceUsdg);
        }
        vm.stopBroadcast();

        console2.log("ScryGameTicket", address(t));
        console2.log("  game", game_);
        console2.log("  scry sink (the splitter)", sink);
        console2.log("  proceeds  IMMUTABLE - sweep() can never be redirected", proceeds);

        // Echo what was actually posted. The amounts are typed by hand against
        // a tape on the day, so the failure that costs real money is a zero
        // miscounted — 0.025 ETH where 0.0025 was meant sells every copy at
        // ten times the posted dollar figure, and nothing on chain objects.
        // Step 5's check (`/api/ticket/gates` -> usd_now) is off chain and
        // comes after arming; this is the same fact at the moment of the act.
        console2.log("  price USD cents", usdCents);
        console2.log("  rail ETH   wei ", priceWei);
        console2.log("  rail SCRY      ", priceScry);
        console2.log("  rail USDG      ", priceUsdg);

        // Only when NOTHING was posted. Keying this on `usdCents` alone said
        // "every rail is closed" while an open rail sat two lines above it.
        if (usdCents == 0 && priceWei == 0 && priceScry == 0 && priceUsdg == 0) {
            console2.log("  NOTE: deployed UNPRICED - every rail is closed until setPrices()");
        }
        if (usdCents != 0 && priceWei == 0 && priceScry == 0 && priceUsdg == 0) {
            console2.log("  WARN: a dollar figure is posted but EVERY RAIL IS SHUT - nobody can buy");
        }
        if (usdCents == 0 && (priceWei != 0 || priceScry != 0 || priceUsdg != 0)) {
            console2.log("  WARN: a rail is open with NO dollar figure - the card cannot show drift");
        }
    }
}
