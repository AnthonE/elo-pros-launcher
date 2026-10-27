---
status: design
lane: [platform]
updated: 2026-10-25
about: "guilds, voice and game lobbies on Elo Pros, so a multiplayer-only store keeps its players' groups and their talk instead of losing them to Discord: what Steam ships, what we already have (the hive's rooms, friends and presence, the launcher's join link, trade chat), the guild as a signed roster with its own room and voice, lobbies that join a server together, self-hosted voice, and the calls only the operator can make"
---
# GUILDS.md — guilds, voice and lobbies

Operator, 2026-10-24: *"we need an easy trader chat. could use hive but needs
to be seamless and not bother the user… wouldnt be crazy for us to get a voice
chat game lobby. hmm maybe the whole elo pros website needs guilds?? … we are
multiplayer only. guilds are key to these games and so is communication. no
reason to lose them to discord. we own the whole stack"*.

**Amends** the 2026-08-01 sentence *"the hive carries the social layer and scry
builds no new social organ … guilds point at the covenant"* (`SENTENCES.md`).
The hive still carries the talk. A guild is a new organ, because the covenant
is the wrong shape for one (§2).

## Status — trade chat, guilds and lobbies are built; voice is not

**G0, trade chat:** the terminal's **chat** button opens the hive's `trade`
room beside every pane (`js/tradechat.js`). It needs no sign-up and no wallet
prompt: the first message makes a chat key in the browser. It is the pattern
everything below reuses: **the hive for text, keys made where the person is, a
wallet signature only when they ask for something a wallet should vouch for.**

**G1, guilds** (built 2026-10-24, on the §8 recommendations):
- the roster: `meter/guilds.py`, with `test_guilds.py`;
- the rooms: `meter/guild_rooms.py`, the `elo-guild-rooms` pm2 app, disarmed
  until `ELO_GUILD_ROOMS=1` (`HIVE-RUNBOOK.md` §1a);
- the pages: `guilds.html` (every guild, yours, start one) and `/g/<TAG>` (one
  guild), under **community**; `chat.html?guild=<TAG>` opens its room, and a
  profile shows a wallet's guilds under its name;
- agents: the `guilds` MCP tool;
- invite links, the share card behind `/g/<TAG>`, recent activity on the page,
  and the guilds' board in the trading season (§9);
- the tag in every game (§3a): `wear`, `GET /guilds/tags`, `guild` on the
  face, and `guild_tags` / `guilds_of` in the three SDKs (2026-10-25);
- a guild's Discord link on its page (§3b).

**G2, lobbies** (built 2026-10-25, §5): `meter/lobbies.py`, with
`test_lobbies.py`. One change from §5: every act is one wallet signature
(`playauth` action `lobby`) rather than the chat key, and `join` marks a member
ready, so a member signs once. Launch needs every member ready and lands on a
row of the title's own server list; `GET /lobbies/pick/<slug>?party=<n>` is
quick join without a lobby, the first row in the developer's order that is up
and has room.

What the build settled that this plan left open:
- **The routes are the friends shape** (`/guilds/prepare` then `/guilds/act`),
  not `/guilds/{tag}/message`, so the page, the MCP tool and an SDK reuse one
  signing path.
- **A kick sticks.** The kicked wallet stays out of an open door until an
  officer invites it back.
- **A wallet is in at most five guilds.** Guilds span games, so one each would
  be too few.
- **A room's id is salted with a secret on the box**
  (`<guilds dir>/room.salt`), and a page is told it only after the house has
  made the room. An id anyone could work out from the public log is one anyone
  could make first on a relay where anyone may make a room.
- **Emblems are ours: a shape and a colour.** A PROS the founder holds is not
  built yet.

## 1 · What Steam ships, and what a multiplayer-only store needs

| Steam | what it is | ours |
|---|---|---|
| Friends, presence, "join game" | who is on, what they play, one click into their server | built: `meter/friends.py`, `FRIENDS.md`; the launcher owns `elo://join/<title>/<host:port>` |
| Steam Chat: group chats with text channels | a group's own rooms | the hive: NIP-29 groups on our buzz relay (`HIVE-RUNBOOK.md`) |
| Voice channels in group chats, party voice | talk while you play | **none** (§4) |
| Steam Groups / clans | a named group with members, officers, announcements, events | **none**; `EloCovenant` is not one (§2) |
| Lobbies and invites (Steamworks matchmaking) | a party forms, readies up, lands on one server together | half: the join link exists, the party does not (§5) |
| Overlay | chat and voice over the game | the launcher, later (§6) |

Discord wins on two things Steam never fixed: **voice that just works** and
**a group's home with its rooms and its people**. Those are §3 and §4.

## 2 · Why the covenant is not a guild

`EloCovenant` (deployed as `ScryCovenant`) records a fleet swearing one oath:
joining costs gas, leaving is a public renouncement that can **never** be undone
(the same wallet cannot re-swear), and there is no officer, invite or kick. That
is right for what it is for, and wrong for a guild, where people leave, come
back, get promoted and get removed every week. A guild may still *also* swear a
covenant as its charter. That is optional, on chain and permanent, and a
separate act.

## 3 · The guild: a signed roster, its own rooms, its own page

**Recommendation: an off-chain roster, every change signed by the wallet that
makes it.** It is the `friends.py` shape: free, instant, and checkable by anyone.
Authority is a chain of signatures (invariant 8): the founder's, then the
officers' they appoint, at most two links. No contract, so no gas and no weld:
a guild that wants a treasury opens a Safe, and its page shows the address.

- **Create:** a name, a 2–5 letter tag, an emblem (one of ours, or a PROS the
  founder holds), and one wallet signature. Tags are first come, first served;
  a takedown is for the law or the posted terms.
- **Join:** open (anyone), apply (an officer accepts) or invite-only. Each is one
  signature from the joiner and, where needed, one from an officer.
- **Roles:** leader · officer · member. A leader passes leadership with one
  signature; officers invite, accept, kick and pin. Leave and come back freely.
- **Rooms:** each guild gets a closed NIP-29 group on the hive, with
  `general` and `officers`, and room membership follows the roster. The house
  key, already a relay admin (`HIVE-RUNBOOK.md` §1), seats and unseats members
  the way the guest-list cron seats linked keys today. A member's chat key is
  the one trade chat made, linked to their wallet.
- **Page:** `/g/<tag>` on the store, under **community**: emblem, the line,
  members with presence (what each plays now, from friends' presence), the
  guild's games, its room, and **join their server** when members are in one.
- **Rank nothing by money.** A guild's page may show its members' game scores
  where a title publishes a board (`GATES.md` §7a). No guild leaderboard by
  holdings, spend or volume (invariant 1's test).
- **Bots and agents** join like anyone (the SDK and MCP get a `guilds` tool).

Routes (`meter/guilds.py`):
- `GET /guilds`, `GET /guilds/{tag}`, `GET /guilds/of/{wallet}`
- `POST /guilds/prepare {wallet, op, args}` (the text to sign), then
  `POST /guilds/act {wallet, op, args, at, signature}`. The ops are create,
  edit, pin, join, leave, disband, accept, decline, invite, kick, promote,
  demote, lead, link, unlink and wear.
- `GET /guilds/tags?wallets=` (§3a).

The roster is a ledger in `scry-data`, each row carrying its signature, and is
replayable by anyone from `GET /guilds/{tag}/log`: the book is the fold of the
log (`guilds.replay`, and `python3 guilds.py check` compares them).

## 3a · The tag in every game

Operator, 2026-10-25: *"the fact that we can bake games right into the SDK is
insane. we are multiplayer open source only… we just make some standard for
multiplayer games"*. Steam's groups never reached inside a game, and Discord
cannot. Here the roster is public and keyed on the wallet the game already
signed in, so any listed game can show it.

- **Inputs:** a member signs `wear {tag}` to choose which of their guilds a
  game shows, or `wear {tag, on: false}` to show none. With no choice, it is
  the guild they joined first.
- **Outputs:** a badge, `{tag, name, color, shape, role, page}`, or null.
  `GET /guilds/tags?wallets=` answers up to 100 wallets in one read, and
  `/face/{wallet}` carries the same badge as `guild`, so a game that already
  reads faces gets tags with no new call.
- **Who can verify:** anyone. The worn choice is a signed row in the same log
  (`replay` rebuilds it).
- **The SDK:** `guild_tags` and `guilds_of` in Rust, Python and JS, tested on
  `sdk/testdata/guilds.json`, which `meter/test_sdk_fixtures.py` rebuilds
  from this module.

Gates is the first game to draw it. Guild halls, guild chat in game and guild
against guild are built on `guilds_of` by each game.

## 3b · Discord: link it now, replace it later

Operator, 2026-10-25: *"im trying to slow vamp. eventually u just dont use
discord and steam and forget about them"*. A crew that already has a Discord
keeps it: `create` and `edit` take `discord`, an invite link
(`discord.gg/…` or `discord.com/invite/…`), and the guild page links it. Our
side wins on what Discord cannot do: the tag in game, landing on one server
together (§5), and voice in the client (§4, §6).

## 4 · Voice: self-hosted, joined with the chat key

**Recommendation: LiveKit**, an open-source (Apache-2.0) WebRTC server, self-hosted
on our box, behind `voice.elopros.com`. It is the server Discord-style voice is
built on, it ships its own TURN for players behind strict NATs, and it has web,
Rust and mobile clients. We run it; nothing about a call touches a third party.

- **Joining without friction:** the page asks the meter for a room token by
  signing a short challenge **with the chat key** (no wallet prompt), and the
  meter checks the key against the guild roster or the lobby. Open voice rooms
  (a game's `lfg` voice) take any key.
- **The rooms:** each guild gets voice channels (General, plus one per game it
  plays); each lobby gets one while it lasts (§5); and the open `lfg` rooms
  are one per title.
- **Defaults:**
  - **Microphone:** push-to-talk on the web, voice activity optional, and you
    mute or deafen yourself.
  - **Moderation:** officers can server-mute in their own guild's rooms.
  - **Data:** no recording, no transcript and no stored audio. Data
    minimisation is a standing rule (`DO-NOT-STRIKE.md`), and voice is where
    it matters most.
- **Cost:** bandwidth, roughly 50 kbps per speaker per listener through the
  SFU. A guild of 20 in one channel is a few Mbps. Meter it like hosting, and
  watch it before any rule about it is written.
- **CSP:** `connect-src` gains `wss://voice.elopros.com`. Media is UDP, not
  under CSP, and the no-analytics rule is untouched.

## 5 · Lobbies: get a party onto one server

A **lobby** is a short-lived party. A friend or guildmate opens one, the others
join from a link or their friends list, and everyone sees who is in and who is
ready. The lobby has its own text room and voice room. **Launch** sends every
member the same `elo://join/<title>/<host:port>` (the launcher already owns it;
the web build gets the same link), so the party lands together.

- **Routes:**
  - `POST /lobbies` to open one
  - `GET /lobbies/{id}`
  - `POST /lobbies/{id}/act` for join, ready, leave and launch
  - a server-sent stream for the room's state
- **Signing:** every act is signed with the chat key, never a wallet prompt.
- **Lifetime:** a lobby closes when its last member leaves, or an hour after
  launch.
- **A lobby picks a server from the title's shard list** (`launcher.py`). The
  queue (`queue`, `unqueue`) is rated matchmaking: parties wait by their
  rating (`meter/ratings.py`) and land on one server together (`meter/lobbies.py`).

## 6 · The launcher

The desktop app already reads the hive directly (`LAUNCHER.md` §10q). It gets
the guild rail, lobbies and voice through LiveKit's Rust SDK, and a small
always-on-top overlay for voice and the lobby while a game runs. It comes after
the web, because the web is where a guild page is shared.

## 7 · Order

| # | what | depends on |
|---|---|---|
| G0 | trade chat | ✅ built |
| G1 | guilds: the roster and its signed acts, the guild's hive rooms, `/g/<tag>`, the profile's guild line, MCP | ✅ built; the rooms wait on the first armed run (`HIVE-RUNBOOK.md` §1a) |
| G1b | the tag in every game: `wear`, `/guilds/tags`, the face's `guild`, the SDK calls, Gates draws it | ✅ built (§3a) |
| G2 | voice: LiveKit on the box, tokens for chat keys, guild channels, the open `lfg` rooms | a host for it (§4) |
| G3 | lobbies: party, ready, launch together, lobby voice | G2 for voice; text works without it |
| G4 | the launcher: guild rail, lobbies, voice, the overlay | G1–G3 |

## 8 · The calls only the operator makes

G1 was built on the recommendations for calls 1, 3, 4 and 5 (operator,
2026-10-24: *"work on the guild features we talked about"*). Each one is a
change, not a weld: the roster is off chain, the tag rule and the page's place
are code. Call 2 waits on G2.

1. **The roster:**
   - **Recommended:** off chain, signed (§3).
   - **Or:** a new `EloGuild` contract (gas to join, welded rules).
2. **The voice host:**
   - **Recommended:** LiveKit on our own box.
   - **Or:** LiveKit Cloud (paid, a third party in the call), or no voice yet.
3. **Tags:**
   - **Recommended:** first come, first served.
   - **Or:** a small fee to found a guild. The fee buys a name, never a rank,
     so it passes invariant 1's test.
4. **Where guilds live:**
   - **Recommended:** under **community**.
   - **Or:** a sixth door on the spine (store · library · community · mods ·
     developers).
5. **Whether a game's guild perks are allowed:** cosmetic only, granted by the
   game, never sold by us. That keeps it inside the no-pay-to-win standard,
   which binds what we sell.

## 9 · Squads, checked (2026-10-24)

Operator, 2026-10-24: check the squads feature trading apps now ship, and make
sure we are not lacking. Squads are invite-only groups with a shared profile
and their own leaderboard. A squads trading competition scores a squad by its
members' summed P&L and splits each prize by each member's positive score; it
bans reshuffling members to game the score. Clan systems in other trading apps
are the same shape. The sources are the platforms' own pages and competition
terms, plus press coverage. Their pages refuse automated reads, so the squad
screens themselves were not seen.

| squads | ours | |
|---|---|---|
| invite-only, joined from a link | `link` and `/g/<TAG>?join=CODE`; the code never reaches our book or log, only its hash | built |
| a link posted in a group chat | `/g/<TAG>` unfurls as the guild's card (emblem, name, line, size) | built |
| a shared profile | name, tag, line, emblem, games, notice | had; an uploaded picture is not built |
| a leaderboard of squads | the guilds' board in the trading season (`FEE-REWARDS.md` §5) | built |
| ranked by summed P&L | ranked on place across five metrics, never summed size, so a bankroll buys nothing | different on purpose |
| no reshuffling | a member counts only from the day they joined | built |
| 2–20 members, one squad each | up to 500, in up to five guilds | ours is a game guild |
| a member feed | `recent` on the page: who joined, left, was promoted or removed | built; a feed of members' trades is not |
| a squads competition with a prize pool | none: the season pot pays the wallet board | **the operator's call** |

**The open call.** A guild prize would come out of the season pot or a new
one. It would be split the way squads split theirs, by each member's share of
the guild's result, and only to members who joined before the month began. The
board is ready for it. The money is not ours to set here (`FEE-REWARDS.md`).

