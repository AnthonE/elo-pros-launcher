// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryGranary.sol";
import "../src/SpoilsToken.sol";

/// Grant the cashier a POSTED daily mint cap on one granary, and nothing else.
///
/// WHAT THIS IS FOR. `meter/payout.py` pushes a real coin to a winner's wallet
/// in one transaction — the ledgerless payout. It does that by calling
/// `ScryGranary.mint(player, amount)`, which is gated by a per-address daily cap
/// the granary's steward sets — so this script is the whole security story of the
/// ledgerless payout, and it is one call.
///
/// WHY IT IS A SCRIPT AND NOT A PASTED `cast send`. The cap IS the blast radius of
/// a hot key that can mint. A number that important should be reviewed in a diff,
/// dry-run first, and printed back off the chain afterwards — not typed once into
/// a shell. Every field is REQUIRED and nothing is defaulted: a cap that appeared
/// because an env var was unset is not a posted cap.
///
///   export GRANARY=0x...        # the granary bound to the coin being paid
///   export CASHIER=0x...        # the payout wallet (SCRY_PAYOUT_FROM)
///   export CASHIER_DAILY_CAP=50000     # WHOLE coins per UTC day
///   forge script script/GrantCashier.s.sol --rpc-url $RPC          # dry run
///   forge script script/GrantCashier.s.sol --rpc-url $RPC --broadcast
///
/// TO REVOKE, and it is one call with no notice period: `revokeGrant(cashier)`
/// from the steward. That is deliberate and it is the other half of why a grant
/// is an acceptable place to put a hot key — lowering exposure is immediate,
/// exactly as `ScryTill` argues a cap lower should be.
///
/// WHAT THIS SCRIPT CANNOT DO: mint, move a minter slot, or touch a pool. It sets
/// one number on one address.
contract GrantCashier is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address steward = vm.addr(pk);
        ScryGranary granary = ScryGranary(vm.envAddress("GRANARY"));
        address cashier = vm.envAddress("CASHIER");
        // Whole coins in, raw units on the chain. The same units trap the payout
        // path carries (`payout.SPOILS_UNIT`, audit §0.3): a cap pasted in raw
        // units reads as a sane number and is 1e18 times too large.
        uint256 capWhole = vm.envUint("CASHIER_DAILY_CAP");
        require(capWhole > 0, "CASHIER_DAILY_CAP must be > 0 - a zero cap pays nobody");
        require(cashier != address(0), "CASHIER must be an address");
        uint256 cap = capWhole * 1e18;

        SpoilsToken token = granary.spoils();
        // Refuse rather than guess: if this key is not the steward the call
        // reverts anyway, and a named refusal beats a revert nobody can read.
        require(granary.steward() == steward, "this key is not the granary's steward");
        // A granary whose token does not point back at it can never mint, so a
        // grant on it is a promise that will fail at the first payout.
        require(token.minter() == address(granary),
            "this token's minter is NOT this granary - grant would be dead on arrival");

        console2.log("granary      ", address(granary));
        console2.log("coin         ", token.symbol());
        console2.log("cashier      ", cashier);
        console2.log("daily cap    ", capWhole);
        console2.log("cap was      ", granary.availableToday(cashier) / 1e18);

        vm.startBroadcast(pk);
        granary.setGrant(cashier, cap);
        vm.stopBroadcast();

        // Read it BACK off the chain rather than echoing the input — the whole
        // discipline of this repo's deploy scripts, and the only proof the grant
        // landed as posted.
        (uint256 dailyCap,,, bool enabled) = granary.grants(cashier);
        console2.log("enabled      ", enabled);
        console2.log("cap on chain ", dailyCap / 1e18);
        require(enabled && dailyCap == cap, "read-back disagrees with the grant");
        console2.log("");
        console2.log("Next: set SCRY_PAYOUT_FROM to the cashier, SCRY_PAYOUT=1,");
        console2.log("      SCRY_PAYOUT_GRANARIES, then GET /payout to read the cap back.");
    }
}
