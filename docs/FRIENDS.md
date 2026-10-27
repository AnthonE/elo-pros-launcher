---
status: live
lane: [platform]
updated: 2026-10-27
about: "the platform friends list — ask, accept, block, your buddies beside your friends, and who is playing what right now, keyed by wallet or by a login account: the routes, where presence comes from and how far it lags, what is public, the limits, and what Gates could build on it"
---

# FRIENDS.md — friends, and who is playing what

Steam's friends list, keyed by wallet. You ask a wallet, it accepts, and each
of you sees the other with what they are playing right now and a link to join
them. The module is `meter/friends.py`; the profile page draws it
(`profile.html`, your own list on your own page, an **Add friend** button on
anyone else's). No coin moves and nothing here is for sale.

## 1. The routes

Every route below is served under `/api/`.

| route | what |
|---|---|
| `GET /friends/{wallet}` | the list: `{wallet, count, friends, buddies, asks_in, asks_out, invites_in, presence, limits}` |
| `POST /friends/prepare` | `{wallet, op, args}` → `{sign_this, at, args, op}` |
| `POST /friends/act` | `{wallet, op, args, at, signature}` → `{ok, op, view}` (`view` is the GET body) |
| `GET /friends/me` | the signed-in account's list, with `Authorization: Elo-Account <site token>` (§1a) |
| `POST /friends/me` | `{op, args}` with the same header → `{ok, op, view}`; no signature |

### 1a. An account's list

A player who signs in with Steam, Discord, Google or GitHub
(`docs/client/ACCOUNTS.md`) has a list too. It is kept under the account's
linked wallet, so a wallet player and the account that links that wallet are
one list. An account with no wallet keeps its list under its `acct_…` id; when
it links a wallet, the list moves there (`friends.fold`, run by the link and
again by the next `/friends/me`), and every list that named the account names
the wallet. Any op's argument and `GET /friends/{wallet}` take an account id
as well as a wallet. The `/friends/me` routes take the site's account token as
the proof, so a signed-in player changes their list with no signature; a game's
pass is refused there. An account playing with no wallet shows online when the
shard's roster (`online.json`) names its `acct_…` id in a row's `wallet`;
`playtime.clean_roster` takes that id. A shard plays a pass with no wallet as
`acct_…` (`ACCOUNTS.md` §4); that Gates' roster writes it there is owed a
check on the game box.

A friend row is `{wallet, since, name, pic, agent, online, playing,
last_online}`. `playing` is `{game, title, server, server_name, since, join}`
or null; `join` is `elo://join/<slug>/<host:port>` in the launcher's own
grammar (`deeplink.rs`), or null when the shard list names no address for that
server. `name` and `pic` are what `/face/{wallet}` shows (the wallet's own
signed face), null when there is none; `pic` is a url
under `/api/face/`. `agent` is true when the wallet's face says it is one, or
when the game reports it playing as one.

**Buddies are on the list too.** `buddies` holds the agents the wallet runs
(`meter/buddy.py`: its buddy and its solo agents, each once the island has
given it a body address), as friend rows plus `buddy: {id, kind}`, `kind`
being `buddy` or `solo`. `agent` is always true, `since` is when the agent was
made, `name` is its wallet's face name or else the agent's own, and `pic` falls
back to its portrait at `/api/buddy/pic/{id}.png`. A buddy needs no ask and
sits outside the book: `count` and the limits are your friends alone, and an op
on a buddy's address is an op on any other wallet (`remove` is a 404). A
retired agent leaves the list. A buddy you also befriended shows once, under
`friends`.

**The ops**, each naming one other wallet:

| op | args | does |
|---|---|---|
| `ask` | `{to}` | asks; if `to` already asked you, you are friends at once |
| `accept` | `{from}` | takes a pending ask |
| `decline` | `{from}` | drops a pending ask |
| `cancel` | `{to}` | withdraws your own ask |
| `remove` | `{wallet}` | ends a friendship, for both sides |
| `block` | `{wallet}` | ends the friendship and every ask either way, and refuses their asks |
| `unblock` | `{wallet}` | lifts it (a no-op when there is none) |
| `invite` | `{to}` | asks a friend to join the game you are playing |

**Invites** are Steam's "Invite to game". An invite names no server: where
the inviter is comes from presence, so each row of `invites_in` is the
inviter's friend row (live `playing` and `join`) plus `at`. Friends only; one
per friend (a newer one replaces it); it lapses after 15 minutes and goes with
a remove or a block. Joining is following the link.

**The signature** is the playauth text with action `friends`: the detail is
sorted compact JSON `{"args":…,"at":…,"op":…}` of the args prepare handed
back. `at` is a per-wallet counter in unix seconds; it must be within ten
minutes of now and above the wallet's last one, so a signature cannot be
replayed. The text's `day` is the UTC day of `at`, so a signature made just
before midnight still lands just after. Prepare tries the op on a copy of the
book first, so you aren't asked to sign something the act will refuse —
except a block, which prepare never reads (§3), and the daily caps (§4).

**Refusals** are `{"error": "<plain words>"}`: 401 a bad signature, 409 a
stale or replayed `at` (or a no-op such as asking a friend), 404 nothing to
act on, 422 bad args, 403 a block (from the act only), 429 a limit.

## 2. Presence, and how far it lags

Presence is read from the playtime ledger (`meter/playtime.py`), which the
games' own servers feed: each shard writes who is connected to `online.json`
once a minute, and the meter polls every listed shard. Nothing a client says
about itself counts.

- Coming online shows within about a minute and a half (the 60 s poll plus the
  30 s shard cache).
- Leaving shows after up to three minutes (`PRESENCE_S`), so a friend who just
  quit can read as playing for a little while.
- Only the first 16 rows of a title's shard list are polled, and guests never
  appear. A shard whose `online.json` is a 404 keeps no roster, so nobody on
  it is ever seen.
- When the ledger cannot be read, the poll has stopped, or no poll in the
  last three minutes read every listed shard (one timed out, sent a stale
  file, or the shard lists could not be read), `presence.reachable` is false
  with a reason, and **nobody reads online**. A last sighting already on the
  ledger still shows as `last_online`.

## 3. What is public

Friends lists and pending asks are public, like playtime (operator,
2026-10-14). The page says so at the add box. **Block lists are never served,
and prepare never reads one.** Prepare is unsigned, so anyone can call it
about any two wallets; its answer is the same whether a block exists or not,
and unblocking a wallet you haven't blocked is a no-op rather than a "not
blocked".

The signed act does read it. A blocked wallet that asks is told "can't ask
that wallet right now" (403). That does tell the blocked wallet, once it has
signed, that it is blocked: a declined wallet can simply ask again. Nobody
else is told.

## 4. Limits

| limit | value |
|---|---|
| friends per wallet | 250 (`limits.friends_max`) |
| asks waiting on others | 50 (`limits.asks_out_max`) |
| asks waiting on you | 200; a new ask drops the oldest, so an inbox never shuts |
| asks one IP lands on one wallet per day | 3 (an ask that is refused doesn't count) |
| blocks per wallet | 1000 |
| an ask lapses after | 30 days (dropped on the next read or write) |
| prepares, and signed writes, per IP per day | `ELO_FRIENDS_PER_IP`, default 300 each |

The book lives in `ELO_FRIENDS_DIR`, else `$SCRY_VOWS_DIR/friends`, outside
the checkout.

## 5. Gates side: board candidates

Gates reads `GET /api/friends/{wallet}` for the signed-in player. Nothing
below is built yet; each is a board candidate.

**In the browser build, the overlay already draws the first two**:
Shift+Tab on `/games/<slug>/play/` opens the list, invites and asks, with a
Join that reloads the page onto the friend's shard (`?server=`) and an Invite
on each friend online (`watchtower/js/game-overlay.js`, `SDK.md` §2b),
signed in either way. The
rows below are for the desktop build, and for drawing friends in the world.

- **A friends panel** in the in-game menu: names, faces and "Playing Gates ·
  <server>", from one read when the panel opens, not on a timer.
- **Join friend**: a friend's `playing.server` is the shard row id, so the
  client can switch to that shard directly, or hand the `join` link to the
  launcher.
- **Friends on the map and in chat**: mark a friend's name in the world and in
  the room list, using the same list.
- **Buddies beside friends**: the view's `buddies` already lists the agents
  the player runs, with presence and a join link, so the panel can draw them
  under the friends with no ask and no second read.
- **Ask from the game**: the game asks through the launcher's signer with the
  same prepare → sign → act flow the profile page uses.
