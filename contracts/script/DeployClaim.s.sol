// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryHarvest.sol";
import "../src/IERC20.sol";

/// Deploy the launch-airdrop claim contracts — one `ScryHarvest` merkle claim
/// per token (OBOL, MYRRH, SCRY), each posting the root that
/// `meter/claim_plan.py merkle` produced. Holders then claim on-chain from
/// watchtower/claim.html with their own wallet (no key held), and get baited
/// into the pools.
///
/// ONE INSTANCE, ONE ROOT PRODUCER — RUN THIS AGAIN FOR EVERY DROP (§0.4).
/// `ScryHarvest` stores `claimed[wallet]` as a LIFETIME total and pays
/// `cumulative - claimed`, so its roots are CUMULATIVE. `claim_plan.py` emits
/// PER-DROP ABSOLUTE amounts from one snapshot and knows nothing about earlier
/// drops. Point drop two at drop one's contracts and a wallet in both is
/// silently underpaid or zeroed:
///
///     drop one root:  alice -> 1,500     alice claims 1,500  (claimed = 1,500)
///     drop two root:  alice ->   900     alice claims 900 - 1,500 = NOTHING
///
/// So each drop gets a FRESH set from this script, and neither set may be the
/// bridge's `ScryHarvest` (DeployHarvest.s.sol), which is the third producer
/// and the only cumulative one. Record the addresses under
/// `chains.4663.claims["<drop_id>"]` in contracts/deployments.json in the same
/// commit as the broadcast — `contracts/preflight.py` gates on exactly that
/// and goes red if two drops ever share an address.
///
///   export PRIVATE_KEY=0x...
///   export OBOL_TOKEN=0x...   MYRRH_TOKEN=0x...   # from DeploySpoils
///   # SCRY_TOKEN defaults to canon on RH-Chain
///   # roots + totals from the published merkle JSON (0x0 root = skip that token):
///   export CLAIM_OBOL_ROOT=0x...   CLAIM_OBOL_TOTAL=<wei>
///   export CLAIM_MYRRH_ROOT=0x...  CLAIM_MYRRH_TOTAL=<wei>
///   export CLAIM_SCRY_ROOT=0x...   CLAIM_SCRY_TOTAL=<wei>
///   export CLAIM_REF="launch-<date>-b<block>"     # ledger reference (the drop_id)
///   forge script script/DeployClaim.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// FUNDING IS A SEPARATE ARMING CAST (this script never moves value — RUNBOOK
/// §2 rule). After deploy, transfer each token's total INTO its claim contract
/// or nobody can claim (claim() reverts "transfer failed" on an empty vault).
///
/// WHO CAN MINT DEPENDS ON WHEN, for BOTH coins — this block said "OBOL:
/// deployer is the minter" flatly until 2026-07-28, which is true only before
/// the `granary` phase. `DeployGranary.s.sol` does
/// `if (token.minter() == deployer) token.setMinter(address(granary))`, and
/// `rotate` moves it too, so read the slot off the chain before you type:
///   cast call $OBOL_TOKEN 'minter()(address)' --rpc-url $RPC
///
///   OBOL  before the `granary` phase / `rotate`, the deployer holds it:
///           cast send $OBOL_TOKEN  'mint(address,uint256)' <obol-claim>  <total>
///         after it, the OBOL granary does — and it must be GRANARY_OBOL, never
///         the bare $GRANARY alias, which names the gardener's MYRRH one:
///           cast send $GRANARY_OBOL 'stewardMint(address,uint256)' <obol-claim> <total>
///   MYRRH after the `gardener` phase the MYRRH granary holds the slot:
///           cast send $GRANARY_MYRRH 'stewardMint(address,uint256)' <myrrh-claim> <total>
///   SCRY  never minted — transferred from the treasury wallet:
///           cast send $SCRY_TOKEN 'transfer(address,uint256)' <scry-claim> <total>
///
/// The addresses are the ones this script prints; town.env records the granaries
/// as GRANARY_MYRRH and GRANARY_OBOL, and neither $OBOL nor $SCRY is a name
/// anything in this repo defines.
/// Then set on the meter: SCRY_CLAIM_DROP + SCRY_CLAIM_{OBOL,MYRRH,SCRY}=<addr>,
/// publish the merkle JSON into SCRY_CLAIM_DIR, and the website arms itself.
///
/// `forge test -vv` green (the Python<->Solidity merkle parity test in
/// ScryEconomy.t.sol is the load-bearing check) before any broadcast. Always.
contract DeployClaim is Script {
    address constant SCRY_CANON = 0xDa2a4b23459e9ca88183e990802be644AcA7C4B0;
    uint256 constant RH_CHAIN_ID = 4663;

    /// Everything the deploy needs as plain values. `run()` fills this from env
    /// for the CLI; tests build it in memory and call `runWith` — `vm.setEnv` is
    /// process-global and races under forge's parallel workers, the trap
    /// `SeedSpoilsUniswapV3` already paid for.
    struct ClaimInputs {
        uint256 pk;
        string ref;
        address obol;
        address myrrh;
        address scry;
        bytes32 obolRoot;
        bytes32 myrrhRoot;
        bytes32 scryRoot;
        uint256 obolTotal;
        uint256 myrrhTotal;
        uint256 scryTotal;
    }

    function run() external {
        ClaimInputs memory inp;
        inp.pk = vm.envUint("PRIVATE_KEY");
        inp.ref = vm.envOr("CLAIM_REF", string("launch-claim"));

        inp.obol = vm.envOr("OBOL_TOKEN", address(0));
        inp.myrrh = vm.envOr("MYRRH_TOKEN", address(0));
        inp.scry = vm.envOr("SCRY_TOKEN", block.chainid == RH_CHAIN_ID ? SCRY_CANON : address(0));

        inp.obolRoot = vm.envOr("CLAIM_OBOL_ROOT", bytes32(0));
        inp.myrrhRoot = vm.envOr("CLAIM_MYRRH_ROOT", bytes32(0));
        inp.scryRoot = vm.envOr("CLAIM_SCRY_ROOT", bytes32(0));
        inp.obolTotal = vm.envOr("CLAIM_OBOL_TOTAL", uint256(0));
        inp.myrrhTotal = vm.envOr("CLAIM_MYRRH_TOTAL", uint256(0));
        inp.scryTotal = vm.envOr("CLAIM_SCRY_TOTAL", uint256(0));

        runWith(inp);
    }

    /// @return obolClaim  the OBOL ScryHarvest, or address(0) if it was skipped
    /// @return myrrhClaim the MYRRH ScryHarvest, or address(0)
    /// @return scryClaim  the SCRY ScryHarvest, or address(0)
    function runWith(ClaimInputs memory inp)
        public
        returns (address obolClaim, address myrrhClaim, address scryClaim)
    {
        // A paste error that points two roots at ONE token deploys two claim
        // contracts over the same balance, each believing it owns the whole
        // stated obligation. Cheap to catch here; expensive after funding.
        _distinct("OBOL/MYRRH", inp.obol, inp.obolRoot, inp.myrrh, inp.myrrhRoot);
        _distinct("OBOL/SCRY", inp.obol, inp.obolRoot, inp.scry, inp.scryRoot);
        _distinct("MYRRH/SCRY", inp.myrrh, inp.myrrhRoot, inp.scry, inp.scryRoot);

        vm.startBroadcast(inp.pk);
        obolClaim = _one("OBOL", inp.obol, inp.obolRoot, inp.obolTotal, inp.ref);
        myrrhClaim = _one("MYRRH", inp.myrrh, inp.myrrhRoot, inp.myrrhTotal, inp.ref);
        scryClaim = _one("SCRY", inp.scry, inp.scryRoot, inp.scryTotal, inp.ref);
        vm.stopBroadcast();

        console2.log("---- launch claim contracts (fund each before opening) ----");
        console2.log("OBOL  claim", obolClaim);
        console2.log("MYRRH claim", myrrhClaim);
        console2.log("SCRY  claim", scryClaim);
        console2.log("set on the meter: SCRY_CLAIM_DROP + SCRY_CLAIM_{OBOL,MYRRH,SCRY}");
    }

    /// Two tokens that are both being deployed must not be the same address.
    /// A token nobody is deploying for (zero root) is not compared — the SCRY
    /// default is populated on RH-Chain whether or not a SCRY drop is armed.
    function _distinct(string memory pair, address a, bytes32 aRoot, address b, bytes32 bRoot) internal pure {
        if (aRoot == bytes32(0) || bRoot == bytes32(0)) return;
        require(a != b, string.concat(pair, ": both roots point at the SAME token address"));
    }

    /// Deploy one ScryHarvest for `token` and post its root, or skip (address 0)
    /// when the root is zero — a token can be armed later on its own.
    function _one(string memory name, address token, bytes32 root, uint256 total, string memory ref)
        internal
        returns (address)
    {
        if (root == bytes32(0)) {
            console2.log(string.concat(name, ": no root set, skipped"));
            return address(0);
        }
        require(token != address(0), string.concat(name, ": token address unset"));
        // §M4 at deploy time. `postRoot` cannot check a total against the tree,
        // and `owedUnderRoot` IS the sweep floor — so a real root posted with a
        // zero total is a published promise the contract will not defend for a
        // single block. The SWEEP_DELAY window does not cover it either: that
        // floor is `priorOwed`, which is 0 on a contract whose first root this
        // is. A zero total here is always a typo; refuse it before broadcast.
        require(total > 0, string.concat(name, ": root is set but the total is 0 - the sweep floor would be zero"));
        ScryHarvest h = new ScryHarvest(IERC20(token));
        h.postRoot(root, total, ref);
        console2.log(string.concat(name, " claim deployed + root posted"), address(h));
        return address(h);
    }
}
