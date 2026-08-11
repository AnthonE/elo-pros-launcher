// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryGranary.sol";
import "../src/SpoilsToken.sol";

/// Deploy ONE ScryGranary bound to one SpoilsToken, and (if the deployer still
/// holds it) hand that token's minter role over.
///
/// WHY THIS EXISTS — the short version: `SpoilsToken` has exactly ONE `minter`
/// address, and a granary IS the thing that holds it. Two tokens therefore need
/// two granaries. That is not a design choice, it falls out of the token.
///
/// `DeployGardener.s.sol` already constructs a granary for the coin the FARM
/// pays, which since 2026-07-26 is **MYRRH** (FARMING.md 3a — the Garden is
/// MYRRH's only source). The **Silo** drips OBOL and spends a granary's grant
/// too, so it needs an OBOL-bound one, and nothing deployed it. Running
/// DeployGardener again would also mint a second Gardener and pool; this script
/// is the granary alone.
///
/// It is the same audited contract deployed twice, not a new one. No player
/// surface names a granary — `watchtower/js/gardens.js` reads only
/// `availableToday(gardener)` and renders it as a daily budget line — so the
/// two-granary shape is invisible in the UI by construction.
///
///   export PRIVATE_KEY=0x...
///   export GRANARY_TOKEN=0x...          # the SpoilsToken this granary binds
///   # optional, repeatable off-chain instead:
///   # export GRANT_TO=0x...             # an address to grant mint room to
///   # export GRANT_DAILY_CAP=250000000000000000000   # 250/day
///
///   forge script script/DeployGranary.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// The grant is OPTIONAL on purpose: the Silo's address does not exist yet when
/// this runs, so the normal order is deploy the granary, deploy the silo, then
/// `granary.setGrant(silo, cap)` as its own steward call. Setting GRANT_TO here
/// is for the case where you already have the address.
contract DeployGranary is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(pk);
        SpoilsToken token = SpoilsToken(vm.envAddress("GRANARY_TOKEN"));

        address grantTo = vm.envOr("GRANT_TO", address(0));
        uint256 grantCap = vm.envOr("GRANT_DAILY_CAP", uint256(250e18));

        vm.startBroadcast(pk);
        ScryGranary granary = new ScryGranary(token);
        if (grantTo != address(0)) {
            granary.setGrant(grantTo, grantCap);
        }
        if (token.minter() == deployer) {
            token.setMinter(address(granary));
        }
        vm.stopBroadcast();

        console2.log("ScryGranary ", address(granary));
        console2.log("bound token ", address(token));
        console2.log("token symbol", token.symbol());
        if (grantTo != address(0)) console2.log("granted     ", grantTo);
        if (token.minter() != address(granary)) {
            console2.log("NOTE: this token's minter is not the granary yet;");
            console2.log("      the current minter must setMinter(granary).");
        }
    }
}
