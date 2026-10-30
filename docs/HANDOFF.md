---
status: live
lane: [platform]
updated: 2026-10-27
about: "moving a connected player from one of a game's servers to another — zones, an arena behind a door, a match server: the server they leave signs a handoff with its server key, the next one checks it offline; no call to Elo Pros per hop, no prompt, the same for a wallet, a login, the launcher and an agent. The text, the checks, the client's part, and the three SDKs"
---

# HANDOFF.md — moving a player between your servers

**The pass gets a player in. A handoff moves them.**

A player joins a game's first server the usual way: a wallet signature, or an
Elo Pros pass (`ACCOUNTS.md`). From then on, the server they are on moves them:

1. it signs a **handoff** with one of the game's server keys (`SDK.md`, the
   keys you name on the dev desk or in `elo.json`);
2. it tells the client *go to `host:port` with this*;
3. the client joins that server with the handoff instead of a signature;
4. that server checks it offline against the game's public keys and admits the
   player it names.

Nothing calls Elo Pros per hop, and nothing asks the player anything. It works
the same whoever the player is: a wallet, a login with no wallet, the
launcher, an agent. An MMO's zones, an arena behind a door in town, a match
server, an instance: one primitive.

## Why not a pass on every hop

A pass works for a hop too, but it needs Elo Pros on every zone line and a
player holding a site token. The server they are on has already checked who
they are; a handoff lets it say so to the next one directly. The pass stays
the way in.

## The text

One field a line, no trailing newline:

```
elo handoff
game: my-game
key: zone-1
player: 0x24445efddb08d4938e3e3627042b2cf4063d9092
from: zone1.my-game.example:7000
to: zone2.my-game.example:7000
id: h4nd0ff-0123456789ab
exp: 1800000060
carry: none
```

| field | what |
|---|---|
| `game` | the slug. A handoff never crosses games |
| `key` | the id of the server key that signed it |
| `player` | who the issuing server admitted: a `0x` wallet (lowercase), a Solana address, or an `acct_` id for a login with no wallet |
| `from`, `to` | server addresses, `host:port`, lowercase. `to` must name one server: two servers on one host differ by port |
| `id` | 16 to 64 of `A-Z a-z 0-9 - _`, fresh each handoff |
| `exp` | unix seconds; at most `TTL_MAX_S` (120) after issue, default 60 |
| `carry` | sha256 hex of the bytes the client carries beside it, or `none` |

**The token** is `base64url(text).base64url(Ed25519 signature over the text)`,
unpadded, at most 1024 characters.

## What the receiving server checks

In this order. Only a handoff that passes them all is marked used, so a
refused one can be retried.

| code | refused when |
|---|---|
| `malformed` | the token or its text is not exactly this shape |
| `wrong_game` | `game` is not this server's |
| `wrong_destination` | `to` is not an address this server answers to |
| `expired` | `exp` is 30 s (`SKEW_S`) or more behind now |
| `too_long` | `exp` is more than 150 s (`TTL_MAX_S` + `SKEW_S`) ahead of now |
| `unknown_key` | `key` is not in the game's current key set |
| `bad_signature` | the signature is not that key's over the text |
| `carry` | the carried bytes are over 4096, or do not hash to `carry` |
| `used` | this `id` was already admitted (kept until `exp` + 30 s) |
| `no_keys` | the door could not read the game's key set at all. Not the player's fault: let them join the usual way |

**Several processes behind one address** share one record of used ids (give
the door your own store); one process keeps it in memory.

**Keys.** A door either pins the game's key set or reads it from
`GET /api/store/serverkeys/{game}`. A door that reads it reads it again every
300 s, so a revoked key stops working within five minutes, and at once (at
most once a minute) when a handoff names a key it has not seen, so a rotated
key works on its first use. If Elo Pros cannot be reached, a door keeps the
keys it last read.

## The client's part

1. The server sends, in your own protocol: the address, the token and the
   carry bytes if any.
2. The client joins the address and presents the token where it would present
   a wallet signature or a pass.
3. If the new server refuses it or does not answer in about ten seconds, the
   client joins the usual way: back to the server it came from, or on to the
   new one with a pass. There is no "return ticket": a second token that also
   works would let one player stand in two places.

**For a seamless crossing** (an MMO's zone line), the client may connect to
the next server before it lets go of the last. The last server freezes the
player once it issues the handoff and drops them when the client disconnects
or the handoff runs out, so they are never live in two places.

## The server's part, issuing

- `player` is who you admitted. Never put in a name the client sent you.
- Save the player's state before you issue (an MMO's character, a match's
  roster): the next server loads it, keyed by `player`, or reads what `carry`
  points at.
- `carry` is for a small blob the client must not change in transit: a match
  id, a party, a position at the zone edge. Big state lives in your own store.

## What it is not

A handoff names one game, and only that game's servers admit it. Elo Pros
reads it as authority for nothing: it does not sign in to the site, link a
wallet, hold value or move any. A server key is the last link of the chain
invariant 8 names, and a handoff is that key telling the game's own servers
who a player is, which those servers already trust it to know.

## The SDKs

| | issue (the server they leave) | admit (the server they reach) |
|---|---|---|
| Python | `ServerKey.handoff(player, frm, to, carry=…, ttl=60)` | `HandoffDoor(game, [address], client=Client()).admit(token, carry)` |
| Rust | `ServerKey::handoff(player, from, to, carry, ttl)` | `HandoffDoor::with_source(game, &[address], Box::new(Client::new(…))).admit_now(&token, carry)` |
| JS (Node, Deno, Bun) | `await key.handoff(player, from, to, { carry, ttl })` | `await new HandoffDoor(game, [address], { client }).admit(token, carry)` |

Each also has `handoff_text`, `read_handoff` and `verify_handoff`, and a door
with pinned keys. `sdk/testdata/handoff.json` pins the text, the token and
every refusal, and all three SDKs and the meter answer it.

**Test one:** `POST /api/handoff/check {"token": "…", "carry_b64": "…"}`
answers what it says and whether it admits now against your registered keys.
It marks nothing used, so it cannot tell you `used`; only the receiving
server can.
