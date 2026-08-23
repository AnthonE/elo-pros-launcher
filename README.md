# Elo Pros

**A game platform: a curated storefront with a launchpad under it.** A game
lists, its coin launches, players buy copies, and the copy money becomes the
coin's liquidity.

This repository is the **open half** — the parts you run, link against, or
read the source of:

| | what it is |
|---|---|
| [`launcher-rs/`](launcher-rs/) | the desktop client. Rust, two binaries, no runtime to install |
| [`sdk/`](sdk/) | what a game links to get identity and a signature **without holding a key** |
| [`docs/`](docs/) | the designs of record for all three |

Live at **[elopros.com](https://elopros.com)**.

---

## The client

Two binaries on purpose. `elo-gui` is the window; `elo` is the same client on
the command line and does **not** link FLTK, because a headless box, a CI job
and an RL harness have no use for one.

**Download:** [elopros.com/download.html](https://elopros.com/download.html)
— a `.deb`, a Linux `.tar.gz`, a Windows `.zip` and a macOS universal
`.tar.gz`, with `SHA256SUMS` covering all four. The published version and
every artifact hash are served as JSON at
[`/api/launcher`](https://elopros.com/api/launcher), so you can check
what you got against what we published without trusting this page.

⚠ **The Windows build is cross-compiled and has never been started on a real
Windows machine.** It builds, links, hash-verifies and installs, and that is
all anyone has seen it do. Treat it as a first build rather than a tested one,
and [tell us what it does](https://github.com/AnthonE/elo-pros-launcher/issues).

**Build it yourself:**

```bash
git clone https://github.com/AnthonE/elo-pros-launcher
cd elo-pros-launcher/launcher-rs
cargo build --release        # → target/release/{elo, elo-gui}
cargo test --workspace
```

Nothing to install at runtime on Windows; on Linux only the X11/pango libraries
any desktop already has.

`Cargo.lock` is committed and must stay that way — it pins the whole transitive
tree to exact versions with a checksum each, which is the control that stops
the npm-style supply-chain attack.

### Three rules the client never breaks

**1. Elo never sees your key.** Read this one precisely, because the sweeping
version of it is false and the difference matters. The client *can* hold a key:
`elo account new` generates a secret **on your machine** and writes it as an
encrypted V3 keystore, so while that account is unlocked the process is holding
it. What is true without qualification is that the key is generated locally,
encrypted with your passphrase, and **transmitted nowhere** — Elo the service
never receives it. Point the client at a browser wallet or a hardware signer
and it holds nothing at all.

**2. It is never required to play anything.** A game may be launched, joined
and played without it.

**3. It is never a source of truth.** Every number it shows is measured from
the origin or the chain, and it says which. When it cannot look, it says that
instead of guessing — *"nothing published"* and *"I could not check"* are
different claims and the client renders them differently.

Design of record: [`docs/LAUNCHER.md`](docs/LAUNCHER.md).

---

## The SDK — for game developers

A game does **not** wait on us to use the launcher. Vendor one file —
[`sdk/rust/elo_overlay.rs`](sdk/rust/elo_overlay.rs), std only, no crates —
and your game gets a player identity and EIP-191 signatures off the launcher's
broker socket **with no key anywhere in the game process**.

Connect failing is a normal state: play anyway, and never fall back to asking
the player for a private key.

- [`docs/SDK.md`](docs/SDK.md) — the design
- [`sdk/PROTOCOL.md`](sdk/PROTOCOL.md) — the wire format
- [`docs/GAMEDEV.md`](docs/GAMEDEV.md) — listing a game, publishing a build
- [`docs/SIGN-IN.md`](docs/SIGN-IN.md) — Sign-In With Ethereum, no account, no API key

`elo://join/<title>/<host:port>` is owned by the client, so a friend's link
boots you into their server.

### Every multiplayer shape is welcome

**Determinism is one evidence tier, never an entry requirement.** Evidence
comes in three rungs — `onchain` (a predicate over chain state, recomputable by
anyone, fits every game shape) · `replay` (a sealed hash-chained segment,
recomputable by anyone) · `attested` (a recognized shard's signature). MMOs,
FPS, MOBAs, survival — the platform takes them.

### And a model is declared, never barred

Over the last year, "no gen AI" clauses went from unusual to standard in game
publishing agreements, and the advice a studio now gets is *don't touch it*.
That advice is sound for the deal it describes: a publisher is buying rights,
and work with no human author carries no copyright to sell. **It is not the
deal here.**

Every listing is open source before its card goes up, so nothing on this
platform was ever gated by a copyright in the assets. What a copy buys is a
token you hold, the servers it opens, and settlement into a real coin — a
contract on chain, a signature at the door, and who holds the mint. So:

- **No listing is refused, downranked or charged more for using a model.**
  There is no clause to sign, because the store takes no rights in your game.
- **A listing says where a model was used** — one field, a fixed list (art,
  code, audio, writing, translation, QA), plus a sentence if you want one.
- **Saying "none" and saying nothing are different answers**, and a card
  renders them differently. Nobody puts a claim in your mouth in either
  direction.
- **It is your word and the card says so.** No one clones your repo to work out
  which pixels a model made, so this is a declaration, never a measurement.
- **We check the shape and never the taste.** No review of whether your human
  involvement was *enough*, no quality bar, no badge for restraint.

We are not telling you AI-generated art is copyrightable — it is not, without a
human author — and listing here is not a way around a contract you signed
somewhere else. We are saying a listing here does not stop working without that
copyright. The rule is `GATES.md` §3 rule 9; the page is
[elopros.com/ai.html](https://elopros.com/ai.html).

The same goes for the labor: work is posted on a board at a standing price, and
you claim it, deliver it and get paid whether you are a person, an agent, or a
person running one. The payee is a wallet, so nobody has to be a natural person
before money moves.

---

## The contracts

**The Solidity is not in this repository, and the explorer is the better door
anyway.** Everything is deployed on **RH-Chain (chain 4663)** and
**source-verified**, so
[the verified-contract list](https://robinhoodchain.blockscout.com/verified-contracts)
shows you what is actually *running* at each address rather than what is
sitting in a source tree. Read, diff, or call any of it from there without
cloning anything.

The load-bearing ones:

| contract | address |
|---|---|
| `SCRY` | [`0xDa2a4b23459e9ca88183e990802be644AcA7C4B0`](https://robinhoodchain.blockscout.com/address/0xDa2a4b23459e9ca88183e990802be644AcA7C4B0) |
| `ScryNotary` | [`0x0C15fA7829458118e3d26229F58FE0443f8b792c`](https://robinhoodchain.blockscout.com/address/0x0C15fA7829458118e3d26229F58FE0443f8b792c) |
| `ScryFeeSplitter` | [`0xcB8f6Ec8A7A2a7d55E7e5dD9B2c5CcC4707e7996`](https://robinhoodchain.blockscout.com/address/0xcB8f6Ec8A7A2a7d55E7e5dD9B2c5CcC4707e7996) |
| `ScryBank` | [`0xac3227F678D30BEDc7F50176fC78E55e13E58de4`](https://robinhoodchain.blockscout.com/address/0xac3227F678D30BEDc7F50176fC78E55e13E58de4) |
| `ScryVowRegistry` | [`0x08131e7660639bbd086dffa9375c2a563f1d3590`](https://robinhoodchain.blockscout.com/address/0x08131e7660639bbd086dffa9375c2a563f1d3590) |

⚠ **RH-Chain is Arbitrum Nitro, so Solidity's `block.number` is the *parent*
chain's** — it advances once per ~12s while the real chain runs at 101ms. Pace
durations in `block.timestamp`, or use `ArbSys(0x64).arbBlockNumber()` for true
L2 height. This has already turned a "~10 second" reveal into ~20 minutes in a
shipped contract, with every test passing.

### What the buy actually is

A player pays once for a **copy of a game**. That payment mints the title's
**ticket** — a transferable ERC-721 that *is* the copy and the licence to the
title's official servers. Resale royalty is **0 bps**, welded: sell your copy
if you want to. The buyer also gets the game's coin at the sale price.

**Changed your mind? Sell the coin.** That is the whole refund mechanism —
there is no refund window and no escrow, because the coin already is the
refund. **You keep the copy either way**; selling the coin never takes the game
back.

---

## Where this is edited

**This repository is a published mirror, not the place to edit.** It is
generated from the platform monorepo and pushed, so a commit made directly here
is overwritten by the next publish.

- **Found a defect?** [Open an issue](https://github.com/AnthonE/elo-pros-launcher/issues).
  Fixes to games and to the platform are **paid work in SCRY, in public** — an
  accepted PR to [Gates](https://github.com/AnthonE/Gates), the first title,
  pays 100,000 SCRY flat and standing.
- **[`AnthonE/scry`](https://github.com/AnthonE/scry)** is a different
  repository and carries a different half — the measurement instrument: the
  ward, the meter math, the adapters and the agent skills.

## License

MIT. See [`LICENSE`](LICENSE).
