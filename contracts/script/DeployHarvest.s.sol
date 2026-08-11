// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryHarvest.sol";

/// Deploy ScryHarvest — the merkle claim bridge for a GAME coin's harvest
/// ledger. This is the script TEST-AUDIT.md / SHIP.md flagged as missing:
/// DeployScryEconomy explicitly defers it (the harvest is the OBOL game-coin
/// claim, never funded by the SCRY splitter).
///
///   export PRIVATE_KEY=0x…
///   export OBOL_TOKEN=0x…          # the SpoilsToken OBOL from DeploySpoils
///   forge script script/DeployHarvest.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// The POOLS.md §3 step-3 runbook (RUNBOOK.md 7c), in order:
///   1. `forge test -vv` green on a real machine FIRST — always.
///   2. Broadcast this script → ScryHarvest(OBOL); the deployer is operator.
///   3. Fund BEFORE posting: mint `total_committed` OBOL TO the harvest
///      address — as the SpoilsToken minter, or via the granary grant to the
///      operator (`granary.stewardMint(harvest, total_committed)`) once the
///      granary holds the minter slot. `GET /tokens/claim-root?token=OBOL`
///      serves the root + total_committed over cumulative bridged balances.
///   4. `postRoot(root, total_committed, ledgerRef)` — only after funding.
///      The contract's "a posted root is a kept promise" invariant is
///      OPERATIONAL (fund-then-post), not enforced: postRoot never checks the
///      balance, and an under-funded root makes valid claims revert in the
///      token (pinned in test/ScryEconomy.t.sol).
///
/// The contract is token-generic (constructor takes a plain IERC20), so the
/// same script deploys a MYRRH harvest — point OBOL_TOKEN at MYRRH.
///
/// ⚠ Broadcast is a deliberate human act — real gas, and every root posted
/// afterward is a real claimable promise against the public ledger.
contract DeployHarvest is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        IERC20 obol = IERC20(vm.envAddress("OBOL_TOKEN"));
        runWith(pk, obol);
    }

    /// The parameterized entry, so a test can EXECUTE this script rather than
    /// merely compile it (AUDIT-2026-07-25.md §0(a): 19 of 20 deploy scripts
    /// ran in no test, and F6 was found in exactly that blind spot). Tests must
    /// call this, never `run()` — `vm.setEnv` is process-global and races under
    /// forge's parallel workers, the same trap `SeedSpoilsUniswapV3` hit.
    function runWith(uint256 pk, IERC20 token) public returns (ScryHarvest harvest) {
        // A harvest over the zero token is a brick: every claim reverts inside
        // SafeERC20 with no way to re-point it (`scry` is immutable), and the
        // failure only shows up after a root has been posted and published.
        // `vm.envAddress` is perfectly happy to hand back address(0).
        require(address(token) != address(0), "OBOL_TOKEN: zero token address");

        vm.startBroadcast(pk);
        harvest = new ScryHarvest(token);
        vm.stopBroadcast();

        console.log("ScryHarvest", address(harvest));
        console.log("  token (OBOL)", address(token));
        console.log("  operator = deployer; now: mint total_committed to the");
        console.log("  harvest, THEN postRoot (fund-then-post, RUNBOOK 7c)");
    }
}
