// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloTicketUrlRenderer.sol";
import "../src/EloGameTicket.sol";

/// Point one title's metadata at the origin instead of at contract storage —
/// the other end of the `setRenderer` hinge `EloGameTicket` posts.
///
/// **WHY A TITLE TAKES THIS TRADE.** On the built-in document, editing the art
/// or the blurb is a transaction and the fields are only what the contract can
/// hold. Through the renderer, an edit is a dev-desk save with no commit and no
/// gas, and the document carries what a marketplace actually renders —
/// `banner_image`, `featured_image`, numeric traits. The bill is a live
/// dependency on an origin, and the door out is one call: `setRenderer(0)`
/// takes the on-chain document back, so a dark origin costs a transaction and
/// never the asset. Gates takes this trade (`meter/tickets.py` header).
///
///   export PRIVATE_KEY=0x...
///   export RENDERER_GAME="gates"              # the slug, as the catalog spells it
///   # optional:
///   # export RENDERER_BASE=https://elopros.com/api    # ⚠ see below
///   # export RENDERER_OWNER=0x...             # default: the broadcasting key
///   # export RENDERER_TICKET=0x...            # the deployed ticket. Set it and
///   #                                         # this script ALSO calls
///   #                                         # setRenderer, which is the act
///   #                                         # that flips the metadata over
///
///   forge script script/DeployTicketRenderer.s.sol --rpc-url $RPC --broadcast
///
/// ⚠ **THE BASE CARRIES `/api` AND THE TICKET'S `baseURI` DOES NOT.** They
/// address two different roots and the difference is invisible until a wallet
/// fetches nothing:
///
///   TICKET_BASE_URI = https://elopros.com       -> …/game.html?slug=gates
///   RENDERER_BASE   = https://elopros.com/api   -> …/api/ticket/gates/1/metadata
///
/// `meter/tickets.py` declares its routes BARE because nginx strips the prefix
/// (`CLAUDE.md`'s named trap), so the bare origin here is a measured 404 and a
/// string test cannot see it. This script refuses a base with no `/api`
/// segment unless `RENDERER_ALLOW_BARE_BASE=1` says a self-hoster meant it —
/// the refusal costs a script run, and being wrong costs a collection of dead
/// metadata plus the transaction to fix it.
contract DeployTicketRenderer is Script {
    function run() external {
        string memory game = vm.envString("RENDERER_GAME");
        string memory base = vm.envOr("RENDERER_BASE", string(""));
        if (bytes(base).length == 0) {
            base = "https://elopros.com/api";
        }
        address rendererOwner = vm.envOr("RENDERER_OWNER", address(0));
        address ticket = vm.envOr("RENDERER_TICKET", address(0));
        bool allowBare = vm.envOr("RENDERER_ALLOW_BARE_BASE", uint256(0)) == 1;

        require(bytes(game).length != 0, "RENDERER_GAME is the slug and cannot be empty");
        require(
            allowBare || _hasApiSegment(base),
            "RENDERER_BASE has no /api segment - the meter is served under it and the bare "
            "origin 404s. Set RENDERER_ALLOW_BARE_BASE=1 if this origin really serves the "
            "meter at its root."
        );
        require(!_endsWithSlash(base), "RENDERER_BASE must not end in / - the renderer adds it");

        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        EloTicketUrlRenderer r = new EloTicketUrlRenderer(rendererOwner, base, game);
        // The flip, and it is deliberately a SEPARATE opt-in: deploying a
        // renderer changes nothing a wallet sees, and pointing a live ticket at
        // it changes what every holder's copy renders. One env var stands
        // between those two facts.
        if (ticket != address(0)) {
            EloGameTicket(payable(ticket)).setRenderer(address(r));
        }
        vm.stopBroadcast();

        console2.log("EloTicketUrlRenderer", address(r));
        console2.log("  game", game);
        console2.log("  base", base);
        // Echo the URLs the chain will hand a marketplace. `curl` them before
        // announcing anything — this is the one check a Solidity test cannot
        // run, and the whole reason the base is guarded above.
        console2.log("  tokenURI(1) ->", r.tokenURI(1));
        console2.log("  contractURI ->", r.contractURI());
        if (ticket != address(0)) {
            console2.log("  WIRED: ticket", ticket);
            console2.log("  the door out is setRenderer(address(0)) on that ticket");
        } else {
            console2.log("  NOT WIRED - the ticket still serves its own on-chain document.");
            console2.log("  To flip it, the ticket OWNER calls setRenderer with this address.");
        }
    }

    /// True when the base carries a `/api` path segment. Substring matching is
    /// enough and a full parser is not: what this catches is the origin typed
    /// with the segment missing, never a hostile string — the caller is the
    /// operator, holding the key, on their own box.
    function _hasApiSegment(string memory s) internal pure returns (bool) {
        bytes memory b = bytes(s);
        if (b.length < 4) return false;
        for (uint256 i = 0; i + 3 < b.length; i++) {
            if (b[i] == "/" && b[i + 1] == "a" && b[i + 2] == "p" && b[i + 3] == "i") {
                // `/api` must end the string or be followed by another segment,
                // so `/apiary.example.com` is not mistaken for it.
                if (i + 4 == b.length || b[i + 4] == "/") return true;
            }
        }
        return false;
    }

    function _endsWithSlash(string memory s) internal pure returns (bool) {
        bytes memory b = bytes(s);
        return b.length != 0 && b[b.length - 1] == "/";
    }
}
