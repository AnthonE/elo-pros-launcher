---
status: live
lane: [platform, security]
updated: 2026-10-30
about: "every contract our apps will ask a wallet to sign for, what each one is and what we ask of it; the two kinds of typed data they sign (a Seaport listing, and in the desktop app a USDG credit pack); the four signing settings and their defaults; and the log each app keeps of everything it was asked to sign"
---
# SIGNING-ALLOWLIST.md: what our apps will ask your wallet to sign

Operator, 2026-10-23: *"it should be pretty limited to our ecosystem… we
should know what's on the allowlist and how they work… just log what it all
said so if something odd happens we catch it."*

## The rule

The trade area of the site (the trade room, the cross-chain room, the
collections room) and the desktop app's `elo trade` and `elo skins` sign a transaction only
when it is:

- a **call to a contract on the list below**, or
- an **approval whose spender is on the list** (`approve`, or a Permit2
  `approve`), or
- **`setApprovalForAll` on one of our collections to OpenSea's operator**
  (Seaport, or OpenSea's conduit), the one-time step before a first listing.

A token contract is listed for approvals only: nothing in the trade area moves
a coin with a direct `transfer`, so one is never signed. Our collections are
listed the same way: an approval to OpenSea's operator, never a direct call. Anything else is
refused before the wallet opens, unless the user turns on *allow contracts
outside our ecosystem*, in which case it is allowed with a warning and logged.

The list ships **inside each app**: `watchtower/js/signguard.js` in the browser,
`launcher-rs/crates/elo-net/src/guard.rs` in the desktop app. An API
answer cannot widen it. In the browser, whoever serves the page serves the list
too; in the desktop app it is compiled in, so even a breached origin cannot
point the app's key at a new contract.

## The list

| contract | chain | address | what we ask of it |
|---|---|---|---|
| Uniswap UniversalRouter | Robinhood Chain | `0x8876789976decbfcbbbe364623c63652db8c0904` | `execute`: a swap through the reserve's Uniswap v4 pool. Uniswap's own audited router |
| Permit2 | Robinhood Chain | `0x000000000022D473030F116dDEE9F6B43aC78BA3` | `approve(token, UniversalRouter, …)`: lets the router spend a coin you sell. Uniswap's audited allowance contract, the same address on every chain |
| LI.FI | Robinhood Chain | `0xB477751B76CF82d00a686A1232f5fCD772414Af3` | the cross-chain and same-chain routes LI.FI quotes (measured as `to` and `approvalAddress` in its quotes) |
| LI.FI | Base, Arbitrum, Ethereum | `0x1231DEB6f5749EF6cE6943a275A1D3E7486F4EaE` | the source leg of a move onto Robinhood Chain |
| Seaport 1.6 (OpenSea) | Robinhood Chain | `0x0000000000000068F116a894984e2DB1123eB395` | buying a listing of one of our collections, cancelling your own listing, selling to an offer; and the operator a collection may approve |
| OpenSea's conduit | Robinhood Chain | `0x1E0049783F008A0085193E00003D00cd54003c71` | an operator our collections may approve; never called directly. Not deployed on chain 4663 when measured (2026-10-23); listed so the day OpenSea uses it here needs no app update |
| Gates skins | Robinhood Chain | `0x93f4965638f9c99dd53f13bf2b47358a6ff7630c` | `setApprovalForAll` to OpenSea's operator only |
| Gates copies | Robinhood Chain | `0x54538c224ae8c5bacc6382b86eed301e10d3e5e6` | buying a copy (`mintFree`, `buyWithReserve`, `buyWithRail`, …, from `/api/ticket/gates/prepare`) after an exact-amount approval to it; `setApprovalForAll` to OpenSea's operator only |
| Elo Pros | Robinhood Chain | `0x2F2169bCD4e674c76C417191d23ef3c1CfDABF7B` | `setApprovalForAll` to OpenSea's operator only |
| the Gates skin shop (`EloItemStore:GATES`) | Robinhood Chain | `0x6ca8f8c4657c08ee291cead3caf827530220c260` | `buy`: one skin at its posted JUNK price, after an exact-amount JUNK approval to it (`/api/items/store/gates/prepare`) |
| the reserve token | Robinhood Chain | `0xf7997e4fd821e39d884d86d25e1655017f241a42` | approvals only, to a listed spender |
| JUNK | Robinhood Chain | `0xd6bd13a8a16e5249fa54961fc92eee0307e0f1b9` | approvals only |
| WETH | Robinhood Chain | `0x0bd7d308f8e1639fab988df18a8011f41eacad73` | approvals only |
| USDG | Robinhood Chain | `0x5fc5360d0400a0fd4f2af552add042d716f1d168` | approvals only |

**Not on it:** EloRouter (written, never broadcast: the reserve is the tier-one
coin and JUNK trades on Abyss's own exchange). A Solana transaction (Jupiter,
LI.FI from Solana) is a serialized transaction. On the site the Solana wallet
simulates and shows it itself, and the page logs it. In the desktop app the
app *is* the Solana wallet, so it simulates first: a send or a move is signed
only when the chain's dry run takes no more SOL than the review says (the
amount, plus at most 0.01 SOL for a move's fees and accounts) and moves no
other coin the account owns (`LAUNCHER.md` §10t).

Adding a row is a change to both apps and this page, in one commit.

## Two kinds of typed data: a Seaport listing, and a credit pack

Selling an item signs a Seaport 1.6 order (EIP-712). The browser's `EloSign.signOrder` and the desktop
app's vault (`elo-vault/src/seaport.rs`) both refuse anything but Seaport's
own domain on chain 4663 offering exactly one item, from the signer, paid to
the signer first, in ETH, at a fixed price. The desktop app hashes the struct
itself from a compiled-in type string, so no caller can name a type or a
domain, and its `elo_net::skins::check_sell` also holds the order to the item,
collection and price typed, with fees only to the recipients the origin named
and under 10% together. No permit, and no other struct, is signable. The meter
checks the order a third time before posting it to OpenSea
(`meter/our_collections.py` `check_listing`).

`elo credits buy` signs the other: an EIP-3009 `TransferWithAuthorization` of
USDG for a credit pack, sent to the store's HTTP 402 door, which submits it
and pays the gas (`meter/paydoor.py`). The vault (`elo-vault/src/usdg.rs`)
hashes it under USDG's own domain (*Global Dollar*, 1, chain 4663) from a
compiled-in type string, and refuses one that is not from the signer, to the
compiled-in credits address, for at most $50 (the largest pack), valid now and
for at most fifteen minutes. A paired browser is not asked for it. The meter
checks the signature, the payee and the amount again and simulates the
transfer before sending it.

## The settings, and their defaults

Each is a toggle: in the browser under *your settings* in the markets pane, in
the desktop app with `elo trade settings`.

| setting | default | what it does |
|---|---|---|
| approve a coin once | on | the first sale of a coin approves it for good (`/prepare/swap/*?approve=once`), so every later sale is one transaction; off approves the exact amount each time |
| approve once for the bridge | off | the same for LI.FI on moves between chains. Off by default: an unlimited approval to a bridge contract is what the 2024 LI.FI exploit drained |
| check before signing | on | each transaction is tried against the chain (`eth_call`; in the desktop app, `eth_estimateGas`) before the wallet is asked; silent unless it would fail |
| allow contracts outside our ecosystem | off | off refuses anything not on the list; on allows it with a warning |

## The log

Every request is logged on the user's own device, whatever the verdict:
what the API returned (to, value, calldata), what it decoded to, the verdict
(ok, warned, refused, stopped, declined), the hash and the result. The browser
keeps the last 300 in `localStorage` (`elo.signlog`; *your activity*, with a
save-as-file button); the desktop app appends to `activity.jsonl` beside the
keystore (`elo trade log`). A signed listing is logged too, beside the
transactions. Neither ever leaves the device.

## What else guards a connection

The desktop app checks every HTTPS certificate chain against Mozilla's roots,
minus the CAs browsers have stopped trusting (`elo-net/src/tls.rs`); a broken
chain is a refused connection, not a warning. Browsers do the same for the site.
