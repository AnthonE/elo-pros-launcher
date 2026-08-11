---
status: live
lane: [platform]
updated: 2026-08-09
about: "how a game gets identity, a profile and a signature without ever holding a key — the signer ladder, the broker socket a game knocks on (Steam's pipe, ripped), and the reference clients Gates links"
---

# SDK.md — how a game signs without holding a key

> Operator, 2026-08-04: *"Does this have the proper key support? also i didnt
> think about it but can we ensure the Gates desktop build can have access to
> that as well i guess? almost like we need a scry sdk?"*
>
> **The honest answer to the first question was no**, and the gap was not only
> in the launcher — it was in shipped code. This page is what closed it. The
> second instinct was right, and Steam had already solved it.

## 0 · What was actually missing

The launcher held no key and handed every write to the browser. That satisfies
`CLAUDE.md` invariant 7 and it is correct for a human clicking *Claim*. It is
not the same as key support, and three real holes were hiding behind the
conflation:

1. **The repo's own separated signer could not sign a game action.**
   `arca.py` spoke `send`, `settle`, `card`, `revoke` — transactions only.
   Every game action in this town is an **EIP-191 message**
   (`meter/playauth.py`). So rung 2 of the autonomy ladder, which `CLAUDE.md`
   calls BUILT, could guard money it could not guard *play*: an agent on a
   real arca was unable to answer, duel, wager, review or swear.
2. **The launcher could not reach a signer the user already ran.** Browser
   handoff was the only path. Someone on their own box with their own key —
   lane A, explicitly ungated — had no way to use it.
3. **A native game had no path at all.** Gates' Rust binary would have had to
   ask the player for a private key, which is the one thing we must never
   teach, and which is unrecoverable once taught. `clients/python` is the
   wrong shape twice over: Python, and it takes a raw key.

## 1 · The answer, and Steam had it first

`steam_api` does not hold your credentials. It talks to the **running Steam
client** over a local pipe, and the client does the privileged part. That is
why a Steam game never asks for your password.

Ripped, it maps onto our invariants exactly:

```
   Gates (Rust)            the launcher                 the signer
  ┌──────────────┐       ┌───────────────┐       ┌────────────────────┐
  │ scry_overlay │──────▶│    broker     │──────▶│ browser · local ·  │
  │  no key      │ unix  │   consent     │       │ arca · external ·  │
  └──────────────┘ sock  └───────────────┘       │ none               │
                                                 └────────────────────┘
                                                    the key lives HERE,
                                                    and scry never sees it
```

Three properties fall out, and none of them cost anything:

- **The game never needs a key.** The strongest ask available to a *hostile*
  game becomes "a message the signer will sign" — a recognised family,
  journaled, with a daily count — instead of "paste your seed phrase".
- **The launcher routes rather than custodies.** Four of the five backends
  hold nothing at all; the fifth is an account you made on your own machine
  (§2a), and scry the service sees none of them. `crates/scry-broker/tests/signer.rs`
  pins the claim against the source per-backend rather than as one sweeping
  scan, because a sweeping claim that stopped being true is worse than a
  narrow one that is.
- **One property Steam cannot have:** the signer refuses to sign what it
  cannot read (§3).

## 2 · The ladder, as five backends

`launcher-rs/crates/scry-broker/src/signer.rs` plus `scry-vault`, which holds
the one key-bearing backend. The rungs are `CLAUDE.md`'s, unchanged.

| setting | rung | who holds the key | what it does |
|---|---|---|---|
| **`local`** **(what the client does)** | 0, wallet moved inside | **this process, while unlocked** | an account made on your machine (§2a). **The only backend `prove` works on** |
| `browser` | 0 — the human signs each act | your wallet, in your browser | builds a link and stops. Collects nothing, listens on nothing |
| `arca` | 2 — bounded standing authority around a real key | `arca.py`, its own uid | a UNIX socket. **The real boundary** |
| `external` | — | a device or a command you name | message on stdin, `0x…` on stdout — hardware wallets, `cast wallet sign`, your script |
| `none` | — | nobody | reads work; writes say plainly that nothing is configured |

**Four of the five reach a signer somebody else runs.** `local` is the one
that does not, and §2a is why it exists and what it costs.

⚠ **The shipped client does `local`, and that is the operator's call rather
than an accident** (2026-08-07: *"we need people to use the launcher. idgaf
about crypto natives. if they cant make a new wallet and xfer fuck them"*). It
reports `local` once an account exists and `none` before — a player makes an
account in the client or plays anonymously, and there is no browser-wallet
onboarding path to fall down. The other rows are the design and are reachable
by anyone building on this crate; they are not what a downloaded client offers.

**A backend is never named by the wire.** Not by a manifest, a catalog, a
depot, or the broker socket — which is the reason a hostile catalog cannot
point your signing at itself, and it matters more now that one backend holds a
key.

### 2a · `local` — an account made here, and what it costs

Operator, 2026-08-04: *"we logged it somewhere we could let a new account be
made inside of the app, user keeps keys on machine."* It was logged, twice, in
tension: `WALLETS.md` §0 says the website creates no account, and `STEAM.md`
§4 names onboarding as the thing that loses us most players. **The resolution
is that the first is a website rule and the second is about the client.**
`WALLETS.md` now carries the carve-out and the reasoning; the short version is
that a web page is the worst place in computing to hold a key and a desktop
binary on your own disk is not, which is why MetaMask is an extension.

What the launcher does (`launcher-rs/crates/scry-vault`):

- **generates from the OS entropy pool on your machine.** scry never sees it.
  Invariant 7 is untouched — *we* hold no user keys, and this is not ours.
- **BIP-39, 12 words, standard path `m/44'/60'/0'/0/0`**, and a **standard V3
  keystore** (scrypt n=262144). Both chosen for the exit: the account imports
  into MetaMask or Rabby later. An account only this client could open would
  be a worse deal than no account.
- **The words are shown once, never written by us, and verified by typing
  three of them back before anything is saved.** A checkbox saying *"I saved
  it"* saves nothing, and the moment before the account is usable is the only
  moment a backup can be insisted on.
- **`wallet.py`'s rule survives, and the word doing the work is `silently`:**
  *a silently generated key is a fresh empty account that swallows whatever
  you send it.* Creation here is a ceremony that prints the address, verifies
  the backup, and says out loud that the account is empty.
- **Nothing is hand-rolled.** `k256` does the ECDSA, `sha3` the keccak,
  `scrypt`/`aes`/`ctr` the keystore. Novel ECDSA holding someone's money is the
  wrong risk to take for a smaller dependency tree, and a subtly wrong nonce
  leaks the key silently. §10b8 is the budget that bought those crates.

⚠ **The cost, and it is on the surface every time, not only here.** With
`local` the launcher **is** the signer. Its recogniser and daily count are a
lint pass over its own intentions — real against a confused game or a model
mistake, worth nothing against a compromised launcher. That is `WALLETS.md`
§8b's finding applied to ourselves, and the honest move is the one `arca.py`
already makes when its boundary is off: **say it on every reply and on the
surface, every time.** The Signing window prints it.

⚠ **This is the trade the operator took knowingly.** A player who already holds
a wallet is not served by it — they make a second account and transfer. What it
buys is that *"install, make an account, play"* has no extension, no seed
phrase to paste, and no chain to add. Onboarding is what `STEAM.md` §4 names as
the thing that loses the most players, and this is the side of that trade the
client is on.

It is therefore **not a new rung**. It is rung 0 with the wallet moved inside
the process — the same authority model (a human approves each act), worse
isolation, traded knowingly for onboarding. Better than a game asking for a
key; strictly worse than an arca, which is one setting away.

## 2b · The overlay verb — a game draws, and never approves

> Operator, 2026-08-05: *"and what about an overlay?"*

Steam injects a library into the game process and hooks the graphics API,
because Steam does not own the games. This platform does, so an overlay here is
**the game rendering its own HUD** from one broker read — which is why this
file's Rust client has been called `scry_overlay.rs` since before the feature
existed. No injection, nothing to break on a driver update, nothing that reads
as malware to an anti-cheat, and nothing that needs a Wayland answer.

`overlay` is the seventh verb (`broker.py`, both SDK clients). It returns one
snapshot: host, signer kind, the watched address, this title's catalog row, and
a `consent` block naming who asks the player. It is cheap enough to poll each
frame — one round trip on a local socket — and every field in it could already
have been asked for one at a time. Bundling is a convenience, never new
authority.

⚠ **The wall, written before it is a precedent.** A game controls its own
pixels, so it can draw a convincing fake of any dialog. Therefore the overlay
may **display** and may never **approve**:

* there is no consent token in the reply, and no way to mint one;
* there is no pending-request queue a game could render as its own prompt and
  then answer;
* there is no "already allowed" flag — consent stays per game, per family,
  counted, asked in the launcher's own window, and dying with the launcher.

`sdk/test_sdk.py` asserts the absence **by key** rather than by scanning the
payload, because the `consent` note legitimately contains the word *approve*
while explaining that nothing may. It said "and `test_launcher.py`" until
2026-08-08 — that suite went with the Python client on 2026-08-07, and a
citation to a deleted gate reads as two checks where there is one.

**The verb count is a budget.** The vocabulary went six to seven for this, and
the test pins the number so every future verb costs the same deliberate edit —
the wire is the whole of a game's authority.

### 2c · `prove` is Sign-In With Ethereum, and the crypto was never the bespoke part

> Operator, 2026-08-07: *"i dont see why we have some special crypto things that
> no one else has lol."*

**The complaint was half right, and the right half was the one that mattered.**
Audited rather than argued: every *primitive* here is off the shelf — EIP-191
`personal_sign`, secp256k1 and `ecrecover`, a V3 keystore, BIP-39 on the
standard path. A game verifies our signatures the way it verifies anyone's.

What was invented was the **message text**, and it landed on the one message a
**third party** has to verify. `prove` used to emit
`scry prove\ngame:…\nserver:…\nnonce:…`, which meant a game's backend hand-rolled
a string concatenation and got it byte-exact or watched signatures fail
silently. **There is an industry standard for exactly that operation — EIP-4361,
Sign-In With Ethereum — and this repo had never mentioned it**: zero hits for
`siwe`, `4361` or the phrase, anywhere.

It now emits a real SIWE message, so a game's whole server side is
`new SiweMessage(msg).verify({ nonce, domain })` with no scry-specific code.
Verified against the reference `siwe` library end to end: it parses, it
verifies, and a wrong nonce or a tampered domain is rejected.

**It was free to switch, and would not have been later.** Nothing verified
`prove` yet — no verifier in the meter, none in Gates, and Gates' native
manifest rows are still empty slots. A format with no consumers costs one
commit; the same change after a title ships costs a migration.

**Three properties the old format did not have**, and none of them are ours:

- **the domain is bound into the signed bytes**, so a proof for one shard
  cannot admit a player to another operator's server;
- **`Issued At`**, so a verifier can age a proof;
- **the address is named in the message**, which is why the broker now refuses
  when the signer's address disagrees with the one it composed for — that
  proof could never verify, and refusing beats handing a game something that
  fails in someone else's server for a reason it cannot see.

⚠ **Two costs, both real and both borne by game developers.** EIP-4361 fixes
the nonce alphabet at **alphanumeric, ≥8**, so a dash-separated uuid must be
normalised; and `server` is now a **domain authority** rather than free text.
The old guard was looser. `sdk/PROTOCOL.md` states both where a developer hits
them, and the test that used to assert *"a guard so tight a uuid is refused
would be a guard nobody can use"* now asserts the opposite and says why.

⚠ **`Host::address()` must be EIP-55 checksummed**, and that stopped being
cosmetic: the reference SIWE parsers **reject an all-lowercase address
outright**, so a launcher returning one issues proofs nobody can verify with
nothing failing on our side. `scry_vault` already returns the checksummed form.
The broker cannot re-derive it — checksumming needs keccak and `scry-vault`
depends on `scry-broker` rather than the reverse — so it is a stated contract
held by the implementor, plus a test.

**What did NOT change, and deliberately.** The `scry <family>` format for
`play`, `vow`, `review` and the rest stays: only *our own* server verifies
those (`meter/playauth.py`), so the shape costs no integrator anything, and it
is what the consent prompt renders. EIP-712 typed data is the standard for
signing *actions* rather than sign-ins, and that is its own question.

### 2d · `profile` — a name and a face, and the drift that made it necessary

> Operator, 2026-08-07: *"i wanna work on making the SDK just like steams so
> its easy for games to auth and get profile info."*

**The bug found first, because it is the more important half.** `prove` — the
verb §6 tells every game to reach for — shipped in the Rust broker, in both
reference clients and in `sdk/PROTOCOL.md`, and **a second broker that then
existed in Python refused it as an unknown op.** Both launchers were on the
download page. A game following this document authenticated for one launcher's
players and hard-failed for the other's, and every suite was green: the Rust
tests drove the
Rust broker, `sdk/test_sdk.py` drove the Python one, and nothing compared them.
Even the two verb-count budgets disagreed — seven on one side, eight on the
other — in files neither suite read.

That is the same shape as `CLAUDE.md`'s nginx trap: **a surface can be dark to
half the world with every local gate passing**, because each half is tested
against itself. The fix is the verb lists being compared *across languages* —
`sdk/test_sdk.py` now parses `protocol.rs` and asserts both brokers, both
clients and `PROTOCOL.md` name the same verbs with the same fields.

**What `profile` is.** One read of `/api/who`, flattened to what a HUD draws.
`/who` is already the origin's join across sworn name, npub, vow and wallet;
without this verb every title reimplements it, and each one discovers
separately that the handle and face hang off a `vow_id` while achievements and
holdings hang off a wallet.

**One verb is one origin read.** Achievements, holdings and reputation come
back as urls under `links` for the game to fetch — the same split Steam makes
between `GetPlayerSummaries` and `GetPlayerAchievements`.

Three properties, and each is a mistake it prevents:

- **The sworn and the self-declared never merge.** `handle` is the NIP-05 name
  out of the locked index and cost SCRY to change; `display_name` is whatever
  its owner typed. Both clients expose `label`, which renders a sworn name bare
  and prefixes anything else with `~` (`ACHIEVEMENTS.md`). A surface showing a
  claimed name in the clothes of a checked one has lied for free.
- **Unreachable is not empty.** `CLAUDE.md` §traps, and the reason this is not
  a passthrough: a 404 from `/who` is a real answer — *nobody has sworn here* —
  while an origin that is down is not an answer at all. The reply carries
  `reachable`, and a host that cannot read **refuses** rather than returning a
  blank profile. A game that draws a confident *"anonymous"* over a network
  outage has invented a fact about a person.
- **It is a read and authorises nothing.** No signature, no consent, no key. It
  describes an address; it does not establish that the player holds its key.
  That is `prove`, and `profile` says so in its own note.

⚠ **Why the launcher fetches instead of handing back a url**, which is what
`servers` does and would have been more consistent: `sdk/rust/scry_overlay.rs`
is `std`-only with no crates by design, and **`std` has no HTTP client**. A
game linking the vendored SDK cannot make the call. The launcher already
carries a reader that keeps unreachable apart from empty, so the choice was
one careful implementation here or one careless one per title.

⚠ **`prove` works only on the `local` backend, in both launchers.** `arca`,
`browser` and `external` all inherit the refusing default: `prove` deliberately
bypasses the family allowlist (it is not one of `FAMILIES` and must never
become one, or a game could reach the prompt-free path through `sign`), and the
arca's own copy of that list beside the key does not know the word. Making an
arca able to prove is a real change to a security-relevant allowlist and gets
its own sentence, not a silent edit. Until then a player on an arca gets a
readable refusal, which is the honest answer.

## 3 · The arca learns to sign — and refuses what it cannot read

The `sign` verb is new (`arca.py`). It follows the rule the socket already
had, which is the only rule that matters here: **the decision is re-derived on
the far side.** The arca does not sign what a caller hands it. It signs a
message it can *parse*, from a family it knows:

- an unrecognised first line is a **refusal**, not a warning. A message the
  arca cannot read is one it cannot describe in its journal, and an unreadable
  line in a signing journal is worth nothing;
- a **single-line** message is a refusal. Every real family is multi-line, and
  a lone hex blob is the shape of an EIP-712 digest or a raw hash — the one
  class of "message" that *can* carry a transaction's authority;
- a caller-named family is refused like any other unknown field. The arca
  derives it.

The thirteen families are `braid card covenant doc familiar hive holder meter
pact play review store vow` — every EIP-191 message the town actually builds.

**The bound is a COUNT, not a value.** A message moves no native value (the
0x19 prefix makes it structurally not a transaction), so charging it against
the ether cap would be theatre; leaving it unbounded would let a compromised
client burn a day of a vow's actions in a loop. Default 200/day, durable
across restarts — *an in-memory count is a cap you lift by crashing.*

The journal keeps the **full text** of everything signed, and of every
refusal. A signing record that keeps only a hash cannot be audited by the
person it was signed on behalf of, which is the only audience that matters.

## 4 · The broker — and what it is not

`launcher-rs/crates/scry-broker`, spec in `sdk/PROTOCOL.md`. Newline-delimited
JSON on a local socket, allowlisted fields per verb, an unknown field refused
rather than ignored. Nine verbs: `hello` `identity` `sign` `prove` `profile`
`overlay` `title` `servers` `open` — and `protocol::VERBS` is the count, not
this sentence. It said "nine" over a list of eight for a while, which is the
shape of every hand-maintained total in this repo.

**Nothing installs, uninstalls, launches another title, reads a file, writes a
setting, or chooses a signer.** A game may ask about itself and ask for a
signature; it may not operate the launcher.

### ⚠ It is not a security boundary against a process running as you

A game the launcher started runs as your uid. It can already read your files.
`arca.py` can check `SO_PEERCRED` and refuse a same-uid client because the
arca is a *different user*; the broker cannot, because it is not. **Say this in
the game's own docs too** — a player who thinks the socket is a sandbox will
make a decision they would not otherwise make.

What it provides instead, and each is real:

1. a game never needs a key (§1);
2. **consent is per game, per family, counted, and dies with the launcher.**
   There is no "always allow" and none is written to disk. A blanket permanent
   grant would make every rule above decorative; what is offered is an
   allowance — *just this one*, 25, or 100 — that ends when the launcher
   closes;
3. **the real wall is the signer behind it.** Point it at an arca and every
   request is re-derived by a process running as somebody else.

The prompt shows the message **verbatim**. A prompt that summarises without
showing can be lied to by a crafted `detail:` line, so the summary sits above
the real text and never replaces it. It is a real window as of 2026-08-08
(`scry_ui::consent` + `windows::consent`); until then this paragraph described
a prompt that did not exist and `sign` refused for want of it —
`docs/client/LAUNCHER.md` §10n is how it was built and what it costs.

**The door is open whenever the launcher's window is**, and there is no
setting — `scry-gui` binds it at startup, the CLI cannot serve one at all, and
closing the launcher closes it along with every grant. This said *"off by
default — opening a socket other processes can reach is an act, not a
default"* until 2026-08-08, which described an intent the code never had.
Whether it should be gated is a live operator question (`LAUNCHER.md` §4), and
it got sharper the day the door could actually sign.

## 5 · What Gates links

`sdk/rust/scry_overlay.rs` — one file, `std` only, no crates, no build script.
Vendor it. A project that already has serde can delete the `json` module.

```rust
let mut scry = Overlay::connect("gates", "0.1.0")?;      // Err is NORMAL
let who = scry.address();                                 // a CLAIM, not auth
let msg = play_message("duel", "vow_x", "ETH up 5", None);
match scry.sign(&msg, "settling round 41") {
    Ok(sig)                      => submit(&sig.signature),
    Err(SignError::Handoff(url)) => show_link(&url),      // NOT an error
    Err(e)                       => show(&e.to_string()),
}
```

Three things the type system is doing on purpose:

- **`connect` returning `Err` is a normal state.** No launcher is running.
  A game must play fine without one, and there is no fallback in this file
  that asks for a key — there is no third option.
- **`SignError::Handoff` is not a failure.** The player's signer is their
  browser and the act finishes there. A game that retries on it spins forever;
  a game that shows it as an error tells the player something broke when
  nothing did. Four variants, three of which are not retryable.
- **`play_message` is built offline**, not fetched. A client that asks a
  server what to sign has handed that server the ability to change what it is
  signing. The format is deterministic (`meter/playauth.py`).

`sdk/python/scry_overlay.py` is the same client for a Python game or an agent.

### 5a · Vendoring is a copy, so drift is the failure mode

A vendored file has no version and no dependency resolver, which is the whole
point and is also the one thing that can go wrong. Both halves of the problem
were measured on 2026-08-09, on the only game that vendors this:

**A consumer's pin catches its own edits and cannot see us moving.** Gates
pins its copy with a sha256 and a test. That test was green while its copy sat
**326 lines behind this file** — no Windows transport (it `use`d
`std::os::unix::net` unconditionally, so a Windows build of that game could not
compile at all), no `prove`, no `profile`, and `Overlay::title` had changed
shape underneath it. Nothing was broken locally in either repo, so nothing went
red in either repo. `sdk_parity.rs`'s own header called this file *"what Gates
has compiled into its binary byte-for-byte"* — a claim about a file in another
tree that nothing checked.

**We cannot gate another repo, so what we owe is a number they can check
against.** `sdk/SHA256SUMS` is that number, and `sdk/test_sdk.py` §what a
vendoring game checks against keeps it honest. The drift check is then one
line at the consumer's end:

```text
sha256sum <their vendored copy>     # must appear in sdk/SHA256SUMS
```

**Two properties are gated here because only a consumer would notice them
break.** The file must be **rustfmt-clean** — a vendoring project runs its own
`cargo fmt --check` over everything in its tree, and an unformatted SDK has
reddened a consumer's CI twice now; `launcher-rs` has no fmt gate of its own,
so this suite is the only place that property lives. And every unix-only
import must sit behind `cfg(unix)` with a `cfg(windows)` arm beside it, because
that failure surfaces in a game's Windows build and nowhere near us.

### The test that makes this real

`sdk/test_sdk.py` builds the **real broker** and drives it over a **real
socket** with the Python reference client;
`crates/scry-broker/tests/sdk_parity.rs` compiles the vendored Rust client and
drives it against the same server. **A protocol whose two halves are only ever
tested separately is a protocol with no test** — and that is not a slogan here,
it is the post-mortem: `prove` was dark to half the players for exactly that
reason until the second broker was deleted.

## 6 · How Gates uses it, concretely

Nothing here waits on the desktop build; the manifest already carries it.

| Gates needs | how |
|---|---|
| **to check a ticket and kick** | two halves, both server-side (**BUILT 2026-08-08**, `TICKET.md` §2b). *Who is this:* **`prove`** — the launcher composes a **SIWE (EIP-4361)** message binding your domain, the player's address and your nonce; your backend hands it to any `siwe` library; no consent prompt fires. §2c. *Do they own it:* `GET /api/ticket/gates/of/{wallet}` at join, then `POST /api/ticket/gates/check {wallets:[…]}` sweeps the roster on your interval — the sell-mid-session answer. ⚠ kick on a definite `false`, **never on `null`** (a failed read); an outage must not boot a paying player |
| who is playing, casually | `identity` → an address. **A claim** — anything can say a number. Use `prove` the moment it matters |
| **a name and a face for a scoreboard** | **`profile`** — one read of `/api/who`, flattened. Steam's `GetPlayerSummaries`. §2d |
| to sign a round | `sign` with `play_message(...)`. The player consents; the signer decides |
| the shard list | `servers` returns Gates' **own URL**; Gates fetches it. The launcher does not proxy, cache or rank it |
| to send a player to a page | `open`, https only |
| to run with no launcher | `connect()` fails and the game plays. Always the shipped default |

**Agents get the same door.** The deterministic sim is an RL environment by
construction (`GATES.md` §0), and an agent harness speaks this protocol with
`sdk/python/scry_overlay.py` exactly as the game does — which means a training
run signs through the same bounded, journaled arca a human does, instead of
being handed a raw key because it was convenient.

## 7 · What is still open

| # | thing | state | operator act? |
|---|---|---|---|
| ~~1~~ | ~~a Gates depot, so there is a native build to broker for~~ | **CLOSED 2026-08-10** — two depots published and notarized, `linux-x86_64` and `win-x86_64` (`LAUNCHER.md` §8 rows 1b–2). The installed binary calls `Scry::discover()` for real: driven 2026-08-06, it found no broker and printed *"gates: playing anonymously — no scry launcher"*, which is this document's required degradation rather than a failure. **What has not been driven is the other branch** — a running `scry-gui` on the same machine, so `discover()` succeeds and a signature is prompted end to end from inside the game | done |
| 2 | an arca actually running on the operator's box | `arca.py` serves; nothing is deployed | **yes** — starting one is a human at a terminal, by design |
| 3 | `messages_per_day` for a real arca | default 200 | a **(knob)** when one runs |
| 4 | transactions through the broker | deliberately absent — `sign` covers messages only; a transaction goes through `/prepare/*` and the arca's own `send` | its own sentence |
| — | macOS | nothing built — no depot, no build, no transport tested. `AF_UNIX` would carry it unchanged | later, with a depot |

**Windows is not on that list any more.** The transport is a named pipe at
`\\.\pipe\scry-launcher-<user>` (`transport.rs`, and both reference clients),
one pipe per user because that namespace is machine-wide, created
`PIPE_REJECT_REMOTE_CLIENTS` because `\\.\pipe\` is otherwise reachable over
SMB. The wire above it is byte-identical. `sdk/PROTOCOL.md` §finding the socket
carries the one caveat that survives: the connect timeout is bounded there and
reads are not.

**Why (4) is absent and not merely unbuilt:** a message authorises an act; a
transaction moves money. Putting both behind one game-facing verb would mean
one consent prompt covering two very different questions, and the ether caps
live on the far side of a different code path. When it lands it gets its own
verb, its own prompt, and its own row here.

## 8 · What this does not change

- **`CLAUDE.md` invariant 7 — we hold no user keys.** Nothing here holds one.
  Reaching a signer somebody else runs is what "no custody" has always meant;
  §2's table is that sentence made operable.
- **scry is not a wallet and never routes funds between parties**
  (`SENTENCES.md`, NEVER). No backend here moves money on anyone's behalf.
- **Untrusted text can never mint authority** (invariant 8). A game's request
  is untrusted text. It cannot name a family, choose a signer, set a cap, or
  raise its own budget — every one of those is refused at the field level.
- **The one test.** Would the payer's identity or the amount change any bit of
  any output? No. Nothing here grades, prices, or ranks anything.

## Reading order

`sdk/PROTOCOL.md` is the wire · `docs/client/LAUNCHER.md` is the client this lives in
· `arca.py`'s docstring is the boundary · `WALLETS.md` §8b is the finding the
arca implements · `GATES.md` is the platform.
