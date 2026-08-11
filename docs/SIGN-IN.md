---
status: live
lane: [platform]
updated: 2026-08-11
about: "sign the website in with the launcher's account — the device-code pairing (type a code, approve in the launcher, the page reads as you), and the relay that lets a page's signables be signed there too. No extension, no session, no cookie, no custody change"
---

# SIGN-IN.md — the website signs in with the launcher

> The purse's only signer used to be a browser extension, and the away state
> said so: *"install any EIP-1193 wallet"*. But `scry account new` already
> writes a real wallet — a V3 keystore every Ethereum tool reads — and a
> player who made theirs in the launcher had no way to prove it to a page.
> This is the bridge. **The reframe that unblocks it: that player does not
> lack a wallet, they lack a browser.**

## 0 · The shape, in one table

The browser cannot knock on a Unix socket, so the two ends never touch — the
origin both already talk to is the bridge (Steam Guard's shape, RFC 8628's
grammar):

| step | who | what |
|---|---|---|
| 1 | the page | `POST /api/signin/start` → a short code (`ABCD-2345`) and a poll secret |
| 2 | the player | **types** the code into the launcher: `scry signin ABCD-2345`, or Account → *sign a website in* in the window |
| 3 | the launcher | fetches the challenge, composes the EIP-4361 text ITSELF, shows it, one passphrase, signs, delivers the proof |
| 4 | the origin | recovers the signer, **recomposes the message from what it knows** — its domain, the code it issued — and compares byte-for-byte |
| 5 | the page | its poll returns `{address, message, signature}` — the purse flips to *proven yours* |

One implementation of the message per side, pinned against each other:
`scry-broker`'s `signin_message` composes, `meter/signin.py` recomposes, and
`sdk/testdata/signin_fixture.json` is the vector both suites assert
(`cross_language.rs`, `meter/test_signin.py`). Neither side moves without it —
the `prove` lesson, applied before the drift instead of after.

## 1 · Typed, never clicked

There is no `scry://signin/…` deep link, **and that is a decision, not a
gap.** A clickable pairing is the classic device-code phish: a stranger pastes
*their* code into a chat window dressed as a free-mint link, and the victim's
approval signs the stranger's browser in as them. A code that must be read off
your own screen and typed into your own launcher cannot be somebody else's.
The page says it in as many words ("nobody legitimate will ever send you one
to enter"), the launcher shows the full message before the passphrase prompt,
and a code lives ten minutes and works once. If a deep link is ever added it
goes behind a prompt that names exactly what is about to happen — read
`meter/signin.py`'s header first.

## 2 · What "signed in" means, and deliberately does not

The browser holds the proof in sessionStorage. **There is no server session,
no cookie, and no account row** — playauth's discipline (*"stateless — no
sessions, no API keys"*) is untouched, and `WALLETS.md` §0's "no account here,
no password, nothing to make" stays literally true. What the proof buys is the
sentence the purse could not say before: *this address is yours* — so the
heading over the balances is "what you hold" instead of "what this address
holds", and the whitelist/free-copy shapes have a way to hear from a holder
who runs no extension. Every write still takes its own signature.

Reads on a proven purse are still the server's (`/api/holdings`) — the page
says whose read it is, same as the look-up. A wallet extension remains the
stronger connection (your own RPC reads, transactions, swaps) and the page
keeps offering it.

## 3 · The relay — the launcher as the page's signer

After the pairing, `scry signin` stays attached (unless `--no-watch`) and the
same code carries signature requests the other way:

    the page    POST /api/signin/{code}/ask   {secret, text}
    the launcher  GET /api/signin/{code}/asks?approver=…   → shows text VERBATIM → y/N
    the launcher POST /api/signin/{code}/answer  {approver, id, signature|refusal}
    the page    GET /api/signin/{code}/ask/{id}?secret=…

Site side, `js/launcher-signin.js` wraps `Deck.wallet.sign`: on a page that
includes the script, a signature request with no extension present rides the
relay when a pairing is live. Pages opt in by the script tag —
`wallet.html` carries it today; a signing page adopts by adding one line.

Four walls, and each is enforced rather than asked:

1. **Messages only, never transactions.** Nothing on this door can move a
   coin. Buying a copy or swapping still wants an extension or hardware
   signer — the launcher growing `eth_sendTransaction` would be a separate
   operator decision, not an extension of this.
2. **A relayed text's first line is `scry <family>`** — the broker's own
   `sign_family` rule, applied by the origin (`_guard_ask_text`). The relay
   carries platform signables and nothing else.
3. **No SIWE rides the relay.** Any text carrying EIP-4361's "wants you to
   sign in" phrase is refused at the origin, so the relay cannot be used to
   phish a login to *anywhere* — sign-ins exist only where the launcher
   writes the words itself.
4. **A relayed signature must recover to the paired address**, or the origin
   refuses to pass it on.

And one honest limit: the player in the launcher answers y/N per request, the
exact text on screen, one ask open at a time, and an unanswered ask lapses as
a no. The passphrase is typed once at pairing and the key stays unlocked for
the watch — rung 0's trade, stated where it is made (`local.rs`).

## 4 · Who holds what

| party | holds | never holds |
|---|---|---|
| the launcher | the keystore (encrypted, 0600), the unlocked key while watching | the browser's poll secret |
| the browser | the code, its poll secret, the finished proof (sessionStorage) | any key material |
| the origin | the pending code, both secrets, the proof — all TTL'd kvstore state | a key, a session, an account row |

Knowing the code alone yields nothing: the browser's poll takes the browser's
secret, the launcher's ask-feed takes the approver secret minted at proof
time. Custody is unchanged — the key is generated, encrypted and used on the
player's machine; a signature and a public address are all that ever cross
(`CLAUDE.md` invariant 7 untouched).

## 5 · The keystore, at rest and at runtime

At rest the file was already the answer to "what if someone steals it":
Web3 Secret Storage V3 at geth's standard work factor (scrypt n=262144),
AES-128-CTR, MAC checked before any plaintext moves, written 0600, never
overwritten. A stolen file without the passphrase is a brick, and the format
opens in geth/MetaMask/cast so the account is never trapped here. **A PIN
would be weaker than what exists** — four digits against a rig is an
afternoon; the passphrase through scrypt is the real wall, so runtime got the
hardening instead:

- **the CLI unlocks per command and forgets at exit** (`scry account sign`,
  `scry entitle`, `scry signin` — one passphrase prompt each);
- **the GUI relocks an idle key**: every signature refreshes `last_used`, and
  an unlocked account nobody has used for `SCRY_RELOCK_MINUTES` (default 30,
  0 disables) locks itself — an afternoon of play never trips it, a launcher
  left open on a shared desk stops being a live signer;
- **Lock now** sits beside Unlock in the Signing window — forgetting the key
  is one press, and undoing it costs the passphrase, which is exactly the
  cost it is supposed to impose;
- **the GUI pairing asks the passphrase every time, unlocked or not** —
  pairing signs a browser in *as this account*, so it is an act of the
  passphrase-holder: whoever happens to be at the keyboard of an unlocked
  launcher cannot sign their own browser in. A transient unlock relocks the
  moment the pairing is delivered;
- **locking is forgetting**: the dropped key's scalar zeroizes on drop.

What none of this is: a boundary against a process running as you. `local.rs`
says it first and stays right — rung 2's answer is the arca, not a timer.

## 6 · Knobs and derivations

| what | where | default |
|---|---|---|
| public domain the origin verifies for | `SCRY_ORIGIN` | `https://scry.moreright.xyz` |
| code lifetime | `SCRY_SIGNIN_PENDING_TTL` | 600 s |
| pairing lifetime | `SCRY_SIGNIN_PAIR_TTL` | 43200 s |
| rate gates | `SCRY_SIGNIN_START_PER_DAY` · `_PROOF_PER_DAY` · `_ASK_PER_DAY` | 60 · 60 · 200 |
| GUI idle relock | `SCRY_RELOCK_MINUTES` | 30 (0 = never) |

Derive, don't quote: `curl -s scry.moreright.xyz/api/signin` is the card.
Tests: `meter/test_signin.py` (the venv rule applies), `cargo test -p
scry-broker --test cross_language`, `watchtower/test_site.py`.

## 7 · Owed, and known not-built

- **the GUI pairs and does not watch.** Account's code field signs a browser
  in; approving a page's relayed signature requests is a stream of prompts,
  and the CLI attach (`scry signin`, in a terminal) is the surface built for
  that. A code works once, so the watch wants its own code, typed there. A
  GUI watch would ride the consent-window machinery — owed, not started.
- **`wallet.html` and `vow.html` include `js/launcher-signin.js`** — the
  remaining signing pages adopt by adding the script tag and letting their
  address come from `Deck.launcher.pairing()`.
- **no deep link, on purpose** (§1). Re-read before "fixing".
