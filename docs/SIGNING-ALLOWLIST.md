---
status: live
lane: [platform, security]
updated: 2026-10-23
about: "every contract our apps will ask a wallet to sign for, what each one is and what we ask of it; the four signing settings and their defaults; and the log each app keeps of everything it was asked to sign"
---
# SIGNING-ALLOWLIST.md: what our apps will ask your wallet to sign

Operator, 2026-10-23: *"it should be pretty limited to our ecosystem… we
should know what's on the allowlist and how they work… just log what it all
said so if something odd happens we catch it."*

## The rule

The trade area of the site (the trade room, the cross-chain room, the
collections room) and the desktop app's `elo trade` sign a transaction only
when it is:

- a **call to a contract on the list below**, or
- an **approval whose spender is on the list** (`approve`, or a Permit2
  `approve`).

A token contract is listed for approvals only: nothing in the trade area moves
a coin with a direct `transfer`, so one is never signed. Anything else is
refused before the wallet opens, unless the user turns on *allow contracts
outside our ecosystem*, in which case it is allowed with a warning and logged.

The list ships **inside each app**: `watchtower/js/signguard.js` in the browser,
`launcher-rs/crates/elo-launcher/src/guard.rs` in the desktop app. An API
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
| Seaport 1.6 (OpenSea) | Robinhood Chain | `0x0000000000000068F116a894984e2DB1123eB395` | buying one of our collections' listings |
| the reserve token | Robinhood Chain | `0xf7997e4fd821e39d884d86d25e1655017f241a42` | approvals only, to a listed spender |
| JUNK | Robinhood Chain | `0xd6bd13a8a16e5249fa54961fc92eee0307e0f1b9` | approvals only |
| WETH | Robinhood Chain | `0x0bd7d308f8e1639fab988df18a8011f41eacad73` | approvals only |
| USDG | Robinhood Chain | `0x5fc5360d0400a0fd4f2af552add042d716f1d168` | approvals only |

**Not on it:** EloRouter (written, never broadcast: the reserve is the tier-one
coin and JUNK trades on Abyss's own exchange). A Solana transaction (Jupiter,
LI.FI from Solana) is a serialized transaction the Solana wallet simulates and
shows itself; our apps log it and do not decode it.

Adding a row is a change to both apps and this page, in one commit.

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
keystore (`elo trade log`). Neither ever leaves the device.

## What else guards a connection

The desktop app checks every HTTPS certificate chain against Mozilla's roots,
minus the CAs browsers have stopped trusting (`elo-net/src/tls.rs`); a broken
chain is a refused connection, not a warning. Browsers do the same for the site.
