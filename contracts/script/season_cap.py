#!/usr/bin/env python3
"""Gates' season calendar, and the daily mint cap a season budget implies.

WHY THIS EXISTS. `EloGranary` caps a grantee's mint **per UTC day** — that is
the throttle, enforced on chain, and it is not going to change. Gates runs on
**seasons: a new game starts the last Thursday of every month** (operator,
2026-08-22). So the number a person actually decides is a SEASON budget, and
the number the contract wants is a DAILY cap, and the conversion depends on a
calendar nobody should be doing in their head:

    daily cap = season budget / days in that season

Season lengths are not equal and not close to equal. Last-Thursday-to-last-
Thursday runs 28 or 35 days depending on how the months fall, so a cap sized
off "about a month" is up to 25% wrong in whichever direction nobody checked.
That is the whole reason this is a module.

⚠ IT IS A CAP, NOT A SCHEDULE. The granary CLAMPS: a grantee over today's
budget receives the remainder and is told how much it got, and nothing
reverts. So a cap set too low does not fail loudly — a player extracts and is
quietly paid short at the day boundary. Size it as a CEILING you expect play
to sit under, never as the emission you are aiming for.

⚠ THE SEASON WORD IS OVERLOADED AND THE COLLISION IS REAL. `FARMING.md` uses
"season" for the Orchard's posted, finite LP-farming rounds — a different
thing, on a different clock, paying different coins. This module is Gates'
GAME season. Do not let one page use both without saying which, the way
`hive` once meant the chat and the collection at once.

    python3 season_cap.py                          # the calendar
    python3 season_cap.py --budget 50000000        # + the daily cap it implies
    python3 season_cap.py --budget 50000000 --wei  # + the granary's argument

stdlib only, no network, no writes.
"""
from __future__ import annotations

import argparse
import calendar
import datetime
import sys

WEEKDAY = calendar.THURSDAY
WEEKDAY_NAME = "Thursday"


def season_start(year: int, month: int) -> datetime.date:
    """The last <WEEKDAY> of a month — the day that month's game starts."""
    day = max(w[WEEKDAY] for w in calendar.monthcalendar(year, month) if w[WEEKDAY])
    return datetime.date(year, month, day)


def _next_month(year: int, month: int) -> tuple[int, int]:
    return (year + 1, 1) if month == 12 else (year, month + 1)


def calendar_from(today: datetime.date, count: int = 6) -> list[dict]:
    """The next `count` seasons, each with the length that decides its cap.

    A season runs start-to-next-start, so its length is a fact about two
    months rather than about one, and it is the denominator of every cap.
    """
    y, m = today.year, today.month
    if season_start(y, m) < today:          # this month's has already begun
        y, m = _next_month(y, m)
    out = []
    for _ in range(count):
        start = season_start(y, m)
        ny, nm = _next_month(y, m)
        nxt = season_start(ny, nm)
        out.append({
            "start": start,
            "ends": nxt,
            "days": (nxt - start).days,
            "in_days": (start - today).days,
        })
        y, m = ny, nm
    return out


def daily_cap(budget: float, days: int) -> float:
    if days <= 0:
        raise ValueError("a season cannot be zero days long")
    return budget / days


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--budget", type=float,
                    help="whole coins a season may mint AT MOST. No default on "
                         "purpose: there is no operator sentence naming one, and a "
                         "cap that lands on its recommendation by accident is "
                         "indistinguishable afterwards from one somebody chose")
    ap.add_argument("--count", type=int, default=6)
    ap.add_argument("--today", help="YYYY-MM-DD; defaults to the system date")
    ap.add_argument("--pool-junk", type=float,
                    help="whole coins sitting in the pool. Adds the drain rate a season's "
                         "emission implies: (emitted and sold) / (in the pool). The RESERVE "
                         "side does not enter it — depth in the coin is what decides how "
                         "fast a faucet empties a pool, and it is the free half")
    ap.add_argument("--wei", action="store_true",
                    help="also print GRANT_DAILY_CAP in wei, the argument DeployGranary takes")
    args = ap.parse_args()

    today = (datetime.date.fromisoformat(args.today) if args.today
             else datetime.date.today())
    seasons = calendar_from(today, args.count)

    print(f"today {today} ({today.strftime('%A')}) — a new game starts the "
          f"last {WEEKDAY_NAME} of every month\n")
    head = f"  {'starts':<12} {'day':<9} {'in':>5}  {'runs':>5}"
    if args.budget:
        head += f"  {'daily cap':>16}"
    print(head)
    for s in seasons:
        line = (f"  {s['start'].isoformat():<12} {s['start'].strftime('%A'):<9} "
                f"{s['in_days']:>4}d {s['days']:>4}d")
        if args.budget:
            line += f"  {daily_cap(args.budget, s['days']):>16,.0f}"
        print(line)

    lens = sorted({s["days"] for s in seasons})
    if len(lens) > 1:
        print(f"\n  ⚠ season lengths differ ({', '.join(str(x) for x in lens)} days) — "
              f"a cap sized off 'about a month' is up to "
              f"{100 * (max(lens) - min(lens)) / min(lens):.0f}% wrong. "
              f"Re-run this at every season boundary.")
    if args.budget and args.pool_junk:
        # Worst case on purpose: every coin emitted this season is sold. Real
        # play never reaches it — Gates loses scrap in-world on death, and
        # players hold and spend — so treat this as the ceiling on how fast
        # the pool can empty, never as a forecast.
        drain = args.budget / args.pool_junk
        print(f"\n  worst-case drain: {100 * drain:.2f}% of the pool's coin side per season")
        print(f"  ({100 * drain * 12:.1f}% a year if every coin emitted is sold, which it is not)")
        if drain > 0.10:
            print("  ⚠ over 10% a season — the pool empties in under three years of play.")
            print("    The lever is the OPENING PRICE, not the budget: a cheaper coin puts")
            print("    more of it in the pool for the same reserve, and it costs nothing.")
    if args.budget:
        nxt = seasons[0]
        cap = daily_cap(args.budget, nxt["days"])
        print(f"\n  next season {nxt['start']} runs {nxt['days']} days")
        print(f"  GRANT_DAILY_CAP = {cap:,.0f} whole coins/day for a "
              f"{args.budget:,.0f} season ceiling")
        if args.wei:
            print(f"  GRANT_DAILY_CAP={int(cap * 10**18)}")
        print("\n  ⚠ the granary CLAMPS rather than reverting: a day that runs over pays "
              "\n    the remainder and reports it. This is a ceiling, never a target.")
    else:
        print("\n  (pass --budget <whole coins> for the daily cap it implies)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
