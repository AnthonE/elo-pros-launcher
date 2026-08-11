#!/usr/bin/env bash
# Gate for deploy_town.sh's pool read-back (pools_read_before / pools_read_after).
#
# WHY THIS EXISTS. The read-back replaced a heredoc that PRINTED `cast call`
# commands and asked the operator to eyeball the result. Replacing a human
# check with an automatic one is only an improvement if the automatic one
# actually fires — a check that cannot fail is worse than the heredoc, because
# the heredoc at least did not claim to have looked. So every case below
# REINTRODUCES the bug the check exists for and asserts it aborts.
#
# `cast` is stubbed, so this runs offline and deterministically. It proves the
# WIRING and the DECISION LOGIC, not RPC behaviour: that a mismatched sqrtP
# stops the run, that a drifted pool stops the top-up, that a missing LP
# position stops both. The live half is covered by SeedSpoilsFork.t.sol.
#
#   ./test_pool_readback.sh
set -uo pipefail
cd "$(dirname "$0")"

# ── the re-exec'd half: source the functions and run one scenario ────────────
if [ "${1:-}" = "--case" ]; then
    # Read the phase BEFORE sourcing: the extracted prologue ends with
    # `cmd="${1:-help}"; shift || true`, so sourcing it CONSUMES our own $1 and
    # leaves $2 unbound under `set -u`. The first run of this file died there —
    # and because it died with a non-zero exit, all five abort cases "passed"
    # while proving nothing. A test whose failure mode is indistinguishable
    # from its success mode is the trap it was written to close.
    PHASE="$2"
    # Everything above `case "$cmd" in` is assignments + function definitions.
    FUNCS=$(mktemp); trap 'rm -f "$FUNCS"' EXIT
    sed -n '1,/^case "$cmd" in/p' deploy_town.sh | sed '$d' > "$FUNCS"
    # shellcheck disable=SC1090
    . "$FUNCS"

    OBOL="${T_OBOL:?}"; MYRRH="${T_MYRRH:?}"; SCRY_CANON="${T_SCRY:?}"
    SCRY_PER_OBOL=10; SCRY_PER_MYRRH=50; OBOL_PER_MYRRH=5
    case "$PHASE" in
        before) pools_read_before ;;
        after)  POOLS_BEFORE="${T_POOLS_BEFORE:-}"
                pools_read_after "${T_LP_TO:-}" "${T_LP_BEFORE:-0}" ;;
        *)      echo "unknown phase '$PHASE'" >&2; exit 2 ;;
    esac
    exit $?
fi

# ── the driver ──────────────────────────────────────────────────────────────
BIN=$(mktemp -d); DB="$BIN/db"; trap 'rm -rf "$BIN"' EXIT
cat > "$BIN/cast" <<'STUB'
#!/usr/bin/env bash
# call <addr> <sig> [args...] --rpc-url <url>   — answers from $DB, one k=v per line.
db="$DB"; sig="$3"
key=""
case "$sig" in
    *getPool*)   key="pool:$(printf '%s' "$4" | tr 'A-F' 'a-f'):$(printf '%s' "$5" | tr 'A-F' 'a-f')" ;;
    *slot0*)     key="slot0:$(printf '%s' "$2" | tr 'A-F' 'a-f')" ;;
    *balanceOf*) key="bal:$(printf '%s' "$2" | tr 'A-F' 'a-f'):$(printf '%s' "$4" | tr 'A-F' 'a-f')" ;;
esac
v=$(grep -E "^${key}=" "$db" 2>/dev/null | tail -1 | cut -d= -f2-)
[ -n "$v" ] || { echo "stub: no answer for '$key'" >&2; exit 1; }
printf '%s\n' "$v"
STUB
chmod +x "$BIN/cast"
export PATH="$BIN:$PATH" DB

export T_OBOL=0x1111111111111111111111111111111111111111
export T_MYRRH=0x2222222222222222222222222222222222222222
export T_SCRY=0xda2a4b23459e9ca88183e990802be644aca7c4b0
export V3_FACTORY=0xfac0000000000000000000000000000000000000
export NPM=0xn000000000000000000000000000000000000000
export NPM=0x9999999999999999999999999999999999999999
export RPC=stub://rpc
P_OS=0xaaa0000000000000000000000000000000000001   # OBOL/$SCRY
P_MS=0xaaa0000000000000000000000000000000000002   # MYRRH/$SCRY
P_MO=0xaaa0000000000000000000000000000000000003   # MYRRH/OBOL
Z=0x0000000000000000000000000000000000000000
# The sqrtP values the seed helper would export. Their exact magnitude is
# irrelevant here — what is under test is equality and drift, not v3 math
# (script/seed_spoils_uniswap.py --selftest owns that).
S_OS=79228162514264337593543950336
S_MS=177159557114295710296101716160
S_MO=17715955711429571029610171616
export OBOL_SQRTP=$S_OS MYRRH_SQRTP=$S_MS MYRRH_OBOL_SQRTP=$S_MO

pairs() {  # $1..$3 = the three pool addresses getPool should return
    printf 'pool:%s:%s=%s\n' "$T_OBOL"  "$T_SCRY"  "$1" \
                             "$T_MYRRH" "$T_SCRY"  "$2" \
                             "$T_MYRRH" "$T_OBOL"  "$3"
}

pass=0; fail=0
run() {  # run <name> <expect: ok|abort> <phase> ; DB already written
    local name="$1" expect="$2" phase="$3" out rc
    out=$(./test_pool_readback.sh --case "$phase" 2>&1); rc=$?
    if { [ "$expect" = ok ] && [ $rc -eq 0 ]; } || { [ "$expect" = abort ] && [ $rc -ne 0 ]; }; then
        printf '  PASS  %s\n' "$name"; pass=$((pass + 1))
    else
        printf '  FAIL  %s (expected %s, exit %s)\n%s\n' "$name" "$expect" "$rc" "$out"
        fail=$((fail + 1))
    fi
}

echo "== pools_read_before — the pre-broadcast half"

# 1. Nothing exists yet: the ordinary first run. Must not abort.
pairs "$Z" "$Z" "$Z" > "$DB"
run "three fresh pools — records 'new' and proceeds" ok before

# 2. THE TOP-UP HAZARD. The canary is live and tradeable between runs, so a
#    stranger can move the price. Seeding the posted ratio into a moved pool
#    mints a mix nobody chose. Nothing checked this before.
{ pairs "$P_OS" "$Z" "$Z"
  # +10% on sqrtP is ~+21% on price — far outside the 100bps bound.
  echo "slot0:$P_OS=$(python3 -c "print(int($S_OS * 1.10))")"; } > "$DB"
run "existing pool drifted 21% — ABORTS before spending anything" abort before

# 3. The same check must not be so tight it blocks an honest top-up.
{ pairs "$P_OS" "$Z" "$Z"
  echo "slot0:$P_OS=$(python3 -c "print(int($S_OS * 1.0004))")"; } > "$DB"
run "existing pool drifted 0.08% — inside the bound, proceeds" ok before

# 4. A chain that will not answer is not a price of zero.
pairs "$P_OS" "$Z" "$Z" > "$DB"   # pool exists, but no slot0 row -> stub errors
run "slot0 unreadable — ABORTS rather than assuming" abort before

echo
echo "== pools_read_after — the post-broadcast half"
BEFORE_ALL_NEW=$(printf 'OBOL/$SCRY|%s|new\nMYRRH/$SCRY|%s|new\nMYRRH/OBOL|%s|new\n' "$Z" "$Z" "$Z")
export T_POOLS_BEFORE="$BEFORE_ALL_NEW" T_LP_TO=0xdead00000000000000000000000000000000dead T_LP_BEFORE=0

# 5. The happy path — every pool opened exactly where the helper said.
{ pairs "$P_OS" "$P_MS" "$P_MO"
  echo "slot0:$P_OS=$S_OS"; echo "slot0:$P_MS=$S_MS"; echo "slot0:$P_MO=$S_MO"
  echo "bal:$NPM:$T_LP_TO=3"; } > "$DB"
run "all three opened at the posted sqrtP, LP NFT landed" ok after

# 6. THE BUG THE CANARY EXISTS FOR, reintroduced: a pool opened at a price the
#    script did not choose. A v3 pool's first mint IS its price.
{ pairs "$P_OS" "$P_MS" "$P_MO"
  echo "slot0:$P_OS=$(python3 -c "print(int($S_OS * 1.5))")"
  echo "slot0:$P_MS=$S_MS"; echo "slot0:$P_MO=$S_MO"
  echo "bal:$NPM:$T_LP_TO=3"; } > "$DB"
run "one pool opened off-ratio — ABORTS before the top-up" abort after

# 7. The third pool is the one a stranger can front-run into existence. If it
#    did not get created, the genesis arb it forecloses is still open.
{ pairs "$P_OS" "$P_MS" "$Z"
  echo "slot0:$P_OS=$S_OS"; echo "slot0:$P_MS=$S_MS"
  echo "bal:$NPM:$T_LP_TO=3"; } > "$DB"
run "MYRRH/OBOL never got created — ABORTS" abort after

# 8. The position NFT is the deed to everything just deposited. The seeder only
#    warned when the recipient had code; nothing proved the mint landed.
{ pairs "$P_OS" "$P_MS" "$P_MO"
  echo "slot0:$P_OS=$S_OS"; echo "slot0:$P_MS=$S_MS"; echo "slot0:$P_MO=$S_MO"
  echo "bal:$NPM:$T_LP_TO=0"; } > "$DB"
run "no LP position minted to the recipient — ABORTS" abort after

# 9. Top-up run: pools already existed at the right price, nothing drifted.
export T_POOLS_BEFORE=$(printf 'OBOL/$SCRY|%s|%s\nMYRRH/$SCRY|%s|%s\nMYRRH/OBOL|%s|%s\n' \
    "$P_OS" "$S_OS" "$P_MS" "$S_MS" "$P_MO" "$S_MO")
export T_LP_BEFORE=3
{ pairs "$P_OS" "$P_MS" "$P_MO"
  echo "slot0:$P_OS=$S_OS"; echo "slot0:$P_MS=$S_MS"; echo "slot0:$P_MO=$S_MO"
  echo "bal:$NPM:$T_LP_TO=6"; } > "$DB"
run "top-up over undrifted pools — proceeds" ok after

echo
if [ $fail -eq 0 ]; then
    echo "  all $pass checks pass — the read-back aborts on every failure it claims to catch"
    exit 0
fi
echo "  $fail of $((pass + fail)) FAILED"
exit 1
