#!/usr/bin/env python3
"""Plan a v3 pool for an ARBITRARY pair, priced off the LIVE market.

WHY THIS EXISTS, separately from `seed_spoils_uniswap.py` (2026-07-27)
─────────────────────────────────────────────────────────────────────
`seed_spoils_uniswap.py` plans the three launch pools. Its prices are
numbers we CHOOSE — 10 SCRY/OBOL, 50 SCRY/MYRRH — so it is correct for it
to take them as arguments and never look at a market.

The season pools are the opposite case. A season stands up a
**MEMECOIN/SCRY** pool (`FARMING.md` §1b), and that tick is not ours to pick: it is
implied by two markets that already exist, `MEMECOIN/WETH` and
`SCRY/WETH`, and it moves minute to minute. Typing it by hand is how you
open a pool at yesterday's price and hand the difference to the first
arber — the exact failure `LAUNCH-DECISIONS.md` opens the MYRRH/OBOL pool
defensively to avoid.

So: same audited math, different price source. This module imports
`plan_pool`/`render_exports` from `seed_spoils_uniswap` rather than
re-deriving them — a second copy of sqrtPriceX96 math is a second thing
that can be subtly wrong, and only one of them would have a selftest.

WHAT IT DOES NOT DO
───────────────────
It does not broadcast, and it does not decide the dust size. It reads two
pools, derives a cross, plans one pool, and prints the export block a
Solidity seeding script consumes. Re-run it MINUTES before broadcast
(`POOLS.md` §2.2) — a cross computed an hour ago is a typed price again.

USAGE
    python3 seed_pair_uniswap.py \
        --base 0xedAe…022d --base-weth-pool 0xdEc8F541FF… \
        --quote 0xDa2a…C4B0 --quote-weth-pool 0xEA8D…7462 \
        --weth 0x0Bd7…AD73 \
        --quote-budget 130 --tag PONGO_SCRY
"""
import argparse
import json
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from seed_spoils_uniswap import plan_pool, render_exports, _norm  # noqa: E402

KEYS_ENV = "/data/apps/secrets/keys.env"
# The ADVERTISED endpoint. Do not default to it: it answers `eth_chainId` from
# this box and then **403s `eth_call`**, which is the shape that costs an hour
# (`POOLS.md`; the keyed pool exists for exactly this). Kept only as the
# last-resort fallback so the script still runs off-box.
PUBLIC_RPC = "https://rpc.mainnet.chain.robinhood.com"


def default_rpc() -> str:
    """First entry of the keyed pool (`SCRY_RH_RPC_POOL`), else the public one."""
    try:
        for line in Path(KEYS_ENV).read_text(encoding="utf-8").splitlines():
            if line.startswith("SCRY_RH_RPC_POOL="):
                first = line.split("=", 1)[1].strip().strip('"').split(",")[0]
                if first:
                    return first
    except OSError:
        pass
    return PUBLIC_RPC


DEFAULT_RPC = default_rpc()
# slot0() -> (uint160 sqrtPriceX96, int24 tick, ...). We want the first word.
SLOT0_SELECTOR = "0x3850c7bd"
TOKEN0_SELECTOR = "0x0dfe1681"  # token0()


def _rpc(url: str, method: str, params: list, timeout: float = 20.0):
    body = json.dumps({"jsonrpc": "2.0", "id": 1,
                       "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body,
                                 headers={"content-type": "application/json",
                                          "user-agent": "scry-seed-pair/1"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        out = json.load(r)
    if "error" in out:
        raise SystemExit(f"RPC {method} failed: {out['error']}")
    return out["result"]


def _call(url: str, to: str, selector: str) -> str:
    return _rpc(url, "eth_call", [{"to": to, "data": selector}, "latest"])


def pool_price_in_weth(url: str, pool: str, subject: str, weth: str,
                       subject_dec: int, weth_dec: int) -> float:
    """WETH per 1 whole `subject`, read from the pool's own slot0.

    sqrtPriceX96 encodes token1/token0 in RAW units, so the decimal shift is
    applied here and the token ordering is read from the chain rather than
    assumed — getting that backwards silently inverts the cross, and an
    inverted cross is a pool opened at the reciprocal of its real price.
    """
    raw = _call(url, pool, SLOT0_SELECTOR)
    sqrtp = int(raw[2:66], 16)
    if sqrtp == 0:
        raise SystemExit(f"{pool}: slot0 sqrtPriceX96 is 0 — pool not initialised")
    token0 = "0x" + _call(url, pool, TOKEN0_SELECTOR)[-40:]
    # price of token0 denominated in token1, raw units
    price_raw_1_per_0 = (sqrtp / (2 ** 96)) ** 2
    if token0.lower() == subject.lower():
        # token1 is WETH: raw price is WETH-raw per subject-raw
        return price_raw_1_per_0 * (10 ** subject_dec) / (10 ** weth_dec)
    if token0.lower() == weth.lower():
        # token1 is the subject: invert, then shift
        return (1.0 / price_raw_1_per_0) * (10 ** subject_dec) / (10 ** weth_dec)
    raise SystemExit(f"{pool}: token0 {token0} is neither the subject "
                     f"{subject} nor WETH {weth} — wrong pool address?")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--rpc", default=DEFAULT_RPC)
    ap.add_argument("--weth", required=True, help="WETH address on this chain")
    ap.add_argument("--base", required=True, help="the token being priced (the memecoin)")
    ap.add_argument("--base-weth-pool", required=True, help="its existing v3 pool vs WETH")
    ap.add_argument("--base-dec", type=int, default=18)
    ap.add_argument("--quote", required=True, help="the token it is priced IN (SCRY)")
    ap.add_argument("--quote-weth-pool", required=True, help="the quote's v3 pool vs WETH")
    ap.add_argument("--quote-dec", type=int, default=18)
    ap.add_argument("--quote-budget", type=float, required=True,
                    help="how much of the QUOTE token to put in (the dust seed)")
    ap.add_argument("--quote-sym", default="SCRY")
    ap.add_argument("--tag", required=True, help="env prefix, e.g. PONGO_SCRY")
    ap.add_argument("--slip-bps", type=int, default=100)
    ap.add_argument("--max-age-note", action="store_true",
                    help=argparse.SUPPRESS)
    a = ap.parse_args()

    base_weth = pool_price_in_weth(a.rpc, a.base_weth_pool, a.base, a.weth,
                                   a.base_dec, 18)
    quote_weth = pool_price_in_weth(a.rpc, a.quote_weth_pool, a.quote, a.weth,
                                    a.quote_dec, 18)
    if quote_weth <= 0:
        raise SystemExit("quote token prices at 0 against WETH — refusing to plan")
    # QUOTE per 1 BASE — what plan_pool calls price_scry_per_spoil.
    cross = base_weth / quote_weth

    print("# --- LIVE CROSS, read from slot0 just now. RE-RUN MINUTES BEFORE")
    print("#     BROADCAST (POOLS.md 2.2) — this number moves. ---")
    print(f"#   {a.base} vs WETH : {base_weth:.12g} WETH")
    print(f"#   {a.quote} vs WETH : {quote_weth:.12g} WETH")
    print(f"#   implied cross     : {cross:.12g} {a.quote_sym} per base token")
    plan = plan_pool(a.tag, _norm(a.base), _norm(a.quote), cross,
                     a.quote_budget, a.base_dec, a.quote_dec, a.slip_bps,
                     quote_sym=a.quote_sym)
    print(render_exports([plan]))
    print(f"# base side needed: {plan['spoil_budget']:.6f} "
          f"(acquire it yourself; this script moves nothing)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
