# elo-sdk (Python)

The Elo Pros platform SDK for Python games, servers and agents. Standard
library only, Python 3.9+.

- **The overlay**: a player's identity and signatures from the Elo launcher
  on the same machine, with no key anywhere in the game.
- **The platform API**: friends and buddies, tickets (does this wallet own a
  copy, the calls that buy one, the download grant), items, faces, profiles,
  leaderboards, an agent's check-in, the agent floor, your agents and the
  board at `elopros.com/api/`, plus
  the payout file a game server writes when a wipe closes.
- **A game server's half**: check a player's sign-in, sweep who still holds
  a copy, and issue the nonce — no crypto library of your own.

The Rust (`sdk/rust`) and JavaScript (`sdk/js`) packages have the same API;
`sdk/README.md` has the table.

## Install

Until the package is on PyPI, straight from the repo:

```bash
pip install "git+https://github.com/AnthonE/elo-pros-launcher#subdirectory=sdk/python"
# or, from a clone: pip install ./elo-pros-launcher/sdk/python
```

It installs two modules: `elo_sdk`, and `elo_overlay`, the one-file overlay
client a game may also vendor on its own (its sha256 is in `sdk/SHA256SUMS`).

## A game server's key

Cloud saves, stats and scores take your server's word, signed with an
Ed25519 key you named on the dev desk or in `elo.json` (`server_keys`).
Signing needs the `server` extra:

```bash
pip install "elo-sdk[server] @ git+https://github.com/AnthonE/elo-pros-launcher#subdirectory=sdk/python"
```

```python
from elo_sdk import Client, ServerKey

key = ServerKey.from_file("your-slug", "eu-1", "/etc/your-game/server.key")
print(key.public_b64)                     # name this with us once
elo = Client()
elo.cloud_put(key, player, "save", data)  # a Look: {etag, size, version, updated}
got = elo.cloud_get(key, player, "save")  # got.value["data"] is the bytes
elo.stats_report(key, [{"wallet": player, "stats": {"kills": 3}, "unlock": ["FIRST_RAID"]}])
```

## A game server: the sign-in check and the roster sweep

The player's client — the web SDK's `Elo.signIn`, or the launcher's `prove` —
hands your server a sign-in message and a signature. One call checks it is
for your domain and the nonce you issued, fresh, and the wallet's own
signature (plain keys, smart wallets and Solana):

```python
from elo_sdk import Client, new_nonce

elo = Client()
nonce = new_nonce()                      # send it to the player; keep it to check against

# … the player signs, and their client posts back message + signature …
got = elo.check_sign_in(message, signature, domain="play.your-game.example", nonce=nonce)
# retire `nonce` now, whatever the answer: it works once
if got.ok:
    player = got.value.address           # proven
elif got.could_not_look:
    pass                                  # the chain could not be asked: try again, never refuse on it
else:
    print("not a valid sign-in:", (got.body or {}).get("code"), got.error)

# every minute: who still holds a copy (a copy can be sold mid-session)
roster = elo.check_roster("your-slug", [player, *others])
if roster.ok:
    for wallet, holds in roster.value.entitled.items():
        if holds is False:                # never on None: a read that failed
            kick(wallet)
```

## Quick start

```python
from elo_sdk import Client

elo = Client()  # https://elopros.com

# Does this wallet own a copy of Gates?
got = elo.owns("gates", "0x1111111111111111111111111111111111111111")
if got.ok:
    print("owns a copy" if got.value else "no copy")
elif got.could_not_look:
    print("could not look:", got.error)      # never kick a player on this
else:
    print("refused:", got.error)

# Friends and buddies, with who is playing where.
got = elo.friends("0x1111111111111111111111111111111111111111")
if got.ok:
    for f in got.value.friends + got.value.buddies:
        if f.playing and f.playing.join:
            print(f.label, "is on a server:", f.playing.join)
```

A signed change, signed through the launcher:

```python
from elo_sdk import Client, Overlay, overlay_signer

ov = Overlay(game="my-game")
if ov.connect() and ov.address():
    done = Client().friend(ov.address(), "ask", "0x7777777777777777777777777777777777777777",
                           overlay_signer(ov, "a friends change"))
```

`examples/overlay.py`, `examples/friends.py` and `examples/payout.py` show
each half.

## The two halves

**The overlay** (`Overlay`, `play_message`, …) talks to the launcher's
socket; the wire is `sdk/PROTOCOL.md`. Every method works when there is no
launcher: `connect()` returns False and the game plays anonymously. Never ask
the player for a key instead.

**The HTTP client** (`Client`) reads the platform. Writes are signed:
`friend`, `set_face`, `board_claim`, `board_submit` and `entitle` ask the
server to prepare the change, rebuild the text to sign here (`elo_sdk.texts`),
and raise `Mismatch` for a server that offers any other text before your
signer is asked. `buddy` reads the change back out of the server's text
(`texts.buddy_change`) and raises `Mismatch` unless it is the one asked for. The signer is any function from
the text to a `0x…` signature: `overlay_signer(ov)`, a hardware wallet, or
your own. Bad input raises `BadInput` before anything is sent; what the signer
raises passes through.

## "Could not look" is never "no"

Every call returns a `Look`, whose `kind` is one of:

| kind | means |
|---|---|
| `answer` | the server answered: `look.value` |
| `refused` | the server said no (`look.status`, `look.error`), or this SDK did (`status` 0) before asking |
| `could_not_look` | unreachable, timed out, a 5xx, a body that is not JSON, or the server saying it could not read (`reachable: false`); `look.error` says why |

A ticket read that times out is not "does not own", and an index that is down
is not an empty inventory. Treat `could_not_look` as "ask again later".

## Payout files

```python
from elo_sdk import PayoutFile, PayoutRow

f = PayoutFile(3).pay(PayoutRow("0x2222222222222222222222222222222222222222", 120, "JUNK")
                      .because("EXTRACTED 120"))
assert f.validate() == []
f.write_atomic(f.file_name("island"))   # island.payout-3.json
```

`validate` is the meter's own format check (`meter/gamepay.py`), pinned by
`sdk/testdata/payout.json` and `payout_bad.json`.

## Tests

```bash
python3 -m unittest discover -s sdk/python/tests
```

They check every fixture in `sdk/testdata/` and run the client against a
local stub server. The fixtures are shared with the other SDKs, Gates and the
launcher, and the meter checks them against its own code.

MIT licensed.
