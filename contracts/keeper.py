#!/usr/bin/env python3
"""keeper.py — the trading half of the triangle watch, behind the four posted rules.

`triangle_watch.py` measures the cross and stops. This sends. It corrects the
OBOL/SCRY <-> MYRRH/SCRY <-> MYRRH/OBOL triangle when the three pools disagree
by more than the rule published in `keeper_rules.json`, and it does nothing else.

WHAT IT IS INSURANCE AGAINST, since that decides whether it is worth running:
not lost volume. The arbitrage IS the LP's loss and no AMM removes it
(`LAUNCH-DECISIONS.md` §the house as market maker) -- running it in-house
changes who captures that value, never whether it is taken. What it buys is an
honest fill for whoever swaps next, and the spread staying home instead of
leaving to a searcher. It is not a volume engine and cannot be used as one:
round-tripping the house's own money would falsify the buy/sell ratio this
audience reads to decide whether the team can take their money.

HOW THE FOUR RULES ARE ENFORCED, each in code rather than in prose:

  1. Its address is public. `keeper_rules.json` names it, and the signer must
     match it or the run aborts before quoting.
  2. It follows a posted rule. Every threshold, venue and size comes from that
     file and NOTHING is accepted from the command line. `--arm` additionally
     requires the file to be byte-identical to the copy at git HEAD, so the
     rule cannot be edited on the way to a trade -- the commit is the timestamp.
  3. It never trades ahead of an emission we have not published. The file
     records OBOL and MYRRH supply and the ScryHarvest root; every run re-reads
     them from chain and HALTS on any difference. Minting, or posting a root,
     stops the keeper until the new figure is committed. Publishing is a
     precondition of trading, not something that follows it.
  4. Its inventory is its own, never the pool seeds. It touches no LP position:
     the only calls it makes are an exact-amount `approve` and one router swap
     whose recipient is itself. There is no transfer-to-another-address path in
     this file, so no run of it can move funds anywhere but through the pools
     and back. A posted SCRY budget caps lifetime drawdown and it stops there.

WHY ONE TRANSACTION AND NOT THREE. The cycle goes through SwapRouter02's
multi-hop `exactInput`, so all three legs settle atomically in one transaction
with one `amountOutMinimum` on the final SCRY. Two consequences, both the point:
a leg can never leave the keeper stranded holding OBOL, and profitability is
enforced ON CHAIN -- min-out is the principal plus the posted floor, so if the
gap closes between quote and mine, the transaction REVERTS and costs gas rather
than executing a losing cycle.

WHY THE GAP DOES NOT DECIDE ALONE. Measured 2026-07-29: fees are 2.97% for the
three legs, price impact runs ~0.6% per 100,000 SCRY at current depth, and gas
is ~$0.015 a cycle. So a 4% gap on a depth-limited trade grosses less than its
own gas. The posted gap is a screen; the binding test is the live quote. The
keeper walks the posted size ladder, quotes each rung for real, and sends only
the best rung that clears both the floor and a multiple of its own gas -- so
size is solved against the book instead of frozen in a file, and every input to
the decision is a number anyone can re-read.

Expect it to fire rarely or never. That is the honest outcome, not a fault.

Usage:
  python3 keeper.py                  # DRY RUN: reads, quotes, sends nothing
  python3 keeper.py --arm            # sends, needs PRIVATE_KEY
  python3 keeper.py --watch 300      # re-read every 300s (add --arm to trade)
  python3 keeper.py --json           # machine output

Env:
  SCRY_RH_RPC   comma-separated RPC pool (see triangle_watch.py -- pass the
                full pool, a throttled single endpoint answers slowly then
                wrongly).
  PRIVATE_KEY   required only for --arm. Must be the posted keeper address.
  SCRY_KEEPER_LEDGER  override the ledger path (default below, outside git).
"""
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import triangle_watch as tw

HERE = Path(__file__).resolve().parent
RULES_PATH = HERE / "keeper_rules.json"
LEDGER = Path(os.getenv("SCRY_KEEPER_LEDGER",
                        "/data/apps/scry-data/keeper/triangle.jsonl"))
WAD = 10 ** 18


class Halt(Exception):
    """A posted rule said stop. Never a crash, always a printed reason."""


# ---------------------------------------------------------------- toolchain
def find_cast():
    """Upstream foundry, never the foundry-zksync fork. `deploy_town.sh` fails
    closed on the fork for forge; the same rule applies to anything that signs.
    If PATH hands us the fork and a sane upstream sits in the sanctioned
    side-by-side directory, use that rather than refusing over PATH order."""
    upstream = Path(os.getenv("UPSTREAM_FORGE_BIN",
                              str(Path.home() / ".foundry-upstream/bin"))) / "cast"
    on_path = shutil.which("cast")
    for cand in (upstream if upstream.is_file() else None, on_path):
        if not cand:
            continue
        try:
            v = subprocess.run([str(cand), "--version"], capture_output=True,
                               text=True, timeout=30).stdout
        except (OSError, subprocess.SubprocessError):
            continue
        if "zksync" not in v.lower():
            return str(cand), v.splitlines()[0].strip()
    raise Halt("no upstream cast found. PATH's cast is the foundry-zksync fork "
               "and ~/.foundry-upstream/bin/cast is missing or broken.\n"
               "   Install upstream side by side (never into ~/.foundry/bin, "
               "which the fork owns) — see contracts/deploy_town.sh.")


def cast(bin_, args, rpc, timeout=120):
    p = subprocess.run([bin_] + args + ["--rpc-url", rpc],
                       capture_output=True, text=True, timeout=timeout)
    if p.returncode != 0:
        raise Halt(f"cast {' '.join(args[:2])} failed: "
                   f"{(p.stderr or p.stdout).strip().splitlines()[0]}")
    return p.stdout.strip()


# ------------------------------------------------------------------- reads
def call_word(to, selector):
    """One no-arg getter, through triangle_watch's rotating RPC pool. Raises
    rather than returning zero: a value the chain would not give us is not a
    value of zero."""
    out = tw.eth_call(to, selector)
    if not out or out == "0x":
        raise Halt(f"{to}{selector} returned no data — wrong chain?")
    return int(out[2:66], 16)


def scry_per_weth(pool):
    """From the SCRY/WETH pool's own slot0. token0 is WETH and token1 is SCRY
    (verified on chain), so slot0's token1-per-token0 IS SCRY per WETH. This is
    how gas priced in ETH becomes gas priced in SCRY, with no external API and
    no USD in the loop."""
    return tw.slot0_price(pool)


def read_emissions(rules):
    e = rules["emissions"]
    h = e["harvest_obol"]
    return {
        "obol_total_supply_raw": str(call_word(tw.OBOL, "0x18160ddd")),
        "myrrh_total_supply_raw": str(call_word(tw.MYRRH, "0x18160ddd")),
        "harvest_root": "0x" + f"{call_word(h, '0xebf0c717'):064x}",
        "harvest_root_count": str(call_word(h, "0x21111bb4")),
        "harvest_total_committed_raw": str(call_word(h, "0x1d3231d4")),
    }


# ------------------------------------------------------------------- rules
def load_rules():
    try:
        raw = RULES_PATH.read_bytes()
    except OSError as ex:
        raise Halt(f"cannot read the posted rule at {RULES_PATH}: {ex}")
    return json.loads(raw), raw


def rule_is_committed(raw):
    """Rule 2's teeth: the rule in force must be the rule that is published.
    Returns (ok, detail) — the caller decides whether that blocks a send."""
    try:
        p = subprocess.run(["git", "show", "HEAD:contracts/keeper_rules.json"],
                           cwd=str(HERE.parent), capture_output=True, timeout=30)
    except (OSError, subprocess.SubprocessError) as ex:
        return False, f"could not ask git: {ex}"
    if p.returncode != 0:
        return False, "keeper_rules.json is not committed at HEAD"
    return (p.stdout == raw), ("matches HEAD" if p.stdout == raw
                               else "differs from the committed copy")


def check_emissions(rules, live):
    """Rule 3. Halts on anything the posted file did not already say."""
    posted = rules["emissions"]
    diffs = [(k, posted.get(k), v) for k, v in live.items() if posted.get(k) != v]
    if not diffs:
        return
    lines = "\n".join(f"     {k}\n       posted {p}\n       live   {l}"
                      for k, p, l in diffs)
    raise Halt(
        "supply moved and the posted rule does not say so yet — rule 3 halt.\n"
        f"{lines}\n"
        "   The keeper does not trade around an emission it has not published.\n"
        "   Record the live figures in contracts/keeper_rules.json, commit, "
        "then re-run.")


# -------------------------------------------------------------------- path
def v3_path(tokens, fee):
    """token, fee, token, fee, ... — packed exactly as v3 wants it."""
    out = tokens[0][2:]
    for t in tokens[1:]:
        out += f"{fee:06x}" + t[2:]
    return "0x" + out


def cycle_for(gap_pct):
    """Which way round. gap = direct/implied - 1 over MYRRH per OBOL.

    gap > 0: the direct pool pays MORE MYRRH per OBOL than routing through
    SCRY, so buy OBOL with SCRY, sell it into the rich direct pool, bring the
    MYRRH home:  SCRY -> OBOL -> MYRRH -> SCRY.
    gap < 0: the mirror.  SCRY -> MYRRH -> OBOL -> SCRY."""
    if gap_pct > 0:
        return ["SCRY", "OBOL", "MYRRH", "SCRY"], [tw.SCRY, tw.OBOL, tw.MYRRH, tw.SCRY]
    return ["SCRY", "MYRRH", "OBOL", "SCRY"], [tw.SCRY, tw.MYRRH, tw.OBOL, tw.SCRY]


# ------------------------------------------------------------------ ledger
def ledger_read():
    if not LEDGER.exists():
        return []
    rows = []
    for line in LEDGER.read_text().splitlines():
        line = line.strip()
        if line:
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return rows


def ledger_write(row):
    LEDGER.parent.mkdir(parents=True, exist_ok=True)
    with LEDGER.open("a") as f:
        f.write(json.dumps(row, sort_keys=True) + "\n")


def ledger_state(rows):
    """Realized lifetime SCRY delta and the last send. A cycle that reverted is
    recorded too — its principal came back, but its gas did not, and a keeper
    that forgot its reverts would understate what it has spent."""
    net = 0
    last = 0.0
    for r in rows:
        if r.get("event") == "cycle":
            net += int(r.get("scry_delta_raw", 0))
            last = max(last, float(r.get("at_unix", 0)))
    return net, last


# ------------------------------------------------------------------- decide
def evaluate(rules, cast_bin, rpc, x):
    """Walk the posted ladder against live quotes. Returns the best rung that
    clears the posted floor and the gas multiple, or None with the reason."""
    fee = rules["venues"]["fee_tier"]
    names, addrs = cycle_for(x["gap_pct"])
    path = v3_path(addrs, fee)

    gp = int(cast(cast_bin, ["gas-price"], rpc))
    per_weth = scry_per_weth(rules["venues"]["scry_weth_pool"])
    head = 1 + rules["gas_headroom_pct"] / 100

    rungs = []
    for size in rules["size_ladder_scry"]:
        if size > rules["max_trade_scry"]:
            continue
        amt = size * WAD
        out = cast(cast_bin, [
            "call", rules["venues"]["quoter"],
            "quoteExactInput(bytes,uint256)(uint256,uint160[],uint32[],uint256)",
            path, str(amt)], rpc)
        lines = [l for l in out.splitlines() if l.strip()]
        # amountOut is first and gasEstimate is LAST. Indexing the gas at [3]
        # would be a silent wrong number the day a long sqrtPriceX96AfterList
        # wraps onto two lines.
        got = int(lines[0].split()[0])
        swap_gas = int(lines[-1].split()[0])
        gas_units = int((swap_gas + rules["approve_gas_allowance"]) * head)
        gas_scry_raw = int(gas_units * gp * per_weth)
        net_raw = got - amt
        rungs.append({
            "size_scry": size,
            "amount_in_raw": amt,
            "quoted_out_raw": got,
            "net_raw": net_raw,
            "net_pct": (got / amt - 1) * 100,
            "gas_units": gas_units,
            "gas_scry": gas_scry_raw / WAD,
            "net_scry": net_raw / WAD,
            "net_after_gas_scry": (net_raw - gas_scry_raw) / WAD,
            "clears_floor": net_raw >= amt * rules["min_net_pct"] / 100,
            "clears_gas": net_raw >= gas_scry_raw * rules["profit_over_gas_x"],
        })

    ok = [r for r in rungs if r["clears_floor"] and r["clears_gas"]]
    best = max(ok, key=lambda r: r["net_after_gas_scry"]) if ok else None
    return {"cycle": " -> ".join(names), "path": path, "gas_price_wei": gp,
            "scry_per_weth": per_weth, "rungs": rungs, "best": best}


# ------------------------------------------------------------------ execute
def send_cycle(rules, cast_bin, rpc, key, me, ev, best):
    """Approve exactly what this cycle spends, then one atomic multi-hop swap
    wrapped in multicall so the deadline is real (SwapRouter02's ExactInputParams
    carries no deadline field — `prime_indexers.sh` and `watchtower/js/pools.js`
    both learned this, and an unwrapped swap can sit unmined and execute later
    at a price nobody agreed to)."""
    amt = best["amount_in_raw"]
    min_out = amt + amt * int(rules["min_net_pct"] * 100) // 10000
    router = rules["venues"]["router"]

    cast(cast_bin, ["send", tw.SCRY, "approve(address,uint256)", router, str(amt),
                    "--private-key", key], rpc)

    inner = subprocess.run(
        [cast_bin, "calldata", "exactInput((bytes,address,uint256,uint256))",
         f"({ev['path']},{me},{amt},{min_out})"],
        capture_output=True, text=True, timeout=60)
    if inner.returncode != 0:
        raise Halt(f"could not encode exactInput: {inner.stderr.strip()}")

    ts = int(cast(cast_bin, ["block", "latest", "--field", "timestamp"], rpc))
    before = call_word(tw.SCRY, "0x70a08231" + me[2:].lower().rjust(64, "0"))
    try:
        tx = cast(cast_bin, ["send", router, "multicall(uint256,bytes[])",
                             str(ts + 600), f"[{inner.stdout.strip()}]",
                             "--private-key", key], rpc, timeout=300)
    except Halt:
        # The approve landed and the swap did not — a gap that closed between
        # quote and send is the expected way to get here. Leaving the router a
        # standing allowance over a million SCRY because a trade did not happen
        # is not acceptable, so put it back to zero before surfacing the halt.
        try:
            cast(cast_bin, ["send", tw.SCRY, "approve(address,uint256)", router,
                            "0", "--private-key", key], rpc)
        except Halt as reset_ex:
            raise Halt(
                "the swap failed AND the allowance could not be reset — "
                f"{router} still holds an allowance of {amt / WAD:,.0f} SCRY "
                f"from {me}. Clear it by hand:\n"
                f"   cast send {tw.SCRY} 'approve(address,uint256)' {router} 0"
                f"\n   ({reset_ex})")
        raise
    after = call_word(tw.SCRY, "0x70a08231" + me[2:].lower().rjust(64, "0"))

    h = ""
    status = ""
    for line in tx.splitlines():
        if line.startswith("transactionHash"):
            h = line.split()[-1]
        if line.startswith("status"):
            status = line.split(maxsplit=1)[-1].strip()
    return {"tx": h, "status": status, "min_out_raw": min_out,
            "scry_before_raw": before, "scry_after_raw": after,
            "scry_delta_raw": after - before}


# ------------------------------------------------------------------- report
def report(rules, committed, live, x, ev, note):
    print("== the posted rule "
          f"({RULES_PATH.name}, posted {rules['posted']}, {committed[1]})")
    print(f"   keeper address   {rules['keeper_address']}")
    print(f"   fires at gap     >= {rules['trigger_gap_pct']:.2f}%"
          f"   floor {rules['min_net_pct']:.2f}% net"
          f"   and >= {rules['profit_over_gas_x']:g}x its own gas")
    print(f"   budget           {rules['budget_scry']:,} SCRY"
          f"   max size {rules['max_trade_scry']:,} SCRY")
    print()
    tw.report(live, x, show_rule=False)   # the header above already posted it
    print()
    print("== the posted rule against that measurement")
    print(f"   gap {x['gap_pct']:+.4f}%  vs trigger {rules['trigger_gap_pct']:.2f}%")
    if ev:
        print(f"   cycle {ev['cycle']}")
        print("   ladder, quoted live (net is after the 2.97% fees and impact):")
        for r in ev["rungs"]:
            flag = "OK" if r["clears_floor"] and r["clears_gas"] else "no"
            print(f"     {r['size_scry']:>9,} SCRY  net {r['net_pct']:+7.3f}%"
                  f"  {r['net_scry']:>+12,.1f} SCRY"
                  f"  gas {r['gas_scry']:>9,.1f} SCRY"
                  f"  after gas {r['net_after_gas_scry']:>+12,.1f}  {flag}")
    print()
    print(f"   {note}")


def once(args, rules, raw, committed, cast_bin, rpc):
    live = tw.read_triangle()
    x = tw.cross(live)
    check_emissions(rules, read_emissions(rules))

    ev = None
    note = ""
    result = None
    if abs(x["gap_pct"]) < rules["trigger_gap_pct"]:
        note = ("inside the posted trigger — nothing to correct. Any trade here "
                "would be volume, not price discovery.")
    else:
        ev = evaluate(rules, cast_bin, rpc, x)
        best = ev["best"]
        if not best:
            note = ("the gap clears the trigger but NO size clears the floor and "
                    "gas — the correction is worth less than the transaction that "
                    "makes it. Not trading is the right answer.")
        elif not args["arm"]:
            note = (f"would trade {best['size_scry']:,} SCRY for "
                    f"{best['net_after_gas_scry']:+,.1f} SCRY after gas. "
                    "Dry run — add --arm to send.")
        else:
            rows = ledger_read()
            net, last = ledger_state(rows)
            if time.time() - last < rules["min_interval_s"]:
                note = (f"cooling down — {rules['min_interval_s']}s between "
                        "cycles, posted.")
            elif net <= -rules["budget_scry"] * WAD:
                note = (f"budget spent: lifetime {net / WAD:+,.1f} SCRY against a "
                        f"posted {rules['budget_scry']:,} SCRY. Stopped. Raising "
                        "it is a separate operator sentence.")
            else:
                me = args["me"]
                bal = call_word(tw.SCRY, "0x70a08231" + me[2:].lower().rjust(64, "0"))
                if bal < best["amount_in_raw"]:
                    note = (f"{me} holds {bal / WAD:,.1f} SCRY, short of the "
                            f"{best['size_scry']:,} this cycle spends.")
                else:
                    result = send_cycle(rules, cast_bin, rpc, args["key"], me,
                                        ev, best)
                    ledger_write({"event": "cycle", "at_unix": time.time(),
                                  "gap_pct": x["gap_pct"], "cycle": ev["cycle"],
                                  "size_scry": best["size_scry"],
                                  "quoted_net_pct": best["net_pct"],
                                  "rule_posted": rules["posted"], **result})
                    d = int(result["scry_delta_raw"]) / WAD
                    note = (f"sent {result['tx']} status {result['status']} — "
                            f"realized {d:+,.4f} SCRY")

    if args["json"]:
        print(json.dumps({"rule": {k: v for k, v in rules.items()
                                   if not k.startswith("_")},
                          "rule_matches_head": committed[0],
                          "pools": live, "cross": x, "evaluation": ev,
                          "sent": result, "note": note},
                         indent=2, default=str))
    else:
        report(rules, committed, live, x, ev, note)
    return 0


def main():
    argv = sys.argv[1:]
    args = {"arm": "--arm" in argv, "json": "--json" in argv,
            "key": os.getenv("PRIVATE_KEY", ""), "me": ""}
    every = None
    if "--watch" in argv:
        i = argv.index("--watch")
        every = float(argv[i + 1]) if len(argv) > i + 1 else 300.0

    try:
        rules, raw = load_rules()
        cast_bin, cast_v = find_cast()
        rpc = tw.RPCS[0]
        committed = rule_is_committed(raw)

        if args["arm"]:
            # Two locks, and the posted one comes first. `triangle_watch.py`
            # prints this field to anyone who runs it, so a stranger reading
            # "disarmed" has to be right — which means the keeper itself must
            # refuse, not just the operator's habit of omitting --arm. An
            # unenforced flag beside a real one is worth nothing.
            if not rules.get("armed", False):
                raise Halt(
                    'refusing to arm: the posted rule says "armed": false.\n'
                    "   That field is what triangle_watch.py shows the public. "
                    "Arming is\n"
                    "   an operator act with a paper trail: set it true in "
                    "contracts/keeper_rules.json,\n"
                    "   commit, then --arm.")
            if not committed[0]:
                raise Halt(
                    f"refusing to arm: {committed[1]}.\n"
                    "   Rule 2 means the rule in force is the rule that is "
                    "published. Commit contracts/keeper_rules.json first — the "
                    "commit is the timestamp on it.")
            if not args["key"]:
                raise Halt("--arm needs PRIVATE_KEY")
            me = subprocess.run([cast_bin, "wallet", "address", "--private-key",
                                 args["key"]], capture_output=True, text=True,
                                timeout=30).stdout.strip()
            if me.lower() != rules["keeper_address"].lower():
                raise Halt(
                    f"refusing to arm: this key is {me}, and the posted keeper "
                    f"address is {rules['keeper_address']}.\n"
                    "   Rule 1 means the address is public BEFORE its first "
                    "transaction. Publish this one or use the published key.")
            args["me"] = me
    except Halt as ex:
        print(f"HALT: {ex}", file=sys.stderr)
        return 2

    while True:
        try:
            rc = once(args, rules, raw, committed, cast_bin, rpc)
        except Halt as ex:
            print(f"HALT: {ex}", file=sys.stderr)
            rc = 2
        except (RuntimeError, subprocess.SubprocessError, OSError) as ex:
            print(f"could not complete a pass: {ex}", file=sys.stderr)
            rc = 2
        if not every:
            return rc
        time.sleep(every)


if __name__ == "__main__":
    sys.exit(main())
