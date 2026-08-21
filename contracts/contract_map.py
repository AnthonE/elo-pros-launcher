#!/usr/bin/env python3
"""What each contract in src/ actually does — derived, never transcribed.

`docs/onchain/CONTRACT-MAP.md` is the reasoning: which contracts are
platform-wide, which are per-coin, which are per-title. This is the INVENTORY,
and it is a script rather than a table because a table of 45 rows with live/dead
status is exactly the thing `CLAUDE.md` says not to write down — it rots, and a
stale one reports a deployed contract as unbuilt or a dropped one as pending.

Two sources, and neither is prose:
  - the @title/@notice each contract writes about itself, in src/*.sol
  - deployments.json, the durable record (foundry's broadcast/ is gitignored)

⚠ INSTANCES ARE NOT SOURCES. deployments.json keys a per-coin deployment as
`EloHarvest:OBOL`, so the instance count is legitimately higher than the number
of contracts we wrote. Both are printed; quoting one for the other is how
"nineteen contracts are live" and "fifteen contracts are live" end up in two
docs that are both correct.

    python3 contracts/contract_map.py           # grouped by job
    python3 contracts/contract_map.py --flat    # one line each
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

# The grouping is the stable half — a contract's JOB does not change when it is
# deployed. Anything unlisted lands in "unsorted", which is the prompt to file
# it here rather than to widen a regex.
GROUPS: list[tuple[str, str, tuple[str, ...]]] = [
    ("the game economy", "Gates' coins and the farm stack. Denominated in the "
     "game coins; takes no reserve argument, so the ELO move does not touch it",
     ("SpoilsToken", "EloGranary", "EloGardener", "EloSilo", "EloGarden",
      "EloOrchard", "EloShrine", "EloHarvest")),
    ("the reserve's money", "welded to the reserve's address — these are the "
     "ones the ELO move forces a redeploy of",
     ("EloBank", "EloFeeSplitter", "EloCistern")),
    ("the registers", "commit and read; hold no token at all",
     ("EloVowRegistry", "EloNotary", "EloCovenant", "EloPact")),
    ("the store", "a title's shelf row, its copy sale, and where that money goes",
     ("EloGameTicket", "EloGameTicketFactory", "EloTicketUrlRenderer",
      "EloGameDirectory", "EloLaunchpad", "EloCurve", "EloFeeConverter",
      "EloTill")),
    ("the collection layer", "the founder mint and the things a player owns",
     ("EloSeat", "EloSeatArt", "EloItem", "EloKiln", "EloTrove",
      "EloGacha", "EloSteleEdition", "EloDeed")),
    ("reserved — built, no population", "written and tested; the loop each "
     "serves has nobody on it, so none is broadcast",
     ("EloJobBoard", "EloArbiter", "EloReputation", "EloInsurancePool",
      "EloBurrow", "EloPeculium", "EloPeculiumFactory")),
    ("DEAD — superseded, do not build on", "the Thousand, replaced by the seat "
     "(SENTENCES.md 2026-08-03). EloEidolon is undeployable by construction: "
     "its constructor takes a EloTicket, dropped in the same sentence",
     ("EloEidolon", "EloTicket")),
    ("plumbing", "libraries and interfaces, not contracts",
     ("IERC20", "IEloArbiter", "Math", "ReentrancyGuard", "SafeERC20")),
]


def inventory() -> tuple[dict[str, str], dict[str, str | None], int]:
    dep = json.load(open(HERE / "deployments.json"))["chains"]["4663"]["contracts"]
    addr: dict[str, str | None] = {}
    for k, v in dep.items():
        addr.setdefault(k.split(":")[0], v.get("address"))
    what: dict[str, str] = {}
    for f in sorted((HERE / "src").glob("*.sol")):
        s = f.read_text(encoding="utf-8")
        m = re.search(r"@title\s+(.+)", s) or re.search(r"@notice\s+(.+)", s)
        line = (m.group(1).strip() if m else "")
        # The title repeats the contract name; the job is what follows the dash.
        line = re.split(r"\s+[—-]\s+", line, maxsplit=1)[-1]
        what[f.stem] = line
    return what, addr, len(dep)


def main() -> int:
    what, addr, instances = inventory()
    live = lambda n: "LIVE" if addr.get(n) else "  — "

    if "--flat" in sys.argv:
        for n in sorted(what):
            print(f"  {live(n)}  {n:22} {what[n][:88]}")
        return 0

    filed: set[str] = set()
    for title, why, names in GROUPS:
        present = [n for n in names if n in what]
        if not present:
            continue
        n_live = sum(1 for n in present if addr.get(n))
        print(f"\n== {title}  ({n_live}/{len(present)} deployed)")
        print(f"   {why}")
        for n in present:
            print(f"     {live(n)}  {n:22} {what[n][:80]}")
        filed.update(present)

    rest = sorted(set(what) - filed)
    if rest:
        print("\n== unsorted — file these in GROUPS rather than widening a regex")
        for n in rest:
            print(f"     {live(n)}  {n:22} {what[n][:80]}")

    n_live = sum(1 for n in what if addr.get(n))
    print(f"\n  {len(what)} sources in src/ · {n_live} deployed · "
          f"{instances} INSTANCES on chain 4663")
    print("  (instances > sources because a per-coin deploy is keyed "
          "`EloHarvest:OBOL` — do not quote one for the other)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
