---
status: live
lane: [platform]
updated: 2026-08-12
about: "the desktop client — the 2003 Steam client ripped and pointed at scry: what it is, the three things it may never become, the manifest/depot seam a game plugs into, how it is packaged for Linux and Windows, and how Gates arrives on it when its desktop build ships. §10o is the UX pass: the shelves that had no buttons, the stock dialogs, the lock screen and the icons. §10p is the second: shelves that scroll, a smaller library icon, and the rows that were being drawn off the window."
---

# LAUNCHER.md — the desktop client

> Operator, 2026-08-04: *"Im on Ubuntu, make me a steam launcher rip for scry…
> and plan for Gates to work with it. i plan a desktop build soon."*
>
> The client is **built and runs today** (`launcher/`, stdlib + tkinter).
> Gates' side of it is **planned and declared, not shipped**:
> `data/launcher/gates.manifest.json` exists with both build rows deliberately
> empty, and filling either one is a one-line edit in that file with **no
> launcher change**. That seam is most of what this page is about.
> Derive the test count, never quote it — this line said `188/188` for a day
> after it had become 342:
> `python3 launcher/test_launcher.py` · `cd launcher-rs && cargo test --workspace`
>
> ⚠ **The move to Rust is done, and the Rust client is the one that ships**
> (operator, 2026-08-05: *"should we make the launcher Rust instead of python.
> i like rust"*). `launcher-rs/` has the depot seam, the updater, the CLI, all
> six windows, the vault and the broker; `launcher/` still runs and still owns
> nothing the Rust half cannot do. §10 is the reasoning, §10b9 is the
> packaging. **The native client is what `/download.html` hands out, and it is
> the only one with a Windows build.**

## 0 · One screen

A desktop program that opens the town's games, browses the curated catalog,
shows what a wallet holds, reads the hive, watches the origin and the chain,
and — when a title publishes one — installs, hash-verifies and starts a native
build.

    sudo apt install ./scry_0.1.0_amd64.deb          # from /download.html
    scry-gui                                          # or `scry` for the CLI

...on Windows, unzip `scry-<v>-windows-x86_64.zip` and run `scry-gui.exe` —
no installer and no redistributable. Or from a checkout:

    cd launcher-rs && cargo build --release

**A `Depends:` field is the only thing that turns a missing library into
`apt`'s problem instead of a player's**, which is why the `.deb` exists at all.
For the native client that library is the X11/pango stack the window links —
**derived off the ELF, never typed**, and the build fails on any soname the
package does not declare (§10b9). `dpkg -i` reads none of it: it leaves the
package unconfigured and the app still broken, which is why `/download.html`
says so in bold.

⚠ **The published sha256 is a weaker claim than reproducibility, and it says
so.** `launcher-rs/build_release.py` can only pin the container; rustc writes
its build path into the binary, so the hash answers *"did I get the bytes scry
published"* and never *"can I rebuild them"*. A checksum sold as
reproducibility that is not reproducible teaches a reader that checking is
theatre.

⚠ **What that costs, stated because it used to be covered.** A second packager
was stdlib-deterministic end to end, so `--check` could prove the committed
artifact was the one the tree builds, and `test_site.py` ran it — a hash proves
what an artifact *is* and never that it is *current*. That check died with the
client it belonged to (2026-08-07) and has no native equivalent. **The
staleness gap is open**, and the fix when it comes is a build stamp in the
binary rather than a stricter hash.

The client needs a toolchain to *build* and nothing at all to *run*, which is
what buys it Windows: no pip, no Node, no bundled runtime, no build step for a
player. That is the same posture the website takes (`SITE-PLATFORM.md`) and for
the same reason — a storefront whose client needs a toolchain to audit is a
storefront nobody audits. `launcher-rs/README.md` is the practical page; this
one is the design.

**The look is a rip, and it is sourced.** The chrome is the 2003/2004 client's
olive, cross-checked against `resource/styles/steam.styles` in
`ungstein/OG-Steam` — the same file the website's `--store-muted: #a0aa95`
already came from (`SITE-PLATFORM.md` §14a). The *meaning* colours are scry's
and unchanged: a solid green fill is reserved for an act that moves money or a
state armed on chain, gold marks in-build, and `test_launcher.py` fails on a
Play button wearing green. That law is the website's (§14b); porting the skin
did not get to relax it.

Multi-window, like the original: Games, Store, Library, Account, Signing,
Friends, Servers, Monitor and Settings each open off a menu of
buttons-with-a-sentence. What is
**not** ripped is the client's hand-drawn title bar — a borderless window on
Ubuntu trades its taskbar entry, its snapping and its keyboard shortcuts for a
strip of pixels. The body is the rip; the frame is the desktop's.

## 1 · The three walls

Written before the features, because each one is a thing a launcher is
normally *for*, and each is refused.

1. **It custodies nothing — and as of 2026-08-04 that is a narrower claim
   than it was.** This wall read *"there is no field, no keyring call, no seed
   prompt, no place to paste one."* That stopped being true when the client
   grew an account you can make in it (§4, `docs/client/SDK.md` §2a), and the honest
   repair is to state the narrower thing rather than leave the sweeping one
   standing. What holds:

   * **scry never sees a key.** `CLAUDE.md` invariant 7 is *we* hold none. A
     local account is generated from the OS entropy pool on the holder's
     machine, encrypted with their passphrase, and transmitted nowhere.
   * **The settings file never holds key material.** It has no key-shaped
     field, a hand-edited one is dropped on load, and the keystore lives in
     its own directory at 0600.
   * **Crypto is confined to one file.** `test_launcher.py` fails if any
     module other than `vault.py` imports `eth_account` or calls `Account.*`,
     and if any module outside a named three so much as names a mnemonic —
     while letting the prose that explains the rule keep saying the words.
   * **Four of the five signer backends still hold nothing at all**, and the
     test pins that per-backend rather than as one scan.

2. **Nothing you own lives inside it — and as of 2026-08-05 that is a
   narrower claim than it was.** This wall read *"it is never required to
   play. Every surface it shows is a URL that works in a browser without
   it."* That stops being true the moment a native-only title ships, and the
   operator's roadmap has four of them: Gates' desktop build, then *"the
   operator's MMO, FPS, minecraft clone, and MOBA"* (`GATES.md` §0). A
   persistent world does not run in a page. Pretending otherwise would leave a
   stated wall quietly false, so it is repaired the way wall 1 was:

   * **A native title may require this client to install and start it.** That
     is what a launcher is for, and it is now a real dependency rather than a
     hypothetical one.
   * **Every browser title still needs nothing.** The town's rooms, and any
     game that ships a browser row, open with a URL and no client.
   * **What the wall was actually protecting is untouched.** Steam's moat is
     that your library lives inside their program. Here the pass is a token,
     items are ERC-721 (`ITEM-CONTRACT.md`), and achievements are `onchain`
     (`ACHIEVEMENTS.md`) — so the library is the chain. Requiring the client
     to *run a binary* is nothing like requiring it to *hold your things*.
     Delete the client and you still own everything you bought.

3. **It is not a source of truth.** It renders what the origin and the chain
   say and adds nothing. No local leaderboard, no cached balance shown as
   current, no invented server list. Where it cannot look, it says so — which
   is the next section.

## 2 · Reachable is not empty

The repo's own trap (`CLAUDE.md` §traps): *a never-raises reader returns `[]`
for both "nothing there" and "we could not look."* Every read in the client
returns a `net.Fetched` carrying `ok`, `reachable` and `why`, and no panel may
print a count without checking `reachable` first. The two states render as
different sentences and a test asserts they differ.

This matters more in a desktop client than on a page, because a laptop's wifi
drops and a page's does not. A launcher that renders "0 games" on a train is
lying about the catalog.

Three states, one glyph, everywhere: **live** (green ink, decoration),
**building** (gold), **dark** (grey). And the town rooms' state is
**measured**, never typed — `catalog.ROOMS` types the routes, because routes
are stable, and `app.probe_rooms` asks the origin whether each one is actually
serving. That is how the client notices a router that exists in the repo but
is not on the origin yet.

## 3 · The seam — manifest, then depot

The whole design turns on one idea: **the launcher knows nothing about any
game.** A title publishes two documents and the client is a function of them.

**The manifest** (`launcher/manifest.schema.json`, one per title) says what
the title is and which builds exist. It is stable; it changes when a game
changes shape.

**The depot** (a separate document, fetched from a manifest build row) is one
build: a flat list of files with sha256s, plus the single command that starts
it. It changes every build.

```
manifest.builds[] ─┬─ kind: browser  → url        → xdg-open, nothing installed
                   └─ kind: native   → depot url  → fetch · hash · move · run
```

**A build row with no `url` and no `depot` is a SLOT, not a build.** The
client renders it as *"no desktop build published yet"* and offers nothing to
click. This is the honest shape for a game that has announced a desktop build
and not shipped it, and it is exactly Gates' shape today.

Installing is: fetch each file, hash it, and **only then** move the whole
build into place from a staging directory. A killed or corrupted install
leaves nothing — no half-build that the library would count as real. Verifying
is re-hashing what is on disk against the install's own receipt, which is
Steam's *verify integrity of game files* and is not interesting.

### 3a · What IS interesting: the digest

A depot's own digest is a `bytes32` — sha256 over canonical JSON of the depot
with `notary` and `signature` removed. `ScryNotary` is deployed on 4663 at
`0x0C15fA7829458118e3d26229F58FE0443f8b792c`, and `notarize(digest, label,
memo)` commits it.

So a player can ask a question no other storefront can answer: **is the build I
just installed the build the house listed?** They recompute the digest from
the depot file, look it up on a block explorer, and never trust the launcher,
the CDN, or us. That is `STEAM.md` rows 3 and 4 — *a claim about a trace,
recomputable by anyone* — pointed at a binary instead of a round.

`depot.digest()` is the one implementation. A second one is a bug, the same
way a second `channel_profile` would be (`CLAUDE.md` invariant 3).

### 3b · The trust boundary, said plainly

Installing a native build means running someone else's binary, and no hash
check changes that — a correctly-hashed backdoor installs perfectly. What the
hash buys is that **what you ran is what was notarized**, so a bad build is
attributable and cannot be swapped silently for one player. What makes it safe
to run at all is the listing rule: a listed game is open source at listing
(`GATES.md` §5), so the binary has a source tree anyone can diff.

The launcher's own job in that chain is narrow, and every part of it is
tested:

| rule | why |
|---|---|
| every depot path lands inside the build directory | `../`, absolute, drive letters, backslash, NUL — and a **symlinked component already on disk**, which is not a textual attack and needs the second, filesystem-level check |
| a path must be **canonical** | not a traversal rule — a uniqueness rule. `a/b` and `./a/b` are two strings for one file, so a depot could list both, slip the duplicate check, and let whichever downloaded last silently win |
| duplicates are case-**insensitive** | two entries differing only in case overwrite each other on mac/win and the second hash wins with no error |
| a file is executable iff the depot said so | never whatever the transfer's umask produced |
| `launch.exec` must be one of the depot's own files | it is re-checked at launch, not trusted from the install: a receipt is a file on disk and can be edited afterwards |
| `LD_PRELOAD`, `LD_AUDIT`, `PATH`, `PYTHONPATH`, `DYLD_*` are refused | a depot does not get to redirect the loader |
| `LD_LIBRARY_PATH` is allowed, and must resolve **inside the build** | game builds legitimately need it; letting it point out is letting the depot choose which libc you run |
| an unknown `{placeholder}` in argv is refused, never passed through | a literal `{token}` reaching a game's argv shows up once, in production, on someone else's machine |
| the depot root must be https | and `open_url` refuses any scheme but http(s), so a catalog cannot hand the client a `file://` or a custom handler |

## 4 · Signing — reach a signer, never become one

**`docs/client/SDK.md` is the design of record for this whole layer.** The short
version, because it changes what the client is:

The launcher **may hold one key: an account you made on this machine, that
scry never sees.** It cannot grow a hidden one — the settings file has no
key-shaped field, a hand-edited one is dropped on load, and crypto is confined
to the vault by a test. The seam (`scry-broker::signer` + the vault) has five
backends:

| setting | who holds the key | what happens |
|---|---|---|
| `browser` **(default)** | your wallet, in your browser | builds a link and stops |
| `local` | **this process, while unlocked** | an account made here, kept here |
| `arca` | `arca.py`, its own uid | a UNIX socket. **The real boundary** |
| `external` | a device or a command you name | stdin → message, stdout → `0x…` |
| `none` | nobody | reads work; writes say so |

The backend comes from the settings file alone — never from a manifest, a
catalog, a depot, or a game.

**`local` is the account you can make in the app** — *Create new account* on
the opening screen, exactly where the 2003 client put it. Twelve BIP-39 words
shown once and verified by typing three back, a standard V3 keystore at 0600,
and the key generated on your machine and sent nowhere. ⚠ It is **not** a new
rung: the launcher holds the key while unlocked, so its checks are a lint pass
over its own intentions rather than a boundary, and every surface says so.
`docs/client/SDK.md` §2a is the reasoning and the cost.

**A game reaches it through the broker** (`launcher-rs/crates/scry-broker`), which is
Steam's local pipe ripped: the launcher serves a socket while it is open, and
a game asks for identity and signatures instead of ever asking for a key.
⚠ That socket is **not** a security boundary against a process running as you;
it is how a game gets a signature without needing a key, and the wall is
whichever signer sits behind it. Consent is per game, per family, counted, and
dies with the launcher — there is no "always allow".

**The door is open whenever the window is, and there is no setting.**
`scry-gui` binds it at startup and prints the endpoint; if the bind fails the
launcher still runs and games play anonymously. The CLI has no door at all —
`scry` cannot serve one, so a headless box is not listening. Closing the
launcher closes it, and every consent grant dies with it.

⚠ **This paragraph said "off by default" until 2026-08-08 and that was never
true of the shipped client** — nothing reads an env var, and `main()` calls
`open_the_door` unconditionally. It mattered less while `sign` refused
everything; now that the door can produce a signature, the honest sentence is
the one above, and **whether it should be gated is an operator decision rather
than a bug to fix quietly.** The argument for leaving it open: the door is the
product, a per-user socket, and it prompts for every signature. The argument
for a gate: it is reachable by any process running as you, and a player who
never installs a game has no use for it.

The Signing window names the backend, says what it can and cannot do, and
offers the unlock. **It does not list what games have asked for** — that claim
was in this paragraph too, and no such list is built. It is worth building now
that requests can succeed: a signing surface a player cannot review after the
fact is one where a mistaken grant leaves no trace.

## 5 · Gates on it

Gates is `in-build` and **desktop-native** — a native Rust (Bevy) client
against the authoritative Rust server, one binary through the depot. It was
browser-native here until 2026-08-06 (*"We use desktop build no more web"* —
the three.js client is deleted), and the alpha is the desktop build: **there
is no browser alpha** (operator, 2026-08-09). Its manifest ships with **both
rows empty**, and that is the correct state, not an omission — the native
row fills the day `published.json` names a build (§8 row 1b), and the
browser row is the slot that stays empty for this title.

**If any title ships a browser build:** set `play_url` on its catalog row
and the manifest's browser row fills itself (`meter/launcher.py::_play_url`)
— the client's Play button lights, zero launcher work. The wiring is live
and dormant; it is not Gates' path.

**The desktop build landed, and so did its packager** (2026-08-05). Gates'
`crates/client` is a native Bevy client, and `Gates/ci/depot.py` turns it into
a depot: compile, stage, hash, write the index. It is one command and the
output installs — measured on 2026-08-05, a depot went from
`scry install gates` to a verified install on disk, and the digest computed by
the packager's own document, by the origin's response and by the client agreed
exactly.

⚠ **A depot is the binary AND everything it opens at runtime**, and the first
Gates depot got this wrong before the client's render slice landed. Bevy
answers a missing texture with a white fallback *and keeps drawing*, so a
binary-only depot installs perfectly, starts perfectly, and renders an
untextured world — no error, no failed hash, nothing for the launcher to
refuse. Nothing in this layer can catch that: every rule here is about the
files a depot *names*, and a file it does not name is not a file it is wrong
about. **It is the publisher's job, and the packager's `--self-test` is where
it belongs.** Worth stating because the seam invites the mistake: the launcher
asks a game for a file list, and a game that answers "just the exe" is
answering a narrower question than the one that matters.

Three things that packager does NOT do, each on purpose:

- **It does not compute the digest.** `scry digest` does. Gates implementing
  the number that gets notarized would be a second implementation of it, which
  is invariant 3 with money attached. If `scry` is on PATH the packager shells
  out to it; otherwise it prints the command and stops.
- **It does not publish.** It prints the two commands — an `rsync` and the
  one-line `published.json` — and performs neither.
- **It does not bundle system libraries.** It *measures* them instead. The
  binary's `DT_NEEDED` list is read off the ELF and recorded as
  `requires.libs`, because the Bevy build genuinely needs `libwayland-client`,
  `libudev` and `libasound` from the player's machine — a fact discovered the
  hard way, when the first `--features render` build died in `wayland-sys`'s
  build script. Shipping one distro's copies of those to another trades a clear
  error for a confusing one; naming them lets a client say which is missing.

What the depot needs, and nothing more:

```json
{ "depot_version": 1, "slug": "gates", "build": "0.1.0",
  "platform": "linux-x86_64",
  "root": "https://…/depot/gates/0.1.0",
  "files": [ {"path": "gates", "sha256": "…", "bytes": …, "executable": true},
             {"path": "lib/libgates_sim.so", "sha256": "…", "bytes": …} ],
  "launch": { "exec": "gates",
              "args": ["--server", "{server}", "--identity", "{wallet}"],
              "env": { "LD_LIBRARY_PATH": "lib" } } }
```

`launcher/examples/gates.depot.example.json` is that file with every field
filled, and `test_launcher.py` parses it so the shape cannot rot before the
first real build exists.

**Why delivery shape is never a fairness question:** the deterministic core
means any build of Gates is the *same simulation* — same seed, same inputs,
same state hashes. Which shape a player runs (today the native binary; a
browser build if one ever returns) is a delivery choice and never a fairness
one, so the launcher does not have to care, and neither does the record.

**And it can already sign, before any of that ships.** Gates links
`sdk/rust/scry_overlay.rs` — one file, `std` only, no crates — and gets
identity and signatures off the broker with no key anywhere in the game
process. `sdk/test_sdk.py` compiles that client and drives a real broker with
it, so the Rust half and the Python half cannot drift. `docs/client/SDK.md` §5 is
the code.

**Agents get the same door.** The sim is an RL environment by construction
(`GATES.md` §0), and a depot is a better distribution shape for a training
substrate than a browser is: pinned bytes, a hash, a fixed argv. The client is
one consumer of the manifest; a harness is another — and it signs through the
same bounded, journaled arca a human does, rather than being handed a raw key
because it was convenient.

## 6 · The shard list

`Servers` is the client's *"browse multiplayer games-in-progress"*. It reads
`manifest.servers.url` and renders the rows. It does not invent, cache or rank
them — **the shard list is the game's to serve.** Until a title publishes one,
the window names the field that would fill it rather than showing an empty
grid, which is the same discipline every dark panel in the town follows.

The shape (`scry-shardlist-v1`, and it is a **(knob)** until Gates has shards
to list):

```json
{"servers": [{"id": "eu-1", "name": "…", "addr": "host:port",
              "players": 47, "max_players": 100, "map": "…", "ping_ms": 31,
              "status_url": "https://…/status.json"}]}
```

Joining passes `addr` into the launch args as `{server}`. Every field but
`id`, `name` and `addr` is optional, and an absent count draws `?` — never a
`0`, which would make a busy shard read as empty.

`scry servers <title>` is the same read from a terminal, and `--json` gives it
back machine-readable with each row's join link.

### Where a title publishes it — `GET /api/launcher/servers/{slug}`

**A game may serve its list anywhere; this origin will also hold it, because
the url games were told to publish at never existed.** `Gates/ci/shardlist.py`
printed `https://scry.moreright.xyz/depot/<slug>/servers.json` and `/depot/`
is not a location here — depot bytes have always come through `launcher.py`
— so the one thing a title was told to do could only have 404'd, which is the
whole of why `servers.url` stayed null while a real shard was up.

Drop the document at `$SCRY_DEPOTS_DIR/<slug>/servers.json` and it serves at
that route, **byte-for-byte**: the origin ranks nothing, merges nothing,
caches nothing beyond a 60-second `Cache-Control`, and adds no field the game
omitted — an absent `players` stays absent. §6's rule is untouched, because
serving a file a game wrote is the same relationship this origin already has
with that game's depot.

Three states, because the window draws three: **404** the title publishes no
list, **503** this origin could not read its depot root (*"nobody knows what is
up right now"*, never *"no shards"*), **200** the game's own bytes. Refusing an
oversized file whole rather than truncating it is the fourth, and it is a 503
for the same reason — a truncated list is a parse error with no explanation.
`meter/test_launcher_api.py` §15 drives all of them through the edge's prefix
transform.

### `status_url` — the count is polled, not read

A row may name where *that shard* answers `GET /status.json`
(`{"players":3,"max_players":100,"tick":123456}`). Additive and optional: the
kind stays `-v1`, and a reader that predates the field ignores it.

It exists because **a count inside the document is stale the moment the file is
written.** A title generates its list and serves it for hours or weeks, so a
`players` baked in describes whenever someone last ran their generator — and a
count that is confidently wrong is worse than the `?` it replaced. So the row
names an endpoint and the reader asks it: the Servers window on open, and the
game's own intro screen every ten seconds.

This is §6's rule one layer down, not a hole in it. The launcher still does not
invent, cache or rank anything — it asks each shard about itself, exactly as it
asks each title's origin about its list. Nothing is intermediated; everyone
measures. A shard that does not answer keeps whatever the row said.

## 6b · Join links — `scry://join/<title>/<host:port>`

**A friend pastes a link and the game comes up on the shard they are standing
on.** The launcher owns the scheme rather than the game, and every reason is
the same reason — the launcher is the piece that is *installed*: it already
substitutes `{server}`, it already holds the account the shard will ask to
sign, one scheme serves every title, and **a link for a title the player has
not installed is still useful**, because the launcher can say so and offer to
install it. A game-owned scheme cannot: the game is what is missing.

| door | what happens |
|---|---|
| `scry join <link>` | resolves it and becomes `play <slug> --server <addr>` — not a second launch path, so playtime, presence and session records cannot drift |
| `scry <link>` | the desktop hands a handler its url as `argv[1]`; it is rewritten to `join` before dispatch, so the clicked path and the typed one are one path |
| the Servers window | **Invite** copies the canonical link for a row |
| Gates itself | parses `scry://join/gates/…` and `gates://host:port` on its argv, for a url handed straight to the game |

**A link may name a title and an address and nothing else.** Not an identity,
not a token, not a flag, not a path — anything past the address is *refused*,
never ignored, because ignored is how an unnoticed segment becomes a supported
one. That narrowness is the security property and not an unfinished feature: a
join link is untrusted input from a stranger by construction, since being
pasted into a chat window is its whole purpose. The slug is checked against the
manifest slug's character class (it reaches a url and an install directory),
and the address gets the same shape check a typed one does.

**Registering the scheme is packaging, not code.** The `.deb` ships a hidden
`scry-join.desktop` carrying `MimeType=x-scheme-handler/scry;` and a `postinst`
that runs `update-desktop-database` — without that rebuild the handler is
installed, correct, and never called. From a tarball, by hand:

```sh
xdg-mime default scry-join.desktop x-scheme-handler/scry
```

Windows has no equivalent yet; it is a registry key under
`HKCU\Software\Classes\scry` and nothing writes it.

## 7 · What is dark, and why

Nothing here is a finding — it is the baseline restating itself, and the
client prints it rather than hiding it.

| surface | state | what would light it |
|---|---|---|
| the pass block in the Store | **gold, "NOT ARMED"** | a pass contract and a price (`GATES.md` §10 row 1) |
| items, skins, keys in the Library | **dark** | a catalog arming |
| Servers | **dark, and now only for want of the url** | any title publishing `servers.url`. The window is built: it fetches the list, polls each shard's `status_url` for a live count, and offers Join and Invite per row (§6, §6b). Gates' manifest still ships `servers.url: null`, so the panel draws its unpublished state — one operator act away from rows |
| Gates' Play button | **not dark any more** — the listing is `state: "live"` and a build is published, so the row draws lit. It still reads **gold, "In build"** on any origin with nothing published | `published.json` naming a build — the row is measured, not typed, so this row answers itself per origin |
| native installs | **not dark any more** — two depots published and notarized 2026-08-10 (§8 rows 1b–2). Read the row, never this cell: `GET /api/launcher/manifests` → `builds[].depot_state`. ⚠ the `win-x86_64` build is **cross-compiled and has never been started on a real Windows machine** — it builds, links, hash-verifies and installs, and that is all anyone has seen it do; its manifest row says so, because a player reads that row | — |
| the town's rooms | **live, and measured every load** | — they work today |

The rooms are the honest answer to *"what can I actually play in this thing."*
Twelve of them, browser-native, and the client opens them.

## 8 · What must be built

> **Run the gate rather than reading this table** (added 2026-08-10):
>
> ```
> python3 deploy/launch_preflight.py gates            # on the origin
> python3 deploy/launch_preflight.py gates --rpc <4663 rpc>
> ```
>
> It checks every operator act below that a launch actually turns on — depot
> built, **published** (`published.json` naming the build, not a directory
> appearing), every file the depot names present on disk, the digest computed
> and **notarized on chain**, `ScryGameTicket` deployed and armed, and the
> listing's own fields coherent with all of it.
>
> **It reports three states, not two.** `UNKNOWN` — no depot root on this box,
> no RPC given — is not a pass, and the run exits non-zero on one. That is the
> `CLAUDE.md` trap (*a never-raises reader returns `[]` for both "nothing
> there" and "we could not look"*) pointed at the one moment it costs most: a
> launch gate run off-box would otherwise print green having looked at nothing.
>
> ⚠ **Run it with the venv on the origin** —
> `/data/apps/scry-deploy/meter/.venv/bin/python deploy/launch_preflight.py
> gates`. It grades the **effective** listing (base + overlay), and reading the
> overlay needs the shelf module: on bare `python3` the run says
> `UNKNOWN — the overlay was not readable from here` rather than pretending the
> committed floor is the answer. It used to read the floor and nothing else,
> which made it report the *pre-launch* state of a title already flipped live
> from the desk, and skip the depot, ticket and storefront checks that flip is
> supposed to have passed.
>
> The drift this table cannot catch used to be `state` itself — **written in
> BOTH `listings.json` and `data/launcher/<slug>.manifest.json`**, the store
> reading one and the client the other. That is cured rather than gated now:
> the origin derives the manifest's `state` from the listing (§11b2), so there
> is one author, and what the gate checks is the residue — a `live` floor in
> the manifest file under a listing that is not live, which is the only copy
> that can still advertise a launch nobody made.

| # | deliverable | state | operator act? |
|---|---|---|---|
| 1 | a Gates depot for `linux-x86_64` | **BUILDABLE, AND DRIVEN END TO END 2026-08-06; PUBLISHED 2026-08-10** — `Gates/ci/depot.py` compiles the Bevy client, stages it, hashes it and writes the depot index. Run it and you get a real one; row 1b is the publish, and it has happened. §8b is what the full chain actually did | done |
| 1b | publish that depot | **DONE 2026-08-10 — and it was two, not one.** `linux-x86_64` `0.1.0-g0c487ad97` (installed and verified end to end) and `win-x86_64` `0.1.0-g38e8bf038`, both staged into the origin's depot root with `published.json` naming them. ⚠ **Publishing `win-x86_64` did nothing visible at first**: the manifest fills native SLOTS that already exist and never creates a row, so the depot served fine over HTTP, `scry games` was happy, and the manifest kept showing one row — found by reading the rendered manifest instead of stopping at a 200. Gates' manifest carries both rows now | done |
| 2 | notarize the depot digests | **DONE 2026-08-10 — both of them**, on `ScryNotary` `0x0C15fA78…` under label `gates-build`: `linux-x86_64` `0x50fef181…8fbb`, `win-x86_64` `0xdf98de59…45cc`, committed by the deployer wallet and recorded in `data/launcher/gates.manifest.json` §notary. `scry digest <depot>` recomputes either from the depot file, and anyone can look it up | done |
| 3 | `scry-shardlist-v1` served by Gates | shape drafted here, unbuilt | the shape is a **(knob)** |
| 4 | serve manifests off the origin (`/api/launcher/manifests`) | **BUILT 2026-08-05, REACHABLE 2026-08-06** — `meter/launcher.py`. Manifests, depot indexes and depot bytes, so `scry install gates` is a slug and not a url. §11 is the design. ⚠ It read *"DONE"* for a day while answering **404 to every client** — §11e is what happened and why no gate saw it | no |
| 5b | the NATIVE client, packaged for **Linux and Windows** | **DONE 2026-08-06** — `launcher-rs/build_release.py` writes `scry_<v>_amd64.deb`, `scry-<v>-linux-x86_64.tar.gz` and `scry-<v>-windows-x86_64.zip`. §10b9 is what it does and the two defects building it found | no |
| 6 | **the ticket flow** — a sold title, end to end | **BUILT 2026-08-08** (`TICKET.md` is the lane's record): `scry entitle <slug>` does the whole dance in one command — the exact text fetched from the origin, one passphrase prompt against the local account, the day grant cached in `grants.json` beside the games, and install/update pick it up with no flags. `SCRY_GRANT` set by hand still wins, and the grant header rides only to the configured origin (`tests/grant.rs`). ⚠ the depot gate itself ships **OFF** (`SCRY_TICKET_GATE_DEPOT` — the servers are the wall, operator 2026-08-08), so this path idles until a deployment arms it; nothing else changes for free titles | arming the gate is an env |
| — | macOS | the platform enum has the row; nobody has built it | later |
| — | Windows and macOS depots | the platform enum has the rows | later |
| — | delta patching | **the DOWNLOAD half is built and the UPLOAD half is one rsync flag — §8d.** What stays unbuilt is *sub-file* delta, which only earns its machinery once a single pak is big enough that rewriting it whole is the cost | later, and only if a build gets big enough to earn it |

⚠ **This is what THIS layer needs — it is not "what next."** `NOW.md` owns the
platform's ordering and wins on order (`GATES.md` §0b).

### 8b · The chain, driven end to end (2026-08-06)

**Every link above row 1b was run against a real build, not a fixture** — the
prefix defect (§11e) was proof that the local suites do not cover this seam, so
the seam was walked. Re-derive it the same way; nothing below is estimated.

The build: `cargo build --release -p client --features render --bin gates`,
8m29s, 90,644,368 B stripped to 68,835,064. Packaged by `ci/depot.py` into
**37 files, 75,829,730 B**, build id `0.1.0-g607af0314`, with `requires.libs`
measured off the ELF as seven sonames (`libasound.so.2`, `libudev.so.1`,
`libwayland-client.so.0`, …) rather than bundled.

| step | what happened |
|---|---|
| the origin measures the row | `depot_state: published`, `bytes`, `files: 37` and `depot_digest` all read off the real document — nothing typed |
| **the digest agrees across both implementations** | `scry digest` and the origin's `depot_doc.digest` returned the same `0xf43be2a2…f56c` on a 75.8 MB depot. The twice-implemented digest — the client's and the origin's — is pinned by frozen golden vectors; this is the same claim on real data |
| `scry install gates` | fetched all 37 by slug, hash-verified each, moved into place, reported the digest |
| `status` / `list` | up to date; 72.3 MB against the digest |
| `play` | **started the real binary**, which parsed the depot's own `--server`/`--identity` argv |
| the SDK ran | `gates: playing anonymously — no scry launcher` — `Scry::discover()` found no broker and degraded exactly as `docs/client/SDK.md` requires, rather than asking for a key |
| the updater | a second build changing 1 of 2 files fetched **only the changed one** — confirmed in the *server's* access log, not from the client's own report, and the prior build's file was reused off disk and re-hashed |
| builds side by side | both remained installed; `prune` stayed a separate act |

**Two things this closed that were open.** The download path had never faced
real HTTP because the suite injects an opener — it has now, installing the same
75.8 MB depot and verifying all 37 hashes. And the *"64.7 MB build"* this
section used to cite was a different, earlier build on another box; the numbers
above replace it.

⚠ **What this run did NOT clear:** row 1b. Nothing in it published anything —
it proved the chain works, not that it had been walked in public. **Row 1b
closed four days later, on 2026-08-10**, with two depots rather than one; the
table above is the state and this section is the rehearsal that preceded it.

### 8c · Re-derived on a container that had never built it (2026-08-09)

§8b was run on a box that already had the client. This one started from a
clone with no `-dev` packages, which is the question §8b could not answer:
**is the depot reproducible off one machine, or only on the operator's?**

`apt-get install libwayland-dev libasound2-dev libudev-dev`, then
`cargo build --release -p client --features render --bin gates` — **13m13s**,
95,083,536 B stripped to 72,445,472. `ci/depot.py` staged 98 asset files
beside it: **99 files, 82,858,306 B**, build `0.1.0-gd39b79384`, seven sonames
measured off the ELF. (§8b's 37 files / 75.8 MB was three days and an art
pass ago — the shape is the claim, never the number.)

| step | what happened |
|---|---|
| **both digest implementations agreed again**, on a bigger depot | `scry digest` and `depot_doc.digest` both returned `0x427adb82…cc6f` |
| the origin served it **through the edge transform** | `NginxClient`, not a bare `TestClient` — all 99 declared files served and hash-verified, the document byte-for-byte, and a `leftover.key` dropped into the build directory refused 404 because the index is the allowlist |
| a real `uvicorn` + a real `scry install` | repackaged against a local root (the root is baked into the digest, so it changed to `0x23e545f0…5d4f` — the property working, not a fault). `games` listed it, `install` fetched all 99 over HTTP and hash-verified each, `verify` re-hashed all 99, `status` read up to date |
| what landed on disk | a 72,445,472 B ELF pie executable at the depot's own path |

**This is what the nightly `depot` job now does unattended**
(`Gates/.github/workflows/nightly.yml`), which is the actual finding: before
it, every depot Gates ever had was built by hand on one box.

⚠ This run did not clear row 1b either, and for the same reason: nothing in it
published anything. **The publish came the next day** (row 1b), which is what
made the finding above matter rather than merely being tidy.

### 8d · The SECOND build — the upload half of the delta (measured 2026-08-09)

Operator, 2026-08-09: *"does uploading a new build do the thing where i dont
have to upload the whole thing only diff?"* §10a answers that for the
**download** and the answer is yes. For the **upload** the answer was no, and
this is why and what fixes it.

**The layout is a full tree per build**, by design — `meter/launcher.py`:

```
$SCRY_DEPOTS_DIR/<slug>/published.json     {"linux-x86_64": "0.1.0"}
$SCRY_DEPOTS_DIR/<slug>/<build>/depot.json
$SCRY_DEPOTS_DIR/<slug>/<build>/<file>     the bytes, at their depot paths
```

Nothing on the origin is content-addressed, and that is not an oversight: a
depot document is served **byte-for-byte** at the paths it names, because the
digest is taken over the whole document (rule 2, same file). A content-addressed
store on the origin would be a second layout to keep honest for no gain the
client can see.

But it means each build is a **new empty directory**, so a plain
`rsync staged/ box:…/gates/0.2.0/` has nothing at the destination to compare
against and sends every byte. rsync's delta only fires against a file already at
the destination path.

**The fix is one flag**, `--link-dest`, pointing at the build already on the
box. Unchanged files are hardlinked from it: nothing goes over the wire for
them, and they cost no second copy on disk.

```sh
# the build currently on the box, from the publish pointer itself — never guessed
PREV=$(curl -s https://scry.moreright.xyz/api/launcher \
       | python3 -c 'import json,sys; print(next((t["published"].get("linux-x86_64","")
           for t in json.load(sys.stdin)["titles"] if t["slug"]=="gates"), ""))')

rsync -a --stats ${PREV:+--link-dest=../$PREV} \
      staged/ box:/data/apps/scry-data/depots/gates/0.2.0/
```

⚠ **Read `/api/launcher`, not the manifest.** The card's `titles[].published` IS
`published.json` read back — `{"linux-x86_64": "0.1.0"}`, the pointer that
decides what is live. The per-title manifest carries the same build under
**`version`**, and its `build` key is `None` on a filled native row, so the
obvious-looking `.builds[].build` yields an empty `PREV` and silently drops the
flag. Both shapes were run against the real route before this was written.

**Measured on a fixture shaped like a real build** — 4 files, 47 MB, of which a
40 MB pak and a 6 MB `.so` are unchanged and a 3 MB binary and a 200 KB ui pak
are not:

| publishing 0.2.0 | files transferred | bytes sent | disk on the box |
|---|---|---|---|
| plain `rsync -a` | 4 of 4 | **49,322,365** | 94 MB |
| `--link-dest=../0.1.0` | 2 of 4 | **3,311,055** | **51 MB** |

**14.9× less uploaded, and the box stops storing a second copy of every
unchanged byte.** Three properties checked rather than assumed, because a
hardlink farm is exactly the kind of thing that looks fine until it isn't:

- **What gets served is unchanged.** All four files in `0.2.0` hash identical to
  the staged tree. `--link-dest` decides where bytes come from, never what they
  are.
- **The unchanged pak is one inode in both builds** (`stat -c %i` agrees); the
  changed binary is two.
- **A later ordinary rsync into `0.1.0` does not touch `0.2.0`.** rsync writes a
  temp file and renames, so the link *breaks* rather than both builds moving.

⚠ **Never add `--inplace`.** It is the one flag that turns this safe, and it was
tested rather than reasoned about: with `--inplace`, writing into `0.1.0` changed
`0.2.0`'s bytes too — a **published** build silently becoming something its own
`depot.json` no longer describes. The client catches it (every file is re-hashed
on install, and a mismatch installs nothing), so the failure is loud for the
player and silent on the origin, which is the worse half to be silent.

Two more things worth knowing before the first second build:

- **`published.json` is still the release act**, unchanged. `--link-dest` is
  about how the bytes arrive; a tree on disk is not live until the pointer names
  it (rule 3), which is exactly what makes an in-flight upload safe.
- **`PREV` empty is a real answer, not a failure.** The `${PREV:+…}` guard drops
  the flag when the origin names no build — the first publish, or an origin we
  could not reach. A missing `--link-dest` costs upload time; a **wrong** one
  costs nothing at all, since rsync transfers whatever does not match. Guessing
  it is safe; the guard is there so the command does not print a path that is
  not on the box.

**Still to do on the Gates side**, and it is a printing change rather than a
mechanism: `ci/depot.py` prints the publish commands and performs neither
(§8 above). It should print the `--link-dest` form with `PREV` resolved, so the
delta is what gets typed by default instead of something a publisher has to
remember.

## 9 · What this does not change

By pointer, because restating a wall weakens it:

- **The ten invariants of `CLAUDE.md`**, unmoved. Nobody can pay for a better
  score — the one test applied to this layer: *would the payer's identity or
  the amount change any bit of any output?* No. A launcher installs, opens and
  reads; it grades nothing, and there is no output here to buy.
- **We hold no user keys.** §1 and §4 are the implementation.
- **SCRY is the only reserve**, and this client mints, prices and pairs
  nothing.
- **The register rule.** The windows say what you put in, what you get back,
  and what is not built yet. "Depot", "manifest", "shard" are the words the
  thing is actually called in its own file format, not flavour.

## 10 · The Rust port, and the updater

> Operator, 2026-08-05: *"lets build this in RUST… we need the updater
> eventually too, and a game updater like steam has with builds… this is a
> launcher for desktop games not so much web."*

**Three requirements pointed the same way, and only one was preference.**
Distribution — first-run then was `sudo apt install python3-tk`, where a Rust
build is one 1.8 MB file. Hive
presence needs a WebSocket client, which stdlib Python does not have. And a
desktop companion needs per-pixel-shaped windows, which Tk has no binding for
on X11. Gates is already Rust, so it is also one toolchain.

**`launcher-rs/` is the beachhead and it is deliberately not the windows.**
What moved is `depot.py`: 500 lines of pure logic, no UI, the highest-risk
code in the client — it writes files a stranger named — and the piece a
game's own updater may want to link. The rules survived the move before any
pixels did.

### 10a · What the updater does

| | |
|---|---|
| notices | an install records its `depot_digest`; stale is that disagreeing with the published depot's |
| keeps builds side by side | `<games>/<slug>/<build>/`, so a failed patch is never an uninstalled game. `prune` is a separate act |
| **downloads only what changed** | and needs no delta format — see below |
| never lies about being current | `UpdateState::Unknown` is its own arm; `scry status` exits 3 for unknown, 10 for stale |

**The delta falls out of content addressing.** Every depot file is identified
by sha256, so a file that did not change between two builds *is the same file*
and the prior install already has it. Steam dedups at chunk level; file level
gets most of that win for a fraction of the machinery, because a patch usually
rewrites a few large paks and leaves the rest untouched. Measured on the smoke
fixture: a build changing one of three files fetched **117 KB and reused 39 KB
off disk**, and the test asserts the server was asked for the changed file and
nothing else. A reused file is re-hashed exactly like a fetched one — the old
build's directory is writable by whoever runs this.

⚠ So §8's *"delta patching — later, and only if a build gets big enough to
earn it"* is **partly answered already**. What remains unbuilt is *sub-file*
delta, which only matters once a single pak is large enough that rewriting it
whole is the cost.

**Two limits worth knowing before a big build**, because neither is visible
from the win above. The reuse is **file-level, not chunk-level** — a 2 GB pak
that changed by one byte is re-fetched whole, which is exactly the case Steam's
chunking exists for. And reuse only comes from **prior installs of the same
title on that machine**: a fresh machine fetches everything, and nothing is
deduped across titles. `prune` interacts with this — it removes superseded
builds, which is what the old build's directory *is*, so pruning before an
update throws the delta away. Update first, prune after.

⚠ **This is the DOWNLOAD half.** The upload half is a different question with a
different answer — §8d.

### 10b · The transport, which had never run

`test_launcher.py` covers the install *logic* well, but it injects `opener=`,
so `_download` never faced real HTTP and no real binary was ever launched.
That was the highest-risk untested path on the critical path for every native
title. `launcher-rs/crates/scry-launcher/tests/transport.rs` stands up an
actual TCP server and covers install-over-the-wire, a truncated body, a 404, a
5xx, an unreachable origin, a redirect, launching a real executable with argv
and `LD_LIBRARY_PATH` applied, and an update that fetches only what changed.

**Two defects it found, both in the product rather than the port:**

1. **`ureq` ignores `NO_PROXY`.** It reads `ALL_PROXY`/`HTTPS_PROXY`/
   `HTTP_PROXY` and applies them to everything — so a client on a proxied
   network would tunnel `--host http://127.0.0.1:3600`, a LAN depot mirror,
   and its own tests. Worse, the proxy *answers* for a dead port, which
   defeats `reachable` one layer below §2. `scry-net/src/proxy.rs` picks the
   agent per request.
2. **`play` started the oldest installed build.** Builds sort by name and a
   title can have several installed, so taking the first match ran `0.1.0`
   immediately after updating to `0.2.0` — a patch that reads as having done
   nothing. Found by driving the real binary, not by any unit test.

### 10b2 · The windows are the rest of the port, and the toolkit is a budget question

> Operator, 2026-08-06: *"why we using python at all can we go pure rust"* —
> asked about **the launcher and the games**, not the meter.

**The logic is nearly ported already; the windows are what is left.** Measured:
`launcher/` is 6,011 lines of Python, `launcher-rs/` is already 5,760 of Rust.
What has no Rust counterpart is `windows.py`/`chrome.py`/`theme.py`/`app.py`
(~2,200 lines of tkinter) and `broker.py`/`signer.py`/`vault.py` (~1,400, the
socket-and-key half). **The broker half is owed regardless of toolkit** and is
also §12 row 1, the Windows blocker — it needs no new dependency at all.

**The toolkit is where it stops being a coding question.** `supply_chain.rs`
sets `MAX_PACKAGES = 110` against a tree of 96 today, and that budget is an
operator sentence about npm-style supply-chain attacks (`SENTENCES.md`
2026-08-05), not a lint. Measured with `cargo generate-lockfile`, 2026-08-06:

| toolkit | packages it adds | build needs | binary |
|---|---|---|---|
| **fltk** | **15** | cmake + g++ + X11/Xft/pango/cairo dev headers | **1.5 MB**, links X11/fontconfig/freetype — present on any desktop |
| eframe/egui | 402 | pure Rust | larger |
| iced | 414 | pure Rust | larger |
| slint | 531 | pure Rust | larger |

**The trade is Rust crates against C system packages, and it is not close on
the axis the budget is about.** fltk fits the existing posture with one crate
of headroom to spare; every pure-Rust toolkit is 4–5× the *entire current tree*
and would retire the budget rather than raise it. What fltk costs instead is a
C++ toolchain at **build** time — verified the hard way here: it failed twice,
first on Xft and then on pango, before linking. That cost falls on whoever
builds a release, never on a player, and the 1.5 MB result is the §10 target.

Two things fall out that are worth saying: the FLTK look is close to the
**2003/2004 client this skin is already a rip of** (§0), so the cheap option is
also the on-brand one.

### 10b3 · Decided, and the first windows are up

> Operator, 2026-08-06: *"fltk it is"*.

**`crates/scry-ui` exists and draws.** The main menu, Games and About render
under FLTK, and the budget did not have to move: the tree went **96 → 105
against the cap of 110**, because bitflags, once_cell and the crossbeam pair
were already in it. A whole toolkit landed inside the existing supply-chain
posture with headroom left.

Three rules the crate is built on, each carried across rather than reinvented:

* **The windows draw; they do not decide.** `windows::games` takes rows, not a
  games root — no I/O, no clock, no origin. Every fact on screen comes from
  `scry-depot`, which is the crate whose tests cover it. Same rule Gates keeps
  on its renderer, for the same reason.
* **The reservation is enforced by the type, not by review.** `Tone` has no
  variant pairing a green fill with an unarmed state, so a caller cannot ask
  for one. `tests/palette.rs` asserts as a **count** that exactly one tone
  paints the reserved green — the failure that guards against is a *second*
  tone quietly acquiring it, which only counting catches. The same file reads
  the olive out of `theme.py` with `include_str!` so the two clients cannot
  drift; delete the Python theme and this stops compiling, which is the correct
  coupling.
* **The FLTK credit is a licence obligation and `cargo deny` cannot see it.**
  Both crates declare MIT — the binding — while `fltk-sys` vendors FLTK under
  LGPL-with-exceptions. Static linking is permitted and the licence text need
  not ship; *identifying the use of FLTK* is required. `tests/credit.rs`
  asserts both the wording and that the About window actually renders it, since
  a constant no window shows identifies nothing.

⚠ **One defect, and no test found it — a capture did.** The Games row reading
*"an update is published"* offered the same **Play** button as the row reading
*"up to date"*, so the sentence and the control disagreed and the control is
what a player acts on: they would have launched the stale build and never
learned there was a new one. Fixed to **Update**, and `tests/rows.rs` pins it.
That is `Gates/CLAUDE.md`'s *look at the picture before tuning the number*
arriving in this repo, and it is the argument for the capture harness the
the client needs, growing a Rust half.

### 10b4 · The broker is Rust, and it speaks to Gates' own SDK

> Operator, 2026-08-06: *"get us a pure rust launcher building on windows and
> ubuntu linux"*.

**`crates/scry-broker` is `broker.py` ported, wire unchanged.** The vocabulary
is the same seven verbs, fields are still allowlisted per verb, an unknown
field is still a refusal rather than an ignore, and every verb still waits for
`hello` to name the game. No new dependency: the wire is newline JSON over a
local stream and both halves are in `std` plus the `serde_json` already here.

**The test that says no game broke.** `tests/sdk_parity.rs` compiles
`sdk/rust/scry_overlay.rs` — the file Gates vendors byte-for-byte under a
sha256 pin — as a module, and drives it against this broker over a real socket.
Handshake, identity, a signature, and a player's refusal arriving *as a
refusal* rather than as a transport error. `sdk/test_sdk.py` still drives the
same client against the Python broker; neither replaces the other, and together
they are what makes swapping the server underneath a live ecosystem safe.

**What the broker does not own, it does not invent.** `Host` is a trait —
address, signer name, consent, signing, title urls, opening a page. There is no
default-allow path through it, which is what lets the whole protocol be tested
with no keys, no windows and no network.

⚠ **Consent is still in memory only**, and that is the design: a grant written
to disk survives the player forgetting they made it. A refusal is remembered
for the session so a game cannot re-prompt in a loop — a different outcome from
a spent grant, and the test asserts the *ask count*, not just the reply.

### 10b5 · Building on Windows and Ubuntu, verified

**Both targets build, and the Windows half is no longer a type-check.**

| | Ubuntu | Windows |
|---|---|---|
| every crate | builds, 168 tests | `cargo check` clean, all six crates |
| the CLI | `scry` | **`scry.exe`, PE32+, 1.9 MB** — linked, not checked |
| the windows | FLTK on X11, 1.5 MB | **FLTK on Win32, `.exe` linked, 1.6 MB** |
| the broker | UNIX socket | **named pipe** (`\\.\pipe\scry-launcher-<user>`) |

**The Windows GUI binary imports stock DLLs and nothing else** — `GDI32`,
`USER32`, `COMCTL32`, `SHELL32`, `gdiplus`, `ole32`, `comdlg32`, `ws2_32`,
`msvcrt`. No redistributable, no runtime to install, one file. That is the §10
promise arriving on the platform that had no story at all a day ago, and it is
the payoff for the toolkit measured on the dependency budget rather than picked
by taste: a pure-Rust toolkit would have cost 400 crates to reach the same
place.

**The MSVC wall is real and mingw goes around it.** `ring` (via `ureq`'s TLS)
runs a C build script wanting `lib.exe`, so `--target x86_64-pc-windows-msvc`
cannot get past it on a Linux box — §12 recorded that as a toolchain fact and
it still is. `x86_64-pc-windows-gnu` with `mingw-w64` builds the whole tree
including FLTK's C++, and `.cargo/config.toml` carries the linker so the check
is one command. **A real Windows release is still built on Windows with MSVC**;
what this buys is that the Windows build cannot silently rot between releases.

⚠ **Compiling and linking are not running.** Nothing here has been *executed*
on Windows. The named-pipe transport is written against the documented Win32
contract and type-checks, and that is all it is — `sdk_parity.rs` is `#![cfg(unix)]`
on purpose rather than faked green on a platform it never ran on.

~~**The SDK's own client is still `UnixStream`-only**, so a game on Windows
finds no launcher.~~ **Closed** — both reference clients grew the far half:
`sdk/rust/scry_overlay.rs` waits on `WaitNamedPipeW` under `cfg(windows)`, and
`sdk/python/scry_overlay.py` opens the pipe as a file under `os.name == "nt"`,
since CPython exposes no `AF_UNIX` there. Re-derive rather than trust this
line: `grep -n 'cfg(windows)' sdk/rust/scry_overlay.rs`. It is still fixed
upstream and re-vendored, never patched in the game.

### 10b6 · `scry-gui` — it is a launcher now, and a real game found it

Until this binary existed there was a CLI, a window *library*, and no launcher.
`crates/scry-ui/src/bin/scry-gui.rs` is the program: it opens the menu, reads
what is really installed through `scry-depot`, and serves the game door through
`scry-broker` for as long as it is open.

**Measured end to end on 2026-08-06, and the line that changed says it:**

```
gates: playing anonymously — no scry launcher        ← before
gates: scry launcher connected, no address set (signer: none)   ← after
```

That is the **real 75.8 MB Gates build** — the one installed by slug in §8b —
connecting to the Rust launcher through the SDK Gates vendors under its sha256
pin. Pure-Rust launcher, real game, ported broker, unchanged wire.

**The CLI stays separate and does not link FLTK.** A headless box, a CI job and
an RL harness have no use for a window, and a launcher operable only through
one is the wrong shape for half its users. `scry` and `scry-gui` read the same
crates; neither is the real one.

| binary | Ubuntu | Windows |
|---|---|---|
| `scry` (CLI) | builds | **`scry.exe` 1.9 MB** |
| `scry-gui` | runs, serves the door | **`scry-gui.exe` 1.75 MB** |

⚠ **What it does not do is visible in it rather than in a list**, which is why
this paragraph no longer enumerates it: an unbuilt window renders deactivated
and says *"not built yet"* (`windows::MENU` carries that per entry), and `sign`
returns a readable refusal rather than a fake signature — a game handed a
forged one will send it somewhere that checks it. As written on the day, this
row also said `signer_name()` answers `none` and `ask_consent` always refuses;
both have since been built out (§10m, §10n) and the menu now has no unbuilt
entry at all.

### 10b7 · All six windows, and four of five signers

**The menu has no dead entries.** Store, Servers, Account and Signing joined
Games and About, so nothing renders deactivated any more — and the `built` flag
and its "not built yet" styling **stay**, because the next window added will
need them.

That change tripped a test on purpose. `rows.rs` asserted *at least one* entry
was unbuilt, with a note saying the assertion should be re-scoped if that ever
stopped being true. It stopped being true and the test failed — which is the
assertion working: a prompt to re-scope, not a thing to delete quietly. It now
asserts the property that actually matters, that no menu entry is a dead
button, plus a second test keeping the not-built path covered now that no live
entry exercises it.

What each new window refuses to fake:

* **Store** carries the pass block in **gold with NOT ARMED** — an outline,
  never a fill, so it cannot be mistaken for armed — and says in plain text
  that it buys nothing today. Its shelf distinguishes *"nothing listed"* from
  *"the catalog could not be read"*, which is the repo's own trap at the one
  place it would cost a player a game.
* **Servers** draws rows when a title publishes `servers.url`, and names that
  field when none does — rather than showing an empty grid. The shard list is
  the game's to serve. Its three empty states are three different screens on
  purpose: *no list published*, *nobody is running one*, and *we could not
  look* are different facts, and the third must never render as the second.
* **Account** says an address is **a claim, not a login** — anything that
  matters asks for a signature and verifies it.
* **Signing** names the backend and what it can do, with the test button
  deactivated while the backend is `none`.

**`scry-broker::signer` is four of the five backends**, and the fifth is
deliberately absent. `none` refuses and says what would change that; `browser`
**hands off rather than failing**, because a game that renders a handoff as an
error tells the player something broke when nothing did; `arca` reimplements
only the client half, since the recogniser, the count, the journal and the key
are all on the far side of that socket — *a cap enforced by the caller is not a
cap*; `external` runs a command the **player** named, split without a shell, and
**refuses output that is not 65 bytes of hex** so an error cannot travel as a
signature and fail far away with a useless message.

**Invariant 7 is asserted rather than promised here.** `tests/signer.rs` reads
`signer.rs` and fails if any non-comment line looks like key material — a
property the file has by construction, because all four work by asking someone
else. When `LocalSigner` lands, that test must be re-scoped per-backend the way
the Python module's was, and it says so in its own failure message.

⚠ **`LocalSigner` was the fifth backend and it has since landed** — §10b8
below is what happened, and this paragraph is kept because the *reasoning* is
the part worth keeping: it wanted secp256k1 + keccak, `k256` was already in the
tree, a keccak crate was not, and it was deliberately not hand-rolled per
`SDK.md` §2a — *signing belongs in an audited library rather than in code
written here* — because a subtly wrong nonce leaks the key silently. That was a
dependency decision, the same shape as the toolkit one, and it was made.

**The key lives in one crate.** `scry-vault` holds it, `scry account` operates
it with no display, and the Account window makes one.

### 10b8 · The vault, and the budget spent on not hand-rolling

> Operator, 2026-08-06: *"don't hand roll anything but yea do what we can."*

**`crates/scry-vault` is the local account, and it is the only crate in the
workspace that holds a key.** That split is the design: `scry-broker::signer`
stays keyless *by construction* and its test keeps asserting so against the
source, which would have had to become a per-backend scan if `LocalSigner` had
landed beside the other four. One crate holds keys and every other crate
provably does not, and it costs a file boundary.

**Nothing is hand-rolled, and that is what the packages bought.**

| | |
|---|---|
| secp256k1 + recoverable ECDSA | `k256` (already here for schnorr; gained `ecdsa`) |
| keccak-256 | `sha3` — the EIP-191 digest **and** the V3 keystore MAC |
| scrypt | `scrypt` at geth's standard n = 262144 |
| AES-128-CTR | `aes` + `ctr` |
| entropy | `getrandom` — the OS pool, never a userspace PRNG |

The cap moved **110 → 122** against a tree of 118, in the same commit as the
crate that spent it, which is what the policy requires. Eleven third-party
packages and none optional — the one that matters most is `rfc6979`, the
deterministic-nonce rule: a *repeated* ECDSA nonce publishes the private key,
and that is precisely the failure with no symptom until somebody else has the
money. Two packages were saved by measuring rather than reading a README:
`scrypt`, `sha3`, `aes` and `ctr` are all taken `default-features = false`,
which drops `password-hash` and `base64ct` for a string API this never calls.

**What this crate implements is the assembly, and only that** — which bytes go
into which primitive, in the order Web3 Secret Storage V3 and EIP-191 say. That
is the part with published answers, so it is checked against them: EIP-55's own
four example addresses, and the canonical `personal_sign` digest for
`"Hello World"`.

Three refusals worth naming:

* **The MAC is checked before the plaintext is used**, and a mismatch is a
  *wrong passphrase*, never "corrupt". Same bytes, different sentences — and
  telling a player their file is corrupt when they mistyped is how someone
  deletes the only copy of their key.
* **A keystore is never overwritten.** It may be the only copy of a key.
* **A document not fully understood is refused, not guessed.** A best effort
  yields 32 bytes that are not the key, and the first symptom is a signature
  nobody can verify. An unsupported KDF says what *can* open it.

The format is V3 on purpose: a keystore this writes opens in geth, in
MetaMask's import, in `cast wallet`. **A launcher that invented its own
encrypted-key format would be a launcher you cannot leave**, which is the moat
this client refuses everywhere else.

⚠ **One bug, and a round trip found it rather than a test.** V3 stores the
address lowercase, so a *locked* account rendered `0x8e66…` while the same
account *unlocked* rendered the EIP-55 `0x8E66…` — one account, two strings,
depending on whether a passphrase had been typed. A player comparing that with
their wallet cannot tell which is theirs. Fixed, and pinned.

**The rung is stated rather than implied:** this is rung 0 with worse isolation
than the browser, not rung 2. The process that draws the window holds the key,
so a compromise of the launcher is a compromise of the key. `ArcaSigner` is the
one with a real boundary. This exists because `STEAM.md` §4 is right that the
alternative loses most players at the door — not because it is the safe one.

**Nothing in the client is Python-only any more.** `launcher/` still runs and
still owns nothing the Rust half cannot do.

### 10b9 · Packaged, for both platforms — and it is downloadable now

> Operator, 2026-08-06: *"we need to work on whatever is left around the
> launcher you can download on windows and linux."*

**What was left was the packaging, and now `/download.html` hands out the
native client for both.** `launcher-rs/build_release.py` writes three
artifacts and the checksum list, deterministically in the container sense the
Python packager already meant:

| artifact | what it answers |
|---|---|
| `scry_<v>_amd64.deb` | the Ubuntu/Debian answer — `scry` and `scry-gui` on PATH, the menu entry, and a **derived** `Depends:` |
| `scry-<v>-linux-x86_64.tar.gz` | the same two binaries, no root, any distro |
| `scry-<v>-windows-x86_64.zip` | `scry.exe` + `scry-gui.exe`. No installer, no redistributable, nothing in the registry |

**The `Depends:` is read off the ELF, not typed.** `_needed()` walks the
binary's own `DT_NEEDED` table and the build **fails** on any soname the
package does not declare. That check earned itself immediately: it caught that
`scry-gui` links the whole pango/cairo/glib stack and not the short
`libX11`/`libfontconfig`/`libfreetype` list `scry-ui/Cargo.toml` had been
claiming since the toolkit landed. A typed `Depends` would have shipped that
claim as a broken install on the first machine without Pango.

⚠ **The determinism claim is bounded, and the page says so rather than
implying more.** The containers are
byte-identical run to run — pinned mtimes, uids, member order, zip epoch. The
*binaries* are not: rustc writes its own build path into them, so a different
checkout produces different bytes. So the published sha256 answers **"did I
get the bytes scry published"** and not **"can I rebuild these bytes"**. A
checksum sold as reproducibility that is not reproducible teaches a reader that
checking is theatre, which is worse than publishing no hash at all.

**Two defects this found, neither in the new code.**

* **`scry-gui.exe` was a CONSOLE subsystem binary.** Every Windows player
  double-clicking it would have got a black terminal behind the window — the
  first thing anyone saw of this client, and it reads as something crashing.
  Now `windows_subsystem = "windows"`, with `AttachConsole(ATTACH_PARENT_PROCESS)`
  buying back the two `println!`s for someone who ran it from a terminal on
  purpose. `SetStdHandle` rather than the CRT's `freopen`, because that route
  needs `__acrt_iob_func` on the UCRT and `__iob_func` on mingw's msvcrt — it
  would have built under one Windows toolchain and failed to link under the other.
* **The published Python `.deb` was three commits stale**, missing 142 lines of
  broker/catalog/depot work, and *every* gate was green over it: `test_site`
  checked the bytes against their own hash, which a stale artifact passes
  perfectly. `build_package.py --check` had always been able to catch it and
  nothing ran it. It runs in `test_site` now. **A hash proves what an artifact
  IS, never that it is current** — the two questions look identical and only
  one of them was being asked.

⚠ **Still true, and it is on the download page in gold: nothing has executed a
single instruction on Windows.** The binaries cross-compile, link, and carry
the right subsystem; the suite that passes is the Linux one. That is a first
build, not a tested one, and the page says the sentence rather than implying
the opposite by silence. It comes off the day someone runs it.

### 10c · The dependency policy

Operator, same day: *"if anything like veersion lock because lost of npm type
chain attacks recently."* `Cargo.lock` is committed and pins the whole
transitive tree with a checksum each — that is the control, and deleting it is
the regression. On top: every direct dependency pinned `=x.y.z`, a tree budget
and duplicate/checksum checks enforced by `supply_chain.rs`, and `deny.toml`
for advisories and licences. Direct dependencies are `serde`, `serde_json`,
`sha2`, `tempfile`, `ureq` — no async runtime, no web framework, no
argument-parsing crate.

**What it does not buy:** a pinned, checksummed dependency is still someone
else's code running as you. The lock stops the bytes changing under you; it
does not make the bytes good.

### 10d · The hive, ripped into the client

> Operator, 2026-08-05: *"can we also check out the hive page we already have
> and like rip the basics out into the app or what not?"*

**Reading the hive is cheaper than §10 first said.** No key, no WebSocket, no
Nostr implementation — the meter already mirrors the relay server-side, so
`GET /api/hive/channels`, `/hive/room` and `/hive/voice` are plain JSON.

⚠ **The correction:** *"hive presence needs a WebSocket client"* is true of
**presence only** — kind:20001 is ephemeral and WS-only
(`meter/hive_presence.py`). Reading is HTTP, and so is **posting**:
`POST /api/hive/publish` carries one already-signed event. What posting needs
is a **schnorr (BIP-340)** key, a different scheme from the launcher's
secp256k1 ECDSA signers, so it is its own decision and not a transport
problem. Reading is free today and reading is the basics.

**The thing a browser was doing for us.** `hive.html` writes into a DOM, where
the browser escapes text and a CSP bounds it. A terminal has neither, and hive
content is **written by strangers** — the town stream carries whatever anyone
signed. A raw escape sequence in a chat line can clear the screen and paint a
fake *"depot digest verified"* above itself; a `\r` can rewrite the visible
line so it differs from what was signed, **and the signature still checks out,
because the bytes are the bytes**; `\x1b]52` is read/write clipboard on
several terminals, next to a wallet. So everything off the relay goes through
`render::safe_line` — CSI, OSC, C0/C1, DEL and bidi overrides out, emoji and
CJK untouched — and each case is a test.

**One rule carried over from `ACHIEVEMENTS.md`:** `who` is the register's
sworn name, `display` is the key's self-declared kind:0 profile, and they
render differently (`~` marks self-declared). A surface may never show a
claimed name in the clothes of a checked one.

### 10e · Sessions and playtime

`run.py` starts a process and forgets it — no running state, no stop, no
playtime, which is what Steam's whole in-game presence layer sits on.

**Playtime is only real if something watched the process.** So `scry play`
supervises by default and `--detach` is the exception, and a session has three
shapes that are not interchangeable: *supervised and finished* (measured, and
the only one that counts), *running*, and *detached* (we know it began and
will never know it ended). A detached launch is kept — *"you launched this"*
is true — but it never becomes a number, and `sessions` prints
*"(+N not measured)"* rather than under-reporting silently. `playtime()`
returns an `Option`, so the type carries the distinction rather than a
convention.

That is wall 3 applied to ourselves: guessing when a detached game ended would
make this client a source of truth about something it did not observe.

### 10f · Speaking in the hive, and why it took a socket

⚠ **§10d's own claim needed correcting.** It said posting was HTTP via
`POST /api/hive/publish`. That endpoint exists, but `meter/hive.py` records
what happened against the real relay on 2026-07-25: **buzz binds an event's
author to the NIP-42 authenticated connection**, so a note carried on the
house's socket is refused — and refused *silently*, with no ruling and the note
simply absent. So speaking needs a **schnorr (BIP-340) key**, **our own
WebSocket**, and NIP-42 AUTH. `publish` returns only when a relay ruled `true`.

**The voice is not the wallet, and that is the whole design.** Nostr is
schnorr; the signer ladder is ECDSA. A voice **cannot move money** — no
balance, no transaction, nothing on chain, and `CLAUDE.md` invariant 7 is
untouched because scry never sees it. It **can speak as you**. At rest it is a
file at 0600 rather than an encrypted keystore, and every surface says so:
`vault.py` wraps the *wallet* in scrypt because that key holds money and is
used rarely, while a chat key is used on every message and a passphrase per
line makes a client nobody uses — which is its own dishonesty, because the
feature then exists only on paper.

⚠ **The bug the parity test earned its place by catching.** `k256`'s blanket
`Signer::try_sign` SHA-256s the message first; Nostr signs the 32-byte id
itself. The Rust self-check passed anyway — it signed and verified with the
same wrong convention, internally consistent and externally wrong. Only
`parity_verify.py`, running `meter/hive.py:verify_event` over real Rust output,
saw it.

### 10g · The MCP face

`scry mcp` is the other half of *no plugin loader* (`SENTENCES.md` 2026-08-05):
**the user's own harness is the mod.** Read-only by construction — no install,
no play, no speaking, because an agent that can install software or post as you
without a human act is a different product. One dispatcher tool with a `what=`
selector, per `COMPANION.md` §4a's token discipline. Local, because a hosted
endpoint cannot see your installs or reach your signer.

### 10h · The overlay

Built, in the broker and both SDK clients. Steam injects and hooks the graphics
API because Steam does not own the games; this platform does, so the overlay is
**the game drawing its own HUD** from one broker read.

The broker's vocabulary went **six verbs to seven**, and `test_launcher.py`
pins the count as a budget rather than a fact — every future verb should cost
the same deliberate edit, because the wire is the game's whole authority.

⚠ **Display freely; never approve.** A game controls its own pixels, so it can
draw a convincing fake of any dialog. The reply carries no consent token, no
pending-request queue and no already-allowed flag; a test asserts their absence
**by key**, not by prose, because the note explaining the rule legitimately
contains the word. Consent stays in the launcher's own window, per game and per
family.

### 10i · One identity, and one set of icons

> Operator, 2026-08-05: *"all name stuff need to be unifed… ONE NAME IN ONE
> PLACE AND EVERYTHING PULLS FROM THAT SAME WITH PFP"* and *"make sure we get
> cool icons for the launcher."*

**The client had three name rules and now has one.** A hive message's `who`, a
kind:0 `display`, and a bare npub were each decided at the point of rendering.
`scry-hive/src/who.rs` is the single resolver over `GET /api/who`, which takes
a wallet, an npub, a vow id or a sworn name — and `Message::speaker()` and
`short_npub()` are both **deleted**, because a second place that decides what
a name is, is the whole problem in miniature.

One name, and the evidence rides **on** it (`NameKind`), which is
`ACHIEVEMENTS.md`'s claim-plus-evidence-kind applied to a handle rather than a
badge. `~` marks a name nobody checked.

⚠ **The remaining gap is the meter's, and it is written up in `PROFILE.md`
§0a:** the account is the wallet, and the wallet has no name or face of its
own — every name belongs to a vow, so a wallet with two vows answers to two
names. The client shows the register's pick **and says so** rather than
presenting one of several as the name.

**The icons are generated, not drawn** (`launcher/art/make_icons.py`): a gate
with an eye, one corner badge per window, read out of `theme.py` with `ast` so
the palette cannot fork and so they build with no display. Two laws are
enforced on the output rather than trusted in the drawing — **no icon may wear
the reserved green** (`SITE-PLATFORM.md` §14b, the same law that fails a green
Play button), and **no two icons may be the same picture**, which shipped once
with the app icon identical to Games'. The `.desktop` entry closes half of §8
row 5. Below ~32px the badge is dropped for a plain variant, because three
elements are mud at tray size — checked by rendering and looking, not assumed.

### 10k · The account had no door, and the windows had no callbacks

> Operator, 2026-08-06: *"scry-gui account window has nothing for me to do with
> it… also i mean even the cli should have some kinda key signing? how do we
> know who is allowed to submit the gates game?"*

**Three separate findings, and the first one explains the other two.** Measured
before anything was changed:

| | |
|---|---|
| callbacks in the whole GUI | **one** — the menu, showing a window |
| buttons across all six windows | 5, **none wired** |
| the Account window | 7 labels, **0 buttons** — nothing to press by construction |
| `LocalSigner::create` / `unlock` | called from `tests/vectors.rs` **and nowhere else** |
| the Store's catalog | `let store_titles = Vec::new()` — hardcoded, never fetched |
| `scry-launcher`'s vault dependency | **absent** — no CLI account path at all |

So the vault shipped complete on 2026-08-06 and **no path a user could reach
ever called it.** §10b8 was true about the crate and wrong about the product,
which is the trap this repo names in `CLAUDE.md`: a note that disagrees with a
measurement is wrong, and *"the windows draw"* had quietly become *"the windows
only draw."*

**What changed.** `scry account new | import | sign | show` — the headless half,
and the one that matters most, because a client whose only way to hold a key is
a desktop window cannot hold one on a server, in CI, or under an agent. The
window's controls now do what they say. Both halves ask `scry_vault::
keystore_path()`, which is now the **only** definition — the GUI carried its own
copy, which was harmless while it was the only thing that could hold a key and
became two-places-to-disagree the moment the CLI could.

**Two defects the work found, neither by a test:**

1. **The keystore did not open in any other wallet.** `encrypt` omitted `id`,
   and geth, MetaMask and `cast` share a decoder that types it as *required* —
   so a file this crate wrote failed to load with `missing field 'id'`, which
   reads like a wrong passphrase and is not one. Every test here round-tripped
   through this crate's own `decrypt`, which never reads `id`, so a field only a
   **foreign** reader requires was invisible to a round trip. Found by opening
   one with `cast wallet address`. `tests/vectors.rs` now asserts the shape
   against the spec rather than against ourselves.
2. **The passphrase prompt went to stdout.** `SIG=$(scry account sign "$X")`
   captured `"passphrase: 0x…"` — 144 characters where a signature is 130 — and
   the verifier rejected it for a length nobody could explain. A prompt is for a
   human and belongs on **stderr**; the answer on stdout is the product. On a
   client whose players include agents, the two sharing a stream makes every
   account command unscriptable.

⚠ **The door refused to sign for a game, and the reason changed twice before
it stopped being true.** First no signer existed. Then `ask_consent` had **no
window to ask in**, and approving a signature on a player's behalf is the thing
this must never do. That window was named here as the next piece and is built —
§10n. What survives of the refusal is the part that was never about a window:
an account that does not exist, and one that is locked.

`examples/click.rs` presses the real buttons with a scripted `Ask` — the same
injection shape `test_launcher.py` uses for `opener=` — and asserts against the
disk. A screenshot proves a control is painted; only this proves it does
anything, which is the whole distinction this section was written for.

### 10l · Who is allowed to submit a game

**The honest answer, before the change: whoever can write to the origin's
disk.** `meter/launcher.py` has no submission route at all — no POST, no
upload — so publishing is a filesystem write, `publisher: "scryworks"` was a
string somebody typed, and nothing signed a manifest. A depot digest answers
**what** you got and says nothing about **who** published it. Integrity and
authenticity are different questions and only one was answered.

`scry-depot/src/publisher.rs` answers the other. A publisher signs a fixed
statement with an Ethereum key:

```text
scry-manifest-v1
slug:gates
name:Gates
publisher:scryworks
repo:https://github.com/AnthonE/Gates
source_required:true
```

`scry publisher <title> --statement` prints those exact bytes and `scry account
sign` signs them, so the signable form is one anybody can produce — **a verifier
whose signature nobody could generate would be a lock with no key cut for it.**

Three decisions worth keeping:

* **Secp256k1, not ed25519, and the budget forced the question the right way.**
  An ed25519 verifier measured **+10 packages against 4 of headroom**, which
  `supply_chain.rs` makes an operator decision rather than a shrug. `k256` was
  already here. An **Ethereum address** is also the better publisher identity:
  a contract, a merkle drop, an explorer and every wallet already understand
  one, where a second ed25519 identity would be legible only to us.
* **The build rows are not signed and cannot be.** The origin fills them in
  from what is actually published, so they are its *measurements*, not the
  publisher's *claims* — and they are covered by `depot_digest`, which is the
  better tool for bytes. A publisher signs what they own.
* **A valid signature is not a trusted one.** `verify` returns the recovered
  address and refuses to decide whether that publisher is *allowed*; the roster
  is a policy, and a client treating "signed by someone" as "signed by the right
  someone" would be worse than one that never checked, because it would look
  like it had.

⚠ **Unsigned is the expected state and is not a finding.** No manifest carries a
signature today, so `scry games` says nothing for an unsigned title and shouts
only for a **broken** one — printing "unsigned" against every row would train a
reader to skim the one line that will ever matter. A stripped signature is
`Malformed`, never `Unsigned`, because half a signature is what tampering
leaves behind.

### 10m · `prove` — identity without a prompt, because the game cannot speak

> Operator, 2026-08-06: *"we shouldnt have this part have consent. its our app.
> its just a message sign."* — asked against a plan that had the consent window
> on the critical path for a ticket check.

**The operator was right and the reason is better than the one given.** Not
*"it is our app"* — that stops being true the day a second game lists, and a
rule that holds only while there is one first-party title breaks silently. The
reason is that **an identity proof authorises nothing**, and EIP-191's prefix
already guarantees such a signature can never be a transaction.

But the risk was never the app. It was this:

```rust
("sign", &["op", "text", "why"])
```

`sign` takes **arbitrary text from the game**, so auto-approving a "login
family" would mean auto-signing whatever a game called a login, with the
launcher checking a first line while the game wrote the rest. `SDK.md` §8
already names a game's request as untrusted text; invariant 8 says untrusted
text cannot mint authority.

**So the verb inverts who writes the sentence.** `prove` takes a nonce and a
server, and `protocol::prove_message` composes:

```text
scry prove
game:gates
server:shard-3.gates.example
nonce:8f14e45fceea167a
```

`game` comes from the `hello` handshake, never from the request — a title
cannot prove as another title. There is no prompt because there is no decision
left: the game contributes two opaque values and cannot say anything with them.
Pressing **Play** was the consent.

⚠ **`check_prove_field` is the whole of the safety and it is not decoration.**
The two game-supplied fields land *inside* the signed bytes, so a nonce of
`"x\nplay:transfer everything"` would let a game append lines to a message the
launcher believes it authored — the exact smuggling this verb exists to
prevent, reintroduced through the one gap the game still controls. No newlines,
≤128 bytes, and a conservative charset that still admits every uuid, hex and
base64url nonce a real server issues.

**The vocabulary budget went 7 → 8**, spent in the commit that added the verb,
the way the dependency cap is. `sign` keeps its consent gate, because that one
*is* arbitrary text.

**What it cost elsewhere:** the account became `Arc<Mutex<…>>`. The door runs on
its own thread, so an unlock in the Signing window has to be visible there —
otherwise `prove` could only ever answer *"locked"*, which is a launcher that
unlocks in a window nothing else can see. `examples/click.rs` asserts exactly
that path: locked → refuses, unlock through the real button → the door proves,
and the proof recovers to the machine's address.

**Proven end to end rather than argued.** `sdk_parity.rs` drives the vendored
SDK over a real socket to a host holding a real key, recomputes the message the
way a server must, `ecrecover`s it, and checks the address — then asserts a
**different nonce** and a **different game** do NOT verify, which is what stops
a captured proof being a skeleton key or one title's proof admitting a player to
another. The CLI's `scry prove` builds the same string through the same
function, so an agent never hand-assembles it.

`signer_name` now reports `local` when an account exists. It said `none` while
`sign` was the only signing verb and a prompt was missing; with `prove` working,
`none` would hide the capability a game actually wants.

### 10n · The consent window — the door learns to sign

**`sign` was the last verb the client could not answer**, and §10k had named
the window as the thing standing in the way rather than as a missing feature.
It is built: `scry-ui/src/consent.rs` is the crossing, `windows::consent` draws
it, `wiring::serve_consent` answers it, and `scry-gui`'s `Host::sign` now signs.

**The problem was a thread, not a dialog.** The game door runs on its own
thread — it has to, `prove` is served from there while a player uses the
windows — and FLTK's widgets belong to the main one. So the ask crosses on a
queue and the door parks on the answer:

```text
  door thread                       main thread (FLTK)
  ───────────                       ──────────────────
  ask() ── push ──▶ queue
  park on recv()                    pump() ── drains ──▶ shows the window
                                    …the player answers…
  ◀────────── the answer ────────── reply.send(…)
```

`Doorbell` links no FLTK at all, which is the whole reason its rules are unit
tests on a real thread pair rather than something only a human could check.

Four decisions worth keeping:

* **Every failure refuses.** A poisoned lock, a launcher closing with a prompt
  up, a window dismissed by the window manager, an event loop that stops
  dispatching — each answers `None`. The two directions are not symmetric: a
  wrong refusal costs a click, a wrong grant costs a signature the player never
  approved. The shutdown case is a test, because it is the one a reader assumes
  is a hang.
* **The account state is NOT checked before prompting**, and that is the
  subtle one. A locked account is not a refusal *by the player*, so returning
  `None` for one would tell the game *"you refused"* — untrue, about a person —
  and record a denial that sticks for the session (`Consent::denied`). The
  prompt therefore fires either way, says on its face that no signature can be
  produced yet, and `sign` gives the game the actionable reason.
* **`why` reaches the player, and it did not before.** The field is in
  `PROTOCOL.md`'s own example and reached nothing but a browser handoff url;
  `Host::ask_consent` now takes it. It is drawn as *"it says: …"* because it is
  the game's sentence rather than the launcher's.
* **The message is a scrolling `TextDisplay`, not labels.** Game-supplied text
  is unbounded, so a well that grows is a well that pushes Refuse off the
  bottom of the window — and a prompt whose refusal is out of reach is answered
  by guessing at the window manager.

⚠ **`consent::one_line` is an injection guard and not tidying.** An FLTK label
breaks on `\n`, so a `why` of `"settling\n\n✓ already approved by scry"` draws
two lines the launcher never wrote, in the launcher's own window, under the
launcher's own words. That is exactly what `check_prove_field` refuses for
`prove`, arriving through the one field a game still fills in on this path.
Control characters become spaces rather than vanishing, so a backspace run
cannot reassemble into a different word than the one that was sent.

⚠ **A game can draw a window that looks exactly like this one**, and nothing on
screen can prove which process painted it. What holds instead is that the
forgery buys nothing: there is no consent token in the protocol to collect, no
way to mint one, and no reply the launcher accepts that says *"already
approved"* (`SDK.md` §2b). A faked prompt gets a click and no signature.

**Proven by pressing it.** `examples/click.rs` now presses the real buttons —
each allowance grants its own number, Refuse answers no and closes, an
unanswered prompt has granted nothing — and then drives the whole path: the
real broker parsing a real `sign` request, the real counter, the real doorbell
across a real thread, and a real key at the end, with the signature recovered
back to the machine's own address. Only the human is stood in for.

### 10j · What is still Python

**Nothing that the Rust client cannot also do.** This section used to list
"every window, the signer ladder and the broker, the vault, the catalog and
hive reads" and each line of that was retired by §10b6–§10b9 without the list
being rewritten — which is exactly the failure `CLAUDE.md` warns about, a note
that disagrees with a measurement. Derive it instead of reading it:
`cd launcher-rs && cargo test --workspace`, then `scry check`.

**Nothing is Python-only any more.** The desktop client that was
(2026-08-07: *"delete the python launcher we dont use it anymore"*) is gone, and
with it the split between a client that could do Windows and one that could
not. What is still Python is the **origin** (`meter/`) and the **SDK client a
Python game or agent harness links** (`sdk/python/scry_overlay.py`) — neither
is a launcher.

The two clients share the receipt format on purpose, so an install made by one
is readable by the other.

## 11 · The origin's half — how a player finds a game at all

> Operator, 2026-08-05: *"can we setup the scry launcher to support Gates? and
> I guess the backend of the site? … make it a real downloader?"*

Everything above described a client that could install a build **if you already
knew the url of the game's manifest.** That is a storefront you cannot shop in,
and it was the actual gap: §8 row 4. `meter/launcher.py` closes it.

```
GET /api/launcher                                     the card
GET /api/launcher/manifests                           every title served
GET /api/launcher/manifest/{slug}                     one, with builds MEASURED
GET /api/launcher/depot/{slug}/{build}                the depot index
GET /api/launcher/depot/{slug}/{build}/files/{path}   the bytes
GET /api/launcher/servers/{slug}                      the title's shard list
```

So `scry install gates` works, and `scry games` says what there is to install.

### 11a · Four rules, each of them a refusal

**The digest is not reimplemented in the router.** It imports `meter/depot_doc.py`
and calls its `digest()`, `parse_depot()` and `safe_relpath()`. A second
implementation of the number that gets notarized is the bug invariant 3 is
about — with the extra charm that two copies would only disagree on some file
nobody thought to test. A test asserts the router contains no hash of its own.

**A depot document is served byte-for-byte.** The digest is taken over the
whole document, so re-serialising it — even to prettier JSON — changes the
number a player recomputes and looks up on chain. The origin therefore never
edits a depot on the way out. The publisher bakes `root` at package time and we
hand over exactly the bytes they wrote. *(Measured 2026-08-05: the digest off
the packager, off the origin's response, and out of `scry install` are one
string. ⚠ Read the scope — that run drove the **app**, and on 2026-08-05 the
app was not what the internet could reach. It says the digest does not drift
through this code; it never said a player could fetch it. §11e.)*

**A build goes live when `published.json` names it — never because a directory
appeared.** An upload in flight is a half-written tree, and "newest directory
wins" would flip the live build to it mid-copy. Publishing is therefore one
small atomic file write, and rolling back is editing one line.

**Only files the depot declares are served.** The index is the allowlist, so a
stray key, a README or a leftover `.git` in a staging tree cannot be pulled out
of a build directory even though it is sitting right there. A declared file
that is *missing* answers **503**, not 404 — "this build is incompletely
uploaded" and "this file does not exist" are different facts.

### 11b · What the manifest row means now

A native row in a served manifest is **measured**, not typed. The origin reads
the publish pointer, opens the depot it names, and fills `depot`, `version`,
`bytes`, `files` and `depot_digest` off the real document. It also carries a
`depot_state`, and the three values are the whole discipline:

| `depot_state` | meaning |
|---|---|
| `published` | a depot is there and parses; the row is a download |
| `unpublished` | we read the depot root and nothing is published. A real zero |
| `unknown` | **we could not look.** Never rendered as "no desktop build" |

That third row is `CLAUDE.md` §traps at the one place it would cost a player a
game: a launcher that says *"no desktop build published"* because a disk was
unmounted has lied about the catalog.

### 11b2 · `state` is the listing's, and the manifest file is a floor

**Fixed 2026-08-10, and it was live when it was found.** `state` — *is this
game out* — was written in **both** `listings.json` and
`data/launcher/<slug>.manifest.json`. The store read one and the client read
the other, so the desk flip that put Gates live in the overlay at 20:33 left
the storefront saying **live** and the desktop client saying **in-build**, each
internally consistent, neither looking wrong on its own. Three copies of one
fact, two of them stale.

The cure is the one `play_url` already had: **the origin derives it.**
`_listing_state()` reads the title's row off `storekeeper.merged()` — base +
overlay, the same read the storefront makes — and the served manifest carries
where the answer came from:

| field | meaning |
|---|---|
| `state_from: "listing"` | derived from the shelf row. The normal case |
| `state_from: "manifest"` + `state_why` | **the shelf could not be read**, so the file's own value answered, and the row says why |

Two consequences worth stating, because both are deliberate:

* **A desk flip reaches the client with no commit and no restart.** The overlay
  is the authority the storefront already obeys; the launcher now obeys the
  same one.
* **The direction is the opposite of the browser row's url below**, where a
  hand-written manifest value wins. A manifest may honestly point at a build
  that is not what the store's play button opens — that escape hatch is worth
  keeping. There is no honest case where the client says a game is out and the
  store says it is not, so here **derived beats explicit** and the file's copy
  answers only when nothing else can. With no shelf *and* no floor the answer
  is `in-build`: an unreadable shelf must never be the thing that advertises a
  launch.

`gates.manifest.json` keeps its `state` line as that floor, with a `_state`
note saying so, and `launch_preflight.py` no longer demands the two agree — it
checks the one combination that can still lie, a `live` floor under a listing
that is not live.

### 11c · Why the origin writes a PATH and the client completes it

A served manifest says `"depot": "/api/launcher/depot/gates/0.1.0"`, not a full
url — because **an origin does not reliably know its own public name.** Behind
nginx and a CDN the only candidate is the request's `Host` header, which is
supplied by whoever is asking. An origin building download urls out of that
would hand every caller a url the caller chose. The client knows where it
asked; the server does not. So the client rebases, and both clients implement
the same three cases: an absolute url is left alone, a path-absolute one is
completed, and **a protocol-relative `//host/…` is blanked rather than
rebased** — it looks relative and is not, and prefixing a scheme would send the
download to another host while the manifest still read as same-origin. A
blanked row is a slot. Losing a build row is recoverable; fetching from the
wrong host is not.

### 11d · Serving the bytes, and when to stop

`FileResponse` through uvicorn is correct and is not fast. It is there because
the alternative — an nginx `alias` — cannot answer *"is this path one the depot
actually names?"*, and at this size that check is worth more than the
throughput. When a build gets big enough to care, the move is
`X-Accel-Redirect` against the same directory with this route keeping the
decision. Nothing about the client changes either way.

### 11e · The prefix, and why every gate was green

**The routes shipped unreachable and nothing anywhere went red.** nginx serves
the meter under `location ^~ /api/` with `proxy_pass http://127.0.0.1:3600/;`
— the trailing slash makes nginx **replace** the matched prefix, so
`/api/launcher/manifests` arrives at uvicorn as `/launcher/manifests`. Every
module in the meter is declared to suit: `tape.py` registers `/tape` and is
reached at `/api/tape`, and `smoke_test.py`'s own usage line says *"local (no
`/api` prefix)"*. `launcher.py` was the one file that declared its five routes
**with** the prefix, so nothing it registered was reachable by anyone.

Measured against the origin the day after: `/api/tape` 200, `/api/launcher`
404. Every other surface the repo names — `/api/health`, `/api/catalog`,
`/api/crier`, `/api/errors`, `/api/pubkey` — 200. One file, one prefix, the
whole desktop distribution path.

**Why no gate saw it, and this is the transferable part: a test client never
crosses the edge.** `test_launcher_api.py` builds a bare `FastAPI()`, includes
the router and asks it for `/api/launcher/...` — which passes, because in that
process the route really is `/api/launcher/...`. The suite was asserting a
topology that exists on no machine. This is `CLAUDE.md` §traps' *"a silent
degradation reads as healthy"* with the degradation at the edge instead of in
the app, and it is the same seam `deploy/wiring_audit.py` was built for one
level down — an address that lands and never reaches the file a page fetches.

Three things now hold it, and they point deliberately in **opposite**
directions — matching one to the other is how this comes back:

| | rule |
|---|---|
| the **declarations** | bare (`/launcher`). nginx has already stripped `/api/` |
| every url in a **response** | keeps `/api/` — a client goes through nginx |
| the **suite** | drives the app through `NginxClient`, which performs the strip the edge performs, so the urls written in the test are the ones a player's client asks for |

`test_launcher_api.py` §0 pins the declarations and §12 pins the handed-out
urls; reverting one route to `/api/` fails §0 by name with exit 1 (verified by
mutation, not assumed). `/launcher` and `/launcher/manifests` also joined
`smoke_test.py`, which is the only gate in the repo that crosses nginx at all.

⚠ **What is still ungated:** the check is per-module, not structural. Nothing
compares the nginx conf's `location` blocks against the app's registered routes,
so a *new* surface mounted under a prefix nginx does not proxy would be just as
dark and just as green. That gate does not exist yet.

**The consequence for local testing, which bites immediately.** Both clients
ask for `{host}/api/launcher/…`, so pointing one at the app directly —
`--host http://127.0.0.1:3600` — now 404s, because the app serves `/launcher/…`
and nginx is the thing that was going to add the difference. **`--host` wants an
origin, not the app.** Put the same mount in front and every url is the real
one:

```python
from fastapi import FastAPI
import launcher
inner = FastAPI(); inner.include_router(launcher.router)
app = FastAPI(); app.mount("/api", inner)     # what nginx does, in one line
```

`--allow-insecure` is the other half — the client refuses an `http` depot root
(correctly), and that switch exists for exactly this. ⚠ And the depot's `root`
is **baked at package time and never rewritten by the origin** (§11), so a
depot packaged against one port does not become a depot for another: serving
`/manifest` from a second port still sends the download to the baked one. That
is the byte-for-byte rule doing its job, not a bug — but it does mean a local
depot has to be packaged for the port it will be served on.

## 12 · Windows — what is measured and what is owed

> Operator, 2026-08-06: *"also plan on it all running on windows one day too."*
>
> ⚠ **"One day" was the same day.** This section was written as an audit of an
> unpaid cost, and four of its six rows closed within hours — rows 1 and 4 by
> the packaging work in §10b9. It is kept as the audit it was, struck through
> where it was paid, because the *ordering* it derived is the thing that turned
> out to be right and is worth keeping legible. **A Windows player can download
> the client now** (`/download.html`); what nobody has done is run it.

**Not a port plan — an audit, so the cost is known before it is paid.** The
manifest format already carries the rows (`win-x86_64`, `mac-arm64`,
`mac-x86_64` are in `manifest.schema.json`'s platform enum), so nothing here
needs a format change. Re-derive any line below; none of it is typed from
memory.

**The good news is the expensive half.** `cargo check -p scry-depot --target
x86_64-pc-windows-msvc` **passes clean today.** The depot core — install,
verify, update, digest, launch, path safety — is the highest-risk code in the
client (it writes files a stranger named) and it is already portable. The
`#[cfg(unix)]` arms in it are deliberate and each has a `not(unix)` sibling.

⚠ The full workspace does **not** cross-check: `ring` (via `ureq`'s TLS) runs a
C build script that wants MSVC's `lib.exe`. That is a toolchain fact, not a
code defect — it needs a Windows box or a cross-toolchain, not a rewrite.

What is genuinely owed, in descending cost:

| # | what | why it is not free |
|---|---|---|
| ~~1~~ | ~~**the broker socket**~~ | **DONE for the native client 2026-08-06.** `scry-broker::transport` binds and accepts on `\\.\pipe\scry-launcher-<user>` under `cfg(windows)`, and both reference SDKs reach it: `scry_overlay.rs` via `WaitNamedPipeW`, `scry_overlay.py` by opening the pipe as a file, since CPython exposes no `AF_UNIX` there. ⚠ **This row claimed the SDK half was "vendored byte-for-byte into Gates and pinned by sha256" and that was not true for three days.** Gates' copy predated the named pipe entirely — it imported `std::os::unix::net` unconditionally, so a Windows build of that game could not compile, while its pin stayed green because a pin catches local edits and cannot see upstream move. Re-vendored 2026-08-09; the check that exists now is `sdk/SHA256SUMS` plus two gates in `sdk/test_sdk.py` (rustfmt-clean, and no unguarded unix import). `docs/client/SDK.md` §5a. Fix upstream and re-vendor, never patch in the game (`Gates/CLAUDE.md` §vendored). |
| ~~2~~ | ~~**`is_alive` lies on Windows**~~ | **DONE 2026-08-06.** It returned a flat `false` for every pid under `cfg(not(unix))`, so the client's `── running ──` block would have been permanently empty and looked like a correct reading of an idle machine — the repo's never-raises-reader trap with a `cfg` on it. Now `OpenProcess` + `GetExitCodeProcess`, declared as a raw `extern "system"` the way `kill` already is, so it costs no crate against the tree budget. ⚠ `STILL_ACTIVE` is 259, so a process exiting *with* code 259 reads as alive — the documented Win32 ambiguity, and it lands in the same display-only place the pid-recycle caveat does |
| ~~3~~ | ~~**the paths**~~ | **DONE for the Rust client 2026-08-06**, and it was worse than "XDG". Both roots read `$HOME` and `unwrap_or_default()`'d it — unset is the *normal* state on Windows, where the variable is `USERPROFILE` — so the games root became `/.local/share/scry/games`, an absolute path at the filesystem root. Now `%LOCALAPPDATA%`/`%APPDATA%`/`%USERPROFILE%` on Windows, XDG on unix, and the working directory rather than `/` when there is nothing to read. `tests/origin.rs` pins it through the real binary with the environment actually cleared. |
| ~~4~~ | ~~**the packaging**~~ | **DONE 2026-08-06** — and it landed exactly as this row predicted. `launcher-rs/build_release.py` writes a `.zip` of two `.exe`s and that is the entire Windows installer story: no MSI, no redistributable, no registry. §10b9. |
| 5 | **`requires.libs`** | `Gates/ci/depot.py` reads ELF `DT_NEEDED` via objdump/readelf. A PE build needs an import-table reader or an honest empty with a reason — it already returns `([], why)` when neither tool is present, so the shape is right |
| 6 | **the executable bit** | `launch.rs` skips its `0o111` check off unix and `install.rs` no-ops `set_executable`. Correct on Windows, where `.exe` carries it — worth stating so nobody "fixes" it |

**The ordering this implies:** the Rust client is the Windows client. Row 4 is
most of the Python GUI's remaining cost and evaporates if the windows move
before the port does, and row 1 is owed either way.

**That ordering held.** Rows 1 and 4 both closed against the Rust client and
What is left in this table is row 5 and row 6's statement-not-fix.

**How the Windows half is checked without a Windows box.** `cargo check -p
scry-depot --target x86_64-pc-windows-msvc` type-checks the `cfg(windows)` arms
including the new `is_alive`; `scry-launcher` cannot be cross-checked because
`ring` (via `ureq`'s TLS) runs a C build script wanting MSVC's `lib.exe`, so
its one `cfg(windows)` block is compiled in isolation with `rustc --target`
instead. ⚠ **That is compilation, not execution** — nothing here has been *run*
on Windows, and a type-checked syscall is not a tested one.

## 12b · How the client updates itself, and whether a store is worth it

> Operator, 2026-08-09: *"how does the launcher its self auto update? or how can
> we make it more compatible with linux ubuntu store things and windows store
> type things if its worth it?"*

### It does not replace itself, and that is the design

`scry update <title>` updates a **game**. Nothing updated the **client** — and
until now nothing even told you a newer one existed. `scry check` now asks the
origin and says one of four things, never three:

```
client      0.1.0 — current
client      0.1.0 — 0.2.0 is published
                    https://scry.moreright.xyz/download.html
client      0.1.0 — ahead of the origin's 0.1.0        (a local build)
client      0.1.0 — could not ask: io: Connection refused
```

The last is the one that matters. **"We could not look" is never printed as
"you are up to date"** — the trap `scry-net` exists around, applied to the
client's own version.

**A notice, not a self-update, and this is a decision rather than a shortfall.**
A binary that rewrites itself is the one part of this client that would have to
be trusted absolutely: it needs its own signature policy, its own rollback, and
its own answer for dying mid-write. §1's third rule says the client is never a
source of truth, and a self-replacing binary is the shape that quietly becomes
one. On Linux the package manager already owns that job and does it better.
On Windows nothing does, which is the honest gap below.

⚠ **Two defects found writing this**, both of the same family — a fact typed in
two places, where only the machine-readable one was wrong:

- `/api/launcher`'s `get_it` rows carried a literal `<v>` that nothing ever
  substituted, so **every download path in the card 404'd**. A human never saw
  it: `download.html` hard-codes the real filenames. The version is now
  measured off the artifacts in `watchtower/dl/`, with each row's sha256 read
  from the published `SHA256SUMS`, and `test_launcher_api.py` §8 pins it —
  including that `0.10.0` sorts above `0.9.0`, and that a published version
  missing one artifact says so instead of advertising a path that 404s.
- `scry check` probed a **hard-coded origin** while every other verb honours
  `--host`, so `scry check --host <local>` reported on production and looked
  healthy.

### The store question, and the wrinkle that decides it

**This app is a bad fit for a sandboxed store format, and not by a little.**
The launcher downloads other people's binaries, marks them executable, runs
them, and binds an IPC socket the game connects back to. That is precisely what
strict confinement forbids. Steam ships as a **classic** snap and as a Flatpak
with extensive permission holes for exactly this reason — the store formats
were designed against the thing a game launcher is.

So the channels split cleanly into the two that are cheap and the ones that are
not:

| channel | what it buys | what it costs | verdict |
|---|---|---|---|
| **an apt repo on our own box** | `apt upgrade` — real auto-update on Ubuntu and Debian, owned by the OS | a GPG signing key, a `Release`/`Packages` index, one nginx `location`, and a `sources.list` line the player adds once. The `.deb` already exists and already declares its dependencies | **the one worth doing** |
| **winget** | `winget install scry` / `winget upgrade` on Windows. No fee, no store account, no review board | a YAML manifest PR to `microsoft/winget-pkgs`, wanting a stable download url and a sha256 — both already published | **cheap, but see the blocker** |
| **Flathub** | cross-distro Linux reach, arguably wider than Snap now | a manifest, review, and a sandbox that must be holed open (`--filesystem`, `--talk-name`) before the launcher can exec what it downloads | later, if Linux players ask |
| **Snap / Ubuntu Store** | the Ubuntu surface, auto-updating by default | **`classic` confinement**, which is not self-serve — it needs a manual review request to the Snap store team, justified. A strict snap cannot exec an arbitrary downloaded binary at all | expensive for this shape |
| **Microsoft Store (MSIX)** | discovery | a Partner Center account (paid), Authenticode signing, store review, and MSIX's own limits on child processes | not now |

**Two costs that land before any store**, and both are easy to under-count:

- **Authenticode signing.** An unsigned Windows binary meets SmartScreen, and
  the warning is worse than no listing. ⚠ This is **not** the publisher
  signature this repo already has: `publisher_address` + `publisher_sig` is
  EIP-191 over a manifest's claims, checked by our own client. Windows does not
  know what that is and never will. A code-signing certificate is a separate,
  recurring, real cost.
- **Nobody has run the Windows build.** §12 is explicit: *"A Windows player can
  download the client now; what nobody has done is run it."* Submitting an
  unrun binary to winget would publish a promise nothing has tested.

### The ordering this implies

1. **Run the Windows build.** It blocks every Windows channel and costs
   nothing but a machine.
2. **The apt repo**, when a Linux population exists to auto-update. It is the
   only item here that delivers real self-updating, and it needs no third
   party's permission.
3. **winget**, once step 1 is done.
4. **Snap / Flathub / MS Store** — only against a measured population asking
   for them. `CLAUDE.md`'s standing advice applies: prefer the thing that gets
   a population over the thing that adds an organ, and a store listing for a
   client with no players adds an organ.

⚠ **None of 2–4 is decided here.** A signing key, a paid account, and a
distribution channel are operator acts with recurring costs; this section is
the audit that makes the cost known, in the shape §12 used.

### 10o · The shelves had no buttons, and the passphrase screen was the toolkit's

> Operator, 2026-08-12: *"the launcher on linux is kinda meh. the passphrase
> screen doesnt match anything else. the store has overrunning text. it talks
> about 'the pass'… and i litarelly cant click anything to buy the game or
> anything. also when i return to the app it should prompt for my passphrase
> and not go pass that. and also if we need the user to reboot say so and try
> to self reboot… also make sure we show of the games with a tiny icon."*

**Seven findings, and one of them is §10k happening again on the two windows
§10k did not cover.** Measured before anything was changed:

| | |
|---|---|
| the Games window's **Play** button | painted, pressable, **and dropped on the floor** — `games()` built the widget as a local and returned only the window, so nothing could wire it |
| the Store's rows | 2 labels each and **no control at all** — no install, no buy, no way to read about a title |
| the Store's `name` and `blurb` | drawn through `chrome::label`, **not `label_untrusted`** — so origin text ran past its box, over the row beside it, and off the window |
| the Store's headline block | **THE PASS** — a designed, undeployed, platform-wide product, over a shelf that sells per-title copies |
| the passphrase prompt | `dialog::password_default` — FLTK's stock grey box with a blue `?`, in a client that is olive everywhere else |
| a title's icon | none, anywhere, for any title — nothing on `/api/launcher/*` named art |
| making an account | ended *"Restart the launcher to unlock it in Signing"* |

#### What changed, and the rule each one follows

**The two shelves hand their controls back** (`windows::GamesWindow`,
`windows::StoreWindow`), which is the whole of the dead-button fix: a window
that keeps its widgets is a window nothing can wire. `wiring::wire_games` and
`wiring::wire_store` give them behaviour through one `Storefront` trait, and
every verb behind it is the CLI's — `scry_depot::install`, `verify`, `launch`
— because a second install path in the widget layer is a second place for the
hash rules to be got wrong.

**The buy opens a browser and that is not a limitation to route around.** A
purchase is a wallet signing a transaction; this client holds no wallet by
design, and the buy box already exists as a page driven end to end
(`TICKET.md` §8a). What the launcher owns is the half after the money.

**Nothing on either shelf wears the reserved green fill**, deliberately. A
solid green fill means an act that moves money (`SITE-PLATFORM.md` §14b);
pressing **Buy a copy…** opens a page. `tests/rows.rs` counts it.

**Every origin-supplied string on both shelves is clipped.** A character cap is
a different bound and not a substitute — a hundred `W`s is three times the
width of a hundred `i`s — so the only bound that holds for any text is the box
(`chrome::label_untrusted`). Gates' real blurb is 140 characters and was what
ran off the window; the capture that missed it used a hand-shortened one, which
is why `examples/shot.rs` now carries the live string.

**The pass block is gone and the money sentence is per row**, read from
`GET /api/ticket/{slug}`. Three states and the third is the one that costs:
`ticketed: false` is **free**, an open rail is **priced**, and a read that
FAILED is **unknown** — never free, because rendering a dropped packet as free
is how a store gives a game away. ⚠ **The launcher renders the dollar figure
the origin published and never converts a wei amount**; the exact amount is on
the page where the wallet signs.

**Three windows replaced three stock dialogs** — `windows::passphrase`,
`windows::notice`, `windows::lock`. A stock dialog cannot be reskinned (its
colours and its icon are inside the toolkit's C++ side), and what the swap buys
beyond matching is that a prompt can now say *whose* account it is asking
about: `Ask` carries a heading as well as a prompt, because one of the four
secrets asked for through it is a **private key** and a window headed
*"Passphrase"* over a field wanting 64 hex characters asks for the wrong thing.

**The client opens locked** when there is an account to unlock, and the idle
relock puts the gate back rather than relocking silently. Two buttons, and the
missing third is the design: there is no *"browse locked"* door, because the
ask was for a gate and a gate with a bypass is a notification.
`SCRY_LOCK_SCREEN=0` is the posted way out (`CLAUDE.md` invariant 10).
⚠ **It is not a security boundary and does not claim to be.** The keystore is
encrypted at rest either way; what this narrows is the walk-by window on an
unlocked key, which is the same thing the relock buys.

**Restarts: the best one is the one that is not needed.** `signing()` draws its
Unlock/Lock controls always and deactivates them, so `adopt` turns them on in
place and making an account no longer ends in a reboot. Both shelves grew a
**Refresh**, so a shelf read on dead wifi and a game installed from the Store
are a button rather than a relaunch. Where a restart genuinely IS the fix — the
game door failed to bind, and nothing retries it while the process lives — the
notice says so **and offers to do it**: `wiring::restart_now` re-execs this
binary with the same arguments. ⚠ **A restart is not an update.** The client
still refuses to replace its own bytes (§8), and `std::process::exit` rather
than a tidy shutdown is what hands the door's socket over safely — a `Drop`
here would unlink the path the child just bound.

**Icons come from the backend, derived and not typed.** `meter/launcher.py`
puts an `art` block on every manifest from the listing row's own `capsule` /
`hero`, so a capsule replaced at the store desk reaches the client with no
commit — the same reason `state` derives (§14b of `test_launcher_api.py`).
`scry_ui::art` sniffs the format from the bytes rather than the url, caps the
fetch at 512 KB on the short-timeout agent, and treats every failure as the
lettered placeholder: **art is decoration on a row whose words are already
correct**, and taking every icon away leaves the Store saying exactly the same
things.

#### Two bugs the pictures found that no test would have

- **A manifest's `depot` is a PATH and needs `rebase`.** Without it the update
  check failed with *"bad uri"* and the row read *"not checked — the origin was
  not reached"*: a sentence about the player's network for a bug in this file.
- **Currency is decided by DIGEST, never by build name.** `update.rs` says it
  — *"a title is current if ANY installed build carries the published digest"*
  — and the first version of the library read compared names, which would have
  offered **Update** over bytes already identical to the published ones.

⚠ **What is still open:** an install blocks the UI thread for the length of the
download. The button says `Installing…` before it starts, so the freeze reads
as the install rather than as a crash, but a progress row is the honest fix and
is not built.

### 10p · The tenth game was drawn off the bottom of the window

> Operator, 2026-08-13: *"can we make the launcher even better? its not bad
> right now. and btw it works on windows and linux. we should we using smaller
> icons for games and a few other things."*

**The ask was the icons and the pass found a data-loss bug**, which is the
order worth recording: looking at the library at a size nobody had drawn it at
is what turned up the rest. Measured before anything was changed:

| | |
|---|---|
| a library of ten or more games | **rows nine and up were drawn past the window's bottom edge** — painted over the footer, then onto nothing. Not clipped and not scrolled to: *gone*, with their Play and Verify buttons |
| the same, on the Store and Servers | identical shape — every shelf sized a well with `.clamp(_, 520)` and then drew rows down the window at a fixed pitch, so the clamp cut the LIST and not the view |
| a title's icon in the library | `44px`, the Store's number, on a two-line row — nine games in a 620px window |
| every window | **no icon at all**: a blank page on the Windows taskbar, the desktop's default on Linux, for the two windows a stranger's game can cause to appear |
| **Escape** on the main menu | quit the launcher. FLTK ends a window on Escape by calling its callback, and for the menu that is the whole program — the same key that dismisses the passphrase prompt a moment earlier |
| any origin string containing `@` | **the text after it was thrown away.** `@home` drew nothing at all, `mail@example.test` drew `mail`, and a title called `Gates @ Home` drew `Gates` — FLTK reads `@` as a symbol spec, and it looks for the second one with `strrchr` |
| the update notice | `dialog::message_default` — the fourth stock dialog, missed by §10o's pass, in the one window a player sees before touching anything |
| **Refresh** on either shelf | rebuilt the window at its constructor's coordinate, so a Store dragged to a second monitor jumped home — and a Store install refreshes Games, so a window moved on its own |

#### What changed, and the rule each one follows

**The shelves scroll** (`chrome::shelf`). The clamp stays and now bounds the
*viewport*: how many rows are on screen is this window's business, how many
exist is the origin's and the player's. ⚠ **FLTK draws `Fl_Scroll`'s bar INSIDE
the well**, so every row's right-hand column is laid out short of it
(`windows::BAR`) whether or not a bar is showing — a Verify button laid out to
the well's edge is one the bar covers the moment a tenth game is installed.
`tests/shelves.rs` asserts both: that no row is a direct child of the window
(the state that could not scroll) and that no control reaches under the bar.

**And the windows grow downward.** `Games`, `Store` and `Servers` are resizable
in height, with the shelf as the resizable child so the footer moves with the
bottom edge instead of being covered. The width is pinned deliberately: every
row is laid out in fixed pixels, so a wider window would stretch the well and
strand the buttons mid-row. Reflowing a row on resize is a different change.

**Two icon sizes, and the split is a rule rather than a taste**
(`art::ROW_ICON` at 28, `art::SHELF_ICON` at 44). **The Store is browsed and
the library is scanned** — a player opens Games to find something they already
own and start it, and a list is measured in rows that reach the eye. Nine games
became thirteen in the same window. Both numbers stay in `art` because one
window quietly taking the other's number is the failure this used to protect
against by having a single constant.

**Every window wears the client's mark** (`chrome::wear_icon`), including the
consent prompt and the lock screen — *which program is asking* is worth a
picture on a taskbar. ⚠ It is a rect-and-circles SVG on purpose: FLTK's reader
is nanosvg, nanosvg does not render `<text>`, and a mark built from a letter
would draw an empty square **silently**. `chrome`'s own test asserts it parses
to something with pixels.

**Escape no longer quits.** Only `Event::Close` — the window manager's own
close box — ends the launcher, and closing the menu *does* quit, because the
menu is the way back to every other window and a client with it shut is one the
player can neither navigate nor see they still have.

**Every `@` is escaped where untrusted text becomes a label**
(`chrome::defuse`). This is the same hole `art::sniff` refuses at, one layer
up: a *parser* reached by a stranger's string. The clipping bound could never
have caught it — clipping bounds where text is drawn, and this is about what
FLTK decides the text *is*.

⚠ **Doubling only the leading one is not enough, and that is not what the
toolkit's docs lead you to.** A label may carry a symbol at each end and
`fl_draw.cxx` finds the second with `strrchr` — *the last `@` in the string,
wherever it is* — so an address in a blurb ends the text at `mail`. `@@` is
FLTK's own escape and renders back to one literal `@`, so doubling all of them
changes what is parsed and not what is read. Confirmed by drawing it both ways
rather than by reading the header: the before picture is a shelf of titles with
their names cut off.

**The full string goes on as a tooltip**, defused as well: FLTK draws tooltips
with symbols ENABLED (`Fl_Tooltip::draw_symbols_` is a compiled-in `1`), so an
undefused tooltip is the same hole with a hover in front of it. A clip is still
the only bound that holds, and it still eats the end of a 140-character blurb —
the tooltip is where the rest stays reachable without letting one row's text
decide another row's layout.

**The controls whose verb is not self-evident carry one too**, and
`Act::what_it_does` is where the shelf's live. Two of the five are why it
exists: **Buy a copy…** opens a browser and moves no money in here, and **No
build yet** means listed-but-nothing-for-this-machine. A player who reads
either as the other learns the client lied to them, and a five-word button has
no room to prevent it.

**The last stock dialog is gone** — the update notice is `windows::notice` now,
`Note::Done` because a published update is news and not a fault.

**Windows open where the player is looking** (`wiring::centre`): the middle of
the screen the mouse is on, a third of the way down. The menu is centred and
every other window is carried by the same offset, which keeps the 20px cascade
they were built with instead of stacking six windows on one spot. A rebuilt
shelf inherits the position of the window it replaces; the *size* is
deliberately not carried, because a library that just lost a game would keep a
well of empty olive where the game used to be.

⚠ **What this pass did not fix:** the install still blocks the UI thread
(above), and a resized window's height is not remembered across a Refresh.

## Reading order

`GATES.md` is the platform frame · `STEAM.md` is the gap list this client's
notary row belongs to · `SITE-PLATFORM.md` §14 is the skin and the green
reservation · `launcher/README.md` is how to run it · `launcher/manifest.schema.json`
is the format · `launcher/test_launcher.py` and `meter/test_launcher_api.py` are
the enforcement.
