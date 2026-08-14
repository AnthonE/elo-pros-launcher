// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryShrine.sol";

/// Deploy the Shrine - the on-chain votive mirror:
/// shrine -> OBOL altar (1 point/token) -> MYRRH altar (5 points/token,
/// the Book's posted base rate: 5 obols per myrrh).
///
///   export PRIVATE_KEY=0x...
///   export OBOL_TOKEN=0x...
///   export MYRRH_TOKEN=0x...
///   # optional override of the posted ladder step (rank 1 = 10 points):
///   # export SHRINE_BASE_STEP=10000000000000000000
///   forge script script/DeployShrine.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// The shrine needs no grant, no minter, no funding: it only burns. A rank
/// is a public record and buys nothing - not reputation, not access, not
/// odds. `forge test -vv` green before any broadcast. Always.
contract DeployShrine is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");

        SpoilsToken obol = SpoilsToken(vm.envAddress("OBOL_TOKEN"));
        SpoilsToken myrrh = SpoilsToken(vm.envAddress("MYRRH_TOKEN"));
        // 100e18, not 10e18: LAUNCH-DECISIONS.md (status: law) names "shrine
        // quantities" in the 10x redenomination, and the shrine was the one
        // surface that missed it. Everything else carries the sweep —
        // meter/augury.py AUGURY_BASE=100, meter/barrow_rules.py TOLL=10.0,
        // DeploySilo's ~120 OBOL/day — so a 10e18 step priced the rank ladder
        // an order of magnitude below every coin it is denominated in.
        // `ScryShrine.baseStep` is immutable with no setter, so `shrine --arm`
        // welds this, and nothing else in the repo would have caught it:
        // meter/test_tokenomics.py law 1 reads the Gardener and Silo defaults
        // and has no shrine entry at all.
        uint256 baseStep = vm.envOr("SHRINE_BASE_STEP", uint256(100e18)); // rank 1: 100 points

        vm.startBroadcast(pk);
        ScryShrine shrine = new ScryShrine(baseStep);
        shrine.addOffering(obol, 1); // altar 0: OBOL, 1 point per token
        shrine.addOffering(myrrh, 5); // altar 1: MYRRH, the Book's base rate
        vm.stopBroadcast();

        console2.log("ScryShrine ", address(shrine));
        console2.log("baseStep   ", baseStep);
    }
}
