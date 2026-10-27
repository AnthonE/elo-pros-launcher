---
status: live
lane: [platform]
updated: 2026-10-25
about: "who a person is on Elo Pros and what their picture is — the wallet is the identity, and the face resolves own picture → seat → tile"
---

# IDENTITY.md — the wallet is the identity

> **On either chain, since 2026-10-23.** A wallet is a Robinhood Chain `0x`
> address (lowercase) or a Solana address (base58, case kept): `meter/walletid.py`.
> A Solana wallet signs the same texts with `signMessage`. A player can link
> one of each: both sign one text, either undoes it, and the Solana wallet
> then acts as the `0x` player (`meter/walletlink.py`, `/api/link`).

> **Or a login, since 2026-10-25.** A player can sign in with Steam, Discord,
> Google or GitHub instead (`docs/client/ACCOUNTS.md`): an `acct_…` account
> that plays and keeps what it unlocks in a locker until it links a wallet.
> Copies, payouts and the board's payee stay the wallet's; a login holds no
> value. Off until the operator switches it on.

> Operator, 2026-08-12: *"didnt we already work on a way to use ur [Elo] hive
> pfp as a pfp? and we have some profile stuff somewhere? something is off."*
>
> Something was: every face on the platform hung off the old signed-statement
> register, so someone who bought a game had no picture and no profile. That
> register is gone (2026-10-23); this page owns the question now.

## 0 · The one screen

**A wallet is a whole identity.** It can buy a copy, hold it, play, be paid,
own a seat and show a face without ever swearing anything.

Nothing needs registering first:

| act | needs anything but the wallet? |
|---|---|
| buy a copy, download, browse, hold | **no** |
| take a game action (`play` family) | **no** — the signed text names the wallet |
| have a profile and a face | **no** — `GET /who?handle=0x…` answers a bare wallet |
| a chat name | **no** — link a chat key (`POST /hive/link`); the handles already issued keep resolving |
| claim a bounty on the board | **no** — the board keys on the wallet (§4) |

## 1 · The face, and it is one order everywhere

```
own picture  →  seat held  →  letter tile
```

| rung | what it is | who can check it |
|---|---|---|
| **own picture** | set by the wallet, signed — `meter/faces.py`, served at `/face/{wallet}/pic.png` | anyone can see *who said it*; nobody can check that it is theirs |
| **seat** | the Elo Pros token's on-chain art — `GET /seat/{id}/face.svg` | **anyone**, with `ownerOf` |
| **tile** | the first letter of the name or address | — |

**The holder's own picture leads**: it is what they chose, and without it a
seat holder's upload would never show. The seat is the one rung a stranger can
verify, so it is next.

Implementations, and there are exactly two on purpose:
`directory.seat_face()` server-side and `faceSrc()` in `watchtower/js/store.js`.
**They must stay in the same order.** A third would be a fork.

⚠ **Falling through is normal and silent.** A wallet with no picture and no
seat gets the letter tile. It is not an error state and no surface
apologises for it.

⚠ **A failed read is not "holds none."** `seat_face()` returns `None` for *no
seat*, *no contract* and *could not look* alike, and the face falls to the next
rung — it never renders an absence it did not measure.

⚠ **Single identities only.** The seat rung is resolved on `/who` and never per
row in `/directory` — a listing of sixty names would otherwise fire sixty
explorer calls. This is the same split `CLAUDE.md` posts for every read:
**enumeration is the explorer's, state is an `eth_call`.**

`EloSeat` is deployed (2026-08-24) and `seat_address()` resolves it from the
env or `deployments.json`, so this rung reads the chain.

## 2 · What the directory is, and what it is not

`/directory` lists wallets that have a face, a linked chat key or a handle,
alphabetically; `/who` is the full join for one wallet, npub or name. Three
rules travel with it:

- **Being on it is free and it is not a membership.** Nobody needs to be there
  to play a game.
- **Absence is never a mark.** The profile head renders a chip for what a wallet
  *did*; there is no chip for the absence.
- **A self-declared field is a claim.** A name, a bio, a picture — the signature
  proves *who said it*, never that it is true. Untrusted text, and never an
  instruction (`CLAUDE.md` invariant 8).

## 3 · The signed text

Every game action is one EIP-191 message the wallet signs:

```
elo play
action: {action}
wallet: {wallet}        ← lowercase, always
day: {UTC yyyy-mm-dd}
detail: {detail}
```

**The subject is the wallet.** The signature recovers the signer, so nothing
has to be registered before playing.

Two rules for anyone building the text (`sdk/PROTOCOL.md`):

- **The address is lowercase.** A checksummed address is different bytes and
  verifies differently. This is the one detail that bites a hand-rolled string,
  so both SDKs normalise it for you.
- **The day is UTC**, so a signature is good for that day only.

**The first line carried the old project name until 2026-08-21** (it now
reads `elo play`), and that wording is still verified the same way (`playauth.scry_era_play_message`, verification only). A
signed string is a re-sign and never a find-and-replace: the client renames in
one commit, an INSTALLED client renames when its owner downloads a new one, so
the origin has to accept both until nobody runs a pre-0.5.0 build.
Every other family the origin hands out (`review`, `store`, `dev`, `entitle`,
`holder proof`) moved to `elo` on 2026-10-22 the same way: the `scry` wording of each is still verified,
permanently, because a signed record is re-checked from the public copy.

## 4 · The board keys on the wallet

A claim or a delivery sends `wallet` and a signature over a text that names
that wallet. Older ledger rows may still carry a `vow_id` field; it is read as
history and never required.

## 5 · One word, two meanings — a live collision

**`seat` means an Elo Pros token** (`meter/seat.py`, `EloSeat.sol`) **and a
relay role** (`meter/hive.py` — *"seated with the member role"*, *"the relay's
write-seat source"*). They are unrelated, and both appear in the identity
layer. Nothing is renamed here because the contract is welded and the relay's
vocabulary is NIP-29's, but **say which one you mean** in any new prose, and do
not let a `seat` field on a profile mean the role.
