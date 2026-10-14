---
status: design
lane: [platform]
updated: 2026-10-08
about: "the rooms server — the open-source multiplayer server a ported casual game runs on, built by the first port and reused by every later one: what it must do, the wire, how it plugs into the shard list and the ticket"
---
# ROOMS-SERVER.md — the server a ported casual game runs on

> **Why this exists.** Nearly every game made in an AI game maker is a
> *rooms* game: casual, social, co-op, party or turn-based, a few to a few
> dozen players in a room (`PORT-BOUNTY.md` §3). The maker carried the
> multiplayer, so a port has to bring its own. This is the server it brings.
>
> ⚠ **Nothing here is built.** The first port builds it (`NOW.md` row 12) and
> lifts it into its own public repo. This page is what that port is checked
> against, so the second port starts from a server and not from a blank file.

## 0 · What it is, in one screen

- **One process, one command.** `rooms-server --config rooms.toml` on a box the
  game's creator controls. No database to install for v1: state saves to
  files.
- **Authoritative.** The server runs the game's rules on a fixed tick. Clients
  send inputs and draw what the server says. A client never decides an
  outcome.
- **Rooms, not one world.** A lobby, rooms with a cap, and a new room when one
  fills (what a game maker calls an *instance*). Each room is a shard to the
  launcher (§4).
- **Browser first.** WebSocket over TLS for v1, because every ported game runs
  in a browser and WebSocket works everywhere. WebTransport, which Gates
  uses, is the upgrade path rather than the starting point (§7).
- **Shared game code.** The rules the server runs are the same module the
  client imports for prediction. That is the single biggest saving in a port,
  so the kit's language follows the game's (§6).

## 1 · The shapes it covers, and the ones it does not

| shape | covered | why |
|---|---|---|
| casual / party / co-op, 2–32 in a room | **yes** | the target |
| turn-based (cards, board, tactics) | **yes** | a tick of 0: the server steps on input |
| persistent world (what you dig stays dug) | **v2** | needs §5's save store to hold world state, not just player state, plus interest management |
| arena FPS | **no** | needs lag compensation and a high tick; a different server |
| fighter / 1v1 | **no** | needs rollback; a different server |
| survival shard | **no** | Gates is the reference for that shape |

**Never tell a creator their game does not fit.** A shape this server does not
cover gets its own server when its first game arrives (`PORT-BOUNTY.md` §3).

## 2 · Rooms

- **Lobby.** A client connects, lists open rooms, and joins one or asks for a
  new one. Quick-play joins the fullest room that is not full.
- **Room lifecycle:** `open` → `playing` → `ended` → removed. A game declares
  whether a room takes joins while `playing` (drop-in) or only while `open`
  (match).
- **Cap and spill.** `max_players` per room comes from config. When every room
  is full, quick-play makes a new one.
- **Join by code.** Every room has a short code a player can share. This is
  the browser's version of a join link (§4).
- **Reconnect.** A client that drops gets the same seat back for
  `reconnect_s` seconds (default 30) and resumes from the server's state. A
  refresh puts you back where you stood. The maker the game came from promised
  that, so a port keeps it.

## 3 · The wire

JSON messages over one WebSocket per client, for v1. Binary is an
optimisation for later, and the message names do not change when it lands.

| from | message | carries |
|---|---|---|
| client | `hello` | protocol version, game slug, optional `auth` (§5) |
| server | `welcome` | player id, server tick rate, the room list |
| client | `join` | room id, room code or `quick` |
| server | `joined` | room id, seat, full snapshot |
| client | `input` | client tick, input sequence number, the game's input object |
| server | `snapshot` | server tick, last input seq applied per player, the game's state (full or delta) |
| server | `event` | one-off happenings the game names (a pickup, a round end) |
| client | `leave` | — |
| server | `kicked` | reason (`not_entitled`, `full`, `version`) |

Three rules, because they are the ones a hand-rolled server gets wrong:

1. **Inputs carry a sequence number and snapshots acknowledge it.** That is
   what lets a client predict its own movement and correct cleanly. Without
   it, a client either waits a round trip to move or rubber-bands.
2. **The server never trusts a position from a client.** It takes an input
   (e.g. "move up-left") and moves the player itself.
3. **Every message is bounded.** Inputs are rate-limited per client and every
   queue has a cap with a stated overflow (drop oldest, or kick). An unbounded
   queue is how one bad client stalls a room.

**Tick.** 20 Hz by default and configurable. Clients interpolate other players
between snapshots (about 100 ms behind) and predict their own avatar.

## 4 · How it plugs into Elo

- **The shard list.** The server writes an `elo-shardlist-v1` document
  (`LAUNCHER.md` §6), one row per room, with `addr`, `max_players` and a
  `status_url`. The game drops it where the origin serves it
  (`GET /api/launcher/servers/{slug}`), so the launcher's Servers window and
  `elo servers <title>` both see it.
- **Status.** `GET /status.json` answers
  `{"players":3,"max_players":8,"tick":123456}` for the launcher to poll. An
  unknown count is left out, never sent as `0`.
- **Join links.** `elo://join/<title>/<host:port>` works on desktop through the
  launcher (`LAUNCHER.md` §6b). In the browser, the room code is the link:
  `<play_url>?room=<code>`.
- **The ticket, for a paid title.** At join and on an interval, the server
  checks entitlement exactly as `TICKET.md` §2b says. At join it reads
  `GET /api/ticket/{game}/of/{wallet}`. On the interval it sends
  `POST /api/ticket/{game}/check` with the roster. **It kicks on
  `entitled: false` and never on `entitled: null`**: a failed read must not
  kick a paying player. A free title skips all of this.
- **The board.** Nothing to wire. The kit is open source, so improvements to it
  land as pull requests like any other work.

## 5 · Identity and saves

- **Guest by default.** A free game may let anyone in with a random id.
- **Wallet when the game wants one.** The client signs a SIWE (EIP-4361)
  message and the server verifies the signature. It never holds a key. On
  desktop the signature comes from the SDK's `prove` (`sdk/PROTOCOL.md`). In
  the browser it comes from the player's own wallet through the web SDK:
  `Elo.signIn({nonce})` in `elo-1.js` (`SDK.md` §5b). The launcher relay does
  not carry it — no SIWE rides the relay (`SIGN-IN.md` §3). A paid title
  requires it, because the ticket check needs an address.
- **Saves.** `load(player)` on join and `save(player)` on leave and on an
  interval, keyed by wallet (or guest id), written to one file per player
  under the server's data dir. The game decides what carries over. Anything
  that must be holdable or tradeable belongs on chain, not in a save file.

## 6 · Language

**The kit's language is the ported game's language, so the rules run on both
sides from one module.** Games made in browser-first AI makers are
JavaScript/TypeScript, so v1 is expected to be **TypeScript on Node or Bun**.
That is a choice for the *game's* server and not for this repo's service,
which stays Python with no Node. If a later port is Rust, the wire in §3 is
the contract a Rust server speaks, and Gates' crates are where to start.

## 7 · Checked when the first port delivers it

- [ ] **Two clients in one room see each other move**: two browsers, or two
      machines.
- [ ] **It is playable at 150 ms round trip with jitter.** Run the server
      behind a delay shim. Your own avatar responds at once; the others move
      smoothly.
- [ ] **A refresh resumes**: same seat, same state, inside `reconnect_s`.
- [ ] **A full room spills** to a new one on quick-play.
- [ ] **A flood is refused.** A client sending 10× the input rate is capped or
      kicked, and the room's tick holds.
- [ ] **`status.json` and the shard list validate** and the launcher shows the
      rooms (`elo servers <slug>`).
- [ ] **Ticket path, if the title is paid**: a non-holder is refused at join, a
      holder who sells mid-session is kicked on the next sweep, and an
      unreachable origin kicks nobody.
- [ ] **It lives in its own public repo** with a licence, a README that starts
      it in one command, and the port that uses it linked as the example.

**Later, and only when a game needs it:** binary messages, WebTransport,
interest management for big rooms, a real database, and the `replay` evidence
tier (a deterministic game can log its inputs as a sealed segment,
`ACHIEVEMENTS.md`).
