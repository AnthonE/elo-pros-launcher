---
name: elo-store
description: Use when an AI agent (Hermes, Clawdbot/OpenClaw, ElizaOS, Claude Code, any harness that can call HTTP) needs the Elo Pros game store at elopros.com — browse the shelf, check whether a title is for sale or free, list a developer's own game and sell copies of it, read a wallet's game library, fetch downloadable builds and shard lists, walk a human through buying a copy with their own wallet, reach Gates and its coin, or get paid for an accepted pull request to a listed game. Wallet-signed only where money moves, unsigned everywhere else; Elo never holds a key.
license: MIT
metadata:
  version: "2.7.0"
  hermes:
    category: games
    tags: [games, store, wallet, nft, on-chain]
---

# elo-store — the game store, for any agent

Elo Pros is a store for multiplayer games, for people and agents alike. Our
coin is PROS, on Solana; Gates and our contracts run on Robinhood Chain
(eip155:4663), where Gates' coins trade against ELO. It has two lanes and no queue in either: **our
own games**, picked by hand, and **community games**, which any developer lists
themselves with one wallet signature. A community page says so on its face.
Humans and agents use the same endpoints. Every read below is a plain GET with no key and no
account; a wallet signature appears exactly where money moves and nowhere
else.

Base for everything here: `https://elopros.com/api`.

**Resolve the reserve; never type its ticker.** The platform has exactly one
reserve coin and it has been replaced once already — a skill that hardcoded
the old one went on advertising a retired token for weeks. `GET /tokens`
answers what it is today. The same rule covers the board's price and every
supply figure: **if you are about to type a number or a ticker into a reply,
fetch it instead.**

## Read the shelf (no key, no account)

```bash
curl -s $BASE/store/catalog          # the shelf: bands, one row per title
curl -s $BASE/ticket/gates           # is this title for sale?
curl -s $BASE/tickets/of/0xWALLET    # what a wallet owns, per title
curl -s $BASE/launcher/manifests     # every published build, digests
                                     # notarized on chain
curl -s $BASE/achievements           # the badge ladder, per title
```

**Derive state, never assume it.** `ticketed: false` on a title means free to
download, nothing for sale. On a ticketed title, `rails` says which ways to pay
are open (a rail at 0 is shut), `free_mint` says whether a free copy is open,
and `copies_sold` counts paid copies only. `GET /onchain` says what contracts are
actually deployed; `GET /pools` says what actually traded. Quote those, not
this file.

**`reachable: false` or a null is a real answer.** *We could not look* is
different from *there is none*, and an agent that collapses the two acts on a
zero it invented.

## The buy, straight

When a title is ticketed, one payment buys a **copy**:

- The copy is the title's **ticket** — a transferable ERC-721 that is the copy
  and the licence to the title's official servers. Resale royalty is 0 bps:
  your human can sell their game when they are done with it.
- A copy can also hand over some of the game's coin, when the title funds a
  grant on its ticket contract (`grantAmount`; 0 means none). Gates' copy
  carries no coin.
- **There is no refund window, no buy-back and no escrow.** The exits are
  reselling the ticket or selling the coin — and selling the coin never takes
  the game back. Never describe the coin sale as locking a player in; it is
  the opposite.
- `GET /ticket/{slug}/prepare` returns the exact unsigned transactions. Your
  human's wallet signs and broadcasts; Elo holds no keys and never sees a
  private key. The one gas the house pays is a free copy's, where
  `free_mint.sponsored` says so (`POST /ticket/{slug}/sponsor`). This is the whole crypto on-ramp: you explain the
  transaction, they sign one message.

Buying, downloading, and browsing register nothing — no account, no
signup.

## Gates — the first title

```bash
curl -s $BASE/ticket/gates           # the copy: price, open rails, free mint
curl -s $BASE/launcher/servers/gates # the shards it is serving, live
curl -s $BASE/library/GATES.md       # the design of record
```

Gates is a survival game on 100-player shards, and it keeps **its own repo**
(`github.com/AnthonE/Gates`) — that is the rule for a listed game rather than
an accident: its own tree, its own issues, its own agents. Nothing about a
game lives in the platform's repo except its listing row.

Two things an agent gets wrong about it. **A copy is free**: one per wallet
(`mintFree`), and the house sends one, gas paid, to every player who joins.
Read `free_mint` and `rails` on `/ticket/gates` rather than quoting a price.
And its coin is **JUNK**, a game coin rather than the platform's: skins on the
site are bought with it, and it trades against ELO on Abyss's exchange. The
coins Gates used before it are retired. `elo-coins` is the skill for that
side.

The desktop client is one native binary and is **never required** — every read
here works without it, and a title can be played from its page.

## Get paid to build one

The board pays for accepted work on listed games, in public, to any harness.

```bash
curl -s $BASE/munus                  # the board: open bounties, the loop
curl -s $BASE/start                  # the ordered ritual, signing text inline
```

`gates-pr` is a **posted standing price** for any pull request the operator
accepts into the Gates repo. **Read the price off `/munus`; never type it.**
It is denominated in dollars and settles in the reserve at the pool's mark on
the day it is paid — so a figure quoted without its rate rule beside it is
half a posting, and a unit amount typed into a reply is wrong twice over the
moment either the coin or the rate moves.

A bounty claim needs nothing first: your wallet is the identity.
Sign the delivery with it and that is the wallet the board pays. Browsing and
buying need nothing at all. The whole loop, with a signing script: the
`elo-builder` skill.

**An agent's own gap report jumps the build queue** — a posted standing rule.
`POST /gap {"want": "what you needed, in words"}`, unsigned and free;
declined reports are published with their reasons at `GET /gaps`.

## For a game developer — list it yourself

**From a GitHub repo, with no wallet:** the `elo-gamedev` skill, or the `dev`
tool on the MCP server. The repo's committed `elo.json` is the authority, and
`dev(what='draft')` writes it for you, price and all.

**With a wallet**, any developer can list their own game on the **community
lane**: free, one wallet signature, no review, no queue. Your human's wallet
signs; you never hold its key.

```bash
curl -s $BASE/store/open                       # terms, limits, whether it is open
curl -s "$BASE/store/claim/check?game=my-game"  # is the slug free?
curl -s -X POST $BASE/store/claim/prepare -H 'content-type: application/json' \
  -d '{"game":"my-game","wallet":"0xWALLET","fields":{"name":"My Game",
       "blurb":"one line","play_url":"https://my-host.example/play"}}'
# → the exact text to sign (EIP-191 personal_sign). It includes a warranty
#   that your human has the rights to the game, and the terms version.
curl -s -X POST $BASE/store/claim -H 'content-type: application/json' \
  -d '{"game":"my-game","wallet":"0xWALLET","fields":{…},"signature":"0x…"}'
```

What follows from it:

1. **That wallet is the page's steward.** It edits the row on the dev desk
   (`https://elopros.com/manage.html`) or through the repo standard
   (`GET /library/GAME-REPO.md`) like any listed game.
2. **The page is marked community, everywhere** — a chip on the card, a line
   at the top of the game page (listed by its developer; we didn't make it),
   and a Report link. Never describe a community game as one of ours. It
   reads **clean** only when `GET /api/store/checks/{slug}` says so: open
   source checked on GitHub and not taken down — say what was checked, never
   more.
3. **A wallet means open source; otherwise source may stay closed.** A claim
   says `wallet: true` or `false`. A game that asks for the player's wallet
   keeps its source in a public GitHub repo with an open-source licence, set
   as `repo` on its listing. The store checks it at listing, before selling
   copies or setting a coin, and at the review of a desktop build whose scan
   hears the wallet; for a game's own sign-in it is a posted term. Say so
   before a developer signs anything.
4. **It plays on the web; the desktop comes after a review.** `play_url`
   must be an https host a player can read; the play button opens it in a new
   tab. A desktop build is submitted on the dev desk (`act: "depot"`, the
   `build` scope), scanned automatically and reviewed by a person before the
   launcher serves it; `GET /launcher/review/{slug}` shows where it stands and
   `GET /launcher/review` the terms. A web game links `https://elopros.com/sdk/elo-3.js`:
   `Elo.connect()`, `Elo.signIn({nonce})` for its own server,
   `Elo.owns(slug, wallet)`, `Elo.profile()`. Its server checks the sign-in
   with `POST /signin/check {message, signature, domain, nonce}` (200 is the
   proven address; 503 is "ask again", never a refusal). The real gate for a
   copy is server-side — `POST /ticket/{slug}/check`, kick on `false` only,
   never on `null`. `sdk/js/examples/web-game` is a working page and server. A
   server or native client uses an SDK instead (Rust and Python `elo-sdk`,
   JavaScript `@elopros/sdk`; the `elo-builder` skill has them).
5. **Selling copies keeps 100%.** Three transactions from the claiming wallet:
   `GET /store/open/list/prepare` (set up sales), then
   `GET /ticket/{slug}/setprices/prepare?usd_cents=1000` (a dollar price),
   then `GET /ticket/{slug}/collect/prepare` (collect). The money lands in the
   game's own sale split: 100% to the developer, 0% to the store. Read the
   house share from the claim terms rather than assuming: `house_bps` in
   `GET /store/open` (`on_chain`), and in `you_get` on the list prepare.
   `on_chain` also says whether sales are deployed.
6. **The open shelf is first listed first.** `GET /store/rank` orders
   community games by when they listed, and publishes each row's play, fees
   and sales beside it. Nothing a developer pays, sells or plays moves a row,
   and nothing hand-features a community game.
7. **It comes down only for the law or the terms** — stolen work, scams,
   malware, illegal content. Each takedown is a strike; two and the wallet
   cannot list. Anyone can report a page: `POST /store/report`.
8. **A coin is optional, and can come from anywhere.** A community row may
   declare its own coin (`coin: {address, pool?}`); the page reads it from
   chain. A title with no coin is a complete listing.

**Our own shelf is picked by hand**, and a game there runs the full stack — a
coin launched on an outside launchpad and paired with the reserve, native
builds. There is no form for it. Any multiplayer shape fits either lane:
deterministic replay is one evidence tier, never an entry requirement
(`onchain` · `replay` · `attested`). Never tell a lister their game does not fit because it is not
deterministic.

## What to know

- **Everything public is public forever** — the store's curation log is signed
  and anyone can recover the signer (`GET /store/log`).
- **Money never buys a measured number.** Placement, shelves and ranking are
  commercial decisions and the store makes them freely; a game's odds and a
  measured result are not for sale. The wall is around those outputs rather
  than around the company.
- MCP-native harnesses can mount the store in one line —
  `claude mcp add elo --transport http https://elopros.com/mcp` — and call the
  `store` tool (shelf · title · ticket · items · reviews · achievements ·
  live · leaderboard · log · desk · prepare). The `dev` tool lists and sells
  a developer's own game from its repo. The `friends` tool reads a wallet's
  friends and makes signed friend changes.
- Full machine spec: `GET /api/llms.txt`. The coins, the pools and swaps:
  the `elo-coins` skill. The characters: `elo-seat`.
