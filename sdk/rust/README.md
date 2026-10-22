# elo-sdk (Rust)

The Elo Pros platform SDK for Rust games, servers and agents.

- **The overlay**: a player's identity and signatures from the Elo launcher
  on the same machine, with no key anywhere in the game.
- **The platform API**: friends and buddies, tickets (does this wallet own a
  copy), items, faces, profiles and leaderboards at `elopros.com/api/`, plus
  the payout file a game server writes when a wipe closes.

The Python (`sdk/python`) and JavaScript (`sdk/js`) packages have the same
API; `sdk/README.md` has the table.

## Install

From the repo folder, until the crate is on crates.io:

```toml
[dependencies]
elo-sdk = { path = "../elo-pros-launcher/sdk/rust" }
# or, without the HTTP client and its TLS stack:
# elo-sdk = { path = "…/sdk/rust", default-features = false }
```

The `http` feature (on by default) is the platform client, over `ureq` with
rustls. Without it you keep the overlay, the models, the signed texts and the
payout file, with no network dependency.

## Quick start

```rust
use elo_sdk::{Client, Look};

let elo = Client::default(); // https://elopros.com

// Does this wallet own a copy of Gates?
match elo.owns("gates", "0x1111111111111111111111111111111111111111") {
    Look::Answer(true) => { /* let them in */ }
    Look::Answer(false) => { /* send them to the store */ }
    Look::Refused(r) => eprintln!("refused: {}", r.error),
    Look::CouldNotLook(why) => eprintln!("could not look: {why}"), // never kick on this
}

// Friends and buddies, with who is playing where.
if let Look::Answer(view) = elo.friends("0x1111111111111111111111111111111111111111") {
    for f in view.friends.iter().chain(&view.buddies) {
        if let Some(join) = f.playing.as_ref().and_then(|p| p.join.as_deref()) {
            println!("{} is on a server: {join}", f.label());
        }
    }
}
```

A signed change, signed through the launcher:

```rust
use elo_sdk::{Client, Overlay};

let elo = Client::default();
let mut ov = Overlay::connect("my-game", "1.0.0")?;
let me = ov.address().unwrap();
let mut sign = |text: &str| ov.sign(text, "a friends change").map(|s| s.signature);
let done = elo.friend(&me, "ask", "0x7777777777777777777777777777777777777777", &mut sign)?;
```

`examples/overlay.rs`, `examples/friends.rs` and `examples/payout.rs` run as
`cargo run --example <name>`.

## The two halves

**The overlay** (`elo_sdk::Overlay`, `play_message`, …) is
`elo_overlay.rs`, the same bytes a game can vendor as one file with no
crates (its sha256 is in `sdk/SHA256SUMS`). It talks to the launcher's
socket; the wire is `sdk/PROTOCOL.md`. No launcher is a normal state: play
anonymously, and never ask the player for a key instead.

**The HTTP client** (`elo_sdk::Client`) reads the platform. Writes are
signed: `friend` and `set_face` ask the server to prepare the change, rebuild
the text to sign here (`elo_sdk::texts`), and refuse a server that offers any
other text before your signer is asked (`ActError::Mismatch`). The signer is
a closure: the overlay, a hardware wallet, or your own.

## "Could not look" is never "no"

Every call returns a `Look`:

| | means |
|---|---|
| `Answer(t)` | the server answered |
| `Refused(r)` | the server said no (`r.status`, `r.error`), or this SDK did (`status` 0) before asking |
| `CouldNotLook(why)` | unreachable, timed out, a 5xx, a body that is not JSON, or the server saying it could not read (`reachable: false`) |

A ticket read that times out is not "does not own", and an index that is down
is not an empty inventory. Treat `CouldNotLook` as "ask again later".

## Payout files

```rust
use elo_sdk::{PayoutFile, PayoutRow};

let file = PayoutFile::new(3)
    .pay(PayoutRow::new("0x2222222222222222222222222222222222222222", 120, "JUNK").because("EXTRACTED 120"));
assert!(file.validate().is_empty());
file.write_atomic(std::path::Path::new(&file.file_name("island")))?; // island.payout-3.json
```

`validate` is the meter's own format check (`meter/gamepay.py`), pinned by
`sdk/testdata/payout.json` and `payout_bad.json`.

## Tests

`cargo test --all-features` checks every fixture in `sdk/testdata/` and runs
the HTTP client against a local stub server. The fixtures are shared with the
other SDKs, Gates and the launcher, and the meter checks them against its own
code.

MIT licensed.
