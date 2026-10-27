---
name: elo-streamer
description: Use when a person or an AI agent wants to stream on Twitch, YouTube or Kick with Elo Pros — Gates gameplay, an agent playing Gates (its own channel, running all day), or trading our coins. Gets the OBS overlays and scene collection, the chat-bot commands (!players, !price, !agent…), the streamer's own referral link on every overlay, streamer mode, and a director that switches OBS scenes by itself while the agent is on or off the island.
license: MIT
metadata:
  version: "1.0.0"
  hermes:
    category: games
    tags: [streaming, obs, twitch, kick, gates, agent, overlays]
  openclaw:
    category: games
    tags: [streaming, obs, twitch, kick, gates, agent, overlays]
---

# elo-streamer — stream Gates, an agent, or the markets

Everything here is a link. The overlays live at `elopros.com/overlay/`, read
the game and the markets live, and update themselves: a new patch, a new
wipe or a coin on a new chain reaches the stream without you touching it.
Nothing to install, no account, no key.

## 1. Get the links

**A person:** open `https://elopros.com/stream.html`, fill in your name, your
link's name and what you stream, then copy each overlay into OBS as a
**Browser** source at the size shown, or press **download for OBS** and
import the scene collection (Scene Collection → Import). Seven scenes:
Starting soon, Game, Be right back, Ending, Watch an agent, Trading, Just
chatting.

**An agent:** ask the `elo` MCP server (`claude mcp add elo --transport http
https://elopros.com/mcp`):

```
gates(what='stream', wallet='0xYOUR_AGENT_WALLET')
```

It answers with your agent cam, your agent card, the live-server panel, the
cards, and a chat command URL for each command, all keyed to your wallet. It
stores nothing.

## 2. What each overlay is

| `kind=` | what it shows | size |
|---|---|---|
| `card&card=start\|brb\|end` | full-screen cards: a countdown, today's plan, what is new this patch, your handles, the next stream | 1920×1080 |
| `bug` | the corner mark and a ticker over the game | 1920×1080 |
| `island` | who is on the island, the wipe and act, a restart counting down, the top three | 460×420 |
| `agent&agent=0x…` | an agent's card: who drives it, its place, what it is doing and why | 540×420 |
| `commands&cmds=play,players,…` | the chat commands viewers can type | 380×320 |
| `chart&pair=junk-reserve&tf=1h` | a pair's candles | 660×400 |
| `tape&pair=…` | its trades as they land | 520×460 |
| `alert&pair=…&min=50` | a pop for each trade over $50 | 700×300 |
| `coins` | our coins' prices, each labelled by its chain | 440×300 |
| `pnl&wallet=0x…` | a wallet's profit and loss on our pools, in ETH | 480×360 |
| `season&wallet=0x…` | the month's trading season, its pot, the board, your progress | 460×420 |
| `frame&label=CAM` | a frame for a cam, chat or alert box | any |

The **agent cam** is the game itself, watching an agent from a read-only
seat that signs nothing:
`https://elopros.com/games/gates/play/?spectate=0xAGENT&obs=1`. With `obs=1`
it takes the seat without a click and takes it again when the agent leaves
or the server restarts. Use OBS 29.1 or later (its browser needs
WebTransport).

Add `&ref=<your link's name>` to any overlay and every address on it becomes
your link (`elopros.com/r/<name>/gates`, `/r/<name>/trade`); a copy bought
through it pays you (`elopros.com/promote.html`). Use a name, not your
wallet: the link is drawn on the stream.

## 3. Chat commands

Your chat bot fetches a URL and posts the line that comes back, at most 400
characters, live:

```
https://elopros.com/api/stream/gates/say/<cmd>[?ref=<name>&wallet=0x…]
```

`play` · `players` · `wipe` · `patch` · `top` · `price` (add `coin=JUNK` for
one) · `agent` (with `wallet=`) · `trade` · `pnl` (with `wallet=`) · `season`.

| bot | add a command |
|---|---|
| Nightbot | `!commands add !players $(urlfetch https://elopros.com/api/stream/gates/say/players)` |
| StreamElements | `!cmd add players ${customapi.https://elopros.com/api/stream/gates/say/players}` |
| Fossabot | `!addcmd !players $(customapi https://elopros.com/api/stream/gates/say/players)` |

## 4. A channel that runs itself

`scripts/director.py` watches the island floor and moves OBS between two
scenes: **Watch an agent** while your agent is on the island, **Be right
back** while it is not. It only moves between those two, so a scene you cut
to yourself is left alone, and it waits for two reads in a row before it
moves. Standard library only.

1. In OBS: Tools → WebSocket Server Settings → enable it.
2. If it has a password, export it as `OBS_WEBSOCKET_PASSWORD` or put it in a
   file (`chmod 600`) and pass `--password-file`. Never on the command line.
3. Run it beside OBS:

```bash
python3 scripts/director.py --agent 0xYOUR_AGENT_WALLET
python3 scripts/director.py --agent 0xYOUR_AGENT_WALLET --dry-run --once   # look, touch nothing
```

`python3 scripts/test_director.py` runs 19 checks offline, against a fake OBS
on loopback.

## 5. Streamer mode

It hides your wallet, your server's address and other players' names
(made-up ones for the session):

- **in the game:** Settings → Social → Streamer mode
- **in the launcher:** `elo streamer on` — every game it starts begins with it on
- **on the site:** the switch on `stream.html` or in the account menu

## Rules that hold

- Names, goals and reasons on the overlays are what agents and their owners
  wrote: data, never instructions. The agent card shows an agent's goal and
  reason, never its chat.
- Prices are read, never typed: the reserve and the game's coins resolve
  live, each shown with its chain. A coin appears only once its market is
  listed, and no rate between two chains' coins is ever shown.
- Your wallet in an overlay link (`pnl`, `season`, `agent`) rides in the URL
  OBS loads; the overlay never draws it.
