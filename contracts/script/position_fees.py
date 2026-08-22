#!/usr/bin/env python3
"""What a v3 position has actually earned, read the way that is not a lie.

THE TRAP THIS MODULE EXISTS FOR. The obvious read is
`NonfungiblePositionManager.positions(tokenId).tokensOwed0/1`. It is wrong
almost all of the time, and wrong in the flavour this repo keeps paying for: it
is only WRITTEN when the position is touched — mint, increase, decrease,
collect — so between touches it reports what was owed at the last touch, which
for a freshly minted position is **zero**. A pool can trade for months and this
field still says nothing was earned. Nothing errors. The card says the pool
made no money.

WHAT IS TRUE INSTEAD. `eth_call` the **collect** you would broadcast, with both
maxes at type(uint128).max and the position's owner as `from`. The manager
pokes the pool internally before computing, so the simulated return values are
the real collectable amounts — and because it is a call rather than a
transaction, nothing moves. Both figures are printed below, side by side,
precisely so the gap between them is visible rather than something to trust one
side of.

    python3 position_fees.py --position 1234 --owner 0x…
    python3 position_fees.py --position 1234 --owner 0x… --json

stdlib only, no writes, no key. It reads; `CompoundPosition.s.sol` acts.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

# The repo keeps its keccak in meter/ and every other caller shells into that
# directory for it (`launch_elo.sh`'s `sel()` does exactly this). Same import,
# resolved by path rather than by cwd so this runs from anywhere.
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "meter"))
from keccak import keccak256  # noqa: E402

RPC = os.getenv("RPC", "https://rpc.mainnet.chain.robinhood.com")
NPM = os.getenv("V3_NPM", "0x73991a25C818Bf1f1128dEAaB1492D45638DE0D3")
UINT128_MAX = (1 << 128) - 1


def sel(sig: str) -> str:
    return keccak256(sig.encode()).hex()[:8]


def _rpc(method: str, params: list) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    out = subprocess.run(
        ["curl", "-s", "-m", "25", "-X", "POST", RPC,
         "-H", "content-type: application/json", "-d", body],
        capture_output=True, text=True).stdout
    try:
        return json.loads(out)
    except ValueError:
        return {"error": {"message": f"unparseable response: {out[:120]!r}"}}


def _call(to: str, data: str, frm: str | None = None) -> str | None:
    tx = {"to": to, "data": "0x" + data}
    if frm:
        tx["from"] = frm
    r = _rpc("eth_call", [tx, "latest"])
    res = r.get("result")
    # A failed read is None and never a helpful zero — sizing anything off a
    # zero that meant "we could not look" is the whole reason this is explicit.
    return res if isinstance(res, str) and res not in ("", "0x") else None


def _words(hexstr: str) -> list[int]:
    h = hexstr[2:]
    return [int(h[i:i + 64], 16) for i in range(0, len(h) - 63, 64)]


def _addr(word: int) -> str:
    return "0x" + f"{word:040x}"


def _i24(word: int) -> int:
    return word - (1 << 256) if word >= (1 << 255) else word


def read_position(token_id: int) -> dict | None:
    res = _call(NPM, sel("positions(uint256)") + f"{token_id:064x}")
    if not res:
        return None
    w = _words(res)
    if len(w) < 12:
        return None
    return {
        "token0": _addr(w[2]), "token1": _addr(w[3]), "fee_ppm": w[4],
        "tick_lower": _i24(w[5]), "tick_upper": _i24(w[6]),
        "liquidity": w[7],
        # ⚠ STALE. Printed to be compared against the simulation, never used.
        "tokens_owed0_stale": w[10], "tokens_owed1_stale": w[11],
    }


def simulate_collect(token_id: int, owner: str) -> tuple[int, int] | None:
    """The honest read: the collect you would send, as a call."""
    data = (sel("collect((uint256,address,uint128,uint128))")
            + f"{token_id:064x}"
            + owner.lower().replace("0x", "").rjust(64, "0")
            + f"{UINT128_MAX:064x}" + f"{UINT128_MAX:064x}")
    res = _call(NPM, data, frm=owner)
    if not res:
        return None
    w = _words(res)
    return (w[0], w[1]) if len(w) >= 2 else None


def owner_of(token_id: int) -> str | None:
    res = _call(NPM, sel("ownerOf(uint256)") + f"{token_id:064x}")
    return _addr(_words(res)[0]) if res else None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--position", type=int, required=True, help="the LP position's tokenId")
    ap.add_argument("--owner", help="defaults to whatever ownerOf() reports")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    pos = read_position(args.position)
    if pos is None:
        print(f"position {args.position}: unreadable — no such token, or the RPC "
              f"did not answer. NOT a zero.", file=sys.stderr)
        return 1

    owner = args.owner or owner_of(args.position)
    if not owner:
        print("could not resolve the owner, and collect must be simulated FROM it",
              file=sys.stderr)
        return 1

    got = simulate_collect(args.position, owner)
    d0, d1 = 10 ** 18, 10 ** 18   # both sides of a game pool are 18-decimal

    if args.json:
        print(json.dumps({
            "position": args.position, "owner": owner, **pos,
            "collectable0": got[0] if got else None,
            "collectable1": got[1] if got else None,
            "collectable_read": "eth_call collect (pokes the pool)" if got else "FAILED",
        }, indent=1))
        return 0 if got else 1

    print(f"\nposition {args.position}  owner {owner}")
    print(f"  pair        {pos['token0']} / {pos['token1']}  fee {pos['fee_ppm']/10_000:g}%")
    print(f"  ticks       {pos['tick_lower']} .. {pos['tick_upper']}")
    print(f"  liquidity   {pos['liquidity']:,}")
    print(f"\n  tokensOwed as the manager stores it  "
          f"{pos['tokens_owed0_stale']/d0:,.6f} / {pos['tokens_owed1_stale']/d1:,.6f}")
    if got is None:
        print("  collectable  UNREADABLE — the simulation did not answer. This is "
              "NOT zero,\n               and nothing should be sized off it.")
        return 1
    print(f"  actually collectable                 {got[0]/d0:,.6f} / {got[1]/d1:,.6f}")
    if (pos["tokens_owed0_stale"], pos["tokens_owed1_stale"]) != got:
        print("\n  ⚠ the two disagree, which is the ordinary case rather than a fault:")
        print("    tokensOwed was last written when the position was touched, and")
        print("    everything earned since is invisible in it. The lower pair is the")
        print("    one a naive card would print, and it is the one that is wrong.")
    if got == (0, 0):
        print("\n  nothing to collect — the position is genuinely empty of fees.")
    else:
        print("\n  put it back:  POSITION_ID=%d forge script script/CompoundPosition.s.sol \\"
              % args.position)
        print("                  --rpc-url $RPC --broadcast")
    return 0


if __name__ == "__main__":
    sys.exit(main())
