// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryGarden.sol";

/// Deploy the spoils pair (OBOL + MYRRH) to Robinhood Chain — earned,
/// elastic-but-budgeted play tokens the Barrow mints and the Agora burns.
///
///   export PRIVATE_KEY=0x…
///   forge script script/DeploySpoils.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// ── A GARDEN NEVER HOLDS SCRY (operator, 2026-07-25) ───────────────────
/// This script used to pair BOTH spoils against canonical SCRY in fresh
/// Gardens by default on RH-Chain. It no longer pairs SCRY at all. It
/// deploys ONE Garden — **MYRRH/OBOL**, both sides house-minted — and that
/// is the LP whose SEED shares the Gardener farms.
///
/// Why (AUDIT-2026-07-25.md F5): `ScryGarden` is a deliberately minimal x·y=k
/// pair with no deadline and no minimum-out on add/remove liquidity, so every
/// action in it is sandwichable within a block. Pointing it at canonical SCRY
/// also stood a thin second pool beside the canonical Uniswap v3 pool for the
/// same pair — a permanent arb surface on which the Garden is always the weak
/// side. The real market is the v3 pool seeded by
/// `SeedSpoilsUniswapV3.s.sol` (POOLS.md §3); the Garden is the farmable LP
/// and now risks only house-minted coins, which is the same reasoning behind
/// the free MYRRH/OBOL 5:1 third pool (LAUNCH-DECISIONS.md).
///
/// SCRY_TOKEN is still read and still logged, because the seeding runbook
/// wants it — it just never becomes a Garden. Setting `GARDEN_PAIR=0` skips
/// the Garden entirely.
///
/// Supply mirrors the meter's off-chain spoils ledger (meter/tokens.py):
/// ELASTIC by default (cap 0 = uncapped, mint/burn with the game economy).
/// Set OBOL_CAP / MYRRH_CAP > 0 to deploy an optional forever ceiling. The
/// deployer starts as minter and should rotate to the merkle distributor once
/// the claim contract exists (`setMinter`).
///
/// THE ONE MINT THIS SCRIPT DOES, and why it is not a pre-mine: the Garden's
/// opening seed (`GARDEN_SEED_MYRRH` / `GARDEN_SEED_OBOL`, posted 5:1). A
/// ScryGarden's first add SETS ITS PRICE, so a Garden left publicly deployed
/// and empty can be opened by anyone at any ratio for dust — and it is the LP
/// the farm pays. Seeding it in the same run collapses that window from days
/// to a couple of blocks. The minted coins go STRAIGHT into the pool and the
/// LP shares come back to the deployer; nothing is left loose, which the suite
/// asserts. Mint and burn are ordinary operations for a game token here —
/// distributor-gated, not capped — so this is a posted, deliberate act rather
/// than a supply decision. Everything else still mints only against the public
/// game ledger. `GARDEN_SEED_MYRRH=0` opts out and says so loudly.
///
/// ⚠ The standing warning is unchanged and now applies to SCRY too: never
/// pool canonical SCRY in a ScryGarden;
/// the weak side of a thin toy AMM is whichever side is worth something.
/// `forge test -vv` first, always.
///
/// The Garden below is the sanctioned FAUCET-SCALE venue (toy AMM,
/// manipulable spot — never an oracle). For a REAL market on canonical
/// Uniswap v3, use `script/SeedSpoilsUniswapV3.s.sol` (+ the
/// `seed_spoils_uniswap.py` helper) after the tokens exist and the minter
/// is rotated — see POOLS.md §3 step 4.
contract DeploySpoils is Script {
    /// Canonical SCRY on RH-Chain (`TOKENOMICS.md` / meter/scry_token.py).
    address public constant SCRY_CANON = 0xDa2a4b23459e9ca88183e990802be644AcA7C4B0;
    uint256 public constant RH_CHAIN_ID = 4663;

    /// Everything the deploy needs, as plain values. `run()` fills this from
    /// env for the CLI; TESTS build it in memory and call `runWith` directly.
    /// vm.setEnv is process-global and forge runs tests in parallel workers
    /// sharing one process, so an env round-trip races between tests and the
    /// failures are nondeterministic — the same lesson SeedSpoilsUniswapV3's
    /// suite learned the hard way. Never round-trip a test through env.
    struct SpoilsInputs {
        uint256 pk;
        string obolName;
        string myrrhName;
        uint256 obolCap;
        uint256 myrrhCap;
        address scry;
        bool pairGarden;
        uint256 seedMyrrh;
        uint256 seedObol;
        address powderTo;
        uint256 powderObol;
        uint256 powderMyrrh;
    }

    function run() external {
        SpoilsInputs memory inp;
        inp.pk = vm.envUint("PRIVATE_KEY");
        // NAME AND SYMBOL ARE WELDED. `SpoilsToken` sets both in its
        // constructor and has no setter, so whatever goes in here is what every
        // wallet, explorer and aggregator shows forever. House form, matching
        // canonical $SCRY ("Scry Agent Ward" / SCRY): the SYMBOL is bare and
        // upper-case, the NAME is ordinary capitalised English. This shipped as
        // lower-case "obol"/"myrrh" until 2026-07-27 — a cosmetic default that
        // could never have been corrected after the broadcast.
        inp.obolName = vm.envOr("OBOL_NAME", string("Obol"));
        inp.myrrhName = vm.envOr("MYRRH_NAME", string("Myrrh"));
        // ── THE CAPS, AND THEY ARE WELDED ───────────────────────────────────
        // `cap` is immutable and it is NOT a supply ceiling — it is a LIFETIME
        // MINT BUDGET. `_burn` moves totalBurned and totalSupply and never
        // touches totalMinted, so a burn does not refund a wei of headroom.
        //
        // OBOL: NEVER CAPPED. Its whole design is mint-through-play,
        // burn-through-sinks (agora, deeds, shrine, tariffs) at 56-77% modelled
        // coverage, so it net-mints deliberately. Under a cap that healthy
        // cycle is a countdown, and the day it lands PLAY STOPS PAYING with no
        // fix short of migrating a token people hold.
        //
        // MYRRH: 21,000,000 — Bitcoin's number, chosen by the operator
        // 2026-07-27 ("its our BTC"). It is safe because MYRRH has exactly ONE
        // tap: the Garden, on a Bitcoin schedule (era-0 6,000/day, halving
        // every 4 years, stopping dead at 40 — see ScryGardener's constants).
        // The WHOLE RUN emits ~17.50M against a ~2.4M genesis float, so ~1.07M
        // of the budget is never minted at all.
        //
        // DO NOT RETYPE THE FLOAT OR THE HEADROOM HERE. Both are DERIVED, and a
        // typed copy has already gone stale three times in this repo — 2,510,000
        // then 2,662,400 then 2,422,640, each one correct on the day the powder
        // last moved and wrong the day after, with all three alive in the corpus
        // at once. `preflight.py` computes it from the literals in THIS file
        // (pools + Garden seed + the drop + POWDER_MYRRH below) and prints it on
        // every run; read it there.
        //
        // What is safe to say without a number, because it is true at any powder
        // size this file can hold: the float is ~2.4M against a 21M budget, so
        // THE CAP IS NEVER REACHED, by construction — the schedule terminates
        // ~1.07M short of it.
        // Say that plainly rather than letting a later session reason that the
        // cap is close: it is a posted scarcity claim anyone can read off
        // `cap()`, not a constraint to plan around.
        inp.obolCap = vm.envOr("OBOL_CAP", uint256(0));
        inp.myrrhCap = vm.envOr("MYRRH_CAP", uint256(21_000_000e18));
        // Reported for the seeding runbook only — canonical SCRY is NEVER
        // put in a ScryGarden (see the header). It is paired on canonical
        // Uniswap v3 by SeedSpoilsUniswapV3.s.sol, and nowhere else.
        inp.scry = vm.envOr("SCRY_TOKEN", block.chainid == RH_CHAIN_ID ? SCRY_CANON : address(0));
        inp.pairGarden = vm.envOr("GARDEN_PAIR", true);
        // The Garden's opening seed, minted here and deposited immediately.
        // Defaults are the posted 5:1 at faucet scale (LAUNCH-DECISIONS.md);
        // the canonical v3 pool is where real depth goes, not this.
        inp.seedMyrrh = vm.envOr("GARDEN_SEED_MYRRH", uint256(20_000e18));
        inp.seedObol = vm.envOr("GARDEN_SEED_OBOL", uint256(100_000e18));
        // THE POWDER (operator, 2026-07-26): a small working allocation of each
        // coin to the operator's main wallet, for the AI agency to hand out and
        // for seeding game pots. Set either to 0 to skip.
        //
        // Called what it is: this IS a pre-mine, and the header below says the
        // Garden mint is not one. The difference is not the mechanism, it is
        // that this is POSTED, PURPOSED and WATCHABLE — it lands in a named
        // wallet whose balance anyone can follow as it drains into giveaways.
        // An unposted allocation is the exact thing this audience screens for
        // (holder concentration); a posted one that visibly empties
        // is the opposite signal. `burn(uint256)` is public on SpoilsToken, so
        // burning powder is the lever back, and it lands as a Transfer to 0x0
        // that anyone can see.
        //
        // ⚠ THIS BLOCK ONCE DESCRIBED A SIZING THIS FILE NO LONGER SHIPS — a
        // "~1% of the genesis float" line and then an "8.8% of OBOL / 9.0% of
        // MYRRH, liquidity-in-pool 95.2% -> 87.7%" line, both computed for
        // allocations (160,000/24,000, then 1,600,000/240,000) that the rule
        // below superseded the same day. The literals underneath were correct
        // and the prose above them was not, which is the failure mode worth
        // naming: a reader trusts the comment and never checks the argument.
        // Percentages of an ELASTIC coin are the specific trap — see below.
        //
        // ── THE RULE, settled 2026-07-27 after three passes at the wording ──
        //
        //   THE POWDER IS 10% OF WHAT THE DROP PAYS, AND THE DROP STAYS WHOLE.
        //
        // Operator: "i just want 10% of whatever amount was getting air dropped,
        // like that total, whatever it is 1000 lets say i want 10% of that
        // number but it stays 1000." So the powder is 10% ON TOP of the drop -
        // never carved out of it, and never inside its own denominator.
        //
        // WHY THIS WORDING AND NOT THE THREE THAT FAILED. "10% of the coins"
        // has no denominator, because these coins are ELASTIC. Every earlier
        // attempt picked one silently and they disagree by up to 1000x on the
        // same allocation:
        //
        //   vs total genesis float            1,600,000 was 8.6%   <- as set 07-26
        //   vs the pool that reaches $SCRY    1,600,000 was 26.7%  <- absorption
        //   vs `circulating` (tokens.py, which EXCLUDES pool float)
        //                                     10% would be 104,075 OBOL / 267 MYRRH
        //
        // ~95% of all OBOL at genesis is pool inventory, so any "% of supply" is
        // really a statement about the pools and any "% of circulating" is
        // mostly a statement about the powder itself. THE DROP IS THE ONE
        // DENOMINATOR THAT IS NEITHER: it is a fixed, published, recomputable
        // total that nothing else here feeds back into.
        //
        // These literals are 10% of drop one as planned against the 2026-07-27
        // snapshot (336,672 OBOL / 2,400 MYRRH -> 33,667 / 240). They are
        // DERIVED, so do not retype them by hand: `preflight`'s
        // gate_powder_tracks_the_drop recomputes the drop and reds if these
        // drift, which is the failure that has now happened twice - a quantity
        // sized against one number while the number moved underneath it.
        inp.powderTo = vm.envOr("POWDER_TO", address(0));
        // THE POWDER, AND THE TWO COINS NOW FOLLOW DIFFERENT RULES (operator,
        // 2026-07-28). It was "10% of what the drop pays" for both.
        //
        // OBOL is a POSTED FIXED ALLOCATION, sized for giving coins away in
        // public — 1,000,000, ~13% of the 7,650,000 OBOL pool float. The old
        // 33,667 was 10% of a 336,672 OBOL drop, which is a rule about the
        // DROP's size and turned out to be far too small to run a giveaway
        // programme on. The drop is still whole and the powder is still on top
        // of it, never carved out. The number lives in LAUNCH-DECISIONS.md and
        // preflight reads it back from there, so this cannot drift from what is
        // published.
        //
        // MYRRH KEEPS THE 10%-OF-DROP RULE and stays at 240, because MYRRH is
        // capped at 21,000,000 and every coin minted here comes out of a
        // lifetime headroom of ~1,000,000 against a 40-year schedule. It also
        // does not need to grow: the MYRRH/OBOL pool opens at launch, so anyone
        // handed OBOL is ONE SWAP from MYRRH (TOKENOMICS.md §every coin reaches
        // every coin). Give OBOL; the town routes it.
        inp.powderObol = vm.envOr("POWDER_OBOL", uint256(1_000_000e18));
        inp.powderMyrrh = vm.envOr("POWDER_MYRRH", uint256(240e18));
        runWith(inp);
    }

    function runWith(SpoilsInputs memory inp) public {
        uint256 pk = inp.pk;
        address deployer = vm.addr(pk);
        uint256 obolCap = inp.obolCap;
        uint256 myrrhCap = inp.myrrhCap;
        address scry = inp.scry;
        bool pairGarden = inp.pairGarden;
        uint256 seedMyrrh = inp.seedMyrrh;
        uint256 seedObol = inp.seedObol;
        // EXACT, not integer division. `seedObol / seedMyrrh == 5` accepted
        // anything in [5.0, 6.0) — on wei-scale inputs that is a ratio wrong by
        // up to 20%, and the Garden's opening add SETS ITS PRICE permanently
        // (ScryGarden mints MINIMUM_LIQUIDITY to itself and never spends it, so
        // totalSupply never returns to 0 and the seeding branch is unreachable
        // for a second attempt). A guard for a welded price cannot round.
        require(
            seedMyrrh == 0 || seedObol == seedMyrrh * 5,
            "GARDEN_SEED_*: the Garden must open at exactly the posted 5 OBOL per MYRRH"
        );
        // A caller that builds this struct in memory (every test does) would
        // otherwise weld a NAMELESS token. The name has no setter, so an empty
        // string here is permanent and silent — it reads as a blank in every
        // wallet and there is no second chance.
        if (bytes(inp.obolName).length == 0) inp.obolName = "Obol";
        if (bytes(inp.myrrhName).length == 0) inp.myrrhName = "Myrrh";
        vm.startBroadcast(pk);
        SpoilsToken obol = new SpoilsToken(inp.obolName, "OBOL", obolCap, deployer);
        SpoilsToken myrrh = new SpoilsToken(inp.myrrhName, "MYRRH", myrrhCap, deployer);
        address garden;
        uint256 seeds;
        if (pairGarden) {
            // The ONE Garden: MYRRH/OBOL, both sides house-minted. Its SEED
            // shares are what ScryGardener farms (DeployGardener.s.sol).
            ScryGarden g = new ScryGarden(IERC20(address(myrrh)), IERC20(address(obol)));
            garden = address(g);

            // AND SEED IT IN THE SAME RUN. A ScryGarden's first add SETS ITS
            // PRICE, so a Garden that sits publicly deployed-and-empty between
            // this broadcast and a later manual seeding cast can be opened by
            // anyone at any ratio they like, for dust. The window used to be
            // however long that gap was - hours or days. Seeding here collapses
            // it to the few blocks between two transactions in one script run,
            // on a chain with no public mempool.
            //
            // Both sides are house-minted, which is the point of a game token:
            // mint and burn are ordinary operations here, distributor-gated
            // (minter-only) rather than capped. This is a posted, deliberate
            // mint of exactly the seed, not a pre-mine of float - the tokens go
            // straight into the pool and the LP shares come back to the
            // deployer. Set GARDEN_SEED_MYRRH=0 to skip and seed by hand.
            if (seedMyrrh > 0 && seedObol > 0) {
                myrrh.mint(deployer, seedMyrrh);
                obol.mint(deployer, seedObol);
                myrrh.approve(garden, seedMyrrh);
                obol.approve(garden, seedObol);
                // opening add: both sides are used as given, so minSeeds is the
                // exact geometric mean less MINIMUM_LIQUIDITY. Anything else
                // means the pool was already opened by someone else - in which
                // case this MUST revert rather than add at their price.
                //
                // THE DEADLINE NEEDS A MARGIN, and `block.timestamp` is not one.
                // ScryGarden checks `block.timestamp <= deadline`, and the
                // timestamp this script reads is the one it was SIMULATED at -
                // by the time the call is re-simulated and then mined it is
                // always older, so a zero-margin deadline reverts "deadline
                // passed" every single run. That is not theoretical: it made
                // `deploy_town.sh launch` (dry AND armed) die here, at the
                // first phase, 100% of the time. RH-Chain produces a block
                // every ~101ms, so "now" is stale before the next line.
                // 30 minutes still collapses the front-running window this
                // deadline exists to bound - the real protection is that both
                // transactions ride one script run on a chain with no public
                // mempool - while leaving room for a slow simulate-and-send.
                seeds = g.addLiquidity(
                    seedMyrrh, seedObol, _openingSeeds(seedMyrrh, seedObol), block.timestamp + 30 minutes
                );
            }
        }
        // THE POWDER — last, so a bad address cannot strand the pools. Minted
        // only if POWDER_TO is set; unset means no allocation happens at all.
        if (inp.powderTo != address(0)) {
            if (inp.powderObol > 0) obol.mint(inp.powderTo, inp.powderObol);
            if (inp.powderMyrrh > 0) myrrh.mint(inp.powderTo, inp.powderMyrrh);
        }
        vm.stopBroadcast();
        if (inp.powderTo != address(0)) {
            console2.log("POWDER (a POSTED pre-mine - watch it drain) ->", inp.powderTo);
            console2.log("  OBOL ", inp.powderObol);
            console2.log("  MYRRH", inp.powderMyrrh);
        }
        console2.log("OBOL", address(obol));
        console2.log("MYRRH", address(myrrh));
        console2.log("MYRRH/OBOL Garden (SEED_MYRRH_OBOL)", garden);
        if (seeds > 0) {
            console2.log("  seeded MYRRH", seedMyrrh);
            console2.log("  seeded OBOL ", seedObol);
            console2.log("  SEED shares to deployer", seeds);
        } else if (garden != address(0)) {
            console2.log("  !! NOT SEEDED - it is publicly empty until you add the first liquidity");
            console2.log("     whoever adds first sets its price. Do it NOW, not tomorrow.");
        }
        console2.log("SCRY (canonical v3 only - NEVER in a Garden)", scry);
    }

    /// What ScryGarden.addLiquidity mints on an EMPTY pool: sqrt(a0*a1) minus
    /// the permanently locked MINIMUM_LIQUIDITY. Used as minSeeds so the seed
    /// reverts if the Garden turned out not to be empty after all.
    function _openingSeeds(uint256 a0, uint256 a1) internal pure returns (uint256) {
        uint256 x = a0 * a1;
        if (x == 0) return 0;
        uint256 z = (x + 1) / 2;
        uint256 y = x;
        while (z < y) {
            y = z;
            z = (x / z + z) / 2;
        }
        return y > 1000 ? y - 1000 : 0;
    }
}
