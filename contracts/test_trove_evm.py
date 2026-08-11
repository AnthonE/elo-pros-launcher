#!/usr/bin/env python3
"""test_trove_evm.py — ScryTrove's behaviour, EXECUTED, on a real EVM.

    cd contracts && python3 test_trove_evm.py

**Why this exists beside a forge suite rather than instead of one.** This box has
no foundry (`ITEM-CONTRACT.md` §3a: the egress policy blocks the installer), and
a contract that has only ever been COMPILED is a contract whose behaviour nobody
has observed. solc ships on npm and `py-evm` ships on PyPI, so between them the
real bytecode can be deployed and driven here. `contracts/test/ScryTrove.t.sol`
remains the canonical gate and is still owed on a box with foundry — this proves
the claims, it does not replace the suite.

⚠ **A skip here is LOUD.** Missing deps print and exit 0 with a reason; they
never print a pass. `CLAUDE.md`'s first trap is a suite reporting a pass for work
it did not do.

WHAT IS ACTUALLY PROVEN, and the list is the design:

  * a wrapper is minted against a real escrowed token, and `contentsOf` names it
  * **the whole point**: two DIFFERENT collections wrap into ONE address, so a
    gacha keyed by collection sees one pool holding both
  * a `GachaLike` harness — `ScryGacha.deposit`'s exact pull, plain
    `transferFrom` then an `ownerOf` assertion — moves a wrapper, and the buyer
    it is pushed to can unwrap into the real NFT
  * the original depositor gets NOTHING back after the wrapper leaves them: a
    recallable wrapper is not a thing anyone should have paid for
  * a paused collection cannot be wrapped, and a soulbound one cannot either —
    the failure lands at the wrap instead of stranding a gacha position
  * the same token cannot be wrapped twice, and an unwrapped token can be
    re-wrapped
  * nothing in the wrapper's own transfer path can revert for a reason of its
    own — no pause, no hook, no admin door
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ART = Path(os.getenv("TROVE_ARTIFACTS", "/tmp/artifacts.json"))

try:
    from eth_tester import EthereumTester, PyEVMBackend
    from eth_tester.exceptions import TransactionFailed
    from web3 import EthereumTesterProvider, Web3
except Exception as e:  # noqa: BLE001
    print(f"SKIP (loudly, never a silent pass): missing deps ({e})")
    print("  pip install eth-tester py-evm web3")
    sys.exit(0)

if not ART.exists():
    print(f"SKIP (loudly, never a silent pass): no artifacts at {ART}")
    print("  build them with solc first — see contracts/RUNBOOK.md, or:")
    print("  npm i solc@0.8.26 && node <emit script> ScryTrove.sol test/TroveMocks.sol")
    sys.exit(0)

ARTIFACTS = json.loads(ART.read_text())
FAILS: list[str] = []


def ok(cond, label):
    print(f"   {'ok  ' if cond else 'FAIL'}  {label}")
    if not cond:
        FAILS.append(label)


def reverts(fn, label):
    try:
        fn()
    except (TransactionFailed, Exception):  # noqa: BLE001 — any revert counts
        ok(True, label)
        return
    ok(False, label)


w3 = Web3(EthereumTesterProvider(EthereumTester(PyEVMBackend())))
ACC = w3.eth.accounts
HOUSE, ARTIST, BUYER, STRANGER = ACC[0], ACC[1], ACC[2], ACC[3]


def deploy(name, *args):
    a = ARTIFACTS[name]
    c = w3.eth.contract(abi=a["abi"], bytecode=a["bin"])
    tx = c.constructor(*args).transact({"from": HOUSE})
    rc = w3.eth.wait_for_transaction_receipt(tx)
    return w3.eth.contract(address=rc.contractAddress, abi=a["abi"])


def send(fn, frm):
    return w3.eth.wait_for_transaction_receipt(fn.transact({"from": frm}))


print("[the wrap]")
trove = deploy("ScryTrove", "https://scry.moreright.xyz/api/trove/")
skins = deploy("MockNft")   # a game's item contract
art = deploy("MockNft")     # a completely different collection — an artist's

send(skins.functions.mint(ARTIST, 7), HOUSE)
send(skins.functions.approve(trove.address, 7), ARTIST)
send(trove.functions.wrap(skins.address, 7), ARTIST)

ok(trove.functions.ownerOf(1).call() == ARTIST, "wrapping mints the wrapper to the wrapper's maker")
ok(skins.functions.ownerOf(7).call() == trove.address, "…and the real token is escrowed in the trove")
coll, tid, dep, live, fung = trove.functions.contentsOf(1).call()
ok(coll == skins.address and tid == 7 and dep == ARTIST and live and not fung,
   "…and contentsOf names exactly what it is standing for, and that it is an NFT")

# ── THE WHOLE POINT ──────────────────────────────────────────────────────────
print("\n[many collections, one address — which is one gacha pool]")
send(art.functions.mint(HOUSE, 99), HOUSE)
send(art.functions.approve(trove.address, 99), HOUSE)
send(trove.functions.wrap(art.address, 99), HOUSE)

ok(skins.address != art.address, "the two collections really are different contracts")
ok(trove.functions.ownerOf(1).call() == ARTIST and trove.functions.ownerOf(2).call() == HOUSE,
   "two wrappers exist, made by different people, from different collections")
ok(trove.functions.contentsOf(1).call()[0] != trove.functions.contentsOf(2).call()[0],
   "…and they hold tokens from DIFFERENT collections")
ok(trove.functions.totalSupply().call() == 2,
   "…yet both are the SAME ERC-721 address, so ScryGacha's mapping(address => Pool) "
   "sees ONE pool holding both — the pot stops being per-collection")

# ── the gacha's own pull, on the wrapper ─────────────────────────────────────
print("\n[the gacha's exact pull, then a buyer unwrapping into the real thing]")
gacha = deploy("GachaLike")
send(trove.functions.approve(gacha.address, 1), ARTIST)
send(gacha.functions.pull(trove.address, 1), ARTIST)
ok(trove.functions.ownerOf(1).call() == gacha.address,
   "ScryGacha.deposit's pull — plain transferFrom, then ownerOf == self — works on a wrapper")

send(gacha.functions.push(trove.address, BUYER, 1), HOUSE)
ok(trove.functions.ownerOf(1).call() == BUYER, "a settled draw hands the wrapper to the buyer")

send(trove.functions.unwrap(1), BUYER)
ok(skins.functions.ownerOf(7).call() == BUYER,
   "…and the buyer unwraps it into the REAL NFT — the wrapper was never the prize")
ok(trove.functions.totalSupply().call() == 1, "…and the wrapper is burned")
ok(trove.functions.contentsOf(1).call()[3] is False, "…and reads as no longer live")

print("\n[what the depositor does NOT keep]")
reverts(lambda: send(trove.functions.unwrap(1), ARTIST),
        "the original depositor cannot unwrap after the wrapper left them — a "
        "recallable wrapper is not a thing anyone should have paid for")
reverts(lambda: send(trove.functions.unwrap(2), STRANGER),
        "…and a stranger cannot unwrap somebody else's wrapper")

print("\n[a collection that cannot survive the gacha cannot survive the wrap]")
paused = deploy("PausableNft")
send(paused.functions.mint(HOUSE, 1), HOUSE)
send(paused.functions.approve(trove.address, 1), HOUSE)
send(paused.functions.setPaused(True), HOUSE)
reverts(lambda: send(trove.functions.wrap(paused.address, 1), HOUSE),
        "a PAUSED collection cannot be wrapped — the failure lands here instead of "
        "stranding a position mid-draw and jamming the whole pool")
send(paused.functions.setPaused(False), HOUSE)
send(trove.functions.wrap(paused.address, 1), HOUSE)
ok(trove.functions.contentsOf(3).call()[0] == paused.address,
   "…and the same collection wraps fine once the switch is off")

soul = deploy("SoulboundNft")
send(soul.functions.mint(HOUSE, 1), HOUSE)
reverts(lambda: send(trove.functions.wrap(soul.address, 1), HOUSE),
        "a SOULBOUND token cannot be wrapped — caught by the pull, as it is in gacha_fit")

print("\n[donated TOKENS — the operator's correction, 2026-08-11]")
coin = deploy("MockToken")
send(coin.functions.mint(HOUSE, 1000 * 10**18), HOUSE)
send(coin.functions.approve(trove.address, 500 * 10**18), HOUSE)
before = trove.functions.nextId().call()
send(trove.functions.wrapTokens(coin.address, 500 * 10**18), HOUSE)
tw = before
ok(trove.functions.ownerOf(tw).call() == HOUSE, "an ERC-20 wraps into the SAME collection as the NFTs")
c, amt, dep2, live2, fung2 = trove.functions.contentsOf(tw).call()
ok(c == coin.address and amt == 500 * 10**18 and fung2 and live2,
   "…and contentsOf reports it as FUNGIBLE, with tokenId carrying the AMOUNT")
ok(coin.functions.balanceOf(trove.address).call() == 500 * 10**18,
   "…and the trove really holds the tokens")

# The same token wrapped twice must both succeed — wrapperOf is a 721 guard only.
send(coin.functions.approve(trove.address, 250 * 10**18), HOUSE)
send(trove.functions.wrapTokens(coin.address, 250 * 10**18), HOUSE)
ok(trove.functions.contentsOf(trove.functions.nextId().call() - 1).call()[1] == 250 * 10**18,
   "the same token wraps MANY times — a fungible is not a token id, so no double-wrap guard applies")

send(trove.functions.unwrap(tw), HOUSE)
ok(coin.functions.balanceOf(HOUSE).call() == 750 * 10**18,
   "unwrapping a token pack returns exactly what went in")

skim = deploy("SkimToken")
send(skim.functions.mint(HOUSE, 1000 * 10**18), HOUSE)
send(skim.functions.approve(trove.address, 100 * 10**18), HOUSE)
reverts(lambda: send(trove.functions.wrapTokens(skim.address, 100 * 10**18), HOUSE),
        "a FEE-ON-TRANSFER token is REFUSED by the balance-delta pull — a wrapper "
        "minted against the stated amount would promise more than it holds")
reverts(lambda: send(trove.functions.wrapTokens(coin.address, 0), HOUSE),
        "a zero-amount wrap is refused")

# ── THE REAL ECOSYSTEM PATH ──────────────────────────────────────────────────
# Everything above drives MOCKS. This drives the actual contract a listed game
# would deploy, through the actual wrapper, into the gacha's actual pull. If
# ScryItem and ScryTrove ever stop fitting each other, this is what says so.
print("\n[the real path: a real ScryItem through the real trove]")
try:
    item = deploy("ScryItem", "Gates Items", "GATES", HOUSE, 500, "https://scry.moreright.xyz/i/")
except KeyError:
    print("   SKIP (loudly): ScryItem is not in the artifacts — add it to build_artifacts.js")
    item = None

if item is not None:
    # ⚠ The owner is NOT a minter by default — ScryItem's own gate, and it is
    # right. Discovered by this test reverting with "not a minter".
    send(item.functions.setMinter(HOUSE, True), HOUSE)
    send(item.functions.mint(ARTIST, 1, 0, b"\x11" * 32), HOUSE)
    ok(item.functions.ownerOf(1).call() == ARTIST, "a real ScryItem mints to a holder")

    send(item.functions.approve(trove.address, 1), ARTIST)
    send(trove.functions.wrap(item.address, 1), ARTIST)
    wid = trove.functions.nextId().call() - 1
    ok(item.functions.ownerOf(1).call() == trove.address,
       "…wraps into the trove with the plain transferFrom the gacha uses")

    send(trove.functions.approve(gacha.address, wid), ARTIST)
    send(gacha.functions.pull(trove.address, wid), ARTIST)
    ok(trove.functions.ownerOf(wid).call() == gacha.address,
       "…survives ScryGacha.deposit's exact pull")

    send(gacha.functions.push(trove.address, BUYER, wid), HOUSE)
    send(trove.functions.unwrap(wid), BUYER)
    ok(item.functions.ownerOf(1).call() == BUYER,
       "…and a buyer who drew it ends up holding the REAL ScryItem")

    recv, amt = item.functions.royaltyInfo(1, 10**18).call()
    ok(recv == HOUSE and amt == 5 * 10**16,
       "…with the CREATOR's ERC-2981 royalty intact — wrapping never touched it")

    # The compatibility rules, asserted against the pair that will actually be
    # deployed rather than against a mock.
    for name, ct in (("ScryItem", item), ("ScryTrove", trove)):
        ok(ct.functions.supportsInterface("0x80ac58cd").call(), f"{name} answers ERC-721")
        ok(not ct.functions.supportsInterface("0xd9b67a26").call(),
           f"{name} does NOT claim ERC-1155, which ScryGacha cannot take")

print("\n[the bookkeeping]")
reverts(lambda: send(trove.functions.wrap(art.address, 99), HOUSE),
        "the same token cannot be wrapped twice")
reverts(lambda: send(trove.functions.wrap(HOUSE, 1), HOUSE),
        "an address with no code is not a collection")
send(trove.functions.unwrap(2), HOUSE)
send(art.functions.approve(trove.address, 99), HOUSE)
before = trove.functions.nextId().call()
send(trove.functions.wrap(art.address, 99), HOUSE)
fresh = trove.functions.nextId().call() - 1
ok(fresh == before and trove.functions.contentsOf(fresh).call()[0] == art.address,
   f"an unwrapped token can be re-wrapped, and gets a NEW id (#{fresh}) — ids are never reused")
reverts(lambda: trove.functions.ownerOf(2).call(),
        "…and the burned id stays burned")

print("\n[nothing clever in the transfer path — the rule the trove imposes on itself]")
# Asserted against the compiled ABI, not against the source text. A grep for
# "pause" also matches the header PARAGRAPH explaining why there is no pause,
# which is how a guard test ends up asserting the absence of its own
# documentation. The ABI is what a caller can actually reach.
FNS = {e["name"] for e in ARTIFACTS["ScryTrove"]["abi"] if e.get("type") == "function"}
for fn, why in (
    ("paused", "no paused() — the probe gacha_fit refuses other contracts for"),
    ("setPaused", "no pause switch"),
    ("pause", "…nor a pause under another name"),
    ("owner", "no owner at all — there is no admin seat to hold"),
    ("transferOwnership", "…so there is nothing to hand over"),
    ("royaltyInfo", "no ERC-2981 — the gacha pays no royalty, and a wrapper claiming "
                    "one would advertise a payment nothing here makes"),
    ("rescue", "no admin rescue — unwrap is the only way a token leaves"),
    ("setBaseURI", "the metadata base is welded at deploy, not repointable"),
):
    ok(fn not in FNS, f"{why} (ABI)")
src = (HERE / "src" / "ScryTrove.sol").read_text()
for word, why in (("selfdestruct", "no selfdestruct"), ("delegatecall", "no delegatecall")):
    ok(word not in src, f"{why} in the source")
ok(trove.functions.supportsInterface("0x80ac58cd").call(), "it answers ERC-165 for ERC-721")
ok(not trove.functions.supportsInterface("0xd9b67a26").call(),
   "…and does NOT claim ERC-1155, which ScryGacha cannot take")
ok(not trove.functions.supportsInterface("0x2a55205a").call(), "…and does not claim ERC-2981")

runtime = ARTIFACTS["ScryTrove"].get("runtime") or 0
ok(0 < runtime < 24576, f"EIP-170: {runtime} B runtime, {24576 - runtime} B of margin")

print()
if FAILS:
    print(f"FAILED {len(FAILS)}")
    for f in FAILS:
        print("   -", f)
    sys.exit(1)
print(f"all good — {len(ARTIFACTS)} artifacts, executed on py-evm. "
      "The forge suite is still the canonical gate and is still owed.")
