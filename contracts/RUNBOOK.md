---
status: live
lane: [ops, launch]
updated: 2026-08-12
about: "the contracts deploy runbook — deploy_town.sh, the real gate (upstream forge + live fork mandates), who holds what power, the standing hazards on live and unbroadcast contracts, and the licence table for anything vendored"
---
# RUNBOOK.md - the town deploy: zero to the full DFK stack on RH-Chain

> ## ▸ For the LAUNCH itself, `./deploy_town.sh launch` is the plan of record
>
> The launch order is **preflight → check → test → spoils → pools → harvest**,
> and it stops at "a DeFi game on RH-Chain." That supersedes the ordering on
> this page for what ships first, and it is no longer prose anywhere: the
> `launch` case in `./deploy_town.sh` **is** the order, and it demands every
> input the later phases need (`PRIVATE_KEY`, `NPM`, `V3_FACTORY`, the full
> `$SCRY` size) before the first send, because a precondition that fires after
> an irreversible act is not a precondition. The locked launch *numbers* — pool
> sizes, opening ratios, the two drops, the powder — are
> `docs/launch/LAUNCH-DECISIONS.md`. This runbook stays correct for the **depth** phases
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
| `FEE_BPS = 30` (0.3%) | `ScryGarden` | a `constant`, not even immutable — the Garden's LP fee cannot move. **Load-bearing, not an inherited default** (operator, 2026-07-27): the Garden is 1% of the v3 pool's depth, so the fee discount is the ONLY thing that can make it the better venue, and it does — up to ~716 OBOL, about one delve's spoils. It went 30 → 100 and straight back the same day; at a matched 1% the Garden is strictly dominated at every trade size. `EloGarden.sol`'s notice carries the crossover table |
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
`--rpc-url "$(printf '%s' "$ELO_RH_RPC_POOL" | cut -d, -f1)"` or the whole
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
| `MockArbSys` used by | `EloGacha.t.sol`, `EloGachaFork.t.sol` — **only the gacha** |

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
| — | `./deploy_town.sh launch --arm` | **the whole launch broadcast in one command** — preflight → check → test → spoils → pools → harvest, each gated (`NOW.md` §1). It does **NOT** run `rotate`: the `launch` case stops at `harvest`, and this row said otherwise until 2026-07-28 | everything above it gates |
| — | `./deploy_town.sh claims` | **read-only.** Checks every posted root and total against the merkle artifact it came from, and refuses to call a drop safe if they disagree. Takes no `--arm` — it broadcasts nothing | a drop's claim contracts recorded in `deployments.json` |

**On the `bank` row, and why it had none until 2026-07-27.** It needs no
minter, no granary, no pool and no address from any other phase — which made it
the one organ nothing ever blocked on, so it was simply left out of the launch
while being the third leg of the JEWEL design.

⚠ **It is unordered with respect to what it CONSUMES, and strictly FIRST with
respect to what consumes IT.** This row read "it can run before phase 2 or
after phase 9" until 2026-07-28, which is wrong in one direction and welds if
followed: `ScryEidolon.feeSplitter` and `ScrySteleEdition.feeSplitter` are
`address public immutable`, and all three of `DeployEloEidolon.s.sol`,
`DeployEloSteleEdition.s.sol` and `DeployEloMarket.s.sol` take
`SCRY_FEE_SPLITTER` as a **mandatory** `vm.envAddress`. Deploy any of them
first and the likely path is exporting a placeholder to get the broadcast
through — after which every priced mint and every ERC-2981 royalty routes
there forever, outside the posted split, with no setter. Only
`ScryJobBoard.setFeeSplitter` can be repointed.

Two properties are not negotiable:

- **The posted split is burn 5000 / bank 4000 / prizes 0 / ops 1000**
  (2026-07-27, `SENTENCES.md`). Prizes take
  no line at open: that cut needs an escrow wallet, and pointing it at ops
  would post four outlets while paying three. **The numbers are DERIVED, not
  retyped** — they live in `DeployEloEconomy.s.sol`'s `vm.envOr` defaults and
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
> key. **Rotate the coins separately** unless that is the intent. The `rotate`
> case in `./deploy_town.sh` carries the same warning at the phase it fires in,
> and its check (0) *refuses* rather than warns once a granary already holds the
> slot — `setMinter` is `onlyMinter`, so the cast would revert anyway; it just
> used to find out at the end of the sitting.

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
`EloEconomy.t.sol::test_seedIntoAnEmptyBankIsCapturedWholeByTheFirstDepositor`
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

## 4c. `ScryBurrow` — the oracle finding, and the one rule for whoever fixes it

*Salvaged 2026-08-12 from §2 of the PvP-lending design doc that lived at
`docs/agent-town/PVP-LENDING` — culled the same day, recoverable from git
history if you want the rest of the design.
The contract is written and **not broadcast** (`contracts/src/EloBurrow.sol`,
absent from `deployments.json`), so unlike §4b this one is still fixable — which
is exactly why it has to survive the doc that carried it.*

**`ScryBurrow.liquidate()` reads the Garden's spot price in the same
transaction that seizes:**

```solidity
function liquidate(address borrower) external nonReentrant {
    require(healthFactor(borrower) < 1e18, "healthy");   // ← Garden spot, this tx
```

(`EloBurrow.sol:130–132`.) No delay, no TWAP, no cooldown. So the entire
attack is **one atomic call**:

```
swap to shove spot  →  liquidate(victim)  →  swap back
```

Nobody gets a turn. There is no reaction window, no counterplay, no opponent. A
bot lands it every time and no human ever plays. **This is not PvP — it is a
solved extraction loop with a leaderboard of one address.** It is also why the
contract was safe *only* while the tokens were worthless: the manipulability was
never a game, it was a disclosed defect that did not matter at zero stakes. The
tokens are no longer worthless, so **do not broadcast this contract as written.**

**The fix is to split liquidation in two**, so the manipulation must be *held*
rather than flashed:

```
flag(borrower)        →  legal when healthFactor < 1, costs a burned fee,
                         emits Flagged(borrower, flagger, block.timestamp)
liquidate(borrower)   →  legal only at flaggedAt + FLAG_DELAY_SECONDS,
                         and only if healthFactor < 1 *again*, now
```

The second health check is the whole design: to collect, an attacker must hold
the price down across the window — capital committed at a bad price, in public,
against every arbitrageur on the chain and against a victim who can see the flag
and act.

> ### ⚠ SECONDS, NOT BLOCKS
>
> `FLAG_DELAY_BLOCKS` appears in no contract; it is an unbuilt spec, which is
> the only reason this is still fixable. **Implementing it against
> `block.number` would reproduce the `ScryGacha` bug exactly** (§0c): chain 4663
> is Arbitrum Nitro, so Solidity's `block.number` returns the PARENT chain's
> height and advances once per ~12s while the L2 runs at ~101ms. A "100 block,
> ~10 second" window would really be **~20 minutes** — and nothing would fail,
> no test would go red, the game would just quietly stop being an arcade.
>
> `EloBurrow.sol` already gets this right and paces everything in
> `block.timestamp` (`:56, :61, :64, :69`). Whatever implements the flag delay
> must do the same, or use `ArbSys(0x64).arbBlockNumber()` for true L2 height.
> Foundry implements no ArbOS precompile, so a fork test proving the clock has
> to etch `contracts/test/MockArbSys.sol` with its fields set explicitly —
> `vm.etch` runs no constructor.

The posted delay is **`FLAG_DELAY_SECONDS = 10`**. RH-Chain's ~100ms blocks are
what make a 10-second window playable — fine-grained enough to be a game — but
the 100ms block time is never what the delay should be *measured* in. State it
in seconds. This is the second entry in §0c's hazard list ("the burrow's siege
window reasons in blocks") stated as the finding it came from.

## 4d. `ScryGacha` — the two identities, and why a shared pot dilutes nobody

*Salvaged 2026-08-12 from §§0b/0c/12b of the gacha design doc that lived at
`docs/items/GACHA` — culled the same day, recoverable from git history. The
contract is LIVE (`ScryGacha`, `0xb63Ce4F300193413884fBf1568548c9b48ba3b0f`),
so these are properties of broadcast code, not of a design page.*

Two numbers never move, whatever the pool holds:

| | value | it is |
|---|---|---|
| depositor's base margin | **+3.455%** | `surcharge × (1 − houseDraw) × (1 − topShare)` = `1.10 × 0.99 × 0.95` = **1.03455** |
| floor-only edge | **−22.727%** | `1 − 0.85/1.10`, i.e. `1 − bidBps/surcharge` |

**Neither expression contains a backing.** Expected exposure per draw is
`pᵢ·bᵢ = ev/n` for *every* position — the same number, because weight is
**inverse** to backing — while the equal fee share is `ev · 1.1 · 0.99 · 0.95 / n`.
The `n` and the backings cancel out. So a dust position and a 2 ETH position earn
the same margin over their own risk, which is why the equal split is the only
fair split and why nobody is subsidising anybody.

**The consequence that matters operationally: a shared pot cannot dilute a
depositor.** Run through `gacha.book()` — the same arithmetic the contract uses —
adding twenty dust positions at the 0.01 ETH floor to a real pool:

| | ticket price | harmonic mean | depositor margin | floor-only edge | at risk / draw |
|---|---:|---:|---:|---:|---:|
| per-collection (0.5 ×3, 2.0) | 0.676923 ETH | 0.615385 | **+3.455%** | **−22.727%** | 0.15384615 |
| shared pot (+20 dust) | **0.013157 ETH** | 0.011961 | **+3.455%** | **−22.727%** | 0.00049838 |

Both invariants are unmoved to the last digit, and that is not luck — **the
margin is an identity in the posted bps.** What dust actually does is drop the
ticket price 51× and lengthen the odds, which is the trade a lottery ticket is.
Reproduce it, no deployment needed:
`GET /api/gacha/book?backings=0.5,0.5,0.5,2.0` against the same with twenty
`0.01`s appended.

⚠ **The wall that survives, and it is an arming rule.** A shared pot is safe
because **nothing accrues to a depositor except the fee split**, which is exactly
proportional to expected loss. Add any second reward — an emission, a points
program, a loss-to-earn — and the self-pool attack comes straight back, because a
second reward is a shared *reward* pot and dust dilutes it. **A shared NFT pot
and a shared reward pot are different objects. We have the first and must never
have the second without time-weighting** (`backing × seconds-deposited`, never a
snapshot). That condition is cheap to honour before the fact and impossible
after.

The posted knobs the identities are computed from are live in the contract —
`houseDrawBps = 100` (1%), `topShareBps = 500` (5%), per-pool `surchargeBps` and
`bidBps` (`bidBps` is fixed at `openPool` and never editable). Derive the two
numbers from whatever the pool actually posted; do not retype the +3.455% /
−22.727% pair for a pool opened at other bps.

## 4e. `ScrySeat` — putting real seats in real wallets

*The operator's own mint. `MintSeat.s.sol` is the script; this section is the
ordering around it, and the first line is the one that surprises people.*

> **For the whole run in order — cohorts → roots → salt → deploy → doors → mint
> → close → reveal → the drop bar — read `docs/launch/HIVE-LAUNCH.md` first.**
> This section is one phase of it. That guide exists because the ceremony was
> spread across six files with no single order, and four of its steps cannot be
> undone.

**⚠ THE FOUR PUBLIC DOORS DEPLOY SHUT, SO THE TREASURY DOOR IS THE ONLY MINT
THAT WORKS ON DAY ZERO.** `DeploySeat` posts no root and sets no price on
purpose — nothing can be minted by anyone, including by whoever is watching the
mempool during the broadcast. `claim` refuses ("door closed") and `buyWithEth`
refuses ("eth leg shut") until `setDoorRoot` / `setPaidDoor` are their own later
transactions. `mintTreasury` is `onlyOwner` and needs neither, which is why it is
how the first seats reach a wallet — and why a front end is first exercised
against **door 5**, not against door 1.

```bash
export PRIVATE_KEY=0x...                     # MUST be the collection's owner()
export SEAT=0x...                            # the deployed ScrySeat
export SEAT_MINT_TO=0xaaa...,0xbbb...        # recipients, comma separated
forge script script/MintSeat.s.sol --rpc-url $RPC              # DRY RUN FIRST
forge script script/MintSeat.s.sol --rpc-url $RPC --broadcast
```

The dry run performs every check against real chain state — owner, mint closed,
cap headroom, supply headroom — and echoes the recipients **parsed rather than
as typed**. Read them there. Nothing burns a seat: `ScrySeat` has no burn path,
so a transposed character is a permanent seat in a wallet nobody holds, and it
has spent an allocation bounded by an immutable cap.

**What a seat minted this way is, and it is not a claimed one:**

| | |
|---|---|
| `doorOf` | **5** — the treasury door, deliberately outside the four a visitor may walk |
| `tierOf` | **0 — BENCHED.** `mintTreasury` does not activate and could not: `activate` is the holder's own call |
| `weightOf` | **0**, until its holder sits it |
| counted at | `treasuryMinted`, public, against an immutable `treasuryCap` |

**⚠ THE RECIPIENT NEEDS ITS OWN GAS AND ITS OWN SCRY TO DO ANYTHING WITH IT.**
`mintTreasury` pushes the seat, so a wallet at nonce 0 with no balance receives
one fine — it just cannot then transfer, activate, or set an election, because
all three are calls the holder signs and `activate` is paid in SCRY by the
holder. A fresh wallet is enough to exercise every **read** surface
(`/api/seat/gallery`, `/api/seat/of/<wallet>`, the seat page, the marketplace
metadata) and none of the **write** ones. Fund it, or point at a wallet that is
already funded, before concluding a button is broken.

**⚠ THE FACES ARE THE SEALED CARD, AND THAT IS NOT A BUG TO CHASE.** Until
`revealSalt` — which cannot be called before the run mints out or `closeMint` —
every face is the same picture by construction. A gallery of identical seats
pre-reveal is the design working. `/api/seat/gallery` says so in `sealed_note`.

### Testing the front end without welding the real collection

The real deploy is welded: supply, the four caps, the ladder and the salt
commitment are immutable, and tier 1's SCRY price is **derived off that day's
tape** — it is the one figure `DeploySeat` refuses to supply. Deploying the real
run to get a front end lit is paying a permanent price for a temporary need.

`meter/seat.py`'s `seat_address()` reads **`SCRY_SEAT` first and
`deployments.json` only as a fallback**, and that seam is the cheap path: deploy
a throwaway with test figures, point the env at it, exercise everything, unset
it. A throwaway that is never written to `deployments.json` leaves the durable
record untouched, and unsetting the var makes the surface dark again.

```bash
export SEAT_SUPPLY=64 SEAT_SNAPSHOT_CAP=8 SEAT_PLAY_CAP=8 SEAT_BUILD_CAP=8
export SEAT_TREASURY_CAP=24 SEAT_BURN_BPS=5000        # float: 64-48 = 16
export SEAT_TIER_COSTS="1,3,6,12,20"                  # whole SCRY — cheap so
export SEAT_TIER_WEIGHTS="100,160,220,275,333"        # activation is testable
```

The multiples are the locked ones (`SENTENCES.md` 2026-08-12 — 1/3/6/12/20,
scaled down by 20,000 from the real **20k → 400k** so a throwaway is cheap to
activate) and the constructor
enforces sublinearity, so a test ladder must keep the shape even when the price
is play money — which is the point: the test collection then behaves like the
real one. `SEAT_TREASURY_CAP` has to be typed high enough for the wallets you
mean to mint to, because it is immutable on the throwaway too.

**What a throwaway does NOT test**, and neither does the real deploy until its
own later transactions land: doors 1–3 need a posted root, door 4 needs a posted
price. `MintSeat` reaches none of them.

## 4f. `ScryCistern` — funding the drop bar by hand

*Written 2026-08-12 answering the operator directly: "so i can just manual dump
fees from SCRY/WETH pool into cistern?" **Yes — the deposit side needs no
integration at all.** The three things that bite are below, and the second one
destroys money.*

⚠ **Nothing here is deployed**, but the script now exists —
`script/DeployCistern.s.sol`, which puts all three unspoken knobs through
`_mustUint` so a forgotten one aborts naming itself instead of defaulting. The
cistern cannot be deployed first: the constructor takes the seat address and
calls `seat.MAX_ELECTION()`, requiring exactly 3, so **`ScrySeat` deploys before
the cistern, always.** `docs/launch/HIVE-LAUNCH.md` §12 is this step in its
place in the run.

### 1. The deposit is a plain transfer, and that is the whole answer

`pooled()` is `scry.balanceOf(address(this)) - outstanding`. It reads a
**balance**, not a ledger, so **SCRY sent to the address by any means is a
contribution** — a wallet transfer, the fee splitter's route, a script, anything.
There is no register step, no approval and no call.

`fill(amount)` exists only so a contributor can be *attributed* on chain: it is
`transferFrom`, so it costs an approval first and emits `Filled`. A bare transfer
funds the bar identically and emits nothing. Use `fill` when you want the deposit
credited to a name; use a transfer when you do not care.

### 2. ⚠ THE POT IS SCRY. WETH SENT HERE IS GONE — PERMANENTLY

Collecting a v3 position pays **both legs**. A SCRY/WETH `collect()` hands back
SCRY *and* WETH, and the two are not interchangeable here:

- the **SCRY leg** → transfer it in. Done, it counts immediately.
- the **WETH leg** → **swap it to SCRY first.** The cistern's own transfers all
  name `address(scry)`, so a WETH balance sitting here counts toward nothing.
  ⭐ **It is no longer destroyed by the mistake.** `rescue(token, to, amount)`
  landed 2026-08-19: `onlyOperator`, and it reverts on `scry`. The old text
  said the absence of a rescue was load-bearing "because it is what the rug
  screen is reading" — struck; what it actually bought was a permanent trap on
  the documented funding path. **The pot is still unreachable, and now for a
  mechanical reason rather than a promise:** `pooled()` is
  `scry.balanceOf(this) - outstanding`, and `scry` is the one token `rescue`
  refuses.

So the hand path is: `collect()` on the position → swap the WETH leg to SCRY on
our own pool → transfer the whole SCRY amount in. Swapping that leg is a **buy**
on SCRY/WETH, which is the honest version of what this bar is for.

The position NFTs are held by the dev wallet
`0xb474a95200bC5de8950E445B1E9d524f4de0f18D` (NPM `0x73991a25`, canonical v3,
1% tier, full range — `deployments.json`).

### 3. A round needs somebody on the roster

`open()` requires two things, and the second is easy to forget:

```
pooled() >= threshold        the posted bar
seat.totalWeight() > 0       "nobody is on the roster"
```

`totalWeight` only moves when a holder **activates**. Until the mint has run and
at least one seat has bought a tier, the cistern will take deposits and no round
can open at any balance. Nothing is lost — value only ever leaves by
`open` → `claim`, and an unclaimed remainder `recycle`s into the next round —
but funding it before the mint just parks SCRY in a contract nobody can empty.

**Order, therefore:** deploy `ScrySeat` → **deploy the cistern** → open the doors
→ mint → somebody activates → fund it → anyone cranks `open()`.

⚠ **DEPLOYING IT LATE WAS A PRODUCT MISTAKE AND THIS LINE USED TO CARRY IT.** It
read *"mint → somebody activates → deploy the cistern"*, which reads as a
requirement and is not one: the constructor needs the seat's address and
`MAX_ELECTION() == 3`, and **not one activated seat**. Waiting only meant the
mint page asked people to burn 20,000 SCRY toward a contract with no address —
`/api/seat/utility` correctly reported the drop bar as *"written, not deployed"*
the whole time, which is honest and is also the weakest possible pitch for the
thing activation buys. Deploy it empty and inert right after the seat; that is
the same shape as the seat itself landing with every door shut.

**Size the three knobs on the day, against the tape:**

```bash
python3 meter/cistern_sizing.py            # --lp-share once you know ours
```

⚠ It prints a trade-off and refuses to pick. Two things pull against each other
and **no value satisfies both at a low fee rate**: a tier-1 share has to beat
what a claim costs that holder (including the router swap, for an elected coin),
and the bar has to fire often enough that somebody watches it. That is a fact
about the revenue rather than about the knob, and the response is to say it, not
to pick a number that hides it.

⚠ **Say no rate on any surface that describes this.** It distributes what the
fees actually collected: a quiet market pays nothing, and that is the design
rather than a shortfall.

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

## 6. Licences — check this BEFORE a line is copied into `contracts/src`

*Salvaged 2026-08-12 from §4 of the fork survey that lived at
`docs/onchain/FORKS` — culled the same day, recoverable from git history. A
licence is a fact, not a design note, and a wrong assumption here is not a bug
you can patch — it is a deployed contract you have to take down.*

**Where the tree stands today** (measured, and re-measure before quoting it):
external Solidity dependencies are **one — `forge-std`, and it is test-only**.
No vendored OpenZeppelin, Solmate or Solady. `IERC20`, `SafeERC20` and
`ReentrancyGuard` are hand-written, local, ~30–60 lines each. **Licence MIT
throughout.** A repo with zero vendored code has zero licence questions, which
is most of why the no-fork default is worth keeping.

**Do not take a licence from this table. Verify it at the source before any line
is copied.** Licences change (Uniswap V3's BUSL converted on schedule) and repos
relicense; the confidence column is part of the content.

| project | licence, to the best of the 2026-07-26 survey | confidence | consequence |
|---|---|---|---|
| OpenZeppelin | MIT | **high** | free to vendor |
| Solady | MIT | **high** | free to vendor |
| **Solmate** | **AGPL-3.0** | **high** | ⚠ copying it into an MIT repo is a licensing event, not a convenience. **This is the trap**, because Solmate is the one people copy from muscle memory |
| Uniswap V2 core | GPL-3.0 | **high** | copyleft; irrelevant, we did not copy it |
| Uniswap V3 core | BUSL-1.1, converted to GPL-2.0-or-later on its change date | medium | fine to read; we consume the deployed pools rather than fork them |
| Compound V2 | BSD-3-Clause | medium-high | permissive; the friendliest thing in the lending column |
| Gnosis CTF | LGPL-3.0 | medium | usable, but weak copyleft has integration implications worth reading before vendoring |
| Aave v3 · Uniswap v4 · Compound III · Morpho Blue | **BUSL-1.1 family** | medium — **verify each** | ⚠ a Business Source Licence forbids competing production use until its change date. **This alone disqualifies them as forks**, whatever their technical merit |

**The rule to carry: anything BUSL is *reading material*, never source.** That
covers most of the modern lending and AMM designs — which is one more reason the
Nexum's answer was going to be "write it" regardless (§4c).

The one place worth breaking the zero-dependency rule is the ERC-20 / 721 / 1155
/ 6909 / 4626 **interfaces and reference behaviour**, where conformance is the
entire value: take OpenZeppelin (MIT), in a single deliberate reviewed commit,
not four contracts each importing something different. Do **not** re-vendor
`SafeERC20` / `ReentrancyGuard` / `Ownable` — they are written, tested and 40
lines. Solady's gas golf is not worth the readability here at L2 gas prices, and
**Solmate is the one to avoid outright** for the licence reason above.
