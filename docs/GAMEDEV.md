---
status: live
lane: [platform]
updated: 2026-08-07
about: "everything a game needs from scry, in one page — sign a player in with standard Sign-In With Ethereum, draw their name and face, read what they own and what they have done, and the honest-zero traps that make an empty answer mean two different things"
---

# GAMEDEV.md — everything a game needs from scry

> Operator, 2026-08-07: *"make the SDK just like steams so its easy for games to
> auth and get profile info"* · *"whatever other api end points a game dev might
> need for players"*

You are building a game and you want to know who is playing, what they own, and
what they have done. That is this page. It assumes nothing about scry and
nothing about crypto beyond *a player has an Ethereum address.*

**Two halves, and you will use both:**

| | | |
|---|---|---|
| **the launcher**, over a local socket | a running client on the player's machine signs for them | `sdk/PROTOCOL.md` |
| **the origin**, over plain HTTPS | public reads about any address | this page, §3 |

Nothing here needs an API key, a partner agreement, or an account with us. Every
read below is public and works from `curl`.

---

## 0 · The five-minute version

```rust
// vendor sdk/rust/scry_overlay.rs — one file, std only, no crates
let mut scry = Overlay::connect("your-game", "1.0.0")?;   // Err is NORMAL

// who is playing, in a form your SERVER can check
let proof = scry.prove("shard-3.yourgame.example", &nonce_you_issued)?;
// → send proof.message + proof.signature to your backend

// a name and a face for the scoreboard
let p = scry.profile("").unwrap();
println!("{}", p.label());       // "moss" if sworn, "~moss" if self-declared
```

On your backend, with **no scry-specific code at all**:

```js
import { SiweMessage } from 'siwe'
const { data } = await new SiweMessage(body.message).verify({
  signature: body.signature,
  nonce:     nonceYouIssued,
  domain:    'shard-3.yourgame.example',
})
// data.address is proven. Now read anything in §3 about it.
```

**Three things that are always true**, and designing against them saves you
work later:

1. **The launcher is never required.** `connect()` failing is a normal state and
   your game must play through it. A player with no launcher, no account and no
   address is a supported player.
2. **You never touch a key.** Not the player's, not ours. The strongest thing
   you can ask for is *a message the player's signer agrees to sign*.
3. **An address is a claim until you verify it.** `identity` and `profile` both
   describe an address; only `prove` establishes that someone holds its key.

---

## 1 · Signing a player in — `prove`

**It is Sign-In With Ethereum (EIP-4361), so use a `siwe` library.** The full
wire, the field rules and every refusal are in `sdk/PROTOCOL.md` §`prove`. The
two things worth repeating here:

- **`server` is your domain**, `host` or `host:port`. No scheme, no path.
- **`nonce` is alphanumeric, at least 8 characters**, issued by you, retired on
  use. A dash-separated uuid is refused by the standard's alphabet — strip the
  dashes.

⚠ **Do not rebuild the message string and compare it.** It carries the
launcher's `Issued At`, so you cannot. Parse it and check the domain and nonce
against your own state; that is what `verify()` does and it is the whole check.

⚠ **A proof authorises nothing.** It says *the holder of this address was here,
for this domain, on this challenge*. It is not a payment, not a permission, and
not a session — it is the input to whatever admission rule you write.

### What you do with the address

Anything in §3. The common shapes:

| you want | do this |
|---|---|
| a ticket check | look the address up against a contract on chain 4663 |
| a ban list | your own database, keyed by address |
| a returning player | your own database — **not** ours; we do not keep your players |
| a display name | §2 |

---

## 2 · Drawing the player — `profile`

One call gives you what a HUD draws. Full shape in `sdk/PROTOCOL.md` §`profile`.

⚠ **The one rule that matters: `handle` is checked and `display_name` is not.**
A handle lives in a locked index and cost SCRY to change. A display name is
whatever its owner typed. Both SDK clients expose `label()`, which renders a
sworn name bare and prefixes anything self-declared with `~`. **Use it, or use
`handle` alone.** Showing a claimed name in the clothes of a checked one is the
cheapest mistake on this page and the hardest to notice.

⚠ **Three states, not two.** `None`/`null` means *we could not look*; a profile
with `sworn: false` and no handle means *we looked and nobody has sworn here*.
Drawing "anonymous" for the first invents a fact about a person. Check
`reachable`.

`identities` is how many vows one wallet holds. A wallet with 3 is one party
with three identities, not three claimants.

---

## 3 · The origin — public reads about any address

Every one of these is a plain `GET`, no auth, and every one is already a `link`
in a `profile` reply so you rarely have to build the URL yourself.

Base: `https://scry.moreright.xyz`

| what you want | endpoint | notes |
|---|---|---|
| **everything about one party** | `/api/who?handle=<address\|name\|npub\|vow_id>` | the join behind `profile`. 404 means *nobody has sworn*, which is a real answer |
| **achievements** | `/api/achievements/of/{wallet}` | `earned` plus **`uncheckable`** — read the second, see below |
| what achievements exist | `/api/achievements` | the catalog and its evidence kinds |
| **items (NFTs)** | `/api/items/of/{wallet}` | ERC-721 on 4663. ⚠ answers `configured: false` today — see below |
| how an item's traits are derived | `/api/items/provenance` | published spec; needs no contract, recomputable by anyone |
| **token balances** | `/api/holdings/{wallet}` | SCRY and each game's own coins, read by `balanceOf`. ⚠ a null balance is a **failed read**, never a zero |
| **does this player own the game** | `/api/ticket/{game}/of/{wallet}` — and `POST /api/ticket/{game}/check` sweeps a whole roster | the licence is holding the title's ticket (an ERC-721): check at join, re-sweep on an interval. ⚠ kick on a definite `false`, **never on `null`** — an outage must not boot a paying player. `TICKET.md` §2b |
| **reputation** | `/api/reputation/{address}` | soulbound; earned or slashed by outcomes, never bought |
| a leaderboard | `/api/arena/leaderboard` | the arena's own; empty until a season runs |
| identity detail | `/api/vow/{vow_id}` | conduct, cadence, the public record |
| an avatar | `/api/vow/{vow_id}/mark.svg` | deterministic, cacheable |

### Your game brings its own coins

Operator, 2026-08-07: *"scry only has SCRY! myrrh and obol are actually gates
now"* · *"its up to games to decide what to do with the 'MYRHH' btc type coin."*

**scry has exactly one coin — SCRY, the reserve.** OBOL and MYRRH are what
**Gates** happens to use; they are a worked example, not a platform currency
you inherit.

So: **a coin policy is your design, and the platform does not dictate design**
(`docs/GATES.md` §3.4). The one rule that does not bend is §2.1 — **every game
coin pairs against SCRY and only there.** That is what makes the reserve the
counter every title shares instead of a pile of unrelated tokens.

#### How your coin reaches a player's purse

**You declare it once, in your listing.** The `disclosure.coins` block your
listing already publishes for the rug screen is the same declaration every coin
surface reads:

```json
"disclosure": { "coins": [
  { "symbol": "MORR", "address": "0x…", "role": "earn",    "decimals": 18, "cap": null },
  { "symbol": "GEM",  "address": null,  "role": "premium", "decimals": 18, "cap": "1000000" }
] }
```

`role` is one of `reserve · earn · spend · premium`. Declare SCRY as `reserve`
if you want it on your screen — it is the platform's, so it is never grouped
under your name. **A coin you have not deployed is declared with
`address: null`**; that is a complete answer, not a missing field, and it
renders as *"no address on this chain yet"* rather than vanishing.

That block is the whole integration. `GET /api/holdings/{wallet}` resolves each
symbol to an address and groups it under your title's name; `GET /api/holdings`
answers the same book with no balances, for a surface that has no wallet yet.
`wallet.html` renders whatever comes back, so **your coins appear in every
player's purse, under your game's name, with no code change anywhere in this
repo and no deploy** — the listing lands and they are there.

Two things worth knowing before you write it. `disclosure` is **not** a dev-desk
field: unlike `blurb` or `capsule`, it arrives through a commit someone reads,
because an address that decides which contract a purse calls `balanceOf` on is
not a thing to let a signature change at 3am. And a title may **not** declare a
coin named `SCRY` or `xSCRY` as its own — those are the platform's two, and a
listing claiming one is dropped rather than rendered.

### ⚠ Three honest-zero traps, and each is a real endpoint answering truthfully

This town publishes its zeros on purpose, which means **an empty answer is
usually a fact, not a failure — but not always, and the difference is in the
payload.** Read the flag, never the count:

- **`/api/items/of/{wallet}`** returns `configured: false, deployed: false` with
  an empty `items` list. That means *no item contract exists yet*, not *this
  player owns nothing*. Render it as "no items in this game" only once
  `configured` is true.
- **`/api/achievements/of/{wallet}`** returns `earned` **and** `uncheckable`.
  The second is the list of badges whose evidence nobody can currently verify.
  Showing `earned` alone as the whole truth overstates; showing
  `earned + uncheckable` as earned overstates worse.
- **`/api/arena/leaderboard`** returns `season: null` when no season is running.
  Zero rows in a live season and zero rows because there is no season are
  different sentences.

**The general rule, and it is this repo's oldest scar:** a reader that never
raises returns nothing for both *"there is nothing there"* and *"we could not
look."* Every endpoint above distinguishes them. Your UI should too.

---

## 4 · What scry will not do for you

Naming these up front is faster than discovering them:

- **We do not store your players.** No save games, no per-title stats, no
  key-value store. Your backend owns your game's state; we own identity and
  what is on chain.
- **We do not proxy your shard list.** `servers` returns *your* URL and you
  serve the list. A launcher between a game and its own servers is a cache
  nobody asked for and a ranking nobody can see.
- **We do not move money between parties.** Not for you, not for players.
- **The launcher socket is not a sandbox.** A game the launcher started runs as
  the player's uid and can already read their files. Say that in your own docs
  too — a player who thinks it is a sandbox will make a decision they otherwise
  would not.
- **We will not sell you a better anything.** The one wall: *would the payer's
  identity or the amount change any bit of an output?* If yes, it does not
  exist here at any price.

---

## 5 · Being listed

Curation is a hand act and **there is no queue** — no submission form, no vote,
no fee to be considered (`docs/GATES.md` §3). A listed game keeps its own repo,
its own issues and its own agents; the only thing that lives in ours is its
listing row.

What a listing needs: an open-source repo, a manifest, and a depot if you ship a
native build (`docs/client/LAUNCHER.md` §3). Games ship free for now — the buy step
lands when there is something worth charging for, and gating is net-new work
rather than a switch (`SENTENCES.md` 2026-08-06).

**And when you do charge, the number is yours.** We do not set a price, suggest
one, or hold a default anywhere: your title's `ScryGameTicket` is born unpriced
and only ever carries what you posted with `setPrices(usdCents, ethWei,
scryWei, usdgUnits)` — a dollar figure and the amount per rail, reposted as
often as you want, and any rail closed by posting it at 0. Curation is a hand
act and pricing is not part of it: charge $2 or $60, run a sale for a weekend,
or ship no ticket contract at all and stay free — the origin answers *free* in
words for that last one, so it is a statement and not an absence. What that
means for anything you build: **read the price, never assume it** —
`/api/ticket/{slug}` off chain, `priceUsdCents()` and `railInfo()` on chain —
because it is a per-title number that can change between two loads of your own
store page.

---

## Reading order

`sdk/PROTOCOL.md` is the wire, verb by verb · `docs/client/SDK.md` is why it is shaped
this way · `docs/client/LAUNCHER.md` is the client it lives in · `docs/GATES.md` is
what being listed means
