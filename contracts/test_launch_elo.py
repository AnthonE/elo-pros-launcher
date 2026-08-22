#!/usr/bin/env python3
"""Laws for `launch_elo.sh` — the ceremony around the one unpatchable transaction.

Bare python: stdlib only, no network, no keys, no `cast`, no venv. A gate whose
own test needs the service's venv is a gate whose test does not run — and this
one guards a transaction that cannot be redone at any price.

WHAT IS ACTUALLY CHECKED, and how. The phases that read the chain cannot be
executed here, so they are held by their TEXT. Everything that can be RUN is run
against a scratch elo.env, because four of the laws below were red when they
were written and a structural assertion would not have caught any of them:

  - `load_env` sourced a RELATIVE path (`. "./$ELO_ENV"`), so any absolute
    ELO_ENV died with "No such file or directory" on a path that plainly exists.
  - elo.env is sourced with `set -a` and carries `ELO_CONFIRM=` BLANK, so it
    overwrote the operator's own export. The one invocation the template
    documents — `ELO_CONFIRM=ELO ./launch_elo.sh launch --arm` — could never
    have worked, and the failure reads as "you forgot to type the word".
  - two phases piped JSON into `python - <<EOF`. The heredoc supplies the
    PROGRAM on stdin, so `json.load(sys.stdin)` re-read the exhausted heredoc:
    an empty parse that presents as an unreachable factory rather than as a
    bug in the shell.
  - `knobs` demanded a python that could import the composer, which made
    "read back the figures I am about to weld" the one phase you could not run
    without the service venv.

Run it anywhere:
    python3 contracts/test_launch_elo.py
"""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "launch_elo.sh"
TEMPLATE = HERE / "elo.env.example"

_fails: list[str] = []
_passed = 0


def check(label: str, cond: bool) -> None:
    global _passed
    if cond:
        _passed += 1
        print(f"  ok  {label}")
    else:
        _fails.append(label)
        print(f"  FAIL {label}")


SRC = SCRIPT.read_text() if SCRIPT.exists() else ""
TPL = TEMPLATE.read_text() if TEMPLATE.exists() else ""


def run(args, env_extra=None, cwd=None):
    """Run the orchestrator with a clean-ish environment."""
    env = dict(os.environ)
    env.pop("ELO_CONFIRM", None)
    env.update(env_extra or {})
    return subprocess.run([str(SCRIPT), *args], capture_output=True, text=True,
                          env=env, cwd=cwd or str(HERE), timeout=120)


def filled(**over) -> str:
    """The template with every refused field given a value."""
    vals = {"ELO_DEPLOYER": "0x" + "11" * 20, "ELO_BUYBACK_ENABLED": "false",
            "ELO_CREATOR_TAX_BPS": "0", "ELO_OPENING_BUY_ETH": "0.25"}
    vals.update(over)
    out = TPL
    for k, v in vals.items():
        out = re.sub(rf"(?m)^{k}=.*$", f"{k}={v}", out)
    return out


print("\n[the two files exist and the script is executable]")
check("contracts/launch_elo.sh exists", SCRIPT.exists())
check("contracts/elo.env.example exists", TEMPLATE.exists())
check("launch_elo.sh is executable — a runbook you have to chmod is a runbook "
      "someone runs with `sh`, losing every `set -e` guarantee",
      SCRIPT.exists() and os.access(SCRIPT, os.X_OK))
check("it is `set -euo pipefail` — an unset variable in a value field must abort, "
      "not expand to empty", "set -euo pipefail" in SRC)

# ── 1 · dry run is the default ───────────────────────────────────────────────
print("\n[dry run by default — the deploy_town.sh bargain]")
check("`--arm` is the only thing that broadcasts", 'armed() {' in SRC and '--arm' in SRC)
_lines = SRC.splitlines()
_sends = [(i, ln) for i, ln in enumerate(_lines) if re.search(r"\bcast send\b", ln)]
# A `cast send` inside a note/printf is text telling the operator what to run.
_real = [(i, ln) for i, ln in _sends if not ln.strip().startswith(("note", "printf", "#"))]
_guarded = [(i, ln) for i, ln in _real
            if any('ARM' in prev for prev in _lines[max(0, i - 40):i])]
check(f"every executable `cast send` sits behind an ARM guard "
      f"({len(_sends)} occurrences, {len(_sends) - len(_real)} of them printed "
      f"instructions, {len(_guarded)}/{len(_real)} guarded)",
      len(_real) >= 2 and len(_guarded) == len(_real))
check("a broadcast needs PRIVATE_KEY from the ENVIRONMENT and the script says so",
      "--arm needs PRIVATE_KEY" in SRC)
check("no private key, mnemonic or keystore path is read from elo.env",
      "PRIVATE_KEY" not in TPL.replace("PRIVATE_KEY IS NOT IN THIS FILE", "")
      .replace("PRIVATE_KEY — export it", "").replace("$PRIVATE_KEY", "")
      or "must not be added to it" in TPL.lower())

# ── 2 · the welded knobs have no defaults ────────────────────────────────────
print("\n[a welded knob has no default — the SEAT_SCRY lesson, executed]")
# ⚠ TWO LISTS, AND THE SPLIT IS THE POINT. `DEMANDED` is every knob
# `require_knobs` refuses to default — blank is refused for all three, because
# false/0 being the RECOMMENDATION is exactly why a blank must not silently
# become it. `WELDED` is the subset the launch transaction makes permanent.
#
# ⚠ ELO_BUYBACK_ENABLED WAS IN BOTH UNTIL 2026-08-21 AND BELONGED IN ONE.
# `setBuybackEnabled(address,bool)` is on the verified factory ABI, so it is a
# setter (SENTENCES.md 2026-08-21; pons' docs: creator-only to turn ON). The law
# below therefore inverts for it: an ANSWERED decision must look answered, so
# the template STAGES `false` and a blank template is the failure. The
# still-welded two keep the original law — an unmade decision must look unmade.
DEMANDED = ("ELO_BUYBACK_ENABLED", "ELO_CREATOR_TAX_BPS", "ELO_OPENING_BUY_ETH")
WELDED = ("ELO_CREATOR_TAX_BPS", "ELO_OPENING_BUY_ETH")
STAGED = {"ELO_BUYBACK_ENABLED": "false"}
for k in WELDED:
    check(f"{k} is present in the template and BLANK — an unmade decision must "
          f"look unmade", re.search(rf"(?m)^{k}=\s*$", TPL) is not None)
for k, v in STAGED.items():
    check(f"{k} is STAGED at {v!r} in the template — a decision that HAS been "
          f"made must look made (SENTENCES.md 2026-08-21)",
          re.search(rf"(?m)^{k}={v}\s*$", TPL) is not None)

with tempfile.TemporaryDirectory() as td:
    blank = Path(td) / "blank.env"
    # ⚠ The template no longer refuses BY ITSELF on every knob: buyback is
    # staged now, so a "fresh template" for the purpose of this law is the
    # template with the STAGED answers cleared back out. Writing TPL verbatim
    # would test that a staged file refuses, which is the opposite law.
    blank_txt = TPL
    for k in STAGED:
        blank_txt = re.sub(rf"(?m)^{k}=.*$", f"{k}=", blank_txt)
    blank.write_text(blank_txt)
    r = run(["knobs"], {"ELO_ENV": str(blank)})
    check("`knobs` on a fresh template REFUSES (exit 1) rather than defaulting",
          r.returncode == 1)
    for k in WELDED:
        check(f"…and names {k} in the refusal", k in r.stdout + r.stderr)
    check("…and it runs with no venv, no cast and no network — reading back what "
          "you are about to weld is the one phase that must work anywhere",
          "UNSPOKEN" in r.stdout + r.stderr)

    # ⚠ blank must not be read as false. This is the whole point: false is also
    # the RECOMMENDATION for buyback, so a blank silently defaulting to it would
    # produce the right answer for no reason and look identical afterwards.
    b2 = Path(td) / "b2.env"
    b2.write_text(filled(ELO_BUYBACK_ENABLED=""))
    r2 = run(["knobs"], {"ELO_ENV": str(b2)})
    check("a BLANK buybackEnabled is refused, not read as false — even though "
          "false is what ONE-SHOT.md §3b recommends", r2.returncode == 1)

    ok = Path(td) / "ok.env"
    ok.write_text(filled())
    r3 = run(["knobs"], {"ELO_ENV": str(ok)})
    check("a fully-typed elo.env passes `knobs`", r3.returncode == 0)
    check("…and prints the welded set under a heading that says so",
          "WELDED BY THIS TRANSACTION" in r3.stdout)

    # ⚠ THE HEADING IS THE PRODUCT HERE. This screen is what the operator reads
    # in the minutes before an unpatchable transaction, and it listed
    # buybackEnabled under "no setter, at any price" for a knob that has one.
    # A figure filed under the wrong permanence is worse than an unlisted one:
    # it makes a reversible decision feel final and a final one feel routine.
    welded_block = r3.stdout.split("WELDED BY THIS TRANSACTION")[-1]
    welded_block = welded_block.split("SETTERS")[0]
    check("…and buybackEnabled is NOT in it — it is a setter "
          "(setBuybackEnabled, verified factory ABI; SENTENCES.md 2026-08-21)",
          "buybackEnabled" not in welded_block)
    check("…it is filed under SETTERS instead, named with the call that moves it",
          "SETTERS" in r3.stdout and "setBuybackEnabled" in r3.stdout)
    for k in ("creatorTaxBps", "opening buy", "salt"):
        check(f"…and {k} is still under the welded heading", k in welded_block)

    # ── 3 · an ABSOLUTE ELO_ENV works ────────────────────────────────────────
    print("\n[the ledger path]")
    check("an ABSOLUTE ELO_ENV is sourced correctly (`.` searches PATH for a "
          "bare name, so only a relative path may take the ./)",
          r3.returncode == 0 and "No such file" not in r3.stderr)

    missing = run(["knobs"], {"ELO_ENV": str(Path(td) / "nope.env")})
    check("a missing elo.env dies pointing at the template, not at a traceback",
          missing.returncode == 1 and "elo.env.example" in missing.stderr)

# ── 4 · the environment beats the file ───────────────────────────────────────
print("\n[the environment beats the file — ELO_CONFIRM is carried BLANK]")
check("the precedence is implemented, not just intended",
      "_ENV_CONFIRM" in SRC and 'ELO_CONFIRM="$_ENV_CONFIRM"' in SRC)
check("…and it is captured BEFORE any default is applied or elo.env is sourced",
      SRC.index("_ENV_CONFIRM=") < SRC.index("load_env()"))
check("the template documents the invocation that depends on it",
      "ELO_CONFIRM" in TPL)
check("the script's own hint prints that invocation",
      "ELO_CONFIRM=$ELO_SYMBOL" in SRC)

# ── 5 · the pin is re-read at broadcast ──────────────────────────────────────
print("\n[the pin — re-read at broadcast, not trusted from the plan]")
check("`launch` recomposes from scratch rather than replaying the plan file",
      "recomposing from scratch" in SRC)
check("…on the PLAN'S OWN SALT, or the recomposition would mint a fresh random "
      "CREATE2 address and the comparison would compare nothing",
      "ELO_SALT_USED" in SRC and 'do_compose "$ELO_SALT_USED"' in SRC)
check("a moved economics digest ABORTS instead of broadcasting either version",
      "THE ECONOMICS MOVED" in SRC)
check("…and the abort explains both wrong outcomes it is preventing",
      "would revert (LaunchEconomicsMismatch)" in SRC
      and "terms nobody read" in SRC)
check("calldata that changed with the pin UNMOVED is also caught — that is a "
      "knob edited after the plan, not pons moving",
      "the calldata changed between plan and now" in SRC)

# ── 6 · the guards at the moment of broadcast ────────────────────────────────
print("\n[the moment of broadcast]")
check("the signing key's own address is checked against the preflighted deployer",
      "cast wallet address" in SRC and "ELO_DEPLOYER is" in SRC)
check("a typed confirmation is required, and it is tied to the ticker",
      '"${ELO_CONFIRM:-}" = "$ELO_SYMBOL"' in SRC)
check("the value is printed in wei AND in ETH before it is sent — the "
      "miscounted zero is the failure TICKET.md §8d exists for",
      "READ THIS DIGIT BY DIGIT" in SRC)
check("…and as a share of the graduation threshold, which is the figure that "
      "makes a wrong magnitude obvious", "% of it" in SRC)
check("the exact calldata is SIMULATED against the chain before sending",
      SRC.count("cast call") >= 2 and "simulation reverted at head" in SRC)
check("a broadcast that returns no hash warns against blind retry — the salt is "
      "frozen, so a second attempt reverts while the first may be live",
      "DO NOT simply re-run" in SRC)

# ── 7 · the factory identity ─────────────────────────────────────────────────
print("\n[WHICH pons v2 — the unpatchable confusion]")
check("`check` refuses the v3-generation PonsLaunchFactory by address shape",
      "0xa5aab3f0*1feb" in SRC)
check("…and confirms the v2 PRODUCT positively, by a function only it answers",
      "previewLaunchEconomics" in SRC and "PonsV2LaunchFactory ok" in SRC)
check("the whole stack is checked for code, because a hook binds to ONE factory "
      "permanently and the set travels together",
      "cast code" in SRC and "feeEscrow" in SRC and "wrong escrow reports ZERO" in SRC)

# ── 8 · the shape of the reads ───────────────────────────────────────────────
print("\n[reads]")
check("selectors and event topics are DERIVED by keccak from signatures, never "
      "pasted as 4-byte constants",
      "sel()" in SRC and "keccak256" in SRC
      and not re.search(r"S_[A-Z_]+=0x[0-9a-fA-F]{8}\b", SRC))
check("the launch record is read as WORDS, so a field added upstream cannot "
      "silently shift what we think we read",
      "is word 10" in SRC and 'word "$raw" 10' in SRC)
check("the watch loop is paced on WALL CLOCK, not block counts — RH-Chain is "
      "Arbitrum Nitro and solidity's block.number runs ~120x slow",
      "wall clock" in SRC.lower() and "sleep 15" in SRC)
check("a phase-1 stall is treated as a permissionless WORK QUEUE, not a fault",
      "createGraduatedPool" in SRC and "work" in SRC.lower())
# ⚠ A never-raises reader returns the same thing for "nothing there" and "we
# could not look". A short return would print `phase ` with an empty value,
# which reads as a healthy zero — NotGraduated — on the card.
check("a short or failed launch-record read REFUSES rather than printing a "
      "blank phase that reads as a healthy zero",
      "require_record" in SRC and SRC.count("require_record") >= 3
      and "960" in SRC)
check("…and the refusal names the real cause: a token launched against an "
      "OLDER pons set is unknown to this factory but still trades",
      "OLDER pons set" in SRC)

# ── 9 · no data is piped into a heredoc-fed python ───────────────────────────
print("\n[the shell trap that produced an empty parse]")
# `python - <<EOF` takes its PROGRAM from stdin. Piping data in as well hands
# the parser the heredoc it already consumed.
bad = re.findall(r"\|\s*\"\$PY\"\s+-\s+[^\n]*<<", SRC)
check("no phase pipes data into `\"$PY\" - <<HEREDOC` (the program comes from "
      f"stdin, so the pipe is silently discarded) — {len(bad)} found", not bad)
check("the phases that parse JSON hand it over as a FILE PATH in argv",
      'json.load(open(sys.argv[1]))' in SRC)
check("…and an empty composer output is caught explicitly rather than parsed",
      "printed nothing" in SRC)

# ── 9b · the two rates, and the dollar sizing ────────────────────────────────
print("\n[2% total is TWO sums — the curve's feeBps and the hook's hookFeeBps]")
check("a `fees` phase exists and reads BOTH rates",
      "\nfees)" in SRC and "hookFeeBps" in SRC and "curve feeBps" in SRC)
check("…and says plainly that the curve fee is pons', not ours",
      "NOT ours to set" in SRC)
check("…and WARNS when the two rates differ, because one creator tax cannot "
      "make the same total in both venues and the tax has no setter",
      "THE RATES DIFFER" in SRC)
check("…and reports whether the reference actually proves a tax survives "
      "graduation, rather than assuming it from pons' prose",
      "phase == 2 and ref_tax > 0" in SRC and "rather than from pons' prose" in SRC)
check("it refuses without a reference launch — ELO has no policy until it exists",
      "ELO has no policy until it exists" in SRC)

print("\n[a dollar figure is converted, never guessed]")
check("`size --usd` requires an explicit rate and says nothing here fetches one",
      "--usd needs a rate" in SRC and "Nothing in this repo fetches one" in SRC)
check("the conversion uses Decimal, not float — it produces a WELDED value",
      "from decimal import Decimal" in SRC and "ROUND_DOWN" in SRC)
check("…and shows the one-decimal-out neighbours, which is what makes a "
      "miscounted zero visible", "one decimal out" in SRC)

# ── 10 · the template and the script agree ───────────────────────────────────
print("\n[the template names every knob the script reads]")
read_keys = sorted(set(re.findall(r"\$\{(ELO_[A-Z_]+?)(?::-[^}]*)?\}", SRC)))
derived = {"ELO_ENV", "ELO_PY", "ELO_SALT_USED", "ELO_PIN_AT_PLAN", "ELO_TOKEN",
           "ELO_CURVE", "ELO_LAUNCH_TX"}   # written BY the script, not typed
missing = [k for k in read_keys if k not in derived and f"\n{k}=" not in TPL]
check(f"every knob the script reads is in the template ({len(read_keys)} read, "
      f"{len(derived)} of them script-written) — missing: {missing or 'none'}",
      not missing)

# ── 11 · record ──────────────────────────────────────────────────────────────
print("\n[record — the step that makes the rename land]")
check("it writes deployments.json under the key reserve.py resolves",
      "RESERVE_KEY" in SRC and "contracts" in SRC)
check("…and curve.config.json's `token`, which is the browser's half",
      "curve.config.json" in SRC and 'd["token"] = token' in SRC)
check("both paths are overridable so the write can be REHEARSED (ONE-SHOT.md "
      "§7 step 0c) against copies",
      'CURVE_CONFIG="${CURVE_CONFIG:-' in SRC and 'DEPLOYMENTS="${DEPLOYMENTS:-' in SRC)
check("a REVERTED transaction records nothing",
      "REVERTED on chain" in SRC)
check("the token and curve come from INDEXED topics, not from decoded data",
      "topics" in SRC and "t[1][-40:]" in SRC)
check("it prints the edits it does NOT make — the pools and listings rows still "
      "name the retired coin by hand (ONE-SHOT.md §2)",
      "pools.config.json" in SRC and "listings.json" in SRC)
check("…and the untracked-file trap before the release",
      "--porcelain" in SRC)

# ── 12 · it composes nothing itself ──────────────────────────────────────────
print("\n[the division of labour]")

# The orchestrator must never assemble a launch's arguments itself. It may build
# a SELECTOR plus one padded address for a read (`getLaunchedToken`,
# `createGraduatedPool`) — that is not a launch and carries no money. What it
# must never do is encode TokenParams, which is a dynamic tuple inside a dynamic
# tuple: the shape a hand-rolled encoder gets wrong while still producing
# well-formed calldata, and the reason test_launch.py checks it against a launch
# the chain already accepted.
check("the launch's calldata is TAKEN from the composer, never assembled here",
      'DATA=$(plan_field "$fresh" tx.data)' in SRC
      and not re.search(r"(TokenParams|string,string,string)", SRC.replace(
          '"$FN"', "").replace("function", "")))
check("…and the value likewise, so the fee/buy split is never re-derived in bash",
      'VALUE=$(plan_field "$fresh" tx.value)' in SRC)
check("plan and launch compose through ONE function, or the comparison at "
      "broadcast would be theatre",
      SRC.count("do_compose") >= 3 and "compose_args()" in SRC)
check("the composer is run with a python proven to import it — a bare-python "
      "run that silently fails to import must abort, not compose nothing",
      "no python here can import" in SRC)

print()
if _fails:
    print(f"FAILED {len(_fails)}/{_passed + len(_fails)}:")
    for f in _fails:
        print(f"  - {f}")
    sys.exit(1)
print(f"all good — {_passed} checks pass (the ceremony refuses a blank welded knob, "
      f"re-reads the pin at broadcast, and records nothing on a revert)")
