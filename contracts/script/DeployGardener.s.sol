// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryGranary.sol";
import "../src/ScryGardener.sol";
import "../src/IERC20.sol";

/// Deploy the Gardener farm stack (FARMING.md, final tokenomics):
/// granary (mint chokepoint) -> gardener (LP farm) -> grant -> pool 0.
///
/// THE FARM PAYS **MYRRH**, AND IS ITS ONLY SOURCE (operator, 2026-07-26).
/// This reverses the 2026-07-25 flip to OBOL recorded below, and the reversal
/// is only safe BECAUSE it ships with the other half: `SCRY_BARROW_MYRRH_BANDS`
/// is now EMPTY, so play mints no MYRRH at all (FARMING.md 3a). The 07-25
/// reasoning had the right problem and the wrong lever -- it throttled the farm
/// while barrow room 3 minted 2,509 MYRRH/day at dau=200 against 1,320/day of
/// burn. Cut the source and the farm becomes the only tap, at a rate we own:
/// `setRewardPerSecond` is onlyOwner, so the era-0 BASE is a dial. The SHAPE is
/// not: since 2026-07-28 the farm halves every 4 years and stops at 40, welded
/// as constants in `ScryGardener` (operator: "farm MYRRH like bitcoin ... max
/// 40 year run"). The owner rescales that curve; it cannot extend or reschedule
/// it. Superseded 07-25 reasoning follows.
///
/// (SUPERSEDED) THE FARM PAYS **OBOL** (operator, 2026-07-25). MYRRH became the premium /
/// IAP coin, and the FARMING.md rule — never farm-emit the coin you are trying
/// to keep valuable — now points at MYRRH exactly as it always pointed at
/// SCRY. OBOL is the base coin with seven sinks and no store-of-value job, so
/// it is the right thing to emit. Nothing in the CONTRACTS changed: the
/// granary binds one SpoilsToken (ScryGranary.sol:24) and the gardener takes
/// the granary, so which coin is farmed is purely a deploy-time choice.
///
/// And the farmed POOL is the **MYRRH/OBOL** Garden SEED (operator,
/// 2026-07-25 — revised from MYRRH/SCRY the same day). Two rules meet here:
///   - the SELF-DRAINING rule, resolved 2026-07-27 (FARMING.md 3c): *never
///     farm-emit a coin into a pool whose other side the house BOUGHT and
///     CANNOT MINT.* The harm audit F6 found is denominated in the unmintable
///     side (pay OBOL to OBOL/SCRY LPs, they sell the harvest back through the
///     pool, and the purse's SCRY walks out) — NOT in "the pool contains the
///     reward coin," which is true of every farm ever built and is the reading
///     that made this comment forbid its own configuration;
///   - a `ScryGarden` never holds canonical SCRY (AUDIT-2026-07-25.md F5) —
///     it has no deadline and no minimum-out, so it is sandwichable within a
///     block, and standing one beside the deeper canonical v3 pool for the
///     same pair makes it the permanent weak side of an arb.
/// MYRRH/OBOL satisfies both, and the FIRST one it satisfies vacuously: the
/// Garden has NO unmintable side at all — both coins are house-minted and it
/// holds 0 SCRY — so neither MYRRH nor OBOL was ever the coin the rule
/// forbids. (This paragraph read "OBOL is not paid to its own LPs" until
/// 2026-07-27, which stopped being true when the reward flipped back to MYRRH
/// on 07-26 and was never the operative test anyway.) Real SCRY depth stays on
/// canonical Uniswap v3 where it belongs. `DeploySpoils.s.sol` deploys exactly
/// this one Garden.
///
///   export PRIVATE_KEY=0x...
///   export REWARD_TOKEN=0x...         # the coin the farm emits — MYRRH since
///                                     # 2026-07-26 (MYRRH_TOKEN is the fallback,
///                                     # and it names the REWARD, not the coin)
///   export SEED_MYRRH_OBOL=0x...      # pid 0 — the MYRRH/OBOL ScryGarden SEED
///                                     # (printed by DeploySpoils.s.sol)
///   # optional overrides of the FINAL posted numbers (FARMING.md section 3):
///   # export REWARD_PER_SECOND=6944444444444444   # 600 MYRRH/day, ERA 0
///   # export LOCK_BPS=6700                       # 67% of harvests cliff-lock
///   # export UNLOCK_AT=<unix>                    # default: now + 90 days
///   # export FARM_DAILY_CAP=12500000000000000000000 # 12,500 MYRRH/day granary cap
///   forge script script/DeployGardener.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// TWO DEFECTS THIS SCRIPT CARRIED UNTIL 2026-07-25, both found by audit
/// because NO TEST EXECUTES ANY DEPLOY SCRIPT (1 of 20 is imported by a suite):
///
///   1. `vm.envOr(primary, vm.envAddress(fallback))` evaluates its arguments
///      EAGERLY, so the "fallback" ran unconditionally and MYRRH_TOKEN +
///      SEED_OBOL_SCRY were mandatory — the runbook above reverted. Resolution
///      is now sequential, and `envAddrOr` is the shape to copy in any other script.
///   2. The header argued (correctly, per FARMING.md: never farm-emit the coin
///      you are trying to keep valuable) that the farmed pool must not be an
///      OBOL pool, and then the code made pid 0 the OBOL/SCRY SEED — the exact
///      self-draining loop it warned against. pid 0 is now the MYRRH/OBOL SEED,
///      which also keeps canonical SCRY out of a toy AMM (F5).
///
/// Wiring rule (FARMING.md section 5): the farm NEVER holds the SpoilsToken
/// minter role directly; the granary does, and the farm spends a posted
/// daily grant. If the broadcaster still holds the reward token's minter role
/// the script rotates it to the granary; otherwise the current minter must
/// call setMinter(granary) themselves before emissions can pay.
///
/// `forge test -vv` green before any broadcast. Always.
contract DeployGardener is Script {
    /// Sequential env resolution. `vm.envOr(a, vm.envAddress(b))` does NOT do
    /// this — Solidity evaluates the fallback argument before the call, so `b`
    /// becomes mandatory. Reverts naming BOTH keys when neither is set.
    function envAddrOr(string memory primary, string memory fallbackKey) public view returns (address a) {
        a = vm.envOr(primary, address(0));
        if (a != address(0)) return a;
        a = vm.envOr(fallbackKey, address(0));
        require(a != address(0), string.concat("set ", primary, " (or ", fallbackKey, ")"));
    }

    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(pk);

        // REWARD_TOKEN is the coin the farm emits — MYRRH since 2026-07-26,
        // when the Garden became its only source (FARMING.md 3a). It is an env
        // var and not a constant on purpose: the crop has moved twice, and a
        // welded one would have to be redeployed to move again.
        // MYRRH_TOKEN is honoured as a fallback so an older runbook still works,
        // but it names the reward slot, not MYRRH specifically.
        SpoilsToken reward = SpoilsToken(envAddrOr("REWARD_TOKEN", "MYRRH_TOKEN"));

        // pid 0 is the MYRRH/OBOL Garden SEED — the one Garden DeploySpoils
        // deploys, both sides house-minted. The older SEED_MYRRH_SCRY spelling
        // is still accepted so a half-updated shell does not silently deploy
        // the wrong farm; it names the LP, not the pairing.
        IERC20 seedPrimary = IERC20(envAddrOr("SEED_MYRRH_OBOL", "SEED_MYRRH_SCRY"));
        // Optional 2nd farm pool. Unset -> single-pool launch. Whatever goes
        // here must satisfy the same two rules as pid 0: not an LP of the
        // reward token's own pool, and never a Garden holding canonical SCRY.
        address seedSecondary = vm.envOr("SEED_SECONDARY", address(0));
        uint256 allocPrimary = vm.envOr("POOL_ALLOC_PRIMARY", uint256(100));
        uint256 allocSecondary = vm.envOr("POOL_ALLOC_SECONDARY", uint256(100));

        // HISTORY, because the direction of travel reversed twice: a flat
        // 240/day was halved to 120 on 2026-07-26 ("make this last for years"),
        // restored on 07-27, and the whole flat-rate premise was replaced on
        // 07-28 by the halving schedule below. "Lasting for years" was never
        // the binding constraint at 240/day — it would have taken 212.
        //
        // The float and the headroom are DERIVED — do not retype them here, as
        // this comment did until 2026-07-28. They fall out of DeploySpoils'
        // literals (pools + Garden seed + drop + powder) and preflight prints
        // them on every run; a typed copy has already gone stale three times.
        // ERA-0 rate. 600 MYRRH/day, halving every 4 years, stopping dead at 40
        // (the schedule is welded into ScryGardener as constants).
        //
        // LOWERED 6,000 -> 600, operator 2026-07-30: "MYRRH needs that BTC
        // vibes, you have to really earn it. but i can add more sources in the
        // future". Both halves of that sentence point the same way, and the
        // second is the load-bearing one:
        //
        //   * `SpoilsToken.cap` is 21,000,000 and IMMUTABLE, and `totalMinted`
        //     is monotone — burning never returns budget. So every future MYRRH
        //     source shares this one ceiling; none can add to it.
        //   * At 6,000/day the farm alone committed ~17.50M of 21M, leaving
        //     1,004,469 (4.8%) for every source ever added afterwards. That is
        //     not room, it is a rounding error.
        //   * At 600/day the run emits ~1.75M, so ~16.76M (79.8%) stays
        //     unallocated and available to whatever earns MYRRH later.
        //
        // The 07-28 intent survives untouched: "early users get way more than
        // people later" is the HALVING's job, not the base rate's. Era 0 still
        // pays 512x era 9. What changed is the size of the whole run.
        //
        // It also closes the imbalance `tokenomics_sim.py` names: at 6,000/day
        // MYRRH minted 6,000 against 1,320 burned at dau=200 — 4,680/day
        // accumulating forever, and sink coverage of 1.6% at dau=20. The farm
        // was calibrated for 1,000 DAU against a town that has 0.
        //
        // NOT WELDED, and that is why this is a cheap decision:
        // `ScryGardener.setRewardPerSecond` is onlyOwner, so the rate is a dial
        // and raising it when DAU justifies costs one transaction. What IS
        // welded is the SHAPE — HALVING_PERIOD and HALVINGS are constants — and
        // the cap, which is why the run's total is the thing to be careful with.
        //
        // WHY NOT DERIVE IT TO THE WEI. Solving the schedule to land exactly on
        // the cap gives 6,368/day and leaves 959 MYRRH under an IMMUTABLE
        // ceiling — a rounding error away from `SpoilsToken.mint` reverting
        // "cap exceeded" in year 39. A round number well under the cap is the
        // safe one.
        uint256 rps = vm.envOr("REWARD_PER_SECOND", uint256(6_944_444_444_444_444));
        uint256 lockBps = vm.envOr("LOCK_BPS", uint256(6700)); // 67% locked
        uint256 unlockAt = vm.envOr("UNLOCK_AT", block.timestamp + 90 days); // the cliff
        // The cap tracks the ERA-0 rate at the SAME ~2.08x it always had. It is
        // a throttle, not a ceiling: `ScryGranary.mint` CLAMPS instead of
        // reverting, so a cap near the rate silently turns multi-day harvests
        // into stash IOUs rather than payments — which is exactly what the OLD
        // 500/day cap would have done to a 6,000/day era-0 farm: clamp it to
        // 8% and stash the rest, a 92% silent underpayment with nothing red.
        //
        // Sizing it off ERA 0 means it never binds again: the rate only ever
        // falls, so a cap set for the first four years is loose for the other
        // thirty-six by construction.
        // 12,500/day — and SIZED BY THE CLIFF, which is the derivation this
        // number never had. It used to be justified as "~2x era 0", true when
        // era 0 paid 6,000 MYRRH/day and stale the moment the 2026-07-30
        // sentence cut era 0 to 600: on that reasoning the cap stood at ~21x
        // the emission it bounds. It was briefly tightened to 1,250 to restore
        // the 2x ratio, then set back the same day on the operator's call —
        // because the 2x ratio was never the thing that mattered.
        //
        // WHAT ACTUALLY BINDS THIS IS `unlockTime`. Only the immediate 33%
        // (~198 MYRRH/day) is mintable while rewards lock. Then, on one dated
        // instant, the whole locked pile — ~36,120 MYRRH at era 0, 67% of 90
        // days — becomes claimable at once, and `claimLocked` is clamped by
        // this same cap. So the cap has to clear a lump it will meet exactly
        // once, and the drain time is what to size on: 12,500/day clears it in
        // ~3 days, 1,250/day in ~29. Nothing is ever lost at either number
        // (`claimLocked` re-credits the shortfall and zeroes before the call),
        // but a farmer who waited 90 days should not then wait a month.
        //
        // The blast-radius cost of the larger number, stated so it is not
        // hidden: 12,500/day is 0.06% of MYRRH's 21,000,000 cap per day, and
        // `setGrant` lowers immediately with no notice period — so a bad day is
        // bounded and a bad discovery is one transaction from stopped.
        uint256 dailyCap = vm.envOr("FARM_DAILY_CAP", uint256(12_500e18)); // 12,500/day

        require(address(seedPrimary) != address(reward), "pid 0 LP is the reward token itself");
        require(seedSecondary != address(reward), "pid 1 LP is the reward token itself");

        vm.startBroadcast(pk);
        ScryGranary granary = new ScryGranary(reward);
        ScryGardener gardener = new ScryGardener(granary, rps, lockBps, unlockAt);
        granary.setGrant(address(gardener), dailyCap);
        gardener.addPool(seedPrimary, allocPrimary); // pid 0: MYRRH/OBOL SEED, the launch pool
        if (seedSecondary != address(0)) {
            gardener.addPool(IERC20(seedSecondary), allocSecondary); // pid 1, if posted
        }
        if (reward.minter() == deployer) {
            reward.setMinter(address(granary));
        }
        vm.stopBroadcast();

        console2.log("ScryGranary ", address(granary));
        console2.log("ScryGardener", address(gardener));
        console2.log("reward token", address(reward));
        console2.log("pid 0 LP    ", address(seedPrimary));
        if (seedSecondary != address(0)) console2.log("pid 1 LP    ", seedSecondary);
        console2.log("unlockAt    ", unlockAt);
        // ECHO THE THREE NUMBERS THIS WELDS. `deploy_town.sh gardener` exports
        // only REWARD_TOKEN and SEED_MYRRH_OBOL and echoes only the addresses,
        // so REWARD_PER_SECOND, LOCK_BPS and FARM_DAILY_CAP were read straight
        // out of the ambient environment by `vm.envOr` and never shown to the
        // operator before the broadcast — the same env-leak class that lets a
        // stale shell variable weld an unintended value. rewardPerSecond is
        // immutable in effect (only `setRewardPerSecond` moves it, and it
        // re-prices the whole remaining curve), and lockBps is retroactive.
        // Read back off the CONTRACTS, never echoed from the inputs: an echo
        // of the input proves the script's arithmetic, not the chain's state.
        console2.log("rewardPerSecond (era-0 base, wei/s)", gardener.rewardPerSecond());
        console2.log("  = MYRRH/day", (gardener.rewardPerSecond() * 86_400) / 1e18);
        console2.log("lockBps     ", gardener.lockBps());
        console2.log("granary daily cap (wei)", granary.availableToday(address(gardener)));
        console2.log("  = MYRRH/day", granary.availableToday(address(gardener)) / 1e18);
        if (reward.minter() != address(granary)) {
            console2.log("NOTE: the reward token's minter is not the granary yet;");
            console2.log("      the current minter must setMinter(granary).");
        }
    }
}
