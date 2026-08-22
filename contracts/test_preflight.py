#!/usr/bin/env python3
"""Laws for `preflight.py` — the off-chain gate before a launch broadcast.

Bare python: stdlib only, no network, no keys. A gate whose own test needs a
venv is a gate whose test does not run.

These cover the three ways this file could report READY while checking nothing,
all from `SECURITY-TODO.md` §doc pipeline + contract tooling. Each was confirmed
RED against the pre-fix `preflight.py` first:

  - `check("no free signed reads", "SCRY_HOLD_ENABLED" not in src("meter/server.py"))`
    — `src()` returns "" for a missing file and `"X" not in ""` is True, so
    renaming or moving server.py made the gate PASS. It is the only negative
    check in the file, and it gates whether a signed read can be handed out for
    a balance.
  - `src()`'s `docs/<basename>` fallback applied to ANY path, so
    `src("meter/server.py")` would read `docs/server.py` — a gate silently
    reading a different file than the one it names.
  - `gate_snapshot` trusted `snap["verified"]`, a field the same JSON supplies
    about itself, and measured freshness by `st_mtime`, which a `touch` (or a
    fresh clone, or a `git checkout`) resets.
"""
from __future__ import annotations

import json
import os
import re
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preflight as P                    # noqa: E402

PASS = FAIL = 0


def check(name: str, cond: bool, extra: str = "") -> None:
    global PASS, FAIL
    if cond:
        PASS += 1
    else:
        FAIL += 1
        print(f"  FAIL: {name}" + (f" — {extra}" if extra else ""))


def results(fn, *a):
    """Run a gate in isolation and return {check name: ok}."""
    P.RESULTS.clear()
    try:
        fn(*a)
    finally:
        out = {r["check"]: r["ok"] for r in P.RESULTS}
        P.RESULTS.clear()
    return out


class Repo:
    """A throwaway REPO root wired into preflight's module global."""

    def __init__(self):
        self.dir = Path(tempfile.mkdtemp(prefix="scry-preflight-"))
        self._saved = P.REPO
        P.REPO = self.dir

    def write(self, rel, text):
        p = self.dir / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text if isinstance(text, str) else json.dumps(text))
        return p

    def __enter__(self):
        return self

    def __exit__(self, *a):
        P.REPO = self._saved


# --------------------------------------------------------------- the fail-open

def test_negative_gate_cannot_pass_vacuously():
    print("the negative gate")
    with Repo() as r:
        r.write("meter/agency.py", 'SCRY_AGENCY_FAUCET_CAP_USD", "0"')

        # 1. server.py present and clean -> the gate passes, as it always did.
        r.write("meter/server.py", "app = FastAPI()\n")
        got = results(P.gate_custody)
        check("a clean server.py passes the gate", got.get("no free signed reads") is True)
        check("a present server.py needs no readability FAIL",
              "meter/server.py is readable" not in got, str(sorted(got)))

        # 2. server.py present and DIRTY -> the gate fails. The check is real.
        r.write("meter/server.py", 'if os.getenv("SCRY_HOLD_ENABLED"):\n    ...\n')
        got = results(P.gate_custody)
        check("the gate still catches the pattern it exists for",
              got.get("no free signed reads") is False)

        # 3. THE ROW. server.py absent -> must FAIL, not pass on an empty string.
        (r.dir / "meter" / "server.py").unlink()
        got = results(P.gate_custody)
        check("an absent server.py does not pass the gate",
              got.get("no free signed reads") is False,
              f"gate returned {got.get('no free signed reads')!r}")
        check("and the absence is reported as its own failure",
              got.get("meter/server.py is readable") is False, str(sorted(got)))

        # The neighbouring custody gate was already safe; keep it that way.
        (r.dir / "meter" / "agency.py").unlink()
        got = results(P.gate_custody)
        check("the faucet-cap gate also fails safe when its file is gone",
              got.get("hosted custody still cap 0 (no wallets exist)") is False)


def test_src_does_not_cross_read():
    print("src() path resolution")
    with Repo() as r:
        r.write("docs/server.py", "SCRY_HOLD_ENABLED = 1  # a decoy in docs/\n")
        check("a missing .py does NOT fall back to docs/<basename>",
              P.src("meter/server.py") == "", repr(P.src("meter/server.py"))[:80])

        # The fallback still does its actual job: the docs/ reorg.
        r.write("docs/money/POOLS.md", "# pools\n")
        check("a doc named at the root still resolves under docs/",
              P.src("POOLS.md") == "# pools\n", repr(P.src("POOLS.md")))
        r.write("TOKENOMICS.md", "# at the root\n")
        check("a doc at the root still resolves there",
              P.src("TOKENOMICS.md") == "# at the root\n")
        check("a genuinely absent doc is still empty", P.src("NOPE.md") == "")

        # A directory must never read as a file.
        (r.dir / "adir.md").mkdir()
        check("a directory is not a source file", P.src("adir.md") == "")


def test_src_required_reports():
    print("src_required")
    with Repo() as r:
        r.write("a.py", "content\n")
        P.RESULTS.clear()
        check("src_required returns the text", P.src_required("a.py", "why") == "content\n")
        check("a readable file registers no check", not P.RESULTS, str(P.RESULTS))
        P.RESULTS.clear()
        check("src_required returns '' when absent", P.src_required("b.py", "why") == "")
        check("an absent file registers a FAIL",
              len(P.RESULTS) == 1 and P.RESULTS[0]["ok"] is False, str(P.RESULTS))
        P.RESULTS.clear()


# ------------------------------------------------------------- the snapshot

def _snap(**over):
    base = {"n_holders": 3, "block": 19231505,
            "taken_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "verified": True,
            "verification": {"repinned_all": 3, "differed_from_indexer": 0, "errors": []},
            "delegation": {"checked": 1, "delegated_eoas": 1, "real_contracts": 0}}
    base.update(over)
    return base


def test_snapshot_verification_needs_evidence():
    print("snapshot verification")
    with Repo() as r:
        r.write("snapshots/scry-holders.2026-07-25.json", _snap())
        got = results(P.gate_snapshot)
        check("an evidenced snapshot passes",
              got.get("snapshot verified against the chain") is True, str(got))

        # THE ROW: `verified: true` alone is a claim the file makes about itself.
        r.write("snapshots/scry-holders.2026-07-25.json",
                _snap(verification={"repinned_all": 0, "differed_from_indexer": 0,
                                    "errors": []}))
        got = results(P.gate_snapshot)
        check("verified=true with nothing re-pinned does NOT pass",
              got.get("snapshot verified against the chain") is False)

        r.write("snapshots/scry-holders.2026-07-25.json",
                _snap(verification={"repinned_all": 3, "differed_from_indexer": 0,
                                    "errors": ["rpc timeout on 0xabc"]}))
        got = results(P.gate_snapshot)
        check("a re-pin that errored does NOT pass",
              got.get("snapshot verified against the chain") is False)

        r.write("snapshots/scry-holders.2026-07-25.json", _snap(verification={}))
        got = results(P.gate_snapshot)
        check("a snapshot with no verification block does NOT pass",
              got.get("snapshot verified against the chain") is False)

        # a partial re-pin (some holders re-read, some not) is the sneaky one
        r.write("snapshots/scry-holders.2026-07-25.json",
                _snap(n_holders=258,
                      verification={"repinned_all": 5, "differed_from_indexer": 0,
                                    "errors": []}))
        got = results(P.gate_snapshot)
        check("a partial re-pin does NOT pass as verified",
              got.get("snapshot verified against the chain") is False)


def test_snapshot_freshness_uses_its_own_clock():
    print("snapshot freshness")
    with Repo() as r:
        old = time.strftime("%Y-%m-%dT%H:%M:%SZ",
                            time.gmtime(time.time() - (P.MAX_SNAPSHOT_AGE_H + 24) * 3600))
        p = r.write("snapshots/scry-holders.2026-07-25.json", _snap(taken_at=old))
        # THE ROW: a touch resets mtime. The content still says it is stale.
        p.touch()
        # SCOPED 2026-07-27: staleness reds only when a drop is actually arming,
        # because neither drop reads the snapshot at launch time (drop one is
        # pinned at a past block, drop two snapshots at announcement). Both
        # halves are asserted here — a gate that is conditional has TWO
        # behaviours and testing one of them is how a condition rots into an
        # always-pass.
        os.environ["SCRY_CLAIM_OBOL"] = "0xdead"
        try:
            got = results(P.gate_snapshot)
            check("a touched-but-stale snapshot is still stale WHEN A DROP ARMS",
                  got.get("snapshot is fresh") is False)
        finally:
            os.environ.pop("SCRY_CLAIM_OBOL", None)
        got = results(P.gate_snapshot)
        check("the same stale snapshot is tolerated when NOTHING is arming",
              got.get("snapshot is fresh") is True)
        check("the clock it used is reported",
              got.get("snapshot carries its own timestamp") is True)

        # No taken_at at all: falls back to mtime, and SAYS SO rather than
        # pretending it measured the thing it names.
        r.write("snapshots/scry-holders.2026-07-25.json", _snap(taken_at=None))
        got = results(P.gate_snapshot)
        check("a snapshot with no taken_at is flagged",
              got.get("snapshot carries its own timestamp") is False)

        r.write("snapshots/scry-holders.2026-07-25.json", _snap())
        got = results(P.gate_snapshot)
        check("a genuinely fresh snapshot passes", got.get("snapshot is fresh") is True)


def test_real_repo_gates_pass():
    """The gates against the actual repo — this file must not be green while
    preflight itself is red."""
    print("the real repo")
    P.RESULTS.clear()
    snap = P.gate_snapshot()
    bad = [r["check"] for r in P.RESULTS if not r["ok"]]
    P.RESULTS.clear()
    check("the repo's own snapshot gates pass", not bad, str(bad))
    check("the snapshot is returned for the drop gate", bool(snap))
    P.gate_custody()
    bad = [r["check"] for r in P.RESULTS if not r["ok"]]
    P.RESULTS.clear()
    check("the repo's own custody gates pass", not bad, str(bad))




# ⛔ `test_declared_decisions_reach_the_ledger` retired 2026-08-21 with the gate
# it covered. `gate_sentences_record_decisions` required a commit declaring an
# operator decision to also write `docs/SENTENCES.md`; the ledger is closed now
# and CHANGELOG.md is the forward record. Its `_git_repo` fixture helper went
# with it — nothing else built a throwaway repo.


def test_a_pinned_drop_is_not_an_arming_drop():
    """`_drop_is_arming` gates how hard the snapshot and proxy checks bite, and
    it read a DEPLOYED drop as an ARMING one.

    `deployments.json` records a claim contract forever, so `if claims:` could
    never say "" again after 2026-07-30. That made `gate_snapshot`'s freshness
    check permanently hard against a snapshot no drop reads — the exact red its
    own comment says it was scoped to avoid. The discriminator is whether a
    fresh snapshot could still CHANGE the drop, which a pinned one cannot."""
    print("a pinned drop is not an arming drop")

    def _claims(entry):
        return {"chains": {"4663": {"claims": {"drop-x": entry}}}}

    # 1. THE BUG: a deployed drop with its snapshot pinned is NOT arming
    with Repo() as r:
        r.write("contracts/deployments.json",
                _claims({"snapshot": "snapshots/scry-holders.b23662441.json",
                         "plan": "snapshots/airdrop-plan.json", "OBOL": "0xabc"}))
        check("a drop with a pinned snapshot does not read as arming",
              P._drop_is_arming() == "", repr(P._drop_is_arming()))

    # 2. a drop recorded before it pins one still needs a fresh reading
    with Repo() as r:
        r.write("contracts/deployments.json", _claims({"_what": "drop two, being prepared"}))
        check("a drop with no pinned snapshot DOES read as arming",
              "drop-x" in P._drop_is_arming(), repr(P._drop_is_arming()))

    # 3. the env door is unchanged and still wins over any file state
    with Repo() as r:
        r.write("contracts/deployments.json",
                _claims({"snapshot": "snapshots/pinned.json"}))
        os.environ["SCRY_CLAIM_OBOL"] = "0xdeadbeef"
        try:
            check("SCRY_CLAIM_* in the environment still arms",
                  "SCRY_CLAIM_OBOL" in P._drop_is_arming(), repr(P._drop_is_arming()))
        finally:
            del os.environ["SCRY_CLAIM_OBOL"]

    # 4. no claims at all is not arming, and must not throw
    with Repo() as r:
        r.write("contracts/deployments.json", {"chains": {"4663": {}}})
        check("no claims recorded is not arming",
              P._drop_is_arming() == "", repr(P._drop_is_arming()))


def test_drop_claim_isolation():
    """§0.4 — one ScryHarvest instance may serve exactly one root producer.

    The gate has to walk a narrow line: DeployClaim runs AFTER the launch
    phases, so "no claim contracts" is the CORRECT state at launch time and
    must not be a red. But the moment one exists, an unrecorded set means the
    gate checked nothing while printing PASS — the vacuity this file exists
    for. Both halves are asserted, plus the two rules themselves.
    """
    print("drop claim isolation (§0.4)")
    import os
    real = P.REPO

    def run(claims, env=None, bridge=""):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / "snapshots").mkdir()
            for i, did in enumerate(("drop-one", "drop-two")):
                (root / "snapshots" / f"p{i}.json").write_text(json.dumps({"drop_id": did}))
            (root / "contracts").mkdir()
            chain = {"contracts": {}, "claims": claims} if claims is not None else {"contracts": {}}
            if bridge:
                chain["contracts"]["ScryHarvest"] = {"address": bridge}
            (root / "contracts" / "deployments.json").write_text(
                json.dumps({"chains": {"4663": chain}}))
            old, P.REPO = P.REPO, root
            saved = {k: os.environ.pop(k, None)
                     for k in ("SCRY_CLAIM_OBOL", "SCRY_CLAIM_MYRRH", "SCRY_CLAIM_SCRY")}
            os.environ.update(env or {})
            try:
                P.RESULTS.clear()
                P.gate_drop_claim_isolation()
                r = P.RESULTS[-1]
                return r["ok"], r["detail"]
            finally:
                P.REPO = old
                P.RESULTS.clear()
                for k in ("SCRY_CLAIM_OBOL", "SCRY_CLAIM_MYRRH", "SCRY_CLAIM_SCRY"):
                    os.environ.pop(k, None)
                    if saved.get(k) is not None:
                        os.environ[k] = saved[k]

    ok, detail = run(claims=None)
    check("nothing deployed yet is not a red (DeployClaim runs after the launch phases)",
          ok and "not yet applicable" in detail, detail)

    ok, detail = run(claims=None, env={"SCRY_CLAIM_OBOL": "0x" + "11" * 20})
    check("a claim contract armed with no per-drop record is a RED, not a vacuous pass",
          not ok and "nothing was verified" in detail, detail)

    A, B = "0x" + "aa" * 20, "0x" + "bb" * 20
    ok, _ = run(claims={"drop-one": {"OBOL": A}, "drop-two": {"OBOL": B}})
    check("distinct contracts per drop pass", ok)

    ok, detail = run(claims={"drop-one": {"OBOL": A}, "drop-two": {"OBOL": A}})
    check("two drops sharing one ScryHarvest is a RED", not ok and "serves both" in detail, detail)

    # case-only difference must not launder past rule 1
    ok, _ = run(claims={"drop-one": {"OBOL": A}, "drop-two": {"OBOL": A.upper()}})
    check("address comparison is case-insensitive", not ok)

    ok, detail = run(claims={"drop-one": {"OBOL": A}}, bridge=A)
    check("the bridge's own harvest may not be a drop's claim target",
          not ok and "BRIDGE" in detail, detail)

    P.REPO = real


def test_pool_float_survives_markdown():
    """The drop-share gate reads the OBOL float out of a markdown TABLE, so its
    parser is coupled to prose formatting — and on 2026-07-26 the 60/20 revision
    bolded the cell and the pattern stopped matching.

    The damage was not the red line. `gate_drop` returns early when the float is
    unreadable, so `the drop is a sane share of the OBOL float` never RAN: the
    run went 18 checks to 17 and the missing one is invisible unless you are
    counting. A gate that can vanish is worse than one that can fail.
    """
    print("pool float parsing")
    row = "| OBOL/SCRY | {scry} | {obol} | 10 SCRY / OBOL |\n"

    for label, obol in (
        ("plain",            "6,000,000 OBOL"),
        ("bold cell",        "**6,000,000 OBOL**"),
        ("bold number only", "**6,000,000** OBOL"),
        ("italic",           "*6,000,000 OBOL*"),
        ("padded",           "   6,000,000 OBOL   "),
    ):
        m = P.POOL_FLOAT_RE.search(row.format(scry="**60,000,000**", obol=obol))
        check(f"the float parses when the cell is {label}",
              m is not None and m.group(1).replace(",", "") == "6000000",
              f"got {m.group(1) if m else None!r}")

    # and it must still be reading the GAME-TOKEN column, not the SCRY one
    m = P.POOL_FLOAT_RE.search(row.format(scry="**60,000,000**", obol="**6,000,000 OBOL**"))
    check("it reads the game-token column, never the SCRY side",
          m and m.group(1) == "6,000,000", f"got {m.group(1) if m else None!r}")

    # the live doc is the case that actually matters
    m = P.POOL_FLOAT_RE.search(P.src("LAUNCH-DECISIONS.md"))
    check("the REAL LAUNCH-DECISIONS.md table parses today", m is not None,
          "the decision doc's pool table is unreadable to the gate")

    # MYRRH: the row that had no gate at all until 2026-07-26, which is exactly
    # where the 60/20 revision did its damage. Must read the MYRRH row and the
    # MYRRH row only — reading OBOL's would report a healthy number for the
    # wrong coin, which is worse than reporting nothing.
    mm = P.POOL_FLOAT_MYRRH_RE.search(P.src("LAUNCH-DECISIONS.md"))
    check("the REAL MYRRH row parses today", mm is not None,
          "the decision doc's MYRRH row is unreadable to the gate")
    check("the MYRRH pattern reads MYRRH, not OBOL",
          mm and "," in mm.group(1) and mm.group(1) != "6,000,000",
          f"got {mm.group(1) if mm else None!r}")
    check("the MYRRH pattern does not match the OBOL row at all",
          P.POOL_FLOAT_MYRRH_RE.search(
              "| OBOL/SCRY | **60,000,000** | **6,000,000 OBOL** | 10 |") is None)


def test_published_plans_are_immutable_commitments():
    """A published plan's terms are fixed; the gate rebuilds from the params the
    ARTIFACT pins, never from live DEFAULTS.

    The bug this pins: the posted rebuild command passed only the knobs that
    differed from DEFAULTS, so halving `myrrh_ratio` for the 60/20 revision took
    drop one's rebuild from 74,022 to 39,041 MYRRH with nothing to notice.
    """
    print("published plans reproduce")
    got = results(P.gate_published_plans_reproduce)
    key = "every published drop plan still reproduces"
    check("the repo's published plans reproduce today", got.get(key) is True,
          str(got))

    # and it must not pass vacuously on a directory with no plans in it
    with Repo() as r:
        (r.dir / "snapshots").mkdir(parents=True, exist_ok=True)
        got = results(P.gate_published_plans_reproduce)
        check("no plans on disk is an explicit not-yet, not a silent pass",
              got.get(key) is True, str(got))

    # a plan whose pinned params no longer produce its own totals must go RED
    with Repo() as r:
        real_snap = json.loads(
            (Path(P.REPO if False else Path(__file__).resolve().parents[1])
             / "snapshots" / "scry-holders.2026-07-25.json").read_text())
        real_plan = json.loads(
            (Path(__file__).resolve().parents[1]
             / "snapshots" / "retired" / "founders-bag.2026-07-25.json").read_text())
        r.write("snapshots/scry-holders.x.json", real_snap)
        tampered = dict(real_plan)
        tampered["totals"] = dict(real_plan["totals"], myrrh=real_plan["totals"]["myrrh"] + 1)
        r.write("snapshots/plan.json", tampered)
        got = results(P.gate_published_plans_reproduce)
        check("a plan that no longer reproduces is a RED", got.get(key) is False,
              str(got))


def test_drop_being_graded_is_named():
    """`gate_drop` estimates the NEXT drop from `claim_plan.DEFAULTS` on the
    newest snapshot on disk, and reported the result as `planned N MYRRH` — which
    reads as drop two and is not. Both halves are wrong at once: those params are
    not the ones drop two will use, and that snapshot is drop ONE's block, so the
    field is narrower than the one drop two is taken over.

    Same failure class as `src()` cross-reading — a gate whose output names one
    thing while measuring another is worse than one that fails, because the
    output still says the name. So the basis is now its own named check.

    It must NOT red a launch that is not arming a drop, for the reason
    `gate_drop_claim_isolation` already documents: `DeployClaim` runs after the
    launch phases, so a drop-shaped complaint would block seeding the pools.
    """
    print("the drop being graded is named")
    key = "the drop being graded is the real one, not a proxy"
    root = Path(__file__).resolve().parents[1]
    snap = json.loads((root / "snapshots" / "scry-holders.2026-07-25.json").read_text())
    plan = json.loads((root / "snapshots" / "retired" / "founders-bag.2026-07-25.json").read_text())

    def run(*, pinned=None, snap_block=None, published=True, env=None):
        saved = {k: os.environ.get(k) for k in
                 ("SCRY_CLAIM_OBOL", "SCRY_CLAIM_MYRRH", "SCRY_CLAIM_SCRY")}
        for k in saved:
            os.environ.pop(k, None)
        for k, v in (env or {}).items():
            os.environ[k] = v
        try:
            with Repo() as r:
                s = dict(snap, block=snap_block or snap["block"])
                r.write("snapshots/scry-holders.t.json", s)
                if published:
                    r.write("snapshots/published.json", plan)
                if pinned is not None:
                    r.write(P.NEXT_DROP_PARAMS, pinned)
                r.write("LAUNCH-DECISIONS.md",
                        "| OBOL/SCRY | **60,000,000** | **6,000,000 OBOL** | 10 |\n"
                        "| MYRRH/SCRY | **20,000,000** | **400,000 MYRRH** | 50 |\n")
                P._JSON_CACHE.clear()
                return results(P.gate_drop, s)
        finally:
            for k, v in saved.items():
                os.environ.pop(k, None)
                if v is not None:
                    os.environ[k] = v
            P._JSON_CACHE.clear()

    # nothing pinned + drop one's own block = a proxy, and it says so
    got = run()
    check("an unpinned estimate on a published block is flagged as a proxy",
          got.get(key) is True, str(got))          # informational, not a blocker

    # ...but the moment a drop is arming, the same proxy is a RED
    got = run(env={"SCRY_CLAIM_OBOL": "0x" + "11" * 20})
    check("arming a drop against a proxy is a RED", got.get(key) is False, str(got))

    # pinning the real terms at a fresh block clears it
    got = run(pinned={"params": {"base_threshold_scry": 5_000}},
              snap_block=99_999_999, published=True)
    check("pinned params at an unpublished block is the real thing",
          got.get(key) is True, str(got))
    got = run(pinned={"params": {"base_threshold_scry": 5_000}},
              snap_block=99_999_999, env={"SCRY_CLAIM_OBOL": "0x" + "11" * 20})
    check("and it stays green even while arming", got.get(key) is True, str(got))

    # a bare params dict (no "params" wrapper) is accepted too
    got = run(pinned={"base_threshold_scry": 5_000}, snap_block=99_999_999,
              env={"SCRY_CLAIM_OBOL": "0x" + "11" * 20})
    check("a bare params dict is accepted", got.get(key) is True, str(got))

    # pinned params must actually DRIVE the plan, or the seam is decorative
    with Repo() as r:
        r.write("snapshots/scry-holders.t.json", dict(snap, block=99_999_999))
        r.write("LAUNCH-DECISIONS.md",
                "| OBOL/SCRY | **60,000,000** | **6,000,000 OBOL** | 10 |\n"
                "| MYRRH/SCRY | **20,000,000** | **400,000 MYRRH** | 50 |\n")
        r.write(P.NEXT_DROP_PARAMS, {"params": {"base_threshold_scry": 1_000_000}})
        P._JSON_CACHE.clear()
        params, _ = P._next_drop_params()
        check("the pinned bar is what gate_drop reads",
              params == {"base_threshold_scry": 1_000_000}, str(params))
    P._JSON_CACHE.clear()

    # the headroom band rides on the MYRRH check's detail, and it must read
    # low-high — an inverted band ("1,101-437") is the sort of thing a reader
    # silently corrects for and a script does not.
    P.RESULTS.clear()
    P.gate_drop(json.loads(
        (root / "snapshots" / "scry-holders.2026-07-25.json").read_text()))
    row = next((r for r in P.RESULTS
                if r["check"] == "the drop is a sane share of the MYRRH float"), None)
    P.RESULTS.clear()
    check("the MYRRH gate still passes on the real repo", row and row["ok"], str(row))
    # TWO SHAPES, and the row must say which one it is in. While a per-holder
    # MYRRH term exists, a wider field costs MYRRH and the detail brackets how
    # many more recipients fit. Since 2026-07-27 the whole MYRRH budget is a
    # FIXED ticket pot, so a wider field costs nothing and a recipient band
    # would be inventing a limit that is not there. Asserting only the band
    # made this test red for a change that made the drop strictly safer.
    detail = (row or {}).get("detail", "")
    band = re.search(r"≈ ([\d,]+)-([\d,]+) more recipients", detail)
    fixed = "FIXED pot" in detail
    check("the MYRRH row explains its headroom one way or the other",
          bool(band) != fixed, detail)
    if band:
        lo, hi = (int(x.replace(",", "")) for x in band.groups())
        check("the headroom band reads low-high", lo <= hi, f"{lo}-{hi}")
        # measured 2026-07-26: ~437 at today's median, ~1,101 at the bar
        check("the band brackets the measured numbers", lo >= 300 and hi >= 1_000,
              f"{lo}-{hi}")
    else:
        check("a fixed pot says turnout does not change the total",
              "never the total minted" in detail, detail)


def test_seed_script_matches_the_decision_table():
    """The wrapper that broadcasts must agree with the doc that decided.

    The real defect, 2026-07-26: `deploy_town.sh pools` passed ONE
    `POOL_SCRY_BUDGET` (default 40000000) to both `--obol-scry-budget` and
    `--myrrh-scry-budget`, so an armed run seeded **40M/40M** — the split the
    operator had revised to **60M/20M** that same day. The decision doc said
    "the seed script takes --myrrh-scry-budget as an argument, so no code
    changes": true of the helper, false of the wrapper, and the wrapper is what
    broadcasts. Every doc and every test was green.
    """
    print("seed script matches the decision table")
    key = "the seed script's pool sizes match the decision table"
    TABLE = ("| OBOL/SCRY | **60,000,000** | **6,000,000 OBOL** | 10 |\n"
             "| MYRRH/SCRY | **20,000,000** | **400,000 MYRRH** | 50 |\n")

    def run(sh, table=TABLE):
        with Repo() as r:
            r.write("contracts/deploy_town.sh", sh)
            r.write("LAUNCH-DECISIONS.md", table)
            return results(P.gate_pool_seed_matches_decision)

    good = ('POOL_SCRY_BUDGET_OBOL="${POOL_SCRY_BUDGET_OBOL:-60000000}"\n'
            'POOL_SCRY_BUDGET_MYRRH="${POOL_SCRY_BUDGET_MYRRH:-20000000}"\n')
    check("the decided split passes", run(good).get(key) is True, str(run(good)))

    # the exact shape that shipped: one budget, both pools
    old = 'POOL_SCRY_BUDGET="${POOL_SCRY_BUDGET:-40000000}"\n'
    check("the ACTUAL 40/40 defect is caught", run(old).get(key) is False, str(run(old)))

    # a half-migration — OBOL split out, MYRRH left behind — is the likelier
    # future mistake, and it must not pass on the strength of the one that moved
    half = ('POOL_SCRY_BUDGET_OBOL="${POOL_SCRY_BUDGET_OBOL:-60000000}"\n'
            'POOL_SCRY_BUDGET="${POOL_SCRY_BUDGET:-40000000}"\n')
    check("a half-migrated script is caught", run(half).get(key) is False, str(run(half)))

    # drift in either direction, on either coin
    for label, sh in (
        ("OBOL drifts", good.replace("60000000", "50000000")),
        ("MYRRH drifts", good.replace("20000000", "30000000")),
    ):
        check(f"{label} is caught", run(sh).get(key) is False, str(run(sh)))

    # and it must read the SCRIPT, not a doc about the script: an absent file is
    # a red, never a vacuous pass (the src_required lesson, applied to a new gate)
    with Repo() as r:
        r.write("LAUNCH-DECISIONS.md", TABLE)
        got = results(P.gate_pool_seed_matches_decision)
        check("a missing deploy script is a RED, not a silent pass",
              got.get("contracts/deploy_town.sh is readable") is False, str(got))

    # the live repo is the case that matters
    got = results(P.gate_pool_seed_matches_decision)
    check("the REAL deploy_town.sh matches the REAL decision table today",
          got.get(key) is True, str(got))


def test_house_never_claims_from_its_own_drop():
    """The operator may not appear in the operator's own giveaway.

    Found live 2026-07-26: the committed drop-one rehearsal paid the operator's
    14.25% wallet 10,500 OBOL + **7,151 MYRRH — 18% of the whole drop** — and
    drew it one of the twelve balance-weighted golden tickets, 833,333 SCRY.
    The drop's stated security property is that the snapshot "cannot be bought
    into after the fact by anyone, INCLUDING the operator"; a balance-weighted
    lottery paying its largest holder is the EXPECTED outcome, so this would
    have read as rigged and the arithmetic would have agreed.

    The gate has to fail in BOTH directions: a house wallet present in a plan,
    and a house list that is missing or empty — which would let every plan pass
    by having nothing to compare against.
    """
    print("no excluded wallet claims from the drop")
    key = "no excluded wallet claims from the drop"
    OP = "0xb474a95200bc5de8950e445b1e9d524f4de0f18d"
    OTHER = "0x1111111111111111111111111111111111111111"

    def run(house, allocations, other=None):
        with Repo() as r:
            if house is not None:
                doc = {"wallets": house}
                if other:
                    doc["excluded_other"] = {"wallets": other}
                r.write("snapshots/house-wallets.json", doc)
            r.write("snapshots/p.json",
                    {"drop_id": "d", "params": {}, "allocations": allocations})
            P._JSON_CACHE.clear()
            try:
                return results(P.gate_house_never_claims)
            finally:
                P._JSON_CACHE.clear()

    rich = {"obol": 10500, "myrrh": 7151, "scry": 833333, "ticket": True}
    check("a house wallet holding an allocation is a RED",
          run({OP: "operator"}, {OP: rich, OTHER: rich}).get(key) is False)
    check("a plan paying only strangers passes",
          run({OP: "operator"}, {OTHER: rich}).get(key) is True)

    # the vacuous-pass direction, which this file has now been bitten by twice
    check("a MISSING house list is a RED, not a silent pass",
          run(None, {OP: rich}).get(key) is False)
    check("an EMPTY house list is a RED too", run({}, {OP: rich}).get(key) is False)

    # case-insensitivity: an address is not a string, and a checksummed plan
    # must not slip past a lowercased list
    check("the comparison is case-insensitive",
          run({OP: "operator"}, {OP.upper().replace("0X", "0x"): rich}).get(key) is False)

    # the 2026-07-22 artifact maps wallet -> int, not -> dict. Assuming the dict
    # shape made the gate RAISE on it, and a gate that crashes gets commented out
    got = run({OP: "operator"}, {OP: 10000})
    check("a single-token (wallet -> int) plan is read, not crashed on",
          got.get(key) is False, str(got))

    # the live repo: the real house list, the real plans
    P._JSON_CACHE.clear()
    got = results(P.gate_house_never_claims)
    check("the REAL snapshots/ pays no house wallet today", got.get(key) is True, str(got))

    # and the list itself must name the wallets the operator confirmed
    import json as _json_mod
    hw = _json_mod.loads(
        (Path(__file__).resolve().parents[1] / "snapshots" / "house-wallets.json").read_text())
    check("the house list is non-empty and lowercased",
          len(hw["wallets"]) >= 3 and all(w == w.lower() for w in hw["wallets"]),
          str(list(hw["wallets"])))

    # ── house-side, not ours: excluded identically, disclosed separately ─────
    # Added 2026-07-27 with the pons fee wallet. The separate key exists ONLY
    # because `wallets` is a claim of OWNERSHIP; the gate must still treat both
    # the same, or the new list is a note rather than a control.
    PONS = "0xda4bcee76b29efec9697fcf663601c2042043968"
    check("a house-side wallet we do not control holding an allocation is a RED too",
          run({OP: "operator"}, {PONS: rich, OTHER: rich},
              other={PONS: "pons"}).get(key) is False)
    def detail_of(house, allocations, other=None):
        """The gate's own failure TEXT, not just its boolean — the point of this
        one is that a reader must be told which kind of exclusion was paid."""
        with Repo() as r:
            doc = {"wallets": house}
            if other:
                doc["excluded_other"] = {"wallets": other}
            r.write("snapshots/house-wallets.json", doc)
            r.write("snapshots/p.json",
                    {"drop_id": "d", "params": {}, "allocations": allocations})
            P._JSON_CACHE.clear()
            P.RESULTS.clear()
            try:
                P.gate_house_never_claims()
                return " ".join(str(x["detail"]) for x in P.RESULTS)
            finally:
                P.RESULTS.clear()
                P._JSON_CACHE.clear()

    check("...and it is reported as house-side-not-ours, not as the operator",
          "house-side, not ours" in detail_of({OP: "operator"}, {PONS: rich},
                                              other={PONS: "pons"}),
          "the failure text must say WHICH kind was paid")
    check("...while an operator wallet is still reported as the HOUSE",
          "[HOUSE (operator)]" in detail_of({OP: "operator"}, {OP: rich},
                                            other={PONS: "pons"}))
    check("a plan paying strangers still passes with the second list present",
          run({OP: "operator"}, {OTHER: rich}, other={PONS: "pons"}).get(key) is True)
    check("an ABSENT excluded_other block is legal — it is optional, not required",
          run({OP: "operator"}, {OTHER: rich}).get(key) is True)

    other_block = (hw.get("excluded_other") or {}).get("wallets") or {}
    check("the real excluded_other list is lowercased and non-empty",
          len(other_block) >= 1 and all(w == w.lower() for w in other_block),
          str(list(other_block)))
    check("the pons wallet is NOT filed under `wallets` — that key means ownership",
          PONS in other_block and PONS not in hw["wallets"])
    check("the 14% wallet is on it", OP in hw["wallets"])


def test_season_pot_and_tvl_tripwires():
    """§Still open #2 and #3 — both are rules that arm later, so both must be
    explicitly not-applicable now rather than silently passing."""
    print("season pot cap + TVL tripwire")
    pot_key = "a season pot is within 10% of the OBOL reserve"
    tvl_key = "no surface publishes a summed TVL"

    def with_pot(v):
        saved = os.environ.get("SEASON_POT")
        if v is None:
            os.environ.pop("SEASON_POT", None)
        else:
            os.environ["SEASON_POT"] = v
        try:
            return results(P.gate_season_pot_cap)
        finally:
            os.environ.pop("SEASON_POT", None)
            if saved is not None:
                os.environ["SEASON_POT"] = saved

    wei = lambda n: str(int(n) * 10 ** 18)          # noqa: E731
    # DERIVED from the decision table, never typed: these boundaries were
    # hardcoded at 600,000 (10% of a 6,000,000 OBOL float) and silently stopped
    # testing a boundary when the pools were deepened on 2026-07-28 and the
    # float became 7,650,000. A cap test pinned to a stale cap passes for the
    # wrong reason — exactly the drift the gate itself exists to catch.
    # WALK docs/ rather than joining a folder name. This was the FIFTH
    # hardcoded doc resolver in the tree, and the one CLAUDE.md's "all four now
    # walk" note did not know about: it joined `docs/LAUNCH-DECISIONS.md` and
    # has raised FileNotFoundError since the 2026-08-09 reorg filed the doc
    # under `docs/launch/`. It failed LOUDLY, which is the only reason it was
    # not a silent pass — but it took the whole preflight suite down for a
    # re-file rather than a fault, which is the red this walk exists to end.
    _docs = Path(__file__).resolve().parents[1] / "docs"
    _ldp = next((p for p in _docs.rglob("LAUNCH-DECISIONS.md") if p.is_file()), None)
    if _ldp is None:
        raise FileNotFoundError("LAUNCH-DECISIONS.md is nowhere under docs/")
    _ld = _ldp.read_text()
    _m = re.search(r"OBOL/SCRY \|\s*\*\*[\d,]+\*\*\s*\|\s*\*\*([\d,]+) OBOL\*\*", _ld)
    OBOL_FLOAT = int(_m.group(1).replace(",", ""))
    CAP = OBOL_FLOAT // 10
    check("no season posted is an explicit not-yet", with_pot(None).get(pot_key) is True)
    check(f"a pot at exactly 10% of the {OBOL_FLOAT:,} float ({CAP:,}) passes",
          with_pot(wei(CAP)).get(pot_key) is True, str(with_pot(wei(CAP))))
    check("a pot at 10% + 1 wei is a RED",
          with_pot(str(CAP * 10 ** 18 + 1)).get(pot_key) is False)
    check("the 13.4%-of-float dump-threshold pot (-22% by xy*k) is a RED",
          with_pot(wei(int(OBOL_FLOAT * 0.134))).get(pot_key) is False)
    check("a non-integer SEASON_POT is a RED, not a crash",
          with_pot("6e23").get(pot_key) is False)

    # TVL: today nothing publishes one, and that must read as "arms later"
    got = results(P.gate_tvl_is_never_summed)
    check("no TVL field yet is an explicit not-yet", got.get(tvl_key) is True, str(got))

    with Repo() as r:
        (r.dir / "meter").mkdir(parents=True, exist_ok=True)
        r.write("meter/x.py", 'card = {"tvl": funded + house_minted}\n')
        check("a summed TVL is a RED",
              results(P.gate_tvl_is_never_summed).get(tvl_key) is False)
    with Repo() as r:
        (r.dir / "meter").mkdir(parents=True, exist_ok=True)
        r.write("meter/x.py",
                'card = {"tvl_funded": a, "tvl_house_minted": b}\n')
        check("the split fields pass",
              results(P.gate_tvl_is_never_summed).get(tvl_key) is True)
    # the word in prose must not trip it — a gate that fires on a comment gets
    # disabled, and a disabled gate protects nothing
    with Repo() as r:
        (r.dir / "meter").mkdir(parents=True, exist_ok=True)
        r.write("meter/x.py", '# we never publish a summed TVL, see LAUNCH-DECISIONS\n')
        check("the word TVL in prose does not trip it",
              results(P.gate_tvl_is_never_summed).get(tvl_key) is True)

    # ── it fires on AGGREGATION, not on the word (refined 2026-07-26) ────────
    # The first version failed ANY file carrying a tvl key, which reddened
    # watchtower/js/pools.js and meter/test_pools.py — both of which only ever
    # show ONE pool's number, which is honest. Measured consequence, within
    # hours: pools.js renamed its local `tvl` to `poolTvl` and cited this gate
    # in the comment, while the actual summed aggregate in pools.py stayed red.
    # A gate that pushes renames instead of disclosure protects nothing.
    with Repo() as r:
        (r.dir / "meter").mkdir(parents=True, exist_ok=True)
        r.write("meter/x.py",
                'def show(p):\n'
                '    tvl = (p.get("holds") or {}).get("tvl_usd")\n'
                '    return {"tvl_usd": tvl}   # ONE pool. honest.\n')
        check("per-pool TVL passes without the split fields",
              results(P.gate_tvl_is_never_summed).get(tvl_key) is True)
    with Repo() as r:
        (r.dir / "meter").mkdir(parents=True, exist_ok=True)
        r.write("meter/x.py",
                'card = {"tvl_usd": sum(c["holds"]["tvl_usd"] for c in live)}\n')
        check("the real shape that shipped — sum() across pools — is a RED",
              results(P.gate_tvl_is_never_summed).get(tvl_key) is False)
    with Repo() as r:
        (r.dir / "meter").mkdir(parents=True, exist_ok=True)
        r.write("meter/x.py",
                'card = {"tvl_funded_usd": _sum(live, False),\n'
                '        "tvl_house_minted_usd": _sum(live, True)}\n'
                'def _sum(cs, h): return sum(c["holds"]["tvl_usd"] for c in cs)\n')
        check("an aggregate that IS split passes",
              results(P.gate_tvl_is_never_summed).get(tvl_key) is True)
    with Repo() as r:
        (r.dir / "meter").mkdir(parents=True, exist_ok=True)
        r.write("meter/x.js", 'const total = pools.reduce((a,p) => a + p.tvl_usd, 0);\n')
        check("a JS reduce() over tvl is caught too",
              results(P.gate_tvl_is_never_summed).get(tvl_key) is False)


def test_powder_tracks_the_drop():
    """The two coins follow different rules, and BOTH are anti-drift.

    OBOL is a posted fixed allocation (2026-07-28) read back out of
    LAUNCH-DECISIONS.md; MYRRH keeps the 10%-of-drop rule because it is capped
    and the lifetime headroom is ~1,000,000. Neither number is typed twice.

    The load-bearing cases are the drift ones: when the POSTED number moves and
    the Solidity does not, and when the DROP moves and the MYRRH powder does
    not. Plus the ceiling, which is what stops a posted number — a figure with
    no denominator of its own — from being raised to anything one edit at a
    time.
    """
    print("powder tracks its posted rule")
    key = "the powder tracks its posted rule (OBOL posted, MYRRH 10% of the drop)"
    root = Path(__file__).resolve().parents[1]
    real_snap = json.loads(
        (root / "snapshots" / "scry-holders.2026-07-27.json").read_text())

    def repo_with(obol, myrrh, posted="1,000,000", float_obol="7,650,000", bar=None):
        r = Repo()
        r.write("contracts/script/DeploySpoils.s.sol",
                "contract DeploySpoils {\n"
                '  inp.powderTo = vm.envOr("POWDER_TO", address(0));\n'
                f'  inp.powderObol = vm.envOr("POWDER_OBOL", uint256({obol}e18));\n'
                f'  inp.powderMyrrh = vm.envOr("POWDER_MYRRH", uint256({myrrh}e18));\n'
                "}\n")
        r.write("LAUNCH-DECISIONS.md",
                "| pool | SCRY side | game-token side | opening ratio |\n"
                "|---|---:|---:|---:|\n"
                f"| OBOL/SCRY | **76,500,000** | **{float_obol} OBOL** | 10 SCRY / OBOL |\n"
                f"| powder (OBOL) | — | **{posted}** | operator working float |\n")
        (r.dir / "snapshots").mkdir(parents=True, exist_ok=True)
        # The gate imports the REAL claim_plan and reads the REAL house-wallet
        # exclusions — a throwaway root without them computes a different drop
        # and the whole comparison becomes meaningless. Symlinked rather than
        # copied so this test can never grade a stale fork of either.
        (r.dir / "meter").symlink_to(root / "meter")
        (r.dir / "snapshots" / "house-wallets.json").symlink_to(
            root / "snapshots" / "house-wallets.json")
        if bar is not None:
            r.write("snapshots/next-drop.params.json",
                    {"params": {"base_threshold_scry": bar}})
        return r

    with repo_with("1_000_000", "240", bar=1_000_000):
        check("the shipped defaults match the posted OBOL and 10% of the real drop",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is True,
              str(results(P.gate_powder_tracks_the_drop, real_snap)))

    # THE OBOL DRIFT CASE. The doc is the source; the Solidity must follow it.
    with repo_with("1_000_001", "240", bar=1_000_000):
        check("one OBOL off the posted number is a RED",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is False)
    with repo_with("1_000_000", "240", posted="2,000,000", bar=1_000_000):
        check("the SAME powder goes RED when the POSTED number moves",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is False)

    # THE MYRRH DRIFT CASE, unchanged in kind: the powder is untouched and
    # correct for yesterday's drop, then the BAR moves and the drop shrinks.
    with repo_with("1_000_000", "239", bar=1_000_000):
        check("one MYRRH under 10% is a RED — both coins are checked",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is False)
    # The BAR is the wrong lever for MYRRH and that is worth pinning: the MYRRH
    # side is a FIXED pot, so a wider or narrower field changes the prize per
    # winner and never the total minted. The pot itself is the lever.
    with repo_with("1_000_000", "240", bar=1_000_000) as r:
        r.write("snapshots/next-drop.params.json",
                {"params": {"base_threshold_scry": 1_000_000, "ticket_pot_myrrh": 5_000}})
        check("the SAME MYRRH powder goes RED when the drop's POT moves",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is False)

    # THE CEILING. A posted number has no denominator of its own, so without
    # this it can be walked up one edit at a time with every gate still green.
    with repo_with("2_000_000", "240", posted="2,000,000", bar=1_000_000):
        check("a posted powder over 25% of the pool float is a RED",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is False)
    with repo_with("1_900_000", "240", posted="1,900,000", bar=1_000_000):
        check("...and just under the ceiling still passes",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is True)

    # Silent-empty reads are REDs. "We could not look" must never read as fine.
    with repo_with("1_000_000", "240", bar=1_000_000) as r:
        r.write("contracts/script/DeploySpoils.s.sol", "contract DeploySpoils {}\n")
        check("an unparseable DeploySpoils is a RED",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is False)
    with repo_with("1_000_000", "240", posted="", bar=1_000_000):
        check("a decision table with no powder row is a RED",
              results(P.gate_powder_tracks_the_drop, real_snap).get(key) is False)
    with repo_with("1_000_000", "240", bar=1_000_000):
        check("no snapshot is a RED, not a vacuous pass",
              results(P.gate_powder_tracks_the_drop, None).get(key) is False)


def test_table_pot_obeys_the_season_pot_rule():
    """The Temptation Table's bankroll seed takes the SAME 10%-of-the-OBOL-
    reserve cap as a season pot, and the cap MOVES WITH THE OPENING RATIO.

    Every case here reintroduces something that was actually true: the pot was
    named at 2,500,000 (4.2x the cap) with nothing checking, and the reserve
    the cap is derived from is `60,000,000 $SCRY / rate` — so the pot and the
    ratio are one decision made in two documents. If raising the rate does not
    tighten this gate, the gate is decorative.
    """
    print("table pot cap")
    key = "the table pot is within 10% of the OBOL reserve"

    def repo_with(reserve, pot_line):
        r = Repo()
        # The real row format, verbatim — POOL_FLOAT_RE is written against it
        # and `test_pool_float_survives_markdown` owns its tolerances.
        r.write("docs/launch/LAUNCH-DECISIONS.md",
                "| pool | SCRY | game token | opening ratio |\n"
                "|---|---:|---:|---:|\n"
                f"| OBOL/SCRY | **60,000,000** | **{reserve} OBOL** | rate |\n")
        r.write("meter/ecosystem.config.example.js",
                "module.exports = { apps: [{ env: {\n"
                f"{pot_line}\n"
                "} }] }\n")
        return r

    # 1. The number the operator first named, against the pool as posted.
    with repo_with("6,000,000", '      SCRY_TABLE_POT_SEED: "2500000",') as r:
        check("2,500,000 against a 6,000,000 reserve is a RED",
              results(P.gate_table_pot_cap).get(key) is False)

    # 2. The revised number: exactly at the cap, which must pass.
    with repo_with("6,000,000", '      SCRY_TABLE_POT_SEED: "600000",') as r:
        check("600,000 (exactly 10%) passes",
              results(P.gate_table_pot_cap).get(key) is True,
              str(results(P.gate_table_pot_cap)))

    with repo_with("6,000,000", '      SCRY_TABLE_POT_SEED: "600001",') as r:
        check("600,001 is a RED — the gate does not round in the permissive "
              "direction", results(P.gate_table_pot_cap).get(key) is False)

    # 3. THE COUPLING. Same 600,000 pot, but the opening ratio moved 10 -> 100,
    #    so the OBOL reserve is a tenth and the cap is 60,000. The pot did not
    #    change and must now be RED. This is the case the gate exists for.
    with repo_with("600,000", '      SCRY_TABLE_POT_SEED: "600000",') as r:
        check("the SAME 600,000 pot goes RED when the ratio moves to 100:1",
              results(P.gate_table_pot_cap).get(key) is False)
    with repo_with("600,000", '      SCRY_TABLE_POT_SEED: "60000",') as r:
        check("60,000 passes at 100:1 — the cap tracks the reserve",
              results(P.gate_table_pot_cap).get(key) is True)

    # 4. A silent-empty read must be a RED. "We could not look" and "it is
    #    fine" are the same bytes otherwise — the trap this repo has hit with
    #    never-raises readers returning [] for both.
    with repo_with("6,000,000", "      // nothing here") as r:
        check("no SCRY_TABLE_POT_SEED in the example config is a RED",
              results(P.gate_table_pot_cap).get(key) is False)
    with Repo() as r:
        r.write("meter/ecosystem.config.example.js",
                'env: { SCRY_TABLE_POT_SEED: "600000" }')
        check("an unparseable pool table is a RED, not a pass",
              results(P.gate_table_pot_cap).get(key) is False)

    # 5. A commented-out knob still counts. The regex tolerates `// ` on
    #    purpose: a decision recorded but disarmed is still the recorded
    #    decision, and letting a comment hide an over-cap number from the gate
    #    would make "disarm it to ship it" a way past the rule.
    with repo_with("6,000,000", '      // SCRY_TABLE_POT_SEED: "2500000",') as r:
        check("a COMMENTED-OUT over-cap seed is still a RED",
              results(P.gate_table_pot_cap).get(key) is False)

    # 6. The live environment is graded too, not just the tracked record.
    saved = os.environ.get("SCRY_TABLE_POT_SEED")
    try:
        with repo_with("6,000,000", '      SCRY_TABLE_POT_SEED: "600000",') as r:
            os.environ["SCRY_TABLE_POT_SEED"] = "2500000"
            check("a compliant config with an over-cap ENV is a RED",
                  results(P.gate_table_pot_cap).get(key) is False)
            os.environ["SCRY_TABLE_POT_SEED"] = "not-a-number"
            check("a non-integer env is a RED, not a crash",
                  results(P.gate_table_pot_cap).get(key) is False)
    finally:
        os.environ.pop("SCRY_TABLE_POT_SEED", None)
        if saved is not None:
            os.environ["SCRY_TABLE_POT_SEED"] = saved


def test_merkle_totals_must_be_derived():
    """§M4's off-chain half. `postRoot` cannot check a total against its tree,
    so the total has to be DERIVED before it ever reaches the chain. The gate
    proves the artifact the operator copies from is self-consistent; these are
    the four ways it can stop being."""
    print("merkle totals are derived (§M4)")
    key = "published merkle totals are derived, not typed"

    # populate sys.modules from the REAL repo so the gate's import resolves
    # while REPO points at a throwaway tree.
    sys.path.insert(0, str(P.REPO))
    from meter import claim_plan as cp

    wallets = ["0x" + f"{i:040x}" for i in (1, 2, 3)]
    amounts = [100 * 10**18, 250 * 10**18, 7 * 10**18]

    def artifact(**over):
        entries = sorted(zip(wallets, amounts))
        leaves = [cp._leaf(w, a) for w, a in entries]
        levels = cp._levels(leaves)
        art = {
            "token": "OBOL",
            "root": "0x" + levels[-1][0].hex(),
            "total_wei": str(sum(a for _, a in entries)),
            "n": len(entries),
            "claims": {w: {"amount_wei": str(a),
                           "proof": ["0x" + n.hex() for n in cp._proof(levels, i)]}
                       for i, (w, a) in enumerate(entries)},
        }
        art.update(over)
        return art

    with Repo() as r:
        (r.dir / "snapshots").mkdir(parents=True, exist_ok=True)
        check("no artifact yet -> arms later, does not red the launch",
              results(P.gate_merkle_totals_are_derived).get(key) is True)

    with Repo() as r:
        r.write("snapshots/d.obol.merkle.json", artifact())
        check("an honest artifact passes",
              results(P.gate_merkle_totals_are_derived).get(key) is True)

    # THE §M4 SHAPE: a real root, a false total. This is the exact thing the
    # contract cannot catch and the sweep floor is armed to.
    with Repo() as r:
        r.write("snapshots/d.obol.merkle.json", artifact(total_wei="0"))
        check("a real root with a ZERO total is refused",
              results(P.gate_merkle_totals_are_derived).get(key) is False)
    with Repo() as r:
        r.write("snapshots/d.obol.merkle.json", artifact(total_wei=str(999 * 10**18)))
        check("a total that is merely WRONG is refused too",
              results(P.gate_merkle_totals_are_derived).get(key) is False)

    with Repo() as r:
        r.write("snapshots/d.obol.merkle.json", artifact(root="0x" + "ab" * 32))
        check("a root that does not recompute is refused",
              results(P.gate_merkle_totals_are_derived).get(key) is False)

    with Repo() as r:
        r.write("snapshots/d.obol.merkle.json", artifact(n=2))
        check("a claim count that disagrees with the claims is refused",
              results(P.gate_merkle_totals_are_derived).get(key) is False)

    # a proof that does not verify: the root and total stay honest, so only the
    # per-wallet check can catch this one
    with Repo() as r:
        art = artifact()
        art["claims"][wallets[0]]["proof"] = ["0x" + "cd" * 32]
        r.write("snapshots/d.obol.merkle.json", art)
        check("a proof that does not verify is refused",
              results(P.gate_merkle_totals_are_derived).get(key) is False)

    # ── THE ACCESS DIALECT. Same tree, same walk, a different second word:
    # `seat_roots.py` imports the leaf and the levels straight out of
    # `claim_plan`, and what changes is that the total arms a door's seat cap
    # instead of a sweep floor. The gate knew only the airdrop shape and went
    # red the day the first seat tree landed, so these cases exist to keep the
    # fix honest — including the one that matters most, which is that a shape
    # the gate cannot read stays RED rather than being skipped.
    def access_artifact(**over):
        entries = sorted((w, 1) for w in wallets)
        leaves = [cp._leaf(w, a) for w, a in entries]
        levels = cp._levels(leaves)
        art = {
            "door": 1,
            "root": "0x" + levels[-1][0].hex(),
            "seats_promised": sum(a for _, a in entries),
            "per_wallet": 1,
            "n": len(entries),
            "claims": {w: {"allowance": str(a),
                           "proof": ["0x" + n.hex() for n in cp._proof(levels, i)]}
                       for i, (w, a) in enumerate(entries)},
        }
        art.update(over)
        return art

    with Repo() as r:
        r.write("snapshots/seat-door1.merkle.json", access_artifact())
        check("an honest ACCESS artifact passes in its own dialect",
              results(P.gate_merkle_totals_are_derived).get(key) is True)

    with Repo() as r:
        r.write("snapshots/seat-door1.merkle.json", access_artifact(seats_promised=999))
        check("a seat cap that disagrees with the tree is refused",
              results(P.gate_merkle_totals_are_derived).get(key) is False)

    with Repo() as r:
        art = access_artifact()
        art["claims"][wallets[0]]["allowance"] = "5"
        r.write("snapshots/seat-door1.merkle.json", art)
        check("an allowance edited under a real proof is refused",
              results(P.gate_merkle_totals_are_derived).get(key) is False)

    # ⚠ THE ONE THAT GUARDS THE FIX ITSELF. The tempting repair for "the gate
    # cannot parse this file" is to skip the file, which turns an ungraded
    # total into a green light — the manufactured green this repo has shipped
    # before. A third dialect must land as work, not as silence.
    with Repo() as r:
        art = access_artifact()
        for c in art["claims"].values():
            c["qty"] = c.pop("allowance")
        r.write("snapshots/seat-door1.merkle.json", art)
        check("a claim shape the gate does not know is REFUSED, never skipped",
              results(P.gate_merkle_totals_are_derived).get(key) is False)

    # both dialects on the shelf at once — the actual state of snapshots/
    with Repo() as r:
        r.write("snapshots/d.obol.merkle.json", artifact())
        r.write("snapshots/seat-door1.merkle.json", access_artifact())
        check("the two dialects grade side by side",
              results(P.gate_merkle_totals_are_derived).get(key) is True)


def test_deploy_script_scanner_discriminates():
    """`gate_deploy_scripts` bans a pattern across all 20 deploy scripts on the
    strength of ONE observed instance (F6, in DeployGardener) — and it had no
    test of its own, so nothing showed it could tell the banned shape from the
    sanctioned one. A scanner that cannot discriminate is either a rubber stamp
    or a false-positive generator that gets switched off; both end the same way.

    The vacuity underneath is worth naming: over an EMPTY script directory both
    source checks report "clean" and PASS. Only the presence check stands
    between that and a green run, so it is pinned first.
    """
    print("the deploy-script scanner")
    key_present = "deploy scripts are present"
    key_eager = "no deploy script hides a mandatory env var inside vm.envOr"
    key_secret = "no deploy script carries a literal 32-byte secret"

    BANNED = ('contract D { function run() external {\n'
              '    address a = vm.envOr("PRIMARY", vm.envAddress("FALLBACK"));\n} }\n')
    SANCTIONED = ('contract D { function run() external {\n'
                  '    address a = envAddrOr("PRIMARY", "FALLBACK");\n'
                  '    uint256 b = vm.envOr("SLIP_BPS", uint256(100));\n} }\n')

    # 1. an empty script dir: the two source checks are vacuously clean, and
    #    the presence check is the only thing that catches it.
    with Repo() as r:
        (r.dir / "contracts" / "script").mkdir(parents=True)
        got = results(P.gate_deploy_scripts)
        check("an empty script dir fails the presence check", got.get(key_present) is False)
        check("...while both source checks report clean - the vacuity it guards",
              got.get(key_eager) is True and got.get(key_secret) is True)

    # 2. it catches the banned shape, and does NOT catch the replacement.
    with Repo() as r:
        r.write("contracts/script/Bad.s.sol", BANNED)
        check("the banned eager fallback is caught",
              results(P.gate_deploy_scripts).get(key_eager) is False)
    with Repo() as r:
        r.write("contracts/script/Good.s.sol", SANCTIONED)
        check("the sanctioned sequential shape is NOT flagged",
              results(P.gate_deploy_scripts).get(key_eager) is True)

    # 3. the shape nested one level deeper still counts — a cast around it is
    #    the obvious way to smuggle it past a naive regex.
    with Repo() as r:
        r.write("contracts/script/Nested.s.sol",
                'contract D { function run() external {\n'
                '    SpoilsToken t = SpoilsToken(vm.envOr("A", vm.envAddress("B")));\n} }\n')
        check("a cast around the banned shape does not hide it",
              results(P.gate_deploy_scripts).get(key_eager) is False)

    # 4. a COMMENT describing the ban must not trip it. DeployGardener's own
    #    header explains the pattern at length; a gate that reds on its own
    #    documentation is a gate someone deletes.
    with Repo() as r:
        r.write("contracts/script/Doc.s.sol",
                '// never write vm.envOr("A", vm.envAddress("B")) - the default is eager\n'
                '/* not this either: vm.envOr("A", vm.envAddress("B")) */\n'
                'contract D { function run() external {} }\n')
        check("a comment explaining the ban does not trip it",
              results(P.gate_deploy_scripts).get(key_eager) is True)

    # 5. every offender is reported, not just the first one found. `results()`
    #    clears RESULTS, so the detail string is read from the gate directly.
    with Repo() as r:
        r.write("contracts/script/Bad1.s.sol", BANNED)
        r.write("contracts/script/Bad2.s.sol", BANNED)
        r.write("contracts/script/Good.s.sol", SANCTIONED)
        P.RESULTS.clear()
        P.gate_deploy_scripts()
        detail = next(x["detail"] for x in P.RESULTS if x["check"] == key_eager)
        P.RESULTS.clear()
    check("both offenders are named, not just the first",
          "Bad1.s.sol" in detail and "Bad2.s.sol" in detail, detail)
    check("and the clean file is not named", "Good.s.sol" not in detail, detail)

    # 6. the secret scan. Deliberately asymmetric with #4: a 64-hex literal in a
    #    COMMENT is still a disclosed key, so this one reads the raw text.
    with Repo() as r:
        r.write("contracts/script/Key.s.sol",
                "contract D { uint256 pk = 0x" + "ab" * 32 + "; }\n")
        check("a 64-hex literal is caught",
              results(P.gate_deploy_scripts).get(key_secret) is False)
    with Repo() as r:
        r.write("contracts/script/KeyInComment.s.sol",
                "// PRIVATE_KEY=0x" + "ab" * 32 + "\ncontract D {}\n")
        check("a 64-hex literal in a COMMENT is caught too - a pasted key is disclosed",
              results(P.gate_deploy_scripts).get(key_secret) is False)
    with Repo() as r:
        r.write("contracts/script/Addr.s.sol",
                "contract D { address a = 0xDa2a4b23459e9ca88183e990802be644AcA7C4B0; }\n")
        check("a 40-hex address is not mistaken for a key",
              results(P.gate_deploy_scripts).get(key_secret) is True)


def test_holder_faucet_stays_disarmed():
    """SECURITY-TODO §0.5 — the shipped default IS the mitigation, so the gate
    that guards it must fail in all three directions: default flipped, armed in
    the environment, and the file it reads gone missing (the `src_required`
    lesson, applied to a new gate)."""
    print("the holder-claim faucet (§0.5)")
    key_default = "the holder-claim faucet ships disarmed (§0.5)"
    key_env = "and it is not armed in this environment"
    saved = os.environ.pop("SCRY_SPOILS_CLAIM", None)
    try:
        with Repo() as r:
            r.write("meter/tokens.py", 'CLAIM_ENABLED = os.getenv("SCRY_SPOILS_CLAIM", "0") == "1"\n')
            got = results(P.gate_holder_faucet)
            check("the shipped default passes", got.get(key_default) is True)
            check("an unset environment passes", got.get(key_env) is True)

            # the default flipped in the code — the fresh-deploy protection
            r.write("meter/tokens.py", 'CLAIM_ENABLED = os.getenv("SCRY_SPOILS_CLAIM", "1") == "1"\n')
            check("a flipped default is refused",
                  results(P.gate_holder_faucet).get(key_default) is False)

            # armed in THIS shell — the protection for this run
            r.write("meter/tokens.py", 'CLAIM_ENABLED = os.getenv("SCRY_SPOILS_CLAIM", "0") == "1"\n')
            os.environ["SCRY_SPOILS_CLAIM"] = "1"
            got = results(P.gate_holder_faucet)
            check("an armed environment is refused", got.get(key_env) is False)
            check("and the default is still read correctly beside it",
                  got.get(key_default) is True)
            os.environ.pop("SCRY_SPOILS_CLAIM")

            # a value that is not "1" arms nothing, and must not red the run
            os.environ["SCRY_SPOILS_CLAIM"] = "0"
            check("an explicit 0 in the environment passes",
                  results(P.gate_holder_faucet).get(key_env) is True)
            os.environ.pop("SCRY_SPOILS_CLAIM")

        # tokens.py absent: the gate reads a DEFAULT out of source, so a missing
        # file must be a red, never a pass for want of data.
        with Repo():
            check("an unreadable tokens.py is a red, not a vacuous pass",
                  results(P.gate_holder_faucet).get(key_default) is False)
    finally:
        os.environ.pop("SCRY_SPOILS_CLAIM", None)
        if saved is not None:
            os.environ["SCRY_SPOILS_CLAIM"] = saved


if __name__ == "__main__":
    for fn in (test_negative_gate_cannot_pass_vacuously, test_src_does_not_cross_read,
               test_src_required_reports, test_snapshot_verification_needs_evidence,
               test_snapshot_freshness_uses_its_own_clock,
               test_a_pinned_drop_is_not_an_arming_drop, test_drop_claim_isolation,
               test_pool_float_survives_markdown,
               test_published_plans_are_immutable_commitments,
               test_drop_being_graded_is_named,
               test_seed_script_matches_the_decision_table,
               test_house_never_claims_from_its_own_drop,
               test_season_pot_and_tvl_tripwires,
               test_powder_tracks_the_drop,
               test_table_pot_obeys_the_season_pot_rule,
               test_merkle_totals_must_be_derived,
               test_deploy_script_scanner_discriminates,
               test_holder_faucet_stays_disarmed,
               test_real_repo_gates_pass):
        fn()
    print(f"\npreflight: {PASS} passed, {FAIL} failed")
    sys.exit(1 if FAIL else 0)
