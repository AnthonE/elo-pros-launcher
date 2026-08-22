#!/usr/bin/env bash
# GAME-COIN SEED WRAPPER
# seed_junk.sh — deploy Gates' play coin and open its pool against the reserve.
#
# The runbook this drives is docs/launch/CHECKLIST.md §7. It is `deploy_town.sh`
# for the current generation's first game pool, and it exists because that
# script seeds the RETIRED pair (OBOL+MYRRH+their Garden) in one act — a shape
# that cannot run when the coins arrive one at a time.
#
# House rules it ENFORCES rather than documents:
#   - DRY RUN by default. Broadcasting takes --arm.
#   - THE SEED IS SIZED AS A FRACTION, NEVER A DOLLAR OR A TYPED TOTAL. The
#     reserve side is `wallet balance - the share we are willing to show on a
#     holder map`, read off chain at run time. A typed total goes stale the
#     moment the opening buy lands at a different size, and it goes stale
#     silently (POOLS.md: "size by fraction, not by dollars").
#   - THE COIN SIDE IS DERIVED, never passed. budget / price. A listed pair
#     ({reserve: 65M, coin: 650M, ratio: 0.1}) has one degree of freedom too
#     many and the inconsistent version looks exactly as plausible.
#   - CANARY, THEN READ BACK OFF CHAIN, THEN TOP UP. `readback` is not
#     optional and not a formality: on 2026-07-29 the previous generation's
#     two reserve-paired pools were recorded in deployments.json at
#     76,576,500 and 23,523,500, and the chain has never held more than their
#     0.1% canaries since the position was pulled. A script's clean summary is
#     not evidence that value landed.
#   - A V3 POOL'S FIRST MINT IS ITS PRICE. There is no second try, at any
#     price. Everything else here is adjustable forever.
#
# Usage:
#   ./seed_junk.sh check                 # tooling, chain, and both addresses
#   ./seed_junk.sh deploy    [--arm]     # broadcast the coin (welds name/symbol/cap)
#   ./seed_junk.sh size                  # what the seed will be, read off chain
#   ./seed_junk.sh plan                  # the planner's exports, at the posted price
#   ./seed_junk.sh canary    [--arm]     # 1/1000 of the seed, to verify ordering
#   ./seed_junk.sh readback              # what the pool ACTUALLY holds, per token
#   ./seed_junk.sh seed      [--arm]     # the rest of it
#   ./seed_junk.sh fees                  # what the position has ACTUALLY earned
#   ./seed_junk.sh compound  [--arm]     # collect it and put it straight back in
#   ./seed_junk.sh season                # the game calendar, and the cap a budget implies
#   ./seed_junk.sh granary   [--arm]     # mint authority: granary + grant + rotate
#   ./seed_junk.sh record                # addresses -> junk.env (and deployments.json)
#
# Env (never echoed):
#   PRIVATE_KEY  required to --arm. The operator's own wallet on the box —
#                lane A (CLAUDE.md, two lanes). Nothing hosted holds it.
#   JUNK_ENV     figures + the address ledger. Default ./junk.env
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/.." && pwd)"
JUNK_ENV="${JUNK_ENV:-$HERE/junk.env}"
RPC="${RPC:-https://rpc.mainnet.chain.robinhood.com}"
CHAIN_ID=4663

# ── THE SHIPPED DEFAULTS — the only place these numbers live ────────────────
# `pool_seeds.py` parses this exact `NAME="${NAME:-value}"` shape out of this
# file, for the same reason it parses deploy_town.sh: this is what broadcasts.
# A doc about it is a copy, and a copy is what went stale last time.
#
# ⛔ SPOKEN 2026-08-22 (operator): 0.1 reserve per Junk, and no more than ~4%
#    of supply left showing on a holder map.
RESERVE_PER_JUNK="${RESERVE_PER_JUNK:-0.1}"
JUNK_WALLET_KEEP_PCT="${JUNK_WALLET_KEEP_PCT:-4}"
# pons launch config 0's supply. Read it back with `check`; it is the
# denominator the keep-share is a share OF, so a wrong value mis-sizes the seed.
RESERVE_SUPPLY="${RESERVE_SUPPLY:-1000000000}"
# The canary is 1/1000, matching the previous generation's discipline.
CANARY_DIVISOR="${CANARY_DIVISOR:-1000}"
FEE="${FEE:-10000}"          # 1% tier, tickSpacing 200 (POOLS.md §2.3)
SLIP_BPS="${SLIP_BPS:-100}"

COIN_NAME="${COIN_NAME:-Junk}"
COIN_SYMBOL="${COIN_SYMBOL:-JUNK}"
COIN_CAP="${COIN_CAP:-0}"    # 0 == UNCAPPED, welded. Junk copies OBOL.
#
# ── THE SEASON BUDGET IS BLANK ON PURPOSE ───────────────────────────────────
# Gates starts a new game the LAST THURSDAY of every month (operator,
# 2026-08-22), so what a person decides is a SEASON ceiling; what EloGranary
# takes is a PER-UTC-DAY cap. `script/season_cap.py` converts, and it has to,
# because last-Thursday seasons alternate 28 and 35 days — sizing off "about a
# month" is 25% wrong in whichever direction nobody checked.
#
# ⚠ NO DEFAULT. `DeployGranary.s.sol` falls back to GRANT_DAILY_CAP=250e18,
#   which is a sane number for the retired MYRRH farm and about four orders of
#   magnitude off for a play coin. It would not fail: the granary CLAMPS, so
#   every extraction past the first few would quietly pay short. `granary`
#   below refuses a blank budget rather than letting that default through.
# ⭐ 8,333,333 = 1,000,000,000 over ten years, twelve seasons a year (operator,
#   2026-08-22: *"most coins trade at 1 billion cap, split that up over like 10
#   years thats how long rust ran"*). It is a SCHEDULE, never a cap — `COIN_CAP`
#   is welded at 0, uncapped — and it is re-granted every season, so the rate is
#   a monthly decision rather than a ten-year weld. That is the answer to
#   "what if we need to scale": change this line and re-run `granary`.
#
#   Against a 0.1 pool it drains ~1.3% of the pool's Junk per season in the
#   worst case where every coin emitted is sold. Real play never reaches that.
#   `./seed_junk.sh season` prints it.
#
# ⚠ DENOMINATE IN JUNK, NOT IN DOLLARS. At launch prices a season's emission is
#   worth about thirty dollars, because the reserve's own FDV at pool open is
#   about thirty-seven thousand. That is the correct size for a thing nobody
#   has bought yet, and it scales with the reserve on its own. Set this from
#   how much scrap a run should yield, never from a dollar figure.
SEASON_BUDGET_JUNK="${SEASON_BUDGET_JUNK:-8333333}"
# The wallet the GAME SERVER signs with — the grantee that calls
# granary.mint(player, amount) on extraction. Not the deploy wallet.
GAME_MINTER="${GAME_MINTER:-}"
#
# ⚠ ORBS GETS ITS OWN COPY OF THIS FILE, not a second lane in it. The coins
#   arrive one at a time on purpose (Junk seeds on launch day, Orbs waits),
#   and a wrapper that seeds both is exactly the shape deploy_town.sh is stuck
#   in. Copy this file, change the three lines above, keep the marker on line 2
#   — `pool_seeds.game_seeds()` finds wrappers by that marker.

say()  { printf '\n== %s\n' "$*"; }
note() { printf '   %s\n' "$*"; }
die()  { printf '\nABORT: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing tool: $1"; }
armed() { [ "${1:-}" = "--arm" ] && echo yes || echo no; }

jget() { [ -f "$JUNK_ENV" ] && grep -E "^$1=" "$JUNK_ENV" | tail -1 | cut -d= -f2- | sed 's/^"//; s/"$//' || true; }
jset() {
  touch "$JUNK_ENV"
  if grep -qE "^$1=" "$JUNK_ENV"; then
    sed -i.bak "s|^$1=.*|$1=\"$2\"|" "$JUNK_ENV" && rm -f "$JUNK_ENV.bak"
  else
    printf '%s="%s"\n' "$1" "$2" >> "$JUNK_ENV"
  fi
  note "$1 -> $2"
}

py() { (cd "$REPO_ROOT/meter" && python3 "$@"); }
sel() { py -c 'import sys;from keccak import keccak256;print("0x"+keccak256(sys.argv[1].encode()).hex()[:8])' "$1"; }

# One eth_call, hex result. Uses cast when present, curl otherwise — the box
# has foundry, a fresh container often does not, and `size` must work anywhere.
raw_call() {
  if command -v cast >/dev/null 2>&1; then
    cast call "$1" --rpc-url "$RPC" --data "$2" 2>/dev/null || true
  else
    curl -s -m 25 -X POST "$RPC" -H 'content-type: application/json' \
      -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_call\",\"params\":[{\"to\":\"$1\",\"data\":\"$2\"},\"latest\"]}" \
      | py -c 'import sys,json;print(json.load(sys.stdin).get("result") or "0x")'
  fi
}

balance_of() { # $1 token, $2 holder -> whole units (18 dec)
  local s d r
  s="$(sel 'balanceOf(address)')"
  d="${s}$(printf '%064s' "$(printf '%s' "$2" | sed 's/^0x//' | tr 'A-Z' 'a-z')" | tr ' ' '0')"
  r="$(raw_call "$1" "$d")"
  if [ -z "$r" ] || [ "$r" = "0x" ]; then
    # A failed read is EMPTY, never a helpful zero. Sizing a seed off a zero
    # that meant "we could not look" is how a pool opens at the wrong price.
    echo ""; return
  fi
  py -c "print(int('$r',16)/1e18)"
}

require_addrs() {
  RESERVE="$(jget RESERVE_TOKEN)"; JUNK="$(jget JUNK_TOKEN)"; WALLET="$(jget SEED_WALLET)"
  [ -n "$RESERVE" ] || die "RESERVE_TOKEN unset in $JUNK_ENV — ELO has no address until it launches (CHECKLIST.md step 1)."
  [ -n "$WALLET" ]  || die "SEED_WALLET unset in $JUNK_ENV — the wallet holding the opening-buy position."
}

cmd_check() {
  say "tooling"; need curl; need python3
  command -v forge >/dev/null 2>&1 && note "forge  yes" || note "forge  NO — deploy/canary/seed cannot broadcast from here"
  command -v cast  >/dev/null 2>&1 && note "cast   yes" || note "cast   NO — falling back to curl for reads"
  say "chain"
  local id; id="$(curl -s -m 20 -X POST "$RPC" -H 'content-type: application/json' \
      -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' | py -c 'import sys,json;print(int(json.load(sys.stdin)["result"],16))')"
  note "chainId $id"; [ "$id" = "$CHAIN_ID" ] || die "wrong chain: got $id want $CHAIN_ID"
  say "addresses ($JUNK_ENV)"
  local r j w
  r="$(jget RESERVE_TOKEN)"; j="$(jget JUNK_TOKEN)"; w="$(jget SEED_WALLET)"
  note "reserve  ${r:-<unset — ELO has no address until it launches>}"
  note "junk     ${j:-<unset — run deploy>}"
  note "wallet   ${w:-<unset>}"
  say "posted figures"
  note "price          $RESERVE_PER_JUNK reserve per Junk   ⛔ welded by the first mint"
  note "map ceiling    ${JUNK_WALLET_KEEP_PCT}% of reserve supply left in the wallet"
  note "fee tier       $FEE (1%), full range"
}

cmd_deploy() {
  local arm; arm="$(armed "${1:-}")"
  say "deploy $COIN_NAME ($COIN_SYMBOL), cap $COIN_CAP  [arm=$arm]"
  note "⛔ name, symbol and cap are welded by this broadcast. cap 0 = UNCAPPED forever."
  note "the minter starts as the deploy wallet — nothing can earn this coin until setMinter."
  [ "$arm" = "yes" ] || { note "(dry run — pass --arm to broadcast)"; return; }
  need forge; [ -n "${PRIVATE_KEY:-}" ] || die "PRIVATE_KEY not in the environment"
  COIN_NAME="$COIN_NAME" COIN_SYMBOL="$COIN_SYMBOL" COIN_CAP="$COIN_CAP" \
    forge script "$HERE/script/DeployGameCoin.s.sol" --rpc-url "$RPC" --broadcast
  note "now: ./seed_junk.sh record --coin <address>"
}

# The sizing, and it is the whole argument of this script.
compute_size() {
  require_addrs
  BAL="$(balance_of "$RESERVE" "$WALLET")"
  [ -n "$BAL" ] || die "could not read the wallet's reserve balance — refusing to size a seed off a failed read"
  read -r KEEP BUDGET COINSIDE CANARY_B CANARY_C <<EOF
$(py -c "
bal=float('$BAL'); keep=float('$RESERVE_SUPPLY')*float('$JUNK_WALLET_KEEP_PCT')/100.0
budget=bal-keep; coin=budget/float('$RESERVE_PER_JUNK') if budget>0 else 0
d=float('$CANARY_DIVISOR')
print(keep, budget, coin, budget/d, coin/d)
")
EOF
}

cmd_size() {
  compute_size
  say "the seed, sized off chain just now"
  note "wallet holds       $(py -c "print(f'{float(\"$BAL\"):,.0f}')") reserve"
  note "keep (${JUNK_WALLET_KEEP_PCT}% of supply) $(py -c "print(f'{float(\"$KEEP\"):,.0f}')")  — what stays visible on a holder map"
  note "reserve side       $(py -c "print(f'{float(\"$BUDGET\"):,.0f}')")"
  note "junk side (derived) $(py -c "print(f'{float(\"$COINSIDE\"):,.0f}')")  @ $RESERVE_PER_JUNK reserve/Junk"
  note "canary (1/$CANARY_DIVISOR)     $(py -c "print(f'{float(\"$CANARY_B\"):,.0f}')") reserve + $(py -c "print(f'{float(\"$CANARY_C\"):,.0f}')") Junk"
  py -c "
b=float('$BUDGET')
raise SystemExit(0 if b>0 else 1)" || die "budget is zero or negative: the wallet holds less than the ${JUNK_WALLET_KEEP_PCT}% it is meant to keep. Nothing to seed."
}

cmd_plan() { # $1 optional: --canary
  compute_size
  local budget="$BUDGET"
  [ "${1:-}" = "--canary" ] && budget="$CANARY_B"
  require_addrs; [ -n "${JUNK:-}" ] || die "JUNK_TOKEN unset — run deploy first"
  say "planner exports (RE-RUN minutes before broadcast — POOLS.md §2.2)"
  # The reserve's ticker is RESOLVED, never typed — a site-wide SCRY->ELO
  # find-and-replace has already rewritten true sentences into false ones once
  # (CLAUDE.md, "resolve the reserve, never type it"). Display only either way.
  local rsym
  rsym="$(py -c 'import reserve;print(reserve.SYMBOL or "RESERVE")' 2>/dev/null || echo RESERVE)"
  (cd "$HERE/script" && python3 seed_spoils_uniswap.py \
      --reserve "$RESERVE" --coin "$JUNK" \
      --coin-price "$RESERVE_PER_JUNK" --coin-reserve-budget "$budget" \
      --reserve-symbol "$rsym" --coin-symbol "$COIN_SYMBOL" --slip-bps "$SLIP_BPS")
}

cmd_readback() {
  require_addrs
  local pool s0
  pool="$(jget JUNK_POOL)"
  [ -n "$pool" ] || die "JUNK_POOL unset — nothing has been seeded yet, or record has not run"
  say "what the pool ACTUALLY holds — eth_call balanceOf, not a script's summary"
  note "reserve in pool  $(balance_of "$RESERVE" "$pool")"
  note "junk in pool     $(balance_of "$JUNK" "$pool")"
  s0="$(raw_call "$pool" "$(sel 'slot0()')")"
  note "slot0            ${s0:0:66}…"
  note ""
  note "compare against ./seed_junk.sh size before topping up. If these are ~1/$CANARY_DIVISOR"
  note "of the intended sides, only the canary landed — that is the state the"
  note "previous generation's two reserve pools are in today."
}

cmd_seed() { # $1 --arm ; $2 optional --canary
  local arm; arm="$(armed "${1:-}")"
  local lane="${2:-}"
  say "seed Junk/reserve  [arm=$arm${lane:+ $lane}]"
  note "⛔ the FIRST mint welds the price at $RESERVE_PER_JUNK reserve per Junk."
  cmd_plan "$lane" > /tmp/junk_exports.$$ || die "planner failed"
  cat /tmp/junk_exports.$$
  [ "$arm" = "yes" ] || { note ""; note "(dry run — pass --arm to broadcast)"; rm -f /tmp/junk_exports.$$; return; }
  need forge; [ -n "${PRIVATE_KEY:-}" ] || die "PRIVATE_KEY not in the environment"
  # shellcheck disable=SC1090
  set -a; . <(grep '^export ' /tmp/junk_exports.$$ | sed 's/^export //'); set +a
  rm -f /tmp/junk_exports.$$
  FEE="$FEE" SLIP_BPS="$SLIP_BPS" LP_RECIPIENT="${LP_RECIPIENT:-$WALLET}" \
    forge script "$HERE/script/SeedSpoilsUniswapV3.s.sol" --rpc-url "$RPC" --broadcast
  note "now: ./seed_junk.sh readback — before anything else,"
  note "then record the LP position's tokenId from the receipt:"
  note "  ./seed_junk.sh record --position <tokenId>"
  note "without it, 'fees' and 'compound' have nothing to look at and the"
  note "position quietly earns fees nobody ever collects."
}

# The position's fees, read the way that is not a lie. See position_fees.py:
# `positions().tokensOwed` is only written when the position is touched, so
# between touches it reports zero on a pool that has been trading all month.
cmd_fees() {
  local pid; pid="$(jget JUNK_POSITION)"
  [ -n "$pid" ] || die "JUNK_POSITION unset — the LP position's tokenId, from the seed receipt. ./seed_junk.sh record --position <id>"
  (cd "$HERE/script" && python3 position_fees.py --position "$pid" ${LP_RECIPIENT:+--owner "$LP_RECIPIENT"})
}

# Collect and re-add. This is the only thing here that makes the pool grow
# without new capital, and nothing in this repo did it before 2026-08-22.
cmd_compound() {
  local arm; arm="$(armed "${1:-}")"
  local pid; pid="$(jget JUNK_POSITION)"
  [ -n "$pid" ] || die "JUNK_POSITION unset — ./seed_junk.sh record --position <id>"
  say "compound position $pid  [arm=$arm]"
  cmd_fees || die "refusing to compound off a failed read"
  note ""
  note "collect sweeps to the signer, then increaseLiquidity offers the wallet's"
  note "WHOLE balance of both sides — last run's unusable remainder included."
  note "⚠ fees do not arrive in the pool's ratio, so one side is always left over."
  note "  That dust is expected; the next run picks it up."
  [ "$arm" = "yes" ] || { note ""; note "(dry run — pass --arm to broadcast)"; return; }
  need forge; [ -n "${PRIVATE_KEY:-}" ] || die "PRIVATE_KEY not in the environment"
  POSITION_ID="$pid" V3_NPM="${V3_NPM:-0x73991a25C818Bf1f1128dEAaB1492D45638DE0D3}" \
    SLIP_BPS="$SLIP_BPS" forge script "$HERE/script/CompoundPosition.s.sol" \
    --rpc-url "$RPC" --broadcast
  note "run ./seed_junk.sh fees again — it should read near zero now"
}

cmd_season() {
  say "Gates' season calendar"
  # The pool's Junk side is derived, never stored: budget / price. It only
  # resolves once the wallet can be read, so a pre-launch run prints the
  # calendar alone rather than a drain rate off a failed balance read.
  # In a SUBSHELL: compute_size calls die() on a missing address, and die()
  # exits — which would take this whole command down, not just the branch.
  local pooljunk=""
  pooljunk="$( (compute_size >/dev/null 2>&1 && printf '%s' "$COINSIDE") 2>/dev/null || true )"
  if [ -n "$SEASON_BUDGET_JUNK" ]; then
    (cd "$HERE/script" && python3 season_cap.py --budget "$SEASON_BUDGET_JUNK" \
       ${pooljunk:+--pool-junk "$pooljunk"} --wei)
  else
    (cd "$HERE/script" && python3 season_cap.py)
    note ""
    note "SEASON_BUDGET_JUNK is unset — set it to see the daily cap it implies."
  fi
}

# Deploy the granary, grant the game server, and rotate Junk's minter — all
# three land in DeployGranary's one broadcast.
cmd_granary() {
  local arm; arm="$(armed "${1:-}")"
  require_addrs
  [ -n "${JUNK:-}" ] || die "JUNK_TOKEN unset — run deploy first"
  [ -n "$SEASON_BUDGET_JUNK" ] || die "SEASON_BUDGET_JUNK is unset. It has no default and must not get one — DeployGranary's own fallback is 250e18, which for a play coin clamps every extraction to nothing WITHOUT failing. Decide a season ceiling and pass it."
  [ -n "$GAME_MINTER" ] || die "GAME_MINTER is unset — the wallet the game server signs with. Without it the granary deploys with no grantee and nothing can mint."

  local capwei days
  read -r days capwei <<EOF
$(cd "$HERE/script" && python3 -c "
import datetime, season_cap
s = season_cap.calendar_from(datetime.date.today(), 1)[0]
cap = season_cap.daily_cap(float('$SEASON_BUDGET_JUNK'), s['days'])
print(s['days'], int(cap * 10**18))
")
EOF
  say "granary for $COIN_SYMBOL  [arm=$arm]"
  note "season ceiling   $SEASON_BUDGET_JUNK $COIN_SYMBOL over the next season ($days days)"
  note "daily cap        $(py -c "print(f'{int(\"$capwei\")/1e18:,.0f}')") $COIN_SYMBOL/day"
  note "grantee          $GAME_MINTER"
  note ""
  note "this one broadcast does three things: deploys EloGranary(Junk),"
  note "setGrant(grantee, cap), and rotates Junk's minter to the granary."
  note "after it, only the granary can mint — and only rotateMinterAway can undo that."
  note ""
  note "⚠ the cap is a CEILING, not a target. The granary clamps rather than"
  note "  reverting, so a day that runs over pays the remainder and reports it."
  note "⚠ re-run ./seed_junk.sh season at every season boundary — 28-day and"
  note "  35-day seasons alternate, so the cap moves 25% between them."
  [ "$arm" = "yes" ] || { note ""; note "(dry run — pass --arm to broadcast)"; return; }
  need forge; [ -n "${PRIVATE_KEY:-}" ] || die "PRIVATE_KEY not in the environment"
  GRANARY_TOKEN="$JUNK" GRANT_TO="$GAME_MINTER" GRANT_DAILY_CAP="$capwei" \
    forge script "$HERE/script/DeployGranary.s.sol" --rpc-url "$RPC" --broadcast
  note "now: ./seed_junk.sh record --granary <address>, and read back Junk.minter()"
}

cmd_record() {
  case "${1:-}" in
    --coin) jset JUNK_TOKEN "$2" ;;
    --pool) jset JUNK_POOL "$2" ;;
    --granary) jset JUNK_GRANARY "$2" ;;
    --position) jset JUNK_POSITION "$2" ;;
    *) die "record needs --coin, --pool, --granary <addr> or --position <tokenId>" ;;
  esac
  note "also record it in contracts/deployments.json — that file is the durable ledger"
  note "(foundry's broadcast/ is gitignored), and record what READBACK says, not what was planned"
}

case "${1:-}" in
  check)    cmd_check ;;
  deploy)   shift; cmd_deploy "${1:-}" ;;
  size)     cmd_size ;;
  plan)     shift; cmd_plan "${1:-}" ;;
  canary)   shift; cmd_seed "${1:-}" --canary ;;
  readback) cmd_readback ;;
  seed)     shift; cmd_seed "${1:-}" ;;
  fees)     cmd_fees ;;
  compound) shift; cmd_compound "${1:-}" ;;
  season)   cmd_season ;;
  granary)  shift; cmd_granary "${1:-}" ;;
  record)   shift; cmd_record "$@" ;;
  *) sed -n '1,44p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' ;;
esac
