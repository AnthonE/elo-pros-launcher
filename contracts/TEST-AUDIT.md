---
status: record
lane: [launch, ops]
updated: 2026-07-26
about: "the 2026-07-22 read-audit of the Solidity test suites: does a green forge test justify broadcasting without a testnet pass? Its opening line — 'no one has ever executed this suite' — was true on 07-22 and is not now; the suite is 413/413 offline. Read it for the argument, never for the counts"
---
# TEST-AUDIT.md — is a green `forge test` a legitimate mainnet gate?

> **Dated 2026-07-22.** A five-track adversarial read-audit of all 17 Solidity
> test suites against their contracts, deploy scripts, and the real on-chain
> state. The question audited is NOT "are the contracts correct" but **"if
> `forge test -vv` runs green, does that green mean enough to broadcast to
> RH-Chain mainnet without a testnet pass?"** Read this next to `NOW.md`
> (the punch-list) and `deployments.json` (what is actually live).
>
> Reality check first: **no one has ever executed this suite** — there is no
> CI, no cached artifacts, and the authoring environment had no forge. Green
> is currently hypothetical. Run it before believing anything, including this
> audit (a read-audit can misjudge what an execution would reveal).

---

## The headline answers

**"Can forge test carry me straight to mainnet?"** — Per track:

| track | carried by green? | why |
|---|---|---|
| Registry organs (VowRegistry·Notary·Covenant·Pact) | **mostly yes** | no fund custody; Covenant is the model suite; close the Pact auto-sign hole first |
| Collectibles (Eidolon·Stele·Deed) | **yes for mint economics** | cap/conservation/royalty genuinely pinned; safe-transfer surface thin (Deed now covers it; Eidolon/Stele do not) |
| Arbiter + JobBoard disputes | **yes, after today's fixes** | fee-independence pinned across all 3 verdicts; churn-mid-case now pinned; two board behaviors flagged below need an operator call |
| Money core (JobBoard modes·Bank·Pool·Splitter·Harvest) | **NO** | RepOnly mode unexecuted, Bank's advertised donation-guard untested, pool cap/auth untested, operator knobs dark |
| Farm stack (Gardener·Granary·Silo·Shrine) | **Silo/Shrine yes; Gardener NO** | Gardener never runs two stakers in one pool — the split math that divides money is unproven |
| Orchard (v3 staker) | **NO — fork test mandatory** | the pool clock is a hand-set mock; mispricing/stranding vs the real NPM is invisible to green |
| Spoils seeding (SeedSpoilsUniswapV3) | **NO — demonstrably** | the suite passes its own price-inconsistent inputs (sqrtP=1.0 vs amounts=0.01); an inverted launch price ships green |

**"Should I testnet?"** — Not necessarily. **Fork tests replace it honestly**
(`forge test --fork-url <RH RPC>` runs against live mainnet state, spending
nothing, with the real token and the real NPM — strictly more realistic than
a testnet's fake liquidity). The two places a fork run is *mandatory*, not
optional: **ScryOrchard** and the **SeedSpoilsUniswapV3 flow**. Everything
else: unit green + the additions below + a dust-first mainnet rollout
(a $1 job, a 1-wei mint) is a defensible path.

---

## Grounding: the real SCRY (so mock-fidelity arguments stop being guesses)

Verified against RH-Chain (Blockscout source + `eth_call`, 2026-07-22):
`0xDa2a…C4B0` is **PonsLauncherToken — a plain OpenZeppelin ERC-20**. Fixed
supply, **no owner, no tax/fee-on-transfer, no blacklist, no pause**. Launch
anti-snipe (max-wallet/max-tx) applied only to pool buys for **366 L1 blocks
(~73 min)** after the 07-15 launch (`launchBlock` 25,539,223 →
`restrictionEndBlock` 25,539,589, both immutable) — long expired. Its own
natspec: "Afterward the token behaves as a plain ERC-20."

Consequences: (a) always-revert-on-shortfall mocks are behaviorally faithful
for every path scry's contracts use; (b) fee-on-transfer / return-false
findings are **robustness** notes (self-hosters may wire any token), not
canonical-token mainnet triggers; (c) OZ reverts with *custom errors* — a
test asserting a token's revert *string* is asserting a mock dialect. OBOL/
MYRRH have no such excuse: the repo owns `SpoilsToken.sol`, so suites must
test against the real one (the Deed suite now does).

---

## Per-suite verdicts (17 suites)

| suite | contracts | verdict | the one thing to know |
|---|---|---|---|
| ScryMarket.t.sol | JobBoard·Reputation·InsurancePool | **ADEQUATE as smoke, NOT a gate** | honest assertions, one-example-deep; RepOnly mode has zero positive tests |
| ScryEconomy.t.sol | Bank·FeeSplitter·Harvest | **THEATER (Bank) / ADEQUATE (rest)** | Bank's `test_noAdminSurface` is a vacuous name; donation attack named in-contract, untested; `leave()` conservation never asserted |
| ScryGardener.t.sol | Gardener·Granary | **ADEQUATE→thin** | never two stakers in one pool; production-scale precision floor (rps 2.78e14) untested; LP-donation grief unpinned |
| ScrySilo.t.sol | Silo | **SOLID** | early-break burn proven three-sided; BUT test tier menu ≠ DeploySilo's menu — align it |
| ScryShrine.t.sol | Shrine | **SOLID** | carries as-is; custodies nothing, setUp matches deploy exactly |
| ScryOrchard.t.sol | Orchard | **mechanics ADEQUATE, integration THEATER** | hand-set mock clock; shared-range different-liquidity positions inexpressible; mid-season unstake denominator unpinned |
| SpoilsToken.t.sol | SpoilsToken·PlayToken·Garden | **ADEQUATE** | minter gate/cap/elastic genuinely proven; `mint(address(0))` allowed + untested; setMinter negatives missing |
| SeedSpoilsUniswapV3.t.sol | the seed script | **THEATER (loss dimension)** | passes price-inconsistent inputs green; `vm.setEnv` cross-test pollution may make first real run flaky; price math lives in un-gated Python |
| ScryPlayground.t.sol | Burrow·Garden·PlayToken | **ADEQUATE(−)** | oracle-manipulation liquidation test is excellent; x·y=k never asserted — matters because Garden is ALSO deployed vs real SCRY |
| ScryVowRegistry.t.sol | VowRegistry (LIVE) | **ADEQUATE** | soulbound paths complete; zero `expectEmit` though events carry the product; live config (owner==oracle) never tested; merkle cross-check depth-1 only |
| ScryNotary.t.sol | Notary | **ADEQUATE (thin)** | first-committer priority proven; `Notarized` event (the whole product) unasserted; one circular assert |
| ScryCovenant.t.sol | Covenant | **SOLID — the model suite** | asserts full event payloads with `expectEmit`; copy this pattern into the other registry suites |
| ScryPact.t.sol | Pact | **ADEQUATE, two holes** | non-party-proposer auto-sign branch untested; `PactProposed` (the via_ir reason) never asserted, never emitted at posted max sizes |
| ScryEidolon.t.sol | Eidolon | **ADEQUATE** | cap boundary + conservation + royalty real; safe-transfer surface untested; forge green is SILENT on trait provenance (that lives in `meter/test_eidolon.py`) |
| ScrySteleEdition.t.sol | SteleEdition | **ADEQUATE (thin); metadata claim THEATER** | tokenURI checked by 29-char prefix — malformed JSON ships green; one bare `expectRevert()` |
| ScryArbiter.t.sol | Arbiter + real JobBoard | **REBUILT 2026-07-22** | see "fixed today"; churn/governance/3-verdict fee-independence now pinned |
| ScryDeed.t.sol | Deed | **REBUILT 2026-07-22** | see "fixed today"; original had 4 misfiring tests + a mock-dialect OBOL |

---

## Fixed today (this branch) — the audit's direct product

1. **`ScryArbiter.sol` hardened (real liveness flaw).** Removing an arbiter
   who had voted could strand a case unfinalizable forever (tallies survive,
   the `== arbiterCount` exhaustion check becomes unreachable) — the deadline
   path would then decide the job, inverting outcomes. Fix: **the bench is
   pinned per case at first vote** (`caseQuorum`/`casePanel`), votes are
   capped at the pinned bench, exhaustion checks the pinned size. This also
   makes double-quorum provably impossible per case (max votes = casePanel <
   2×caseQuorum), so the finalize branch order is immaterial — the prior code
   could, under churn, have let two verdicts both reach quorum and silently
   ruled by code order.
2. **`ScryDeed.t.sol` rebuilt.** The first version had four tests that
   misfire on Foundry cheatcode semantics (`deed.SCOPE_*()` getter staticcalls
   in argument position consume a pending `vm.prank`/`vm.expectRevert`) and a
   `MockOBOL` whose `burnFrom` dialect (order + revert strings) contradicted
   the real `SpoilsToken`. Now: real `SpoilsToken`, hoisted constants, plus
   operator-pays-toll conveyance, safe-transfer accept/reject receivers,
   zero-to / wrong-from / plot-(0,0) / re-found-after-convey pins, event
   asserts, infinite-allowance carve-out, and a **byte-exact tokenURI
   known-answer** (offline-generated, JSON-validated).
3. **`ScryArbiter.t.sol` extended.** Constructor guards (incl. the pinned
   1-of-1 acceptance vs the runbook's "never a single oracle" warning),
   remove-path governance incl. the 2→1 shrink deadlock, transferOwnership,
   churn-mid-case, quorum-pin vs later raise, `Voted`/`Ruled` event asserts,
   fee-independence across **all three** verdicts, seller-initiated dispute,
   dispute on a Delivered job, stranger-dispute revert, and two pinned board
   behaviors (next section).

---

## The operator questions — DECIDED 2026-07-22 ("that sounds legit — do it all")

All three resolved under the repo's own walls and implemented same-day
(`SENTENCES.md` has the row):

1. **Undecided never slashes.** The panel declined to decide; rep moves only
   on an EARNED outcome (CREED #9), and slashing here would pay spurious
   disputes in seller damage. `dispute()` now refunds under its own honest
   reason `"undecided"`; ForBuyer keeps its slash. Test:
   `test_disputeUndecidedRefundsWithoutSlash`.
2. **Events report what actually moved** (CREED #4 — never fake a number).
   The RepOnly fall-through now emits `Closed(id, "completed-unfunded", 0,
   0)`; the insured-cover path captures `pool.claim()`'s return and emits
   `"completed-insured"` with the amount the pool really paid (cap/thin-pool
   honest). Settlement logic unchanged. Tests:
   `testRepOnlyBuyerSilentClosesUnfundedWithHonestZeros`,
   `testInsuredThinPoolCoverEmitsActualPaidAmount`,
   `test_repOnlyDisputeForSellerUnfundedPaysNothing`.
3. **The Gardener floor is fixed, not accepted.** `carryScaled[pid]`
   accumulates the scaled remainder in `updatePool`, so per-second and
   patient callers arrive at identical state — the permissionless
   permanent-grief and the organic-loss-at-scale both close. Conservation
   proven to the wei at production parameters:
   `test_production_rps_per_second_updates_lose_nothing_carry_conserves`.

---

## Systemic patterns (the cross-suite lessons)

- **Events are the unguarded half of the product.** Vow text, notary memos,
  pact terms, mint/burn amounts live only in events, and only Covenant/Pact
  assert any. `ScryCovenant.t.sol`'s `expectEmit`-with-full-payload is the
  in-repo template — port it.
- **Operator/governance knobs are near-universally untested** (`setFeeBps`,
  `setArbiter`, `setThreshold`, `setMaxPayout`, `setAllocPoint`, every
  `transferOwner/Operator/Steward`). These are the first functions pulled
  after broadcast, on immutable contracts. One test each.
- **Single-actor reward tests.** Gardener (and originally the arbiter suite)
  test one actor per stream; splitting money among concurrent actors is the
  hard part and the untested part.
- **Mock fidelity**: never mock a token the repo owns (SpoilsToken, Garden);
  for SCRY the OZ grounding above bounds what a mock must model; for
  Uniswap v3, no mock is faithful — fork it.
- **Zero fuzz/invariant tests repo-wide** despite forge-native support; every
  numeric law is pinned at hand-picked points.
- **Bare `vm.expectRevert()`** (any revert passes) at: ScrySilo.t.sol:302,
  ScrySteleEdition.t.sol:52, ScryEidolon.t.sol:65, SeedSpoils:156.

---

## Minimum pre-broadcast additions, per track

**Gate 0 (everything):** run `forge build && forge test -vv` on a real box.
Add a CI job that runs it on every push — broadcast stays human, the *gate*
should not be.

**Money core (before `DeployScryMarket` / `DeployScryEconomy`):**
- ReputationOnly positive path + deadline default; `deliver` guards;
  double-settle (`complete`→`close`, `dispute`→`close`, double `close`);
  deadline boundary (`==`, `-1`); the four JobBoard operator knobs incl.
  `setFeeBps` mid-job (decide: lock fee at post?); InsurancePool cap /
  thin-pool / unauthorized-claimant; Bank conservation + donation attack +
  leave-rounds-to-zero; Splitter reverting-recipient (documents all-or-
  nothing); Harvest under-funded claim + **a deploy script** (none exists).
- Resolve the two operator questions above; update the pins.

**Farm (before `deploy_town.sh --arm`):**
- Gardener: two-stakers-one-pool exact split; late-joiner pending==0;
  partial withdraw; partial `claimLocked` under the 50/day cap;
  production-parameter run (rps 2.78e14, large lpSupply — the 1-second
  `updatePool` precision floor); port Silo's rate-change-retroactivity test.
- Silo: align the tier menu to `DeploySilo.s.sol`; `setAllocPoint`;
  mid-interval join pending==0.
- All five: `transferOwner`/`transferSteward` + Granary `setGrant` gates.

**Orchard:** mid-season unstake exact-value test, then **a fork test on
RH-Chain** (real NPM mint → stake → real swap moves price across the range →
unstake; verify seconds-in-range pricing) — the one item no mock replaces.
Also: a rescue path (or pinned refusal) for NFTs sent by plain
`transferFrom`, which currently strand forever.

**Spoils seeding (before ANY pool seed):**
- Script: add the sqrtP-vs-amounts consistency `require` (mulDiv, within
  slipBps) — the missing check the suite currently *pins the absence of*.
- Suite: make the mock enforce mins; fix the `vm.setEnv` pollution (one
  sequential test or zero-address sentinel); assert all MintParams fields +
  `initFee`; exercise the existing-pool path and both sort branches.
- Gate: wire `python3 script/seed_spoils_uniswap.py --selftest` into
  `deploy_town.sh` next to `forge test`.

**Registry/collectibles (lower stakes):** Pact non-party-proposer + max-size
`PactProposed` emit; VowRegistry `expectEmit` sweep + live-config (owner==
oracle) + 3/5-leaf merkle fixtures from `anchor_worker.py`; base64-decode
the Stele/Eidolon tokenURIs (or pin known answers like the Deed's);
safe-transfer receiver tests for Eidolon/Stele; `mint(address(0))` guard in
SpoilsToken (or pin the behavior).

---

## Round 2 (same day) — the minimum additions LANDED

Four parallel tracks executed the "minimum pre-broadcast additions" above:
**~110 new tests** across 15 suite files, plus `DeployHarvest.s.sol` (the
missing script), `OrchardFork.t.sol` (the mandated fork harness, env-guarded
on `RH_FORK_URL`), the **sqrtP-vs-amounts tripwire** in
`SeedSpoilsUniswapV3.s.sol`, the `seed_spoils_uniswap.py --selftest` wired
into `deploy_town.sh`'s arm gate, and one authorized `src/` guard
(`SpoilsToken.mint` now refuses `address(0)`). All compile (0.8.26, via_ir);
all exact values independently re-derived. Execution still pends Gate 0.

**Verdict corrections from writing the tests:**
- **ScryBank: THEATER → covered, and the defense is REAL.** The
  MINIMUM_SHARES donation guard proves out: a victim cannot be rounded to
  zero shares (explicit revert), and the attacker strictly loses ~the whole
  donation. `leave`-rounds-to-zero is unreachable (pool ≥ shares invariant).
- **Orchard mid-season "drain shape": RESOLVED SAFE-BY-DESIGN.** The
  denominator is the full posted window, so an early leaver is diluted,
  never enriched. Pinned exact.
- **The seed suite's old fixture was itself the inverted launch** (amounts
  encoded 0.01 SCRY/OBOL — the inverse of the 100 SCRY/OBOL posted **on the
  date of this audit**; the ratio was later revised and the pools opened at
  **10 SCRY/OBOL** on 2026-07-29 — `LAUNCH-DECISIONS.md` is the authority, not
  this line). The THEATER verdict was, if anything, understated. The tripwire
  now rejects it.
- **Silo deploy-menu divergence: closed** — the verbatim production menu
  seals and matures in-test.

**New/upgraded findings:**
- **Gardener precision floor is a permissionless PERMANENT grief**
  (upgraded from "untested"): `updatePool` is public and advances
  `lastRewardTime` even when the accPerShare increment floors to 0, so at
  `lpSupply > rps × 1e12` a per-second caller zeroes emission forever.
  Pinned, not fixed — **whether to fix the contract (remainder
  accumulation / min-elapsed) is a third operator decision** beside the two
  JobBoard questions.
- **Orchard stranding is total**: a plain-`transferFrom`'d NFT has
  `owner == address(0)` in deposits — no caller, including the contract
  owner, can recover it. Pinned.
- **Pact `roles` strings are unbounded** (title/terms/obligations all carry
  requires; roles do not) — a gas-priced event-spam vector, unpinned, new.
- **`meter/anchor_worker.py` does NOT import on bare python** (module-level
  `web3`; the "pure-stdlib" keccak module imports fastapi) — the merkle
  fixtures were generated through a 12-line shim with the tree code itself
  unmodified. If bare-python importability of the anchor path is claimed
  anywhere, it does not hold today.
- **Merkle proof length is non-uniform** (an odd carried leaf's proof is a
  single node even in a 5-leaf tree) — anyone validating `/vow/{id}/proof`
  by expected depth would falsely reject; now pinned by fixtures.

**Unchanged:** Gate 0 (run forge); both fork mandates (the tripwire checks
env-vs-env only — a rerun against a pre-existing pool still never reads the
live price); the two JobBoard operator questions.

## Standing notes

- `ScryHarvest` has tests but **no deploy script**; `DeployScryEconomy.s.sol`
  explicitly defers it and nothing picks it up. Write `DeployHarvest.s.sol`.
- `SeedSpoilsUniswapV3.s.sol`'s hardcoded NPM/factory defaults are exercised
  by zero tests (every test overrides them); the on-chain `npm.factory()`
  check passes for any *mutually consistent* wrong pair. Re-verify against
  the canonical RH-Chain addresses before arming (`POOLS.md` §4.1).
- The audit itself is a read-audit produced without executing forge; its
  claims about what *would* fail (e.g. the original Deed misfires) are
  high-confidence but unexecuted. The first real `forge test -vv` run is
  also a test of this document.

---

## ADDENDUM 2026-07-23 — the first real run, and the fork mandates: GREEN

The gate this audit companions was **run for the first time 2026-07-23**
(upstream foundry v1.7.1): **322/322 PASS, deterministic**. The audit's
self-test clause above resolves as follows:

- The audit's confidence held for the *contracts*; the surprises were in
  the **harness**: (1) `via_ir` legally CSEs `TIMESTAMP` within a call,
  so `vm.warp` + consecutive `block.timestamp` reads silently collapse —
  14 first-run failures across Gardener/Silo/Playground/SeedSpoils, all
  test-side, fixed with `vm.getBlockTimestamp()`; (2) `vm.setEnv` is
  process-global under parallel workers — the SeedSpoils env round-trips
  raced nondeterministically, fixed by a parameterized `runWith(SeedInputs)`
  script entry (env parsing covered by exactly one test). Rule for new
  suites: never read `block.timestamp` around a warp; at most one
  env-writing test per process.
- One real contract bug surfaced: `ScryBurrow.repay(max)` stranded 1 wei
  of unpayable dust debt (share floor); full repay now zeroes shares.
- **Both mandatory fork runs PASSED against live RH-Chain state
  2026-07-23**: `OrchardFork.t.sol` (the real-NPM round-trip) and the new
  `SeedSpoilsFork.t.sol` (the real `runWith` flow through the real
  factory+NPM: pool created + initialized at exactly the passed sqrtP,
  LP NFT to the recipient; plus a dedicated test pinning the hardcoded
  DEFAULT_* literals against live chain — closing the "zero tests
  exercise the defaults" hole above). Offline both SKIP-pass.
- Still standing from this audit: the money-core test additions listed
  per track. Two items age out: the Gardener two-staker split and the
  production-scale carry conservation now RUN and PASS
  (`test_two_stakers_one_pool_exact_proportional_split`,
  `test_production_rps_per_second_updates_lose_nothing_carry_conserves`).

## ADDENDUM 2026-07-23 (b) — the last two standing items CLOSED

- **The Orchard price-movement-in-range manual step is now a fork test.**
  `OrchardFork.t.sol::test_fork_thin_range_clock_stops_out_of_range` stakes
  a ONE-TICK-SPACING position and drives the live pool's tick across that
  range's bounds with real direct `pool.swap`s (no SwapRouter dep), proving
  the exact clock the Orchard reads — `snapshotCumulativesInside`'s
  `secondsPerLiquidityInsideX128` — GROWS in range, FREEZES byte-for-byte
  across an out-of-range warp, and RESUMES on re-entry, with the credited
  reward reflecting only in-range seconds and bounded by the pot. PASSED
  against live RH-Chain 2026-07-23. "Out-of-range liquidity earns nothing"
  is now proven against real bytecode, not asserted. **One fork gotcha this
  surfaced, worth reusing:** forge sets a fork's `block.number` to the RPC's
  height, which is BELOW SCRY's L1-denominated `restrictionEndBlock`
  (25,539,589) — so the token's long-expired launch max-wallet cap springs
  back to life and reverts large real-swap payouts to a non-whitelisted
  address. The test `vm.roll`s past it to match live reality; any future
  fork test that receives a large SCRY transfer must do the same.
- **The zero-fuzz note now has coverage on the two load-bearing money
  laws.** `testFuzz_emission_never_exceeds_pot` (Orchard) fuzzes two
  stakers' seconds-in-range + unstake timing and asserts the pot is never
  over-emitted and is conserved to the wei; `testFuzz_supply_conservation_
  and_cap` (SpoilsToken) fuzzes a mint/burn sequence and asserts
  `supply == minted - burned` and the cap holds after every op. Both 256
  runs, deterministic. The broader repo-wide fuzz/invariant gap is narrowed,
  not closed — the money core (Bank/JobBoard) and the Gardener carry are the
  next candidates.

Gate totals as of this addendum: **325/325 offline PASS, deterministic;
fork mandates 4/4 PASS against live RH-Chain.**

## ADDENDUM 2026-07-23 (c) — the per-track minimum additions CLOSED; the gate made real

A three-track re-audit against the ACTUAL current test files (not this doc)
found the money-core and collectibles tracks still short of testnet-redundant
on exactly the governance/knob + metadata/safe-transfer surface a testnet's
real value catches. All gaps now closed (pure-unit, no fork):

- **Money core** (`ScryMarket.t.sol` + `ScryEconomy.t.sol`): `FeeSplitter.rescue`
  — the untested operator drain that bypasses the split — pinned (auth, both
  zero-guards, partial + the `amount==0` full-drain, split bypass);
  `FeeSplitter.distribute` all-or-nothing when a recipient's receipt fails
  (atomic — the earlier good cut rolls back); `transferOperator` on
  Reputation/InsurancePool/FeeSplitter/Harvest; `Reputation.setThreshold`
  effect+auth; `InsurancePool.setMaxPayout` effect+auth; non-operator reverts
  for `setAuthority`/`setClaimant`; `Harvest.sweep` (never below the posted
  root) +auth; and the `dispute→close` double-settle the suite had only in
  reverse.
- **Collectibles**: `ScrySteleEdition` tokenURI is now DECODED (a real base64
  decoder + `vm.parseJson` + field asserts) — the "malformed JSON ships green"
  prefix-only theater is gone; safe-transfer receiver accept+reject for BOTH
  `ScryEidolon` and `ScrySteleEdition`.

**The gate itself was hardened** (`deploy_town.sh`), closing two silent
footguns that would make a green gate meaningless: (1) `need_upstream_forge`
fails closed if PATH resolves the foundry-zksync fork (breaks via_ir/vm.warp
AND would broadcast zkSync bytecode, not EVM); (2) `fork_mandates_green` runs
the fork suite against live RH-Chain and PROVES it executed — a plain offline
`forge test` SKIP-passes the very tests that replace the testnet. Verified
end-to-end: `./deploy_town.sh test` → guard → 348/348 → selftest → fork 4/4 live.

Gate totals: **348/348 offline PASS, deterministic; fork mandates 4/4 PASS
against live RH-Chain, now enforced by the arm gate.** Per-track verdict:
**Registry, Spoils-seeding, Farm, Money-core, and Collectibles are all
testnet-redundant on coverage.**

## ADDENDUM 2026-07-24 — Orchard holder path and post-season accounting

- **Holder deposit path added and tested.** The preferred route is now
  `NPM.approve(orchard, tokenId)` then `ScryOrchard.depositToken(tokenId)`,
  then `stakeToken`. The Orchard calls the canonical NPM's
  `safeTransferFrom` itself and verifies its receiver hook recorded the same
  holder. `test_holderDepositRequiresNpmApprovalAndRecordsOwner` pins it.
  The public Orchard page is config-gated and prepares only these
  holder-signed calls; it stays dark with no verified deployment config.
- **Post-season denominator bug fixed.** The previous denominator grew with
  wall time after `endTime`, so a late exit could receive less for identical
  earned in-range seconds. It now uses the fixed posted window
  `endTime - startTime`; `test_late_unstake_uses_the_posted_window_not_extra_wall_time`
  proves a full-window position remains entitled to the full pot after a
  30-day delayed exit.
- **The bare-transfer trap remains a release blocker to communicate, not a
  hidden rescue power.** A manual ERC-721 `transferFrom` still bypasses the
  receiver record and strands the NFT. The UI warns against it; no
  owner-recovery authority was silently introduced. Decide explicitly whether
  that disclosure plus safe UI is acceptable before mainnet seasons.
- **Current local evidence:** 364/364 contract tests and 650/650 static-site
  checks pass. The prior 4/4 RH-Chain fork run predates these changes; rerun
  the enforced upstream-Foundry fork gate immediately before any arm.

## ADDENDUM 2026-07-25 — the admin-rotation audit

All 27 contracts were read for one question the suites did not ask: **does
every privileged role have a way to move?** Table XI of the creed
(`HELD-KEYS.md`) is a claim about bytecode, not intent — an admin that cannot
be rotated is a permanently held key the moment it broadcasts, whatever the
docs say.

- **Fifteen contracts already rotate** — `transferOwnership` (VowRegistry,
  Arbiter, Eidolon, Deed, SteleEdition), `transferOwner` (Gardener, Orchard,
  Silo, Shrine), `transferOperator` (FeeSplitter, Harvest, Reputation,
  InsurancePool), `transferSteward` + `rotateMinterAway` (Granary), `setMinter`
  (SpoilsToken), plus the VowRegistry's self-rotating `oracle`.
- **`ScryJobBoard` was the one gap.** Its `operator` was set in the constructor
  and could never move, while governing `setFeeBps`, `setRepRewards`,
  `setArbiter`, `setFeeSplitter`. Broadcasting as-was would have welded the fee
  and arbiter controls to the deploying key — a retired or compromised arbiter
  could never be replaced. `transferOperator` added in the sibling shape
  (zero-check + `OperatorTransferred`), with
  `testTransferOperatorHandsOverTheKnobs` covering stranger-refused /
  zero-refused / old-operator-loses-it / new-operator-works.
- **Seven contracts have no privileged role at all** and need nothing:
  `ScryNotary`, `ScryCovenant`, `ScryPact`, `ScryBank`, `ScryGarden`,
  `ScryBurrow`, `PlayToken`.
- **`ScryPeculium` is exempt by design.** Its signer moves only through the
  verifier quorum; an admin door there would break the keyless claim outright.
- **A renounce is not automatically the freer move.** On the params-only roles
  a dead admin ENDS a live path (a dead job-board operator freezes the fee and
  makes `setArbiter` unusable) — Table XII read backwards. Rotate to a quorum
  first; void only what is proven. `ScryGranary` is the one where a dead
  steward is genuinely attractive, and `rotateMinterAway` already exists for it.

**Current local evidence: 365/365** (upstream foundry v1.7.1, the +1 being the
rotation test) and 661/661 static-site checks. **Scope note on the fork
mandates:** this change touches neither the farm track (`deploy_town.sh`
deploys spoils → gardener → silo → shrine → orchard; `ScryJobBoard` is not in
it) nor anything `OrchardFork`/`SeedSpoilsFork` exercise — `ScryJobBoard`
broadcasts via `DeployScryMarket.s.sol`, after `ScryArbiter`. The rerun
requirement above stands on the **Orchard `depositToken`/denominator** changes,
not this one; the arm gate enforces it either way.

---

## ADDENDUM 2026-07-26 — the seeding standing note, closed

This file is `status: record`; this appends rather than edits.

§Standing notes said, of the sqrtP tripwire added in Round 2:

> the tripwire checks env-vs-env only — a rerun against a pre-existing pool
> still never reads the live price

That was accurate and is now closed. `_seedOne` reads the existing pool's
`slot0()` and refuses by name past `slipBps` — **before any approval is
granted** — because `createAndInitializePoolIfNecessary` is a no-op against a
pool that already exists, so a re-run mints at *that* pool's price rather than
the one the tripwire just validated. Two ways that happens and both are real:
the launch's top-up after the 0.1% canary is a deliberate second seed into the
pool the canary just opened (`RUNBOOK.md` §0b2), and a squatter can open the
pair first.

**Read via `staticcall`, not a typed call, and this is the part worth keeping.**
An address that cannot answer `slot0()` must not turn a good run into a revert
with no data — so the check is skipped **loudly** (the script says the live
check did not run and that the amount mins are the only guard left) rather than
either crashing or passing silently. The mins were always the hard backstop;
what was missing was a readable diagnosis instead of an opaque revert from
inside an NPM multicall.

Coverage, in `SeedSpoilsUniswapV3.t.sol` (13 → 16, plus a `MockPool`):

| law | what it pins |
|---|---|
| `..._divergent_live_price_is_refused_by_name` | refused, and `mintCount == 0` **and** allowance still 0 — nothing was approved |
| `..._no_readable_slot0_still_fails_closed_on_the_mins` | the coverage the old test carried, kept rather than replaced |
| `..._at_the_posted_price_tops_up` | the re-run that is *supposed* to work still does |
| `..._uninitialized_pool_is_not_a_price_disagreement` | `slot0().sqrtPriceX96 == 0` is created-but-uninitialized, not a disagreement — refusing it would block a good first seed |

Also closed from this file's §headline table: **"Money core … Harvest — NO"** now
has `DeployHarvest`/`DeployClaim` executed by tests (`DeployClaimHarvest.t.sol`,
15 tests) and `ScryHarvest` under invariants (`Invariants.t.sol`) — money
conservation, `totalClaimed` honesty, and `owed <= posted`. The mode gaps that
row also named (RepOnly, pool cap/auth) are unchanged; this closes the Harvest
part only.

Gate: **448/448 offline · 4/4 live-fork.** Counts in the body of this file are
from 2026-07-22 and should be read as history, per its own front-matter.

---

## ADDENDUM 2026-07-26 (b) — a third fork mandate: the gacha against a real collection

**Gate: 487/487 offline · 8/8 live-fork.** Both counts moved for one reason —
`ScryGacha` landed with 36 offline tests and a fork harness, and the fork harness
was **wired into the arm gate** rather than left beside it.

**The wiring was the actual finding.** `deploy_town.sh::fork_mandates_green`
hardcoded `--match-contract 'OrchardFork|SeedSpoilsFork'` and a four-name
must-have list, so a new fork suite could pass forever without the broadcast gate
ever running it — a mandate outside the gate proves nothing on broadcast day. The
filter now includes `ScryGachaFork`, the list carries its four tests, and **the
printed count is derived from the list** (`${#mandates[@]}`) instead of the
literal `(4/4)` that would have kept saying 4 while the list grew. Verified end
to end: `./deploy_town.sh test` → 487/487 → seed selftest → *"fork mandates
executed live (8/8)"*.

**What the gacha mandate covers that no mock can.** It runs deposits, a paid
draw, a settlement and both resolution branches against **StonkBrokers'** real
deployed bytecode (`0x539CdD042c2f3d93EbC5BE7DfFf0c79F3B4fAbF0`) and **real
holders discovered by walking `ownerOf`** — never a pinned owner, which would rot
the day that token trades. Three things kill this contract and only live bytecode
shows them: an operator-filter registry that refuses a pool address, a pause
hook, and a `transferFrom` that returns without moving anything. StonkBrokers has
none of the three (measured: `supportsInterface(0x80ac58cd)` = 1, no filter
registry constant, no `paused()`).

**What a fork mandate CANNOT cover, recorded so a green line is not over-read:**
inside a fork the EVM is foundry's, so `BLOCKHASH` there is emulation, not chain
4663. That question — the one the whole draw rests on — was settled separately
and keylessly against the live node: a 13-byte probe run through `eth_call` with
a state override, each returned word resolved with `eth_getBlockByHash`. Every
value came back a **canonical** block hash, and the window is **exactly 256**
(≤256 returns a hash, ≥257 returns zero), which is what `BLOCKHASH_WINDOW`
assumes. Do not read the *offsets* from that probe: the state-override path is
served by backends at differing heights. Plain `eth_blockNumber` is tight — 12
reads spanned 6 blocks, monotonic — so a keeper watching for a reveal block reads
a fresh head, and the ~26s settle deadline is real rather than theoretical.

**And it turned the gacha's own cost-per-pull from an estimate into a number:** a whole pull
(request + settle + keep) is **580,523 gas**, a deposit **872,903**. At the
0.0479 gwei base fee measured the same day a pull costs **≈$0.055** — the earlier
estimate had the gas low and the fee 10× high, and what retired the concern was a
fee level, not a design change.

---

## ADDENDUM 2026-07-26 (c) — the clock was wrong, and the suite was green anyway

**Gate unchanged at 487/487 offline · 8/8 live-fork, and that is the finding.**
`ScryGacha` shipped reading Solidity's `block.number` and `blockhash`. **4663 is
Arbitrum Nitro, where `block.number` is the PARENT chain's** — measured the same
day at 25,620,709 against an L2 height of 20,319,770, advancing **0.08 blocks/s**
against the L2's 9.9. So a "100 block, ~10 second" reveal was really **~20
minutes**, the pool stayed locked for all of it, and the throughput argument that
justifies one-draw-at-a-time over FWA's sequencer — ~8,600 draws/day/pool — was
really ~72. **Thirty-six unit tests and four live-fork mandates passed
throughout.**

**Why no test could have caught it.** Every test drove the clock with `vm.roll`,
so `block.number` advanced one-per-block *in the test* exactly as the contract
assumed. The wrong assumption was about the CHAIN, and a mock cannot disagree
with the thing it is standing in for. A fork run cannot catch it either: foundry
implements no ArbOS precompile, and its EVM advances `block.number` the Ethereum
way. **The defect was only reachable by measuring the live node**, which is the
generalisable lesson — for anything whose correctness rests on a chain's own
semantics, the test suite is not the instrument.

**What found it:** the operator asking whether being on top of Arbitrum mattered.
It did. `deploy_town.sh` had said "RH-Chain (Arbitrum L2)" the whole time.

**The fix, and the shape worth reusing.** The contract now reads `ArbSys` at
`0x64`: `arbBlockNumber()` for the height, `arbBlockHash(n)` for the seed — the
**exact** L2 block hash, verified against `eth_getBlockByHash` at head-1, -2, -100.
`arbBlockHash` **reverts** outside its window rather than returning zero, so
`settle` and `expire` both branch on the call's own success instead of comparing
against a hardcoded 256. That matters: the window measures **253-254** against a
documented 256 (drift between reading a head and the call landing), and a
hardcoded count would have been wrong at the boundary in whichever direction the
node actually enforces.

**Two testing traps this leaves behind, both now in `test/MockArbSys.sol`:**

1. **Foundry implements no ArbOS precompile.** On a fork `0x64` holds one byte of
   non-executable code, so the contract's clock is simply dead there. The mock is
   etched into the unit AND fork suites — which means the gacha fork mandate
   proves the *collection's* bytecode and never the clock.
2. **`vm.etch` copies runtime code and nothing else** — no constructor runs, no
   storage travels with it. The mock's `window = 256` field initializer therefore
   never executed at the etched address; window read 0, every reveal read as
   expired, and **the expiry test passed for the wrong reason.** Set such fields
   explicitly after etching.

Gas moved slightly with the precompile calls: a whole pull 580,523 → **588,738**.
