---
status: live
lane: [launch, economy]
updated: 2026-07-27
about: "the contract deploy phases in order, with the numbers each one carries"
---
# Contract phases — the DFK-shaped SCRY loop

> ## ⚠ A `Phase N` here is NOT a phase of `LAUNCH.md` (settled 2026-07-27)
>
> These are **epochs of the whole build-out**, over months. `LAUNCH.md`'s phases
> are **steps of one script run**, in one sitting, and they are identified by
> `deploy_town.sh` subcommand name — `spoils`, `pools`, `rotate` — not by number.
> The collision that keeps restarting the same conversation is **4**: here it is
> *optional sinks and seasons*; there it is `pools`, seeding the v3 pools, which
> is the keystone. Same digit, opposite ends of the project.
>
> **The one real relation:** Phase 1 below is what a completed `LAUNCH.md` run
> delivers. Nothing else on the two lists lines up, and none of it should be
> made to.
>
> Where this page's *sequence* disagrees with `LAUNCH.md` about what ships
> first, `LAUNCH.md` is the later document and wins; this one carries the
> parameters and the phase definitions. **Phase 1's pool sizes are `LAUNCH.md`'s
> to state** — they were revised 40M/40M → **60M/20M** on 2026-07-26
> (`LAUNCH-DECISIONS.md`), and this page had the old pair until 2026-07-27.
> Derive them there, never from here.

> A public sequence, not a promise of yield or a deployment authorization.
> A phase is **written** until its addresses, funding transactions, and
> read-back checks are published on RH-Chain. No agent holds a user key or
> broadcasts for a user.

## The token roles

| role | scry asset | job |
|---|---|---|
| base / JEWEL | SCRY | public market pair, service currency, Bank stake |
| game coin / Gold | OBOL | earned through play and spent in town sinks |
| farm crop / power token | **MYRRH** — and the Garden is its **only** source (operator 2026-07-26, reversing the 07-25 flip to OBOL; this row still said OBOL until 2026-07-27) | earned from participation and LP farming; **never** SCRY-farmed. Play mints no MYRRH at all now, which is what made farming it safe again — `FARMING.md` §3a |
| Bank receipt | xSCRY | minted only when SCRY is staked in ScryBank |
| planted position | SEED or a v3 position NFT | evidence of supplied liquidity, never a separate public sale |

## Phase 0 — live foundation

**Everything on-chain today, exhaustively — three things:**

| asset | address | what it is |
|---|---|---|
| `SCRY` | `0xDa2a4b23459e9ca88183e990802be644AcA7C4B0` | the token + its existing SCRY/WETH market. External, pre-existing; this roadmap does not create another SCRY pool |
| `ScryVowRegistry` | `0x08131e7660639bbd086dffa9375c2a563f1d3590` | soulbound vows + daily merkle roots |
| `ScryNotary` | `0x0C15fA7829458118e3d26229F58FE0443f8b792c` | permissionless inscription/first-seen (broadcast 2026-07-25) |

- No protocol farm, Bank, spoils-token, or reward contract is live merely
  because its source and tests exist.
- **State it plainly, because the website does not:** OBOL, MYRRH and the
  whole spoils economy — barrow, agora, roads, tabernae, augury, arena, duels,
  table — are a **file ledger** (`meter/tokens.py`), not chain state.
  `SpoilsToken.sol` is written, tested and **never deployed**. A player's
  balance today is a database row: it cannot be held in a wallet, traded,
  seen in an explorer, or pooled. That is the single biggest gap between what
  the site shows and what the chain holds.

## Phase 1 — spoils and public markets (**the keystone; everything waits on it**)

Nothing else in this file can proceed until the coins exist on-chain — a pool
cannot be seeded with a token that has no address, and a farm cannot emit one.

1. **Deploy OBOL and MYRRH** — distributor-gated mint, OBOL elastic
   (`cap = 0`), MYRRH capped 21,000,000 (welded, 2026-07-27); the one posted
   allocation is the `POWDER_TO` powder; publish both addresses.
   `DeploySpoils.s.sol`; `forge test` green (derive the count by running it).
2. **The ledger↔chain cash-out — decided by construction, one sentence short
   of closed** (aligned 2026-07-28; this step still offered a three-shape menu
   long after the machinery had picked one). What is built and law-backed is
   shape (b): **the ledger stays the system of record**, and a player exits
   through `POST /tokens/withdraw`, which *burns* the ledger balance and feeds
   the cumulative root `GET /tokens/claim-root` emits; `ScryHarvest` pays
   `cumulative − claimed` on-chain, roots republishable, nobody forfeits by
   being slow. `ONCHAIN-LINE.md` §what the ledger IS (status: law) forecloses
   the cut-over shape — play cannot demand a signed transaction per action —
   and the snapshot shape has nothing to snapshot: the game ledger has never
   been written.

   **Two claim machines share this mechanic and must never share a contract**
   (`LAUNCH.md` §4 carries the failure table): the **holder drops** — classic
   airdrop claims, per-drop *absolute* amounts, a fresh `DeployClaim` pair per
   drop — and the **standing play cash-out** — *cumulative*, one harvest,
   forever. A delve touches the drops in exactly one place: it is one of the
   ten trials that unlocks **drop two's** claim (activation gate; drop one is
   stealth and bar-gated only). Naming note: where older docs say "the
   bridge," they mean this cash-out path — never anything cross-chain;
   `CLAUDE.md`'s "no bridge" is about other chains. Prefer **cash-out** in
   anything player-facing.

   **Still open, each one operator sentence:** the ledger-as-record shape has
   never been *spoken* (built ≠ decided); the path is **exit-only** — on-chain
   coin cannot re-enter the ledger, so a cashed-out player cannot spend in
   town until they earn again (wants a deliberate yes); root-posting cadence;
   and whether the house auto-submits claims — `claim()` is callable by
   anyone *for* anyone by design, so gasless push-style settlement is a
   keeper decision, not a contract change.
3. **Seed the canonical Uniswap v3 pools** at the 1% tier, protocol-owned and
   disclosed as such — the settled sizes (`LAUNCH-DECISIONS.md`,
   `TREASURY.md`, operator 2026-07-24):

   | pool | SCRY side | game-token side | opening ratio |
   |---|---|---|---|
   | OBOL/SCRY | **76,500,000** | **7,650,000 OBOL** | 10 SCRY / OBOL |
   | MYRRH/SCRY | **23,500,000** | **470,000 MYRRH** | 50 SCRY / MYRRH |
   | MYRRH/OBOL | — (**costs no SCRY**) | both sides house-minted, 5:1 | consistent with 50÷10, so no genesis arb; never an oracle |

   > **Deepened 80M → 100M (operator, 2026-07-28)**, out of *operator,
   > unallocated* (50M → 30M), so the 180M total is unchanged. Every opening
   > ratio is unchanged — this is depth, not a re-price. The split is 76.5/23.5
   > rather than a strict 60/20 scaling because 100,000 new MYRRH would drop
   > lifetime headroom under the 21M cap to 974,469, below the 1,000,000 floor
   > `meter/test_tokenomics.py` law 8b asserts; weighting toward OBOL buys the
   > full 20M for 70,000 MYRRH and leaves 1,004,469. **Derive these from
   > `LAUNCH-DECISIONS.md`, never from here** — `preflight` reads that table
   > against the shipped script.
   >
   > *Prior: 60/20, revised from 40/40 (operator, 2026-07-26).* Every season
   > emission is OBOL, so all farm sell pressure lands on OBOL/`SCRY`;
   > circulating MYRRH is **0**; and the MYRRH/OBOL row already gives MYRRH a
   > free market. The *ratio* was held, not the token count. No code change: `seed_spoils_uniswap.py` takes
   > `--myrrh-scry-budget` as an argument. `SEASONS.md` §1d carries the reasoning.

   Scripts: `SeedSpoilsUniswapV3.s.sol` + `seed_spoils_uniswap.py`.
4. Record the positions, prices, transaction hashes, and minter rotation.

This phase creates real markets but **no farm promise**. SCRY/WETH remains a
separate existing market; it is not replaced or migrated.

*Corrected 2026-07-25: this phase previously read "50M SCRY + 5M OBOL and 50M
SCRY + 1M MYRRH," which was superseded by the purse allocation — the pools
came down to 40M each and the freed 20M went to the Great Work, on the finding
that pool size is not a game-balance parameter.*

## Phase 2 — the DFK loop and fee spine

Two separate, compatible lanes:

- **Town Garden + Gardener:** the DFK-shaped loop — the one MYRRH/OBOL
  `ScryGarden` issues fungible SEED shares; holders stake them in ScryGardener
  for **MYRRH** (era-0 6,000/day on the 4-year halving schedule since 2026-07-28,
  and the Garden is MYRRH's only source since
  2026-07-26 — `FARMING.md` §3a) under the posted lock and exit-slash schedule.
  The Silo and the Orchard still pay OBOL, from a second granary. It is not
  connected to ScryBurrow or used as an oracle.
- **ScryBank + ScryFeeSplitter:** every SCRY fee the town takes — service fees,
  and the SCRY-denominated game entries and rakes since *"scry can enter"*
  (2026-07-25) — distributes through one published split: the **burn**
  (`0xdEaD`), the Bank, a public campaign/prize wallet, and founder/operations.
  xSCRY is a receipt, not a separately sold token. The Bank has no owner, no
  pause, no lock and no emission schedule, so **no rate of return is quoted on
  any surface** — its income is event-driven, not a rate.
  Driver: `./deploy_town.sh bank [--arm]` (one phase, both contracts, needs
  nothing from any other phase). **The posted split is burn 5000 / bank 4000 /
  prizes 0 / ops 1000** (2026-07-27, `SENTENCES.md`, closing `FEES.md` §9 #2)
  — prizes take no line at open, because that cut needs an escrow wallet and the
  purse already funds prizes directly. The phase derives the four numbers from
  `DeployScryEconomy.s.sol` rather than keeping a copy; an override is printed
  as *posted X → you set Y*.

Both lanes require their own pre-broadcast review, funding decisions, and
address read-back. The split must be published before receiving fees.

> ⚠ **The Bank opens empty, and seeding it is a rollout error, not a kindness.**
> While `totalSupply() == 0`, SCRY sitting in the Bank belongs entirely to
> whoever deposits first — `enter()` prices the first deposit against itself, so
> a 1 SCRY deposit redeems the whole balance and `MINIMUM_SHARES` (1000 *wei* of
> shares) defends nothing. `TREASURY.md` §P9's 9M trickle cannot begin until a real
> stake exists; `./deploy_town.sh status` reports the Bank NOT SEEDABLE until it
> does, and `GET /bank` carries the same gate as a `seedable` boolean.

## Phase 3 — real v3 LP reward seasons

ScryOrchard rewards **specific canonical Uniswap v3 pools**, including the
existing SCRY/WETH pool or new SCRY/OBOL and SCRY/MYRRH pools. A season:

1. names one pool, a start/end time, and a finite OBOL pot;
2. moves that pot into the contract before the season begins;
3. pays a position by liquidity × seconds-in-range, not by a price or score;
4. starts with no retroactive entitlement and no quoted APY.

Before this phase, the Orchard's NFT-deposit UX and contract must be hardened
and independently reviewed. The current holder route is
`approve(tokenId) → depositToken(tokenId) → stakeToken(incentiveId)`;
`depositToken` hard-codes the recipient and proves the NPM receiver record
matches the caller. Reward accounting does **not** use the frozen `[start, end]` window: past
`endTime` the denominator keeps growing with wall time (`clockEnd =
max(block.timestamp, endTime)`), so an out-of-range position's share decays
the longer it stays staked after a season closes. That is the deliberate
anti-camper rule from 2026-07-27; this line asserted the reverted design
until 2026-07-28. It is a payout term a staker can lose money to, so the UI
must say it — `watchtower/orchard.html` now does. A UI must remain dark without a verified
deployment config, use that holder flow (not a manual bare `transferFrom`),
and make the range, in-range state, season pot, deadline, and custody
consequences plain. A bare `transferFrom` still records no deposit because it
bypasses the receiver hook — but the NFT is no longer lost: the owner-only
`rescueOrphan` (operator, 2026-07-25) returns it, and it structurally cannot
reach any properly deposited position (`deposits[id].owner` must be zero).

## Phase 4 — optional sinks and seasons

- **Silo:** time-lock OBOL/MYRRH for a posted OBOL drip; early exit burns
  part of principal.
- **Shrine:** permanent OBOL/MYRRH burn for cultural rank and public record;
  never reputation, access, odds, price, or payout.
- **Harvest:** explicit, funded Merkle campaigns for community distributions.

## Held outside this sequence

- **The gacha** (`ScryGacha.sol` + `DeployGacha.s.sol`) deploys on its own,
  independent of every phase above: it is native-ETH between depositors and
  buyers, touches no game coin, and its `SCRY` toll rail ships in the
  contract **disarmed** (`tollRate`/`tollBurnBps` = 0). First collection is
  decided (StonkBrokers — `NOW.md` §2, `GACHA.md` §8a); the deploy is the
  one act left, and it has no `deploy_town.sh` subcommand — `forge script
  script/DeployGacha.s.sol` is the driver.
- The Burrow is the Nexum's contract (PvP lending, designed and never deployed); the pGOLD/pTEARS playground it was first written for was retired 2026-07-26. Formerly an optional zero-stakes practice
  world, not a dependency of the real market loop.
- ScryPeculium remains custody-off until its enclave, attestation, recovery,
  deployment operations, and independent audit are complete.

## Website vs chain — the settled read (2026-07-25)

The site is **ahead of the chain nearly everywhere**, on purpose, and that is
fine as long as each page says which it is. Audited page by page:

| surface | backed by | honest today? |
|---|---|---|
| `barrow` · `agora` · `roads` · `tabernae` · `augury` · `arena` · `duels` · `table` · `tokens` | **ledger only** (`meter/tokens.py`) | playable and real as a *game*; nothing is on-chain. Say so — a coin you cannot withdraw is a score |
| `registry` · `directory` · `verify` · `read` | ScryVowRegistry + the meter | ✅ fully live |
| `tools` | ERC-8257 registry on RH-Chain | ✅ live once the branch deploys |
| `claim` | claim contracts | disarmed teaser, correctly labelled |
| `gardens` · `orchard` | ScryGranary/Gardener/Orchard | **dark by config** until addresses post — correct behavior |
| `eidolon` | ScryEidolon | SEAL/coming-soon until the mint, held with adoption |
| `pools` | the v3 pools | dark until Phase 1 seeds them |
| `floor` | the agency | `HOSTED_ADOPTION_OPEN = false`, held with the mint |
| `town` · `streets` | ScryDeed + the ledger | walkable; deeds are ledger records until `ScryDeed` broadcasts |

**The rule this settles on:** a page may ship before its contract, but it must
show the seal rather than imply chain state. The dark-by-config pattern
(`gardens.js`, `orchard.js`, `eidolon.js`) is the correct one and should be
copied by anything new. The gap to close is the *unlabelled* case — the spoils
pages read like an on-chain economy and are not one.

**For an RH-Chain audience specifically:** what a memecoiner can hold, trade or
verify on-chain today is `SCRY` and two registry contracts. Everything that
looks like a token economy is off-chain. Phase 1 is the whole answer to that,
and it is one deploy plus one bridge decision away.

## UI and agent boundary

The UI may discover positions, estimate outcomes, explain risk, and prepare
calldata. An agent may do the same through read-only tools. The wallet owner
reviews, signs, and broadcasts every liquidity, staking, or NFT-custody move.
No agent receives a private key or an open-ended allowance.
