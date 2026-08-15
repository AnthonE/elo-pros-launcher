// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {ScryCistern, IScrySeat, ISwapRouter} from "../src/ScryCistern.sol";
import {IERC20} from "../src/IERC20.sol";

/// @title DeployCistern — the drop bar, broadcast
/// @notice The one gap between `RUNBOOK.md` §4f and a working bar: the contract
///         was written, documented and tested with no way to deploy it.
///
///   export RPC=...  PRIVATE_KEY=0x...
///   export CISTERN_SCRY=0xDa2a4b23459e9ca88183e990802be644AcA7C4B0
///   export CISTERN_SEAT=0x...              # the deployed ScrySeat. MUST exist first
///   export CISTERN_THRESHOLD=...           # whole SCRY. The posted bar
///   export CISTERN_TIP_BPS=...             # the crank's cut. Max 500
///   export CISTERN_ROUND_WINDOW=...        # seconds a round stays claimable
///   # export CISTERN_ROUTER=0x...          # v3 router; omit => every claim pays SCRY
///   forge script script/DeployCistern.s.sol --rpc-url $RPC          # DRY RUN FIRST
///   forge script script/DeployCistern.s.sol --rpc-url $RPC --broadcast
///
/// ⚠ THE SEAT DEPLOYS FIRST AND THIS IS NOT A PREFERENCE. The constructor calls
///    `seat.MAX_ELECTION()` and requires exactly 3, so pointing this at an
///    address with no code reverts the whole run. That check is the good kind:
///    it welds this contract's unrolled election width to the seat's, and the
///    two can never drift.
///
/// ⚠ THREE KNOBS HAVE NO OPERATOR SENTENCE AS OF 2026-08-12 — the threshold, the
///    tip and the window. Every one of them goes through `_mustUint` for the
///    reason `DeploySeat`'s own header gives: a default erases the difference
///    between a number somebody chose and a number nobody typed. Unlike the
///    seat's, none of these is immutable — `threshold` moves on a 3-day
///    timelock and the other two are plain setters — so a wrong one here is
///    recoverable. It should still be a choice.
///
/// ⚠ WHAT THIS SCRIPT DELIBERATELY DOES NOT DO: fund it. A deployed cistern with
///    an empty pot is the correct opening state, exactly as `DeploySeat` lands
///    with every door shut. Funding is `RUNBOOK.md` §4f and it is a later act.
contract DeployCistern is Script {
    /// Same sentinel idiom as `DeploySeat._mustUint`, same reasoning: no real
    /// value can be `type(uint256).max`, so it is safe as "unset", and 0 stays
    /// a typeable answer rather than an indistinguishable default.
    function _mustUint(string memory name) internal view returns (uint256 v) {
        v = vm.envOr(name, type(uint256).max);
        require(
            v != type(uint256).max,
            string.concat(
                "set ", name, " - this script will not guess a knob on a contract that "
                "distributes money. 0 is a valid answer where the contract allows it; it "
                "has to be typed"
            )
        );
    }

    function run() external {
        address scry = vm.envAddress("CISTERN_SCRY");
        address seat = vm.envAddress("CISTERN_SEAT");
        // address(0) is MEANINGFUL here and is not "unset": it makes every claim
        // pay SCRY with no swap, which is the honest opening state before any
        // pool is routed. `envOr` is correct for this one and wrong for the rest.
        address router = vm.envOr("CISTERN_ROUTER", address(0));

        // ⚠ WHOLE SCRY IN, 18-decimal UNITS OUT — the same conversion
        // `DeploySeat._costs` does and for the same reason: the env is typed by
        // a person, so it takes the unit a person says out loud. A threshold
        // typed in wei by mistake is a bar nothing will ever fill, and the
        // contract cannot tell the two apart.
        uint256 thresholdWhole = _mustUint("CISTERN_THRESHOLD");
        require(thresholdWhole > 0, "CISTERN_THRESHOLD: zero - the constructor rejects it");
        uint256 threshold = thresholdWhole * 1e18;

        uint256 tipRaw = _mustUint("CISTERN_TIP_BPS");
        require(tipRaw <= 500, "CISTERN_TIP_BPS is basis points - max 500 (MAX_TIP_BPS)");
        uint256 windowRaw = _mustUint("CISTERN_ROUND_WINDOW");
        require(windowRaw > 0 && windowRaw <= type(uint32).max, "CISTERN_ROUND_WINDOW out of range");

        // ⚠ PACE IT IN SECONDS, NOT BLOCKS, AND THE CONTRACT ALREADY DOES —
        // `RUNBOOK.md` §0c. On Nitro `block.number` is the PARENT chain's, so a
        // window counted in blocks would run ~120x long. This is only a warning
        // because `roundWindow` is seconds by construction; it is here because
        // the number a person types is where the confusion enters.
        if (windowRaw < 1 hours) {
            console2.log("WARNING: round window is under an hour");
            console2.log("  a holder who misses it loses nothing: after the window anyone");
            console2.log("  calls recycle and the remainder joins the NEXT round's pot.");
            console2.log("  Rounds do not block each other - open() reads pooled(), which");
            console2.log("  already nets off outstanding - so a stale unrecycled round");
            console2.log("  delays nothing. It only parks its own remainder until somebody");
            console2.log("  frees it. A short window just churns shares away from stragglers.");
        }

        console2.log("ScryCistern - the drop bar");
        console2.log("  scry", scry);
        console2.log("  seat", seat);
        console2.log("  threshold (whole SCRY)", thresholdWhole);
        console2.log("  tip bps", tipRaw);
        console2.log("  round window (seconds)", windowRaw);
        if (router == address(0)) {
            console2.log("  router NONE - every claim pays SCRY, no swap, no counterparty");
        } else {
            console2.log("  router", router);
            console2.log("  ...but no track is routed yet: coinOf is empty and track 0 is SCRY");
        }

        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        ScryCistern cis = new ScryCistern(
            IERC20(scry), IScrySeat(seat), ISwapRouter(router),
            threshold, uint16(tipRaw), uint32(windowRaw)
        );
        vm.stopBroadcast();

        console2.log("");
        console2.log("ScryCistern", address(cis));
        console2.log("");
        console2.log("DEPLOYED EMPTY, which is the correct opening state.");
        console2.log("Two things must be true before a round can open, and neither is yet:");
        console2.log("  1. pooled() >= threshold - fund it. A PLAIN SCRY TRANSFER COUNTS;");
        console2.log("     fill() is only for on-chain attribution. RUNBOOK.md 4f.");
        console2.log("     WETH SENT HERE IS STRANDED FOREVER - swap that leg first.");
        console2.log("  2. seat.totalWeight() > 0 - somebody has to have ACTIVATED a seat.");
        console2.log("");
        console2.log("Then ANY wallet calls open() and keeps the tip. Not ours to time,");
        console2.log("which is the whole invariant-9 argument and not a courtesy.");
        console2.log("");
        console2.log("Next: record the address in contracts/deployments.json as ScryCistern.");
    }
}
