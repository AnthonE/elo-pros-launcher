#!/usr/bin/env python3
"""Laws for `keeper.py` — the triangle keeper's four posted rules, and the
arithmetic that decides whether it sends.

WHY THIS FILE HAS TO EXIST. The keeper's trading path cannot be exercised by
running it: the live gap is ~0.02% against a 4.00% trigger, so a plain run
returns "inside the posted trigger" and every line that picks a direction,
builds a path, quotes a ladder and chooses a size stays cold. That is the
`CLAUDE.md` trap about a suite reporting a pass for work it did not do, wearing
a different hat — and the first real execution of that code would otherwise be
the one spending money. So the profitable branch is driven here with a stubbed
quoter, and the direction/path/ladder code is driven against the real chain.

Network is REQUIRED and its absence is a FAIL, never a skip. Two sections read
chain (the live ladder, and rule 3 against real supply); a run that could not
look must not print a pass.

  python3 test_keeper.py           # needs SCRY_RH_RPC (the pool, not one url)
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import keeper as K                        # noqa: E402
import triangle_watch as tw               # noqa: E402

PASS = FAIL = 0


def check(name: str, cond: bool, extra: str = "") -> None:
    global PASS, FAIL
    if cond:
        PASS += 1
        print(f"  ok   {name}")
    else:
        FAIL += 1
        print(f"  FAIL {name}" + (f" — {extra}" if extra else ""))


rules, raw = K.load_rules()

# ---------------------------------------------------------------- 1. the path
print("\n== 1. the v3 path, byte for byte")
# This exact string was quoted against QuoterV2 on 2026-07-29 and returned a
# real amountOut, so it is a verified vector and not a re-derivation of the
# same code under test.
VERIFIED = ("0xDa2a4b23459e9ca88183e990802be644AcA7C4B0"
            "002710a003af4a6c38629a986545afc8f9312c7eb76220"
            "002710de967108cC27dB651e8cFeC7dd18db814508b893"
            "002710Da2a4b23459e9ca88183e990802be644AcA7C4B0")
built = K.v3_path([tw.SCRY, tw.OBOL, tw.MYRRH, tw.SCRY], 10000)
check("3-hop path matches the vector the quoter accepted",
      built.lower() == VERIFIED.lower(), f"built {built}")
check("path is 89 bytes (20+3+20+3+20+3+20)", (len(built) - 2) // 2 == 89,
      f"{(len(built) - 2) // 2} bytes")
check("fee tier encodes as 3 bytes, not 32", "002710" in built.lower())

print("\n== 2. which way round the cycle goes")
pos_names, pos_addrs = K.cycle_for(+5.0)
neg_names, neg_addrs = K.cycle_for(-5.0)
# gap > 0 means the direct pool pays MORE MYRRH per OBOL than the SCRY route,
# so OBOL must be SOLD into the direct pool: buy it first with SCRY.
check("gap>0 goes SCRY -> OBOL -> MYRRH -> SCRY",
      pos_names == ["SCRY", "OBOL", "MYRRH", "SCRY"], str(pos_names))
check("gap<0 mirrors it", neg_names == ["SCRY", "MYRRH", "OBOL", "SCRY"],
      str(neg_names))
check("both cycles start and end in SCRY (so inventory is SCRY at rest)",
      pos_addrs[0] == pos_addrs[-1] == tw.SCRY
      and neg_addrs[0] == neg_addrs[-1] == tw.SCRY)
check("the two directions are not the same path",
      K.v3_path(pos_addrs, 10000) != K.v3_path(neg_addrs, 10000))

# ------------------------------------------------------- 3. rule 4, structural
print("\n== 3. rule 4: there is no way out of this file")
src = (Path(__file__).resolve().parent / "keeper.py").read_text()
# The claim in the docstring is that no run can move funds anywhere but through
# the pools and back. That is only true while these stay absent, and a later
# edit that adds one should break a test rather than a promise.
for bad in ("transfer(", "transferFrom(", "sweep(", "withdraw(",
            "collect(", "decreaseLiquidity("):
    check(f"no {bad} anywhere in keeper.py", bad not in src)
check("the only approve is exact-amount, never unlimited",
      "approve(address,uint256)" in src
      and "MaxUint" not in src and "ffffffff" not in src.lower())
check("every swap recipient is the keeper itself",
      "{me}," in src or ",{me}," in src.replace(" ", ""))
check("the swap is wrapped in multicall so the deadline is real",
      "multicall(uint256,bytes[])" in src)
check("min-out is principal + the posted floor, enforced on chain",
      'min_out = amt + amt * int(rules["min_net_pct"] * 100) // 10000' in src)
# Unreachable from a test that must not send, so it is asserted structurally:
# an approve that landed while its swap reverted must not leave the router a
# standing allowance over the budget.
check("a failed swap resets the allowance to zero",
      'approve(address,uint256)", router,\n                            "0"' in src
      or '"0", "--private-key"' in src)
check("...and says so by hand if even the reset fails",
      "allowance could not be reset" in src)

print("\n== 4. the min-out arithmetic")
amt = 100_000 * K.WAD
min_out = amt + amt * int(rules["min_net_pct"] * 100) // 10000
check("0.75% floor on 100,000 SCRY is 750 SCRY",
      (min_out - amt) / K.WAD == 750.0, f"{(min_out - amt) / K.WAD}")
check("min-out exceeds the principal (a cycle cannot settle at a loss)",
      min_out > amt)

# -------------------------------------------------- 5. the profitable branch
print("\n== 5. the branch that has never run: a quote that clears")
real_cast = K.cast


def stub_factory(out_scry, gas_units=259149):
    def stub(bin_, args, rpc, timeout=120):
        if args[0] == "gas-price":
            return "21858000"
        if args[0] == "call":
            amt_in = int(args[-1])
            got = int(amt_in * out_scry)
            return f"{got} [x]\n[1, 2, 3]\n[0, 0, 0]\n{gas_units} [x]"
        raise AssertionError(f"stub saw an unexpected cast call: {args[0]}")
    return stub


K.cast = stub_factory(1.02)          # every rung returns +2%
ev = K.evaluate(rules, "cast", "rpc", {"gap_pct": 5.0})
check("a +2% cycle produces a chosen size", ev["best"] is not None)
if ev["best"]:
    check("it picks the rung with the best net after gas",
          ev["best"]["net_after_gas_scry"]
          == max(r["net_after_gas_scry"] for r in ev["rungs"]))
    check("the chosen rung cleared both gates",
          ev["best"]["clears_floor"] and ev["best"]["clears_gas"])
    check("no rung exceeds the posted max size",
          all(r["size_scry"] <= rules["max_trade_scry"] for r in ev["rungs"]))
    check("gas is priced in SCRY, not assumed away",
          ev["best"]["gas_scry"] > 0)

K.cast = stub_factory(1.004)         # +0.4%: above zero, under the 0.75% floor
ev_thin = K.evaluate(rules, "cast", "rpc", {"gap_pct": 5.0})
check("a +0.4% cycle is refused by the 0.75% floor", ev_thin["best"] is None)
check("...and the refusal is legible per rung",
      all(not r["clears_floor"] for r in ev_thin["rungs"]))

# A cycle that clears the floor but not gas: 1% on a tiny size. At 25,000 SCRY,
# 1% is 250 SCRY against ~808 SCRY of gas.
K.cast = stub_factory(1.01)
ev_gas = K.evaluate(rules, "cast", "rpc", {"gap_pct": 5.0})
small = next(r for r in ev_gas["rungs"] if r["size_scry"] == 25000)
check("a small rung clears the floor but NOT gas",
      small["clears_floor"] and not small["clears_gas"],
      f"floor {small['clears_floor']} gas {small['clears_gas']} "
      f"net {small['net_scry']:.1f} vs gas {small['gas_scry']:.1f}")
check("...and the keeper still trades, at a size where gas is worth paying",
      ev_gas["best"] is not None and ev_gas["best"]["size_scry"] > 25000)
K.cast = real_cast

# ------------------------------------------------------------ 6. the ledger
print("\n== 6. the ledger and the posted budget")
with tempfile.TemporaryDirectory() as d:
    K.LEDGER = Path(d) / "triangle.jsonl"
    K.ledger_write({"event": "cycle", "at_unix": 1000.0,
                    "scry_delta_raw": str(-800 * K.WAD)})
    K.ledger_write({"event": "cycle", "at_unix": 2000.0,
                    "scry_delta_raw": str(+2000 * K.WAD)})
    net, last = K.ledger_state(K.ledger_read())
    check("realized delta sums across cycles", net == 1200 * K.WAD,
          f"{net / K.WAD}")
    check("the last send is the newest, not the last written", last == 2000.0)
    # A reverted cycle returns the principal and keeps the gas. A keeper that
    # dropped those rows would understate what it has spent.
    K.ledger_write({"event": "cycle", "at_unix": 3000.0, "status": "0",
                    "scry_delta_raw": "0"})
    net2, _ = K.ledger_state(K.ledger_read())
    check("a reverted cycle is still a row", len(K.ledger_read()) == 3)
    check("...and does not change realized SCRY", net2 == net)
    K.LEDGER.write_text('{"event": "cycle", "at_unix": 1.0, "scry_de\n')
    check("a truncated line does not crash the reader",
          K.ledger_state(K.ledger_read()) == (0, 0.0))

print("\n== 7. rule 2: the rule in force is the rule that is published")
ok, detail = K.rule_is_committed(raw)
check("git is asked, and answers one way or the other",
      isinstance(ok, bool) and bool(detail), detail)
check("a rule that differs from HEAD is reported as differing",
      K.rule_is_committed(raw + b" ")[0] is False)
check("no threshold is read from argv",
      "--gap" not in src and "--size" not in src and "--min" not in src)
for k in ("trigger_gap_pct", "min_net_pct", "profit_over_gas_x",
          "size_ladder_scry", "max_trade_scry", "budget_scry",
          "min_interval_s", "keeper_address"):
    check(f"{k} is posted in keeper_rules.json", k in rules)

print("\n== 8. rule 1 + rule 2 refuse a live --arm (end to end)")
kb, _ = K.find_cast()
throwaway = subprocess.run([kb, "wallet", "new"], capture_output=True, text=True,
                           timeout=60).stdout
tk = ""
for line in throwaway.splitlines():
    if "Private key" in line:
        tk = line.split(":")[-1].strip()
check("made a throwaway key to arm with (it holds nothing, ever)",
      tk.startswith("0x") and len(tk) == 66)
if tk:
    env = dict(os.environ, PRIVATE_KEY=tk)
    p = subprocess.run([sys.executable, str(Path(__file__).parent / "keeper.py"),
                        "--arm"], capture_output=True, text=True, timeout=300,
                       env=env)
    err = p.stderr
    check("--arm with an unposted key exits non-zero", p.returncode != 0,
          f"rc={p.returncode}")
    check("...and refuses before it quotes anything",
          "refusing to arm" in err, err.strip()[:160])
    # Whichever guard is live must be the one that fired, in the posted order:
    # the public "armed" flag, then the committed rule, then the address. Never
    # both, never neither — a refusal that names the wrong reason is a refusal
    # nobody can act on.
    if not rules.get("armed", False):
        check('the public "armed": false is what stops it first',
              '"armed": false' in err, err.strip()[:200])
    elif not ok:
        check("an uncommitted rule is caught by rule 2 next",
              "Commit contracts/keeper_rules.json" in err, err.strip()[:200])
    else:
        check("a committed, armed rule means rule 1 catches the wrong address",
              "posted keeper address" in err, err.strip()[:200])
    check("the armed flag is enforced by code, not just printed",
          'rules.get("armed", False)' in src)
    check("triangle_watch shows the public the same field the keeper obeys",
          '"armed"' in (Path(__file__).parent / "triangle_watch.py").read_text())
    check("nothing was sent (no transaction hash in the output)",
          "transactionHash" not in p.stdout + err)

# ------------------------------------------- 9 + 10. against the real chain
print("\n== 9. rule 3 against live supply (network required)")
try:
    live_em = K.read_emissions(rules)
except (K.Halt, RuntimeError) as ex:
    check("could read OBOL/MYRRH supply and the harvest root", False, str(ex))
    live_em = None

if live_em:
    check("read live supply and root", True)
    try:
        K.check_emissions(rules, live_em)
        posted_matches = True
    except K.Halt:
        posted_matches = False
    check("the posted emissions figures match chain right now", posted_matches,
          "keeper_rules.json is stale — that is the halt working, but commit "
          "the live figures")
    for field in ("obol_total_supply_raw", "harvest_root",
                  "harvest_root_count"):
        perturbed = dict(live_em)
        perturbed[field] = (perturbed[field] + "1"
                            if not perturbed[field].startswith("0x")
                            else "0x" + "1" * 64)
        halted = False
        try:
            K.check_emissions(rules, perturbed)
        except K.Halt as ex:
            halted = field in str(ex)
        check(f"a changed {field} halts the keeper", halted)

print("\n== 10. the live ladder, real quotes (network required)")
try:
    cast_bin, _ = K.find_cast()
    ev_live = K.evaluate(rules, cast_bin, tw.RPCS[0], {"gap_pct": 5.0})
except (K.Halt, RuntimeError, subprocess.SubprocessError) as ex:
    check("quoted the ladder against the live pools", False, str(ex))
    ev_live = None

if ev_live:
    check("every posted rung got a real quote",
          len(ev_live["rungs"]) == len(rules["size_ladder_scry"])
          and all(r["quoted_out_raw"] > 0 for r in ev_live["rungs"]))
    check("gas price came from chain", ev_live["gas_price_wei"] > 0)
    check("SCRY per WETH is a sane order of magnitude (1e7..1e9)",
          1e7 < ev_live["scry_per_weth"] < 1e9,
          f"{ev_live['scry_per_weth']:,.0f}")
    # The real gap is well inside the trigger, so a forced +5% picks a direction
    # and the honest quotes come back losing roughly the fee cost. If any rung
    # showed a profit here, the triangle would actually be dislocated.
    check("with the triangle tight, no rung clears — so nothing would send",
          ev_live["best"] is None,
          "a rung cleared: the pools may genuinely be dislocated, "
          "re-read triangle_watch.py")
    worst = min(r["net_pct"] for r in ev_live["rungs"])
    best_pct = max(r["net_pct"] for r in ev_live["rungs"])
    check("impact grows with size (biggest rung is the worst)",
          worst < best_pct < 0, f"best {best_pct:.3f}% worst {worst:.3f}%")
    check("round-trip on the smallest rung is close to the 2.97% fee cost",
          -3.4 < best_pct < -2.9, f"{best_pct:.3f}%")

print(f"\n{PASS} passed, {FAIL} failed")
if FAIL:
    print("This suite requires network. A section that could not look is a "
          "FAIL above, never a silent skip.")
sys.exit(1 if FAIL else 0)
