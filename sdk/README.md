# The Elo Pros SDK

What a game, a game server or an agent links to use the platform: a player's
identity and signatures with no key in the game, and the platform's API.

| package | folder | for |
|---|---|---|
| **`elo-sdk`** (Rust crate) | [`rust/`](rust/) | native games and servers; the launcher overlay plus the HTTP client |
| **`elo-sdk`** (Python) | [`python/`](python/) | servers, tools and agents; standard library only |
| **`@elopros/sdk`** (JavaScript) | [`js/`](js/) | Node, Deno, Bun and browsers; ships the web SDK `elo-1.js` |

Each folder's README has its install line and a quick start. The three have
one API, tested against one set of fixtures ([`testdata/`](testdata/)), which
the meter checks against its own code (`meter/test_sdk_fixtures.py`).

## Two halves

**The overlay** is how a desktop game reaches the Elo launcher on the same
machine: who is playing, a signature, the title's catalog row, the server
list. The game never holds a key. The wire is [`PROTOCOL.md`](PROTOCOL.md);
the clients are one file each, [`rust/elo_overlay.rs`](rust/elo_overlay.rs)
and [`python/elo_overlay.py`](python/elo_overlay.py), which the packages
include and a game may also vendor on its own. No launcher is a normal state:
play anonymously, and never ask the player for a key instead.

A browser game has no launcher to reach. It uses the web SDK,
[`js/elo-1.js`](js/elo-1.js) (served at `https://elopros.com/sdk/elo-1.js`),
to connect the player's wallet and sign them in to its own server.

**The platform API** is `https://elopros.com/api/`: friends with buddies,
tickets, items, faces, profiles, leaderboards, and the payout file a game
server writes when a wipe closes.

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
| a title's boards | `leaderboard(slug, kind)` | `leaderboard(slug, kind)` | `leaderboard(slug, { kind })` |
| a call's outcome | `Look::{Answer, Refused, CouldNotLook}` | `look.kind` | `look.kind` |
| the signed texts | `texts::friends_sign_text`, `face_detail`, `day_of` | `texts.friends_sign_text`, `face_detail`, `day_of` | `friendsSignText`, `faceDetail`, `dayOf` |
| the payout file | `PayoutFile`, `payout::check_doc` | `PayoutFile`, `check_doc` | `payoutDoc`, `checkPayout`, `writePayout` |
| the launcher overlay | `Overlay` | `Overlay` | — (a browser uses `elo-1.js`) |

Three rules hold in all three:

- **"Could not look" is never "no".** Every call answers an answer, a
  refusal in the server's words, or could-not-look (unreachable, timed out,
  a 5xx, not JSON, or the server saying `reachable: false`). A ticket read
  that times out is not "does not own"; never kick a player on it.
- **What you sign is rebuilt locally.** `friend` and `set_face` ask the
  server to prepare the change, rebuild the text from what was asked for, and
  refuse a server that offers any other text before the signer is asked.
- **The signer is yours.** It is a function from the text to a signature:
  the launcher overlay, a browser wallet, a hardware wallet, or your own.

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
