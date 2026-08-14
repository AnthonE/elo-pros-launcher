// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryDeed.sol";

/// Deploy ScryDeed — the tabula: a house on a named street, as a REGULAR NFT
/// that is also a sink.
///
///   export PRIVATE_KEY=0x…
///   export OBOL_TOKEN=0x…          # the SpoilsToken OBOL from DeploySpoils
///   export FOUNDING_BURN=…         # OBOL burned to found a deed (18-dec units)
///   export CONVEYANCE_BURN=…       # OBOL burned on every hand-change
///   forge script script/DeployScryDeed.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// The deed is a sink BY DESIGN: set the burns high.
/// Land speculation is fine — every flip burns the conveyance toll, and an
/// elastic OBOL supply wants exactly this (POOLS.md §3.5). The plot space (how
/// much land exists) is the off-chain scarcity knob (SCRY_TABERNAE_STREETS),
/// not a cap in here.
///
/// Custody stays cap 0: this contract only ever mints to msg.sender and never
/// holds a key. Founding pulls+burns OBOL from the caller, so the deed layer
/// wires to the SpoilsToken OBOL (needs open burnFrom — it has it). Reputation
/// is NOT here: it stays soulbound to the vow at ScryReputation and never moves
/// with a deed. Written in an env without foundry — `forge test -vv` first.
contract DeployScryDeed is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address obol = vm.envAddress("OBOL_TOKEN");
        uint256 foundingBurn = vm.envUint("FOUNDING_BURN");
        uint256 conveyanceBurn = vm.envUint("CONVEYANCE_BURN");
        string memory baseURI = vm.envOr("BASE_URI", string("https://scry.moreright.xyz/api"));
        vm.startBroadcast(pk);
        ScryDeed deed = new ScryDeed(ISpoils(obol), foundingBurn, conveyanceBurn, baseURI);
        vm.stopBroadcast();
        console2.log("ScryDeed", address(deed));
        console2.log("founding burn", foundingBurn);
        console2.log("conveyance burn", conveyanceBurn);
    }
}
