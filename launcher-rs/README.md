# launcher-rs — the desktop client, in Rust

**The client that ships, and the only one.** The depot seam, the updater, the
command line, all six windows, the vault and the broker. Design of record is
[`docs/client/LAUNCHER.md`](../docs/client/LAUNCHER.md).

The Python+tkinter client this replaced was **deleted 2026-08-07** — do not
reintroduce one. The half of it that had to survive is `meter/depot_doc.py`,
which the origin still imports for the depot digest.

```bash
cargo build --release          # → target/release/{elo, elo-gui}
cargo test --workspace
```

Two binaries on purpose. `elo-gui` is the window; `elo` is the same client on
the command line and does **not** link FLTK, because a headless box, a CI job
and an RL harness have no use for a window.

Nothing to install at runtime on Windows, and on Linux only the X11/pango
libraries any desktop already has. That is the point of moving: the Python
client's first-run failure is `sudo apt install python3-tk`, and its one
signing backend asks for `pip install eth-account`.

## Cutting a release

```bash
python3 build_release.py              # builds both targets, writes watchtower/dl/
python3 build_release.py --check      # what test_site.py runs; needs no toolchain
```

Three artifacts — a `.deb`, a `.tar.gz` and a Windows `.zip` — plus the
`SHA256SUMS` covering everything `/download.html` hands out, both clients.
The `.deb`'s `Depends:` is read off the ELF's own `DT_NEEDED` table and the
build **fails** on any library the package does not declare.

The Windows half cross-compiles from Linux and needs two things once:

```bash
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu
```

⚠ `x86_64-pc-windows-msvc` cannot be used here: `ring` (via `ureq`'s TLS) runs
a C build script wanting `lib.exe`. A real MSVC release is built on Windows.
And **compiling is not running** — nothing has executed on Windows yet, which
is why `/download.html` says so on the download button's own panel.

## What it does today

```
elo check                     what this machine can run
elo games                     every title the origin serves a manifest for
elo list                      every installed build, read off its receipt
elo status  <title>           is a newer build published?
elo install <title>           install the native build for this machine
elo update  <title>           install it only if it is newer
elo verify  <slug> [build]    re-hash an install against its receipt
elo prune   <slug>            remove builds superseded by the newest
elo uninstall <slug> <build>  remove one build
elo play    <title>           start the newest installed build (and watch it)
elo digest  <depot>           the bytes32 a notary would commit
elo hive [rooms|read|voice]   the town's talk annex, read
elo hive whoami               the voice this machine speaks with
elo hive voice-new            make one (schnorr, kept here, never sent)
elo hive say <text>           speak — town stream, or --room
elo sessions                  what has been played, and what is running
elo mcp                       serve these reads over MCP on stdio
```

`--games PATH · --host URL · --platform TAG · --server ADDR · --wallet ADDR ·
--browser CMD · --room ID · --limit N · --json · --no-reuse ·
--allow-insecure · --dry-run · --detach`

**The CLI came before the window on purpose.** A launcher whose only interface
is a GUI cannot be tested on a build machine, scripted by a player, or driven
by an agent — and this platform has agents as first-class players
(`GATES.md` §0).

## `<title>` is a name, not a url

`elo install gates`. Until the origin served manifests (`LAUNCHER.md` §11)
the only accepted shape was a full url to a JSON file, which meant this client
could install a game **only if you already knew where that game's manifest was
published** — a storefront you cannot shop in. Three shapes now resolve, local
first:

1. `https://…` — fetched as given,
2. a path that exists on this disk — read as a file, and it **wins over the
   origin**, so a title can be tested from a checkout,
3. a bare slug — read from `{host}/api/launcher/manifest/{slug}`.

Anything else is an error naming all three, rather than a fetch of something
that looked like a url and was not.

⚠ **The client completes relative urls; the server does not write absolute
ones.** A served manifest says `"depot": "/api/launcher/depot/gates/0.1.0"`,
because an origin behind nginx and a CDN has no reliable way to know its own
public name — the only candidate is the `Host` header, which whoever is asking
supplied. So `Manifest::rebase` finishes the job against the host we actually
asked. An absolute url is left alone; a **protocol-relative** `//host/…` is
*blanked rather than rebased*, because it looks relative and is not, and
prefixing a scheme would point the download at another host while the manifest
still read as same-origin. A blanked row becomes a slot: losing a build row is
recoverable, fetching from the wrong host is not.

`crates/elo-launcher/tests/origin.rs` drives the real binary against a stub of
those routes — slug resolution, rebasing, a tampered install caught by
`verify`, and the exit codes.

## The updater

*"a game updater like steam has with builds"* (operator, 2026-08-05). Four
properties, and the last is the one this repo has to be careful about:

- **it notices.** An install records its `depot_digest`; the manifest's native
  row points at the published depot. Stale is those two disagreeing.
- **builds live side by side** under `<games>/<slug>/<build>/`, so a failed
  patch is never an uninstalled game. Removal is a separate `prune`.
- **it downloads only what changed — with no delta format.** Every depot file
  is content-addressed, so a file that did not change between builds is
  *already on disk* in the prior build and is copied locally. Steam dedups at
  chunk level; file level gets most of that for a fraction of the machinery,
  because a patch usually rewrites a few large paks and leaves the rest alone.
  A reused file is re-hashed exactly like a fetched one — the old build's
  directory is writable by whoever runs this.
- **it never says "up to date" when it could not look.** `UpdateState` has a
  separate `Unknown` arm and nothing collapses it into `Current`. This is the
  repo's own trap (`CLAUDE.md` §traps) with a patch schedule attached:
  a reader that returns `[]` for both *nothing there* and *we could not look*.
  `elo status` exits **3** for unknown and **10** for stale, so a script can
  tell them apart too.

## The hive, ripped

`watchtower/hive.html` is the browser client for the town's talk annex. The
basics move into the app for **less than the last session estimated**: reading
needs no key, no WebSocket and no Nostr implementation, because the meter
already mirrors the relay server-side.

| | |
|---|---|
| `GET /api/hive/channels` | the relay's own open rooms |
| `GET /api/hive/room` | recent messages — town stream, or one room |
| `GET /api/hive/voice` | the beekeeper's outbox, the house's own notes |

⚠ **Correcting a claim from earlier the same day:** *"hive presence needs a
WebSocket client"* is true of **presence only** — kind:20001 is ephemeral and
WS-only. Reading is HTTP, and so is posting (`POST /api/hive/publish` carries
one already-signed event). Writing needs a **schnorr (BIP-340)** key, which is
a different scheme from the launcher's secp256k1 ECDSA signers and therefore
its own decision. Reading is free today.

### Speaking, and why it needed more than an HTTP POST

⚠ **Correcting this README's own earlier claim.** It said posting was HTTP too,
via `POST /api/hive/publish`. That endpoint exists, but `meter/hive.py` records
what happened against the real relay on 2026-07-25: **buzz binds an event's
author to the NIP-42 authenticated connection**, so a note carried on the
house's socket is refused — *"event pubkey does not match authenticated
identity"* — and refused **silently**, no ruling returned, the note simply
absent. A silent refusal is the worst shape a failure can take.

So speaking needs three things and the client now has all three: a **schnorr
(BIP-340)** key, our **own WebSocket**, and NIP-42 AUTH on it. `publish`
returns only when a relay has ruled `true`; nothing else counts as published.

**The key is not the wallet.** The hive is Nostr, so a voice is schnorr; the
signer ladder is ECDSA. A voice **cannot move money** — no balance, no
transaction, not on chain — and **can speak as you**. At rest it is a file at
0600, not an encrypted keystore, and every surface says so: `vault.py` wraps
the *wallet* in scrypt because that key holds money and is used rarely, while a
chat key is used on every message and a passphrase per line makes a client
nobody uses. The trade is stated rather than hidden.

Signing itself is `k256`, not code written here — the same reasoning `SDK.md`
§2a used for `eth-account`.

⚠ **The bug that justifies the parity test.** `k256`'s blanket
`Signer::try_sign(msg)` SHA-256s the message first; Nostr signs the 32-byte
event id itself. The Rust self-check passed anyway, because it signed and
verified with the same wrong convention — internally consistent, externally
wrong. Only `tests/parity_verify.py`, which runs `meter/hive.py:verify_event`
over real Rust output, caught it. `sign_prehash`/`verify_prehash` are the
correct pair.

### What the browser gave us that a terminal does not

`hive.html` writes into a DOM, where the browser escapes text and a CSP bounds
it. A terminal has neither, and **hive content is written by strangers.** A raw
escape sequence in a chat line can clear the screen and paint a fake
*"depot digest verified"* above itself; `\r` can rewrite the visible line so it
differs from what was signed — and the signature still checks out, because the
bytes are the bytes. `\x1b]52` is read/write clipboard on several terminals,
which is not a thing a chat message gets to do next to a wallet.

So everything off the relay goes through `render::safe_line` first — CSI, OSC,
C0/C1 controls, DEL and bidi overrides out; emoji, CJK and accents untouched.
Each case above is a test.

One more rule carried over from `ACHIEVEMENTS.md`: `who` is the register's
sworn name and `display` is the key's own self-declared kind:0 profile. They
render differently (`~` marks self-declared), because a page may never show a
claimed name in the clothes of a checked one.

## Sessions and playtime

The Python client starts a process and forgets it. Steam's whole in-game
presence layer sits on not doing that.

**Playtime is only real if something watched the process**, so `elo play`
supervises by default and `--detach` is the exception. Three shapes, and they
are not interchangeable:

| | `ended_at` | counts toward playtime |
|---|---|---|
| supervised, finished | measured | **yes** |
| running | none | no — it has not happened |
| detached | never known | **no**, and `sessions` prints "(+N not measured)" |

A detached launch is kept because *"you launched this"* is true. What it may
never do is become a number — `LAUNCHER.md` wall 3 is that this client is not
a source of truth, and a guessed end time is invented data. `playtime()`
returns `Option`, so the type system carries the distinction rather than a
convention.

## The MCP face

`elo mcp` serves the launcher's reads as MCP over stdio, which is the other
half of *no plugin loader*: **the user's own harness is the mod.** Three rules,
each a refusal:

- **read-only.** No install, no play, no uninstall, no speaking. An agent that
  can install software or post as you without a human act is a different
  product. Writes stay on the CLI where a person typed them.
- **one tool, not five.** A tool costs context in every session whether it is
  called or not, which is why the hosted endpoint went 66 → 25. `launcher`
  takes a `what=` selector.
- **local, and it could not be otherwise.** A hosted endpoint cannot see your
  installs or reach your signer, and the hosted `/mcp` is at 29,614 B against a
  32,000 B tripwire besides.

Hive text reaches the model through the same sanitiser the terminal uses, and
carries a line saying it is untrusted stranger-written input.

## The overlay

Steam injects a library and hooks the graphics API because Steam does not own
the games. This platform does, so the overlay is **the game drawing its own
HUD** from a broker read — which is why `sdk/rust/elo_overlay.rs` has carried
the name since before the feature existed. No injection, nothing to break on a
driver update, nothing that reads as malware to an anti-cheat.

The broker's vocabulary went six verbs → seven for `overlay`, and the test that
pins the count is a budget, not a fact — every future verb should cost the same
deliberate edit.

⚠ **Display freely; never approve.** A game controls its own pixels, so it can
draw a convincing fake of any dialog. The reply carries **no consent token, no
pending-request queue and no already-allowed flag**, and a test asserts the
absence by key. Asking the player stays in the launcher's own window, per game
and per family.

## The digest is the one number that becomes money

`ScryNotary` commits a depot's digest on chain, which is how a player answers
*is the build I just installed the build the house listed?* without trusting
the launcher, the CDN, or us.

So this crate contains a **second implementation** of `depot.py:digest()` —
exactly what `CLAUDE.md` invariant 3 calls a bug. It is allowed to exist only
because `tests/digest_parity.rs` pins it byte-for-byte to golden vectors
produced by the Python one (unicode, escapes, key ordering, big integers, the
notary/signature exclusion, the shipped Gates example). Regenerate them with:

```bash
python3 crates/elo-depot/tests/parity_gen.py
```

⚠ **If that test is deleted or allowed to skip, this crate becomes the bug the
invariant is about.**

Floats are refused rather than hashed: Python and Rust agree on `1.0` and
disagree on `1e+100` vs `1e100`, a depot has no float field, and a digest that
silently disagrees with the chain is the expensive failure.

## Dependencies, and why the list is short

Operator, 2026-08-05: *"if anything like veersion lock because lost of npm type
chain attacks recently."*

| control | where |
|---|---|
| `Cargo.lock` committed — the whole transitive tree pinned with a checksum each | repo root of this workspace |
| every direct dependency pinned `=x.y.z` | `Cargo.toml` `[workspace.dependencies]` |
| tree budget, checksum and duplicate checks | `crates/elo-launcher/tests/supply_chain.rs` |
| advisories, licences, registry allowlist | `deny.toml` → `cargo deny check` |

Direct: `serde`, `serde_json`, `sha2`, `tempfile`, `ureq` (rustls). No async
runtime, no web framework, no argument-parsing crate — the tree is a budget,
and `clap` alone would have been the largest thing in it.

**One trade worth recording, because it went the other way.** Asked to *"really
utilize rust and any nice trusted rust packages"*, the obvious candidate was
`jiff` for time. It is excellent and its author is about as trusted as Rust
gets — and it pulls **14 packages**, including `defmt` (an embedded logging
framework) and a second major `syn`, which would have tripped the duplicate
check, to render *"3m ago"*. `crates/elo-hive/src/clock.rs` is 60 lines
instead. That is the budget working as intended: it forces the trade to be
made deliberately rather than by reflex. When something here needs real civil
calendars, time zones or parsing, `jiff` is the right answer and the cap
should move for it.

**What none of that buys, said plainly:** a pinned, checksummed dependency is
still someone else's code running as you. The lock stops the bytes changing
under you. It does not make the bytes good.

## The crates

| | |
|---|---|
| `elo-depot` | manifests, depots, the digest, path safety, install/verify/update, launch. **No network dependency** — fetching is a `Fetcher` trait the caller supplies, exactly as the Python port injected `opener=`, so every rule is testable offline and a TLS stack stays out of the crate that decides where a stranger's bytes land on your disk |
| `elo-net` | `ureq` behind `Fetched { ok, reachable, why }`, plus the `NO_PROXY` handling `ureq` does not do (below) |
| `elo-hive` | the hive read, terminal-safe rendering, and a small UTC clock |
| `elo-launcher` | the `elo` binary |
| `elo-ui` | every window, and the only crate that links FLTK. It **draws and dispatches; it computes nothing** — a caller hands it what `elo-depot` already measured, which is what keeps `windows.rs` constructible in a test with no display, no disk and no origin. `wiring.rs` is what the controls do, kept out of the `elo-gui` binary because an integration test cannot reach into a `[[bin]]` |
| `elo-broker` | the game door: the socket a title asks the launcher through, its wire format and its signer trait |
| `elo-vault` | the keystore — scrypt + AES-CTR at rest, and the one place a private key is held |

## Two things found by writing this

Both are in the product, not the port:

- **`ureq` 3.1.2 ignores `NO_PROXY`.** It reads `ALL_PROXY`/`HTTPS_PROXY`/
  `HTTP_PROXY` and applies them to everything, so a launcher on a proxied
  network would tunnel `--host http://127.0.0.1:3600` (a local uvicorn), a LAN
  depot mirror, and its own test server. Worse, the proxy *answers* for a dead
  port — which defeats `Fetched.reachable` one layer below us. `elo-net`
  picks the agent per request; `crates/elo-net/src/proxy.rs` is the rule.
- **`play` started the oldest installed build.** Builds sort by name and a
  title can have several installed, so `find` took `0.1.0` right after an
  update to `0.2.0` — a patch that reads as having done nothing. Caught by the
  CLI smoke test rather than by any unit test, which is the argument for
  having driven the real binary.

## What is not here yet

⚠ **This section said "the windows" until 2026-08-13**, when the Python client
that owned them had been deleted for six days and this one had drawn all six
for longer than that. A stale gap list is worse than none: it tells a reader
the thing in front of them does not exist. What is actually missing:

- **Four of the original client's nine screens.** `windows::MENU` carries six —
  Games, Store, Servers, Account, Signing, About — and every entry names a
  window that exists, because the menu renders an unbuilt one as deactivated
  and says so. `Library`, `Friends`, `Monitor` and `Settings` are not built.
- **A progress row for an install.** The download blocks the UI thread for its
  whole length; the button says `Installing…` first so the freeze reads as the
  install rather than as a crash, but that is a mitigation and not the fix.
- **A resized shelf forgets its height on Refresh.** The position carries over,
  the size does not (`docs/client/LAUNCHER.md` §10p).

Porting `elo-depot` first was the point: 500 lines of pure logic, no UI, the
highest-risk code in the client, and the piece a game's own updater may want to
link. The rules survived the move before any pixels did.
