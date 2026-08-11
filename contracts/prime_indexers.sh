#!/usr/bin/env bash
# prime_indexers.sh — write the first Swap events on the three game pools.
#
# WHY THIS EXISTS. DEX Screener and every other aggregator index a v3 pool off
# its `Swap` events. Our pools opened with real liquidity at real prices and a
# completely empty event log, so they are invisible to every chart — not hidden,
# not censored, just never traded. A handful of genuine swaps fixes that
# permanently.
#
# WHAT IT IS AND IS NOT. This is a ONE-SHOT primer: six swaps, both directions
# across all three pools, then it stops. It is not a volume bot and there is no
# loop in it. Two honest things follow from that:
#   - say it out loud when you post. "I made the first swaps myself so the
#     charts would populate" costs nothing and is true; letting the resulting
#     volume read as organic demand is the part that would not be.
#   - it doubles as the only end-to-end proof that the swap path WORKS from a
#     real wallet — router, approvals, calldata, slippage — before a stranger
#     is the one to discover it does not.
#
# THE DEADLINE IS NOT OPTIONAL. SwapRouter02's `exactInputSingle` has no
# deadline field (the original SwapRouter's was removed), and the router expects
# you to supply one by wrapping the call in `multicall(uint256 deadline,
# bytes[] data)`. Skip it and a signed swap can sit unmined and execute later at
# a price you never saw. Every leg below goes through the wrapper —
# `watchtower/js/pools.js` learned this first and its comment is the citation.
#
# Usage:
#   ./prime_indexers.sh              # DRY RUN: quotes every leg, sends nothing
#   ./prime_indexers.sh --arm        # sends
#
# Env:
#   PRIVATE_KEY     required to --arm
#   PRIME_SCRY      SCRY spent per opening leg (default 100000, ~$1.87)
#   PRIME_SLIP_BPS  slippage tolerance (default 200 = 2%)
#   RPC             default https://rpc.mainnet.chain.robinhood.com
set -euo pipefail
cd "$(dirname "$0")"

RPC="${RPC:-https://rpc.mainnet.chain.robinhood.com}"
ROUTER=0xcaf681a66d020601342297493863e78c959e5cb2   # SwapRouter02, Uniswap's own
QUOTER=0x33e885ed0ec9bf04ecfb19341582aadcb4c8a9e7   # QuoterV2
SCRY=0xDa2a4b23459e9ca88183e990802be644AcA7C4B0
OBOL=0xa003af4a6c38629a986545afc8f9312c7eb76220
MYRRH=0xde967108cC27dB651e8cFeC7dd18db814508b893
FEE=10000                                            # the 1% tier, all three
PRIME_SCRY="${PRIME_SCRY:-100000}"
SLIP_BPS="${PRIME_SLIP_BPS:-200}"

say()  { printf '\n== %s\n' "$*"; }
die()  { printf 'ABORT: %s\n' "$*" >&2; exit 1; }
armed=no; [ "${1:-}" = "--arm" ] && armed=yes

command -v cast >/dev/null || die "cast not on PATH (use the upstream foundry, not the zksync fork)"
[ "$(cast chain-id --rpc-url "$RPC")" = 4663 ] || die "not chain 4663"

sym() { case "${1,,}" in
  "${SCRY,,}") echo SCRY ;; "${OBOL,,}") echo OBOL ;; "${MYRRH,,}") echo MYRRH ;; *) echo "?" ;; esac; }
bal() { cast call "$1" 'balanceOf(address)(uint256)' "$2" --rpc-url "$RPC" | awk '{print $1}'; }
units() { python3 -c "print(f\"{int('$1')/10**18:,.4f}\")"; }

ME=""
if [ -n "${PRIVATE_KEY:-}" ]; then
  ME=$(cast wallet address --private-key "$PRIVATE_KEY") || die "PRIVATE_KEY is not a usable key"
fi
[ "$armed" = yes ] && [ -z "$ME" ] && die "--arm needs PRIVATE_KEY"
[ -z "$ME" ] && ME=0x24445EFddB08d4938E3E3627042B2Cf4063d9092   # quote-only default

say "primer: 6 swaps, both directions, all three pools — then it STOPS"
echo "   wallet   $ME"
echo "   router   $ROUTER  (SwapRouter02, Uniswap's own RH-Chain deployment)"
echo "   per leg  $PRIME_SCRY SCRY   slippage ${SLIP_BPS}bps"
[ "$armed" = yes ] || echo "   (dry run — add --arm to send)"

RAW_IN=$(python3 -c "print(int($PRIME_SCRY) * 10**18)")
NEED=$(python3 -c "print(2 * int($RAW_IN))")
HAVE=$(bal "$SCRY" "$ME")
echo
echo "   SCRY needed  $(units "$NEED")   (two opening legs)"
echo "   SCRY held    $(units "$HAVE")"
if python3 -c "import sys; sys.exit(0 if int('$HAVE') < int('$NEED') else 1)"; then
  echo "   !! short by $(units "$(python3 -c "print(int('$NEED')-int('$HAVE'))")") SCRY"
  [ "$armed" = yes ] && die "$ME does not hold the SCRY this primer spends.
   SCRY is transferred, never minted here. Send some in, then re-arm."
fi

# quote(tokenIn, tokenOut, amountIn) -> amountOut, via QuoterV2. A quote that
# reverts is a leg that would revert, so it fails loudly rather than defaulting.
quote() {
  local out
  out=$(cast call "$QUOTER" \
    'quoteExactInputSingle((address,address,uint256,uint24,uint160))(uint256,uint160,uint32,uint256)' \
    "($1,$2,$3,$FEE,0)" --rpc-url "$RPC" 2>&1) \
    || die "quoter refused $(sym "$1")->$(sym "$2") at $3 — the leg would revert:
   $out"
  printf '%s' "$out" | head -1 | awk '{print $1}'
}

# One leg: approve exactly what we spend (never an unlimited allowance), then
# swap through multicall so the deadline is real.
leg() {
  local tin="$1" tout="$2" amt="$3" n="$4"
  local q min inner dl
  q=$(quote "$tin" "$tout" "$amt")
  min=$(python3 -c "print(int('$q') * (10000 - $SLIP_BPS) // 10000)")
  printf '   %d. %-5s -> %-5s  in %s  quoted %s  min %s\n' \
    "$n" "$(sym "$tin")" "$(sym "$tout")" \
    "$(units "$amt")" "$(units "$q")" "$(units "$min")"
  [ "$armed" = yes ] || return 0

  cast send "$tin" 'approve(address,uint256)' "$ROUTER" "$amt" \
    --private-key "$PRIVATE_KEY" --rpc-url "$RPC" >/dev/null \
    || die "approve failed for $(sym "$tin")"

  inner=$(cast calldata \
    'exactInputSingle((address,address,uint24,address,uint256,uint256,uint160))' \
    "($tin,$tout,$FEE,$ME,$amt,$min,0)")
  dl=$(python3 -c "
import subprocess
b=subprocess.run(['cast','block','latest','--field','timestamp','--rpc-url','$RPC'],
                 capture_output=True,text=True).stdout.strip()
print(int(b)+600)")
  local tx
  tx=$(cast send "$ROUTER" 'multicall(uint256,bytes[])' "$dl" "[$inner]" \
        --private-key "$PRIVATE_KEY" --rpc-url "$RPC" 2>&1) \
    || die "swap $(sym "$tin")->$(sym "$tout") failed:
   $tx"
  printf '      sent  %s\n' "$(printf '%s' "$tx" | awk '/^transactionHash/{print $2}')"
}

say "the six legs"
leg "$SCRY"  "$OBOL"  "$RAW_IN" 1
leg "$SCRY"  "$MYRRH" "$RAW_IN" 2
if [ "$armed" = yes ]; then
  O=$(bal "$OBOL" "$ME");  H=$(python3 -c "print(int('$O')//2)")
  leg "$OBOL"  "$MYRRH" "$H" 3
  M=$(bal "$MYRRH" "$ME"); H=$(python3 -c "print(int('$M')//2)")
  leg "$MYRRH" "$OBOL"  "$H" 4
  O=$(bal "$OBOL" "$ME");  leg "$OBOL"  "$SCRY" "$O" 5
  M=$(bal "$MYRRH" "$ME"); leg "$MYRRH" "$SCRY" "$M" 6
else
  # A dry run cannot know what legs 1-2 will actually yield, so it quotes the
  # reverse legs off the QUOTED output rather than inventing a balance.
  QO=$(quote "$SCRY" "$OBOL" "$RAW_IN"); QM=$(quote "$SCRY" "$MYRRH" "$RAW_IN")
  leg "$OBOL"  "$MYRRH" "$(python3 -c "print(int('$QO')//2)")" 3
  leg "$MYRRH" "$OBOL"  "$(python3 -c "print(int('$QM')//2)")" 4
  leg "$OBOL"  "$SCRY"  "$(python3 -c "print(int('$QO')//2)")" 5
  leg "$MYRRH" "$SCRY"  "$(python3 -c "print(int('$QM')//2)")" 6
fi

if [ "$armed" = yes ]; then
  say "done — six Swap events are now on chain"
  echo "   SCRY  $(units "$(bal "$SCRY" "$ME")")"
  echo "   OBOL  $(units "$(bal "$OBOL" "$ME")")"
  echo "   MYRRH $(units "$(bal "$MYRRH" "$ME")")"
  cat <<'EOT'

   Indexers are pull-based and run on their own clock — minutes to hours, not
   instant. Check with:
     curl -s "https://api.dexscreener.com/latest/dex/pairs/robinhood/<pool>"

   And the honest sentence for the announcement, because it is free:
   these were the first swaps and they were mine.
EOT
else
  say "dry run complete — every leg quoted, nothing sent"
  echo "   the quotes above ARE the proof the router path works; a leg that"
  echo "   would revert fails this run rather than failing armed."
fi
