#!/usr/bin/env python3
"""preflight.py — the OFF-CHAIN gate before any launch broadcast.

`deploy_town.sh` gates hard on the contracts (upstream forge, 365/365, the
four live-fork mandates). Nothing gated the meter side, and the meter side is
where the launch actually goes wrong: the on-chain half can be perfect while
the ledger that feeds it is denominated wrong, the drop is sized against a
stale snapshot, or the balancing gauge is about to be swamped by genesis float.

Every check here is a thing that was ACTUALLY WRONG in this repo at some point
in the week before launch. None of them is hypothetical.

  python3 preflight.py            # report, exit 1 on any FAIL
  python3 preflight.py --json     # machine-readable

No network, no keys. Reads the committed source and the local snapshot.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
RESULTS = []


def check(name, ok, detail, fix=""):
    RESULTS.append({"check": name, "ok": bool(ok), "detail": detail, "fix": fix})
    return ok


def strip_comments(text):
    """Drop // and /* */ comments so a source check reads CODE, not prose.
    Naive about strings containing "//" — none of the checks below care."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


_JSON_CACHE: dict = {}


def _json(path):
    """Parse a JSON file once. Returns {} for anything unreadable rather than
    raising — the callers use it to CLASSIFY files, and a stray non-JSON file in
    snapshots/ must not take a launch gate down (the same shape as the anchor's
    stray-.json defect, meter/test_anchor_resilience.py)."""
    key = str(path)
    if key not in _JSON_CACHE:
        try:
            _JSON_CACHE[key] = json.loads(Path(path).read_text())
        except Exception:  # noqa: BLE001
            _JSON_CACHE[key] = {}
    return _JSON_CACHE[key] if isinstance(_JSON_CACHE[key], dict) else {}


def src(rel):
    """Read a repo file, tolerating the docs/ reorg.

    A markdown doc named at the repo root may now live under docs/ (and vice
    versa). A gate that reads "" because a file MOVED does not fail loudly —
    it fails for the wrong reason, or worse passes a `X not in src(...)` check
    on an empty string. Try both layouts before believing a file is absent.

    The docs/ fallback is for DOCS. It used to apply to any path, so
    `src("meter/server.py")` would happily read `docs/server.py` — a gate
    silently reading a different file than the one it names is worse than a
    gate that fails, because its output still says `meter/server.py`.
    """
    if (REPO / rel).is_file():
        return (REPO / rel).read_text()
    # A DOC may be anywhere under docs/, so the fallback WALKS rather than
    # naming folders. Naming them was wrong twice: `docs/<name>` alone missed
    # `docs/archive/`, and on 2026-08-09 docs/ was foldered by subject
    # (start/ store/ money/ client/ items/ onchain/ builders/ ops/ launch/
    # agent-town/) and eight gates went red at once for a reorg rather than a
    # fault. A walk cannot go stale when a folder is added.
    if Path(rel).suffix == ".md":
        for found in (REPO / "docs").rglob(Path(rel).name):
            if found.is_file():
                return found.read_text()
    return ""


def src_required(rel, why):
    """`src()` for a gate whose PASS depends on the file being there.

    `src()` returns "" for a missing file, and `"X" not in ""` is True — so a
    renamed, moved or deleted file turns a NEGATIVE check into a silent PASS.
    `src()`'s own docstring warned about exactly this and the warning was not
    applied to the only negative check in the file (`no free signed reads`,
    which gates whether a signed read can be given away for a balance). Every
    other gate here was re-read and fails safe; this is the one that did not.

    Returns "" and registers its own FAIL if the file is absent, so a caller
    cannot accidentally proceed on an empty string.
    """
    text = src(rel)
    if not text:
        check(f"{rel} is readable", False, f"{rel} is missing or empty",
              f"this gate cannot run without it — {why}")
        return ""
    return text


def doc_exists(rel):
    return bool(src(rel))


def grab(rel, pattern, cast=int):
    m = re.search(pattern, src(rel), re.M)
    return cast(m.group(1)) if m else None


# ── 1. the denomination + sybil laws, via the test that owns them ────────────
def gate_tokenomics():
    p = subprocess.run([sys.executable, str(REPO / "meter/test_tokenomics.py")],
                       capture_output=True, text=True)
    tail = (p.stdout or p.stderr).strip().splitlines()
    check("tokenomics laws (meter/test_tokenomics.py)", p.returncode == 0,
          tail[-1] if tail else "no output",
          "run it directly to see which law broke; do NOT edit the test green")


def gate_reliquary_odds():
    """A gacha whose odds do not sum is a gacha that lies. Read the COMMITTED
    default table out of the source rather than importing (reliquary.py pulls
    fastapi, and preflight must run on a bare box)."""
    bps = [int(x) for x in re.findall(r'"bps":\s*(\d+)', src("meter/reliquary.py"))]
    total = sum(bps)
    check("reliquary odds sum to 10000 bps", bool(bps) and total == 10000,
          f"{len(bps)} tiers summing to {total}" if bps else "no bps entries found",
          "fix _DEFAULT_TABLE in meter/reliquary.py so the posted odds sum")


# ── 2. the drop is sized against a FRESH snapshot ────────────────────────────
MAX_SNAPSHOT_AGE_H = 24

# The drops repeat quarterly, funded from collected LP fees rather than a mint
# (operator, 2026-07-27), and the Garden must out-earn any one of them: a drop
# may mint at most this share of what the farm emits over one period. Both
# numbers are policy and live here so a reader finds them in one place; the
# farm's RATE is read from the deploy script, never typed.
DROP_PERIOD_DAYS = 91.25
FARM_BEATS_DROP_SHARE = 0.25


def gate_snapshot():
    snaps = sorted((REPO / "snapshots").glob("scry-holders.*.json"))
    if not snaps:
        return check("holder snapshot exists", False, "no snapshots/scry-holders.*.json",
                     "python3 meter/holder_snapshot.py snapshot --out snapshots/…")
    # Pick the newest by the snapshot's OWN `taken_at`, then by filename date —
    # never by mtime. `git clone` and every CI checkout stamp every file with
    # the SAME mtime, so `max(key=st_mtime)` is a tie among all of them and the
    # winner is arbitrary. Found by CI on its first run: three snapshots in the
    # tree, and a fresh checkout graded the **2026-07-22** one, which predates
    # the EIP-7702 reclassification — so a launch gate on any clean machine was
    # measuring a snapshot nobody chose. The permissive direction is the
    # dangerous one: it could equally have picked a stale file that PASSES.
    #
    # Same lesson as the freshness check below, one line up: mtime is not a
    # clock, and a file's content is the only thing that travels with it.
    def _stamp(p):
        try:
            t = str(json.loads(p.read_text()).get("taken_at") or "")
            if t:
                return (1, t)
        except Exception:                        # noqa: BLE001 — unreadable/torn
            pass
        m = re.search(r"(\d{4}-\d{2}-\d{2})", p.name)
        return (0, m.group(1) if m else "")

    newest = max(snaps, key=_stamp)
    snap = json.loads(newest.read_text())
    # Age from the snapshot's OWN `taken_at`, not from st_mtime — a `touch`, a
    # `git checkout` or a fresh clone resets mtime and makes a stale snapshot
    # read as minutes old. mtime is the fallback, and the detail says which one
    # answered so a reader is never guessing which clock this ran on.
    stamped = str(snap.get("taken_at") or "")
    clock = "taken_at"
    try:
        age_h = (time.time() - time.mktime(time.strptime(
            stamped, "%Y-%m-%dT%H:%M:%SZ")) + time.timezone) / 3600
    except (ValueError, TypeError):
        age_h, clock = (time.time() - newest.stat().st_mtime) / 3600, "file mtime"
    check("snapshot carries its own timestamp", clock == "taken_at",
          f"taken_at={stamped!r}" if stamped else "no `taken_at` in the snapshot",
          "freshness measured from mtime is defeated by a touch or a fresh clone")
    # SCOPED TO A DROP ACTUALLY ARMING, 2026-07-27 — same shape as the §0.4
    # claim-isolation and season-pot gates, and for a reason worth stating.
    # Unconditionally red, this check forced a snapshot re-take that NEITHER
    # drop uses, purely to unblock a pools broadcast that does not read it:
    # drop one is a STEALTH drop already pinned at a past block with its plan
    # published (`founders-bag.2026-07-25.json`, block 19231505) — a fresh
    # snapshot cannot change it — and drop two is ANNOUNCED a week later and
    # takes its own snapshot then, by which time today's would be stale again.
    # A ritual that produces a green light nobody is relying on is worse than
    # no light. The moment a drop arms, this is hard again.
    arming = _drop_is_arming()
    fresh_ok = age_h <= MAX_SNAPSHOT_AGE_H
    check("snapshot is fresh", fresh_ok or not arming,
          (f"{newest.name} is {age_h:.1f}h old by {clock} "
           f"({snap.get('n_holders')} holders, block {snap.get('block')})")
          + ("" if fresh_ok else
             (f" — and a drop IS arming ({arming})" if arming else
              " — and NOTHING is arming, so no drop reads it: drop one is "
              "pinned at its own past block, drop two snapshots at announcement")),
          f"holder count moved 155->258 in two days once; re-snapshot within "
          f"{MAX_SNAPSHOT_AGE_H}h of the drop that will USE it")
    # the EIP-7702 fix must have run, or ~26% of supply is silently excluded
    d = snap.get("delegation") or {}
    check("snapshot ran the EIP-7702 reclassification", bool(d),
          (f"checked {d.get('checked')} flagged, recovered {d.get('delegated_eoas')} "
           f"delegated EOAs, {d.get('real_contracts')} real contracts")
          if d else "no `delegation` block — snapshot predates the 7702 fix",
          "re-take the snapshot; without this, smart-account wallets are cut "
          "from their own airdrop")
    # `verified: true` is a claim the same JSON supplies about itself. Check the
    # EVIDENCE beside it instead: §0.2 re-reads every balance by `eth_call
    # balanceOf` at the pinned block, so a verified snapshot must show one
    # re-read PER HOLDER and an empty error list. A hand-set flag now has to be
    # accompanied by a consistent count to survive.
    v = snap.get("verification") or {}
    n = snap.get("n_holders")
    repinned, errs = v.get("repinned_all"), v.get("errors")
    check("snapshot verified against the chain",
          snap.get("verified") is True and repinned == n and errs == [],
          f"verified={snap.get('verified')} repinned_all={repinned}/{n} "
          f"errors={len(errs) if isinstance(errs, list) else errs} "
          f"differed_from_indexer={v.get('differed_from_indexer')}",
          "re-take it with repin on; a shaky snapshot ships nothing")
    # The second block, same shape of check as the 7702 one above: a snapshot
    # without it is not wrong, it is just blind in a way nothing else reports.
    # A balance at one instant cannot tell a holder from a wallet caught
    # mid-round-trip — measured 2026-07-26, one bot back-runs buys on the SCRY
    # pool and holds a fixed unit against a ~20-minute timer, so it looks like a
    # 3.89M-token holder at ~6% of blocks for free. `complete` is checked, not
    # just presence: a partial second read is the state that quietly re-admits
    # exactly the wallets it exists to catch, which is why claim_plan refuses it.
    #
    # Scoped to an arming drop for the same reason as the freshness check above
    # — with one extra fact worth writing down, because it reads as a hole and
    # is not one. **Drop one has no persistence read and does not need it.**
    # Persistence defends an ANNOUNCED drop, where a wallet can position for a
    # snapshot it knows is coming. Drop one is stealth: taken with no
    # announcement at a block that had already passed, so there was nothing to
    # round-trip into. That is the whole security argument for taking it first
    # (`SENTENCES.md` 2026-07-25). Drop two is the announced one, and drop two
    # is exactly when this check has to bite.
    pr = snap.get("persistence") or {}
    persist_ok = bool(pr) and pr.get("complete") is True and pr.get("read") == pr.get("of")
    check("snapshot read balances at a SECOND, earlier block",
          persist_ok or not arming,
          (f"{pr.get('read')}/{pr.get('of')} read at block {pr.get('prior_block')} "
           f"(~{(pr.get('approx_seconds_back') or 0)//3600}h back), "
           f"complete={pr.get('complete')}")
          if pr else
          ("no `persistence` block — one instant cannot show what a wallet KEPT, "
           f"so a round-tripper qualifies on luck (a drop IS arming: {arming})"
           if arming else
           "no `persistence` block, and nothing is arming — drop one is stealth "
           "so there was nothing to round-trip into; drop two is announced and "
           "must be snapshotted with it"),
          "re-take the snapshot (persistence is on by default; "
          "--persist-blocks 0 turns it off)")
    return snap


# ── 3. the drop vs the pool float it will land in ────────────────────────────
# Tolerate markdown emphasis around the cell: a doc going **bold** must never
# disarm a launch gate. It did on 2026-07-26 — the 60/20 revision bolded the
# game-token cell, the pattern stopped matching, and the drop-share check below
# silently VANISHED from the run (17 checks, not 18) instead of going red.
# A check that disappears reads as a shorter list, which is the worse failure.
# Anchored to the OBOL/SCRY row's THIRD cell (`[^|]` cannot cross a pipe, so
# the SCRY column is consumed and never read), first number in it, and the cell
# must still say OBOL — emphasis anywhere is ignored.
POOL_FLOAT_RE = re.compile(
    r"\|\s*OBOL/\SCRY\s*\|[^|]+\|[^|\d]*([\d,]+)[^|]*OBOL[^|]*\|")
# The same read for MYRRH. It had NO gate at all until 2026-07-26, and that is
# precisely where the damage landed: the 60/20 revision cut MYRRH's pool side
# 800,000 -> 400,000 while the drop's MYRRH did not move, taking it from 10.3%
# to 20.6% of float and a full dump from -17.8% to -31.3%. Every check in this
# file passed throughout. A gate that watches one of two coins is a gate that
# reports on the coin that did not break.
POOL_FLOAT_MYRRH_RE = re.compile(
    r"\|\s*MYRRH/\SCRY\s*\|[^|]+\|[^|\d]*([\d,]+)[^|]*MYRRH[^|]*\|")


def gate_published_plans_reproduce():
    """A published drop plan is a public commitment, and the commitment is to
    the BYTES. Rebuild every plan on disk from the params it pins on itself.

    This gate exists because the posted rebuild command depended on live
    DEFAULTS, so halving `myrrh_ratio` silently changed what drop one rebuilt to
    (74,022 -> 39,041 MYRRH) with nothing to catch it. Verifying against the
    plan's OWN params is what makes a published drop reproducible forever.
    """
    sys.path.insert(0, str(REPO))
    try:
        from meter import claim_plan as cp
    except Exception as e:  # noqa: BLE001
        return check("published plans reproduce", False, f"{e}", "")
    plans = [p for p in sorted((REPO / "snapshots").glob("*.json"))
             if isinstance(_json(p), dict) and "params" in _json(p)
             and "allocations" in _json(p)]
    if not plans:
        return check("every published drop plan still reproduces", True,
                     "no published plan with pinned params on disk yet — "
                     "this gate arms with the first one", "")
    bad = []
    for p in plans:
        plan = _json(p)
        snaps = [s for s in (REPO / "snapshots").glob("scry-holders.*.json")
                 if str(_json(s).get("block")) == str(plan.get("snapshot", {}).get("block"))]
        if not snaps:
            bad.append(f"{p.name}: no snapshot on disk at its block")
            continue
        label = str(plan.get("drop_id", "launch")).split("-")[0] or "launch"
        try:
            # Exclusions are part of the commitment and do NOT live in `params`,
            # so read them from the artifact for the same reason `params` is:
            # a published plan reproduces from ITSELF.
            rebuilt = cp.build_launch_plan(
                _json(snaps[0]), plan["params"], label=label, pinned=True,
                exclude=list((plan.get("skipped") or {}).get("excluded") or []))
        except Exception as e:  # noqa: BLE001
            bad.append(f"{p.name}: {e}")
            continue
        if rebuilt != plan:
            bad.append(f"{p.name}: does NOT reproduce")
    check("every published drop plan still reproduces", not bad,
          f"{len(plans)} plan(s) rebuild byte-for-byte from their own params"
          if not bad else "; ".join(bad),
          "a published plan's terms are fixed — if a knob changed, the NEW drop "
          "takes the new knob and the published one keeps its pinned params. "
          "Never edit a published plan green.")


# The seam for the drop being PLANNED. Absent, this gate estimates the next drop
# from `claim_plan.DEFAULTS` on the newest snapshot on disk — which is a PROXY,
# not a plan, and the proxy understates in two directions at once: those params
# are not the ones drop two will use, and that snapshot is drop ONE's block, so
# the field is narrower than the one drop two will actually be taken over.
# Present, the gate grades the real thing. Shape: the params dict, or {"params": …}.
NEXT_DROP_PARAMS = "snapshots/next-drop.params.json"


def _next_drop_params():
    """(params, basis) for the drop being planned — pinned if it exists, else None."""
    p = REPO / NEXT_DROP_PARAMS
    if not p.is_file():
        return None, ""
    d = _json(p)
    params = d.get("params") if isinstance(d.get("params"), dict) else d
    return (params or None), (d.get("note") or NEXT_DROP_PARAMS)


def _drop_is_arming():
    """Is a drop actually being armed right now? Same signal `gate_drop_claim_
    isolation` uses, and for the same reason: `DeployClaim` runs AFTER the launch
    phases, so a drop-shaped complaint must not red the run that seeds the pools.
    Returns a reason string, or "" when nothing is armed."""
    armed = [k for k in ("SCRY_CLAIM_OBOL", "SCRY_CLAIM_MYRRH", "SCRY_CLAIM_SCRY")
             if os.getenv(k, "").strip()]
    if armed:
        return f"{', '.join(armed)} set in the environment"
    dep = REPO / "contracts" / "deployments.json"
    if dep.is_file():
        try:
            claims = json.loads(dep.read_text()).get(
                "chains", {}).get("4663", {}).get("claims", {}) or {}
        except Exception:  # noqa: BLE001
            claims = {}
        if claims:
            return f"{len(claims)} drop(s) have claim contracts in deployments.json"
    return ""


def gate_drop(snap):
    if not snap:
        return
    sys.path.insert(0, str(REPO))
    try:
        from meter import claim_plan as cp
    except Exception as e:  # noqa: BLE001
        return check("drop plan computes", False, f"{e}", "")

    params, basis = _next_drop_params()
    # The estimate must model what will actually be BUILT, and what is actually
    # built excludes the house (`gate_house_never_claims`). Estimating without
    # the exclusion overstates every share below by whatever the operator holds
    # — which on this snapshot is 18% of the drop's MYRRH from one wallet.
    plan = cp.build_launch_plan(
        snap, params, exclude=cp.excluded_wallets(str(REPO / "snapshots")))
    t = plan["totals"]

    # Say WHICH drop is being graded, out loud, because the answer is usually
    # "not the one you think." Its own named check, so it can never vanish the
    # way the OBOL float check did when a doc went bold — a run that silently
    # loses a line reads as a shorter list, not as a failure.
    published_blocks = {str(_json(p).get("snapshot", {}).get("block"))
                        for p in (REPO / "snapshots").glob("*.json")
                        if "params" in _json(p) and "allocations" in _json(p)}
    proxy_block = str(snap.get("block")) in published_blocks
    why = " and ".join(filter(None, [
        f"no pinned params (using claim_plan.DEFAULTS, bar "
        f"{cp.DEFAULTS['base_threshold_scry']:,} SCRY)" if not params else "",
        f"snapshot block {snap.get('block')} is a PUBLISHED drop's block, so this "
        f"field ({t['base_holders']} recipients) is not the one the next drop is "
        f"taken over" if proxy_block else ""]))
    arming = _drop_is_arming()
    check("the drop being graded is the real one, not a proxy",
          not why or not arming,
          (f"pinned params from {basis}, block {snap.get('block')}, "
           f"{t['base_holders']} recipients" if not why else
           f"PROXY, and a drop IS arming ({arming}) — {why}" if arming else
           f"PROXY, and nothing is arming yet — {why}. The two share checks below "
           f"are an ESTIMATE; they arm for real when {NEXT_DROP_PARAMS} exists"),
          f"take the real snapshot and pin the real terms in {NEXT_DROP_PARAMS} "
          f"(the params dict, or {{\"params\": …}}), then re-run. Measured "
          f"2026-07-26, so the estimate's direction is known rather than assumed: "
          f"the BAR barely moves it (1,000,000 -> 1 SCRY is 19.5% -> 21.4%) and "
          f"the field would have to grow ~6x to break the MYRRH gate")

    ld = src("LAUNCH-DECISIONS.md")
    m = POOL_FLOAT_RE.search(ld)
    float_obol = float(m.group(1).replace(",", "")) if m else None
    check("pool float is readable from LAUNCH-DECISIONS.md", float_obol is not None,
          f"OBOL float = {float_obol:,.0f}" if float_obol else "could not parse the pool table",
          "keep the pool table in the decision doc parseable — code reads it")
    if float_obol:
        share = t["obol"] / float_obol
        check("the drop is a sane share of the OBOL float", share <= 0.35,
              f"drop {t['obol']:,} OBOL = {share:.1%} of the {float_obol:,.0f} float "
              f"({t['base_holders']} recipients)",
              "do NOT multiply the drop by the 10x (LAUNCH-DECISIONS.md); if it "
              "is still too big, raise `bonus_ratio` — the PROPORTIONAL knob — "
              "and never widen the pool to fit it. This line used to say 'lower "
              "bonus_cap_obol', and that was the wrong knob: a per-wallet cap "
              "pays a holder who already spreads across addresses (measured at "
              "3.73x on 2026-07-30) and the cap was retired for it")

    # MYRRH, the coin this gate was blind to until 2026-07-26. Both drops mint
    # into the SAME pool, so the published drop one counts against the float
    # too — checking only the drop being planned would understate the pressure
    # by whatever has already been committed.
    mm = POOL_FLOAT_MYRRH_RE.search(ld)
    float_myrrh = float(mm.group(1).replace(",", "")) if mm else None
    check("MYRRH pool float is readable too", float_myrrh is not None,
          f"MYRRH float = {float_myrrh:,.0f}" if float_myrrh
          else "could not parse the MYRRH row of the pool table",
          "keep the pool table in the decision doc parseable — code reads it")
    if float_myrrh:
        # Deduped BY DROP_ID, not summed per file. Drop one is re-planned
        # immediately before rollout, so a superseded artifact sitting beside
        # its replacement is the expected state, not an odd one — and summing
        # both would count the same drop twice and red the gate for a reason
        # that does not exist. Take the LARGEST per drop: if two files disagree,
        # the conservative reading is the one that pressures the pool more.
        by_drop: dict[str, int] = {}
        for p in sorted((REPO / "snapshots").glob("*.json")):
            d = _json(p)
            if "params" in d and "allocations" in d:
                did = str(d.get("drop_id") or p.name)
                by_drop[did] = max(by_drop.get(did, 0),
                                   int(d.get("totals", {}).get("myrrh", 0)))
        published = sum(by_drop.values())
        total = t["myrrh"] + published
        share_m = total / float_myrrh
        # Report the TRIGGER, not just the verdict. A binary gate on an estimate
        # tells a reader nothing about how much the estimate can move before it
        # matters, so every re-run is an act of faith. The headroom converts it
        # into one number to watch: recipient count. Measured 2026-07-26 on the
        # 07-25 snapshot — MYRRH binds at ~1,100 more recipients, OBOL at ~3,000,
        # so MYRRH is genuinely the tight side and the field would have to grow
        # ~6x to break it. The BAR is nearly irrelevant by comparison: 1,000,000
        # -> 1 SCRY moves this 19.5% -> 21.4%, because 181 of 242 non-contract
        # holders already clear 10k.
        room = 0.35 * float_myrrh - total
        # Bracket the count rather than pick a point estimate. What a newcomer
        # costs depends entirely on how big they are, and the two ends are far
        # apart: a holder arriving at the bar pays the flat 25 + the 1-in-4
        # bonus's expected 25, while the MEDIAN of today's field is 126 because
        # the existing holders skew rich. Quoting the MEAN (253) would be the
        # worst of the three — whales already in the field drag it up and they
        # do not arrive twice, so it understates headroom by ~5x.
        p = {**cp.DEFAULTS, **(params or {})}
        # What one more recipient costs in MYRRH. Since 2026-07-27 the honest
        # answer is usually ZERO: every broad MYRRH term was retired and the
        # whole budget is a FIXED ticket pot, so a wider snapshot pays the same
        # total to more people rather than minting more. The recipient-headroom
        # bracket below only means something while a per-holder term exists —
        # computing one against a fixed pot would invent a limit that is not
        # there, and dividing by the retired `myrrh_ratio` used to crash.
        ratio = p.get("myrrh_ratio") or 0
        odds = p.get("treasure_bonus_odds") or 0
        cheap = (p["treasure_myrrh"]
                 + (p["base_threshold_scry"] // ratio if ratio else 0)
                 + (p["treasure_bonus_myrrh"] // odds if odds else 0))
        if cheap:
            vals = sorted(a["myrrh"] for a in plan["allocations"].values())
            dear = vals[len(vals) // 2] if vals else cheap
            lo, hi = sorted((int(room // max(cheap, 1)), int(room // max(dear, 1))))
            lo, hi = min(lo, hi), max(lo, hi)
            headroom = (f"  ·  headroom {room:,.0f} MYRRH ≈ {lo:,}-{hi:,} more "
                        f"recipients ({t['base_holders']} -> "
                        f"{t['base_holders'] + lo:,}-{t['base_holders'] + hi:,}) at "
                        f"{dear}/recipient (today's median) to {cheap}/recipient "
                        f"(a holder arriving at the bar)") if room > 0 else ""
        else:
            headroom = ("  ·  the MYRRH budget is a FIXED pot, so a wider field "
                        "costs nothing — turnout changes the prize per winner, "
                        "never the total minted")
        check("the drop is a sane share of the MYRRH float", share_m <= 0.35,
              f"planned {t['myrrh']:,} + published {published:,} = {total:,} MYRRH "
              f"= {share_m:.1%} of the {float_myrrh:,.0f} float" + headroom,
              "MYRRH's pool side is the smaller one and it halved at 60/20. The "
              "lever is `myrrh_ratio` (the PROPORTIONAL term — never a flat one, "
              "which the sybil law holds); never widen the pool to fit the drop")

    # ── THE FARM MUST OUT-EARN THE AIRDROP (operator, 2026-07-27) ───────────
    # *"make sure gardens are MORE competitive than an airdrop."* Stated as
    # something a run can fail rather than a sentence in a doc: one drop may not
    # mint more MYRRH than a fraction of what the Garden emits between drops.
    # The drops repeat quarterly and the Garden is MYRRH's ONLY other source, so
    # a drop that out-mints a quarter of the farm makes farming the slower way
    # to get the coin farming is supposed to be the only way to get.
    #
    # The rate is READ from the phase that deploys the farm, never typed here —
    # `setRewardPerSecond` is a live dial (`ScryGardener.sol`), and a hardcoded
    # 120/day would grade a farm that no longer exists.
    g = src("contracts/script/DeployGardener.s.sol")
    m_rps = re.search(r'vm\.envOr\("REWARD_PER_SECOND",\s*uint256\(([\d_]+)\)\)', g)
    if m_rps:
        per_day = int(m_rps.group(1).replace("_", "")) * 86_400 / 1e18
        per_quarter = per_day * DROP_PERIOD_DAYS
        ceiling = per_quarter * FARM_BEATS_DROP_SHARE
        # Graded PER DROP, not summed: each drop is compared against the farm's
        # output over the period before it, so two quarterly drops of the same
        # size are both fine and one double-sized drop is not.
        per_drop = [("planned", int(t["myrrh"]))]
        for p_ in sorted((REPO / "snapshots").glob("*.json")):
            d_ = _json(p_)
            if "params" in d_ and "allocations" in d_:
                per_drop.append((str(d_.get("drop_id", p_.name)),
                                 int((d_.get("totals") or {}).get("myrrh") or 0)))
        over = [(n, v) for n, v in per_drop if v > ceiling]
        check("the Garden out-earns any single drop (MYRRH)",
              not over,
              (f"farm {per_day:,.0f} MYRRH/day = {per_quarter:,.0f} a quarter; "
               f"ceiling {ceiling:,.0f} ({FARM_BEATS_DROP_SHARE:.0%}). "
               + " · ".join(f"{n} {v:,}" for n, v in per_drop))
              + ("" if not over else
                 f"  ·  OVER: {', '.join(f'{n} mints {v:,}' for n, v in over)}"),
              "cut the drop's MYRRH (the whole budget should be the fixed ticket "
              "pot — every broad term was retired 2026-07-27), or raise the "
              "farm's rate deliberately. Never widen the drop to fit the farm")


# ── 3b. the script that broadcasts must agree with the doc that decided ──────
# The SCRY column of the same table, per row. `[^|\d]*` skips markdown emphasis
# without crossing a pipe, so the row's SECOND cell is what gets read.
POOL_SCRY_RES = {
    "OBOL": re.compile(r"\|\s*OBOL/\SCRY\s*\|[^|\d]*([\d,]+)[^|]*\|"),
    "MYRRH": re.compile(r"\|\s*MYRRH/\SCRY\s*\|[^|\d]*([\d,]+)[^|]*\|"),
}


def gate_pool_seed_matches_decision():
    """`deploy_town.sh pools` is what actually mints the liquidity. Its defaults
    must be the decision doc's, and on 2026-07-26 they were not.

    One `POOL_SCRY_BUDGET` (default 40000000) was passed to BOTH
    `--obol-scry-budget` and `--myrrh-scry-budget`, so an armed run would have
    seeded **40M/40M** — the split the operator revised to **60M/20M** the same
    day. `LAUNCH-DECISIONS.md` said "the seed script takes `--myrrh-scry-budget`
    as an argument, so no code changes", which is true of the helper and false of
    the wrapper that calls it, and the wrapper is what broadcasts. The pools would
    have opened at the superseded split, out of the operator's own SCRY, with
    every doc and every test green — and a v3 pool's first mint IS its price, so
    it is not a knob you turn afterwards.

    Reads the SHIPPED defaults out of the script, never a doc about the script.
    """
    sh = src_required("contracts/deploy_town.sh",
                      "it asserts what the broadcasting script will actually seed")
    if not sh:
        return
    ld = src("LAUNCH-DECISIONS.md")
    bad, seen = [], []
    for sym, rx in POOL_SCRY_RES.items():
        m = rx.search(ld)
        decided = float(m.group(1).replace(",", "")) if m else None
        d = re.search(rf'POOL_SCRY_BUDGET_{sym}="\$\{{POOL_SCRY_BUDGET_{sym}:-(\d+)\}}"', sh)
        shipped = float(d.group(1)) if d else None
        if decided is None:
            bad.append(f"{sym}: no SCRY cell in the decision table")
        elif shipped is None:
            bad.append(f"{sym}: deploy_town.sh has no POOL_SCRY_BUDGET_{sym} default")
        elif shipped != decided:
            bad.append(f"{sym}: script seeds {shipped:,.0f} but the doc decided {decided:,.0f}")
        else:
            seen.append(f"{sym} {shipped:,.0f}")
    check("the seed script's pool sizes match the decision table", not bad,
          " · ".join(seen) + " SCRY, both as decided" if not bad else "; ".join(bad),
          "deploy_town.sh is what broadcasts — fix the DEFAULTS there, not the "
          "prose. A v3 pool's first mint sets its price; there is no second try")


def gate_pool_seed_single_source():
    """The seed sizes may live in ONE place, and it is the script that broadcasts.

    The gate above proves the script agrees with the decision doc. It cannot
    prove there are no OTHER copies, and on 2026-07-26 there were two: the same
    four figures were retyped in `dump_sim.py` and `tokenomics_sim.py`.
    `dump_sim`'s own header flagged it and told the reader to "change one, change
    all three" — a procedure, and procedures are what fail.

    It matters more in a SIM than in a doc. A stale doc is read by a human who
    can notice; a stale sim does not error, it prints a confident wrong answer
    about how far the token falls, from the one artifact whose entire job is to
    be believed before an irreversible mint. Both now read `pool_seeds.py`,
    which parses the script's shipped defaults.

    So: assert the reader agrees with the script, and assert nobody has
    reintroduced a literal beside it.
    """
    sh = src_required("contracts/deploy_town.sh",
                      "it is the single source the sims read their seeds from")
    if not sh:
        return
    sys.path.insert(0, str(REPO))
    try:
        import pool_seeds
        got = pool_seeds.seeds()
    except Exception as e:  # noqa: BLE001
        check("the pool seeds have a single source", False,
              f"pool_seeds.py did not load: {type(e).__name__}: {e}",
              "the sims import it; if it cannot read deploy_town.sh they run on "
              "nothing at all")
        return

    want = {}
    for sym, budget, ratio in (("OBOL", "POOL_SCRY_BUDGET_OBOL", "SCRY_PER_OBOL"),
                               ("MYRRH", "POOL_SCRY_BUDGET_MYRRH", "SCRY_PER_MYRRH")):
        b = re.search(rf'{budget}="\$\{{{budget}:-([\d.]+)\}}"', sh)
        r = re.search(rf'{ratio}="\$\{{{ratio}:-([\d.]+)\}}"', sh)
        want[sym] = (float(b.group(1)) if b else None, float(r.group(1)) if r else None)

    bad = []
    for sym, (budget, ratio) in want.items():
        if budget is None or ratio is None:
            bad.append(f"{sym}: deploy_town.sh default not found")
            continue
        if got[sym]["scry"] != budget:
            bad.append(f"{sym}: reader says {got[sym]['scry']:,.0f} SCRY, script says {budget:,.0f}")
        if abs(got[sym]["coin"] - budget / ratio) > 1e-6:
            bad.append(f"{sym}: coin side {got[sym]['coin']:,.0f} != budget/ratio")
    check("the seed reader agrees with the script it reads", not bad,
          f"OBOL {got['OBOL']['scry']:,.0f} + MYRRH {got['MYRRH']['scry']:,.0f} SCRY, "
          f"coin sides derived" if not bad else "; ".join(bad),
          "fix deploy_town.sh — pool_seeds.py is a reader, never a second opinion")

    # …and no launch artifact may carry its own copy. Check the SEED figures
    # specifically (60000000 / 20000000 with or without underscores), not any
    # large number, so an unrelated constant cannot fail this.
    lits = [re.compile(r"\b60[_,]?000[_,]?000\b"), re.compile(r"\b20[_,]?000[_,]?000\b")]
    retyped = []
    for rel in ("dump_sim.py", "tokenomics_sim.py"):
        body = src(rel)
        if not body:
            retyped.append(f"{rel}: unreadable")
            continue
        code = re.sub(r"#[^\n]*", "", body)          # a comment may still narrate it
        if any(rx.search(code) for rx in lits):
            retyped.append(rel)
    check("no sim retypes a pool seed figure", not retyped,
          "dump_sim.py + tokenomics_sim.py both read pool_seeds.py"
          if not retyped else f"a hardcoded seed is back in: {', '.join(retyped)}",
          "import pool_seeds and read it — a sim that disagrees with the "
          "broadcasting script is a wrong answer nobody will question")


def gate_season_pot_cap():
    """§Still open #2 — a season pot may not exceed 10% of the OBOL pool's OBOL
    reserve at the time it is funded.

    The sharpest asymmetry the launch maths found: the Gardener guards 240
    OBOL/day behind a 67% lock, a 90-day cliff and a slash ladder, while the
    Orchard has no lock, no cliff and no slash — and one season pot at the
    13.4% dump threshold is ~9.2 YEARS of Gardener emission in a single drop.
    The careful machinery is on the small number.

    Ten percent, under the ~13.4% dump threshold with margin: a fully-dumped
    10% pot moves price −17% by xy·k. (Corrected 2026-07-28 — the old label
    "13.4% causes −25%" mislabelled a proceeds figure; 13.4% moves price −22%
    and −25% takes ~15.6%. LAUNCH-DECISIONS.md §2 carries the correction; the
    cap is unchanged and conservative.) Not applicable until a pot is actually being
    posted (`SEASON_POT`, DeployOrchard's own env, in wei).
    """
    raw = os.getenv("SEASON_POT", "").strip()
    m = POOL_FLOAT_RE.search(src("LAUNCH-DECISIONS.md"))
    reserve = float(m.group(1).replace(",", "")) if m else None
    if not raw:
        return check("a season pot is within 10% of the OBOL reserve", True,
                     "SEASON_POT unset — no season is being posted. This gate "
                     "arms the moment one is"
                     + (f" (the cap would be {reserve * 0.10:,.0f} OBOL)"
                        if reserve else ""), "")
    try:
        pot_wei = int(raw)               # DeployOrchard takes reward WEI
    except ValueError:
        return check("a season pot is within 10% of the OBOL reserve", False,
                     f"SEASON_POT={raw!r} is not an integer wei amount",
                     "DeployOrchard reads it with vm.envOr(...uint256)")
    if reserve is None:
        return check("a season pot is within 10% of the OBOL reserve", False,
                     "could not read the OBOL reserve from LAUNCH-DECISIONS.md",
                     "keep the pool table parseable — code reads it")
    # Integer wei throughout. Dividing by 1e18 first made a pot one wei over the
    # cap compare EQUAL — float64 has 53 bits of mantissa and the cap is ~2^79
    # wei, so the last ~26 bits are unrepresentable. A gate that rounds in the
    # permissive direction is the one direction a gate must never round.
    cap_wei = int(reserve) * 10 ** 18 // 10
    pot, cap = pot_wei / 1e18, cap_wei / 1e18
    check("a season pot is within 10% of the OBOL reserve", pot_wei <= cap_wei,
          f"SEASON_POT = {pot:,.0f} OBOL = {pot_wei / (int(reserve) * 10 ** 18):.1%} "
          f"of the {reserve:,.0f} reserve (cap {cap:,.0f})",
          "posted rule (LAUNCH-DECISIONS.md §Still open #2): 10%, under the "
          "~13.4% dump threshold — a fully-dumped 10% pot moves price -17% by "
          "xy*k. Shrink the pot or run more, shorter seasons")


POWDER_RE = {
    "OBOL": re.compile(r"powderObol\s*=\s*vm\.envOr\(\s*\"POWDER_OBOL\"\s*,\s*"
                       r"uint256\(([\d_]+)e18\)\s*\)"),
    "MYRRH": re.compile(r"powderMyrrh\s*=\s*vm\.envOr\(\s*\"POWDER_MYRRH\"\s*,\s*"
                        r"uint256\(([\d_]+)e18\)\s*\)"),
}


def gate_powder_tracks_the_drop(snap):
    """The powder is 10% of what the drop pays, and the drop stays whole.

    Operator, 2026-07-27, after three passes at the wording: *"i just want 10%
    of whatever amount was getting air dropped, like that total, whatever it is
    1000 lets say i want 10% of that number but it stays 1000."*

    WHY THE DROP IS THE DENOMINATOR. "10% of the coins" has none - OBOL and
    MYRRH are elastic. The three denominators tried before this all disagree on
    the same allocation, by up to 1000x: 1,600,000 OBOL was simultaneously 8.6%
    of genesis float, 26.7% of the pool that reaches SCRY, and (against
    `tokens.circulating`, which excludes pool float on purpose) a share that
    would put 10% at 104,075. About 95% of genesis OBOL is pool inventory, so
    "% of supply" is really a claim about the pools, and "% of circulating" is
    mostly a claim about the powder itself. The drop is the one total that is
    neither: fixed, published, and recomputable by anyone from the snapshot.

    WHY IT NEEDS A GATE AND NOT A COMMENT. This exact failure has now happened
    twice - a quantity sized against a number that later moved without it. The
    60/20 revision cut the MYRRH/SCRY pool 800,000 -> 400,000 and took the
    drop's MYRRH from 10.3% to 20.6% of float (gated 2026-07-26); the powder
    then sat at 60% of that same pool for a day with nothing watching. A rule
    that lives only in a comment is the shape both of those had.

    So this recomputes the drop from the SAME snapshot and pinned params the
    drop gates use, and reds if the Solidity defaults are not exactly 10% of it.
    Change the bar, the snapshot, or a payout knob and this names the new
    number rather than letting the powder drift.
    """
    name = "the powder tracks its posted rule (OBOL posted, MYRRH 10% of the drop)"
    if not snap:
        return check(name, False, "no snapshot to compute the drop from",
                     "the powder is defined as a share of the drop; without a "
                     "snapshot there is no drop to take a share of")
    sys.path.insert(0, str(REPO))
    try:
        from meter import claim_plan as cp
    except Exception as e:  # noqa: BLE001
        return check(name, False, f"claim_plan will not import: {e}", "")

    params, _ = _next_drop_params()
    plan = cp.build_launch_plan(
        snap, params, exclude=cp.excluded_wallets(str(REPO / "snapshots")))
    totals = plan["totals"]

    text = src("contracts/script/DeploySpoils.s.sol")
    if not text:
        return check(name, False, "cannot read DeploySpoils.s.sol",
                     "it carries the powder defaults this rule constrains")

    # THE TWO COINS FOLLOW DIFFERENT RULES SINCE 2026-07-28, and the split is
    # the point rather than an exception. OBOL is a POSTED FIXED allocation for
    # public giveaways; MYRRH keeps the 10%-of-drop rule because it is capped
    # and its lifetime headroom is ~1,000,000 against a 40-year schedule.
    # Neither is typed twice: OBOL is read back out of LAUNCH-DECISIONS.md,
    # MYRRH is recomputed from the drop. The anti-drift property the original
    # rule bought is preserved for both, against different denominators.
    ld = src("LAUNCH-DECISIONS.md") or ""
    m_posted = re.search(r"powder \(OBOL\)\s*\|[^|]*\|\s*\*\*([\d,]+)\*\*", ld)
    posted_obol = int(m_posted.group(1).replace(",", "")) if m_posted else None

    bad, shown = [], []
    for sym in ("OBOL", "MYRRH"):
        m = POWDER_RE[sym].search(text)
        if m is None:
            bad.append(f"{sym}: no parseable powder default in DeploySpoils")
            continue
        got = int(m.group(1).replace("_", ""))
        if sym == "MYRRH":
            want = int(totals["myrrh"]) // 10          # floor, never round up
            shown.append(f"MYRRH {got:,} vs {want:,} (10% of the {totals['myrrh']:,} drop)")
            if got != want:
                bad.append(f"MYRRH: powder default is {got:,}, 10% of the "
                           f"{totals['myrrh']:,} drop is {want:,}")
            continue
        if posted_obol is None:
            bad.append("OBOL: LAUNCH-DECISIONS.md carries no `powder (OBOL)` row "
                       "to read the posted allocation from")
            continue
        shown.append(f"OBOL {got:,} vs {posted_obol:,} posted")
        if got != posted_obol:
            bad.append(f"OBOL: powder default is {got:,}, the posted allocation "
                       f"is {posted_obol:,}")
        # ...and it must stay a MINORITY of the pool inventory. A posted number
        # has no denominator of its own, so without a ceiling it can be raised
        # to anything one edit at a time. 25% of the OBOL that opens the pools
        # is generous and still unmistakably a working allocation.
        mf = re.search(r"OBOL/SCRY \|\s*\*\*[\d,]+\*\*\s*\|\s*\*\*([\d,]+) OBOL\*\*", ld)
        if mf:
            float_obol = int(mf.group(1).replace(",", ""))
            if got > float_obol // 4:
                bad.append(f"OBOL: powder {got:,} exceeds 25% of the "
                           f"{float_obol:,} OBOL pool float — that is an "
                           f"allocation, not a working float. Say it out loud "
                           f"in SENTENCES.md and raise this ceiling on purpose")
            else:
                shown.append(f"{got / float_obol:.1%} of pool float")

    check(name, not bad,
          " · ".join(shown) if not bad else "; ".join(bad),
          "OBOL is read back from LAUNCH-DECISIONS.md's `powder (OBOL)` row and "
          "MYRRH is recomputed from the drop - neither is typed beside the "
          "other. The drop stays whole either way: the powder is ON TOP of it, "
          "never carved out of it")


TABLE_POT_RE = re.compile(r'^\s*(?://\s*)?SCRY_TABLE_POT_SEED\s*:\s*"(\d+)"',
                          re.MULTILINE)


def gate_every_emission_knob_is_recorded():
    """Every numeric dial on a surface that mints, burns or prices coin is
    classified in `meter/knobs.py` — so a new one cannot appear without someone
    recording whether a denomination change has to move it.

    This is a COMPLETENESS check, not a values check. The failure it exists
    against did not error when it happened: the 10x redenomination moved the
    per-event rewards and missed two AGGREGATE caps, so the augury paid the
    right amount to about nine wallets a day town-wide and bound above ~11 DAU
    with every gate green. `test_tokenomics.py` checks ratios BETWEEN knobs —
    but only across the seven it hand-types into a `FILES` table, and a
    hand-typed list of things-to-check is exactly what rots. This makes the
    list derive itself: discovery is AST over the economy modules, so adding a
    knob adds a row here whether or not anyone remembered to.
    """
    name = "every emission knob is recorded"
    meter = str(REPO / "meter")
    if not os.path.isdir(meter):
        return check(name, True, "no meter/ in this tree — not applicable", "")
    sys.path.insert(0, meter)
    try:
        import knobs as _knobs
    except Exception as e:  # noqa: BLE001 — a missing dep must not red the launch
        return check(name, True, f"knobs.py not importable here ({e!r})",
                     "run `python3 meter/knobs.py` directly")
    finally:
        if meter in sys.path:
            sys.path.remove(meter)
    missing = _knobs.unclassified()
    total = len({k["knob"] for k in _knobs.discover_python() if k["economy"]})
    check(name, not missing,
          f"{total} knobs on {len(_knobs.ECONOMY_MODULES)} economy surfaces, "
          f"all classified" if not missing else
          "UNRECORDED: " + ", ".join(
              f"{m['knob']} ({m['module']}:{m['line']})" for m in missing),
          "classify each in meter/knobs.py CLASSIFIED — `emission` if a "
          "redenomination must move it, `policy` if it must NOT (a day count, "
          "a bps rate, a per-day limit). That decision is the whole point")


def gate_table_pot_cap():
    """The Temptation Table's bankroll seed obeys the SAME 10%-of-the-OBOL-
    reserve rule as a season pot, because it is the same hazard.

    §Still open #2 wrote that rule for `SEASON_POT` and stopped there. But a
    pot is a pot: `SCRY_TABLE_POT_SEED` mints OBOL through
    `tokens.harvest_earn` -> `mint(HARVEST_TOKEN)` into the `__table__`
    account, and every coin of it can reach a player as winnings and then the
    OBOL/SCRY pool. Nothing was checking it, and the first number named for it
    (2,500,000 on 2026-07-27) was **4.2x** the cap the identical rule already
    imposed next door — 41.7% of the OBOL reserve against a posted 10%.

    THE COUPLING THIS GATE ACTUALLY CLOSES, and it is why the cap is expressed
    as a FRACTION rather than a literal: the pot and the OPENING RATIO are one
    decision made in two documents. The reserve is 60,000,000 SCRY / rate, so
    moving the posted 10 SCRY/OBOL to 100 takes the OBOL side from 6,000,000
    to 600,000 and a fixed 2,500,000 pot from 41.7% of it to **416.7%** — four
    times the entire OBOL side of the pool it would sell into. Read off
    LAUNCH-DECISIONS.md's table, this fires the moment either number moves.

    Softer than the season gate in exactly one way, and deliberately: a season
    pot is PAID OUT, while the table's pot is a BANKROLL that only leaks
    through net wins against a rake that is negative-EV for players. So this is
    a ceiling on realised emission, not a forecast of it. The ceiling is still
    the right thing to bound — `payout <= fee income` cannot mean anything
    against an unbounded tail, which is the whole reason the Table was
    converted from minting to a bankroll in the first place.
    """
    name = "the table pot is within 10% of the OBOL reserve"
    m = POOL_FLOAT_RE.search(src("LAUNCH-DECISIONS.md"))
    reserve = float(m.group(1).replace(",", "")) if m else None
    if reserve is None:
        return check(name, False,
                     "could not read the OBOL reserve from LAUNCH-DECISIONS.md",
                     "keep the pool table parseable — code reads it")
    cap = int(reserve) // 10

    # Two sources, and BOTH are graded. The env is what an armed meter would
    # actually run; the tracked example config is where the DECISION lives, and
    # the live ecosystem.config.js is untracked, so the example is the only
    # copy any gate here can see. A silent-empty read is not a pass: if the
    # example cannot be read or carries no value at all, that is a RED, because
    # "we could not look" and "it is zero" are the same bytes otherwise.
    cfg = REPO / "meter" / "ecosystem.config.example.js"
    try:
        text = cfg.read_text(encoding="utf-8", errors="replace")
    except OSError as e:  # noqa: BLE001
        return check(name, False, f"cannot read {cfg.name}: {e}",
                     "the example config is the tracked record of this decision")
    hit = TABLE_POT_RE.search(text)
    if hit is None:
        return check(name, False,
                     "ecosystem.config.example.js names no SCRY_TABLE_POT_SEED",
                     "record the decided seed there — an unrecorded knob is one "
                     "nobody can check, and the live config is untracked")
    declared = int(hit.group(1))

    env_raw = os.getenv("SCRY_TABLE_POT_SEED", "").strip()
    over = []
    if declared > cap:
        over.append(f"the example config declares {declared:,}")
    if env_raw:
        try:
            live = int(env_raw)
        except ValueError:
            return check(name, False,
                         f"SCRY_TABLE_POT_SEED={env_raw!r} is not an integer",
                         "meter/table.py reads it with int()")
        if live > cap:
            over.append(f"the environment sets {live:,}")

    where = f"declared {declared:,}" + (f", env {env_raw}" if env_raw else "")
    check(name, not over,
          f"{where} OBOL vs a {cap:,} cap "
          f"({declared / reserve:.1%} of the {reserve:,.0f} reserve)"
          if not over else
          f"OVER THE CAP — {' and '.join(over)} OBOL against a {cap:,} ceiling "
          f"({declared / reserve:.1%} of the {reserve:,.0f} OBOL reserve)",
          "same posted rule as a season pot (LAUNCH-DECISIONS.md §Still open "
          "#2): 10%, under the ~13.4% dump threshold — a fully-dumped 10% pot "
          "moves price -17% by xy*k. The pot and the opening "
          "ratio are ONE decision — if you raise the SCRY/OBOL rate the "
          "reserve shrinks and this cap shrinks with it")


def gate_tvl_is_never_summed():
    """§Still open #3 — no surface may publish a TVL that sums house-minted
    inventory with purse-funded liquidity.

    The free MYRRH/OBOL Garden books $6,080 — more than both purse-funded pools
    combined ($4,864) and 2.5x the $2,432 of real SCRY they cost. It is the
    largest game-pool line on the sheet and it cost nothing, by design. Summing
    them would be the most misleading number this town could publish, and it
    would be entirely self-inflicted.

    A tripwire, not a gate on today. It armed for real on 2026-07-26, the day
    `GET /pools` shipped, and caught the aggregate on the first run:
    `"tvl_usd": sum(c["holds"]["tvl_usd"] for c in live ...)`.

    ⚠ IT FIRES ON AGGREGATION, NOT ON THE WORD. The first version failed any
    file carrying a `tvl` key at all, which reddened `watchtower/js/pools.js`
    and `meter/test_pools.py` — both of which only ever show ONE POOL's number,
    which is honest and is the whole point of a per-pool table. Demanding the
    split names there would have taught everyone that this gate cries wolf, and
    a check people learn to skip protects nothing (the rule the secret guard
    states next door about bare 64-hex). So: per-pool TVL is fine, always. What
    is forbidden is ONE NUMBER that mixes the two kinds of depth.
    """
    # Summing constructs, anywhere near a tvl reference.
    agg = re.compile(r"\bsum\s*\(|\.reduce\s*\(|\btotal[_a-z]*tvl\b|\btvl[_a-z]*total\b|tvl\w*\s*\+=")
    # A tvl KEY or BINDING, capturing only its own value — so `+` in the value
    # ("tvl": funded + house) aggregates, while a `+` merely on the same line
    # (string concatenation around a per-pool read) does not.
    binding = re.compile(r"""["']tvl\w*["']\s*:|\btvl\w*\s*=""")

    def aggregates(text):
        lines = text.splitlines()
        for i, ln in enumerate(lines):
            if "tvl" not in ln and "total_value_locked" not in ln:
                continue
            if agg.search("\n".join(lines[max(0, i - 1):i + 3])):
                return True
            m = binding.search(ln)
            if m:
                rhs = re.split(r"[,;}]", ln[m.end():], 1)[0]
                if "+" in rhs:
                    return True
        return False

    hits = []
    for d in ("meter", "watchtower", "watchtower/js"):
        base = REPO / d
        if not base.is_dir():
            continue
        for p in sorted(base.glob("*.py")) + sorted(base.glob("*.js")) + \
                sorted(base.glob("*.html")):
            t = p.read_text(errors="ignore")
            # Candidate = the file touches TVL at all. Prose alone never fails —
            # only aggregation does — so casting a wide net here costs nothing
            # and closes the JS hole: `pools.reduce((a,p) => a + p.tvl_usd, 0)`
            # has no quoted key and no `tvl =` binding, so a key-only scan never
            # saw the one shape that most obviously sums.
            if "tvl" not in t and "total_value_locked" not in t:
                continue
            ag = aggregates(t)
            if not (ag or binding.search(t)):
                continue  # mentions TVL, publishes none — nothing to say about it
            split = "tvl_funded" in t and "tvl_house_minted" in t
            hits.append((str(p.relative_to(REPO)), split, ag))
    unsplit = [h for h, split, ag in hits if ag and not split]
    per_pool = [h for h, _, ag in hits if not ag]
    check("no surface publishes a summed TVL", not unsplit,
          "no TVL field anywhere yet — this gate arms with the first one"
          if not hits else
          (f"{len(hits)} surface(s) publish TVL; {len(per_pool)} per-pool only, "
           f"{len(hits) - len(per_pool)} aggregated and split into "
           f"tvl_funded + tvl_house_minted" if not unsplit
           else f"summed TVL in: {', '.join(unsplit)}"),
          "report `tvl_funded` and `tvl_house_minted` as separate fields, never "
          "a total. The free MYRRH/OBOL Garden books more than both purse-funded "
          "pools combined and cost nothing — summing them prints a number that is "
          "2.5x the real SCRY at risk")


# ── 4. genesis float must not swamp the balancing gauge ──────────────────────
POOL_FLOAT_REASON = "pool-float"


def gate_house_never_claims():
    """The house may not appear in its own drop.

    The stealth drop's whole security argument is that the snapshot "cannot be
    bought into after the fact by anyone, INCLUDING the operator"
    (`LAUNCH-DECISIONS.md`). A plan that pays the operator's own wallets
    contradicts that sentence on its face, whatever the intent was.

    Found live, 2026-07-26: the committed drop-one rehearsal paid the operator's
    14.25% wallet 10,500 OBOL + **7,151 MYRRH — 18% of the entire drop's MYRRH**
    — and drew it one of the twelve golden tickets, 833,333 SCRY. The draw is
    balance-weighted, so the largest holder winning is the EXPECTED outcome
    rather than bad luck: anyone who checked would have read it as a rigged
    lottery, and they would have had the arithmetic on their side. Nothing was
    armed, which is the only reason this is a fix and not an incident.

    Two failure directions, and both are checked, because the list is only a
    control if it is READ:
      - a house wallet holds an allocation in some plan on disk;
      - `snapshots/house-wallets.json` is missing/empty, so every plan "passes"
        by having nothing to compare against — the vacuous pass this file has
        now been bitten by twice.
    """
    sys.path.insert(0, str(REPO))
    try:
        from meter import claim_plan as cp
    except Exception as e:  # noqa: BLE001
        return check("no excluded wallet claims from the drop", False, f"{e}", "")
    house = set(cp.house_wallets(str(REPO / "snapshots")))
    # House-side wallets the operator does not control (the pons fee wallet) are
    # excluded identically and must be verified identically — an exclusion
    # nobody checks is a note, not a control, which is the sentence the house
    # list itself was written under. Kept as its own set so the failure message
    # can say WHICH kind was paid: "the operator paid himself" and "a protocol
    # fee wallet was paid" are different incidents with different fixes.
    other = set(cp.other_excluded(str(REPO / "snapshots")))
    if not house:
        return check(
            "no excluded wallet claims from the drop", False,
            f"snapshots/{cp.HOUSE_WALLETS_FILE} is missing, unreadable, or lists no "
            f"wallets — every plan below would pass by having nothing to compare to",
            "list the operator's own addresses there. On-chain heuristics can "
            "suggest a link and never prove ownership, so it is a confirmed list, "
            "not a derived one")
    bad, checked = [], 0
    for p in sorted((REPO / "snapshots").glob("*.json")):
        # The list cannot be checked against itself: `house-wallets.json` names
        # every one of these wallets by design, and that is the control rather
        # than a violation of it.
        if p.name == cp.HOUSE_WALLETS_FILE:
            continue
        d = _json(p)
        # ⚠ THE LINE IS DISTRIBUTION vs ACCESS, AND IT IS NOT THE FILE FORMAT.
        # This gate read `allocations` only — the airdrop-plan dialect — and
        # `continue`d past everything else. That skipped the drop's own MERKLE
        # artifacts, which are the same distribution in a different shape and
        # carry `claims[w].amount_wei`; a house wallet could sit in one and this
        # gate would pass it. That is the real gap, and it is closed below.
        #
        # ⚠ WHAT IS **NOT** A GAP: a free-mint door's cohort. `SENTENCES.md`
        # 2026-08-12 row 497 (*"im fine with house wallets"*) settles it —
        # `house-wallets.json` is deliberately NOT subtracted from the seat
        # door, because the door is an ACCESS shape and a distribution is not.
        # One free seat is not a transfer to itself, and that row explicitly
        # names `build_allowlist`'s no-exclusion default as the correct
        # behaviour. A session "closed" this on 2026-08-13 by reading the pass
        # as a hole, rebuilt the cohort against a rule nobody had asked for, and
        # had to put it back. So the access dialect is skipped here BY DECISION,
        # and re-adding it needs a sentence rather than an instinct.
        #
        # A holder snapshot is not graded either, for a plainer reason: it is a
        # measurement of who held what, and the house appearing in one is the
        # truth rather than a payout.
        rows: dict[str, object] | None = None
        what = ""
        if isinstance(d.get("allocations"), dict):
            rows, what = d["allocations"], "holds"
        elif isinstance(d.get("claims"), dict):
            # An access tree's claim carries `allowance`; a distribution's
            # carries `amount_wei`. Only the second is this gate's business.
            first = next(iter(d["claims"].values()), None)
            if isinstance(first, dict) and "allowance" in first:
                continue
            rows, what = d["claims"], "can claim"
        if rows is None:
            continue
        checked += 1
        # Map lowercase -> the artifact's own key. Comparing lowercased and then
        # indexing with the lowercased key crashes on a checksummed plan, which
        # is a legal artifact — an address is not a string, and every comparison
        # here has to survive the difference.
        by_lower = {w.lower(): w for w in rows}
        for w in sorted((house | other) & set(by_lower)):
            kind = "HOUSE (operator)" if w in house else "house-side, not ours"
            a = rows[by_lower[w]]
            # Artifact shapes on disk, and every one must be readable: the
            # three-token plan maps wallet -> {obol, myrrh, scry, ticket}, the
            # 2026-07-22 single-token one maps wallet -> an int, and an access
            # tree maps wallet -> {allowance, proof}. Assuming any one shape
            # crashed the gate on the others — and a gate that raises is a gate
            # that gets commented out.
            if isinstance(a, dict) and "allowance" in a:
                amt = f"{int(a['allowance']):,} seat(s)" + (
                    f" at door {d['door']}" if d.get("door") else "")
            elif isinstance(a, dict) and "amount_wei" in a:
                amt = f"{int(a['amount_wei']) // 10 ** 18:,} {d.get('token', 'units')}"
            elif isinstance(a, dict):
                amt = (f"{a.get('obol', 0):,} OBOL / {a.get('myrrh', 0):,} MYRRH / "
                       f"{a.get('scry', 0):,} SCRY"
                       + (" + A GOLDEN TICKET" if a.get("ticket") else ""))
            else:
                amt = f"{a:,} {d.get('token', 'units')}"
            bad.append(f"{p.name}: [{kind}] {w[:10]}… {what} {amt}")
    check("no excluded wallet claims from the drop", not bad,
          f"{len(house)} operator + {len(other)} house-side-not-ours, "
          f"{checked} distribution(s) on disk (plans + their merkle artifacts), "
          f"none pays either. Free-mint door cohorts are ACCESS and deliberately "
          f"out of scope — SENTENCES.md 2026-08-12 row 497"
          if not bad else "; ".join(bad),
          "re-plan with the house excluded: `python3 meter/claim_plan.py plan "
          "--snapshot … --label founders` excludes them by default. Note the draw "
          "is over the WHOLE field, so removing a wallet re-draws every ticket — "
          "that is correct, and it is free while nothing is armed")


def gate_drop_claim_isolation():
    """§0.4 — one ScryHarvest instance may serve exactly ONE root producer.

    `ScryHarvest` stores `claimed[wallet]` as a LIFETIME figure and pays
    `cumulative - claimed`, i.e. its roots are cumulative. `claim_plan.py`
    emits PER-DROP ABSOLUTE amounts computed from one snapshot, with no
    reference to what a wallet was paid before. The two conventions are not
    compatible on one instance:

      drop one posts  alice -> 1,500      alice claims 1,500, claimed = 1,500
      drop two posts  alice ->   900      alice claims  900 - 1,500 -> NOTHING

    A wallet in both drops is silently underpaid or zeroed, and the bridge
    (which IS cumulative, off `/tokens/claim-root`) is a third producer with
    the other convention. So: every drop gets its own contracts, and the
    bridge's harvest is never a drop's claim target.

    This was written when the launch grew a SECOND drop a week after the
    first — the collision is dormant with one drop and permanent with two.
    """
    plans = sorted((REPO / "snapshots").glob("*.json"))
    drops = set()
    for p in plans:
        try:
            d = json.loads(p.read_text())
        except Exception:
            continue
        if isinstance(d, dict) and d.get("drop_id"):
            drops.add(str(d["drop_id"]))

    dep = REPO / "contracts" / "deployments.json"
    claims, bridge = {}, ""
    if dep.is_file():
        try:
            ch = json.loads(dep.read_text()).get("chains", {}).get("4663", {})
            claims = ch.get("claims", {}) or {}
            bridge = (ch.get("contracts", {}).get("ScryHarvest", {}) or {}).get("address", "")
        except Exception:
            claims = {}

    # Rule 1: no address serves two drops.
    owner, shared = {}, []
    for drop_id, toks in claims.items():
        for tok, addr in (toks or {}).items():
            a = str(addr).lower()
            if a in owner and owner[a] != drop_id:
                shared.append(f"{a} serves both {owner[a]} and {drop_id}")
            owner[a] = drop_id
    # Rule 2: the bridge's own harvest is never a drop's claim target.
    if bridge and bridge.lower() in owner:
        shared.append(f"{bridge} is the BRIDGE harvest and also {owner[bridge.lower()]}'s claim")

    # Non-vacuity, without blocking a launch that has not got there yet.
    # DeployClaim runs AFTER the launch phases, so "no claim contracts" is the
    # correct state at `deploy_town.sh launch` time and must not be a red. But
    # once any claim contract EXISTS and the per-drop record does not, this
    # gate has checked nothing while looking like a pass — the exact defect
    # wave 6 closed one function over (`src()` -> "" and `"X" not in ""`).
    armed = [k for k in ("SCRY_CLAIM_OBOL", "SCRY_CLAIM_MYRRH", "SCRY_CLAIM_SCRY")
             if os.getenv(k, "").strip()]
    if not claims:
        if armed:
            return check(
                "each drop's claim contracts are its own (§0.4)", False,
                f"{len(armed)} claim contract(s) armed in the environment ({', '.join(armed)}) "
                f"and deployments.json records no `claims` map — nothing was verified",
                "record chains.4663.claims = {\"<drop_id>\": {\"OBOL\": \"0x…\", …}} in the "
                "SAME commit as each DeployClaim broadcast, then re-run")
        return check(
            "each drop's claim contracts are its own (§0.4)", True,
            f"not yet applicable — {len(drops)} drop plan(s) on disk "
            f"({', '.join(sorted(drops))}), no claim contract deployed or armed. "
            "This gate arms the moment one is.",
            "")

    return check(
        "each drop's claim contracts are its own (§0.4)", not shared,
        f"{len(drops)} drop plan(s), {len(owner)} claim contract(s) recorded, no reuse"
        if not shared else "; ".join(shared),
        "ScryHarvest roots are CUMULATIVE and claim_plan emits PER-DROP ABSOLUTE "
        "amounts — sharing an instance between two drops permanently underpays "
        "every wallet in both. Deploy a fresh set per drop (DeployClaim.s.sol).")


def gate_pool_float_accounting():
    """The genesis pool mints (4M OBOL, 800k MYRRH, and 10M/2M for the free
    MYRRH/OBOL pool) are NOT play emission. If they land in the same bucket as
    earned coin, `sink_coverage` reads ~0 forever and the balancing gauge is
    useless from the first day. This wants a real implementation, not a
    docstring — so the check looks for the identifier, not the word."""
    t = src("meter/tokens.py")
    has_reason = POOL_FLOAT_REASON in t or "POOL_FLOAT" in t
    has_excl = re.search(r"def\s+circulating|exclude_pool|pool_held", t) is not None
    check("pool-float is excluded from the balancing gauge", has_reason and has_excl,
          f"reason marker: {'yes' if has_reason else 'NO'}; "
          f"exclusion logic: {'yes' if has_excl else 'NO'}",
          "mint genesis pool inventory with its own reason string, and have "
          "/tokens + /tokens/flux report circulating EXCLUDING pool-held "
          "balances. Until this lands, sink_coverage will be swamped by "
          "~16M units of genesis float and tell you nothing.")


# ── 5. the farm's reward coin matches the decision of record ─────────────────
def gate_farm_reward():
    """The farm's crop moved twice in two days (OBOL 07-25, back to MYRRH 07-26),
    so what matters is not WHICH coin but that the choice stays a deploy-time
    knob and that the docs agree with the script. Plus the half that makes the
    07-26 flip safe: **play must mint no MYRRH.**"""
    g = src("contracts/script/DeployGardener.s.sol")
    check("the farm's reward token is a deploy-time choice, not welded",
          "REWARD_TOKEN" in g,
          "DeployGardener reads REWARD_TOKEN" if "REWARD_TOKEN" in g
          else "DeployGardener hardcodes its reward token",
          "the crop moved twice in two days; keep it an env var")

    # THE COIN IS DERIVED FROM THE ONE LINE THAT BROADCASTS IT, never typed
    # here. `deploy_town.sh gardener` exports REWARD_TOKEN, and that export is
    # what `DeployGardener` binds into the granary — so it is the only sentence
    # in the repo that actually decides the farm's crop. Everything else in
    # this gate is checked against it.
    dt = src("contracts/deploy_town.sh")
    m = re.search(r'export REWARD_TOKEN="\$(OBOL|MYRRH)" SEED_MYRRH_OBOL', dt)
    farm = m.group(1) if m else None
    check("the farm's crop is readable from the phase that deploys it",
          farm is not None,
          f"deploy_town.sh gardener exports REWARD_TOKEN=${farm}" if farm
          else "could not find the gardener phase's REWARD_TOKEN export",
          "every other check below compares against this one line")
    if not farm:
        return
    other = "OBOL" if farm == "MYRRH" else "MYRRH"

    gardens = src("FARMING.md")
    doc_ok = f"farm reward token | **{farm}**" in gardens
    doc_says = ("MYRRH" if "farm reward token | **MYRRH**" in gardens
                else "OBOL" if "farm reward token | **OBOL**" in gardens
                else "nothing this gate can read")
    check(f"FARMING.md agrees the farm emits {farm}",
          doc_ok,
          f"FARMING.md posts {farm} as the farm reward" if doc_ok else
          f"deploy_town.sh pays {farm}; FARMING.md §3 posts {doc_says}",
          "doc and script must not disagree about what the farm pays — and the "
          "failure names BOTH sides, because a message that only names one is "
          "how the last two flips read as correct")

    # ── the two places IN deploy_town.sh that name the coin a second time ────
    # Both have been wrong, in both directions, and neither announced itself.
    # `status` printed a confident "= the granary (correct)" for a coin the farm
    # does not pay — in the single tool you read to confirm a broadcast worked.
    # `config` wrote the wrong symbol AND the wrong token address into
    # gardens.config.json, which is the file the gardens PAGE reads: the only
    # one of the three that reaches a player. Both must derive, not restate.
    for phase, needle in (("status", "FARM_COIN_SYM=$(farm_coin_sym)"),
                          ("config", "FARM_SYM=$(farm_coin_sym)")):
        ok = needle in dt
        check(f"deploy_town.sh `{phase}` derives the farm coin, never restates it",
              ok,
              f"{phase} asks farm_coin_sym()" if ok else
              f"{phase} names a coin of its own — it will desync on the next move",
              "the crop has moved twice and each move left a second place "
              "asserting the old answer; derive it from the gardener phase")
    # Narrow on purpose: `"sym": "OBOL"` appears legitimately in the same JSON,
    # naming token1 of the MYRRH/OBOL pair being farmed. Only the REWARD slot is
    # the one that has been wrong, so only the reward slot is graded.
    emits_ok = ('"reward": {"sym": farm_sym, "addr": farm_addr' in dt
                and not re.search(r'"reward":\s*\{\s*"sym":\s*"(OBOL|MYRRH)"', dt))
    check("the emitted gardens.config.json cannot name the wrong coin",
          emits_ok,
          "the config phase writes the derived symbol and address" if emits_ok
          else f'the config phase still hardcodes "{other}" as the farm reward',
          "gardens.config.json is what the gardens page reads — of the places "
          "that name this coin, it is the one a player sees")
    # The sole-source rule, as a launch gate rather than a paragraph. Broadcasting
    # a MYRRH granary + gardener while the barrow still drops MYRRH re-opens the
    # exact divergence the 07-26 flip closed — measured at 2,509 MYRRH/day of play
    # mint against 1,320/day of burn at dau=200 (tokenomics_sim.py). Nothing in
    # the contracts can see this; it lives in a Python env default.
    br = src("meter/barrow_rules.py")
    m = re.search(r'SCRY_BARROW_MYRRH_BANDS",\s*\'(.*?)\'', br)
    empty = bool(m) and m.group(1).strip() in ("{}", "{ }")
    check("play mints no MYRRH — the Garden is its only source",
          empty,
          "SCRY_BARROW_MYRRH_BANDS is empty; the Gardener is the only tap"
          if empty else
          f"the barrow still drops MYRRH: {m.group(1) if m else 'unreadable'}",
          "FARMING.md §3a — farm the premium coin, do not also loot it. "
          "meter/test_tokenomics.py law 6 asserts the same thing")


# ── 5b. the two strings that can never be corrected ──────────────────────────
def gate_token_identity():
    """`SpoilsToken` sets `name` and `symbol` in its constructor and has NO
    setter, so both are welded at broadcast — they are what every wallet,
    explorer and aggregator shows forever. This shipped as lower-case
    "obol"/"myrrh" and nobody would have noticed until it was permanent.

    House form, matching canonical SCRY ("Scry Agent Ward" / SCRY): the symbol
    is a bare upper-case ticker, the name is ordinary capitalised English."""
    ds = src("contracts/script/DeploySpoils.s.sol")
    for coin in ("OBOL", "MYRRH"):
        m = re.search(rf'vm\.envOr\("{coin}_NAME",\s*string\("([^"]*)"\)\)', ds)
        nm = m.group(1) if m else None
        ok = bool(nm) and nm[0].isupper() and nm != nm.upper()
        check(f"{coin}'s deployed NAME is house form and welded-safe",
              ok,
              f'name = "{nm}"' if ok else
              (f'name = "{nm}" — a welded name must be capitalised English, '
               f'not a ticker or lower case' if nm else
               f"could not read {coin}_NAME's default out of DeploySpoils"),
              "SpoilsToken has no setter for it; SCRY's own name is "
              '"Scry Agent Ward"')
        sym_ok = bool(re.search(rf'new SpoilsToken\(inp\.\w+, "{coin}",', ds))
        check(f"{coin}'s deployed SYMBOL is the bare upper-case ticker",
              sym_ok,
              f'symbol = "{coin}"' if sym_ok else
              f"DeploySpoils does not construct {coin} with a bare "
              f'"{coin}" symbol and a variable name',
              "tickers are bare and upper case; CLAUDE.md §the register")

    # ── the caps, which are welded and asymmetric ON PURPOSE ────────────────
    # `cap` is a LIFETIME MINT BUDGET, not a supply ceiling: `_burn` never
    # decrements `totalMinted`, so a burn refunds nothing. That makes a cap
    # lethal to OBOL (mint-through-play, burn-through-sinks — the healthy cycle
    # would be a countdown to play not paying) and free for MYRRH, which has
    # exactly one tap, the Garden.
    #
    # The headroom is DERIVED, never typed. It was typed once ("~420 years at
    # the posted 120/day") and went stale the moment the rate went back to 240
    # on 2026-07-27 — a gate whose PASS message asserts a number the repo no
    # longer holds is worse than no message, because it reads as confirmation.
    _g = src("contracts/script/DeployGardener.s.sol")
    _m_rps = re.search(r'vm\.envOr\("REWARD_PER_SECOND",\s*uint256\(([\d_]+)\)\)', _g)
    _per_yr = (int(_m_rps.group(1).replace("_", "")) * 86_400 * 365 / 1e18) if _m_rps else 0
    # ⚠ _per_yr is the ERA-0 rate, and dividing headroom by it is WRONG since
    # 2026-07-28: the farm halves every 4 years and terminates, so a flat
    # extrapolation reported "~8 years of headroom" against a schedule that
    # actually runs 40 and stops ~1.07M short of the cap. Read the schedule off
    # ScryGardener's own constants and sum the geometric series instead.
    _gard = src("contracts/src/ScryGardener.sol")
    _mp = re.search(r"HALVING_PERIOD\s*=\s*(\d+)\s*\*\s*365 days", _gard)
    _mh = re.search(r"HALVINGS\s*=\s*(\d+)", _gard)
    _run_years = int(_mp.group(1)) * int(_mh.group(1)) if (_mp and _mh) else 0
    # total = era0_per_yr * period_years * (2 - 2^-(N-1))
    _lifetime = (_per_yr * int(_mp.group(1)) * (2 - 2 ** -(int(_mh.group(1)) - 1))
                 if (_mp and _mh and _per_yr) else 0)
    # pools + Garden seed + the drop + powder. The powder is the only term here
    # that is a KNOB, so it is read rather than typed: it moved 24,000 ->
    # 240,000 on 2026-07-27 (operator, "give me 10% not 1% of the coins") and a
    # typed total would have gone stale silently — the exact failure the comment
    # above describes, in the exact message that comment is about.
    _pm = re.search(r'vm\.envOr\("POWDER_MYRRH",\s*uint256\(([\d_]+)e18\)\)', ds)
    _powder_myrrh = int(_pm.group(1).replace("_", "")) if _pm else 0
    # ...AND SO ARE THE POOLS, since 2026-07-28. This read `2_400_000 + ...`
    # with the pool term TYPED, directly under a comment explaining why the
    # powder must not be. Deepening the pools 80M -> 100M moved the MYRRH side
    # 400,000 -> 470,000 and this line kept reporting the old genesis float, so
    # the headroom it published was 70,000 too generous — against a floor with
    # only ~1,000,000 of room. Read from pool_seeds, which reads the script that
    # actually broadcasts.
    try:
        sys.path.insert(0, str(REPO))
        import pool_seeds as _ps
        _seeds = _ps.seeds()
        _pool_myrrh = _seeds["MYRRH"]["coin"] + _seeds["MYRRH_OBOL"]["myrrh"]
    except Exception:                                    # noqa: BLE001
        _pool_myrrh = None
    GENESIS_MYRRH_FLOAT = (int(_pool_myrrh) + 20_000 + 2_400 + _powder_myrrh
                           if _pool_myrrh else None)
    check("the MYRRH genesis float is derived from the seeds, not typed",
          GENESIS_MYRRH_FLOAT is not None,
          f"{GENESIS_MYRRH_FLOAT:,} = {int(_pool_myrrh):,} pooled + 20,000 Garden "
          f"+ 2,400 drop + {_powder_myrrh:,} powder" if _pool_myrrh else
          "pool_seeds would not import — the headroom below cannot be trusted",
          "every term here is a knob; a typed pool float published 70,000 of "
          "headroom that did not exist when the pools were deepened")
    if GENESIS_MYRRH_FLOAT is None:
        GENESIS_MYRRH_FLOAT = 0
    obol_cap = re.search(r'vm\.envOr\("OBOL_CAP",\s*uint256\(([0-9_e]+)\)\)', ds)
    myrrh_cap = re.search(r'vm\.envOr\("MYRRH_CAP",\s*uint256\(([0-9_e]+)\)\)', ds)
    o = obol_cap.group(1).replace("_", "") if obol_cap else None
    m_ = myrrh_cap.group(1).replace("_", "") if myrrh_cap else None
    check("OBOL ships UNCAPPED, and that is not an oversight",
          o == "0",
          "OBOL_CAP default = 0 (elastic)" if o == "0" else
          f"OBOL_CAP default = {o} — a cap on OBOL is a countdown, because "
          f"burns never refund mint budget",
          "OBOL net-mints by design (56-77% sink coverage). A cap lands as "
          "'play stops paying', with no fix that does not strand holders")
    check("MYRRH ships capped at 21,000,000 (operator, 2026-07-27)",
          m_ == "21000000e18",
          (f"MYRRH_CAP default = 21,000,000 — Bitcoin's number, and a Bitcoin "
           f"schedule: era-0 {_per_yr / 365:,.0f}/day, halving every "
           f"{_mp.group(1)} years, stopping dead at {_run_years}. The whole run "
           f"emits ~{_lifetime:,.0f} above the ~{GENESIS_MYRRH_FLOAT:,} genesis "
           f"float, so ~{21_000_000 - GENESIS_MYRRH_FLOAT - _lifetime:,.0f} is "
           f"NEVER MINTED"
           if _lifetime else "MYRRH_CAP default = 21,000,000 (schedule unreadable)")
          if m_ == "21000000e18" else
          f"MYRRH_CAP default = {m_}, not 21000000e18",
          "immutable at deploy; MYRRH has one tap (the Garden) so the budget "
          "is not a constraint, it is a posted scarcity claim anyone can read "
          "off `cap()`")


# ── 6. the deploy scripts nobody runs ────────────────────────────────────────
def gate_fork_mandates_are_wired():
    """A fork suite OUTSIDE the arm gate proves nothing on broadcast day.

    Fork tests are the testnet replacement: they are the only thing that touches
    real NPM/pool bytecode and a real tick walk. They are also `RH_FORK_URL`-
    guarded, so a plain `forge test` SKIP-PASSES them — the suite reports green
    while the mandates do nothing (the tell is gas: ~5k skipping vs ~3.5M live).
    `deploy_town.sh::fork_mandates_green` is what forces them to run, and it
    names its targets in TWO hand-maintained places: a `--match-contract` filter
    and a `mandates=(...)` list of test function names.

    That pair drifted once already: the filter read `OrchardFork|SeedSpoilsFork`
    long after `ScryGachaFork` existed, so four real fork tests could pass
    forever without the broadcast gate ever running them. It was fixed by hand,
    which fixes today and nothing else. This gate makes the NEXT one red:
    enumerate the fork suites on disk and prove every contract and every
    `test_fork_*` is named in the runner."""
    dt = src("contracts/deploy_town.sh")
    forks = sorted((REPO / "contracts" / "test").glob("*Fork*.t.sol"))
    m_filter = re.search(r"--match-contract\s+'([^']+)'", dt)
    # forge treats --match-contract as a REGEX, so the alternative `OrchardFork`
    # already covers `OrchardForkTest`. Match the way the runner matches; a
    # stricter equality check here fails on a correctly-wired suite, which is a
    # gate that cries wolf and gets deleted.
    alts = [a for a in (m_filter.group(1) if m_filter else "").split("|") if a]

    missing_c, missing_t = [], []
    for p in forks:
        body = p.read_text(encoding="utf-8")
        for c in re.findall(r"^contract\s+(\w*Fork\w*)\s", body, re.M):
            if not any(re.search(a, c) for a in alts):
                missing_c.append(f"{c} ({p.name})")
        for t in re.findall(r"^\s*function\s+(test_fork_\w+)\s*\(", body, re.M):
            if t not in dt:
                missing_t.append(f"{t} ({p.name})")

    check("every fork suite on disk is inside the broadcast gate",
          bool(forks) and not missing_c and not missing_t,
          f"{len(forks)} fork suites, all named in deploy_town.sh's "
          f"--match-contract filter and mandates list"
          if forks and not missing_c and not missing_t else
          ("no *Fork*.t.sol found — the testnet replacement is gone" if not forks
           else "NOT IN THE ARM GATE — "
                + " · ".join(missing_c + missing_t)),
          "add it to fork_mandates_green's --match-contract filter AND its "
          "mandates=() list. A fork test the arm gate never runs is a test that "
          "does not exist on broadcast day")


def gate_deploy_scripts():
    """Only 1 of 20 deploy scripts is imported by any test suite (audit
    2026-07-25), so `forge test` green says almost nothing about the code that
    actually broadcasts. These are cheap source checks over ALL of them."""
    scripts = sorted((REPO / "contracts" / "script").glob("*.s.sol"))
    check("deploy scripts are present", bool(scripts),
          f"{len(scripts)} scripts under contracts/script/", "")

    # F6: vm.envOr(primary, vm.env*(fallback)) — Solidity evaluates arguments
    # eagerly, so the "fallback" runs unconditionally and its key becomes
    # MANDATORY. DeployGardener shipped this for weeks; its documented runbook
    # simply reverted. Nested vm.env* inside a vm.envOr default is banned.
    eager = []
    for p in scripts:
        body = strip_comments(p.read_text())  # or the doc explaining the ban trips it
        for m in re.finditer(r"vm\.envOr\(", body):
            # walk the balanced argument list of this envOr call
            depth, arg = 1, []
            for ch in body[m.end():]:
                if ch == "(":
                    depth += 1
                elif ch == ")":
                    depth -= 1
                    if depth == 0:
                        break
                arg.append(ch)
            if re.search(r"vm\.env(Address|Uint|Int|Bytes32|String|Bool)\s*\(", "".join(arg)):
                eager.append(p.name)
    check("no deploy script hides a mandatory env var inside vm.envOr",
          not eager,
          "clean" if not eager else f"eager fallback in: {', '.join(sorted(set(eager)))}",
          "vm.envOr's default is evaluated EAGERLY. Resolve sequentially instead "
          "— see DeployGardener.envAddrOr for the shape to copy.")

    # A broadcast script that never says PRIVATE_KEY is not a broadcast script;
    # one that hardcodes a key is a catastrophe. Cheap to check, so check.
    leaked = [p.name for p in scripts
              if re.search(r"(0x[0-9a-fA-F]{64})", p.read_text())]
    check("no deploy script carries a literal 32-byte secret", not leaked,
          "clean" if not leaked else f"64-hex literal in: {', '.join(leaked)}",
          "keys come from the environment, never from the repo")


# ── 7. custody: the launch must not quietly start holding keys ───────────────
def _phase_block(dt, name):
    """The body of one `deploy_town.sh` phase, so a check reads THAT phase and
    not a string that happens to appear elsewhere in a 1900-line file."""
    i = dt.find(f"\n{name})")
    if i < 0:
        return ""
    j = dt.find("\n    ;;", i)
    return dt[i:j] if j > 0 else dt[i:]


def gate_posted_destinations():
    """THE ADDRESSES THAT MUST NOT BE THE DEPLOYER, AND MUST NOT BE RETYPED.

    Operator, 2026-07-29: *"set anything we are not setting to deployer to the
    dev wallet the same as powder."* Three fields answer to that — `SCRY_OPS`
    (the ops fee sink), `LP_RECIPIENT` (the deed to 100,000,000 SCRY of pool
    depth), and `POWDER_TO`, which was already pointed there on 07-27.

    Two failure modes, and this gate exists for the second one.

    The obvious one is a silent fall-through: both forge scripts default their
    field to `vm.addr(pk)`, so an unset variable posts the DEPLOYER — a hot key
    whose whole purpose is to broadcast once and be retired — as a permanent
    public fee recipient and as the holder of the LP deed. `deploy_town.sh` now
    supplies `$DEV_WALLET` and prints that it did.

    The one worth a gate is the typo. That address is 42 characters and, before
    this, it appeared as a literal in `deploy_town.sh` AND in
    `snapshots/house-wallets.json` (where it is the drop-exclusion key) with
    nothing comparing them. One transposed character in the driver and the fee
    sink and the LP deed land on an address nobody holds — while the drop
    exclusion, reading the other copy, still works perfectly and hides it. So:
    ONE literal, checked against the file that already had to be right.
    """
    dt = src_required("contracts/deploy_town.sh",
                      "it is where the posted destination lives")
    m = re.search(r"^DEV_WALLET=(0x[0-9a-fA-F]{40})", dt, re.M)
    dev = m.group(1) if m else ""
    check("the posted dev wallet is declared once, in the driver", bool(dev),
          dev or "no `DEV_WALLET=0x…` line in deploy_town.sh",
          "every field that must not be the deployer reads this one literal")

    # `_json` returns {} for an unreadable file, so an EMPTY `owned` must fail
    # loudly rather than read as "not one of ours" — the never-raises-reader
    # trap, which this gate walked straight into on its first run by calling a
    # helper that does not exist and letting `except Exception` swallow it.
    hw = _json(REPO / "snapshots" / "house-wallets.json")
    owned = {k.lower() for k in (hw.get("wallets") or {})}
    check("and it is a wallet house-wallets.json says we hold",
          bool(dev) and bool(owned) and dev.lower() in owned,
          f"{dev} is in house-wallets.wallets ({len(owned)} listed)"
          if dev and owned and dev.lower() in owned
          else (f"{dev or '(none)'} is NOT in house-wallets.json" if owned
                else "house-wallets.json unreadable or has no `wallets` — cannot verify"),
          "the fee sink and the LP deed go here; a typo lands them on an address "
          "nobody holds, and the drop exclusion reading the other copy would not notice")

    # And the driver must still refuse the thing the default exists to prevent.
    check("an ops sink pointed AT the deployer is refused",
          "SCRY_OPS is the DEPLOYER" in dt,
          "bank refuses it by name" if "SCRY_OPS is the DEPLOYER" in dt
          else "no refusal found",
          "a posted default is not a guard if an explicit wrong value still passes")

    # The one-way acts must NOT have acquired a default.
    one_way_defaulted = re.search(r'(MINTER_NEXT|STEWARD_NEXT)="?\$\{?(MINTER_NEXT|STEWARD_NEXT)[^}]*:-\$DEV_WALLET', dt)
    check("the one-way destinations still have NO default", not one_way_defaulted,
          "rotate/steward still demand an explicit address"
          if not one_way_defaulted else "a default crept into a one-way act",
          "setMinter and transferSteward cannot be undone; a destination that "
          "arrives by default is a destination nobody read")


def gate_closing_act():
    """WHICH PHASE RETIRES UNLIMITED MINT DEPENDS ON THE ORDER YOU RAN, and the
    launch has to know that before broadcast day, not during it.

    `rotate` retires `SpoilsToken.minter`. `gardener` and `granary` move that
    slot into a granary on their way past (DeployGardener.s.sol:175,
    DeployGranary.s.sol:54), so on a depth-first launch the deployer holds
    neither minter role and `rotate --arm` reverts on `minter only`. The seat it
    holds instead is `ScryGranary.steward`, whose `stewardMint` is UNCAPPED —
    the same unlimited mint, one door over. The closing act on that ordering is
    `./deploy_town.sh steward`.

    That was worked out by hand on 2026-07-28 from four files, and the fix for
    it sat UNCOMMITTED in the tree for a day — 262 lines of launch-path code one
    `git checkout` from gone, with no gate anywhere that would have noticed. So
    this gate holds the three pieces together: both closing phases exist, the
    wrong one refuses by naming the right one, and the mandate that proves the
    whole story is on disk. Remove any one and this reds.

    It also pins the PREMISE. If a future edit stops the depth scripts from
    handing the minter to the granary, the ordering story changes and every
    sentence above becomes wrong — so the handoff itself is checked here, where
    the conclusion lives, rather than being assumed by three documents."""
    dt = src_required("contracts/deploy_town.sh",
                      "every check below asserts something about its contents, so an "
                      "unreadable file would pass them vacuously")
    rot, stew = _phase_block(dt, "rotate"), _phase_block(dt, "steward")
    header = dt[:4000]

    check("both closing acts exist as phases", bool(rot) and bool(stew),
          f"rotate {'ok' if rot else 'MISSING'} · steward {'ok' if stew else 'MISSING'}",
          "one ordering per phase: rotate when the depth phases run after the "
          "closing act, steward when they run before it")
    check("both are in the usage header",
          "deploy_town.sh rotate" in header and "deploy_town.sh steward" in header,
          "listed" if "deploy_town.sh steward" in header else "steward is not listed",
          "a phase nobody can find from the header is prose with a case label")

    # The refusal, not the revert. `cast send` already stops a wrong rotate —
    # at the END of a long sitting, reading as a broken key.
    check("rotate refuses the depth-first case by name",
          "./deploy_town.sh steward" in rot and "minter()(address)" in rot,
          "reads minter() first and points at `steward`" if rot else "no rotate block",
          "rotate must read the seat off the chain BEFORE the send and, when the "
          "slot already sits in a granary, name the phase that does the job")

    # The steward phase's teeth: the two preconditions with no equivalent in
    # rotate. setGrant is onlySteward, so an ungranted organ is dead forever.
    check("steward proves the seat is worth retiring, and every grant is set",
          "availableToday" in stew and "transferSteward(address)" in stew,
          "grant + minter preconditions present" if stew else "no steward block",
          "an organ deployed and never granted becomes a permanently dry drip "
          "the moment the seat moves — the phase must refuse on it")

    mandate = src_required("contracts/test/AuthorityRetirement.t.sol",
                           "it is the only executable statement of who can mint after "
                           "the launch; absent, the story is prose again")
    # The two-step steward (2026-07-28) SPLIT the old single
    # `test_transferSteward_closes_every_door_the_deployer_had` into the two
    # halves the handoff now has — proposed-but-unclaimed, and claimed — which
    # is strictly better coverage. This list kept naming the retired name, so
    # the gate read RED over an improvement and `./deploy_town.sh preflight`
    # aborted the launch on it. Both halves are named now, because "the seat
    # moved" and "the seat was only offered" are different facts about who can
    # mint, and the whole point of this check is that the story stays
    # executable. (Found 2026-07-29 taking the drop-one snapshot.)
    want = ("test_depth_phases_take_the_minter_so_rotate_cannot_run",
            "test_the_unlimited_mint_moved_it_did_not_retire",
            "test_a_proposed_but_unclaimed_seat_still_drives_the_town",
            "test_the_claimed_handoff_closes_every_door_the_deployer_had",
            "test_depth_after_the_closing_act_leaves_rotate_working")
    gone = [t for t in want if t not in mandate]
    check("the authority mandate covers both orderings", bool(mandate) and not gone,
          f"{len(want)}/{len(want)} present" if mandate and not gone
          else f"missing: {', '.join(gone) or 'file'}",
          "both orderings stay covered, or the next reorder rediscovers this on "
          "broadcast day")

    # THE PREMISE. Everything above is only true while the depth scripts still
    # perform the handoff.
    handoff = []
    for f, coin in (("contracts/script/DeployGardener.s.sol", "MYRRH"),
                    ("contracts/script/DeployGranary.s.sol", "OBOL")):
        s = strip_comments(src(f))
        if not re.search(r"\.setMinter\(\s*address\(granary\)\s*\)", s):
            handoff.append(f"{coin} ({f.split('/')[-1]})")
    check("the depth phases still hand the minter to the granary", not handoff,
          "both scripts do the handoff" if not handoff else f"no handoff in: {', '.join(handoff)}",
          "if this stops being true the ordering story changes — re-read the "
          "rotate/steward split and this gate's docstring before editing either")


def gate_custody():
    a = src("meter/agency.py")
    cap = re.search(r'SCRY_AGENCY_FAUCET_CAP_USD",\s*"([^"]+)"', a)
    check("hosted custody still cap 0 (no wallets exist)", cap and cap.group(1) == "0",
          f"faucet cap = {cap.group(1) if cap else '?'}",
          "cap > 0 is its own separately spoken operator act (CLAUDE.md invariant 7) — "
          "it must not ride along on a launch")
    # The one NEGATIVE gate in the file, so the one that could fail open: an
    # absent server.py made `"X" not in ""` true and the gate PASSED.
    server = src_required("meter/server.py",
                          "it asserts the ABSENCE of a pattern, so an unreadable "
                          "file would pass it vacuously")
    check("no free signed reads", bool(server) and "SCRY_HOLD_ENABLED" not in server,
          "the hold-to-unlock gate is gone" if server else "meter/server.py unreadable",
          "a signed read is the product; it is never given away for a balance")


DECISION_RE = re.compile(r"^Operator, (\d{4}-\d{2}-\d{2}):", re.M)
DECISION_LOOKBACK_DAYS = 30
# Any hex run long enough to be a commit citation. 7 is git's own floor for an
# abbreviation, and it is what most rows on the ledger carry.
HEX_RUN_RE = re.compile(r"\b[0-9a-f]{7,40}\b")


def _git(args):
    """Local git, never raises. A repo state this cannot read is reported as a
    skip by the caller, never as a pass — see the note in `src_required`."""
    try:
        r = subprocess.run(["git", *args], cwd=str(REPO), capture_output=True,
                           text=True, timeout=20)
        return r.stdout if r.returncode == 0 else None
    except Exception:
        return None


def gate_sentences_record_decisions():
    """A commit may not claim an operator decision the ledger has never heard of.

    This is the miss that cost a day of re-derivation. Commit `9c918cf` opened
    *"Operator, 2026-07-29: pick ScryTill"* and changed eight files — none of
    them `SENTENCES.md`. The decision was real, findable, and invisible to every
    session that (correctly) went to the ledger first, so it got re-asked three
    times.

    **A date-level check would NOT have caught it, and that is why this one is
    shaped the way it is.** 2026-07-29 already carried a row — the dev-wallet
    sentence — so *"does this date appear"* was already true while the Till
    decision was still missing. The discriminator has to be the commit, not
    the day.

    So: a commit whose message declares a decision must either **write the
    ledger itself**, or **be cited by it**. The second door is not a loophole —
    it is how a decision recorded a day late still passes, and the citation is
    worth having on its own, because a row that names the commit it came from
    can be audited back to the words that were actually said.

    Bounded to `DECISION_LOOKBACK_DAYS` because the convention post-dates the
    early history, and a gate that reds over commits nobody can amend is a gate
    people learn to ignore. Only the line-initial form counts: this repo cites
    past decisions parenthetically (*"(operator, 2026-07-26)"*) in almost every
    message, and flagging those would make the check noise."""
    ledger = src_required("SENTENCES.md",
                          "it decides whether a declared decision was recorded, "
                          "and an empty read would clear every commit at once")
    if not ledger:
        return

    # ⚠ THE CITATION IS MATCHED BY PREFIX, AND AN EXACT MATCH IS A BUG THAT
    # FIRES ON ITS OWN. git abbreviates `%h` to whatever keeps a sha unique in
    # THIS repo, and that length grows as the repo does: every row on the
    # ledger cites 7 characters because 7 is what `%h` printed the day it was
    # written, and `%h` prints 8 now. An `in` test against the full abbreviation
    # therefore stopped matching all ten citations AT ONCE — the gate went red
    # claiming the decisions were never recorded, when what actually changed was
    # git's abbreviation length. It cost a red preflight from 2026-08-09 to
    # 2026-08-12 and the message sent every reader to look for missing rows that
    # were sitting right there.
    #
    # So: take the FULL sha and ask whether the ledger cites any prefix of it.
    # That is what a citation MEANS, it is exact in both directions (a 7-char
    # citation cannot match a different commit unless git itself would have
    # collided), and it never rots again when the abbreviation grows to 9.
    log = _git(["log", f"--since={DECISION_LOOKBACK_DAYS}.days", "--no-merges",
                "--format=%h%x1f%H%x1f%s%x1f%b%x1e"])
    if log is None:
        check("every declared operator decision reaches SENTENCES.md", True,
              "git log unavailable — skipped (not a launch blocker)",
              "")
        return

    cited = set(HEX_RUN_RE.findall(ledger))

    unrecorded = []
    for entry in (e for e in log.split("\x1e") if e.strip()):
        parts = entry.strip().split("\x1f")
        if len(parts) < 4:
            continue
        sha, full, subject, body = parts[0], parts[1], parts[2], parts[3]
        if not DECISION_RE.search(body) and not DECISION_RE.search(subject):
            continue
        touched = _git(["show", "--name-only", "--format=", sha]) or ""
        if "SENTENCES.md" in touched or any(full.startswith(c) for c in cited):
            continue
        unrecorded.append((sha, subject))

    check("every declared operator decision reaches SENTENCES.md", not unrecorded,
          "all declared decisions are on the ledger" if not unrecorded else
          "; ".join(f"{s} '{sub[:52]}'" for s, sub in unrecorded),
          "append the compressed row (headline, the operator's own words, a "
          "pointer) to docs/SENTENCES.md — or, if it is already there, cite the "
          "commit hash in that row so the decision can be traced to what was said")


# The two dialects this repo publishes, and the one thing they share: the tree.
# `seat_roots.py` imports `_leaf`, `_levels` and `verify_proof` straight out of
# `claim_plan`, so an access tree is the SAME sorted-pair keccak walk over the
# same 52-byte leaf. Only the second word differs, and with it the total that
# gets armed:
#
#   airdrop  claims[w].amount_wei · total_wei      -> ScryHarvest's sweep floor
#   access   claims[w].allowance  · seats_promised -> ScrySeat's door cap
#
# ⚠ AN UNKNOWN SHAPE IS RED, NEVER SKIPPED. This gate went red the day the
# first access tree landed because it knew one dialect and globbed both — and
# the tempting fix, skipping what it cannot parse, is the manufactured green
# this repo has shipped before. A file it cannot grade is a file whose total
# nobody checked.
_MERKLE_SHAPES = (
    # (the field on each claim, the artifact's total, what that total arms)
    ("amount_wei", "total_wei", "the sweep floor"),
    ("allowance", "seats_promised", "the door's seat cap"),
)

# ⚠ VERIFYING EVERY PROOF STOPPED BEING FREE. The walk is O(n log n) keccak in
# pure Python: the 55-recipient drop trees finish instantly, while door 1's
# 57,010-wallet tree costs ~9 MINUTES for the full sweep on top of a 63s root
# recompute (measured 2026-08-12). A gate nobody waits for is a gate nobody
# runs, so above this size the proofs are SAMPLED — evenly spaced, so the walk
# reaches both ends where the odd-node carry lives — and the count checked is
# PRINTED rather than implied. The root recompute is never sampled: it reads
# every leaf, and it is the number that gets armed.
_PROOF_SAMPLE_MAX = 500
_PROOF_SAMPLE_N = 200


def gate_merkle_totals_are_derived():
    """§M4's off-chain half — the half that IS closable.

    `ScryHarvest.postRoot` takes `totalCommitted_` from the operator and cannot
    check it against the tree: a merkle root commits to its leaves, never to
    their sum, so there is no on-chain arithmetic that catches a wrong total.
    That stays true, the contract's header says so, and the SWEEP_DELAY window
    is the structural consolation — which, note, does nothing on a FIRST root,
    where `priorOwed` is still zero (pinned in test/DeployClaimHarvest.t.sol).
    `ScrySeat` has the same hole in the same place: a root commits to its
    leaves, never to how many seats they add up to.

    What is fixable is where the number COMES FROM. Both builders derive their
    total by summing their own leaves, so an operator who pastes that field
    into `CLAIM_*_TOTAL` or a door's cap is asserting a number nobody typed.
    This gate makes that true of the artifact rather than assumed: the root
    must recompute from the claims it ships, the total must equal their sum,
    and the proofs must verify in the dialect the contract implements — both
    dialects, since the trees are the same walk over a different second word.

    A red here means the number about to be welded into a broadcast disagrees
    with the tree it claims to describe.
    """
    sys.path.insert(0, str(REPO))
    try:
        from meter import claim_plan as cp
    except Exception as e:  # noqa: BLE001
        return check("published merkle totals are derived, not typed", False, f"{e}", "")

    arts = sorted((REPO / "snapshots").glob("*.merkle.json"))
    if not arts:
        return check("published merkle totals are derived, not typed", True,
                     "no merkle artifact on disk yet — this gate arms with the "
                     "first `claim_plan.py merkle` output, which is what feeds "
                     "CLAIM_*_TOTAL", "")
    bad, notes = [], []
    for p in arts:
        art = _json(p)
        claims = art.get("claims") or {}
        field = total_field = arms = None
        for _f, _tf, _arms in _MERKLE_SHAPES:
            if claims and all(_f in c for c in claims.values()):
                field, total_field, arms = _f, _tf, _arms
                break
        if field is None:
            seen = sorted({k for c in list(claims.values())[:1] for k in c})
            bad.append(f"{p.name}: unknown claim shape {seen or '(no claims)'} — "
                       f"expected one of {[s[0] for s in _MERKLE_SHAPES]}")
            continue
        try:
            entries = sorted((w, int(c[field])) for w, c in claims.items())
            leaves = [cp._leaf(w, v) for w, v in entries]
            root = "0x" + cp._levels(leaves)[-1][0].hex() if leaves else None
        except Exception as e:  # noqa: BLE001
            bad.append(f"{p.name}: unreadable ({e})")
            continue
        if root != art.get("root"):
            bad.append(f"{p.name}: root does not recompute from its own claims")
        summed = sum(v for _, v in entries)
        if summed != int(art.get(total_field, -1)):
            bad.append(f"{p.name}: {total_field} {art.get(total_field)} != "
                       f"sum of leaves {summed}")
        if int(art.get("n", -1)) != len(entries):
            bad.append(f"{p.name}: n {art.get('n')} != {len(entries)} claims")
        if len(entries) <= _PROOF_SAMPLE_MAX:
            idxs = list(range(len(entries)))
        else:
            step = (len(entries) - 1) / (_PROOF_SAMPLE_N - 1)
            idxs = sorted({int(round(i * step)) for i in range(_PROOF_SAMPLE_N)})
        for i in idxs:
            w, v = entries[i]
            if not cp.verify_proof(w, v, claims[w]["proof"], art["root"]):
                bad.append(f"{p.name}: {w}'s proof does not verify")
                break
            # The leaf binds BOTH words, and that is the property worth testing:
            # a proof that still walks to the root with a bumped value commits
            # to the wallet alone, which makes every amount claimable.
            if cp.verify_proof(w, v + 1, claims[w]["proof"], art["root"]):
                bad.append(f"{p.name}: {w}'s proof accepts a tampered {field}")
                break
        notes.append(f"{p.name}: {len(entries)} claims, {total_field}={summed} "
                     f"({arms}), {len(idxs)} proof(s) verified"
                     f"{' (sampled)' if len(idxs) < len(entries) else ''}")
    check("published merkle totals are derived, not typed", not bad,
          "; ".join(notes) if not bad else "; ".join(bad),
          "the total is what the contract's door cap or sweep floor is armed "
          "to — it must come from the tree, never from a keyboard")


def gate_holder_faucet():
    """SECURITY-TODO §0.5, the one open money-path row at launch time.

    `tokens.verify_holder` reads `balanceOf` at `latest` — no snapshot block, no
    holding period, nothing binding the holding to the claim, and `_claim_reserve`
    is one-per-WALLET rather than one-per-HOLDING. So ONE stack of SCRY, moved
    from wallet to wallet, drains the whole genesis claim budget. It ships
    disarmed and that is the entire mitigation, which makes the shipped default
    a launch-blocking invariant rather than a preference: re-arming it needs the
    claim bound to a snapshot block FIRST.

    Two things are checked, because they fail differently. The default in the
    code is the one that protects a fresh deploy; the live environment is the
    one that protects THIS run, and a shell that happens to carry the variable
    is exactly how a disarmed thing gets armed by accident.
    """
    t = src_required("meter/tokens.py",
                     "the shipped default IS the mitigation, so an unreadable "
                     "file must not pass for one")
    m = re.search(r'CLAIM_ENABLED\s*=\s*os\.getenv\(\s*"SCRY_SPOILS_CLAIM",\s*"([^"]*)"\s*\)\s*==\s*"1"', t or "")
    check("the holder-claim faucet ships disarmed (§0.5)", bool(m) and m.group(1) != "1",
          f'SCRY_SPOILS_CLAIM defaults to "{m.group(1)}"' if m
          else "could not find CLAIM_ENABLED's default in meter/tokens.py",
          "§0.5 is open: the claim reads balanceOf at `latest`, so one stack of "
          "SCRY moved between wallets claims the budget repeatedly. Bind the "
          "claim to a snapshot block before this default may change")
    env = os.environ.get("SCRY_SPOILS_CLAIM", "")
    check("and it is not armed in this environment", env != "1",
          "SCRY_SPOILS_CLAIM unset" if env == "" else f"SCRY_SPOILS_CLAIM={env}",
          "unset it, or bind the claim to a snapshot block first — an armed "
          "faucet at launch pays the same SCRY out once per wallet it visits")


# ── the Arbitrum clock law (RUNBOOK.md §0c) ──────────────────────────────────
# Chain 4663 is Arbitrum Nitro, where Solidity's `block.number` is the PARENT
# chain's height — one per ~12s against the L2's 101ms. Measured 2026-07-26:
# 25,620,709 vs an L2 height of 20,319,770, advancing 0.08 blocks/s vs 9.9.
_CLOCK_FORBIDDEN = {
    "block.number": "the PARENT chain's height here (~12s/block); pace in block.timestamp "
                    "or read ArbSys(0x64).arbBlockNumber()",
    "blockhash(": "Arbitrum documents the opcode as unfit for randomness; use "
                  "ArbSys(0x64).arbBlockHash, which returns the exact L2 hash",
    "block.difficulty": "no meaningful value on Nitro",
    "prevrandao": "no meaningful value on Nitro",
}


def gate_arbitrum_clock():
    """`RUNBOOK.md` §0c as a law rather than a dated note.

    `ScryGacha` shipped pacing a "~10 second" reveal on `block.number`, which is
    ~12s per block here: the reveal was really ~20 minutes and its pool stayed
    locked for all of it. **36 unit tests and 4 live-fork mandates stayed green**,
    because every test drove the clock with `vm.roll` and a mock cannot disagree
    with the chain it stands in for. Nothing but a source-level gate catches the
    next one, so this is that gate.
    """
    hits = []
    for p in sorted((REPO / "contracts/src").glob("**/*.sol")):
        # Comments stripped on purpose: the contracts DISCUSS block.number at
        # length — they must, it is the trap — and a gate that cannot tell prose
        # from code would either fire forever or get deleted.
        code = strip_comments(p.read_text())
        for needle, why in _CLOCK_FORBIDDEN.items():
            if needle in code:
                hits.append(f"{p.name} uses {needle} — {why}")
    check("no contract paces on block.number (Arbitrum Nitro, RUNBOOK 0c)",
          not hits, "; ".join(hits) if hits else
          f"{len(list((REPO / 'contracts/src').glob('**/*.sol')))} contracts clean; "
          "everything time-based uses block.timestamp, which is accurate here",
          "pace durations in block.timestamp, or read ArbSys(0x64).arbBlockNumber() for "
          "true L2 height. If a contract genuinely needs the parent chain's height, say so "
          "here deliberately — do not widen the pattern to make a red go away")

    # The other half: a suite that exercises a clock-reading contract without
    # etching the precompile is not testing the clock, it is testing nothing.
    # Foundry implements no ArbOS precompile, so 0x64 is dead code on a fork.
    missing = []
    for p in sorted((REPO / "contracts/test").glob("*.t.sol")):
        t = p.read_text()
        if "new ScryGacha(" in t and "MockArbSys" not in t:
            missing.append(p.name)
    check("every suite that deploys a clock-reading contract etches MockArbSys",
          not missing, "ok" if not missing else f"no MockArbSys in: {', '.join(missing)}",
          "vm.etch(0x64, address(new MockArbSys()).code) in setUp, and set its fields "
          "explicitly — vm.etch runs no constructor, so a field initializer reads 0")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    # A gate that reads a moved/missing doc reports a red for the wrong reason.
    # Say so once, up front, instead of leaving four confusing failures.
    missing = [d for d in ("TOKENOMICS.md", "FARMING.md", "LAUNCH-DECISIONS.md", "POOLS.md")
               if not doc_exists(d)]
    check("the docs the gates read are readable", not missing,
          "all present" if not missing else f"unreadable: {', '.join(missing)}",
          "these live at the repo root or under docs/; if a reorg is mid-flight, "
          "finish it before trusting any doc-derived check below")

    gate_tokenomics()
    gate_reliquary_odds()
    snap = gate_snapshot()
    gate_drop(snap)
    gate_powder_tracks_the_drop(snap)
    gate_pool_seed_matches_decision()
    gate_pool_seed_single_source()
    gate_season_pot_cap()
    gate_table_pot_cap()
    gate_every_emission_knob_is_recorded()
    gate_tvl_is_never_summed()
    gate_published_plans_reproduce()
    gate_house_never_claims()
    gate_drop_claim_isolation()
    gate_pool_float_accounting()
    gate_farm_reward()
    gate_token_identity()
    gate_deploy_scripts()
    gate_fork_mandates_are_wired()
    gate_arbitrum_clock()
    gate_merkle_totals_are_derived()
    gate_closing_act()
    gate_posted_destinations()
    gate_custody()
    gate_holder_faucet()
    gate_sentences_record_decisions()

    if a.json:
        print(json.dumps(RESULTS, indent=1))
    else:
        print("\n== preflight: the off-chain gate =========================================")
        for r in RESULTS:
            print(f"  {'PASS' if r['ok'] else 'FAIL'}  {r['check']}")
            print(f"        {r['detail']}")
            if not r["ok"] and r["fix"]:
                print(f"        fix: {r['fix']}")
        bad = [r for r in RESULTS if not r["ok"]]
        print()
        if bad:
            print(f"  {len(bad)} of {len(RESULTS)} checks FAILED — the ledger that feeds "
                  "the chain is not ready.")
        else:
            print(f"  all {len(RESULTS)} checks pass — the off-chain half is ready to be "
                  "given a chain.")
    sys.exit(1 if any(not r["ok"] for r in RESULTS) else 0)


if __name__ == "__main__":
    main()
