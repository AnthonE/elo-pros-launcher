# the elo overlay protocol — v1

What a game says to the running launcher. This is the whole wire; the
reference clients in `sdk/python/` and `sdk/rust/` are each about two hundred
lines and implement all of it.

Design of record: `docs/client/SDK.md`. The server is
`launcher-rs/crates/elo-broker`.

## The shape

A UNIX stream socket. **Newline-delimited JSON**, one request object per line,
one reply object per line, in order, on one connection. No framing header, no
length prefix, no handshake beyond `hello`.

```
→ {"op":"hello","game":"gates","protocol":1,"version":"0.1.0"}
← {"ok":true,"protocol":1,"launcher":"elo-launcher","signer":"arca", …}
```

Every reply has `ok`. A reply with `ok:false` has `reason`, and the reason is
written to be shown to a player, not parsed.

## Finding the socket

1. `$ELO_LAUNCHER_SOCKET` — set by the launcher on every native build it
   starts. **This is the path a game should use**, and it is the same variable
   on every platform.
2. The platform default, for a game started some other way:

   | | |
   |---|---|
   | unix | `$XDG_RUNTIME_DIR/elo/launcher.sock`, else `~/.cache/elo/launcher/launcher.sock` |
   | Windows | `\\.\pipe\elo-launcher-<USERNAME>` |

**If neither exists there is no launcher, and that is a normal state.** A game
plays without one; what it must never do is fall back to asking the player for
a private key. There is no third option and the SDK does not offer one.

### Windows is a named pipe, and the wire is unchanged

There is no `AF_UNIX` in Rust's `std` on Windows and CPython does not expose
one either, so the door there is a **named pipe** — the platform's own answer
to the same question, and what the client this is ripped from actually uses.
A named pipe opens as an ordinary file, so both reference clients change only
where they open the door; every byte after that is identical.

Two things follow, and both are properties rather than details:

* **One pipe per user.** The Windows pipe namespace is machine-wide, so the
  name carries `USERNAME`. Two people signed into one box must not land on the
  same door, or a game asks the wrong session's launcher to sign.
* **The pipe is created `PIPE_REJECT_REMOTE_CLIENTS`.** The `\\.\pipe\`
  namespace is otherwise reachable over SMB, and this door is local by design.

⚠ **The connect timeout is bounded on Windows; reads are not.** The launcher
waits for a free pipe instance and then gives up, so a launcher that is not
running still fails fast — but once open, a read blocks. Bounding it wants
overlapped I/O, which is a great deal of machinery for a file whose whole
property is that it vendors with no crates and no build script. What it costs:
a launcher that accepts and then never answers hangs the calling game instead
of erroring, where on unix it would error. That is a launcher bug in both
cases; on Windows the game feels it as a freeze. **Call the SDK off your main
thread** — a consent prompt waits for a human, so that is right regardless.

⚠ **The default MUST agree across three files** — `elo-broker`'s
`transport.rs`, `sdk/rust/elo_overlay.rs`, and `sdk/python/elo_overlay.py`.
A mismatch is the worst shape of bug this protocol has: a game finds no
launcher on a machine that is running one, reports the perfectly normal
*"playing anonymously"*, and nothing anywhere goes red.

## The verbs

Fields are **allowlisted per verb**. An unknown field is a refusal, never an
ignore — a caller that thinks it set something and was quietly ignored has
been misled in the dangerous direction.

### `hello` — required first

```json
{"op": "hello", "game": "gates", "protocol": 1, "version": "0.1.0"}
```

Reply carries `protocol`, `launcher`, `host`, `capabilities` (the verb list),
and `signer` (which backend holds the key: `local` · `arca` · `external` ·
`browser` · `none`).

⚠ **Only `local` can `prove`.** `prove` bypasses the signer's family allowlist
by construction, and the other backends do not implement it — `browser` cannot,
because it finishes in a browser tab rather than in this round trip. A player
whose launcher has an account gets `local`; one with no account gets `none` and
a refusal that says so. Anonymous is a supported state.

Every other verb refuses until `hello` has named the game, because a consent
prompt with no name in it is not consent.

### `identity`

```json
{"op": "identity"}
→ {"ok": true, "address": "0x…", "host": "https://elopros.com", "signer": "arca"}
```

**An address, never a key, and never authentication.** It is what the player
asked the launcher to watch. Treat it as a claim; if the answer matters, ask
for a signature and verify it. `address` may be `null` — someone browsing
without an address set is a normal player.

### `sign`

```json
{"op": "sign", "text": "elo play\naction: duel\nwallet: 0xabc…\nday: 2026-08-04\ndetail: ETH up 5",
 "why": "settling round 41"}
→ {"ok": true, "signature": "0x…", "address": "0x…", "family": "play", "backend": "arca"}
```

The message must be an EIP-191 personal_sign text whose first line is
`elo <family>`. Build it with the same rules the server uses
(`meter/playauth.py` — the format is deterministic and rebuildable offline).

> ⚠ **The subject is the `wallet` line, and it was `vow: <vow_id>` until
> 2026-08-12.** A game no longer has to send its players to swear a vow before
> they can act — the signature recovers the signer, so the wallet IS the
> identity. **Two rules if you build the text yourself:** the address is
> **lowercase** (a checksummed address is different bytes and verifies
> differently), and the `day` is UTC `yyyy-mm-dd`, so a signature is good for
> that day only.
>
> The old vow-keyed text is **still accepted** for a caller that has one, so an
> integration written against the previous shape keeps working while it moves.
> It is a transition, not a second dialect — build the wallet form.

**A human reads both fields, so write them for one.** The launcher shows the
player your `text` **verbatim** — every line of it, in a scrolling well, in the
launcher's own window — and your `why` above it, attributed as *"it says: …"*
because it is your sentence and not the launcher's. Two things follow for a
game developer:

* **`detail:` is player-facing.** It is inside the bytes being signed and it is
  on screen while somebody decides. A `detail:` full of internal ids asks a
  player to approve something they cannot read.
* **`why` is a claim and is drawn as one**, flattened to a single line and cut
  if it is long. Newlines and control characters are stripped rather than
  rendered — a `why` that could add lines could forge the launcher's own words
  in the launcher's own window, which is the same smuggling `prove`'s field
  rules exist to stop.

⚠ **This is the one call that waits for a person.** Both reference clients
allow 300 seconds for it and the launcher gives up at the same mark, so a
prompt nobody answers comes back as a refusal rather than hanging either side.
Call it off your main thread.

Three ways it does not return a signature, and they are **not the same
outcome**:

| reply | what happened | what a game should do |
|---|---|---|
| `ok:false` + `refused_by: "the player"` | the consent prompt was refused | show it; do not retry |
| `ok:false` + `handoff: "https://…"` | the launcher's signer is the browser | show the link; the player finishes there; **do not retry in a loop** |
| `ok:false`, neither | the signer refused (unknown family, budget spent, unarmed) | show `reason` |

A game that treats `handoff` as an error will tell the player something broke
when nothing did.

### `prove` — who is playing, in a form your server can check

```json
{"op": "prove", "nonce": "8f14e45fceea167a", "server": "shard-3.gates.example"}
→ {"ok": true,
   "signature": "0x…",
   "address": "0x24445EFddB08d4938E3E3627042B2Cf4063d9092",
   "scheme": "eip4361",
   "message": "shard-3.gates.example wants you to sign in with your Ethereum account:\n0x24445…",
   "verify": "this is a SIWE (EIP-4361) message — hand it to any siwe library …"}
```

**Reach for this, not `identity`.** `identity` returns an address, and an
address is a claim any modified client can make. This returns a signature your
server can check.

#### It is Sign-In With Ethereum, EIP-4361 — nothing elo-specific

The message is the standard one, so **your backend needs no elo code at all.**
`siwe` ships for JS, Python, Rust and Go; viem and ethers have it built in.

```js
import { SiweMessage } from 'siwe'
const msg = new SiweMessage(body.message)
const { data } = await msg.verify({
  signature: body.signature,
  nonce:     nonceYouIssued,           // yours, not theirs
  domain:    'shard-3.gates.example',  // yours, not theirs
})
// data.address is now proven. Look it up and admit or kick.
```

The full message looks like this:

```
shard-3.gates.example wants you to sign in with your Ethereum account:
0x24445EFddB08d4938E3E3627042B2Cf4063d9092

Prove your identity to play gates. This signature authorises nothing and moves no funds.

URI: https://shard-3.gates.example
Version: 1
Chain ID: 4663
Nonce: 8f14e45fceea167a
Issued At: 2026-08-07T12:00:00Z
```

#### ⚠ Do not recompute the string and compare

This changed with the standard, and it is the one habit to unlearn. The message
carries `Issued At` from the **launcher's** clock, so you cannot rebuild it
byte for byte. Instead **parse it and check the fields against your own state**
— which is exactly what passing your `nonce` and `domain` to `verify()` does.

The security is identical: the signature covers precisely these bytes, and
every field you rely on is one you checked rather than accepted. What you must
never do is take `message` and act on its contents *without* checking the
domain and nonce against what you issued — that proves only that the client can
concatenate strings.

#### The launcher writes the message, and that is why there is no prompt

`sign` asks the player, because the game supplies the text. `prove` does not,
because the game supplies **only two opaque fields**: the launcher composes
every word, and `game` comes from your `hello` handshake rather than from this
request — so a title cannot prove as another title. A signature over this
message authorises nothing; it says *the holder of this address was here, for
this domain, on this challenge*.

#### The two fields, and their rules are the standard's

| field | rule | why |
|---|---|---|
| `nonce` | **alphanumeric, 8–128 characters** | EIP-4361's own requirement. It is also the injection guard — a value that cannot spell a newline cannot add a line to a message the launcher authored |
| `server` | a **domain authority**: `host` or `host:port`. No scheme, no path | EIP-4361 binds the domain, and the launcher builds the `URI:` line from it. It lands in the signed bytes twice |

⚠ **A dash-separated uuid is refused.** `550e8400-e29b-41d4-a716-446655440000`
does not fit the alphabet; strip the dashes and it does. Base64url nonces
(`abc_-123+/=`) need re-encoding to hex or base62. This is stricter than a
hand-rolled format would be, and it is the price of a message any off-the-shelf
verifier reads.

⚠ **The nonce must be one you issued and have not seen before.** With no fresh
nonce, a captured proof is valid forever to anyone who has it. Bind it to the
connection and retire it on use.

⚠ **Check `Issued At` if you care about staleness.** EIP-4361 carries it, and
most `siwe` libraries will enforce `Expiration Time` if present — this launcher
does not set one, so ageing a proof is your call and your nonce retirement is
what actually stops replay.

#### The refusals, and each names a different fix

| `reason` contains | what happened | what to show |
|---|---|---|
| `no account on this machine` | the player has no key — **SIWE names the address, so there is no message to compose without one** | "make an account in your launcher" — playing on is normal |
| `the account is locked` | there is one, unlocked nowhere | "unlock it in Signing" |
| `nonce`/`server` complaints | your field broke a rule above | your bug, not the player's |
| `cannot verify against its own message` | the launcher composed for one account and signed with another | a launcher bug; it refuses rather than hand you an unverifiable proof |
| `cannot prove an identity` | this launcher has no signer at all | treat as anonymous |

**Anonymous is a supported state.** A game that refuses to start without a
proof has made the launcher required, which it never is.

### `profile` — a name and a face to draw

```json
{"op": "profile"}
{"op": "profile", "address": "0x…"}
→ {"ok": true, "reachable": true, "address": "0x…",
   "profile": {"handle": "moss", "sworn": true, "display_name": "Moss",
               "avatar": "/api/vow/…/mark.svg", "identities": 2,
               "links": {"achievements": "/api/achievements/of/0x…", …}},
   "source": "https://…/api/who?handle=0x…"}
```

Steam's `GetPlayerSummaries`. With no `address` it answers for the one this
launcher watches; pass one to look up somebody else — the address you got back
from `prove`, or another player on your shard.

Behind it is one read of `/api/who`, the origin's own join across sworn name,
npub, vow and wallet, flattened to what a HUD actually draws. **One verb is one
origin read**: achievements, holdings and reputation are separate endpoints and
arrive here as urls under `links`, the same way Steam splits summaries from
achievements. Fetch those yourself.

#### ⚠ Three states, and collapsing them tells a lie

| reply | what happened | what to draw |
|---|---|---|
| `ok:true` + a `profile` | we looked and found one | the profile |
| `ok:true` + `profile:null` + `reachable:true` | we looked; **nobody has sworn on this address** | an anonymous player — a normal way to play |
| `ok:false` + `reachable:false` | **we could not look** | nothing about identity; say the origin is unreachable |

`CLAUDE.md` §traps — *a never-raises reader returns `[]` for both "nothing
there" and "we could not look."* A game that draws a confident *"anonymous"*
over a network outage has invented a fact about a person. The `reachable` flag
is the machine-readable half; the `reason` says the same thing in words.

#### ⚠ `handle` is sworn and `display_name` is not

`handle` is the NIP-05 name out of the locked index — it cost SCRY to change.
`display_name` is whatever its owner typed into a face. **Never show a claimed
name in the clothes of a checked one.** Both reference clients expose a `label`
that does this for you: a sworn name renders bare, anything self-declared gets
a `~` prefix (`ACHIEVEMENTS.md`'s convention), and a nameless player falls back
to a shortened address.

#### ⚠ This is not authentication

It describes an address. It does not establish that the player is holding its
key — anything can name an address. `prove` is the verb for that; look up the
address you recovered, not one a client handed you.

`identities` is how many vows the wallet holds: one party, several identities.
A wallet with 3 is not three claimants.

### `overlay` — one snapshot a game can DRAW

```json
{"op": "overlay", "slug": "gates"}
→ {"ok": true, "overlay": {"host": "https://…", "signer": "arca",
                           "address": "0x…", "title": {…}, "consent": {…}}}
```

Steam injects a library into the game process and hooks the graphics API,
because Steam does not own the games. This platform does, so an overlay here is
**the game rendering its own HUD** from one read — no injection, nothing to
break on a driver update, nothing that reads as malware to an anti-cheat.
Cheap enough to poll each frame: one round trip on a local socket, and every
field could already have been asked for one at a time.

⚠ **Display freely; never approve.** A game controls its own pixels, so it can
draw a convincing fake of any dialog. There is no consent token in this reply
and no way to mint one, no pending-request queue to render as your own prompt,
and no "already allowed" flag. Consent stays per game, per family, counted,
asked in the launcher's own window, and dies with the launcher. A HUD that
draws its own *"allow this signature?"* is drawing a forgery — call `sign` and
let the launcher ask.

### `title` · `servers`

```json
{"op": "title", "slug": "gates"}
{"op": "servers", "slug": "gates"}
→ {"ok": true, "url": "https://…", "note": "fetch it yourself; the launcher does not proxy it"}
```

`servers` returns the **url** from the manifest, not the list. The launcher
does not proxy, cache or rank a game's shard list — that list is the game's to
serve.

### `open`

```json
{"op": "open", "url": "https://elopros.com/wallet.html"}
```

https only. For sending a player to a page — their coins, a store page, the
handoff link from a `sign` refusal.

## What is deliberately absent

No verb installs, uninstalls, launches another title, reads a file, writes a
setting, chooses a signer, or names a signing command. A game may ask about
**itself** and ask for a **signature**; it may not operate the launcher.

Adding one of those is a redesign, not a feature.

## What this socket is not

**It is not a security boundary against a process running as the same user.**
A game the launcher started runs as your uid; it can already read your files.
`arca.py` can check `SO_PEERCRED` and refuse a same-uid client because the
arca is a *different user* — the broker cannot, because it is not.

What it provides instead, and each of these is real:

1. **A game never needs a key.** The strongest ask available to a hostile
   game is *a message the signer will sign* — a recognised family, journaled,
   with a daily count — instead of "paste your private key here", which is
   unrecoverable once a player has been taught it.
2. **Consent is per game, per family, counted, and dies with the launcher.**
   There is no "always allow" and none is written to disk.
3. **The real wall is the signer behind it.** With `signer: arca`, every
   request is re-derived by a process running as somebody else, against its
   own ledger, with its own refusals.

Say this in your game's own docs too. A player who thinks the socket is a
sandbox will make a decision they would not otherwise make.

## Versioning

`protocol` is an integer and this is `1`. A launcher answers `hello` with the
protocol it speaks; a game that needs a newer one should say so and carry on
without the launcher rather than degrade silently.
