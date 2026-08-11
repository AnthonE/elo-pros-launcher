#!/usr/bin/env python3
"""triangle_watch.py — read the three game pools and price the triangle.

READ-ONLY. Holds no key, signs nothing, sends nothing. It answers one
question: are OBOL/SCRY, MYRRH/SCRY and MYRRH/OBOL still consistent with
each other, and if not, is the gap big enough that correcting it is worth
doing?

WHY THIS STAYS READ-ONLY NOW THAT A KEEPER EXISTS. The trading half is
`keeper.py`, and it lives in its own file on purpose. `LAUNCH-DECISIONS.md`
§the house as market maker posts four rules a keeper arms with, and rule 2
is "it follows a posted rule — the venues it arbitrages and the deviation
it acts on are published, not discretionary." A published threshold nobody
outside can check against a live measurement is still just a claim. So this
file holds the measurement the keeper triggers on, prints the threshold
from the same committed `keeper_rules.json` the keeper reads, holds no key,
and can be run by a stranger on their own box to see exactly what the house
said it would do and whether the pools are there yet.

WHAT IT IS NOT FOR, said out loud because the request that produced it was
for the other thing: this is not a volume bot. Round-tripping the house's
own money to inflate the trade count falsifies buy/sell ratio, which
`CLAUDE.md`'s own front matter names as one of the two metrics this
audience uses to decide whether the team can take their money. The file
also says: "What it forbids: pretending the surfaces are busier than they
are. We publish our zeros on purpose." A pool that self-corrects because a
posted rule said it would is a service to whoever trades next; a pool that
looks busy because the house traded with itself is a lie with a chart.

THE ARITHMETIC THAT MAKES THE TRIANGLE STICKY. Every leg is the 1% tier, so
a three-leg round trip pays ~3% in fees before gas. The cross therefore has
to drift more than that before correction is profitable at all — which is
why an honest keeper here trades RARELY, and why arbitrage cannot be a
volume strategy no matter who runs it.

Usage:
  python3 triangle_watch.py                # one read, human output
  python3 triangle_watch.py --json         # machine output
  python3 triangle_watch.py --watch 60     # re-read every 60s

Env:
  SCRY_RH_RPC   comma-separated RPC pool (falls back to the public endpoint).
                Pass the full pool: a throttled single endpoint does not
                error, it answers slowly and then wrongly.
"""
import json
import os
import sys
import time
import urllib.error
import urllib.request

RPCS = [u.strip() for u in os.getenv(
    "SCRY_RH_RPC",
    "https://rpc.mainnet.chain.robinhood.com").split(",") if u.strip()]
TIMEOUT_S = float(os.getenv("SCRY_TRIANGLE_TIMEOUT", "15"))

SCRY = "0xda2a4b23459e9ca88183e990802be644aca7c4b0"
OBOL = "0xa003af4a6c38629a986545afc8f9312c7eb76220"
MYRRH = "0xde967108cc27db651e8cfec7dd18db814508b893"

SYM = {SCRY: "SCRY", OBOL: "OBOL", MYRRH: "MYRRH"}

# Pools, and the price each one POSTED at genesis (LAUNCH-DECISIONS.md).
# `posted` is expressed as token1-per-token0, the same orientation slot0
# gives, so nothing has to be flipped at comparison time.
POOLS = [
    {"label": "OBOL/SCRY", "addr": "0xfba63d463d7d2b868dd023ef41e9903cd296de12",
     "t0": OBOL, "t1": SCRY, "posted": 10.0},
    {"label": "MYRRH/SCRY", "addr": "0xa8b10968383bd31e68e5a024029cefd678020590",
     "t0": SCRY, "t1": MYRRH, "posted": 0.02},
    {"label": "MYRRH/OBOL", "addr": "0x18f02c6f868b1012140f06a08775abbc8158cb15",
     "t0": OBOL, "t1": MYRRH, "posted": 0.2},
]

FEE_BPS_PER_LEG = 100          # the 1% tier
LEGS = 3
ROUND_TRIP_COST = (1 - (1 - FEE_BPS_PER_LEG / 10000) ** LEGS) * 100  # ≈2.97%

_rpc_i = 0


def rpc(method, params):
    """One JSON-RPC call, rotating the pool. Raises rather than returning a
    zero — a price the chain would not give us is NOT a price of zero, the
    same rule `deploy_town.sh`'s v3_sqrtp() learned the hard way."""
    global _rpc_i
    body = json.dumps({"jsonrpc": "2.0", "id": 1,
                       "method": method, "params": params}).encode()
    last = None
    for _ in range(len(RPCS)):
        url = RPCS[_rpc_i % len(RPCS)]
        _rpc_i += 1
        try:
            req = urllib.request.Request(
                url, data=body, headers={"Content-Type": "application/json"})
            with urllib.request.urlopen(req, timeout=TIMEOUT_S) as r:
                d = json.loads(r.read())
            if "error" in d:
                last = RuntimeError(f"{url}: {d['error']}")
                continue
            return d["result"]
        except (urllib.error.URLError, TimeoutError, OSError,
                json.JSONDecodeError) as e:
            last = e
    raise RuntimeError(f"every RPC in the pool failed; last: {last}")


def eth_call(to, data):
    return rpc("eth_call", [{"to": to, "data": data}, "latest"])


def slot0_price(addr):
    """token1 per token0, from sqrtPriceX96. This is the pool's own number —
    not an indexer's, not a cached one."""
    out = eth_call(addr, "0x3850c7bd")            # slot0()
    if not out or out == "0x":
        raise RuntimeError(f"{addr}: slot0() returned no data — wrong chain?")
    sqrt_x96 = int(out[2:66], 16)
    if sqrt_x96 == 0:
        raise RuntimeError(f"{addr}: sqrtPriceX96 is 0 — pool uninitialised")
    return (sqrt_x96 / 2 ** 96) ** 2


def read_triangle():
    live = []
    for p in POOLS:
        price = slot0_price(p["addr"])
        drift = (price / p["posted"] - 1) * 100
        live.append({**p, "price": price, "drift_pct": drift})
    return live


def cross(live):
    """The MYRRH/OBOL rate IMPLIED by the two SCRY legs, versus the rate the
    third pool actually quotes. At genesis these agree exactly (50/10 = 5),
    which is the whole reason the third pool was opened by the house rather
    than left for a stranger to open off-cross."""
    obol_scry = next(p for p in live if p["label"] == "OBOL/SCRY")["price"]
    myrrh_per_scry = next(p for p in live if p["label"] == "MYRRH/SCRY")["price"]
    direct = next(p for p in live if p["label"] == "MYRRH/OBOL")["price"]
    # MYRRH per OBOL, the long way: OBOL -> SCRY -> MYRRH
    implied = obol_scry * myrrh_per_scry
    gap = (direct / implied - 1) * 100 if implied else float("nan")
    return {"implied_myrrh_per_obol": implied,
            "direct_myrrh_per_obol": direct,
            "gap_pct": gap,
            "round_trip_cost_pct": ROUND_TRIP_COST,
            "actionable": abs(gap) > ROUND_TRIP_COST}


def posted_rule():
    """The keeper's published threshold, read from the file the keeper itself
    reads. A stranger running THIS file sees the number the house acts on, which
    is what makes rule 2 checkable from outside rather than promised. Returns
    None when the file is not beside us — the read still works without it."""
    try:
        p = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                         "keeper_rules.json")
        with open(p) as f:
            r = json.load(f)
        return {"posted": r.get("posted", "?"),
                "trigger_gap_pct": float(r["trigger_gap_pct"]),
                "min_net_pct": float(r["min_net_pct"]),
                "keeper_address": r.get("keeper_address", "?"),
                "armed": bool(r.get("armed", False))}
    except (OSError, ValueError, KeyError, TypeError):
        return None


def report(live, x, show_rule=True):
    print("== the three game pools, read off slot0 "
          "(the pool's own number, not an index)")
    for p in live:
        s0, s1 = SYM[p["t0"]], SYM[p["t1"]]
        print(f"   {p['label']:11} {p['price']:>14,.6f} {s1} per {s0}"
              f"   posted {p['posted']:<8g} drift {p['drift_pct']:+.4f}%")
    print()
    print("== is the triangle consistent?")
    print(f"   MYRRH per OBOL, implied by the two SCRY legs : "
          f"{x['implied_myrrh_per_obol']:.8f}")
    print(f"   MYRRH per OBOL, quoted by the direct pool    : "
          f"{x['direct_myrrh_per_obol']:.8f}")
    print(f"   gap                                          : {x['gap_pct']:+.4f}%")
    print(f"   a 3-leg round trip costs                     : "
          f"{x['round_trip_cost_pct']:.2f}% in fees, before gas")
    print()
    if x["actionable"]:
        print("   ** the gap EXCEEDS the fee cost — correcting it is profitable.")
        print("      If the house does not, an external searcher will, and that")
        print("      value leaves the LP either way (LAUNCH-DECISIONS.md: doing")
        print("      it in-house changes WHO captures it, never whether it is")
        print("      taken). Still not automatic: arming a keeper is its own")
        print("      operator sentence, and rules 1-4 post before it trades.")
    else:
        print("   the gap is inside the fee cost, so there is nothing to correct.")
        print("   No trade here would be price discovery; it would only be volume,")
        print("   and volume the house pays for is not a signal about anything.")

    r = posted_rule() if show_rule else None
    if r:
        print()
        print(f"== the house's posted rule (keeper_rules.json, {r['posted']})")
        print(f"   the keeper at {r['keeper_address']}")
        print(f"   acts only when this gap exceeds {r['trigger_gap_pct']:.2f}% "
              f"AND a live quote clears {r['min_net_pct']:.2f}% net over gas.")
        print(f"   right now: gap {x['gap_pct']:+.4f}% — "
              f"{'ABOVE' if abs(x['gap_pct']) >= r['trigger_gap_pct'] else 'below'}"
              " the trigger. "
              f"Trading half {'ARMED' if r['armed'] else 'disarmed'}.")
        print("   You are reading the same file the keeper reads. Nothing here "
              "holds a key.")


def main():
    args = sys.argv[1:]
    as_json = "--json" in args
    every = None
    if "--watch" in args:
        i = args.index("--watch")
        every = float(args[i + 1]) if len(args) > i + 1 else 60.0
    while True:
        try:
            live = read_triangle()
            x = cross(live)
        except RuntimeError as e:
            print(f"could not read the triangle: {e}", file=sys.stderr)
            if not every:
                return 2
            time.sleep(every)
            continue
        if as_json:
            print(json.dumps({"pools": [
                {k: v for k, v in p.items() if k != "posted"} | {"posted": p["posted"]}
                for p in live], "cross": x}, indent=2, default=str))
        else:
            report(live, x)
        if not every:
            return 0
        time.sleep(every)


if __name__ == "__main__":
    sys.exit(main())
