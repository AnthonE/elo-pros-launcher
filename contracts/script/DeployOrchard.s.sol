// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/SpoilsToken.sol";
import "../src/EloOrchard.sol";
import {INonfungiblePositionManagerOrchard, IUniswapV3FactoryOrchard} from "../src/interfaces/IUniswapV3Orchard.sol";

/// Deploy the Orchard (FARMING.md section 9) - the v3-staker farm for the
/// canonical pools. Seasons are posted separately; this script only plants
/// the contract (and, optionally, posts season 0 if a pot is provided).
///
///   export PRIVATE_KEY=0x...
///   export REWARD_TOKEN=0x... # the coin a season pays: OBOL since 2026-07-25
///                             # (no MYRRH_TOKEN fallback - that key names a
///                             # different live coin since the granary split)
///   export NPM=0x...          # canonical NonfungiblePositionManager (POOLS.md 4.1
///   export V3_FACTORY=0x...   #   table; VERIFY ON-CHAIN at deploy, docs are not
///                             #   the authority - the chain is)
///   # optional season 0 (broadcaster must hold + approve the pot;
///   # fund it the harvest way: granary.stewardMint(broadcaster, pot) first):
///   # export SEASON_POOL=0x...            # the canonical v3 pool to pay
///   # export SEASON_POT=...               # reward wei; 0 = no season posted
///   # export SEASON_START=<unix>          # default: now + 1 hour
///   # export SEASON_END=<unix>            # default: start + 30 days
///   forge script script/DeployOrchard.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// THE ORCHARD PAYS **OBOL** — still true, and no longer for the reason this
/// header used to give. It said "like the Gardener and the Silo", which stopped
/// being so on 2026-07-26 when the Gardener moved to MYRRH (the Garden is
/// MYRRH's only source now, FARMING.md 3a). The Silo is the one it matches.
/// A season is funded by granary.stewardMint and a granary binds ONE
/// SpoilsToken, so a season pays whatever ITS granary holds — the **OBOL** one
/// (./deploy_town.sh granary), which is also the Silo's. FARMING.md 9 is the
/// standing rule: a season pays OBOL, and the orchard pays no MYRRH.
/// The contract is token-generic (it binds one SpoilsToken at construction),
/// so this is purely which address you export here. Point it at the same coin
/// the granary mints, or a stewardMint cannot fund a season.
///
/// The Orchard never holds a granary grant and cannot mint - a season's pot
/// moves at posting time or the season does not exist. `forge test -vv`
/// green before any broadcast. Always.
contract DeployOrchard is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");

        // REWARD_TOKEN names the coin a season pays (OBOL since 2026-07-25).
        // The MYRRH_TOKEN fallback the audit-era version honoured is GONE:
        // since the granary split MYRRH_TOKEN is a real town.env key naming a
        // DIFFERENT live coin, so honouring it here deploys a MYRRH-paying
        // Orchard the moment REWARD_TOKEN is merely unset - a wrong-coin
        // footgun, not a spelling courtesy (FARMING.md 9: a season pays
        // OBOL). deploy_town.sh's orchard phase exports
        // REWARD_TOKEN=OBOL_TOKEN; a bare forge-script run must name it too.
        address rewardAddr = vm.envOr("REWARD_TOKEN", address(0));
        require(rewardAddr != address(0), "set REWARD_TOKEN (the OBOL address)");
        SpoilsToken rewardToken = SpoilsToken(rewardAddr);
        address npm = vm.envAddress("NPM");
        address v3factory = vm.envAddress("V3_FACTORY");

        // the wrong-pairing guard the seed script also runs: the NPM must
        // actually mint against the factory we validate positions with.
        require(INonfungiblePositionManagerOrchard(npm).factory() == v3factory, "NPM/factory mismatch");

        uint256 pot = vm.envOr("SEASON_POT", uint256(0));

        vm.startBroadcast(pk);
        EloOrchard orchard =
            new EloOrchard(rewardToken, INonfungiblePositionManagerOrchard(npm), IUniswapV3FactoryOrchard(v3factory));
        if (pot > 0) {
            address pool = vm.envAddress("SEASON_POOL");
            uint256 startAt = vm.envOr("SEASON_START", block.timestamp + 1 hours);
            uint256 endAt = vm.envOr("SEASON_END", startAt + 30 days);
            rewardToken.approve(address(orchard), pot);
            orchard.createIncentive(pool, pot, startAt, endAt);
            console2.log("season 0 pool", pool);
            console2.log("season 0 pot ", pot);
            console2.log("season 0 window", startAt, endAt);
        }
        vm.stopBroadcast();

        console2.log("EloOrchard", address(orchard));
        if (pot == 0) {
            console2.log("NOTE: no season posted; fund one with stewardMint ->");
            console2.log("      approve -> createIncentive when ready.");
        }
    }
}
