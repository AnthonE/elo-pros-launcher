---
status: live
lane: [platform]
updated: 2026-10-28
about: "everything a game needs from Elo, in one page — sign a player in with standard Sign-In With Ethereum, draw their name and face, read what they own and what they have done, and the honest-zero traps that make an empty answer mean two different things"
---

# GAMEDEV.md — everything a game needs from Elo

> Operator, 2026-08-07: *"make the SDK just like steams so its easy for games to
> auth and get profile info"* · *"whatever other api end points a game dev might
> need for players"*

You are building a game and you want to know who is playing, what they own, and
what they have done. That is this page. It assumes nothing about Elo and
nothing about crypto beyond *a player has an Ethereum address.*

**Two halves, and you will use both:**

| | | |
|---|---|---|
| **the launcher**, over a local socket | a running client on the player's machine signs for them | `sdk/PROTOCOL.md` |
| **the origin**, over plain HTTPS | public reads about any address | this page, §3 |

**A browser game on its own domain swaps the first row** for the web SDK,
`https://elopros.com/sdk/elo-3.js`: it connects the player's own wallet
(Ethereum or Solana), signs them in to your server with the same SIWE
message, and draws their name and face (§5). A game built in an engine
exports to the web the same way (§5a).

**The SDK packages wrap both halves**, one per language: the Rust `elo-sdk`
crate (`sdk/rust`), the Python `elo-sdk` package (`sdk/python`) and
`@elopros/sdk` (`sdk/js`). Each has the launcher client, typed calls for the
reads in §3 (friends and their signed changes, copies, items, faces, `who`,
the leaderboard), **a game server's half** — the sign-in check, the roster
sweep and a nonce (§1) — and a writer for a wipe's payout file. They are not
on the package registries yet; install from the mirror
(`https://github.com/AnthonE/elo-pros-launcher/tree/main/sdk`,
`sdk/README.md` is the index):

```bash
pip install "git+https://github.com/AnthonE/elo-pros-launcher#subdirectory=sdk/python"
# Cargo.toml: elo-sdk = { git = "https://github.com/AnthonE/elo-pros-launcher" }
# JavaScript: the <script> tag in §5, or npm install ./elo-pros-launcher/sdk/js from a clone
```

**A whole working example** — a page and a zero-dependency Node server that
signs players in, checks their copy and sweeps the roster — is
`sdk/js/examples/web-game/`. Run it with `node server.mjs`.

Nothing here needs an API key, a partner agreement, or an account with us. Every
read below is public and works from `curl`.

---

## 0 · The five-minute version

```rust
// vendor sdk/rust/elo_overlay.rs — one file, std only, no crates
let mut elo = Overlay::connect("your-game", "1.0.0")?;   // Err is NORMAL

// who is playing, in a form your SERVER can check
let proof = elo.prove("shard-3.yourgame.example", &nonce_you_issued)?;
// → send proof.message + proof.signature to your backend

// a name and a face for the scoreboard
let p = elo.profile("").unwrap();
println!("{}", p.label());       // "moss" if sworn, "~moss" if self-declared
```

On your backend, in **any language**, one call checks it — the domain, your
nonce, freshness and the signature, smart wallets and Solana included:

```bash
curl -s https://elopros.com/api/signin/check -H 'content-type: application/json' \
  -d '{"message": "…", "signature": "…", "domain": "shard-3.yourgame.example", "nonce": "…"}'
# 200 {"ok": true, "address": "0x…", "chain": "evm", …} — proven. Now read anything in §3 about it.
```

Or with **no Elo-specific code at all**, since it is a standard message:

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

## 1 · Signing a player in

**Most games want Sign in with Elo Pros** (`ACCOUNTS.md` §6): the player signs
in to Elo Pros with Steam, Discord, Google, GitHub or a wallet, says yes to
your site, and your page gets a pass from `sdk/elo-signin-1.js`. Your server
asks `POST /api/account/play/check {token, game, domain, nonce}` (or the SDKs'
`checkPass` / `check_pass`) and gets the player's account, name and linked
wallet. Your game never asks for a wallet, so a closed-source game needs no
public repo (`TWO-CHAIN-STORE.md` §7 rule 3). The rest of this section is the
wallet's own sign-in, for a game that wants the wallet itself.

### With the wallet — `prove`

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

### Checking it: `POST /api/signin/check`

The same check for the launcher's `prove` and the web SDK's `signIn`, from a
server in any language — Godot, Go, Rust, Python, Elixir — with no crypto
library and no RPC endpoint of your own:

| send | |
|---|---|
| `message`, `signature` | exactly what the player's client handed you |
| `domain` | **required** — your host or `host:port`. Without it, a sign-in made on another site would pass |
| `nonce` | **required** — the one you issued |
| `uri`, `chain_id` | optional: the `URI:` and `Chain ID:` lines must equal them |
| `max_age_s` | optional: how old `Issued At` may be (default 600, at most 86400) |

| answer | means | do |
|---|---|---|
| `200` | `address` (EIP-55, or base58 for Solana) and `chain` are proven | admit, by your own rule |
| `401` | not a valid sign-in; `code` names the field: `domain`, `nonce`, `stale`, `expired`, `not_yet`, `uri`, `chain_id`, `version`, `signature`, `format` | refuse, and ask for a new sign-in |
| `400` | your request is malformed | your bug |
| `503` | a smart wallet's chain could not be asked (`reachable: false`) | ask again — **never refuse a player on it** |

**It stores nothing**: retire the nonce yourself the moment you send it, so a
captured sign-in cannot be replayed. The SDKs wrap it — `checkSignIn` /
`check_sign_in` — and add `newNonce` / `new_nonce` for the challenge (letters
and digits, from the system's random source) and `checkRoster` /
`check_roster` for the sweep in §3. viem's `verifySiweMessage` is the local
alternative for an Ethereum sign-in on a Node server.

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
A handle lives in a locked index and costs ELO to change. A display name is
whatever its owner typed. Both SDK clients expose `label()`, which renders a
sworn name bare and prefixes anything self-declared with `~`. **Use it, or use
`handle` alone.** Showing a claimed name in the clothes of a checked one is the
cheapest mistake on this page and the hardest to notice.

⚠ **Three states, not two.** `None`/`null` means *we could not look*; a profile
with no name and no face means *we looked and nothing is set*.
Drawing "anonymous" for the first invents a fact about a person. Check
`reachable`.

---

## 3 · The origin — public reads about any address

Every one of these is a plain `GET`, no auth, and every one is already a `link`
in a `profile` reply so you rarely have to build the URL yourself. The ones a
page draws — `who`, items, achievements, the leaderboard, friends, faces and
the copy reads — also answer a page on **your** origin (CORS `*`, no
cookies), so a web game can call them straight from the browser.

Base: `https://elopros.com`

| what you want | endpoint | notes |
|---|---|---|
| **everything about one party** | `/api/who?handle=<address\|name\|npub>` | the join behind `profile`. A bare wallet always answers 200; 404 means an unknown name |
| **achievements** | `/api/achievements/of/{wallet}` | `earned` plus **`uncheckable`** — read the second, see below |
| what achievements exist | `/api/achievements` | the catalog and its evidence kinds |
| **items (NFTs)** | `/api/items/of/{wallet}` — `?title=<slug>` keeps one game's | ERC-721 on 4663; Gates' contract is `EloItem:GATES`. ⚠ `reachable: false` means *could not look* — see below |
| how an item's traits are derived | `/api/items/provenance` | published spec; needs no contract, recomputable by anyone |
| **token balances** | `/api/holdings/{wallet}` | the reserve and each game's own coins, read by `balanceOf`. ⚠ a null balance is a **failed read**, never a zero |
| **does this player own the game** | `/api/ticket/{game}/of/{wallet}` — and `POST /api/ticket/{game}/check` sweeps a whole roster (100 wallets a call; the SDKs' `checkRoster` pages it) | the licence is holding the title's ticket (an ERC-721): check at join, re-sweep on an interval. ⚠ kick on a definite `false`, **never on `null`** — an outage must not boot a paying player. `TICKET.md` §2b |
| **friends** | `/api/friends/{wallet}` | who the player plays with and what each is playing now (with an `elo://join` link), the player's own AI agents (`buddies`) and pending asks. Public. A change is signed: `POST /api/friends/prepare`, then `POST /api/friends/act` (`docs/client/FRIENDS.md`) |
| a leaderboard | `/api/leaderboard/{slug}` | the boards your shard reports, ranked by its game score: this wipe's and the closed ones (`final: true`). `?kind=agent` or `?kind=human` ranks one side. The house ranks what the shard published and never re-scores it |
| a face | `/api/face/{wallet}` · `/api/face/{wallet}/pic.png` | the wallet's own signed name and picture, and its `avatar` (below) |

### Drawing a player's avatar

`avatar` on `/api/face/{wallet}` is the blocky figure a player picked on their
profile, one 32-bit word, or null if they never set one. Six parts at 5 bits
each, an index into a 32-colour palette: head bits 0-4, torso 5-9, left arm
10-14, right arm 15-19, left leg 20-24, right leg 25-29. Bit 30 set is a
robot's face (two lit eyes, a grille), clear a smile; bit 31 is set on every
avatar. The palette, by index:

```
#f5cd30 #ffd9b3 #e0ac85 #b07a55 #7a4e31 #4a2e1d #ffffff #c7c9cc
#8a8d91 #4d5054 #1c1d21 #d7332b #8e1f1b #f07a2e #ffc82e #f2e6b8
#7fd13b #3c9a47 #1f5c33 #2ec4b6 #7cc6f2 #2f6fd6 #1f2f6b #6a4bc4
#b48af0 #e05ab8 #ff9fc8 #8b5a2b #c2b280 #5e6b3a #9aa7b0 #3a2a4d
```

A player with none wears the default if your game draws one: `0xa31002a0`,
a yellow head and arms, a blue torso, green legs and a smile
(`meter/faces.py` `AVATAR_DEFAULT`). Fireplace draws it on its riders and its
fighters. A player sets theirs on their profile; an agent sends `avatar` to
the MCP `face` tool, signed like the name.

### Your game brings its own coins

**Elo has exactly one coin — the reserve.** JUNK is what **Gates** happens
to use; it is a worked example, not a platform currency you inherit.

So: **a coin policy is your design, and the platform does not dictate design**
(`docs/GATES.md` §3.4). The one rule that does not bend is §2.1 — **every game
coin pairs against the reserve and only there.** That is what makes the reserve the
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

`role` is one of `reserve · earn · spend · premium`. Declare ELO as `reserve`
if you want it on your screen — it is the platform's, so it is never grouped
under your name. **A coin you have not deployed is declared with
`address: null`**; that is a complete answer, not a missing field, and it
renders as *"no address on this chain yet"* rather than vanishing.

That block is the whole integration. `GET /api/holdings/{wallet}` resolves each
symbol to an address and groups it under your title's name; `GET /api/holdings`
answers the same book with no balances, for a surface that has no wallet yet.
The terminal's coins pane (`terminal.html#coins`) renders whatever comes back, so **your coins appear in every
player's purse, under your game's name, with no code change anywhere in this
repo and no deploy** — the listing lands and they are there.

Two things worth knowing before you write it. `disclosure` is **not** a dev-desk
field: unlike `blurb` or `capsule`, it arrives through a commit someone reads,
because an address that decides which contract a purse calls `balanceOf` on is
not a thing to let a signature change at 3am. And a title may **not** declare a
coin named `ELO` or `xELO` as its own — those are the platform's two, and a
listing claiming one is dropped rather than rendered.

### ⚠ Three honest-zero traps, and each is a real endpoint answering truthfully

This town publishes its zeros on purpose, which means **an empty answer is
usually a fact, not a failure — but not always, and the difference is in the
payload.** Read the flag, never the count:

- **`/api/items/of/{wallet}`** returns an empty `items` list with
  `reachable: false` when the explorer or the chain could not be read. That
  means *we could not look*, not *this player owns nothing*. A meter with no
  item contract answers `configured: false` the same way. Render "no items"
  only when neither flag says otherwise.
- **`/api/achievements/of/{wallet}`** returns `earned` **and** `uncheckable`.
  The second is the list of badges whose evidence nobody can currently verify.
  Showing `earned` alone as the whole truth overstates; showing
  `earned + uncheckable` as earned overstates worse.
- **`/api/leaderboard/{slug}`** answers `published: false` when a title has no
  board, and `shard.why` when its shard did not answer. Zero rows in a live
  wipe and zero rows because there is no board are different sentences, and a
  card that renders them the same way is lying in one of the two cases.

**The general rule, and it is this repo's oldest scar:** a reader that never
raises returns nothing for both *"there is nothing there"* and *"we could not
look."* Every endpoint above distinguishes them. Your UI should too.

## 3b · Your server's key, and what it writes

Your servers sign their writes with an Ed25519 key you name: on the dev desk
(act `serverkeys`) or in `elo.json`:

```json
"server_keys": [{ "id": "eu-1", "pub": "<the 32-byte public key, base64>" }]
```

`openssl genpkey -algorithm ed25519 -out server.key` makes one;
`openssl pkey -in server.key -pubout -outform DER | tail -c 32 | base64` is the
public half. We keep only that half. Each request carries `X-Elo-Server:
<game>/<key id>`, `X-Elo-Time`, a fresh `X-Elo-Nonce` and `X-Elo-Signature`
over the text `GET /api/store/serverkeys/<game>` spells out; the SDKs' `ServerKey`
builds all four, and `POST /api/serverkeys/check` tests them.

| with the key | route |
|---|---|
| a player's saves (Steam Cloud) | `PUT`, `GET`, `DELETE /api/cloud/<game>/<wallet>/<slot>`; `GET /api/cloud/<game>/<wallet>` lists them |
| stats, achievement unlocks, leaderboard scores | `POST /api/gamestats/<game>/report`, against what you declared (act `gamestats`, or `stats` in `elo.json`) |
| your game's bug reports, in full | `POST /api/bugs/<game>/token`, then the token as a bearer |

| at join, no key needed | route |
|---|---|
| who is in a party's lobby | `GET /api/lobbies/<id>` |
| a server with room for a party | `GET /api/lobbies/pick/<game>?party=<n>` |
| a player's Workshop mods, with sha256s to check | `GET /api/workshop/<game>/subscribed/<wallet>` |
| is this wallet in your playtest | `GET /api/codes/<game>/access/<wallet>` |

A line of rich presence ("Wave 5 · 3/4") rides the roster your server already
publishes: add `"status"` to a row of `online.json`, and it shows beside the
game on the player's profile and friends list.

---

## 4 · What Elo will not do for you

Naming these up front is faster than discovering them:

- **We store only what your servers send us.** Cloud saves, stats,
  achievements and scores land here when your servers write them with their
  server key (§3b); nothing else about your game is kept, and the player can
  read out and delete their own saves. Your backend still owns your game's
  state; we own identity, what is on chain, and what your servers hand us.
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

**There is no queue, in either lane** (`docs/GATES.md` §1). A listed game keeps
its own repo, its own issues and its own agents; the only thing that lives in
ours is its listing row.

**List it from your repo, with no wallet.** Commit `elo.json` and a LICENSE,
paste the repo into `https://elopros.com/dev-console.html#repo`, and list it
when the checklist reads ready. The console writes the file for you, price and
all; an agent gets the same from `dev` on `/mcp` (`docs/store/GAME-REPO.md` §9).

**List it yourself — the community lane.** Free, one wallet signature, and the
page is up (`docs/GATES.md` §3b). Use *list your game* on
`https://elopros.com/manage.html#list`, or the API: `POST /api/store/claim/prepare`,
sign the text with your wallet (EIP-191 — it carries a warranty that you have
the rights to the game and accepts the community terms), then
`POST /api/store/claim`. That wallet is the row's steward on the dev desk from
then on. The page is marked *community*, it runs on your own host
(`play_url`, https, opened in a new tab), and it rises on the shelves only by
copies sold. To sell copies, the desk's *sell copies* makes you a ticket whose
money lands in a sale split: **100% to you, 0% to us**. The house share is read
from the claim terms, never assumed: `house_bps` in `GET /api/store/open`
(`on_chain`), and in `you_get` on `GET /api/store/open/list/prepare`.

**A game that asks for the player's wallet publishes its source**: a public
GitHub repo with an open-source licence, set as `repo` on your listing, and
`wallet: true` on the claim. We check it — the licence file read from GitHub
with no login, and recognised — as you list, before you sell copies or set a
coin, and when a desktop build's scan hears the launcher's signing socket or
a wallet's requests; it is a posted term for a game's own sign-in
(`terms.html#community`). Players can then read what your game asks their
wallet to sign. **A game that never asks may keep its source closed**
(`wallet: false`). Open source and not taken down is what puts the **clean**
chip on your page (`GET /api/store/checks/{slug}`).

A community game plays **on the web from the day it lists**: the store and the
launcher open its `play_url`. **A desktop build** ships through the launcher
after a review — sign and submit its depot on the dev desk, and an automatic
scan and a person's review come before players can install it
(`docs/store/DESKTOP-BUILDS.md`; the terms and any fee at
`GET /api/launcher/review`). For the web, link the web SDK, pinned by its hash:

```html
<script src="https://elopros.com/sdk/elo-3.js"
        integrity="sha384-pKOcdX/DZSGO11b/QS2tFdcm6YFqcVCl0xc6IHDJKFBYRJCvY2JBNjbaVA3jIih/"
        crossorigin="anonymous"></script>
<script type="module">
  const wallet = await Elo.connect();                       // the player's own wallet, Ethereum or Solana
  const { message, signature } = await Elo.signIn({ nonce }); // nonce from YOUR server
  const owned = await Elo.owns("your-slug", wallet);        // true | false | null
  const me = await Elo.profile();                           // me.label, me.avatar — null: could not look
  Elo.on("account", (who) => { /* the wallet switched accounts: sign in again */ });
</script>
```

Check `message` + `signature` on your server with `POST /api/signin/check`
(§1), passing your domain and the nonce — or, on Node, viem's
`publicClient.verifySiweMessage({ message, signature, domain, nonce })`, which
also checks smart-contract wallets (Safe, Coinbase Smart Wallet, Robinhood's
EIP-7702 wallets) where a bare `ecrecover` refuses them. With no wallet in the
browser, `connect()` throws `code: "no_wallet"`. For phone wallets, pass your
own WalletConnect project ID: `Elo.config({ walletconnectProjectId: "…" })`.
TypeScript types for `window.Elo` ship in the package (`@elopros/sdk/elo-3`).
`owns()` is for your UI; **the real gate is your server**, at join and on an
interval: `POST /api/ticket/{slug}/check {"wallets": [...]}`. Kick on `false`
only — `null` means the read failed, and a failed read never kicks a player.
`sdk/js/examples/web-game/` does all of it.

### 5a · From an engine's web export

An engine that exports to the web — Godot, Bevy, Defold, raylib through
Emscripten, three.js — keeps the wallet in the page around the game. Load
`elo-3.js` in the HTML shell the export writes, do the sign-in there, and
hand the game what it needs: the address and your server's session.

| engine | calling the page from the game |
|---|---|
| Godot 4 | `JavaScriptBridge.eval("Elo.address")`, or `JavaScriptBridge.get_interface("Elo")` and a callback for the promise |
| Bevy / any Rust on wasm | `wasm-bindgen` imports, or `web_sys::window()` and `js_sys::Reflect::get` on `Elo` |
| raylib, SDL or C++ through Emscripten | `EM_JS` / `EM_ASYNC_JS` functions that `await Elo.signIn(...)` |
| Defold | `html5.run("…")`, or a native extension's JS bridge |

**Clips for your players.** Load `elo-clip-1.js` in the same shell and
`EloClip.keep(canvas)` keeps the last seconds of the canvas the export draws
into; a clip key calls `save()` for a 9:16 MP4 with your title and link on
the end card, made in the player's browser (`sdk/js/README.md`). A WebGL
export's context needs `preserveDrawingBuffer: true`, or `manual: true` and
`keep.frame()` after each frame. If your game can replay a match, hand the
replay's frames to `EloClip.film()` instead and pick the camera yourself.

The sign-in check stays on your game server (§1), whatever the engine. A
desktop build of the same game reaches the launcher instead (`sdk/PROTOCOL.md`:
newline-delimited JSON on a local socket, or a named pipe on Windows), and
plays anonymously when there is none.

**Get picked — an Elo Pros original.** Our own shelf is placed by hand, and a
game there runs the whole stack: open source at listing, a coin launched on an
outside launchpad and paired with the reserve, native builds through the
launcher (a manifest and a depot — `docs/client/LAUNCHER.md` §3). There is no form for that one either.
Made it in an AI game maker? `docs/builders/PORT-BOUNTY.md` is the paid port —
optional help, never the door.

---

## Reading order

`sdk/PROTOCOL.md` is the wire, verb by verb · `docs/client/SDK.md` is why it is shaped
this way · `docs/client/LAUNCHER.md` is the client it lives in · `docs/GATES.md` is
what being listed means
