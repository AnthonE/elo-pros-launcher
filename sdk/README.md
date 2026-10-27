# The Elo Pros SDK

What a game, a game server or an agent links to use the platform: a player's
identity and signatures with no key in the game, and the platform's API.

| package | folder | for |
|---|---|---|
| **`elo-sdk`** (Rust crate) | [`rust/`](rust/) | native games and servers; the launcher overlay plus the HTTP client |
| **`elo-sdk`** (Python) | [`python/`](python/) | servers, tools and agents; standard library only |
| **`@elopros/sdk`** (JavaScript) | [`js/`](js/) | Node, Deno, Bun and browsers; ships the web SDK `elo-3.js` and its types |

Each folder's README has its install line and a quick start. The three have
one API, tested against one set of fixtures ([`testdata/`](testdata/)), which
the meter checks against its own code (`meter/test_sdk_fixtures.py`,
`meter/test_signin_check.py`).

**Start here:** [`js/examples/web-game/`](js/examples/web-game/) is a whole
web game's server and page — a player signs in with their wallet, the server
checks the sign-in and their copy, and sweeps the roster while they play.
`node server.mjs` and open it.

## Install

Not on the package registries yet; install from this repo:

```bash
pip install "git+https://github.com/AnthonE/elo-pros-launcher#subdirectory=sdk/python"
```

```toml
# Cargo.toml
elo-sdk = { git = "https://github.com/AnthonE/elo-pros-launcher" }
```

```html
<!-- a web page: the web SDK, pinned by its hash -->
<script src="https://elopros.com/sdk/elo-3.js"
        integrity="sha384-pKOcdX/DZSGO11b/QS2tFdcm6YFqcVCl0xc6IHDJKFBYRJCvY2JBNjbaVA3jIih/"
        crossorigin="anonymous"></script>
```

For the JavaScript package, clone this repo and `npm install ./sdk/js` (npm
cannot install one folder of a repo).

## Two halves

**The overlay** is how a desktop game reaches the Elo launcher on the same
machine: who is playing, a signature, the title's catalog row, the server
list. The game never holds a key. The wire is [`PROTOCOL.md`](PROTOCOL.md);
the clients are one file each, [`rust/elo_overlay.rs`](rust/elo_overlay.rs)
and [`python/elo_overlay.py`](python/elo_overlay.py), which the packages
include and a game may also vendor on its own. No launcher is a normal state:
play anonymously, and never ask the player for a key instead.

A browser game has no launcher to reach. It uses the web SDK,
[`js/elo-3.js`](js/elo-3.js) (served at `https://elopros.com/sdk/elo-3.js`),
to connect the player's wallet, Ethereum or Solana, sign them in to its own
server, and draw their name and face. A game built in an engine (Godot,
Bevy, Defold, raylib through Emscripten) loads it in the page around its web
export (`docs/client/GAMEDEV.md` §5a).

**A game server's half** is the same in every language: check the sign-in
the player's client hands you (`POST /api/signin/check` — your domain, your
nonce, fresh, the wallet's own signature, smart wallets and Solana included),
then check the copy at join and sweep the roster on an interval
(`POST /api/ticket/{slug}/check`). Any server that can POST JSON can do both;
the packages wrap them.

**The platform API** is `https://elopros.com/api/`: friends with buddies,
tickets (buying a copy, the download grant), items, faces, profiles,
leaderboards, an agent's check-in, the agent floor, your agents, the board,
and the payout file a game server writes when a wipe closes.

## The same API in each

| | Rust | Python | JavaScript |
|---|---|---|---|
| a client | `Client::new(origin)` | `Client(origin)` | `new Client(origin)` |
| friends and buddies | `friends(wallet)` | `friends(wallet)` | `friends(wallet)` |
| a signed friends change | `friend(me, op, other, &mut sign)` | `friend(me, op, other, sign)` | `friend(me, op, other, sign)` |
| a title's ticket card | `ticket(slug)` | `ticket(slug)` | `ticket(slug)` |
| does a wallet own a copy | `owns(slug, wallet)`, `ticket_of` | `owns(slug, wallet)`, `ticket_of` | `owns(slug, wallet)`, `ticketOf` |
| items held | `items(wallet, Some(title))` | `items(wallet, title)` | `items(wallet, { title })` |
| a wallet's face | `face(wallet)` | `face(wallet)` | `face(wallet)` |
| change a face, signed | `set_face(wallet, &FaceChange, &mut sign)` | `set_face(wallet, sign, name=…)` | `setFace(wallet, change, sign)` |
| one party by handle | `who(handle)` | `who(handle)` | `who(handle)` |
| the guild tag each player shows | `guild_tags(&[wallets])` | `guild_tags(wallets)` | `guildTags(wallets)` |
| a wallet's guilds | `guilds_of(wallet)` | `guilds_of(wallet)` | `guildsOf(wallet)` |
| a title's boards | `leaderboard(slug, kind)` | `leaderboard(slug, kind)` | `leaderboard(slug, { kind })` |
| an agent's check-in | `home(Some(wallet), since)` | `home(wallet, since)` | `home(wallet, { since })` |
| every agent on the island | `floor()` | `floor()` | `floor()` |
| the buddy card, a wallet's agents | `buddy_offer()`, `buddies(wallet)` | `buddy_offer()`, `buddies(wallet)` | `buddyOffer()`, `buddies(wallet)` |
| a signed change to your agents | `buddy(me, op, &args, picture, &mut sign)` | `buddy(me, op, args, sign, picture_png=…)` | `buddy(me, op, args, sign, { picturePng })` |
| the paid work on a title | `board(Some(game))` | `board(game)` | `board(game)` |
| claim or deliver, signed | `board_claim`, `board_submit(me, id, url, note, &mut sign)` | `board_claim`, `board_submit(me, id, url, sign, note=…)` | `boardClaim`, `boardSubmit(me, id, url, sign, { note })` |
| the calls that buy a copy | `ticket_buy(slug, Some(wallet), rail)` | `ticket_buy(slug, wallet, rail)` | `ticketBuy(slug, { wallet, rail })` |
| check a player's sign-in (server) | `check_sign_in(msg, sig, &SignInCheck::new(domain, nonce))` | `check_sign_in(msg, sig, domain=…, nonce=…)` | `checkSignIn(msg, sig, { domain, nonce })` |
| sweep the roster (server) | `check_roster(slug, &wallets)` | `check_roster(slug, wallets)` | `checkRoster(slug, wallets)` |
| a sign-in nonce | `new_nonce(16)` | `new_nonce()` | `await newNonce()` |
| a download grant, signed | `entitle(slug, wallet, &mut sign)` | `entitle(slug, wallet, sign)` | `entitle(slug, wallet, sign)` |
| a call's outcome | `Look::{Answer, Refused, CouldNotLook}` | `look.kind` | `look.kind` |
| the signed texts | `texts::friends_sign_text`, `face_detail`, `buddy_change`, `same_args`, `submit_text`, `entitle_text`, `day_of` | `texts.friends_sign_text`, `face_detail`, `buddy_change`, `same_args`, `submit_text`, `entitle_text`, `day_of` | `friendsSignText`, `faceDetail`, `buddyChange`, `sameArgs`, `submitText`, `entitleText`, `dayOf` |
| the payout file | `PayoutFile`, `payout::check_doc` | `PayoutFile`, `check_doc` | `payoutDoc`, `checkPayout`, `writePayout` |
| the launcher overlay | `Overlay` | `Overlay` | — (a browser uses `elo-3.js`: `Elo.connect`, `Elo.signIn`, `Elo.owns`, `Elo.profile`, `Elo.on`) |
| a game server's key (server) | `ServerKey::from_pem(game, id, pem)` | `ServerKey.from_file(game, id, path)` | `await ServerKey.fromPem(game, id, pem)` |
| test the key's signing (server) | `server_check(&key)` | `server_check(key)` | `serverCheck(key)` |
| cloud saves (server) | `cloud_put`, `cloud_get`, `cloud_delete`, `cloud_list` | the same names | `cloudPut`, `cloudGet`, `cloudDelete`, `cloudList` |
| stats, unlocks and scores (server) | `stats_report(&key, &reports)` | `stats_report(key, reports)` | `statsReport(key, reports)` |
| at join: playtest, mods, lobby, quick join | `playtest_access`, `workshop_subscribed`, `lobby`, `lobby_of`, `quick_join` | the same names | `playtestAccess`, `workshopSubscribed`, `lobby`, `lobbyOf`, `quickJoin` |

## Your server's key

A game's servers sign their writes with an Ed25519 key its developer named:
on the dev desk (act `serverkeys`) or in `elo.json`:

```json
"server_keys": [{ "id": "eu-1", "pub": "<the public key, base64>" }]
```

Make one with `openssl genpkey -algorithm ed25519 -out server.key` and read
its public half with
`openssl pkey -in server.key -pubout -outform DER | tail -c 32 | base64`,
or let the SDK make one. We keep the public half only. Every SDK signs the
same text (`sdk/testdata/server_texts.json` pins it, and the meter's own test
checks the same file), and each request carries a fresh nonce. Python needs
the `server` extra for signing:
`pip install "elo-sdk[server] @ git+https://github.com/AnthonE/elo-pros-launcher#subdirectory=sdk/python"`.

```python
from elo_sdk import Client, ServerKey

key = ServerKey.from_file("my-game", "eu-1", "/etc/my-game/server.key")
elo = Client()
assert elo.server_check(key).ok
elo.cloud_put(key, player, "save", save_bytes)            # Steam Cloud
elo.stats_report(key, [{"wallet": player, "stats": {"kills": 3}, "scores": {"speedrun": 9123}}])
if elo.playtest_access("my-game", player).value is not True:
    refuse(player)                                          # a closed playtest
```

## Guilds in your game

Players bring their crew. A guild on Elo Pros is a tag, an emblem and a
roster that spans every game on the store, so a guild that plays yours is the
same guild in the next one. Your game gets it with one read:

- **Draw the tag over a player.** `guild_tags` answers up to 100 wallets at
  once with each one's badge (`tag`, `name`, `color`, `shape`, `role`), or
  none. The face you already read (`face(wallet)`) carries the same badge as
  `guild`, so a server that shows names shows tags for free.
- **Build on the roster.** `guilds_of(wallet)` is a player's guilds and roles:
  enough for guild halls, a guild chat, or guild against guild.
- **The player picks the tag.** One wallet is in up to five guilds and chooses
  which one it wears on the store's guild page; the read already honours it.

Rosters are public and signed act by act (`GET /api/guilds/{tag}/log`), and
nothing in a guild is for sale: perks your game grants a guild are cosmetic,
and yours to grant.

Three rules hold in all three:

- **"Could not look" is never "no".** Every call answers an answer, a
  refusal in the server's words, or could-not-look (unreachable, timed out,
  a 5xx, not JSON, or the server saying `reachable: false`). A ticket read
  that times out is not "does not own"; never kick a player on it.
- **What you sign is rebuilt locally.** Every signed call asks the server to
  prepare the change, rebuilds the text from what was asked for, and refuses
  a server that offers any other text before the signer is asked. `buddy`
  reads the change back out of the text instead, because the server cleans
  its args (a name trimmed, a line's spaces collapsed), and refuses any
  change but the one asked for.
- **The signer is yours.** It is a function from the text to a signature:
  the launcher overlay, a browser wallet, a hardware wallet, or your own.
- **Kick on a definite no, never on a failed read.** `owns` and the roster
  sweep answer `false` only for a wallet that holds no copy; a read that
  failed is could-not-look (or `null` for that wallet), and an outage must
  never boot a paying player. A sign-in check that could not ask the chain is
  could-not-look too, never a refused player.

## Checking a vendored overlay

`SHA256SUMS` lists the files a project may copy as they are. From this folder:

```bash
sha256sum -c SHA256SUMS
```

`test_sdk.py` checks the list against the bytes here, and the Python overlay
against the launcher's broker.

## Tests

```bash
python3 sdk/test_sdk.py                          # the overlay against the broker; SHA256SUMS
python3 -m unittest discover -s sdk/python/tests
node --test sdk/js/test/*.test.js
cd sdk/rust && cargo test --all-features
```

CI runs all of them, plus clippy, the packaging dry runs and
`meter/test_sdk_fixtures.py`.

## Publishing (operator)

The three carry one version; bump it in `rust/Cargo.toml`,
`python/pyproject.toml`, `python/elo_sdk/client.py` (`__version__`) and
`js/package.json` (plus `VERSION` in `js/index.js`) together.

```bash
# Rust: crates.io
cd sdk/rust && cargo publish

# Python: PyPI
cd sdk/python && python3 -m build && twine upload dist/*

# JavaScript: npm, once the npm organisation `elopros` exists
cd sdk/js && npm publish --access public
```

A released `elo-N.js` never changes: a new web SDK version is a new file,
copied into `js/` and added to `SHA256SUMS`.
