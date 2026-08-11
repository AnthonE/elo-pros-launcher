// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryGameDirectory.sol";
import "../src/ScryGameTicketFactory.sol";
import "../src/ScryGameTicket.sol";

/// Deploy the store's on-chain shelf and the ticket factory that feeds it.
///
/// WHAT THIS IS FOR. Today a listing is a hand `forge script`, an address typed
/// into `deployments.json`, and a `SCRY_TICKET_<SLUG>` env var on the box —
/// three records our own server is the sole authority on. After this, a
/// stranger reads one address and walks every listed title, its ticket, its
/// coin, its pool and the hash of the copy we are showing. That is what lets
/// somebody else run a mirror of the store, and what stops the next round of
/// address sprawl before it starts.
///
///   export PRIVATE_KEY=0x...
///   # optional: who owns the directory. Default: the broadcasting key.
///   # export DIRECTORY_OWNER=0x...
///   forge script script/DeployGameDirectory.s.sol --rpc-url $RPC --broadcast
///
/// The factory pins ONE ticket build: its `ticketCodeHash` is
/// `keccak256(type(ScryGameTicket).creationCode)` at the commit it was deployed
/// from. That is the whole known-code signal — a new ticket build means a new
/// factory, blessed with `setTrustedFactory`, and recorded in
/// `deployments.json` beside the old one. Do not try to make it mutable; a
/// factory that could be repointed attests nothing.
///
/// After the broadcast:
///   1. record BOTH addresses in `deployments.json`
///   2. `setTrustedFactory(<factory>, true)` — the deploy below does this
///   3. `setKeeper(<addr>, true)` for anyone else who curates
///   4. register the live titles (`register`), or list a new one in one act
///      (`listNew`)
contract DeployGameDirectory is Script {
    function run() external {
        address dirOwner = vm.envOr("DIRECTORY_OWNER", address(0));

        // The pin. Read at compile time from the very source in this tree, so
        // the factory and the ticket can never drift apart in a broadcast.
        bytes32 codeHash = keccak256(type(ScryGameTicket).creationCode);

        uint256 pk = vm.envUint("PRIVATE_KEY");
        address broadcaster = vm.addr(pk);

        vm.startBroadcast(pk);
        ScryGameTicketFactory factory = new ScryGameTicketFactory(codeHash);
        // Own it with the broadcasting key first: blessing the factory is an
        // owner act, and a directory born owned by somebody else could not do
        // it in this transaction. If a different owner is wanted, it is OFFERED
        // below and the handover completes when they accept — two steps, as
        // everywhere here, so a mistyped address costs a call and not the shelf.
        ScryGameDirectory directory = new ScryGameDirectory(broadcaster);
        directory.setTrustedFactory(address(factory), true);
        if (dirOwner != address(0) && dirOwner != broadcaster) {
            directory.transferOwnership(dirOwner);
        }
        vm.stopBroadcast();

        if (dirOwner != address(0) && dirOwner != broadcaster) {
            console2.log("OWNERSHIP OFFERED, NOT YET TRANSFERRED - call acceptOwnership() from", dirOwner);
        }

        console2.log("ScryGameTicketFactory", address(factory));
        console2.log("  pinned ticket code hash");
        console2.logBytes32(codeHash);
        console2.log("  permissionless - deploying is NOT listing");
        console2.log("ScryGameDirectory", address(directory));
        console2.log("  owner", directory.owner());
        console2.log("  reading is open to everyone; listing is keeper-gated by design");
    }
}
