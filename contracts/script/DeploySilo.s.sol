// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryGranary.sol";
import "../src/ScrySilo.sol";

/// Deploy the Silo spoils lockers (FARMING.md, posted tokenomics):
/// silo -> posted tier menu -> OBOL bin 0 -> granary grant.
///
///   export PRIVATE_KEY=0x...
///   export GRANARY=0x...              # the deployed ScryGranary (the mint hub)
///   export OBOL_TOKEN=0x...           # the OBOL SpoilsToken, bin 0
///   # optional: the MYRRH parking bin (allocPoint 25 vs OBOL's 100):
///   # export MYRRH_TOKEN=0x...
///   # optional overrides of the posted numbers (FARMING.md section 8):
///   # export SILO_REWARD_PER_SECOND=1388888888888888  # ~120 OBOL/day
///   # export SILO_MAX_BREAK_BPS=5000                 # 50% same-second break burn
///   # export SILO_DAILY_CAP=250000000000000000000    # 250 OBOL/day granary cap
///
/// THE SILO EMITS WHATEVER THE GRANARY BINDS, and it must be given the
/// **OBOL** granary -- NOT the gardener's, which binds MYRRH since 2026-07-26
/// (FARMING.md 3a/3b). A ScryGranary holds one token's minter slot, so there
/// are two: `./deploy_town.sh gardener` builds the MYRRH one, `./deploy_town.sh
/// granary` builds the OBOL one, and the silo phase refuses to run without it
/// rather than silently taking the wrong granary and dripping MYRRH.
///
/// This script needs no change for any of that -- it already reads GRANARY from
/// the environment. Pass the right one.
///
/// (Superseded 07-25 note, kept because the parking bin still reads this way:
/// what used to be called the "MYRRH self-bin" is a MYRRH PARKING bin that
/// pays the granary's coin -- the premium coin gets somewhere to sit without
/// being emitted to fund its own lockup, which holds under either route.)
///   forge script script/DeploySilo.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// Wiring rule, same as the Gardener: the silo NEVER holds the SpoilsToken
/// minter role; it spends a posted daily grant from the granary. If the
/// broadcaster is the granary steward the grant is set inline; otherwise
/// the steward must call granary.setGrant(silo, cap) before rewards pay.
/// (Principal never depends on the grant: exits work with a dead grant.)
///
/// `forge test -vv` green before any broadcast. Always.
contract DeploySilo is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(pk);

        ScryGranary granary = ScryGranary(vm.envAddress("GRANARY"));
        SpoilsToken obol = SpoilsToken(vm.envAddress("OBOL_TOKEN"));

        // THE COIN CHECK, and it belongs here rather than only in the shell.
        // Two granaries exist and they bind different coins (FARMING.md §3a):
        // the gardener's mints MYRRH, this one must mint OBOL. `deploy_town.sh
        // silo` reads GRANARY_OBOL and dies without it, but the documented
        // MANUAL path exports the gardener's granary under the same `GRANARY`
        // name the driver writes — so the shell guard is bypassable by exactly
        // the invocation the docs print. A wrong-granary silo self-arms three
        // lines below (setGrant on whatever it was handed) and drips the wrong
        // coin forever, because ScrySilo.granary is immutable.
        // `spoils()` is a public immutable getter and both values are already
        // in scope; this is the same standard `status` applies to the Orchard's
        // `rewardToken()`. One staticcall, before anything broadcasts.
        require(
            address(granary.spoils()) == address(obol),
            "GRANARY binds a different coin than OBOL_TOKEN - this is the MYRRH granary"
        );

        uint256 rps = vm.envOr("SILO_REWARD_PER_SECOND", uint256(1_388_888_888_888_888)); // ~120/day
        uint256 maxBreak = vm.envOr("SILO_MAX_BREAK_BPS", uint256(5000)); // 50% at t=0
        uint256 dailyCap = vm.envOr("SILO_DAILY_CAP", uint256(250e18)); // 250/day

        vm.startBroadcast(pk);
        ScrySilo silo = new ScrySilo(granary, rps, maxBreak);
        // the posted seasons (FARMING.md section 8): the cJEWEL weight ladder
        silo.addTier(7 days, 10_000); // a week,    1x
        silo.addTier(30 days, 15_000); // a month,   1.5x
        silo.addTier(90 days, 20_000); // a quarter, 2x
        silo.addTier(180 days, 30_000); // half a year, 3x
        silo.addBin(obol, 100); // bin 0: OBOL, the launch bin
        address myrrhToken = vm.envOr("MYRRH_TOKEN", address(0));
        if (myrrhToken != address(0)) {
            silo.addBin(SpoilsToken(myrrhToken), 25); // bin 1: the MYRRH parking bin
        }
        if (granary.steward() == deployer) {
            granary.setGrant(address(silo), dailyCap);
        }
        vm.stopBroadcast();

        console2.log("ScrySilo   ", address(silo));
        console2.log("rewardPerSecond", rps);
        console2.log("maxBreakBps    ", maxBreak);
        if (granary.steward() != deployer) {
            console2.log("NOTE: broadcaster is not the granary steward;");
            console2.log("      steward must granary.setGrant(silo, dailyCap).");
        }
    }
}
