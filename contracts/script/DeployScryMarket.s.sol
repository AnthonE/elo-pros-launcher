// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryReputation.sol";
import "../src/ScryInsurancePool.sol";
import "../src/ScryJobBoard.sol";
import "../src/IScryArbiter.sol";

/// Deploy the agent-labor market spine to Robinhood Chain mainnet.
///
///   export PRIVATE_KEY=0x…
///   export SCRY_TOKEN=0xDa2a4b23459e9ca88183e990802be644AcA7C4B0
///   export SCRY_FEE_SPLITTER=0x…     # existing ScryFeeSplitter
///   export SCRY_ARBITER=0x…          # a deployed IScryArbiter (panel)
///   export SCRY_REP_THRESHOLD=50
///   export SCRY_MAX_PAYOUT=100000000000000000000000     # 100,000 SCRY ≈ $3.50
///   export SCRY_MAX_ARB_FEE=250000000000000000000000    # 250,000 SCRY ≈ $8.70, IMMUTABLE
///   forge script script/DeployScryMarket.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// Order: reputation + pool first, then the board pointing at both, then wire
/// the board as an authorized rep-mutator and pool claimant, then bind the
/// arbiter panel to this board. The board is the ONLY contract allowed to move
/// rep or claim from the pool.
///
/// ── DENOMINATION (re-based 2026-07-25; the old numbers were ~4 orders out) ──
/// Every figure here is in SCRY, and SCRY is NOT a dollar. Live at the time
/// of writing: **$0.0000348** (Uniswap v3 slot0 on RH-Chain, FDV ~$34.8k). The
/// original defaults — 10 SCRY for the arbitration ceiling, 1000 SCRY for a
/// claim cap — were written as if SCRY ≈ $1, which made them $0.00035 and
/// $0.035 respectively: an insurance pool that cannot pay a meaningful claim
/// and a dispute that cannot pay a real arbiter. The defaults below are sized
/// against real dollars at that price and SAY SO, so the next person can see
/// at a glance whether the price has moved out from under them:
///
///   arbiter flatFee   15,000 SCRY  ≈ $0.52   (DeployScryArbiter.s.sol)
///   SCRY_MAX_ARB_FEE 250,000 SCRY  ≈ $8.70   ~17× the fee — a CEILING, not a price
///   SCRY_MAX_PAYOUT  100,000 SCRY  ≈ $3.48   per claim; also the daily window
///
/// SCRY_MAX_ARB_FEE is the one number here that can NEVER be changed after the
/// broadcast. It bounds what any future arbiter — including one a later
/// operator points at — may charge the party opening a dispute, and disputes
/// are charged against a standing SCRY approval. Set it to a real arbitration
/// fee with headroom, never to type(uint256).max; an under-set ceiling shows up
/// as a refused dispute rather than as a drained wallet, which is the safe
/// direction to be wrong in. Raising it later means a redeploy.
///
/// ── INSURED MODE SHIPS CLOSED ────────────────────────────────────────────
/// The board's `insuredOpen` defaults to FALSE and this script does not flip
/// it (operator, 2026-07-25 — AUDIT-2026-07-25.md §4b). Escrow and
/// ReputationOnly go live; Insured waits on an underwriting step. When it is
/// opened, the POOL'S BALANCE is the real exposure limit — fund it with what
/// you would shrug at, not with what the cap suggests.
///
/// ⚠ Broadcast is a deliberate human step — real gas, real custody of escrow.
/// `forge test -vv` first, and sanity-check the arbiter is a real panel (not a
/// single oracle) before pointing money at it.
contract DeployScryMarket is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        IERC20 scry = IERC20(vm.envAddress("SCRY_TOKEN"));
        address feeSplitter = vm.envAddress("SCRY_FEE_SPLITTER");
        IScryArbiter arbiter = IScryArbiter(vm.envAddress("SCRY_ARBITER"));
        uint256 threshold = vm.envOr("SCRY_REP_THRESHOLD", uint256(50));
        // Sized in real dollars at SCRY = $0.0000348 (2026-07-25) — see the
        // DENOMINATION block above before changing either.
        uint256 maxPayout = vm.envOr("SCRY_MAX_PAYOUT", uint256(100_000e18)); // ≈ $3.48/claim
        uint256 maxArbFee = vm.envOr("SCRY_MAX_ARB_FEE", uint256(250_000e18)); // ≈ $8.70, IMMUTABLE
        // The upper bound is a "never type(uint256).max" sanity rail, not a
        // recommendation: 10,000,000 SCRY ≈ $348 at the price above.
        require(maxArbFee > 0 && maxArbFee <= 10_000_000e18, "SCRY_MAX_ARB_FEE: set a real ceiling, it is immutable");

        vm.startBroadcast(pk);
        ScryReputation reputation = new ScryReputation(threshold);
        ScryInsurancePool pool = new ScryInsurancePool(scry, maxPayout);
        ScryJobBoard board = new ScryJobBoard(
            scry, IReputation(address(reputation)), IInsurancePool(address(pool)), feeSplitter, arbiter, maxArbFee
        );

        reputation.setAuthority(address(board), true); // only the board moves rep
        pool.setClaimant(address(board), true); // only the board claims coverage

        // Bind the panel to this board. ONE-TIME and owner-only on the arbiter
        // side (audit 2026-07-25), so this only works when the broadcaster also
        // owns the panel; otherwise the panel's owner must call it themselves
        // and NO DISPUTE CAN BE VOTED ON until they do.
        bool bound = _tryBindArbiter(address(arbiter), address(board));
        vm.stopBroadcast();

        console.log("ScryReputation", address(reputation));
        console.log("ScryInsurancePool", address(pool));
        console.log("ScryJobBoard", address(board));
        console.log("maxArbFee (IMMUTABLE)", maxArbFee);
        console.log("insuredOpen", board.insuredOpen()); // ships FALSE by design
        if (bound) {
            console.log("arbiter bound to this board");
        } else {
            console.log("!! ARBITER NOT BOUND - its owner must call setBoard(board) ONCE");
            console.log("   until then every vote() reverts and no dispute can be ruled");
        }
    }

    /// `setBoard` is owner-only on the arbiter and may already be bound (or the
    /// arbiter may be a third-party IScryArbiter with no such function). A
    /// failure here is reported, never fatal — the market deploy is still good.
    function _tryBindArbiter(address arbiter, address board) internal returns (bool) {
        (bool ok,) = arbiter.call(abi.encodeWithSignature("setBoard(address)", board));
        return ok;
    }
}
