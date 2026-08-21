#!/usr/bin/env python3
"""Every `immutable` in src/, classified — and a gate against welding a new one.

⚠ WHY THIS EXISTS. The operator has asked repeatedly not to lock these
contracts down (`SENTENCES.md` 2026-08-17: *"make sure no taxes get locked
in"*; 2026-08-19: *"we should be able to adjust anything freely… that's calling
for a redeploy in the future if something changes"*). It kept being answered in
prose and prose does not stop the next `immutable` from being typed.
`ONE-SHOT.md` §8 item 10 asked for the decision "as one table, not one deploy at
a time" and nobody made it. This is that table, as a script, because a
hand-typed inventory of 106 rows rots the way `contract_map.py` says it does.

⚠ THE RULE WAS REWRITTEN 2026-08-19 AND THE FIRST VERSION WAS TRUST-SHAPED.
It read *"weld a field only if a holder's decision to hand over money depends on
it not changing"* — which quietly welds anything a skeptic might squint at, and
that is the frame the operator struck the same day (*"its no longer about a rug
screen… people want legit products. UX and other things matter more… if
something is fucked up or we need to adjust or change a thing to point to
another token PEOPLE WANT THIS"*). Under the old rule a settable payout token
looked like a liability. It is a feature. THE RULE NOW:

    Weld only where a setter BREAKS THE MECHANISM, or where someone has
    ALREADY PAID against this exact value. Nowhere else. A weld needs a
    reason that is not reputational.

Four classes, and the middle two are what survived the rewrite:

  SETTER     — should be adjustable, and is not today. The default answer.
  MECHANISM  — a setter makes the thing not work. One field qualifies: a
               settable salt commitment is not a commitment, so the reveal
               proves nothing. That is arithmetic, not optics.
  STRUCK     — somebody already paid against this exact number, so moving it
               retroactively rewrites what they bought. ⚠ THE FIX IS ALMOST
               NEVER `immutable` — it is a setter that FREEZES at the first
               commit (first trade, first mint). That keeps the thing
               adjustable in the window where being wrong is likely and
               harmless, which is the window we are actually in.
  IDENTITY   — a setter would name a different thing. A v3 pool's token pair,
               fee tier and tick range ARE the pool. Not over-welding.

⚠ NOTE ON RATES. An earlier draft said a rate must take a CAPPED setter, "both
adjustable and un-ruggable." The second half of that was rug-screen reasoning
and is struck; a bare setter on a fee is fine. Cap one only where an
uncapped value would break arithmetic downstream.

⚠ **THE UNWELD PLAN HAS A SIZE BUDGET, AND TWO CONTRACTS HAVE ALMOST NONE.**
Measured 2026-08-19 with the repo's own settings (solc 0.8.26, `--via-ir`,
`--optimize --optimize-runs 200`), runtime bytes against EIP-170's 24,576:

    EloGameTicket   24,086   →    490 headroom   ⚠ 3 setters recommended here
    EloSeatArt      22,978   →  1,598 headroom
    EloSeat         18,251   →  6,325 headroom
    EloCurve        12,415   →  12,161 headroom
    EloCistern      11,772   →  12,804 headroom
    EloFeeSplitter   4,425   →  20,151 headroom

So "make it a setter" is free advice on four of these and a real constraint on
`EloGameTicket`, where `scry`, `usdg` and `scrySink` are all recommended and
490 bytes may not hold all three. **Measure before promising the change**, and
prefer one `setRails(scry, usdg, sink)` over three setters if it comes to that.

⚠ This was learned the expensive way on `EloSeatArt`: an owner plus
`setText`/`sealText` was built, compiled at **24,492 — 84 bytes under the cap**
— and reverted, because a renderer that cannot survive its next edit is a worse
trap than the weld. Its escape hatch already existed and cost nothing (the
contract holds no user state, so a correction is a fresh deploy plus
`EloSeat.setRenderer`, which stays open until `freezeMetadata()`).
**Not every weld is worth a setter. Compile first.**

    python3 contracts/immutables.py            # the inventory, by class
    python3 contracts/immutables.py --launch   # only the undeployed launch path
    python3 contracts/immutables.py --check    # exit 1 on an UNCLASSIFIED weld
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parent / "src"

SETTER, MECHANISM, STRUCK, IDENTITY = "setter", "mechanism", "struck", "identity"

# The six on the launch path. Nothing here is deployed, so every row below is
# still a free choice — which is the whole point of auditing it now.
LAUNCH_PATH = {
    "EloFeeSplitter.sol", "EloSeat.sol", "EloCistern.sol",
    "EloGameTicket.sol", "EloCurve.sol", "EloLaunchpad.sol",
}

# (contract, field) -> (class, why). The `why` is the argument, not a label —
# if you cannot write one, the field is not classified and --check will say so.
DECIDED: dict[tuple[str, str], tuple[str, str]] = {
    # ── EloFeeSplitter ──────────────────────────────────────────────────────
    ("EloFeeSplitter", "scry"): (SETTER,
        "⭐ THE HEADLINE. This one field welds the splitter to a token, and "
        "EloSeat.feeSplitter + EloGameTicket.scrySink weld to the splitter — "
        "so a reserve change forces a redeploy of all three IN ORDER "
        "(ONE-SHOT.md §7 step 9). A setToken deletes that entire launch-day "
        "ordering hazard, and 'point it at another token' is the operator's "
        "own example of what people want."),

    # ── EloSeat ─────────────────────────────────────────────────────────────
    ("EloSeat", "maxSupply"): (STRUCK,
        "somebody mints 1 of 8,192 and that is what they bought. ⚠ But the "
        "honest shape is a DECREASE-ONLY setter, not an immutable — cutting "
        "supply never rewrites a holder's deal, and welding it means a "
        "mis-typed 8,192 is permanent."),
    ("EloSeat", "snapshotCap"): (SETTER,
        "bounded by maxSupply, so a setter cannot inflate the collection — it "
        "only moves allocation between doors that have not been claimed."),
    ("EloSeat", "playCap"): (SETTER,
        "⛔ WELDED AT 0 TODAY, WHICH MEANS A PLAY DOOR CAN NEVER OPEN, at any "
        "price, for the life of the contract. This is the operator's complaint "
        "in its purest form and it is pure downside: bound the SUM at "
        "maxSupply and let unclaimed allocation move."),
    ("EloSeat", "buildCap"): (SETTER, "welded 0, same permanent lockout, same fix."),
    ("EloSeat", "treasuryCap"): (SETTER, "bounded by maxSupply; a setter inflates nothing."),
    ("EloSeat", "reservedUntil"): (SETTER,
        "0 = free forever is the right call and it should still be a call we "
        "can revisit. Welding it buys nothing a setter does not."),
    ("EloSeat", "scry"): (SETTER,
        "⭐ THE OPERATOR'S LITERAL EXAMPLE — 'change a thing to point to "
        "another token.' The activation ladder is denominated in it, so a "
        "change reprices the rungs; that is a reason to announce it, not a "
        "reason to make it impossible."),
    ("EloSeat", "feeSplitter"): (SETTER,
        "this is the immutable that makes the splitter's own weld contagious. "
        "Deploy against a stale splitter and activation does not revert — the "
        "ELO lands and distribute() reverts forever after."),
    ("EloSeat", "proceeds"): (SETTER, "a destination for paid-door ETH."),
    ("EloSeat", "royaltyBps"): (SETTER,
        "marketplaces re-read ERC-2981 live and setters are conventional. "
        "500 bps welded forever is a rate we could never lower."),
    ("EloSeat", "activationBurnBps"): (SETTER,
        "it governs FUTURE activations; changing it rewrites nobody's past "
        "burn. Announce a change, do not forbid one."),
    ("EloSeat", "saltCommitment"): (MECHANISM,
        "⭐ THE ONE GENUINE WELD ON THIS PAGE. A settable commitment is not a "
        "commitment — the reveal would prove nothing at all. This is "
        "arithmetic about what the mechanism can demonstrate, not a signal "
        "aimed at a skeptic, so the 08-19 reframe does not touch it."),

    # ── EloCistern ──────────────────────────────────────────────────────────
    ("EloCistern", "scry"): (SETTER, "the payout token — again, the operator's own example."),
    ("EloCistern", "seat"): (SETTER,
        "the roster source. A setter is how a seat contract that has to be "
        "redeployed does not strand the bar holding funds."),
    ("EloCistern", "router"): (SETTER,
        "a DEX router welded for the life of the contract means the cistern "
        "dies the day that router is deprecated, with funds in it and no path out."),
    ("EloCistern", "electionWidth"): (SETTER, "a shape parameter on the election."),

    # ── EloGameTicket ───────────────────────────────────────────────────────
    ("EloGameTicket", "scry"): (SETTER,
        "a payment rail. Welding it means a new rail is a new ticket contract, "
        "on the one surface whose whole job is conversion."),
    ("EloGameTicket", "usdg"): (SETTER,
        "same, and the constructor refuses zero — so an ETH-only launch still "
        "has to weld a real address it may never use."),
    ("EloGameTicket", "scrySink"): (SETTER,
        "third of the three fields the splitter's weld propagates to."),
    ("EloGameTicket", "proceeds"): (SETTER,
        "⚠ RECLASSIFIED 2026-08-19. This was welded because 'the rug screen "
        "rests on it', which is exactly the struck reasoning. What survives is "
        "narrower and is a BUSINESS term, not a trust signal: on a THIRD-PARTY "
        "listing the studio needs assurance the house cannot redirect their "
        "raise. The ticket deploys per title, so weld it there and leave the "
        "house's own settable."),
    ("EloGameTicket", "supplyCap"): (STRUCK,
        "it bounds free claims and paid sales out of one counter. Decrease-only "
        "again: raising it after a sold-out mint changes what scarcity meant, "
        "lowering it never can. 0 = uncapped is the current call."),

    # ── EloCurve ────────────────────────────────────────────────────────────
    ("EloCurve", "coin"): (IDENTITY, "the pool's own token pair."),
    ("EloCurve", "quote"): (IDENTITY, "a different quote asset is a different pool."),
    ("EloCurve", "positionManager"): (IDENTITY, "the venue the position lives in."),
    ("EloCurve", "v3Factory"): (IDENTITY, "ditto."),
    ("EloCurve", "poolFee"): (IDENTITY, "part of the v3 PoolKey."),
    ("EloCurve", "tickLower"): (IDENTITY, "the position's range."),
    ("EloCurve", "tickUpper"): (IDENTITY, "ditto."),
    ("EloCurve", "virtualQuote"): (STRUCK,
        "the curve's shape, which every buyer priced against. ⚠ Settable UNTIL "
        "THE FIRST TRADE and frozen after is strictly better than immutable: "
        "the window where the number is likely wrong is the window before "
        "anyone has bought, and today we are in it."),
    ("EloCurve", "curveAlloc"): (STRUCK, "same — freeze at first trade, not at deploy."),
    ("EloCurve", "threshold"): (STRUCK,
        "the graduation bar. Same freeze-at-first-trade shape; a bar moved "
        "mid-curve does rewrite what a holder bought."),
    ("EloCurve", "poolAlloc"): (MECHANISM,
        "⭐ NEITHER WELD NOR SET — DERIVE IT. pons computes the same quantity "
        "as supply * phantom / (phantom + threshold), which makes the step "
        "from the curve's close to the pool's open an identity that cannot be "
        "got wrong. Taking it as a free parameter is ONE-SHOT.md §8 item 5, "
        "'the single number most likely to be got wrong in the whole plan.' "
        "Deriving deletes the item; welding and setting both keep it."),
    ("EloCurve", "feeBps"): (SETTER,
        "the operator asked for this by name — no tax locked in. ⚠ A bare "
        "setter is fine; the 'capped so it is un-ruggable' half of the earlier "
        "recommendation was rug-screen reasoning and is struck."),
    ("EloCurve", "protocolShareBps"): (SETTER, "same — bare setter."),
    ("EloCurve", "creatorFeeTo"): (SETTER,
        "one of FOUR welded fee destinations on a PER-GAME-COIN contract, so "
        "every future routing change is a redeploy of a live pool's fee path. "
        "ONE-SHOT.md §8 item 10 flags all four as welded with no sentence."),
    ("EloCurve", "protocolFeeTo"): (SETTER, "one of the four."),
    ("EloCurve", "quoteFeeRecipient"): (SETTER, "one of the four."),
    ("EloCurve", "coinFeeRecipient"): (SETTER, "one of the four."),

    # ── EloLaunchpad ────────────────────────────────────────────────────────
    ("EloLaunchpad", "scry"): (SETTER, "the raise's denomination — repointable."),
    ("EloLaunchpad", "coin"): (IDENTITY, "the pair."),
    ("EloLaunchpad", "weth"): (IDENTITY, "the rail the pool is built on."),
    ("EloLaunchpad", "positionManager"): (IDENTITY, "the venue."),
    ("EloLaunchpad", "v3Factory"): (IDENTITY, "ditto."),
    ("EloLaunchpad", "router"): (SETTER, "same deprecation argument as the cistern's."),
    ("EloLaunchpad", "poolFee"): (IDENTITY, "part of the PoolKey."),
    ("EloLaunchpad", "convertFee"): (SETTER, "a swap route."),
    ("EloLaunchpad", "scryFeeRecipient"): (SETTER,
        "⚠ FEES, NOT THE RAISE. The vault's raise path is a different question "
        "and is a term we offer third-party studios — a business commitment, "
        "not a trust signal. Moving where fee income lands is not moving where "
        "the raise can go."),
    ("EloLaunchpad", "coinFeeRecipient"): (SETTER, "fees, not the raise."),
}

DECL = re.compile(r'^\s*([A-Za-z_][\w\[\]]*)\s+public\s+immutable\s+(\w+)\s*;', re.M)


def inventory() -> list[tuple[str, str, str]]:
    """(file, contract, field) for every public immutable, comments stripped."""
    out = []
    for f in sorted(SRC.glob("*.sol")):
        body = re.sub(r'/\*.*?\*/', '', re.sub(r'//.*', '', f.read_text()), flags=re.S)
        for _type, name in DECL.findall(body):
            out.append((f.name, f.stem, name))
    return out


def main() -> int:
    args = set(sys.argv[1:])
    inv = inventory()
    if "--launch" in args:
        inv = [r for r in inv if r[0] in LAUNCH_PATH]

    unclassified = [(c, n) for f, c, n in inv if (c, n) not in DECIDED]
    by_class: dict[str, list] = {SETTER: [], MECHANISM: [], STRUCK: [], IDENTITY: []}
    for f, c, n in inv:
        if (c, n) in DECIDED:
            by_class[DECIDED[(c, n)][0]].append((c, n))

    if "--check" in args:
        # The gate. A NEW immutable nobody argued for is the failure — not the
        # count, which is allowed to be whatever the reviewed table says.
        launch = [r for r in inventory() if r[0] in LAUNCH_PATH]
        bad = [(c, n) for f, c, n in launch if (c, n) not in DECIDED]
        if bad:
            print(f"immutables: FAIL — {len(bad)} welded field(s) on the launch path "
                  f"that nobody classified:")
            for c, n in bad:
                print(f"  {c}.{n}")
            print("\n  Every immutable is a redeploy the day it has to change. Add a row to")
            print("  DECIDED in this file with a reason that is NOT reputational —")
            print("  MECHANISM if a setter breaks the thing, STRUCK if somebody already")
            print("  paid against this exact value, IDENTITY if a setter names a different")
            print("  thing. Otherwise it is a SETTER and should not be immutable.")
            return 1
        n = len([1 for f, c, x in launch if DECIDED[(c, x)][0] == SETTER])
        print(f"immutables: ok — all {len(launch)} launch-path welds are classified; "
              f"{n} should be setters and are not")
        return 0

    print(f"\n{len(inv)} public immutable(s)"
          + (" on the launch path" if "--launch" in args else " across src/") + "\n")
    for cls, head in ((SETTER, "SETTER — should be adjustable and is not"),
                      (STRUCK, "STRUCK — freeze at first commit, not at deploy"),
                      (MECHANISM, "MECHANISM — a setter breaks the thing"),
                      (IDENTITY, "IDENTITY — a setter would name a different thing")):
        rows = [r for r in by_class[cls] if not ("--launch" in args)
                or f"{r[0]}.sol" in LAUNCH_PATH]
        print(f"── {head} ({len(rows)}) " + "─" * max(0, 52 - len(head)))
        for c, n in rows:
            print(f"  {c}.{n}\n      {DECIDED[(c, n)][1]}")
        print()
    if unclassified:
        print(f"── UNCLASSIFIED ({len(unclassified)}) — outside the launch path, "
              "not yet argued either way")
        for c, n in unclassified:
            print(f"  {c}.{n}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
