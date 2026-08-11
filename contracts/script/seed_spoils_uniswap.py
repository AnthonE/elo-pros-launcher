#!/usr/bin/env python3
"""seed_spoils_uniswap.py — compute the exact numbers to seed OBOL/SCRY and
MYRRH/SCRY on canonical Uniswap v3 (RH-Chain), then print the `export` block
that `script/SeedSpoilsUniswapV3.s.sol` consumes.

Why this lives in Python (POOLS.md §2.2): sqrtPriceX96 encodes a RAW-unit price
and token decimals differ in the general case — eyeballing it is how a fresh
pool gets sniped. The math is done and tested here; the Solidity script only
takes the results and lands create+initialize+mint atomically.

The price you pass is the LIVE cross rate at execution time. For a spoil with no
external market yet, that is a chosen opening price (the first mint IS the price
until the first trade). Recompute and re-run minutes before broadcasting.

Usage:
    python3 seed_spoils_uniswap.py \
        --scry   0xDa2a4b23459e9ca88183e990802be644AcA7C4B0 \
        --obol   0x...  --obol-price  0.01   --obol-scry-budget  5000 \
        --myrrh  0x...  --myrrh-price  0.05   --myrrh-scry-budget 2500 \
        --slip-bps 100

    # price  = SCRY per 1 spoil (SCRY_per_OBOL / SCRY_per_MYRRH)
    # budget = how much SCRY (human units) to put on the quote side of that pool
    #          (the base-token amount is derived: budget / price)

Run the self-check any time:
    python3 seed_spoils_uniswap.py --selftest
"""
from __future__ import annotations
import argparse
import sys
from decimal import Decimal, getcontext
from math import isqrt

getcontext().prec = 60

Q96 = 1 << 96
MIN_SQRT_RATIO = 4295128739
MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342


def sqrt_price_x96(price_raw: float) -> int:
    """floor(sqrt(price_raw) * 2**96) via integer sqrt (no float rounding at the
    last bit). price_raw = token1-raw per token0-raw."""
    # sqrt(price_raw) * 2**96 == sqrt(price_raw * 2**192).  Do it in integers by
    # representing price_raw as a fraction num/den with high precision.
    num = int(price_raw * 10**36)
    den = 10**36
    # sqrt(num/den) * 2**96 = isqrt(num * 2**192 // den)
    val = isqrt((num * (Q96 * Q96)) // den)
    return val


def plan_pool(tag: str, spoil: str, scry: str, price_scry_per_spoil: float,
              scry_budget: float, spoil_dec: int, scry_dec: int,
              slip_bps: int, quote_sym: str = "SCRY") -> dict:
    """Plan one v3 pool.

    `scry` / `scry_budget` / `price_scry_per_spoil` are the QUOTE side and were
    named for the only case that existed at first. Nothing here is SCRY-specific
    -- the third pool (2026-07-25) prices MYRRH in OBOL by passing the OBOL
    address as `scry` and quote_sym="OBOL". The names are kept so the frozen
    selftest keyword calls still read the same.
    """
    if price_scry_per_spoil <= 0:
        raise ValueError(f"{tag}: price must be > 0")
    if scry_budget <= 0:
        raise ValueError(f"{tag}: scry budget must be > 0")

    spoil = _norm(spoil)
    scry = _norm(scry)
    if spoil == scry:
        raise ValueError(f"{tag}: spoil == scry")

    # Exact decimal math so the raw amounts have no float-tail artifacts.
    price_d = Decimal(str(price_scry_per_spoil))
    scry_budget_d = Decimal(str(scry_budget))
    spoil_budget_d = scry_budget_d / price_d          # base amount, human units
    spoil_budget = float(spoil_budget_d)
    base_raw = int((spoil_budget_d * (10**spoil_dec)).to_integral_value())
    quote_raw = int((scry_budget_d * (10**scry_dec)).to_integral_value())
    if base_raw <= 0 or quote_raw <= 0:
        raise ValueError(f"{tag}: rounded to zero amount — raise the budget")

    # Address-sorted pair; sqrtP is token1-raw per token0-raw.
    if int(spoil, 16) < int(scry, 16):
        token0, token1 = spoil, scry
        amount0, amount1 = base_raw, quote_raw
    else:
        token0, token1 = scry, spoil
        amount0, amount1 = quote_raw, base_raw

    price_raw = amount1 / amount0  # token1/token0 in raw units
    sp = sqrt_price_x96(price_raw)
    if not (MIN_SQRT_RATIO <= sp <= MAX_SQRT_RATIO):
        raise ValueError(f"{tag}: sqrtPriceX96 {sp} out of v3 range")

    return {
        "tag": tag, "token0": token0, "token1": token1,
        "amount0": amount0, "amount1": amount1,
        "base_raw": base_raw, "quote_raw": quote_raw,
        "sqrtp": sp, "price_raw": price_raw,
        "spoil_budget": spoil_budget, "scry_budget": scry_budget,
        "quote_sym": quote_sym,
        "amount0_min": amount0 * (10_000 - slip_bps) // 10_000,
        "amount1_min": amount1 * (10_000 - slip_bps) // 10_000,
    }


def _norm(addr: str) -> str:
    """Validate and lowercase a 20-byte hex address.

    The hex check is not decoration. This module's stdout is consumed by
    `deploy_town.sh`, which evaluates the `export` lines it prints. Length and
    prefix alone would admit 40 arbitrary characters -- including shell
    metacharacters -- into that stream. Nothing untrusted reaches here today
    (the addresses come from town.env, written by the operator's own broadcast
    receipts), so this is defence in depth on a path that runs seconds before
    real value moves, not a patched exploit.
    """
    addr = addr.strip()
    if not addr.startswith("0x") or len(addr) != 42:
        raise ValueError(f"bad address: {addr!r}")
    body = addr[2:]
    if not all(c in "0123456789abcdefABCDEF" for c in body):
        raise ValueError(f"address is not hex: {addr!r}")
    return addr.lower()


def render_exports(plans: list[dict]) -> str:
    lines = []
    for p in plans:
        t = p["tag"].upper()
        q = p.get("quote_sym", "SCRY")
        base = p['tag'].split('_')[0]
        lines.append(f"# {base}/{q}  —  {p['spoil_budget']:.4f} {base} + "
                     f"{p['scry_budget']:.4f} {q}, token0={p['token0']}")
        lines.append(f"export {t}_SQRTP={p['sqrtp']}")
        lines.append(f"export {t}_BASE_RAW={p['base_raw']}")
        lines.append(f"export {t}_QUOTE_RAW={p['quote_raw']}")
    return "\n".join(lines)


def _selftest() -> int:
    # A tokenB < tokenA case and its mirror must give reciprocal raw prices, and
    # the amount ratio must recover the intended human price both ways.
    scry = "0x" + "d" * 40
    lo = "0x" + "1" * 40   # sorts before scry -> spoil is token0
    hi = "0x" + "f" * 40   # sorts after  scry -> scry  is token0

    for spoil, name in [(lo, "lo"), (hi, "hi")]:
        p = plan_pool("OBOL", spoil, scry, price_scry_per_spoil=0.01,
                      scry_budget=5000.0, spoil_dec=18, scry_dec=18, slip_bps=100)
        # 5000 SCRY at 0.01 SCRY/OBOL -> 500000 OBOL
        assert abs(p["spoil_budget"] - 500000.0) < 1e-6, (name, p["spoil_budget"])
        # recover human price = SCRY_raw/OBOL_raw (decimals equal)
        if int(spoil, 16) < int(scry, 16):
            recovered = p["amount1"] / p["amount0"]  # scry/obol
        else:
            recovered = p["amount0"] / p["amount1"]  # scry/obol
        assert abs(recovered - 0.01) < 1e-9, (name, recovered)
        assert MIN_SQRT_RATIO <= p["sqrtp"] <= MAX_SQRT_RATIO
        # sqrtP must square back to price_raw within a hair
        got = (p["sqrtp"] / Q96) ** 2
        assert abs(got - p["price_raw"]) / p["price_raw"] < 1e-6, (name, got, p["price_raw"])

    # decimals that differ (e.g. a 6-dp quote) must still recover the human price
    p = plan_pool("MYRRH", lo, scry, price_scry_per_spoil=2.0, scry_budget=1000.0,
                  spoil_dec=18, scry_dec=6, slip_bps=50)
    # token0 = spoil(18dp), token1 = scry(6dp); human price = scry/spoil
    human = (p["amount1"] / 10**6) / (p["amount0"] / 10**18)
    assert abs(human - 2.0) < 1e-9, human

    # The third pool: MYRRH priced in OBOL, no SCRY anywhere in it. The
    # quote side being another spoil must change nothing about the math, and
    # the opening rate must be exactly the cross of the two SCRY pools --
    # 50 SCRY/MYRRH divided by 10 SCRY/OBOL = 5 OBOL/MYRRH. If those two ever
    # disagree there IS a genesis arb, which is the whole reason we open it.
    obol_a, myrrh_a = lo, hi
    p3 = plan_pool("MYRRH_OBOL", myrrh_a, obol_a, price_scry_per_spoil=50.0 / 10.0,
                   scry_budget=10_000_000.0, spoil_dec=18, scry_dec=18, slip_bps=100,
                   quote_sym="OBOL")
    assert p3["quote_sym"] == "OBOL"
    assert abs(p3["spoil_budget"] - 2_000_000.0) < 1e-6, p3["spoil_budget"]
    # 10M OBOL against 2M MYRRH is the posted LAUNCH-DECISIONS.md size
    if int(myrrh_a, 16) < int(obol_a, 16):
        recovered = p3["amount1"] / p3["amount0"]
    else:
        recovered = p3["amount0"] / p3["amount1"]
    assert abs(recovered - 5.0) < 1e-9, recovered
    assert MIN_SQRT_RATIO <= p3["sqrtp"] <= MAX_SQRT_RATIO
    got3 = (p3["sqrtp"] / Q96) ** 2
    assert abs(got3 - p3["price_raw"]) / p3["price_raw"] < 1e-6, (got3, p3["price_raw"])
    # and the env block it renders must carry the MYRRH_OBOL_* keys the script reads
    block = render_exports([p3])
    for key in ("MYRRH_OBOL_SQRTP", "MYRRH_OBOL_BASE_RAW", "MYRRH_OBOL_QUOTE_RAW"):
        assert f"export {key}=" in block, key
    print("seed_spoils_uniswap selftest: OK")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--selftest", action="store_true", help="run internal checks and exit")
    ap.add_argument("--scry")
    ap.add_argument("--scry-dec", type=int, default=18)
    ap.add_argument("--obol"); ap.add_argument("--obol-price", type=float)
    ap.add_argument("--obol-scry-budget", type=float); ap.add_argument("--obol-dec", type=int, default=18)
    ap.add_argument("--myrrh"); ap.add_argument("--myrrh-price", type=float)
    ap.add_argument("--myrrh-scry-budget", type=float); ap.add_argument("--myrrh-dec", type=int, default=18)
    # The THIRD pool: MYRRH priced in OBOL, no SCRY. We open it ourselves
    # rather than let a stranger set the tick -- see the seed script's header.
    ap.add_argument("--myrrh-obol-price", type=float, default=5.0,
                    help="OBOL per MYRRH (default 5.0 = 50/10, consistent with the two SCRY pools)")
    ap.add_argument("--myrrh-obol-budget", type=float,
                    help="OBOL side of the MYRRH/OBOL pool; omit to skip the third pool")
    ap.add_argument("--slip-bps", type=int, default=100)
    args = ap.parse_args()

    if args.selftest:
        return _selftest()
    if not args.scry:
        ap.error("--scry is required (or use --selftest)")

    plans = []
    if args.obol:
        if args.obol_price is None or args.obol_scry_budget is None:
            ap.error("--obol needs --obol-price and --obol-scry-budget")
        plans.append(plan_pool("OBOL", args.obol, args.scry, args.obol_price,
                               args.obol_scry_budget, args.obol_dec, args.scry_dec, args.slip_bps))
    if args.myrrh:
        if args.myrrh_price is None or args.myrrh_scry_budget is None:
            ap.error("--myrrh needs --myrrh-price and --myrrh-scry-budget")
        plans.append(plan_pool("MYRRH", args.myrrh, args.scry, args.myrrh_price,
                               args.myrrh_scry_budget, args.myrrh_dec, args.scry_dec, args.slip_bps))
    if args.myrrh_obol_budget is not None:
        if not (args.obol and args.myrrh):
            ap.error("--myrrh-obol-budget needs BOTH --obol and --myrrh (it is their pool)")
        # THE CROSS-CONSISTENCY GATE (security review 2026-07-25). The third
        # pool exists to foreclose a genesis arb, so opening it off the cross
        # of the other two creates the exact thing it was added to prevent --
        # and the intra-pool sqrtP-vs-amounts tripwire cannot see it, because
        # that only checks one pool against itself. Fail closed instead.
        cross = args.myrrh_price / args.obol_price
        drift = abs(args.myrrh_obol_price - cross) / cross
        if drift > args.slip_bps / 10_000:
            ap.error(
                f"third pool would open OFF-CROSS: {args.myrrh_obol_price:g} OBOL/MYRRH "
                f"vs the cross implied by the two SCRY pools "
                f"({args.myrrh_price:g}/{args.obol_price:g} = {cross:g}), "
                f"a {drift:.2%} gap against a {args.slip_bps/100:g}% tolerance. "
                "That gap is free money for the first arber, paid out of the LP "
                "position this same run mints. Pass --myrrh-obol-price "
                f"{cross:g} (or fix the two ratios)."
            )
        # base = MYRRH, quote = OBOL. Same helper, different quote token.
        plans.append(plan_pool("MYRRH_OBOL", args.myrrh, args.obol, args.myrrh_obol_price,
                               args.myrrh_obol_budget, args.myrrh_dec, args.obol_dec,
                               args.slip_bps, quote_sym="OBOL"))
    if not plans:
        ap.error("give at least one of --obol / --myrrh")

    print("# --- computed at the price you passed; RE-RUN minutes before broadcast (POOLS.md 2.2) ---")
    print(f"export SCRY_TOKEN={_norm(args.scry)}")
    if args.obol:
        print(f"export OBOL_TOKEN={_norm(args.obol)}")
    if args.myrrh:
        print(f"export MYRRH_TOKEN={_norm(args.myrrh)}")
    print(render_exports(plans))
    print()
    for p in plans:
        print(f"# {p['tag']}: initialize price (raw t1/t0) = {p['price_raw']:.6g}, "
              f"sqrtPriceX96 = {p['sqrtp']}, slip mins = "
              f"{p['amount0_min']}/{p['amount1_min']}")
    print("# then: forge script script/SeedSpoilsUniswapV3.s.sol --rpc-url $RPC --broadcast --ledger")
    return 0


if __name__ == "__main__":
    sys.exit(main())
