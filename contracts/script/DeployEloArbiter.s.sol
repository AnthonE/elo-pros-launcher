// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloArbiter.sol";

/// Deploy the dispute panel (IEloArbiter) that EloJobBoard rules through.
///
///   export PRIVATE_KEY=0x…
///   export SCRY_ARB_FEE_RECIPIENT=0x…                # where the FLAT fee lands (fixed forever)
///   export SCRY_ARB_FLAT_FEE=15000000000000000000000  # 15,000 SCRY ≈ $0.52, in wei (18 dp)
///   # full 20-byte arbiter addresses, comma-separated (each a metered vow):
///   export SCRY_ARB_PANEL=0x1111111111111111111111111111111111111111,0x2222222222222222222222222222222222222222,0x3333333333333333333333333333333333333333
///   export SCRY_ARB_QUORUM=2                       # a STRICT majority of the panel
///   forge script script/DeployEloArbiter.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// Deploy this FIRST, then pass the printed address as SCRY_ARBITER to
/// DeployEloMarket.s.sol — and afterwards call `setBoard(board)` here ONCE, or
/// the panel cannot take a vote at all (see below). The fee and its recipient
/// are immutable: retuning the fee is a fresh deploy + board.setArbiter(new),
/// which keeps the fee's verdict-independence airtight.
///
/// ── DENOMINATION ─────────────────────────────────────────────────────────
/// The fee is SCRY, and SCRY is not a dollar: **$0.0000348** live on
/// 2026-07-25. The old "e.g. 2 SCRY" example was seven hundredths of a CENT.
/// 15,000 SCRY ≈ $0.52 is a fee a real panel might work for; it must sit well
/// under the board's immutable `maxArbFee` ceiling (250,000 SCRY ≈ $8.70 by
/// default in DeployEloMarket.s.sol) or every dispute reverts.
///
/// ── BIND THE PANEL TO ITS BOARD (audit 2026-07-25) ───────────────────────
/// `vote()` used to accept ANY jobId, including ids no board had issued yet, so
/// a panel could pre-rule a job before the evidence existed and `dispute()`
/// would settle on it instantly. `EloArbiter.setBoard` is now a ONE-TIME,
/// owner-only binding and voting is impossible until it is set. One-time on
/// purpose: an owner who could re-point the board could point it at a puppet
/// that reports whatever jobCount makes a stale verdict look fresh.
///
/// ⚠ Broadcast is a deliberate human step. `forge test -vv` first, and confirm
/// the panel is genuinely independent (≥3 distinct, metered arbiters — never a
/// single oracle) before pointing money at it.
contract DeployEloArbiter is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address feeRecipient = vm.envAddress("SCRY_ARB_FEE_RECIPIENT");
        uint256 flatFee = vm.envUint("SCRY_ARB_FLAT_FEE");
        address[] memory panel = vm.envAddress("SCRY_ARB_PANEL", ",");
        uint256 quorum = vm.envUint("SCRY_ARB_QUORUM");

        vm.startBroadcast(pk);
        EloArbiter arbiter = new EloArbiter(feeRecipient, flatFee, panel, quorum);
        vm.stopBroadcast();

        console.log("EloArbiter", address(arbiter));
        console.log("  feeRecipient", feeRecipient);
        console.log("  flatFee", flatFee);
        console.log("  panel size", panel.length);
        console.log("  quorum", quorum);
        console.log("  NEXT: deploy the board, then call setBoard(board) here ONCE");
        console.log("        until then vote() reverts with 'board not bound'");
    }
}
