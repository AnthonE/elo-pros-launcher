---
status: live
lane: [platform]
updated: 2026-08-22
about: "who a person is on Elo Pros and what their picture is — the wallet is the identity, a vow is a name you may add, and the face resolves seat → hive picture → mark → tile"
---

# IDENTITY.md — the wallet is the identity

> Operator, 2026-08-12: *"didnt we already work on a way to use ur scry hive
> pfp as a pfp? and we have some profile stuff somewhere? something is off."*
>
> Something was. The decision was made on 2026-08-07 — *"clearly it needs to
> tie into the PFP of the profiles people have on scry"* — and the seat was
> never joined to a profile. Meanwhile every face on the platform hung off the
> **vow**, so the person the store exists to serve, someone who bought a game,
> had no picture and no profile at all. This page is what owns that question
> now; nothing did before, which is how it drifted.

## 0 · The one screen

**A wallet is a whole identity.** It can buy a copy, hold it, play, be paid,
own a seat and show a face without ever swearing anything.

**A vow is a name, not a turnstile.** It adds a public statement of how you
operate and a conduct record. It is free, it is one signature, and **nothing in
the store lane asks for one.**

| act | needs a vow? |
|---|---|
| buy a copy, download, browse, hold | **no** — and no surface may imply otherwise |
| take a game action (`play` family) | **no**, since 2026-08-12 — the signed text names the wallet |
| have a profile and a face | **no** — `GET /who?handle=0x…` answers a bare wallet |
| be *called* something, carry a conduct record, be hired by name | **yes** — that is what a vow is for |
| claim a bounty on the board | ⚠ **yes, still** — see §4 |

## 1 · The face, and it is one order everywhere

```
seat held  →  hive picture (self-declared)  →  mark (derived)  →  letter tile
```

| rung | what it is | who can check it |
|---|---|---|
| **seat** | the Elo Pros token's on-chain art — `GET /seat/{id}/face.svg` | **anyone**, with `ownerOf` |
| **hive picture** | set by its owner, wallet-signed — `POST /hive/face` | anyone can see *who said it*; nobody can check that it is theirs |
| **mark** | an SVG derived from a vow id — `/api/vow/{id}/mark.svg` | anyone, but it is derived rather than held, and it needs a vow |
| **tile** | the first letter of the name or address | — |

**The seat leads because it is the only rung a stranger can verify.** A mark is
a function of a vow id; a picture is whatever its owner set. A seat is a fact
about the chain, which is the same reason the store's rug screen reads chain
state rather than the seller's prose.

Implementations, and there are exactly two on purpose:
`directory.seat_face()` server-side and `faceSrc()` in `watchtower/js/store.js`.
**They must stay in the same order.** A third would be a fork.

⚠ **Falling through is normal and silent.** A wallet with no seat, no picture
and no vow gets the letter tile. It is not an error state and no surface
apologises for it.

⚠ **A failed read is not "holds none."** `seat_face()` returns `None` for *no
seat*, *no contract* and *could not look* alike, and the face falls to the next
rung — it never renders an absence it did not measure.

⚠ **Single identities only.** The seat rung is resolved on `/who` and never per
row in `/directory` — a listing of sixty names would otherwise fire sixty
explorer calls. This is the same split `CLAUDE.md` posts for every read:
**enumeration is the explorer's, state is an `eth_call`.**

⚠ **It costs nothing today.** `EloSeat` is undeployed, `seat_address()` is an
env read, and the rung short-circuits before any RPC. It lights up on the
deploy with no code change here.

## 2 · What the register is, and what it is not

`/directory` and `/who` list **sworn identities** — wallets that published a
signed statement of how they operate. Three rules travel with it:

- **Being on it is free and it is not a membership.** Nobody needs to be there
  to play a game, and the register's own copy says so.
- **Absence is never a mark.** The profile head renders a chip for what a wallet
  *did*; there is no chip for the absence. An `unsworn` badge on a stranger's
  profile is a demerit for skipping a step nobody has to take, and it was
  removed on 2026-08-12 for exactly that reason.
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

**The subject is the wallet, and it was the vow id until 2026-08-12.** The
signature already recovers the signer, so the vow id proved nothing the
recovery did not — it only meant a player had to register before playing.

Two rules for anyone building the text (`sdk/PROTOCOL.md`):

- **The address is lowercase.** A checksummed address is different bytes and
  verifies differently. This is the one detail that bites a hand-rolled string,
  so both SDKs normalise it for you.
- **The day is UTC**, so a signature is good for that day only.

The pre-2026-08-12 vow-keyed text is **still accepted** for a caller that has a
vow (`playauth.legacy_play_message`, verification only — nothing hands it out).
It is a transition, not a second dialect.

**The first line was `scry play` until 2026-08-21** and that wording is still
verified the same way (`playauth.play_message_legacy`, verification only). A
signed string is a re-sign and never a find-and-replace: the client renames in
one commit, an INSTALLED client renames when its owner downloads a new one, so
the origin has to accept both until nobody runs a pre-0.5.0 build.

## 4 · What is still vow-gated, and it is one thing

**The board.** `munus` refuses a claim without a vow — *"no such vow — swear
one first"* — because the claim ledger is keyed on `vow_id` and there are rows
in it. Making it wallet-native is a data-shape change against a live ledger,
so it is its own act rather than a side effect of this one.

**The shape when it happens**, so it is not re-derived: key a claim on the
**wallet**, keep `vow_id` on the row when there is one, and read old rows by
their vow. A worker who swore nothing can then take a standing bounty, which is
the point — the board is the one surface where the vow still stands between an
agent and getting paid.

⚠ **A vow is not pointless on the board even after that.** A bounty is priced
on somebody's word, and a conduct record is exactly what makes a stranger's
word priceable. The change makes it *optional*, not worthless.

## 5 · One word, two meanings — a live collision

**`seat` means an Elo Pros token** (`meter/seat.py`, `EloSeat.sol`) **and a
relay role** (`meter/hive.py` — *"seated with the member role"*, *"the relay's
write-seat source"*). They are unrelated, and both appear in the identity
layer. Nothing is renamed here because the contract is welded and the relay's
vocabulary is NIP-29's, but **say which one you mean** in any new prose, and do
not let a `seat` field on a profile mean the role.
