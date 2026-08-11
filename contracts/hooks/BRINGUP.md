---
status: design
lane: [economy, ops]
updated: 2026-08-09
about: "bringing up contracts/hooks — how to compile, mine and deploy ScryClock and ScryTide, and the checklist that must clear before either holds a cent"
---
# BRINGUP.md — contracts/hooks, Uniswap v4

> Named `BRINGUP.md` rather than `README.md` because the doc shelf serves
> `/library` by basename and the root `README.md` already owns that name.

> ## ✅ IT COMPILES AND THE UNIT TESTS PASS — 2026-08-09
>
> Written 2026-07-31 in a container with **no `forge`**; first compiled and run
> on a box that has one. **33 passed, 0 failed, 1 skipped** — `ScryClockTest`
> 23/23 (+1 skip), `ScryTideTest` 10/10. Sizes: `ScryClock` 5,988 B runtime,
> `ScryTide` 2,780 B, both far inside EIP-170.
>
> The prediction below was right — the first build failed on imports, and every
> bug the run found was in v4 plumbing or in the tests, none in either hook's
> own arithmetic. What it took is in §churn.
>
> **What this does NOT mean.** The one skipped test is the fork swap, and it is
> still a `TODO` with a hard `vm.skip(true)` — so `_afterSwap`'s delta-return
> path, the single most intricate thing in `ScryClock`, has still never run
> against a real `PoolManager`. That box below stays unticked.
>
> Nothing in here is deployed. `../deployments.json` is the record of what is,
> and both hooks are absent from it on purpose.

## What is here

| hook | what it is | pool | permission bits |
|---|---|---|---|
| **`ScryClock`** | the Longbarrow Clock — buying the prize coin skims a slice into a pot and resets a countdown; last buyer takes it. `docs/onchain/HOOKS.md` §2.1 | **funded** — halve the v3 MYRRH/OBOL pair | `0x2044` — beforeInitialize, afterSwap, afterSwapReturnDelta |
| **`ScryTide`** | the fee rises and falls on a fixed public cycle, forever. `docs/onchain/HOOKS.md` §2.2 | **none yet** — see below | `0x2080` — beforeInitialize, beforeSwap |

`ScryClock` is **not novel** and the card must not claim it is — the format
shipped twice on v4 already (§2.8). Both are dead, neither published source,
and on 4663 we would be the only one running. `ScryTide` turned up no prior art
in either registry, which is a weaker claim than "nobody has done it."

### ⚠ The blocker for a second pool is depth, not code

Measured 2026-07-31: the house's uncommitted float is **94.78 OBOL and 178.13
MYRRH** — everything else is already pooled. Every hook is a new pool starting
at zero (`HOOKS.md` R1), and the one raidable source of depth is halving the v3
MYRRH/OBOL pair, ~$2,002, which the Clock has first claim on.

So `DeployScryTide.s.sol` has its pair deliberately left as zero addresses and
refuses to run. Fill it when a pair and its depth exist. `HOOKS.md` §5 §the
depth ceiling carries the options, including the cheap one: a single hook can
implement both `beforeSwap` and `afterSwap` and put both mechanisms on one pool.

## Why this is a separate foundry project

The parent project builds against `forge-std` alone. A file in `../src` that
imports v4-core would break `forge build`, and therefore `./deploy_town.sh
test`, for everyone until the libraries are installed. That gate runs before
every broadcast. Keeping this project separate keeps the town's build green
while the hook is still being brought up.

## Bringing it up

This is the whole install, and it is two packages rather than three:

```bash
cd contracts/hooks
forge install --no-git foundry-rs/forge-std
forge install --no-git OpenZeppelin/uniswap-hooks
forge build
forge test -vvv
```

`remappings.txt` is committed and is not optional — it points the bare
`v4-core/` and `v4-periphery/` prefixes at the copies **vendored inside
uniswap-hooks** (v4-core 1.0.2, v4-periphery 1.0.3) rather than at separately
installed ones. That is the point: `BaseHook` and the core it compiles against
travel together, so they cannot drift into two silently different versions.
Do not `forge install Uniswap/v4-core` alongside it.

### §churn — what the first build actually found

The prediction that the imports would break first was correct. The resolution
was not either of the alternatives this table used to guess at:

| import | what was wrong | fixed to |
|---|---|---|
| `v4-periphery/src/utils/BaseHook.sol` | **the file is not in v4-periphery at all.** Uniswap deleted it 2026-02-06 — *"remove hooks and move to hook repo"* (v4-periphery#510), five months before this project was written. Both alternatives guessed at here were already dead too. | `@openzeppelin/uniswap-hooks/src/base/BaseHook.sol` — where Uniswap's own `v4-template` now points |
| `SwapParams` from `v4-core/src/types/PoolOperation.sol` | nothing — correct as written | — |
| `HookMiner` from `v4-periphery/src/utils/` | nothing — correct in the vendored 1.0.3 | — |

Then four compile errors that were not imports, all in our own code:

| error | cause |
|---|---|
| `NotPoolManager` already declared | OZ's `BaseHook` declares it. Ours is deleted; inheriting it is better anyway — one selector for one condition. |
| `PoolKey key` already declared (both suites) | `Deployers` already has a `key`; ours shadowed it. |
| three invalid address checksums | `DeployScryClock`/`DeployScryTide` carried the PoolManager, OBOL and MYRRH addresses in the wrong EIP-55 case. **The hex was right** — checked against `../deployments.json` and `HOOKS.md` §80 — so this was capitalization, not a wrong address. |
| `ScryClock.NotPoolManager.selector` not found | follows from the first; the test now asserts `BaseHook.NotPoolManager`. |

And two test bugs that only a run could find:

- **`TickMisaligned(-120, 200)`** in both suites' `setUp`. Both pools use
  `tickSpacing` 200, but liquidity was added at ticks ±120 — Uniswap's stock
  `LIQUIDITY_PARAMS`, which is written for spacing 60. Now full-range, which is
  also what `test_the_horizon_caps_the_deadline` needs: 60 one-ether buys walk
  the price out of any narrow band, and the next swap reverts
  `PriceLimitAlreadyExceeded`.
- **Both constructor-rejection tests passed for the wrong reason.** Each was
  `vm.expectRevert(BadParameter); new ScryHook(...)`. `BaseHook`'s constructor
  validates the address bits *before* the subclass constructor body runs, so a
  `new` — landing wherever CREATE puts it — always reverts
  `HookAddressNotValid`, and the parameter guards are never reached. They now
  run the constructor at an address carrying the right permission bits, so the
  guards are genuinely exercised; a separate test pins the address check.

## Licensing

Only `v4-core/src/PoolManager.sol` is BUSL-1.1. `IHooks.sol`,
`StateLibrary.sol` and **all of v4-periphery, including `BaseHook`, are MIT** —
and we call the deployed singleton rather than copying it. Nothing here needs
a licence grant. (`NOW.md`, checked before this project existed.)

## Deploying — the two things that are not optional

**1. The address is the permission set.** v4 encodes a hook's permissions in
the low bits of its own address, so deployment is a CREATE2 salt grind until
the address matches `getHookPermissions()`. `ScryClock` needs
`BEFORE_INITIALIZE | AFTER_SWAP | AFTER_SWAP_RETURNS_DELTA` = **`0x2044`**;
`ScryTide` needs `BEFORE_INITIALIZE | BEFORE_SWAP` = **`0x2080`**.
`BaseHook`'s constructor validates the match, so a wrong salt fails loudly at
deploy rather than quietly at the first swap.

**2. Deploy and initialize in the SAME transaction.** The hook address is part
of `PoolKey`, so anyone can open a pool against our hook — including one full
of worthless tokens, to drive our clock. `ScryClock` binds to the first pool
that initializes and reverts on every later one, which turns that attack into a
race. `DeployScryClock.s.sol` closes the race by doing both in one script run.
Splitting them re-opens it.

## Before it holds a cent

- [x] It compiles — both hooks. (2026-08-09)
- [x] `forge test` passes — 33/34, the one skip being the fork test below.
      Someone has read the tests rather than the count: two of them were
      passing for the wrong reason and are rewritten (§churn).
- [ ] **A fork test against 4663 has swapped through it** — the delta-return
      path in `_afterSwap` is the single most intricate thing in the file and
      unit tests with a mock PoolManager will not prove it.
      ⚠ `test_fork_real_swap_through_the_real_singleton` is still a `TODO`
      body behind `vm.skip(true)`. It is the only unit-level thing still owed,
      and it needs a mined hook address, so it is real work rather than a flag.
- [ ] Source-verified via the `/api/v2` recipe in `../RUNBOOK.md` §verify.
      `forge verify-contract` speaks the legacy `/api` route, which
      rate-limits this box near-permanently.
- [ ] The parameters are the ones an operator actually chose, since not one of
      them can be changed afterwards.
- [ ] **ScryTide only:** the pool was opened with `LPFeeLibrary.DYNAMIC_FEE_FLAG`
      and a swap genuinely costs more at high tide. Both of that hook's failure
      modes are SILENT — a static-fee pool ignores the tide, and a returned fee
      without `OVERRIDE_FEE_FLAG` is discarded. Neither reverts.
- [ ] `../deployments.json` updated in the same commit as the broadcast.
