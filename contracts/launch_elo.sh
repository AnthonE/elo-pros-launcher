#!/usr/bin/env bash
# launch_elo.sh - phase-gated orchestrator for the reserve's own launch (ELO)
# on pons v2, chain 4663.
#
# The runbook this script drives is docs/launch/ELO-LAUNCH.md §5 and
# docs/launch/ONE-SHOT.md §7. It is `deploy_town.sh` for the one transaction
# those pages call the barrier: everything else in the platform's launch takes
# ELO's address as a constructor argument with no setter, so nothing below it
# can even be composed until this has confirmed.
#
# House rules it ENFORCES rather than documents:
#   - DRY RUN by default. Broadcasting takes --arm, a PRIVATE_KEY whose own
#     address matches the deployer that was preflighted, and a typed ELO_CONFIRM.
#   - NOTHING IS COMPOSED HERE. Every byte of calldata comes from
#     meter/launch.py, which reads each figure back off chain and pins it. This
#     script is the ceremony; that module is the arithmetic.
#   - THE PIN IS RE-READ AT BROADCAST. `plan` records the economics digest;
#     `launch --arm` recomposes from scratch and ABORTS if it moved. A pinned
#     launch reverts on moved terms, which is safe but wastes the fee; catching
#     it locally costs nothing.
#   - A WELDED KNOB HAS NO DEFAULT. A blank buyback/tax/opening-buy is refused,
#     never defaulted - a welded figure that lands on its recommendation by
#     accident is indistinguishable afterwards from one somebody chose.
#   - addresses flow between phases through elo.env (written by this script,
#     sourced on every run) - no copy-paste drift.
#
# Usage:
#   ./launch_elo.sh check                # tooling, chain, and WHICH FACTORY
#   ./launch_elo.sh knobs                # the welded decisions; refuses blanks
#   ./launch_elo.sh preflight            # every pons gate, on the signing wallet
#   ./launch_elo.sh size                 # what the opening buy does to the price
#   ./launch_elo.sh plan                 # the pinned, unsigned transaction
#   ./launch_elo.sh launch    [--arm]    # THE ONE TRANSACTION (unpatchable)
#   ./launch_elo.sh record               # receipt -> deployments.json + curve.config.json
#   ./launch_elo.sh watch     [--arm]    # phase poller; --arm clears a stalled phase 1
#   ./launch_elo.sh status               # where the launch is, read off chain
#
# Env this script reads (never echoed):
#   PRIVATE_KEY   required to --arm. The launch is signed by the operator's own
#                 wallet on the box - lane A (CLAUDE.md, two lanes). The meter
#                 never holds it and never broadcasts.
#   ELO_ENV       the figures + the address ledger. Default ./elo.env
#   ELO_PY        the python that can import meter/launch.py. Auto-detected;
#                 bare python3 usually CANNOT (fastapi), and a module that fails
#                 to import must abort rather than skip.
#   RPC           default https://rpc.mainnet.chain.robinhood.com
#
# ⚠ WHAT THIS SCRIPT CANNOT DO FOR YOU. It checks that a transaction is
#   well-formed, pinned, affordable and simulated. It cannot check that the
#   knobs are the ones you meant. Four of them are welded and two of those
#   (ELO_BUYBACK_ENABLED, ELO_CREATOR_TAX_BPS) had no operator sentence when
#   this was written - ONE-SHOT.md §8 is the open list.
set -euo pipefail

cd "$(dirname "$0")"
REPO_ROOT="$(cd .. && pwd)"

# ⚠ THE ENVIRONMENT BEATS THE FILE, and it has to be captured BEFORE any
# default is applied or any sourcing happens. elo.env is sourced with `set -a`,
# so a key it carries BLANK silently unsets whatever the operator exported -
# and ELO_CONFIRM is carried blank on purpose (it is the per-broadcast typed
# word). Without this, the one invocation the template documents,
# `ELO_CONFIRM=ELO ./launch_elo.sh launch --arm`, could never work: the file
# would overwrite it with "" a few lines later. Same rule as the meter's
# SCRY_LAUNCH_FACTORY beating curve.config.json.
_ENV_CONFIRM="${ELO_CONFIRM:-}"
_ENV_RPC="${RPC:-}"

CHAIN_ID_WANT=4663
RPC="${RPC:-https://rpc.mainnet.chain.robinhood.com}"
ELO_ENV="${ELO_ENV:-elo.env}"
PLAN_FILE="${PLAN_FILE:-elo-plan.json}"
# Overridable so `record` can be REHEARSED against copies before the real
# thing - ONE-SHOT.md §7 step 0c wants every downstream act rehearsed against a
# throwaway address, and the two files this writes are the ones a mistake in
# would mislabel the whole store.
CURVE_CONFIG="${CURVE_CONFIG:-$REPO_ROOT/watchtower/curve.config.json}"
DEPLOYMENTS="${DEPLOYMENTS:-deployments.json}"

# The reserve's key in deployments.json. reserve.py reads exactly this
# (RESERVE_KEY, default "ELO") and resolves the whole service off it, which is
# what makes `record` the step that lands the rename on a box that only pulls.
RESERVE_KEY="${SCRY_RESERVE_KEY:-ELO}"

say()  { printf '\n== %s\n' "$*"; }
note() { printf '   %s\n' "$*"; }
die()  { printf '\nABORT: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing tool: $1"; }
lc()   { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }
armed() { [ "${1:-}" = "--arm" ] && echo yes || echo no; }

# ── the python that can actually import the composer ─────────────────────────
# ⚠ A TEST ON BARE PYTHON SILENTLY SKIPS, and the same trap eats a launch: if
# `import launch` fails on a missing fastapi, a naive runner reports nothing
# composed and moves on. This resolves the venv first and PROVES the import
# before any phase depends on it.
pick_python() {
    if [ -n "${ELO_PY:-}" ]; then printf '%s' "$ELO_PY"; return; fi
    for c in /data/apps/scry-deploy/meter/.venv/bin/python \
             "$REPO_ROOT/meter/.venv/bin/python" \
             python3; do
        [ -x "$c" ] || command -v "$c" >/dev/null 2>&1 || continue
        if (cd "$REPO_ROOT/meter" && "$c" -c 'import launch' >/dev/null 2>&1); then
            printf '%s' "$c"; return
        fi
    done
    printf ''
}

PY=""
need_python() {
    [ -n "$PY" ] && return
    PY="$(pick_python)"
    [ -n "$PY" ] || die "no python here can import meter/launch.py.
   The composer needs the service's venv (fastapi + the meter's own modules).
   CLAUDE.md: run against the venv, never bare python -
     ELO_PY=/data/apps/scry-deploy/meter/.venv/bin/python ./launch_elo.sh $CMD
   A composer that fails to import must abort, not compose nothing quietly."
    note "python: $PY"
}

# meter modules import each other by bare name, so they run from meter/.
meter_py() { (cd "$REPO_ROOT/meter" && "$PY" "$@"); }

# ── elo.env: the figures on the way in, the ledger on the way out ────────────
elo_get() { [ -f "$ELO_ENV" ] && grep -E "^$1=" "$ELO_ENV" | tail -1 | cut -d= -f2- | sed 's/^"//; s/"$//' || true; }
elo_set() {
    touch "$ELO_ENV"
    # Two different grep exits, kept apart: 1 is "no such key yet" (the ordinary
    # case), 2 is "I could not do it" - a full disk, a read-only mount, another
    # session racing the mv. Conflating them installs an EMPTY file over the
    # ledger and every later phase reports a live launch as missing.
    local g=0
    grep -vE "^$1=" "$ELO_ENV" > "$ELO_ENV.tmp" || g=$?
    [ "$g" -le 1 ] || die "could not rewrite $ELO_ENV (grep exit $g).
   The ledger is UNTOUCHED and nothing was lost. Fix the filesystem first."
    cp "$ELO_ENV" "$ELO_ENV.bak" 2>/dev/null || true
    mv "$ELO_ENV.tmp" "$ELO_ENV"
    printf '%s=%s\n' "$1" "$2" >> "$ELO_ENV"
    printf '   %s  %s=%s\n' "$ELO_ENV" "$1" "$2"
}

load_env() {
    if [ -f "$ELO_ENV" ]; then
        # `.` searches PATH for a bare name, so a relative path needs the `./`
        # and an absolute one must not get it.
        case "$ELO_ENV" in
            /*) set -a; . "$ELO_ENV";   set +a ;;
            *)  set -a; . "./$ELO_ENV"; set +a ;;
        esac
        # …and hand the operator's own exports back, per the note at the top.
        [ -n "$_ENV_CONFIRM" ] && ELO_CONFIRM="$_ENV_CONFIRM"
        [ -n "$_ENV_RPC" ] && RPC="$_ENV_RPC"
        RPC="${RPC:-https://rpc.mainnet.chain.robinhood.com}"
    else
        die "no $ELO_ENV. Start from the template, which explains every field:
     cp elo.env.example $ELO_ENV"
    fi
}

# ── chain reads ──────────────────────────────────────────────────────────────
# ⚠ RAW CALLDATA, ON PURPOSE. Every read here passes a hex blob rather than a
# solidity signature, because that is the exact path the broadcast takes and
# `check` proves it works before anything arms. A struct getter is also read as
# words rather than decoded, so a field ADDED to pons' struct cannot silently
# shift what we think we read.
raw_call() { cast call "$1" "$2" --rpc-url "$RPC"; }

# word N (0-indexed) of a raw return blob
word() { "$PY" - "$1" "$2" <<'PY'
import sys
blob = sys.argv[1].removeprefix("0x")
i = int(sys.argv[2])
chunk = blob[i * 64:(i + 1) * 64]
print(int(chunk, 16) if chunk else "")
PY
}
word_addr() { "$PY" - "$1" "$2" <<'PY'
import sys
blob = sys.argv[1].removeprefix("0x")
i = int(sys.argv[2])
print("0x" + blob[i * 64:(i + 1) * 64][-40:])
PY
}
# keccak from the meter's own implementation - selectors and topics are DERIVED
# from signatures here, never pasted as 4-byte constants.
sel()   { meter_py -c 'import sys;from keccak import keccak256;print("0x"+keccak256(sys.argv[1].encode()).hex()[:8])' "$1"; }
topic() { meter_py -c 'import sys;from keccak import keccak256;print("0x"+keccak256(sys.argv[1].encode()).hex())' "$1"; }

# ⚠ A NEVER-RAISES READER RETURNS THE SAME THING for "nothing there" and "we
# could not look". `getLaunchedToken` returns 15 static words; anything shorter
# is a failed read or a token this factory never launched, and printing `phase `
# with an empty value reads as a healthy zero. Refuse instead.
require_record() { # $1 = raw blob, $2 = token
    local hex="${1#0x}"
    [ "${#hex}" -ge 960 ] || die "the factory returned no launch record for $2
   (got ${#hex} hex chars, a full record is 960). Either the read failed, or
   this token was not launched by the factory in curve.config.json — a token
   created against an OLDER pons set still trades and still pays out, through
   the hook and escrow it launched against. Look the stack up; do not assume it."
}

cfg_addr() { "$PY" - "$CURVE_CONFIG" "$1" <<'PY'
import json, sys
print((json.load(open(sys.argv[1])).get(sys.argv[2]) or "").strip())
PY
}

# ── the knobs ────────────────────────────────────────────────────────────────
require_knobs() {
    local bad=0
    [ -n "${ELO_DEPLOYER:-}" ] || { printf '   MISSING  ELO_DEPLOYER  (the wallet that signs)\n'; bad=1; }
    [ -n "${ELO_NAME:-}" ]     || { printf '   MISSING  ELO_NAME\n'; bad=1; }
    [ -n "${ELO_SYMBOL:-}" ]   || { printf '   MISSING  ELO_SYMBOL\n'; bad=1; }

    case "${ELO_BUYBACK_ENABLED:-}" in
        true|false) ;;
        *) printf '   UNSPOKEN [WELDED]  ELO_BUYBACK_ENABLED  -> write true or false.\n'
           printf '            Blank is NOT false. It decides whether WE or pons own the\n'
           printf '            timing of every fee sweep, forever. ONE-SHOT.md §3b says false.\n'
           bad=1 ;;
    esac

    case "${ELO_CREATOR_TAX_BPS:-}" in
        ''|*[!0-9]*)
           printf '   UNSPOKEN [WELDED]  ELO_CREATOR_TAX_BPS  -> write an integer (0 is valid).\n'
           printf '            ONE-SHOT.md §3b-b: 125 if the tax rides the v4 hook after\n'
           printf '            graduation, 0 if it dies with the curve. THE SCOPE IS\n'
           printf '            UNCONFIRMED and it decides the answer - ask pons.\n'
           bad=1 ;;
    esac

    case "${ELO_OPENING_BUY_ETH:-}" in
        ''|*[!0-9.]*)
           printf '   UNSPOKEN [WELDED]  ELO_OPENING_BUY_ETH  -> write a number (0 is valid).\n'
           printf '            A zero opening buy is a CHOICE, not a default: this is the only\n'
           printf '            reserve the treasury ever gets without buying more at market.\n'
           bad=1 ;;
    esac

    [ "$bad" = 0 ] || die "the figures above have no value in $ELO_ENV.
   Every one of them is welded by the launch transaction or decides one that is.
   elo.env.example explains each; ONE-SHOT.md §8 is the open decision list."
}

# wei from a decimal ETH string, without bc and without float error
to_wei() { "$PY" - "$1" <<'PY'
import sys
from decimal import Decimal
print(int(Decimal(sys.argv[1]) * Decimal(10) ** 18))
PY
}
eth_of() { "$PY" -c 'import sys;print(f"{int(sys.argv[1])/1e18:.9f}")' "$1"; }

# ── composition: one entry point, driven from the ledger ─────────────────────
# `plan` and `launch` MUST compose identically or the comparison at broadcast is
# theatre, so both call this and nothing else.
compose_args() {
    local salt="${1:-}"
    set -- --deployer "$ELO_DEPLOYER" \
           --name "$ELO_NAME" --symbol "$ELO_SYMBOL" \
           --config-id "${ELO_LAUNCH_CONFIG_ID:-0}" \
           --creator-tax-bps "$ELO_CREATOR_TAX_BPS" \
           --opening-buy-eth "$ELO_OPENING_BUY_ETH" \
           --min-tokens-out "${ELO_MIN_TOKENS_OUT:-0}"
    [ "${ELO_BUYBACK_ENABLED:-}" = true ] && set -- "$@" --buyback
    [ -n "${ELO_PAIR_TOKEN:-}" ]           && set -- "$@" --pair-token "$ELO_PAIR_TOKEN"
    [ -n "${ELO_CREATOR_FEE_RECIPIENT:-}" ] && set -- "$@" --creator-fee-recipient "$ELO_CREATOR_FEE_RECIPIENT"
    [ -n "${ELO_BUY_RECIPIENT:-}" ]        && set -- "$@" --recipient "$ELO_BUY_RECIPIENT"
    [ -n "${ELO_LOGO:-}" ]                 && set -- "$@" --logo "$ELO_LOGO"
    [ -n "${ELO_DESCRIPTION:-}" ]          && set -- "$@" --description "$ELO_DESCRIPTION"
    [ -n "${ELO_SNIPE_EXEMPTIONS:-}" ]     && set -- "$@" --exempt "$ELO_SNIPE_EXEMPTIONS"
    [ "${ELO_ATOMIC:-true}" = false ]      && set -- "$@" --no-atomic
    local socials
    socials="$("$PY" - "${ELO_TWITTER:-}" "${ELO_TELEGRAM:-}" "${ELO_DISCORD:-}" \
                     "${ELO_WEBSITE:-}" "${ELO_FARCASTER:-}" <<'PY'
import json, sys
k = ("twitter", "telegram", "discord", "website", "farcaster")
print(json.dumps(dict(zip(k, sys.argv[1:6]))))
PY
)"
    set -- "$@" --socials-json "$socials"
    [ -n "$salt" ] && set -- "$@" --salt "$salt"
    printf '%s\0' "$@"
}

do_compose() { # $1 = salt (may be empty) -> plan JSON on stdout
    local args=()
    while IFS= read -r -d '' a; do args+=("$a"); done < <(compose_args "${1:-}")
    meter_py launch.py "${args[@]}"
}

plan_field() { "$PY" - "$1" "$2" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
cur = d
for k in sys.argv[2].split("."):
    cur = (cur or {}).get(k) if isinstance(cur, dict) else None
print("" if cur is None else cur)
PY
}

# ── phases ───────────────────────────────────────────────────────────────────
CMD="${1:-help}"
shift || true

case "$CMD" in

check)
    need cast; need_python
    say "chain"
    got=$(cast chain-id --rpc-url "$RPC")
    [ "$got" = "$CHAIN_ID_WANT" ] || die "RPC serves chain $got, want $CHAIN_ID_WANT (RH-Chain)"
    note "RPC -> chain $got ok"

    say "the stack, from watchtower/curve.config.json"
    [ -f "$CURVE_CONFIG" ] || die "no $CURVE_CONFIG - the service reads this exact path"
    FACTORY="$(cfg_addr factory)"; FWD="$(cfg_addr launchAndBuy)"
    [ -n "$FACTORY" ] || die "no \`factory\` in curve.config.json"
    for k in factory hook feeEscrow buybackVault locker launchAndBuy launchDeployer; do
        a="$(cfg_addr "$k")"
        [ -n "$a" ] || die "curve.config.json is missing \`$k\`.
   ⚠ A pons hook binds to ONE factory permanently, so the launchpad is replaced
   as a whole SET rather than upgraded in place. Reading a creator balance from
   the wrong escrow reports ZERO rather than failing."
        c=$(cast code "$a" --rpc-url "$RPC")
        [ "$c" != "0x" ] || die "no code at $k $a on chain $got"
        note "$k $a ok ($((${#c} / 2 - 1)) bytes)"
    done

    say "WHICH pons v2 - the trap that is unpatchable if it is got wrong"
    # ⚠ "pons v2" names two different contracts and both are in pons' own docs.
    # 0xA5aAb3F0…1feB is `PonsLaunchFactory`, the v3-POOL generation SCRY itself
    # launched through, where "V2" means their 70/30 FEE generation. The product
    # Pons V2 - bonding curve, Uniswap v4, ETH creator payouts - is
    # `PonsV2LaunchFactory`. Launching against the wrong one cannot be undone.
    f_lc="$(lc "$FACTORY")"
    case "$f_lc" in
        0xa5aab3f0*1feb)
            die "the configured factory is the V3-GENERATION PonsLaunchFactory
   ($FACTORY), not PonsV2LaunchFactory. Their docs call it \"Active Factory
   (V2)\" and mean the FEE generation, not the product. A launch there is not
   patchable. ELO-LAUNCH.md §0." ;;
    esac
    # The positive tell: only the v2 product answers previewLaunchEconomics.
    # This is also the gate that proves `cast` accepts RAW CALLDATA positionally,
    # which is exactly how the broadcast passes its own data. If this call is
    # rejected on syntax, every later phase would have failed at the worst
    # possible moment instead.
    S_PREVIEW="$(sel 'previewLaunchEconomics(uint256,address)')"
    pin=$(raw_call "$FACTORY" "${S_PREVIEW}$(printf '%064x' "${ELO_LAUNCH_CONFIG_ID:-0}")$(printf '%064d' 0)" 2>/dev/null || true)
    [ -n "$pin" ] && [ "$pin" != "0x" ] \
        || die "the factory did not answer previewLaunchEconomics(uint256,address).
   Either it is not PonsV2LaunchFactory, or this \`cast\` did not accept raw
   calldata as a positional argument - and the broadcast passes its data the
   same way. Check: cast call $FACTORY ${S_PREVIEW}… --rpc-url \$RPC"
    note "answers previewLaunchEconomics -> PonsV2LaunchFactory ok"
    note "raw-calldata positional works on this cast ok"

    say "the composer imports and runs"
    meter_py -c 'import launch; print("   meter/launch.py ok, factory:", launch.stack()["factory"])'
    note "check clean"
    ;;

knobs)
    # Deliberately needs NOTHING - no venv, no cast, no network. Reading back
    # what you are about to weld is the one phase that must run anywhere.
    load_env
    say "the figures in $ELO_ENV"
    require_knobs
    printf '\n   %-26s %s\n' "ELO_DEPLOYER" "$ELO_DEPLOYER"
    printf '   %-26s %s (%s)\n' "name / symbol" "$ELO_NAME" "$ELO_SYMBOL"
    printf '   %-26s %s\n' "quote asset" "${ELO_PAIR_TOKEN:-native ETH (0x0)}"
    printf '   %-26s %s\n' "launch config id" "${ELO_LAUNCH_CONFIG_ID:-0}"
    printf '\n   WELDED BY THIS TRANSACTION - no setter, at any price:\n'
    printf '   %-26s %s\n' "buybackEnabled" "$ELO_BUYBACK_ENABLED"
    printf '   %-26s %s bps\n' "creatorTaxBps" "$ELO_CREATOR_TAX_BPS"
    printf '   %-26s %s ETH\n' "opening buy" "$ELO_OPENING_BUY_ETH"
    printf '   %-26s %s\n' "snipe exemptions" "${ELO_SNIPE_EXEMPTIONS:-none (deployer + creator are automatic)}"
    printf '   %-26s %s\n' "salt" "${ELO_SALT:-fresh random, chosen at plan time}"
    printf '\n   TIMELOCKED, not welded:\n'
    printf '   %-26s %s\n' "creatorFeeRecipient" "${ELO_CREATOR_FEE_RECIPIENT:-<the deployer>}"
    printf '\n   Path: %s\n' "$([ "${ELO_ATOMIC:-true}" = false ] && echo 'DIRECT - the opening buy is a SECOND transaction' || echo 'atomic launchAndBuy')"
    ;;

preflight)
    load_env; need_python
    [ -n "${ELO_DEPLOYER:-}" ] || die "set ELO_DEPLOYER in $ELO_ENV - the gate is per wallet"
    say "every pons gate, read off chain against $ELO_DEPLOYER"
    # ⚠ The JSON goes through a FILE, not a pipe. `python - <<EOF` takes its
    # PROGRAM from stdin, so piping data in as well silently hands the parser
    # the heredoc it already consumed — an empty read that looks like an
    # unreachable factory.
    pre="$(mktemp)"; trap 'rm -f "$pre"' EXIT
    meter_py launch.py --deployer "$ELO_DEPLOYER" \
            --config-id "${ELO_LAUNCH_CONFIG_ID:-0}" \
            --pair-token "${ELO_PAIR_TOKEN:-}" \
            --creator-tax-bps "${ELO_CREATOR_TAX_BPS:-0}" \
            --preflight-only > "$pre" || true
    [ -s "$pre" ] || die "the composer printed nothing — it could not run at all"
    "$PY" - "$pre" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for k, c in (d.get("checks") or {}).items():
    print(f"   [{'ok ' if c['ok'] else 'NO '}] {k:16} {c['why']}")
    if k == "launch_config" and (c.get("got") or {}).get("config"):
        g = c["got"]["config"]
        print(f"        supply {g['supply']/1e18:,.0f} · fee {g['curveFeeBps']}bps"
              f" · phantom {g['phantomQuote']/1e18} · threshold {g['graduationThreshold']/1e18}"
              f" · poolFee {g['poolFee']} · tickSpacing {g['tickSpacing']}")
    if k == "launch_fee" and c.get("got") is not None:
        print(f"        launchFee {int(c['got'])/1e18:.6f} ETH — msg.value must EQUAL this")
s = d.get("snipe_tax") or {}
print(f"\n   snipe tax opens at {s.get('start_bps')} bps and decays across {s.get('seconds')}s")
print("   ⚠ at 101ms blocks that window is TENS of blocks wide, not one or two")
sys.exit(0 if d.get("ok") else 1)
PY
    rc=$?
    [ "$rc" = 0 ] || die "a gate above is red. canLaunch() is the only one whose fix is
   not a code change - it is an email to contact@ponsfamily.com."
    note "every gate green"
    ;;

size)
    load_env; need_python
    # ⚠ NOTHING HERE FETCHES A PRICE, and that is deliberate rather than lazy —
    # `curve_sim.py` says the same in its own flag help. A dollar figure is
    # sized against a rate somebody read somewhere; the tool does the
    # arithmetic and names the rate it used, so the ETH number you weld is one
    # you can re-derive rather than one a script guessed for you.
    USD=""; RATE="${ELO_ETH_USD:-}"
    while [ $# -gt 0 ]; do
        case "$1" in
            --usd) USD="$2"; shift 2 ;;
            --eth-usd) RATE="$2"; shift 2 ;;
            *) shift ;;
        esac
    done
    if [ -n "$USD" ]; then
        [ -n "$RATE" ] || die "--usd needs a rate: --eth-usd <ETH price in USD>, or ELO_ETH_USD.
   Nothing in this repo fetches one. Read it wherever you normally would and
   pass it, so the figure you weld is one you can re-derive."
        say "sizing \$$USD at \$$RATE/ETH"
        "$PY" - "$USD" "$RATE" <<'PY'
from decimal import Decimal, ROUND_DOWN
import sys
usd, rate = Decimal(sys.argv[1]), Decimal(sys.argv[2])
eth = (usd / rate).quantize(Decimal("0.000001"), rounding=ROUND_DOWN)
print(f"   ${usd} / ${rate} = {eth} ETH")
print(f"   put this in ELO_OPENING_BUY_ETH:  {eth}")
print(f"   …and read it back digit by digit — it is welded, and one decimal out")
print(f"      is ${(eth * rate * 10).quantize(Decimal('1'))} or ${(eth * rate / 10).quantize(Decimal('1'))}")
PY
        echo
    fi
    say "what the opening buy does to the price, priced off the live config"
    # ⚠ It prices; it composes nothing and sends nothing. Two facts a plan gets
    # wrong in opposite directions: a buy multiplies the marginal price by
    # ((Q+net)/Q)² - the SQUARE of the reserve's growth - and graduating costs
    # threshold/(1-fee-tax) GROSS, not the threshold.
    meter_py curve_sim.py \
        --config-id "${ELO_LAUNCH_CONFIG_ID:-0}" \
        --pair-token "${ELO_PAIR_TOKEN:-}" \
        --creator-tax-bps "${ELO_CREATOR_TAX_BPS:-0}" \
        --dev-buy "${ELO_OPENING_BUY_ETH:-0}"
    ;;

fees)
    # What a trade costs, in both venues, read off a live launch.
    #
    # A trader pays `feeBps + creatorTaxBps` on the curve, where feeBps comes
    # from pons' LAUNCH CONFIG (100 bps at last read) and is not ours to set.
    # After graduation the pool is created with its own fee at ZERO and the
    # HOOK charges `hookFeeBps` instead — which is why two separate fields
    # exist. pons documents them as netting out the same: "you pay the same
    # rate whether the token is on its curve or in its Uniswap pool."
    #
    # So this CONFIRMS a documented fact on chain rather than resolving an open
    # question — cheap, and the standing rule here is derive rather than quote.
    # It still warns on a mismatch, because the two fields are genuinely
    # separate in the ABI and a config could in principle set them apart.
    #
    # ELO does not exist yet, so its policy cannot be read. Any live pons v2
    # launch inherits the same defaults, so this reads a REFERENCE launch and
    # reports what ELO would inherit today.
    load_env; need cast; need_python
    REF="${1:-${ELO_FEE_REF:-}}"
    [ -n "$REF" ] || die "name a reference launch — any live pons v2 token:
     ./launch_elo.sh fees 0x…
   ELO has no policy until it exists; a launch already on this factory carries
   the defaults ELO would inherit."
    FACTORY="$(cfg_addr factory)"
    arg="$(printf '%064s' "$(lc "$REF" | sed 's/^0x//')" | tr ' ' '0')"
    rec=$(raw_call "$FACTORY" "$(sel 'getLaunchedToken(address)')${arg}")
    require_record "$rec" "$REF"
    pol=$(raw_call "$FACTORY" "$(sel 'getLaunchFeePolicy(address)')${arg}")

    REF_CURVE=$(word_addr "$rec" 1)
    REF_TAX=$(word "$rec" 8)
    REF_PHASE=$(word "$rec" 10)
    # FeePolicy is 5 static members: recipient, protocolShare, buybackBurn,
    # hookFee, maxInternalPriceImpact.
    P_SHARE=$(word "$pol" 1); P_BURN=$(word "$pol" 2); P_HOOK=$(word "$pol" 3)
    CURVE_FEE=$(raw_call "$REF_CURVE" "$(sel 'feeBps()')" 2>/dev/null || echo 0x0)
    CURVE_FEE=$(word "$CURVE_FEE" 0)

    say "reference launch $REF"
    printf '   %-28s %s  (0 curve · 1 swept · 2 v4 pool · 3 rescued)\n' "phase" "$REF_PHASE"
    printf '   %-28s %s bps   (pons'"'"' launch config — NOT ours to set)\n' "curve feeBps" "${CURVE_FEE:-?}"
    printf '   %-28s %s bps   (the hook, after graduation)\n' "hookFeeBps" "${P_HOOK:-?}"
    printf '   %-28s %s bps\n' "protocolFeeShareBps" "${P_SHARE:-?}"
    printf '   %-28s %s bps\n' "buybackBurnBps" "${P_BURN:-?}"
    printf '   %-28s %s bps\n' "its creatorTaxBps" "${REF_TAX:-?}"

    say "what a total of ${ELO_TOTAL_TRADE_BPS:-200} bps costs in creator tax"
    "$PY" - "${CURVE_FEE:-0}" "${P_HOOK:-0}" "${ELO_TOTAL_TRADE_BPS:-200}" \
            "${REF_PHASE:-0}" "${REF_TAX:-0}" <<'PY'
import sys
curve, hook, want, phase, ref_tax = (int(x or 0) for x in sys.argv[1:6])
print(f"   on the CURVE      {want} - {curve} = {want - curve} bps creator tax")
print(f"   in the POOL       {want} - {hook} = {want - hook} bps creator tax")
if curve == hook:
    print(f"\n   the two rates AGREE at {curve} bps, so one creator tax of "
          f"{want - curve} makes {want / 100:g}% in both venues.")
else:
    print(f"\n   ⚠ THE RATES DIFFER ({curve} on the curve, {hook} in the pool). One "
          "creator tax\n     CANNOT make the same total in both. Pick which venue "
          f"the {want / 100:g}% is\n     promised in and say so on the surface, because the "
          "other one will not\n     match it and the tax has no setter.")
# The on-chain half of the creator-tax scope question.
if phase == 2 and ref_tax > 0:
    print(f"\n   ✓ this reference is GRADUATED (phase 2) and still carries a "
          f"{ref_tax} bps creator\n     tax on its record — the tax rides the hook rather than "
          "dying with the curve,\n     confirmed on chain rather than from pons' prose.")
elif phase == 2:
    print("\n   this reference is graduated but has NO creator tax, so it cannot "
          "confirm\n     whether a tax survives graduation. Find one that charges a tax.")
else:
    print("\n   this reference has NOT graduated (phase != 2), so it says nothing "
          "about\n     whether a creator tax survives graduation. Find a phase-2 launch.")
PY
    ;;

plan)
    load_env; need_python; require_knobs
    say "composing the pinned, unsigned transaction"
    # The salt is FROZEN here and reused at broadcast. Without that, recomposing
    # to re-read the pin would mint a fresh random salt, a different CREATE2
    # address and different calldata - and the comparison at broadcast would
    # compare nothing.
    salt="${ELO_SALT:-}"
    tmp="$(mktemp)"; trap 'rm -f "$tmp"' EXIT
    if do_compose "$salt" > "$tmp"; then :; else
        printf '%s\n' "$(cat "$tmp")" | "$PY" -c 'import json,sys
d=json.load(sys.stdin)
for r in d.get("refusals") or []: print("   REFUSED  " + r)'
        die "the composer refused. There is no half-composed transaction to sign."
    fi
    mv "$tmp" "$PLAN_FILE"; trap - EXIT

    to=$(plan_field "$PLAN_FILE" tx.to)
    val=$(plan_field "$PLAN_FILE" tx.value)
    fn=$(plan_field "$PLAN_FILE" function)
    pin=$(plan_field "$PLAN_FILE" pinned_economics)
    usedsalt=$(plan_field "$PLAN_FILE" salt)
    atomic=$(plan_field "$PLAN_FILE" atomic)

    printf '\n   %-22s %s\n' "function" "$fn"
    printf '   %-22s %s\n' "to" "$to"
    printf '   %-22s %s wei  (%s ETH)\n' "value" "$val" "$(eth_of "$val")"
    printf '   %-22s %s\n' "economics pin" "$pin"
    printf '   %-22s %s\n' "salt" "$usedsalt"
    printf '   %-22s %s\n' "atomic" "$atomic"
    warn=$(plan_field "$PLAN_FILE" warning)
    [ -n "$warn" ] && printf '\n   ⚠ %s\n' "$warn"

    elo_set ELO_SALT_USED "$usedsalt"
    elo_set ELO_PIN_AT_PLAN "$pin"
    say "plan written to $PLAN_FILE (unsigned; nothing has moved)"
    note "next: ./launch_elo.sh launch        # dry run, simulates against the chain"
    note "then: ./launch_elo.sh launch --arm  # THE unpatchable transaction"
    ;;

launch)
    load_env; need cast; need_python; require_knobs
    ARM="$(armed "${1:-}")"
    [ -f "$PLAN_FILE" ] || die "no $PLAN_FILE - run ./launch_elo.sh plan first"
    [ -n "${ELO_SALT_USED:-}" ] || die "$ELO_ENV has no ELO_SALT_USED - re-run plan"

    say "recomposing from scratch, on the plan's own salt"
    # THE PIN IS RE-READ HERE. Every term of a launch is owner-updatable on
    # pons' factory; previewLaunchEconomics digests the set, and passing it back
    # makes movement REVERT rather than silently reprice. The revert is the
    # backstop - catching the move locally is free.
    fresh="$(mktemp)"; trap 'rm -f "$fresh"' EXIT
    do_compose "$ELO_SALT_USED" > "$fresh" || {
        printf '%s\n' "$(cat "$fresh")" | "$PY" -c 'import json,sys
d=json.load(sys.stdin)
for r in d.get("refusals") or []: print("   REFUSED  " + r)'
        die "the composer refuses NOW even though the plan composed. A gate moved
   under us - re-read the preflight before anything else."; }

    old_pin=$(plan_field "$PLAN_FILE" pinned_economics)
    new_pin=$(plan_field "$fresh" pinned_economics)
    [ "$old_pin" = "$new_pin" ] || die "THE ECONOMICS MOVED between plan and broadcast.
     planned: $old_pin
     now:     $new_pin
   pons' owner edited the launch config - the supply, the curve fee, the phantom
   reserve, the threshold, the pool tier or the hook's fee policy. Broadcasting
   the planned transaction would revert (LaunchEconomicsMismatch); broadcasting
   the fresh one would launch on terms nobody read. Re-run:
     ./launch_elo.sh preflight && ./launch_elo.sh size && ./launch_elo.sh plan"

    old_data=$(plan_field "$PLAN_FILE" tx.data)
    new_data=$(plan_field "$fresh" tx.data)
    [ "$old_data" = "$new_data" ] || die "the calldata changed between plan and now, with the
   pin unmoved. A knob in $ELO_ENV was edited after the plan was written.
   Re-run ./launch_elo.sh plan and read it again."

    TO=$(plan_field "$fresh" tx.to)
    VALUE=$(plan_field "$fresh" tx.value)
    DATA=$(plan_field "$fresh" tx.data)
    FN=$(plan_field "$fresh" function)

    printf '\n   %-22s %s\n' "function" "$FN"
    printf '   %-22s %s\n' "to" "$TO"
    printf '   %-22s %s wei\n' "value" "$VALUE"
    printf '   %-22s %s ETH   <- READ THIS DIGIT BY DIGIT\n' "" "$(eth_of "$VALUE")"
    printf '   %-22s %s\n' "pin (unmoved)" "$new_pin"

    # ⚠ THE MISCOUNTED ZERO. TICKET.md §8d exists for exactly one failure: a
    # value typed against a tape, one decimal out, that nothing on chain
    # objects to. Print the buy three ways so the error has to survive all of
    # them, including as a share of the threshold that governs graduation.
    if [ -n "${ELO_PAIR_TOKEN:-}" ]; then
        note "ERC-20 quote: this value is the LAUNCH FEE ONLY and the buy is PULLED."
        note "approve first:  cast send $ELO_PAIR_TOKEN 'approve(address,uint256)' $TO $(to_wei "$ELO_OPENING_BUY_ETH") --rpc-url \$RPC --private-key \$PRIVATE_KEY"
    fi
    say "sanity on the opening buy"
    meter_py - "$ELO_OPENING_BUY_ETH" "${ELO_LAUNCH_CONFIG_ID:-0}" <<'PY'
import sys
import launch
buy = float(sys.argv[1])
pre = launch.preflight("0x" + "0" * 40, int(sys.argv[2]))
cfg = ((pre.get("checks") or {}).get("launch_config", {}).get("got") or {}).get("config") or {}
thr = (cfg.get("graduationThreshold") or 0) / 1e18
ph = (cfg.get("phantomQuote") or 0) / 1e18
fee = (cfg.get("curveFeeBps") or 0)
print(f"   opening buy      {buy} ETH")
if thr:
    print(f"   threshold        {thr} ETH   -> the buy is {buy / thr * 100:.2f}% of it")
    print(f"   to graduate      {thr / (1 - fee / 10000):.4f} ETH GROSS, not {thr} — "
          "the threshold is measured NET of fees")
if ph:
    print(f"   price move       {((ph + buy) / ph) ** 2:.4f}x  (the SQUARE of the reserve's growth)")
PY

    if [ "$ARM" != yes ]; then
        say "DRY RUN - simulating the exact calldata against the chain"
        # `cast call` runs it in the node's EVM at head, with the value and the
        # sender that will actually be used. A revert here is the launch
        # reverting, found for free.
        if cast call "$TO" "$DATA" --value "$VALUE" --from "$ELO_DEPLOYER" --rpc-url "$RPC" >/dev/null 2>"$fresh.err"; then
            note "simulation OK - the transaction would succeed at this block"
        else
            printf '\n'; sed 's/^/   /' "$fresh.err" || true
            die "SIMULATION REVERTED. Nothing was sent. Read the error above against
   ELO-LAUNCH.md §2 - the common ones are NotWhitelisted (canLaunch),
   LaunchFeeNotPaid (value != launchFee), CreatorTaxTooHigh and
   LaunchEconomicsMismatch."
        fi
        say "nothing moved (no --arm)"
        note "to broadcast:  ELO_CONFIRM=$ELO_SYMBOL ./launch_elo.sh launch --arm"
        exit 0
    fi

    # ── armed ────────────────────────────────────────────────────────────────
    say "ARMED - this transaction is not patchable"
    [ -n "${PRIVATE_KEY:-}" ] || die "--arm needs PRIVATE_KEY in the env (never in $ELO_ENV)"
    [ "${ELO_CONFIRM:-}" = "$ELO_SYMBOL" ] || die "set ELO_CONFIRM to exactly '$ELO_SYMBOL' to broadcast.
   It is the typed confirmation against a value one decimal out, which is the
   failure TICKET.md §8d exists for and which nothing on chain objects to."

    # A preflight run against one wallet and a broadcast from another checks
    # nothing at all.
    keyaddr=$(cast wallet address --private-key "$PRIVATE_KEY")
    [ "$(lc "$keyaddr")" = "$(lc "$ELO_DEPLOYER")" ] || die "PRIVATE_KEY controls $keyaddr but
   ELO_DEPLOYER is $ELO_DEPLOYER. canLaunch, the CREATE2 namespace and the
   default creator fee recipient are all keyed to the DEPLOYER. Fix one of them."

    bal=$(cast balance "$ELO_DEPLOYER" --rpc-url "$RPC")
    "$PY" - "$bal" "$VALUE" <<'PY' || die "the deployer cannot cover value + gas"
import sys
bal, val = int(sys.argv[1]), int(sys.argv[2])
print(f"   balance {bal/1e18:.6f} ETH, sending {val/1e18:.6f} ETH")
sys.exit(0 if bal > val else 1)
PY

    say "simulating once more, immediately before sending"
    cast call "$TO" "$DATA" --value "$VALUE" --from "$ELO_DEPLOYER" --rpc-url "$RPC" >/dev/null \
        || die "simulation reverted at head. NOTHING WAS SENT."

    say "BROADCASTING $FN (chain $CHAIN_ID_WANT)"
    cast send "$TO" "$DATA" --value "$VALUE" --rpc-url "$RPC" --private-key "$PRIVATE_KEY" --json > "$fresh.rcpt"
    TXH=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1])).get("transactionHash",""))' "$fresh.rcpt")
    [ -n "$TXH" ] || die "broadcast returned no transaction hash - check the chain before retrying.
   ⚠ DO NOT simply re-run: the salt is frozen, so a second launch on the same
   terms reverts (the pair already exists) but a launch that DID land is live."
    elo_set ELO_LAUNCH_TX "$TXH"
    say "sent: $TXH"
    note "next: ./launch_elo.sh record"
    ;;

record)
    load_env; need cast; need_python
    TXH="${1:-${ELO_LAUNCH_TX:-}}"
    [ -n "$TXH" ] || die "no transaction hash. Pass one, or run \`launch --arm\` first:
     ./launch_elo.sh record 0x…"
    say "reading the receipt"
    # ⚠ Through a FILE, not a pipe — see the note in `preflight`.
    rcpt="$(mktemp)"; trap 'rm -f "$rcpt"' EXIT
    cast receipt "$TXH" --rpc-url "$RPC" --json > "$rcpt"

    # TokenLaunched(address indexed token, address indexed curve,
    #               address indexed deployer, address pairToken,
    #               uint256 launchConfigId, uint256 graduationThreshold)
    # All three addresses are INDEXED, so token and curve are topics 1 and 2 -
    # no data decoding, and the topic is derived by keccak rather than pasted.
    T0="$(topic 'TokenLaunched(address,address,address,address,uint256,uint256)')"
    read -r TOKEN CURVE BLOCK < <("$PY" - "$rcpt" "$T0" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
if str(r.get("status", "")).lower() not in ("0x1", "1", "true"):
    sys.exit("   the transaction REVERTED on chain - there is nothing to record")
want = sys.argv[2].lower()
for log in r.get("logs") or []:
    t = [x.lower() for x in (log.get("topics") or [])]
    if t and t[0] == want:
        print("0x" + t[1][-40:], "0x" + t[2][-40:], int(str(r.get("blockNumber")), 0))
        break
else:
    sys.exit("   no TokenLaunched log in this receipt - is it the launch transaction?")
PY
)
    [ -n "${TOKEN:-}" ] || die "could not read the launch out of the receipt"
    note "token $TOKEN"
    note "curve $CURVE"
    note "block $BLOCK"

    say "recording - this is the step that makes the rename land"
    # ⚠ reserve.py reads deployments.json under RESERVE_KEY, so a box that only
    # PULLS gets the new reserve with no env at all (ELO-LAUNCH.md §5 step 5).
    "$PY" - "$DEPLOYMENTS" "$RESERVE_KEY" "$TOKEN" "$CURVE" "$TXH" "$BLOCK" "$ELO_DEPLOYER" "$CHAIN_ID_WANT" <<'PY'
import json, sys
path, key, token, curve, tx, block, deployer, chain = sys.argv[1:9]
d = json.load(open(path))
c = d["chains"][str(chain)]["contracts"]
c[key] = {
    "address": token, "deployer": deployer, "tx": tx, "block": int(block),
    "source": None, "script": None, "external": True,
    "curve": curve,
    "note": ("the platform reserve, launched on pons v2 (PonsV2LaunchFactory). "
             "`curve` is this launch's own bonding curve, resolved from the factory "
             "and recorded here only as a convenience - read it from the factory, "
             "never from this file, because it is per launch."),
}
json.dump(d, open(path, "w"), indent=2)
open(path, "a").write("\n")
print(f"   deployments.json  chains.{chain}.contracts.{key} = {token}")
PY
    "$PY" - "$CURVE_CONFIG" "$TOKEN" <<'PY'
import json, sys
path, token = sys.argv[1], sys.argv[2]
d = json.load(open(path))
d["token"] = token
json.dump(d, open(path, "w"), indent=2)
open(path, "a").write("\n")
print(f"   curve.config.json  token = {token}")
PY
    elo_set ELO_TOKEN "$TOKEN"; elo_set ELO_CURVE "$CURVE"

    cat <<EOF

   ⚠ STILL BY HAND - the browser half does not read reserve.py:
     1. watchtower/pools.config.json  - the OBOL and MYRRH rows are LABELLED
        \`/ ELO\` while their \`addr\` is the retired coin's. Today that is
        harmless; from this transaction on, the store displays two pools
        claiming to hold a token that holds none of it (ONE-SHOT.md §2).
     2. watchtower/listings/listings.json - same, for listing rows.
     3. Untracked files pass every local gate and the release does not have
        them:   git status --porcelain | grep '^??'
     4. Release, then verify AT THE ORIGIN - /api/reserve, /api/curve,
        /api/launch all 404 there today:
          cd /data/apps/scry-deploy && git pull && pm2 restart scry-meter \\
            && python3 meter/smoke_test.py
     5. Check /pubkey against the origin - it must stay
        LvuPBMbKKyoNEuyvLf7f+rbyjK67vBcWy9MDwaRINGE=

   Then: ./launch_elo.sh watch
EOF
    ;;

watch)
    load_env; need cast; need_python
    ARM="$(armed "${1:-}")"
    TOKEN="${ELO_TOKEN:-$(cfg_addr token)}"
    [ -n "$TOKEN" ] || die "no launch token yet - run ./launch_elo.sh record first"
    FACTORY="$(cfg_addr factory)"
    S_GET="$(sel 'getLaunchedToken(address)')"
    S_GRAD="$(sel 'createGraduatedPool(address)')"
    arg="$(printf '%064s' "$(lc "$TOKEN" | sed 's/^0x//')" | tr ' ' '0')"

    # ⚠ PACED ON THE WALL CLOCK, NEVER ON BLOCK COUNTS. RH-Chain is Arbitrum
    # Nitro: solidity's block.number advances once per ~12s while the real chain
    # runs at 101ms. A "wait 100 blocks" loop here means ~20 minutes, which is
    # the bug ScryGacha shipped with and nothing failed (RUNBOOK.md §0c).
    say "watching $TOKEN (Ctrl-C to stop; polls every 15s of wall clock)"
    while :; do
        raw=$(raw_call "$FACTORY" "${S_GET}${arg}") || die "the factory did not answer"
        require_record "$raw" "$TOKEN"
        # The struct is 15 STATIC members, so it is 15 inline words and `phase`
        # is word 10. Read as words rather than decoded: a field ADDED upstream
        # shifts a decoder silently.
        phase=$(word "$raw" 10)
        case "$phase" in
            0) label="NotGraduated - trading on the curve" ;;
            1) label="SWEPT - curve closed, pool NOT built" ;;
            2) label="PoolCreated - trading on Uniswap v4" ;;
            3) label="RESCUED - off the normal path, surface this explicitly" ;;
            *) label="unknown ($phase)" ;;
        esac
        printf '   %s  phase %s  %s\n' "$(date -u +%H:%M:%SZ)" "$phase" "$label"

        if [ "$phase" = 2 ]; then
            say "graduated. Route both directions to the v4 pool from here."
            note "⚠ a v4 pool has NO ADDRESS - it is a bytes32 id into the PoolManager"
            note "next: snapshot ELO holders at the graduation block (ONE-SHOT.md §7 step 4)"
            exit 0
        fi
        if [ "$phase" = 1 ]; then
            # Graduation normally completes inside the buy that finishes the
            # curve; when that buy is near its gas limit the curve emits
            # AutoGraduationFailed and the launch parks here. It is a WORK
            # QUEUE, not a fault: createGraduatedPool is permissionless and
            # retryable, and a launch parked in phase 1 looks dead on every card
            # at exactly the moment the most people are looking.
            say "phase 1 - graduation stalled. Anyone may finish it; reserves are not stranded."
            if [ "$ARM" = yes ]; then
                [ -n "${PRIVATE_KEY:-}" ] || die "--arm needs PRIVATE_KEY"
                note "calling createGraduatedPool($TOKEN)"
                cast send "$FACTORY" "${S_GRAD}${arg}" --rpc-url "$RPC" --private-key "$PRIVATE_KEY"
            else
                note "to clear it:  ./launch_elo.sh watch --arm"
            fi
        fi
        sleep 15
    done
    ;;

status)
    load_env 2>/dev/null || true
    need_python
    say "what this tree calls the reserve"
    meter_py -c 'import json, reserve; print(json.dumps(reserve.resolve(), indent=2))'
    say "what the site config names"
    printf '   curve.config.json token = %s\n' "$(cfg_addr token)"
    tok="${ELO_TOKEN:-$(cfg_addr token)}"
    if [ -n "$tok" ] && command -v cast >/dev/null 2>&1; then
        say "the launch, off chain"
        FACTORY="$(cfg_addr factory)"
        S_GET="$(sel 'getLaunchedToken(address)')"
        arg="$(printf '%064s' "$(lc "$tok" | sed 's/^0x//')" | tr ' ' '0')"
        raw=$(raw_call "$FACTORY" "${S_GET}${arg}") || die "factory did not answer"
        require_record "$raw" "$tok"
        printf '   phase %s  (0 curve · 1 swept · 2 v4 pool · 3 rescued)\n' "$(word "$raw" 10)"
        printf '   curve %s\n' "$(word_addr "$raw" 1)"
    else
        note "no launch recorded yet - this is the correct opening state, not a fault"
    fi
    ;;

help|--help|-h|*)
    sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'
    ;;
esac
