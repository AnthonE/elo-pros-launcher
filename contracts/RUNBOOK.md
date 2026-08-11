---
status: live
lane: [ops, launch]
updated: 2026-07-27
about: "the contracts deploy runbook — deploy_town.sh, the real gate (upstream forge + live fork mandates), and who holds what power"
---
# RUNBOOK.md - the town deploy: zero to the full DFK stack on RH-Chain

> ## ▸ For the LAUNCH itself, `docs/launch/LAUNCH.md` is the plan of record (2026-07-25)
>
> `LAUNCH.md` supersedes the ordering on this page for what ships first — it
> runs preflight → check → test → spoils → pools → harvest and stops at "a
> DeFi game on RH-Chain." This runbook stays correct for the **depth** phases
> after that (gardener → granary → silo → shrine → orchard) and for the arming
> decisions, and it still owns `./deploy_town.sh`'s per-phase gates.
>
> ⚠ **The launch phases come FIRST, and the boundary is the minter.** The
> `pools` phase mints its own coin sides and harvest funding mints too, so both
> need the deployer still holding the minter slots — while `gardener` hands
> MYRRH's slot to its granary and `granary` hands OBOL's to the other one, and
> neither script fails closed if the slot is already gone (each logs a NOTE and
> carries on, leaving a successful-looking broadcast whose farm cannot pay
> until the slot's holder does the handoff). Depth after launch, always.

> The game plan for getting everything in `contracts/` running:
> spoils -> pools -> harvest, then gardener -> granary -> silo -> shrine ->
> orchard, then the arming decisions that move real value. The orchestrator is
> **`./deploy_town.sh`** (this directory): every phase **dry-runs by
> default** and only broadcasts with `--arm`; every armed broadcast is
> gated on a full green suite immediately before it; addresses flow
> between phases through `town.env` so nothing is re-typed. Run it from a
> machine with foundry (`forge`, `cast`) and `python3`.
>
> **The gate is real, not just `forge test` (hardened 2026-07-23).** The
> `test` phase (and every armed phase's pre-broadcast gate) now: (1)
> **refuses the foundry-zksync fork** (`need_upstream_forge`) — that fork
> breaks via_ir/vm.warp AND would broadcast zkSync bytecode, not the EVM
> bytecode RH-Chain runs, so use UPSTREAM foundry (v1.7.x); (2) runs the
> **fork mandates against live RH-Chain and proves they executed**
> (`fork_mandates_green`) — a plain offline `forge test` SKIP-passes the
> Orchard/Seed fork tests, i.e. the exact tests that replace a testnet.
> The suite count is DERIVED, never quoted from here — run
> `./deploy_town.sh test` and read the tail (this line carried three stale
> counts in two days). Offline, the fork mandates report `[PASS]` at ~5k gas
> each instead of the multi-million-gas a real fork run burns; that gas figure
> is the cheapest way to tell a skip-pass from a run.
> Rerun the live-fork gate — `./deploy_town.sh test` with upstream Foundry
> and `RH_FORK_URL` — immediately before any arm or broadcast; the last full
> read was 9/9 live-fork mandates green, 2026-07-27 (`NOW.md` §0), taken
> AFTER the Orchard accounting and holder-deposit changes.
> TEST-AUDIT.md (a dated 2026-07-22 record) rates all five tracks
> testnet-redundant only when that live fork gate is green. The sandbox rule stands: the first green suite on your
> Foundry box is still the point.
>
> 🛑 **A green gate on YOUR box is not a green gate. Check what is staged
> first** (2026-07-25, security wave 6):
> ```sh
> git status --porcelain contracts/src   # must be empty of `??` lines
> ```
> This fired here 2026-07-25: `src/SafeERC20.sol` and `src/ReentrancyGuard.sol`
> were **untracked** while twelve tracked contracts imported them — commit the
> twelve without the two and **`forge build` fails on every one** — no build,
> no test, no broadcast — while the working tree stays perfectly green, because
> it has the files. Both are committed now (closed 2026-07-25, verified
> `git ls-files`); the check above stays, because the general rule is standing:
> **an import is a dependency and a dependency has to be committed** — the same
> trap took the meter down one lane over the same day (`SECURITY-TODO.md`).

---

## 0. The map - who holds what power over whom

| organ | power it holds | held over | escape hatch |
|---|---|---|---|
| deployer wallet | `minter` of MYRRH (nothing mints it yet) · `steward` of the granary · `owner` of gardener/silo/shrine/orchard | everything below | transfer each role to a multisig when scale warrants |
| **ScryGranary** | `minter` slot of **OBOL** (rotated in at the `granary` step) | all farm/silo/orchard emission | `rotateMinterAway` |
| granary grants | posted daily caps: gardener **12,500**/day, silo 250/day | farm + locker drip | `setGrant(who, 0)` is the gentler hatch and the one to reach for: it leaves the grant enabled with a cap of 0, so `mint` computes room 0 and returns without reverting. `revokeGrant` deletes the row, after which `mint` reverts `no grant` — organs catch it and stash, so principal never bricks either way, but only one of them keeps the accounting path warm |
| **ScryGardener** | custody of staked SEED | LP farm | `emergencyWithdraw` (staker side) |
| **ScrySilo** | custody of sealed spoils | lockers | `breakSeal` (staker side, burns the ladder) |
| **ScryShrine** | nothing - it only burns | - | `setOfferingEnabled(false)` |
| **ScryOrchard** | custody of staked v3 NFTs + posted season pots | v3 farm | anyone-unstakes-after-window + `endIncentive` sweep |

The one rule over all of it: money never moves a measurement, and no
meter number keys any mint, weight, rank, or penalty. The stack below is
labor/game surface only.

⚠ **One membership rule, on the organ that is not in that table.**
`ScryArbiter`'s bench is pinned per case at the first vote (`casePanel`), and
**shrinking the live panel below that pinned size can strand an open case
unfinalizable** — every remaining member has voted, no further vote is
accepted, the exhaustion branch never fires, and `rule` reverts forever.
So: **do not remove arbiters while a case is open unless you seat a
replacement.** Check `casePanel(jobId)` against `arbiterCount()` before any
`setArbiter(a, false)`.

No money is trapped by that — `dispute` closes the job *before* calling `rule`,
so the revert rolls the status change back and `close()` after the deadline
still releases the escrow. What is lost is the arbitration path for that one
job: the parties get the timeout outcome instead of a verdict. Audited
2026-07-27; the contract header carries the recipe and the reasoning, and the
fix is deliberately operational — a force-finalize would hand the owner power
over verdicts, which is the one thing that court is built to withhold.

## 0b. What you can retune later, and what you cannot (audited 2026-07-25)

The posture is DYNAMIC: every rate that governs ongoing economics is
owner-settable and every admin rotates. This table is the exception list — the
values welded at construction, which a redeploy is the only way to change.
Check it before each broadcast, because you have the least information you will
ever have at the moment you must pick them.

| welded | contract | consequence |
|---|---|---|
| `foundingBurn`, `conveyanceBurn` | `ScryDeed` | **the whole land sink, priced once.** A redeploy is a new deed collection |
| `FEE_BPS = 30` (0.3%) | `ScryGarden` | a `constant`, not even immutable — the Garden's LP fee cannot move. **Load-bearing, not an inherited default** (operator, 2026-07-27): the Garden is 1% of the v3 pool's depth, so the fee discount is the ONLY thing that can make it the better venue, and it does — up to ~716 OBOL, about one delve's spoils. It went 30 → 100 and straight back the same day; at a matched 1% the Garden is strictly dominated at every trade size. `ScryGarden.sol`'s notice carries the crossover table |
| `unlockTime` | `ScryGardener` | the 90-day cliff is an ABSOLUTE timestamp, not a duration: deploy late and the cliff is shorter |
| `maxBreakBps` | `ScrySilo` | ceiling on the early-break burn |
| `baseStep` | `ScryShrine` | the whole rank ladder scales off it |
| `flatFee`, `feeRecipient` | `ScryArbiter` | **deliberate.** A court whose fee an owner can move is Bar Hadya with extra steps. Retune = redeploy + `board.setArbiter(new)` |
| `maxArbFee` | `ScryJobBoard` | **deliberate.** A ceiling an operator can raise is not a ceiling |
| the slash ladder | `ScryGardener` | hardcoded 25% same-block → 8% → 4% → 2% → 1% → 0.5% → 0.25% → 0.01% |

`ScryBank` has no knobs at all, correctly — it is a pure share-ratio model with
nothing to tune.

**Everything else is live.** `setRewardPerSecond` · `setAllocPoint` · `addPool`
/ `addBin` / `addTier` / `setTierEnabled` · `setGrant` / `revokeGrant` ·
`setFeeBps` · `setMinPremiumBps` · `setRepRewards` · `setArbiter` ·
`setFeeSplitter` · `setInsuredOpen` · `setMaxPayout` · `setWindow` ·
`setThreshold` · `setSplit` · `setOfferingEnabled`, plus
`transferOwner`/`transferOperator`/`transferSteward` on every role-bearing
contract. Orchard seasons are themselves the knob: each `createIncentive` is a
fresh pot and window.

⚠ The one worth a decision rather than a glance is **`ScryDeed`**. Every other
welded number is either intentional (arbiter, board ceiling), cosmetic
(shrine), or a bound you can simply stay under. The deed burns are the entire
land economy, set once, in a game with zero players.

## 0b2. Four things the REAL launch taught, 2026-07-29

None of these were visible from a dry run. All four cost time on the night.

**`verify --arm` reports failures that already succeeded.** Blockscout accepted
every submission, then answered `Unknown UID` while the script polled for
status, so the phase printed *"verification failed"* for contracts that were
verified. Re-running the phase prints `already verified` for all of them. **Do
not chase the first pass's failures — re-run once, then check the explorer**,
which is the authority. ⚠ **Two endpoints, and only one carries `is_verified`:**
`/api/v2/addresses/<addr>` has the boolean; `/api/v2/smart-contracts/<addr>`
returns the **source** for a verified contract and no such key, so reading
`is_verified` off it answers `None` for everything and looks like a repo-wide
failure. Both are correct; asking the wrong one is not. And

⚠ **`forge verify-contract` STOPPED WORKING HERE on 2026-07-30, and phase 9
still calls it.** It uses Blockscout's **legacy `/api`** route for both its
pre-check and its submit, and that route now answers *"Too many requests"* to
this box almost permanently — six contracts, three-attempt retries at ten-minute
spacing, one success in the lot. `/api/v2` is not rate-limited at all. **The
path that works, and verified all sixteen addresses:**

```bash
forge verify-contract <addr> src/X.sol:X --show-standard-json-input > si.json
curl -X POST "https://robinhoodchain.blockscout.com/api/v2/smart-contracts/<addr>/verification/via/standard-input" \
  -F compiler_version=v0.8.26+commit.8a97fa7a -F license_type=mit \
  -F contract_name=src/X.sol:X -F autodetect_constructor_args=false \
  -F constructor_args=0x… -F "files[0]=@si.json;type=application/json"
```

`{"message":"Smart-contract verification started"}` or `{"message":"Already
verified"}` are both wins. **Derive `constructor_args`, never retype them** —
subtract the artifact's creation bytecode from the CREATE input in
`broadcast/<script>.s.sol/4663/run-latest.json`. That is what caught
`ScryGacha`'s being `(uint256 drawDelay, address feeSink)` rather than the
address-first order the deploy log's reading order suggests; a wrong pair fails
identically to a rate limit and costs a whole window to diagnose.

And
note the explorer shows both coins as **`SpoilsToken`**, not OBOL/MYRRH; the
ticker only appears under `symbol()`. Looking for "OBOL" finds nothing and reads
as unverified.

**The canary and the top-up each mint a NEW position NFT, so you end with 6, not
3.** `SeedSpoilsUniswapV3` calls `npm.mint()` unconditionally; growing the
canary would need `increaseLiquidity(tokenId)` and the script tracks no
tokenIds. Harmless — every pair has identical ticks and the *pool* keys a
position on `(owner, ticks)`, so the pool holds one position per pair and NPM
tracks two claims on it. Liquidity sums, fees accrue pro-rata. It costs you two
`collect` calls per pool instead of one. **If a future launch wants 3: record
the tokenIds in `town.env` and call `increaseLiquidity` when they exist.**

**`DeploySpoils` sends the Garden's SEED to `vm.addr(pk)` — the deployer — and
nothing redirects it.** The 07-29 sentence pointed `LP_RECIPIENT` and
`SCRY_OPS` at the dev wallet so the deployer would hold no deeds; the Garden's
LP was missed, so 100% of the SEED landed on the one key that exists to be used
once and retired. Moved by hand afterwards (SEED is a plain ERC-20, no lock).
**A fresh deploy should take a `SEED_RECIPIENT` the way `pools` takes
`LP_RECIPIENT`.**

**The public RPC will Cloudflare-challenge a script.**
`https://rpc.mainnet.chain.robinhood.com` started answering HTTP 403 *"Just a
moment…"* mid-session under ordinary `cast` use. Always pass the keyed pool —
`--rpc-url "$(printf '%s' "$SCRY_RH_RPC_POOL" | cut -d, -f1)"` or the whole
comma list where the tool accepts it. Same lesson the holder snapshot learned
via HTTP 429, in a different costume: **the failure is not an error you notice,
it is an answer you believe.**

## 0c. ⚠ THIS IS ARBITRUM NITRO — what that changes, audited 2026-07-26

Chain 4663 is **Arbitrum Nitro/Orbit**, not an L1 and not OP-stack. Measured:
`ArbSys`, `ArbGasInfo`, `ArbOwnerPublic` and `ArbRetryableTx` all carry code,
no OP predeploys exist, `arbChainID()` = 4663, `arbOSVersion()` = 116.

**The one that already cost a redeploy-class defect:**

| clock | measured | pace |
|---|---|---|
| Solidity `block.number` | 25,620,709 | **0.08/s — one per ~12s. It is the PARENT chain's.** |
| `ArbSys(0x64).arbBlockNumber()` | 20,319,770 | 9.9/s — the real 101ms blocks |
| `block.timestamp` | accurate to ~1s | **fine to use** |

`ScryGacha` shipped reading `block.number`, which silently turned a *"100 block,
~10 second"* reveal into **~20 minutes**, locked its pool for that whole time,
and cut throughput from ~8,600 draws/day to ~72. **Nothing failed and the tests
passed.** The fact was already in the repo — `deploy_town.sh` says *"RH-Chain
(Arbitrum L2)"* — and went unread.

### The rules for anything new

1. **Never pace anything in `block.number`.** Use `block.timestamp` for
   durations (it is accurate here), or `ArbSys(0x64).arbBlockNumber()` if you
   genuinely need L2 height.
2. **`arbBlockHash(n)` REVERTS outside its window** (`InvalidBlockNumber`)
   rather than returning zero — branch on the call's success, never on a
   zero-check or a block-count comparison. The window is documented 256 and
   measures 253–254.
3. **Never use `blockhash` or `prevrandao` for randomness.** Arbitrum documents
   the opcode as unfit, and this town has a better answer anyway: the augury's
   committed day seed, revealed next day, verifiable forever.
4. **A fork test cannot prove your clock.** Foundry implements no ArbOS
   precompile — `0x64` holds one byte of non-executable code on a fork. Etch
   `contracts/test/MockArbSys.sol` in **unit and fork** suites both.
5. **`vm.etch` copies runtime code and nothing else** — no constructor runs, no
   storage travels. A mock whose `window = 256` was a field initializer read
   **0** at the etched address, every reveal looked expired, and the expiry test
   **passed for the wrong reason**. Set such fields explicitly after etching.
6. **L1 calldata is the dominant gas cost here**, not compute. It is why a
   merkle proof is cheap at our sizes and why a design that posts large calldata
   per action is not.
7. **A "block number" means two different things depending on which side reads
   it, and that collision is already in this repo.** *On-chain*, Solidity's
   `block.number` is the parent chain's. *Off-chain*, every block number the RPC
   speaks — `eth_blockNumber`, a `latest`/height block tag, an explorer link — is
   **L2**. Both are correct in their own frame, which is why nothing complains.
   `meter/holder_snapshot.py` converts its lookback at **~10 blocks/s** and is
   right to, because it reads balances at L2 block tags.
   **This explains a gotcha the repo had already hit and never explained:**
   `OrchardFork.t.sol` must `vm.roll(25_539_590)` because forge sets a fork's
   `block.number` from the RPC's **L2** height (~20.3M), which sits *below*
   SCRY's parent-chain-denominated `restrictionEndBlock` (25,539,589) — so a
   long-expired launch restriction springs back to life on a fork while being
   genuinely expired on-chain. Audited 2026-07-27: no off-chain code converts
   time to blocks at the wrong rate, and no contract reads a block number at all.

### The audit, as of 2026-07-26

**Clean, and the lesson is currently the gacha's alone — which is the gap.**

| check | result |
|---|---|
| `block.number` outside `ScryGacha` | **none** |
| `blockhash` · `prevrandao` · `block.difficulty` · `block.coinbase` | **none anywhere** |
| `gasleft()` · `.gas()` | **none** — no gas assumptions |
| everything time-based | **`block.timestamp`**, which is correct on Nitro |
| `MockArbSys` used by | `ScryGacha.t.sol`, `ScryGachaFork.t.sol` — **only the gacha** |

⚠ **Two live hazards that are not defects yet:**

- **The burrow's siege window reasons in blocks** — *"at 100ms blocks a 10s window gives
  ~100 blocks"*. That arithmetic is **correct** (it is about arbitrage frequency,
  not a timer) but the siege is **unwritten**, and whoever writes it will read
  "100 blocks" and reach for `block.number`. Pace the siege in
  **`block.timestamp`**.
- **`OrchardFork` and `SeedSpoilsFork` do not etch `MockArbSys`.** They do not
  need it today because neither contract reads a block clock — but that is a
  property of the code, not of the harness, and it will not announce itself if
  one starts to.

## 1. The phases

Each row: run the dry-run first, read the simulation, then re-run with
`--arm`. After every armed phase: paste the new addresses into
`deployments.json` **in the same commit** as the updated `town.env`
(an entry there means BROADCAST AND LIVE), then `./deploy_town.sh status`.

| # | command | what happens | needs first |
|---|---|---|---|
| 0 | `./deploy_town.sh check` | tooling, RPC serves chain 4663, canon SCRY has code, NPM/factory pairing (if set) | - |
| 1 | `./deploy_town.sh test` | `forge build` + the full suite, `-vv` | 0 |
| 2 | `./deploy_town.sh spoils --arm` | OBOL (elastic) + MYRRH (capped 21,000,000, welded) with deployer = minter, + the ONE **MYRRH/OBOL** Garden | 1 green |
| 3 | `./deploy_town.sh gardener --arm` | MYRRH granary + gardener + **12,500**/day grant + pool 0 (**MYRRH/OBOL SEED**) + **MYRRH minter rotates to the MYRRH granary** (the farm's crop — §7d; this row said OBOL until 2026-07-27) | 2, and after the launch's `pools`/`harvest` — the minter boundary in the header |
| 3b | `./deploy_town.sh granary --arm` | the **OBOL granary** (the silo's drip and season pots; **OBOL minter rotates to it**). A `ScryGranary` binds ONE token, so the farm's MYRRH one is not interchangeable | 2 |
| 4 | `./deploy_town.sh silo --arm` | silo + tier menu (7d/30d/90d/180d) + OBOL bin + MYRRH parking bin + 250/day grant — **refuses without `GRANARY_OBOL`** | 3b |
| 5 | `./deploy_town.sh shrine --arm` | shrine + OBOL altar (1 pt) + MYRRH altar (5 pt) - no grant, it only burns | 2 |
| 6 | `export NPM=0x... V3_FACTORY=0x...` then `./deploy_town.sh orchard --arm` | orchard, **no season posted** | 2 + `check` again |
| 7 | manual casts, below | the arming decisions that move value | each its own sentence |
| 8 | `./deploy_town.sh config` + `status` | `gardens.config.json` emitted, the farm page lights up; wiring read back from chain | 3 |
| 9 | `./deploy_town.sh verify --arm` | every address broadcast above carries **published source** on Blockscout | any broadcast |
| — | `./deploy_town.sh bank --arm` | `ScryBank` (stake SCRY → **xSCRY**) + `ScryFeeSplitter` (the one posted route for every SCRY fee). **Deliberately unnumbered** | 1 green, **nothing else** |
| — | `./deploy_town.sh preflight` | the ledger-readiness read the launch runs first (referenced in §2 7b) | - |
| — | `./deploy_town.sh launch --arm` | **the LAUNCH.md broadcast in one command** — preflight → check → test → spoils → pools → harvest, each gated (`NOW.md` §1). It does **NOT** run `rotate`: the `launch` case stops at `harvest`, and this row said otherwise until 2026-07-28 | everything above it gates |
| — | `./deploy_town.sh claims` | **read-only.** Checks every posted root and total against the merkle artifact it came from, and refuses to call a drop safe if they disagree. Takes no `--arm` — it broadcasts nothing | a drop's claim contracts recorded in `deployments.json` |

**On the `bank` row, and why it had none until 2026-07-27.** It needs no
minter, no granary, no pool and no address from any other phase — which made it
the one organ nothing ever blocked on, so it was simply left out of the launch
while being the third leg of the JEWEL design.

⚠ **It is unordered with respect to what it CONSUMES, and strictly FIRST with
respect to what consumes IT.** This row read "it can run before phase 2 or
after phase 9" until 2026-07-28, which is wrong in one direction and welds if
followed: `ScryEidolon.feeSplitter` and `ScrySteleEdition.feeSplitter` are
`address public immutable`, and all three of `DeployScryEidolon.s.sol`,
`DeployScrySteleEdition.s.sol` and `DeployScryMarket.s.sol` take
`SCRY_FEE_SPLITTER` as a **mandatory** `vm.envAddress`. Deploy any of them
first and the likely path is exporting a placeholder to get the broadcast
through — after which every priced mint and every ERC-2981 royalty routes
there forever, outside the posted split, with no setter. Only
`ScryJobBoard.setFeeSplitter` can be repointed.

Two properties are not negotiable:

- **The posted split is burn 5000 / bank 4000 / prizes 0 / ops 1000**
  (2026-07-27, `SENTENCES.md` — the answer to `FEES.md` §9 #2). Prizes take
  no line at open: that cut needs an escrow wallet, and pointing it at ops
  would post four outlets while paying three. **The numbers are DERIVED, not
  retyped** — they live in `DeployScryEconomy.s.sol`'s `vm.envOr` defaults and
  the phase greps them back out, so the driver cannot drift from what it
  broadcasts. Export one to override and the phase prints *posted X → you set
  Y* and tells you to append a new sentence. It still refuses a sum ≠ 10000
  (before spending gas on a constructor that would revert), refuses
  `SCRY_PRIZE_ESCROW == SCRY_OPS` if prizes carry bps, and requires `SCRY_OPS`
  so the deployer key is never posted as a fee sink by accident.
  ⚠ *"Welds at broadcast"* overstates it: `setSplit` can repost the table, and
  what is irreversible is narrower — **fees collected before a repost pay out
  under the split they were collected under.**
- **The bank opens empty and is not seedable.** See 7g below.

After it: paste both addresses into `deployments.json`, and wire
`SCRY_BANK` / `SCRY_FEE_SPLITTER` into `meter/ecosystem.config.js` and
`watchtower/bank.config.json` — neither is an on-chain act and neither happens
by itself, so `GET /bank` and the bank page stay dark until they are done.

**On 9, because it had no row until 2026-07-27 and the habit had already
lapsed:** `ScryVowRegistry` is verified on Blockscout and `ScryNotary` —
broadcast eight days later — is not. `forge script --broadcast` carries no
`--verify` here, so nothing ever did it automatically. The phase derives the
constructor arguments from foundry's own receipt and refuses any address whose
deployed bytecode is not the contract it claims to be, because
`broadcast/<script>/4663/` is keyed on **chain id** and a local anvil fork of
RH-Chain writes receipts there that look identical to real ones. Several such
rehearsal receipts are in this repo now.

Notes on the order: the shrine (5) only needs the tokens, so it can run
any time after 2. The orchard (6) is independent of the granary - its
seasons are funded pots, not grants. NPM/V3_FACTORY come from the
POOLS.md 4.1 table and are **verified on-chain by `check` and by the
deploy script itself** (`npm.factory()` must match) - docs are never the
authority.

## 2. The arming decisions (deliberately NOT automated)

Everything that moves real value is a manual `cast send`, run when the
operator says the sentence, sized per the posted numbers. The script will
never do these.

**7a. Seed the MYRRH/OBOL garden** - the farmable SEED, both sides
house-minted, at the posted 5:1 base rate (5 OBOL per MYRRH; POOLS.md 2.2).
Publicly note it.

⚠ **Canonical SCRY never goes in a ScryGarden** (operator 2026-07-25;
AUDIT-2026-07-25.md F5). The Garden has no deadline and no minimum-out, so
every action in it is sandwichable within a block, and standing one beside the
deeper canonical v3 pool for the same pair makes it the permanent weak side of
an arb. Real SCRY depth is the `pools` phase (POOLS.md 3) - `SeedSpoilsUniswapV3.s.sol`
against canonical Uniswap v3 - and nowhere else.

`DeploySpoils` already seeds the Garden inline (20,000 MYRRH + 100,000 OBOL in
the same broadcast, so it is never publicly empty) — this cast is only for a
later TOP-UP:

```bash
source town.env
cast send $MYRRH_TOKEN 'approve(address,uint256)' $GARDEN_MYRRH_OBOL <MYRRH_WEI> --private-key ...
cast send $OBOL_TOKEN  'approve(address,uint256)' $GARDEN_MYRRH_OBOL <OBOL_WEI>  --private-key ...
cast send $GARDEN_MYRRH_OBOL 'addLiquidity(uint256,uint256,uint256,uint256)' \
          <MYRRH_WEI> <OBOL_WEI> <MIN_SEEDS> <DEADLINE_UNIX> --private-key ...
```

The signature is four arguments — the two-argument form this page carried
matched no function and reverted. Amounts are MAXIMUMS since the audit: only
the in-ratio side is pulled, so an over-supplied side is left in your wallet
rather than donated to the pool. `<MIN_SEEDS>` guards the price you add at;
`<DEADLINE_UNIX>` needs a real margin (`date +%s` + 1800 — a bare "now" is
stale before it mines, the DeploySpoils lesson).

**7b. Orchard season 0** - a posted, finite pot on one canonical pool
(FARMING.md 9 suggests ~0.5x the Gardener's monthly stream over 30 days).
Funded the harvest way - a deliberate stewardMint, never a grant. Two gates
before the casts: run `SEASON_POT=<POT_WEI> ./deploy_town.sh preflight` first —
the pot-cap check (≤10% of the OBOL reserve) only ever sees a pot through that
env var, and a manual cast never traverses it on its own. And if a stalled
broadcast is resumed more than an hour late, recompute `<START_UNIX>`:
`createIncentive` requires a start that is not in the past at inclusion time.

```bash
source town.env
# the OBOL granary, NOT $GRANARY — that alias names the gardener's MYRRH one
# (FARMING.md 3b). stewardMint on the wrong granary mints MYRRH while the
# approve below spends OBOL you do not hold: the season fails to post, or
# posts beside an unbacked MYRRH mint sitting in your wallet.
cast send $GRANARY_OBOL 'stewardMint(address,uint256)' <OPERATOR> <POT_WEI> --private-key ...
cast send $OBOL_TOKEN 'approve(address,uint256)' $ORCHARD <POT_WEI>   --private-key ...
cast send $ORCHARD 'createIncentive(address,uint256,uint256,uint256)' \
          <V3_POOL> <POT_WEI> <START_UNIX> <END_UNIX>                 --private-key ...
```

When the season ends: **unstake every remaining position first (permissionless
after the close), then `endIncentive`.** A stake left in place past the close
keeps the sweep hostage, and clearing them promptly is what keeps the
remainder the operator's to sweep.

**7c. Harvest funding** (when the claim bridge arms, POOLS.md 3 step 3):
before the `granary` phase runs, the deployer is still OBOL's minter and mints
straight to the harvest; after it, the mint goes
`$GRANARY_OBOL.stewardMint(harvest, total_committed)` — the **OBOL** granary,
never `$GRANARY` (the MYRRH one; a MYRRH mint into an OBOL-denominated harvest
makes every claim revert on an empty balance AFTER the root is public). Then
`ScryHarvest.postRoot(...)` against the public ledger — fund first, always.

**7d. MYRRH's minter is the granary, not the deployer** - corrected
2026-07-26, and the old text here was actively dangerous. **The Garden is
MYRRH's ONLY source** (operator, `FARMING.md` §3a): play mints none, the barrow
drops none, so the farm's reward token is MYRRH and `DeployGardener` calls
`reward.setMinter(granary)` on it. That is a real rotation, it happens at the
gardener step, and **only the current minter may perform it**.

> ⚠ **Consequence for `rotate`.** If `rotate --arm` moves MYRRH's mint key before
> the gardener step has run, the farm's setup is handed to whoever holds the new
> key. **Rotate the coins separately** unless that is the intent. `LAUNCH.md` §4
> carries the same warning at the phase it fires in.

The silo and the orchard still pay **OBOL**; only the Gardener's reward moved.

**If the silo grant did not set inline** (broadcaster was not the steward):
`cast send $GRANARY_OBOL 'setGrant(address,uint256)' $SILO 250000000000000000000`
— the **OBOL** granary and the posted **250/day** cap. (This line carried the
MYRRH granary and a 10×-low 25e18 until 2026-07-27: the wrong granary lands the
grant on a contract the silo never calls, and 25/day against a 120/day drip
turns ~4 of every 5 harvested OBOL into a stash IOU with nothing reverting.)

**7g. The Bank seed — the one arming decision whose correct first move is to
NOT make it.** `TREASURY.md` §P9 earmarks **9,000,000 SCRY** to swell the bank by
plain transfer. That is right *after* the bank has a staker and wrong before it,
in a way that costs the whole tranche rather than a slice:

```bash
# the gate, every time, before any transfer to the bank:
cast call $BANK 'totalSupply()(uint256)' --rpc-url $RPC   # must NOT be 0
```

`enter()` reads the pool **before** it pulls the deposit, and the
`totalSupply == 0` branch mints `amount - MINIMUM_SHARES` against the
depositor's own amount. So SCRY sitting in an unstaked bank is priced into
nobody's shares: **the first deposit of any size redeems the entire balance.**
9,000,000 in, one SCRY deposited, 9,000,001 out. `MINIMUM_SHARES` is 1000 *wei*
of shares and is not a haircut. Once a real stake exists, a transfer accrues pro
rata to the stakers who are there, and §P9's trickle reasoning applies as
written. `./deploy_town.sh status` prints **NOT SEEDABLE** while the supply is
zero; `GET /bank` carries the same gate as `seedable`, and
`ScryEconomy.t.sol::test_seedIntoAnEmptyBankIsCapturedWholeByTheFirstDepositor`
holds the arithmetic.

## 3. What green looks like (`./deploy_town.sh status`)

- OBOL minter **= the OBOL granary** once the `granary` phase has run (it
  funds the silo's drip and every season pot via `stewardMint`; if not, those
  cannot pay - fix before anything else). The orchard itself never mints — a
  season is funded up front or it does not exist.
- grant room today: gardener 12500e18 (asked of the MYRRH granary), silo 250e18
  (asked of the OBOL one — `status` asks each organ's own granary since
  2026-07-27; it used to ask the MYRRH one for both and printed the silo as 0).
  ⚠ **Derive these, never trust this line.** It said 500e18 for the gardener
  in three places until 2026-07-28 while `DeployGardener.s.sol` welded 12,500 —
  so `status` printed a CORRECT deploy and this page called it broken. The
  remediation that invites, `setGrant(gardener, 500e18)`, clamps a 6,000/day
  farm to 8% and stashes the rest with nothing red. `meter/test_tokenomics.py`
  law 8b.3 gates the real number against the deploy script.
- gardener pools 1 · silo bins 2 / tiers 4 · shrine altars 2 ·
  orchard seasons 0 (until 7b).
- MYRRH minter **= the granary** once the gardener step has run (the Garden is
  MYRRH's only source - see 7d). Before that step it reads as the deployer,
  which is expected; after it, a deployer still holding the slot means the farm
  cannot pay. `status` derives the farm's coin rather than hardcoding it, so
  confirm the line names **MYRRH**.

## 4. Week one (unchanged from POOLS.md 4.3, restated)

Watch `minted - burned` on `GET /tokens` and the pool's SCRY side daily.
The levers, in the order to reach for them: granary daily caps ->
`setRewardPerSecond` -> allocation points -> sink prices. Tuning is an
operator responsibility the code meters but does not decide. No APY is
ever quoted anywhere.

## 4b. Standing hazards on contracts that are ALREADY LIVE

These cannot be patched — the contracts are broadcast. They are here so the
next operator meets them in the runbook and not in an incident.

**`ScryVowRegistry.setOracle` has no zero-check** (live at
`0x08131e7660639bbd086dffa9375c2a563f1d3590`, audit 2026-07-25 §3). The
oracle rotates itself and nothing else can rotate it:

```solidity
function setOracle(address newOracle) external {
    if (msg.sender != oracle) revert NotOracle();
    oracle = newOracle;      // no require(newOracle != address(0))
}
```

So **one mistyped address ends daily anchoring forever**, with no owner path
back — the vows already sworn stay readable, but no new merkle root can ever
be posted. Before any `setOracle`:

```bash
# 1. the address must have code or be a key you control - check BOTH
cast code   <NEW_ORACLE> --rpc-url https://rpc.mainnet.chain.robinhood.com
cast call   0x08131e7660639bbd086dffa9375c2a563f1d3590 'oracle()(address)' \
            --rpc-url https://rpc.mainnet.chain.robinhood.com   # who holds it now
# 2. send from the CURRENT oracle key, and read it straight back
cast call   0x08131e7660639bbd086dffa9375c2a563f1d3590 'oracle()(address)' \
            --rpc-url https://rpc.mainnet.chain.robinhood.com
```

Rotate to a key you have already signed something with. Never rotate to an
address you have only read off a screen.

**`./deploy_town.sh status` now reads the slot back on every run** (added
2026-07-26). A hazard note in a runbook is only met by someone who opens the
runbook, and this failure is *silent* — nothing else on the box reads
`oracle()`, so a bad rotation shows up as roots quietly ceasing to post, weeks
later, with no error anywhere. `status` prints the current oracle, says whether
it is an EOA or a contract, and shouts if it has become the zero address. That
does not make the slot recoverable — nothing can — it just means the town finds
out the same day.

## 5. What is NOT in this runbook, on purpose

- **Custody above cap 0** (agency wallets) - different organ, own
  sentence, sanity-check first (CLAUDE.md).
- **Canonical-pool seeding** (real SCRY/USDG depth) - POOLS.md 3-4 is
  its own runbook (`SeedSpoilsUniswapV3.s.sol` + helper).
- **UI pages for silo and shrine** - `gardens.html` and `orchard.html`
  both exist (the orchard page ships a full approve → deposit → stake →
  unstake → claim flow behind a `data-wip` banner, linked from
  `gardens.html` and `floor.html`). This line claimed the orchard page did
  not exist until 2026-07-28. The silo and shrine remain contract-first.
- **Anything the meter serves** - that is `DEPLOY.md`, a different
  machine and a different checklist. This runbook never touches the
  signing key, the rails, or a measurement.
