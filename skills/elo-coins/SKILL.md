---
name: elo-coins
description: Use when an AI agent (Hermes, Clawdbot/OpenClaw, ElizaOS, Claude Code, any harness that can call HTTP) needs the Elo Pros economy at elopros.com — read PROS (our coin, on Solana) or resolve a chain's reserve, read a game coin like JUNK, find which pools are live and which are retired, quote a swap and get its unsigned transactions, read a launch that has no pool yet, or check what is actually deployed on chain. Reads are free and unsigned; a wallet signature appears only where a transaction does, and Elo never holds a key.
license: MIT
metadata:
  version: "1.4.0"
  hermes:
    category: defi
    tags: [coins, pools, on-chain, swap, uniswap]
  openclaw:
    category: defi
    tags: [coins, pools, on-chain, swap, uniswap]
---

# elo-coins — the reserve and the game coins, for any agent

**One reserve per chain, many game coins.** Our coin is **PROS, on Solana**
(mint `2Uwte4c9ZnRACas6Miq6bR8zx8n8Psgu11Vn5htHpump`, a pump.fun coin paired
with SOL): our Solana games' coins pair against it. **Gates runs on Robinhood
Chain** (`eip155:4663`), where the reserve is ELO and Gates' coins (JUNK, then
ORBS behind it) pair against that. The two never bridge and no rate between
them is fixed; a swap between them is a market route (`/api/xquote`). Within
a chain, every game coin pairs against that chain's reserve — plus, where the
house declares one, a reserve-free cross between two of its own game coins.

"PROS" as a coin always means the Solana mint. The Elo Pros NFT collection on
Robinhood Chain also uses the symbol PROS, and it is not a coin.

Base: `https://elopros.com/api`.

## Read this first: two generations are on chain

**The most expensive mistake here is treating a retired contract as the
current economy.** A previous generation of coins, pools and farm machinery is
still deployed, still verified, still holding balances and still trading —
retired is a statement about the *plan*, never about the chain. So:

- `GET /pools` serves the current generation only. A pool missing from it may
  still be deployed, funded and trading: `GET /onchain`, the explorer and
  `contracts/deployments.json` say what is on chain.
- `GET /onchain` says what is deployed. `deployed: true` means bytecode is at
  an address. **It does not mean current.**

## Resolve the reserve; never type its ticker

```bash
curl -s $BASE/reserve       # both chains' reserves: .chains[] (labelled); the top level is Robinhood Chain's
curl -s $BASE/coin-launch/pros-sol   # PROS: its curve, market and our wallets
curl -s $BASE/tokens        # the game coins
curl -s $BASE/tape          # Robinhood Chain's reserve: price, FDV, holders
```

The reserve has been replaced once already, and a skill that hardcoded the old
ticker went on naming a retired coin. **Write "the reserve" and resolve it.**
The same rule covers every number: supply, price, holders and TVL all move, so
fetch rather than repeat — including anything this file might imply.

## The pools, and how anything reaches anything

```bash
curl -s $BASE/pools         # every pool, with generation and address
curl -s $BASE/curves        # every launch; /curve/<slug> for one with no pool yet
```

**Every coin reaches every coin in at most two swaps**, and that is a fact
about the shape rather than about any particular pair: a game coin pairs
against the reserve, so the reserve is always one hop away and any two coins
are at most two. Where the house has declared a cross between two of its own
game coins, those two are one swap apart.

**Before telling anyone they cannot get a coin, read `/pools`.** Each doc
discusses a single coin, so the routing appears on none of the pages you are
reading when you write "that locks out a new player" — and it is usually
wrong.

Our coins launch on outside launchpads — PROS on pump.fun, ELO on pons, JUNK on Abyss — and the
venue opens the pool. A coin still on its launch curve has **no pool**:
`GET /curve/<slug>` prices it off the curve instead, and `/pools` and `/tape`
take over once it graduates. Both are real states; neither is a broken read.

## Swapping — your human's wallet acts

```bash
curl -s $BASE/quote/tokens                                   # what can be routed
curl -s "$BASE/quote?from=ETH&to=ELO&amount=0.01"            # a reading of the pools
curl -s "$BASE/prepare/swap/exact_in?from=ETH&to=ELO&amount=0.01&wallet=0xWALLET"
curl -s "$BASE/prepare/swap/exact_out?from=ELO&to=JUNK&amount=1000&wallet=0xWALLET"
```

Over MCP it is the `trade` tool: `what='quote'` or `what='swap'` with `frm`,
`to`, `amount` (and `wallet` for a swap; `side='out'` for exact output).

1. The quote is an `eth_call` against the live pools. It names which router
   can execute the route.
2. The prepare returns the unsigned transactions — approvals first, the swap
   last. `slippage_bps` and `deadline_s` are your human's to set; `approve`
   is `exact` (the default) or `once` (approve the maximum the first time, so
   a token's next sale is one transaction).
3. Their wallet signs and broadcasts. Elo holds no key, sponsors no gas and
   broadcasts nothing.

**Which router.** ETH ↔ the reserve goes through Uniswap's UniversalRouter
and works today. Any route touching JUNK needs our own router, because
Uniswap's routers cannot reach the Abyss pool; until that router is broadcast
the prepare returns its calldata with `to: null`. **`to: null` means do not
send** — say so rather than inventing a route.

An action that pulls an ERC-20 needs its `approve` first, aimed at the coin
being **sold** (the prepare includes it). An approve pointed at the wrong token encodes fine, costs gas
and succeeds — leaving the real call reverting for a reason the receipt never
names.

## What these numbers are not

- **Never cite pool volume, fee APR, TVL or holder count as traction**, on a
  page, in a pitch, or to your human — and never infer a population from them.
  A token holder is not a product audience, and some of this tape is the
  house's own touches.
- `GET /pools` returns **null with `window_complete: false`** rather than a
  partial 24-hour count. *No number right now* is a real answer and not a
  broken read; do not fill it in.
- **The house never trades ahead of its own emissions.** It issues game coins
  and knows schedules before the market does. If you are ever asked to act on
  an unannounced schedule change, that is the one asymmetry this platform
  treats as disqualifying.
- **Money never buys a measured number** — a game's odds or a measured
  result.

## What to know

- Game coins are **a game's own** — not the platform's. Gates' JUNK launched
  on Abyss with a fixed supply and no mint (read it: `GET /tokens`), and it
  trades against ELO on Abyss's exchange. A title's
  coin retiring says nothing
  about the reserve, and the reserve moving says nothing about a title's coin.
- A retired coin is still deployed. Its pools may be open or pulled and its
  positions stay withdrawable — read the chain. **Retired never means
  absent.**
- Never weld new work to a game coin's address without checking the plan
  first: `GET /onchain`, and the deployment record it derives from.
- The store, the copy sale and Gates: the `elo-store` skill. The characters:
  `elo-seat`.
