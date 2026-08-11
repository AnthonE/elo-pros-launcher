#!/usr/bin/env bash
# deploy_town.sh - phase-gated orchestrator for the whole on-chain town
# (spoils -> gardens -> granary/gardener -> silo -> shrine -> orchard).
#
# The runbook this script drives is contracts/RUNBOOK.md. House rules it
# ENFORCES rather than documents:
#   - DRY RUN by default: every phase simulates. Broadcasting takes --arm.
#   - forge test -vv green immediately before EVERY armed broadcast.
#   - addresses flow between phases through town.env (written by this
#     script, sourced on every run) - no copy-paste drift.
#   - canon addresses are verified against the chain, never trusted from
#     docs (POOLS.md 4.1).
#
# Usage:
#   ./deploy_town.sh check                 # tooling + RPC + canon addresses
#   ./deploy_town.sh preflight             # the OFF-CHAIN gate (../contracts/preflight.py)
#   ./deploy_town.sh test                  # forge build && forge test -vv
#   ./deploy_town.sh launch   [--arm]      # THE LAUNCH: spoils -> pools -> harvest
#                                          # (armed: seeds 0.1% and STOPS)
#   ./deploy_town.sh spoils   [--arm]      # OBOL + MYRRH + the ONE MYRRH/OBOL Garden
#   ./deploy_town.sh gardener [--arm]      # MYRRH granary + gardener + pool 0
#   ./deploy_town.sh granary  [--arm]      # the OBOL granary (silo + season pots)
#   ./deploy_town.sh cashier  [--arm]      # grant the payout wallet a POSTED daily
#                                          # mint cap on one granary - the whole
#                                          # security story of the ledgerless
#                                          # payout, in one call (CASHIER.md)
#   ./deploy_town.sh silo     [--arm]      # sealed lockers (+ MYRRH self-bin)
#   ./deploy_town.sh shrine   [--arm]      # votive burn-to-rank altars
#   ./deploy_town.sh pools    [--arm]      # mint the coin sides, seed the 3 v3 pools
#   ./deploy_town.sh harvest  [--arm]      # ScryHarvest (the players' claim)
#   ./deploy_town.sh bank     [--arm]      # ScryBank (xSCRY) + the SCRY fee
#                                          # splitter on the posted 50/40/0/10
#                                          # split. Needs nothing above it
#   ./deploy_town.sh orchard  [--arm]      # v3-staker (no season by default)
#   ./deploy_town.sh rotate   [--arm]      # retire the deployer's mint key (ONE-WAY)
#                                          # - the closing act when the depth
#                                          # phases run AFTER it
#   ./deploy_town.sh steward  [--arm]      # retire the granary steward seat (ONE-WAY)
#                                          # - the closing act when the depth
#                                          # phases run BEFORE it, because
#                                          # `gardener`/`granary` already took
#                                          # minter and stewardMint is UNCAPPED
#   ./deploy_town.sh verify   [--arm]      # publish source for what we broadcast
#   ./deploy_town.sh status                # read the wiring back from chain
#   ./deploy_town.sh claims                # posted roots vs the merkle artifacts
#   ./deploy_town.sh config                # emit watchtower/gardens.config.json
#
# Env this script reads (never echoed):
#   PRIVATE_KEY    required to --arm (the deploy scripts read it themselves)
#   RPC            default https://rpc.mainnet.chain.robinhood.com
#   NPM V3_FACTORY required by `pools` and `orchard`, and demanded UP FRONT by
#                  an armed `launch` (verified on-chain first)
#   LAUNCH_SKIP_CANARY=1  seed the pools full-size in one act (default: 0.1%
#                  first, then stop for the read-back)
#   ROTATE_FORCE=1 rotate despite an unproven precondition (ONE-WAY)
#   STEWARD_FORCE=1 same, for `steward` (ONE-WAY)
#   POOL_SCRY_BUDGET_OBOL / _MYRRH   the two $SCRY pool sides (76.5M / 23.5M)
#   POOL_SCRY_BUDGET                 a TOTAL, split on that ratio (canary knob)
#   MINTER_NEXT[_OBOL|_MYRRH]        where the mint role retires to (rotate)
#   STEWARD_NEXT[_OBOL|_MYRRH]       where the granary steward seat retires to
#                                    (steward). Leave one unset to retire only
#                                    the other granary
#   POWDER_TO                        operator wallet for the posted OBOL/MYRRH
#                                    allocation (unset = no allocation at all)
#   POWDER_OBOL / POWDER_MYRRH       TWO RULES, not one, since 2026-07-28.
#                                    OBOL 1,000,000 is POSTED (read back from
#                                    LAUNCH-DECISIONS.md, capped at 25% of the
#                                    pool float); MYRRH 240 is 10% OF WHAT THE
#                                    DROP PAYS, recomputed from the snapshot in
#                                    use. The drop stays whole either way.
#                                    preflight grades each against its OWN rule
#                                    and reds on drift - never retype these
#   SCRY_BPS_BURN/_BANK/_PRIZES/_OPS the posted fee split, spoken 2026-07-27:
#                                    5000 / 4000 / 0 / 1000. DERIVED - `bank`
#                                    reads them out of DeployScryEconomy.s.sol
#                                    rather than keeping a copy. Export one to
#                                    override and the phase names both values
#   SCRY_OPS                         the ops fee sink. DEFAULTS TO $DEV_WALLET
#                                    (operator 2026-07-29) and says so out loud;
#                                    never to the deployer, which is what the
#                                    forge script would do. Refuses outright if
#                                    you point it AT the deployer
#   LP_RECIPIENT                     who holds the v3 position NFTs - the deed to
#                                    the whole 100M SCRY of depth. Same posted
#                                    default, same reason
#   SCRY_PRIZE_ESCROW                only needed if you give prizes bps; must
#                                    differ from SCRY_OPS or the cuts merge.
#                                    Prizes are 0 in the posted split, so unset
#   MINTER_NEXT* / STEWARD_NEXT*     NO default, deliberately - see below. Both
#                                    are ONE-WAY, and the refusal prints the
#                                    posted address for you to paste
# Everything else (caps, rates, windows) passes through to the forge
# scripts untouched - see each script's header for its knobs.

set -euo pipefail

cd "$(dirname "$0")"

RPC="${RPC:-https://rpc.mainnet.chain.robinhood.com}"
CHAIN_ID_WANT=4663
SCRY_CANON=0xDa2a4b23459e9ca88183e990802be644AcA7C4B0
# LIVE since 2026-07-17 and unpatchable. `status` reads its oracle back because
# `setOracle` has no zero-check and no owner path home (RUNBOOK.md 4b).
VOW_REGISTRY=0x08131e7660639bbd086dffa9375c2a563f1d3590
TOWN_ENV="${TOWN_ENV:-town.env}"   # overridable so a phase can be rehearsed
                                   # against a scratch address ledger

# THE DEV WALLET — the one posted destination for anything that must name an
# address and must NOT be the deployer. Operator, 2026-07-29: *"set anything we
# are not setting to deployer to the dev wallet the same as powder."*
# (SENTENCES.md; the powder itself was pointed here on 2026-07-27.)
#
# It is written ONCE, here, and every field below reads it — the alternative is
# the same 42 characters retyped in four places, which is how one of them ends
# up a character out on launch night with no gate able to tell.
#
# Why not simply let these fall through to the deployer: the deployer is a hot
# key that exists to broadcast and then be retired. A fee sink and an LP deed
# that outlive the launch should not sit on it. That is a posture, not a
# custody claim — this address is an EOA the operator holds (an EIP-7702
# account; `snapshots/house-wallets.json` records it and why `code.length != 0`
# is a false alarm here), and it is NOT a multisig, because none exists yet.
DEV_WALLET=0xb474a95200bC5de8950E445B1E9d524f4de0f18D

say()  { printf '\n== %s\n' "$*"; }
die()  { printf 'ABORT: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing tool: $1"; }
# case-insensitive address compare (checksummed vs lowercase is not a mismatch)
lc()   { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }

# The gate AND every broadcast must run on UPSTREAM foundry, never the
# foundry-zksync fork that ships in ~/.foundry/bin on some boxes. The fork's
# via_ir handling breaks vm.warp (14 spurious failures on this repo) and -
# far worse - `forge script --broadcast` would compile + deploy zkSync
# artifacts, NOT the standard EVM bytecode RH-Chain (Arbitrum L2) runs. The
# proven-green gate was run on upstream v1.7.1. Fail closed if PATH resolves
# the fork. (scry-contracts-toolchain memory; TEST-AUDIT.md.)
#
# UPSTREAM_FORGE_BIN is the sanctioned side-by-side install: upstream lives
# THERE and never in ~/.foundry/bin, because that directory belongs to the
# zksync fork the morr side needs. If PATH hands us the fork and a sane
# upstream is sitting in that directory, we prepend it rather than dying -
# refusing to run over a fixable PATH ordering was costing whole sessions.
UPSTREAM_FORGE_BIN="${UPSTREAM_FORGE_BIN:-$HOME/.foundry-upstream/bin}"

forge_is_fork() { case "$1" in *zksync*|*zkSync*|*ZKsync*) return 0 ;; *) return 1 ;; esac; }

need_upstream_forge() {
    need forge
    local v; v=$(forge --version 2>/dev/null || true)
    [ -n "$v" ] || die "forge --version returned nothing - is forge on PATH?"

    if forge_is_fork "$v" && [ -x "$UPSTREAM_FORGE_BIN/forge" ]; then
        local u; u=$("$UPSTREAM_FORGE_BIN/forge" --version 2>/dev/null || true)
        if [ -n "$u" ] && ! forge_is_fork "$u"; then
            PATH="$UPSTREAM_FORGE_BIN:$PATH"; export PATH
            hash -r 2>/dev/null || true
            v="$u"
            echo "   PATH's forge was the zksync fork; using $UPSTREAM_FORGE_BIN"
        fi
    fi

    forge_is_fork "$v" && die "PATH's forge is the foundry-zksync fork:
     $v
   scry's contracts need UPSTREAM foundry (v1.7.x). The fork breaks
   via_ir/vm.warp and broadcasts zkSync bytecode, not EVM. Install upstream
   side by side (NOT into ~/.foundry/bin, which the fork owns):
     D=\$HOME/.foundry-upstream/bin; mkdir -p \$D; cd \$D
     V=v1.7.1   # the asset name is versioned; the _stable_ alias 404s
     curl -sSL -O https://github.com/foundry-rs/foundry/releases/download/\$V/foundry_\${V}_linux_amd64.tar.gz
     curl -sSL -O https://github.com/foundry-rs/foundry/releases/download/\$V/foundry_\${V}_linux_amd64.sha256
     sha256sum -c foundry_\${V}_linux_amd64.sha256 && tar xzf foundry_\${V}_linux_amd64.tar.gz
   then rerun (this script finds it there automatically)."

    echo "   forge ok: $(printf '%s' "$v" | head -1)"
}

# town.env is the address ledger between phases. KEY=0x... lines only.
town_get() { [ -f "$TOWN_ENV" ] && grep -E "^$1=" "$TOWN_ENV" | tail -1 | cut -d= -f2 || true; }
town_set() {
    touch "$TOWN_ENV"
    # `|| true` was swallowing TWO different exits. grep returns 1 for "no line
    # matched", which is the ordinary case for a new key — and 2 for "I could not
    # do it": a full disk, a read-only mount, a concurrent session racing the
    # `mv`. The unconditional `mv` then installed an EMPTY file over the address
    # ledger, and the phase printed a cheerful `town.env KEY=0x…` for the single
    # surviving key while every contract already broadcast became unreachable to
    # every later phase ("need spoils first" for coins that are live on chain).
    # Separate the two, and keep one generation back. (Review, 2026-07-29.)
    g=0; grep -vE "^$1=" "$TOWN_ENV" > "$TOWN_ENV.tmp" || g=$?
    [ "$g" -le 1 ] || die "could not rewrite $TOWN_ENV (grep exit $g).
   The address ledger is UNTOUCHED and nothing was lost. Fix the filesystem
   before running another phase - a later phase reading a truncated town.env
   would report contracts that are live on chain as missing."
    cp "$TOWN_ENV" "$TOWN_ENV.bak" 2>/dev/null || true
    mv "$TOWN_ENV.tmp" "$TOWN_ENV"
    printf '%s=%s\n' "$1" "$2" >> "$TOWN_ENV"
    printf '   town.env  %s=%s\n' "$1" "$2"
}

# THE FARM'S CROP, DERIVED ONCE. It has moved MYRRH -> OBOL (2026-07-25) ->
# MYRRH (2026-07-26, when play stopped minting MYRRH and the Garden became its
# only source, FARMING.md 3a) - and every move left a SECOND place in this file
# still asserting the old coin. First `status`, which printed a confident
# "correct" for a coin the farm does not pay. Then `config`, which wrote the
# wrong symbol AND the wrong token address into gardens.config.json, the file
# the gardens page reads - the only one of the three that reaches a player.
# Both now ask this function, and it reads the `gardener` phase's own
# REWARD_TOKEN line, so a fourth move cannot desync anything.
farm_coin_sym() {
    if grep -q 'export REWARD_TOKEN="\$OBOL" SEED_MYRRH_OBOL' "$0"; then
        echo OBOL
    else
        echo MYRRH
    fi
}

# Pull CREATE addresses (in order) out of foundry's broadcast receipt.
creates_from_broadcast() { # $1 = script file name, e.g. DeploySpoils.s.sol
    local f="broadcast/$1/$CHAIN_ID_WANT/run-latest.json"
    [ -f "$f" ] || die "no broadcast receipt at $f (did the arm succeed?)"
    python3 - "$f" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for t in d.get("transactions", []):
    if t.get("transactionType") == "CREATE":
        print(t.get("contractName"), t.get("contractAddress"))
PY
}

# The gate in front of every broadcast: a full green suite, every time.
#
# Memoized PER PROCESS, not per phase. The house rule is "green immediately
# before EVERY armed broadcast" and that still holds - but a phase that sends
# more than one transaction (the `pools` mint, then the seed) would otherwise
# run the whole suite plus eight live-fork mandates twice for two sends a few
# seconds apart, which buys nothing and tempts the next person to skip it.
# Across phases the memo does NOT apply: `launch` invokes each phase as its own
# process, so every phase still re-proves the gate.
TESTS_GREEN_DONE=0
tests_green() {
    if [ "${TESTS_GREEN_DONE:-0}" = 1 ]; then
        echo "   (suite already proven green in this run)"
        return 0
    fi
    say "forge build + full test suite (the gate in front of any broadcast)"
    need_upstream_forge
    # --sizes, and it is not a nicety. `forge test` NEVER asks how big a contract
    # is: on 2026-07-27 the gacha's toll rail put ScryGacha 366 bytes over
    # EIP-170's 24,576-byte runtime limit and the whole suite stayed green over a
    # contract that could not be deployed at all. A gate that proves behaviour
    # and not deployability is the shape of green this repo keeps getting caught
    # by. `--sizes` exits nonzero when anything is over.
    forge build --sizes || die "a contract is over the EIP-170 runtime limit - it cannot be deployed, green suite or not"
    forge test -vv || die "tests are red - nothing broadcasts on red. Ever."
    say "seed-helper selftest (the sqrtP/amount math the spoils seeding trusts)"
    python3 script/seed_spoils_uniswap.py --selftest \
        || die "seed_spoils_uniswap.py --selftest is red - the price math gates the arm too"
    # The read-back is the only check standing between a mispriced first mint
    # and 100,000,000 $SCRY of depth on top of it. It replaced a heredoc that
    # asked a human to run cast calls, so it has to be PROVEN to abort - an
    # automatic check that cannot fail is worse than the heredoc it replaced,
    # because it claims to have looked. Every case in there reintroduces the
    # bug it exists for.
    say "pool read-back gate (proves pools_read_before/after actually abort)"
    ./test_pool_readback.sh \
        || die "test_pool_readback.sh is red - the launch's only price check is not trustworthy"
    fork_mandates_green
    TESTS_GREEN_DONE=1
}

# Whole units, for reading. Never used for arithmetic - every amount this
# script sends is raw wei, computed in python from raw wei.
fmt_units() { python3 -c 'import sys; print(f"{int(sys.argv[1]) / 1e18:,.0f}")' "$1"; }
sub_floor0() { python3 -c 'import sys; print(max(0, int(sys.argv[1]) - int(sys.argv[2])))' "$1" "$2"; }
add_raw()    { python3 -c 'import sys; print(int(sys.argv[1]) + int(sys.argv[2]))' "$1" "$2"; }
lc()         { printf '%s' "$1" | tr 'A-F' 'a-f'; }

# A balance the chain would not give us is NOT a balance of zero, and this one
# decides how much gets minted. Fail loudly rather than hand a blank to the
# arithmetic - an empty string reaching python is a traceback in the middle of
# a launch, and a zero reaching it would mint the whole float a second time.
erc20_bal() {
    local out v
    out=$(cast call "$1" 'balanceOf(address)(uint256)' "$2" --rpc-url "$RPC" 2>&1) \
        || die "could not read balanceOf($2) at $1 — the chain did not answer:
   $out"
    v=$(printf '%s' "$out" | awk '{print $1}')
    case "$v" in
        ''|*[!0-9]*) die "balanceOf($2) at $1 returned '$v', which is not a number.
   Is that address really an ERC-20 on chain $CHAIN_ID_WANT?" ;;
    esac
    printf '%s' "$v"
}

# ── the ratio lock ──────────────────────────────────────────────────────────
# THE THREE OPENING PRICES ARE THE ONE THING NOTHING HERE CAN UNDO, and the
# read-back is structurally unable to check them: sqrtP is COMPUTED FROM the
# ratio, so a wrong SCRY_PER_OBOL passes every arithmetic assertion at full
# confidence and welds the pool at the wrong price. That is the warning above
# `pools_read_before`, and it ended "only the operator can say the script
# matches what he decided". That is no longer the whole truth — LAUNCH-DECISIONS
# posts the three numbers in a table, so the machine CAN check the script
# against the posted decision, and this is that check.
#
# THE HOLE IT CLOSES. preflight already compares the driver's defaults to that
# table — but it does so by reading this file's TEXT, and the ratios are
# `${SCRY_PER_OBOL:-10}`. An exported SCRY_PER_OBOL re-prices the entire launch
# while preflight AND pool_seeds.py both keep reporting 10, because both read
# the source and neither reads the environment. Proven, not theorised:
# `SCRY_PER_OBOL=7 python3 pool_seeds.py` still prints "@ 10 SCRY/OBOL".
#
# So this compares the RESOLVED, ABOUT-TO-BE-USED values against the posted
# table at the moment of use. The difference between "the file says 10" and
# "the number going into sqrtP is 10" is the whole point.
#
# RATIO_ACK=1 is the deliberate-re-price escape, and it is meant to be rare:
# a v3 pool's first mint IS its price.
ratios_locked() {
    local out rc
    # `if out=$(…)` and NOT `out=$(…); rc=$?` — this file runs under `set -e`,
    # where a failing command substitution in a bare assignment kills the script
    # BEFORE the next line reads $?. The gate then aborts the launch with an
    # empty stderr and exit 1, which reads as a crash rather than as a refusal.
    # Caught by rehearsing `pools` in situ; a unit test of this function passed
    # because its harness did not set -e.
    if out=$(python3 - "$SCRY_PER_OBOL" "$SCRY_PER_MYRRH" "$OBOL_PER_MYRRH" <<'EOPY'
import re, sys
runtime = [float(x) for x in sys.argv[1:4]]
try:
    doc = open("../docs/launch/LAUNCH-DECISIONS.md", encoding="utf-8").read()
except OSError as e:
    print(f"cannot read LAUNCH-DECISIONS.md ({e}) — the posted table is the "
          f"authority for these three numbers and it is unreadable"); sys.exit(2)
pats = (("SCRY per OBOL",  r"\|\s*([\d.]+)\s*SCRY\s*/\s*OBOL\s*\|"),
        ("SCRY per MYRRH", r"\|\s*([\d.]+)\s*SCRY\s*/\s*MYRRH\s*\|"),
        ("OBOL per MYRRH", r"\|\s*([\d.]+)\s*OBOL\s*/\s*MYRRH\s*\|"))
posted, missing = [], []
for label, rx in pats:
    m = re.search(rx, doc)
    posted.append(float(m.group(1)) if m else None)
    if not m:
        missing.append(label)
if missing:
    print("the posted table no longer carries: " + ", ".join(missing)); sys.exit(2)
bad = [f"{lbl}: about to use {got:g}, LAUNCH-DECISIONS posts {want:g}"
       for (lbl, _), got, want in zip(pats, runtime, posted)
       if abs(got - want) > 1e-9]
# The third is the CROSS of the other two and must stay so, or the launch opens
# with the exact genesis arb the third pool exists to foreclose.
if abs(posted[2] * posted[0] - posted[1]) > 1e-9:
    bad.append(f"the posted table is internally inconsistent: "
               f"{posted[1]:g}/{posted[0]:g} = {posted[1]/posted[0]:g}, not {posted[2]:g}")
if bad:
    print("\n      ".join(bad)); sys.exit(1)
print(f"{posted[0]:g} SCRY/OBOL · {posted[1]:g} SCRY/MYRRH · {posted[2]:g} OBOL/MYRRH")
EOPY
    ); then
        echo "   ok ratios match the posted table — $out"
        return 0
    else
        rc=$?
    fi
    [ "$rc" = 2 ] && die "the ratio lock could not read its own authority.

      $out

   This is not a mismatch — it is the check being unable to run, which is the
   one outcome that must never pass silently. LAUNCH-DECISIONS.md's ratio table
   is what `pools` is graded against."
    if [ "${RATIO_ACK:-0}" = 1 ]; then
        printf '\n   !! RATIO_ACK=1 — PROCEEDING ON RATIOS THAT DO NOT MATCH THE POSTED TABLE:\n      %s\n' "$out"
        return 0
    fi
    die "the opening prices do not match LAUNCH-DECISIONS.md.

      $out

   A v3 pool's first mint IS its price, and the read-back cannot catch this —
   it derives sqrtP from these very numbers, so a wrong ratio passes every
   check it makes. Check your environment first: these resolve from
   \`\${SCRY_PER_OBOL:-10}\`, so a stray export re-prices the launch while
   preflight and pool_seeds.py both still report the posted number.

     env | grep -E 'SCRY_PER_|POOL_.*BUDGET'

   If the re-price is deliberate, change the table in LAUNCH-DECISIONS.md
   (preflight reads it, so the two stay married) — or, for a one-off rehearsal,
   RATIO_ACK=1."
}

# ── the powder door ─────────────────────────────────────────────────────────
# THE POWDER IS MINTABLE ONLY WHILE THE DEPLOY KEY HOLDS `minter`, and FOUR
# phases close that door for good: `gardener` and `granary` hand the slot to a
# granary (DeployGardener.s.sol:176, DeployGranary.s.sol:55), `rotate` and
# `steward` retire it. `setMinter` is onlyMinter and there is no admin door, so
# a powder deferred and then forgotten is unmintable FOREVER — 1,000,000 OBOL
# that LAUNCH-DECISIONS.md posts as an allocation and the chain never gets.
#
# This is the other half of the rule `spoils` already enforces. That phase
# refuses to GUESS `POWDER_TO`; this one refuses to let the answer "later"
# expire silently. Deferring is a real and recommended choice — the first sitting
# picks `POWDER_TO=none` so the chain shows players made whole before the house
# took its cut — but until now "later" had a deadline that nothing announced.
#
# It reads the chain rather than a flag: whoever the powder was aimed at, the
# posted destination is $DEV_WALLET, so a balance at or above the posted amount
# is proof it landed. POWDER_ACK=1 means "minted by hand, or never wanted".
powder_door() {
    local phase="$1" armed_p="$2" obol myrrh want_o want_m bal_o bal_m missing
    if [ "${POWDER_ACK:-0}" = 1 ]; then
        echo "   POWDER_ACK=1 — powder door acknowledged, not checked"; return 0
    fi
    obol=$(town_get OBOL_TOKEN); myrrh=$(town_get MYRRH_TOKEN)
    # Nothing deployed yet means nothing to strand.
    [ -n "$obol" ] || return 0
    want_o=$(python3 -c 'import sys; print(int(sys.argv[1]) * 10**18)' "${POWDER_OBOL:-1000000}")
    want_m=$(python3 -c 'import sys; print(int(sys.argv[1]) * 10**18)' "${POWDER_MYRRH:-240}")
    bal_o=$(erc20_bal "$obol" "$DEV_WALLET"); missing=""
    python3 -c 'import sys; sys.exit(0 if int(sys.argv[1]) >= int(sys.argv[2]) else 1)' \
        "$bal_o" "$want_o" || missing="OBOL $(fmt_units "$want_o") (holds $(fmt_units "$bal_o"))"
    if [ -n "$myrrh" ]; then
        bal_m=$(erc20_bal "$myrrh" "$DEV_WALLET")
        python3 -c 'import sys; sys.exit(0 if int(sys.argv[1]) >= int(sys.argv[2]) else 1)' \
            "$bal_m" "$want_m" \
            || missing="${missing:+$missing, }MYRRH $(fmt_units "$want_m") (holds $(fmt_units "$bal_m"))"
    fi
    if [ -z "$missing" ]; then
        echo "   ok the powder has landed — $DEV_WALLET holds it, this phase may close the mint"
        return 0
    fi
    printf '\n   !! THE POWDER IS NOT MINTED, AND `%s` CLOSES THE DOOR ON IT.\n\n' "$phase"
    printf '      unminted: %s\n' "$missing"
    cat <<EOPOWDER
      destination: $DEV_WALLET (LAUNCH-DECISIONS.md, posted)

      \`$phase\` moves or retires SpoilsToken.minter, and setMinter is onlyMinter
      with NO admin door. After it, the deploy key cannot mint and neither can
      you. Mint it FIRST, while the seat is still yours:

        cast send \$OBOL_TOKEN  'mint(address,uint256)' $DEV_WALLET ${POWDER_OBOL:-1000000}000000000000000000
        cast send \$MYRRH_TOKEN 'mint(address,uint256)' $DEV_WALLET ${POWDER_MYRRH:-240}000000000000000000

      Or, if the powder is deliberately abandoned (POWDER_TO=none was a real
      answer and you are keeping it), say so and this stops asking:

        POWDER_ACK=1 ./deploy_town.sh $phase ${armed_p:+--arm}
EOPOWDER
    [ "$armed_p" = yes ] && die "refusing to close the mint on an unminted powder — mint it, or pass POWDER_ACK=1"
    return 0
}

# ── the pool read-back, in code instead of in a heredoc ──────────────────────
# `pools --arm` used to PRINT a `cast call` recipe and ask the operator to
# eyeball whether the pool's slot0 sqrtPriceX96 matched a number printed forty
# lines earlier. Both sides of that comparison are already in this process, so
# it is arithmetic, and arithmetic left to a human at 3am on launch night is
# arithmetic that does not happen. It is a machine's job and it is now done
# here, twice: once BEFORE the broadcast (so an abort costs nothing) and once
# after (so what actually landed is proven, not assumed).
#
# ⚠ WHAT THIS CANNOT DO: every check below compares the chain against THIS
# SCRIPT'S OWN INTENTION. sqrtP is computed from the ratio the script was
# handed, so against a wrong SCRY_PER_OBOL every assertion here passes with
# maximum confidence and the pool is welded at the wrong price. This repo has
# the receipt - the POOL_SCRY_BUDGET bug would have seeded the operator's own
# SCRY at a split he had revised the day before, with every arithmetic check
# green.
#
# That gap is now covered ONE LAYER UP rather than by a human: `ratios_locked`
# checks the resolved prices against the posted LAUNCH-DECISIONS table before
# anything is derived from them, so the tautology below runs on numbers that
# have already been matched to the decision. What is left for the operator is
# strictly the policy question - whether the posted table is what he meant -
# and not the arithmetic. Automate the tautology, keep the human for the
# policy; the line between those two moved, and this is where it sits now.
ZERO_ADDR=0x0000000000000000000000000000000000000000
# The fee tier every read below asks the factory for. It MUST track what
# SeedSpoilsUniswapV3.s.sol mints into (`vm.envOr("FEE", DEFAULT_FEE)`, the 1%
# tier) - a read-back against the wrong tier would look at an empty slot and
# report "no pool" while the real one sat beside it.
POOL_FEE="${FEE:-10000}"
# Relative PRICE drift tolerated on a pool that already exists (the top-up run).
# Price is sqrtP squared, so this is compared against (got/want)^2 - 1.
POOL_DRIFT_MAX_BPS="${POOL_DRIFT_MAX_BPS:-100}"

# The three pools this phase opens: label|tokenA|tokenB|expected-sqrtP.
# getPool sorts the pair internally so the order here is free, and slot0's
# sqrtPriceX96 is quoted in SORTED order - which is exactly what the seed
# helper computes into *_SQRTP. The two are directly comparable.
pool_triples() {
    printf '%s\n' \
        "OBOL/\$SCRY|$OBOL|$SCRY_CANON|${OBOL_SQRTP:-}" \
        "MYRRH/\$SCRY|$MYRRH|$SCRY_CANON|${MYRRH_SQRTP:-}" \
        "MYRRH/OBOL|$MYRRH|$OBOL|${MYRRH_OBOL_SQRTP:-}"
}

v3_pool_addr() {
    local out
    out=$(cast call "$V3_FACTORY" 'getPool(address,address,uint24)(address)' \
            "$1" "$2" "$POOL_FEE" --rpc-url "$RPC" 2>&1) \
        || die "could not read getPool($1, $2, $POOL_FEE) — the chain did not answer:
   $out"
    lc "$(printf '%s' "$out" | awk 'NR==1{print $1}')"
}

# A price the chain would not give us is NOT a price of zero. Same rule as
# erc20_bal, and it matters more here: this number decides whether 100,000,000
# $SCRY goes in on top of what is already there.
v3_sqrtp() {
    local out v
    out=$(cast call "$1" 'slot0()(uint160,int24,uint16,uint16,uint16,uint8,bool)' \
            --rpc-url "$RPC" 2>&1) \
        || die "could not read slot0() at pool $1 — the chain did not answer:
   $out"
    v=$(printf '%s' "$out" | awk 'NR==1{print $1}')
    case "$v" in
        ''|*[!0-9]*) die "slot0() at $1 returned '$v', which is not a number.
   Is that really a Uniswap v3 pool on chain $CHAIN_ID_WANT?" ;;
    esac
    printf '%s' "$v"
}

# (got/want)^2 - 1, as a signed percentage. Price, not sqrtP: a 1% sqrtP move
# is a ~2% price move, and the number the operator reasons about is the price.
sqrtp_drift_pct() {
    python3 -c 'import sys
w, g = int(sys.argv[1]), int(sys.argv[2])
print(f"{((g / w) ** 2 - 1) * 100:+.4f}")' "$1" "$2"
}

# Set by pools_read_before, consumed by pools_read_after. One line per pool:
#   label|addr|(new|<sqrtP>)
POOLS_BEFORE=""

# BEFORE the broadcast. Two jobs: record which pools we are about to CREATE (so
# the after-check knows which ones must match exactly), and refuse outright if a
# pool already exists at a price that has drifted off the ratio this run is
# about to seed at. That second one is the top-up's real hazard and nothing was
# checking it: between the canary and the full seed the pool is LIVE and
# tradeable, so a stranger can move the price, and depositing the posted ratio
# into a moved pool mints a position at a mix you did not choose.
pools_read_before() {
    say "reading the three pools BEFORE anything broadcasts"
    POOLS_BEFORE=""
    local label a b want addr sp drift
    while IFS='|' read -r label a b want; do
        [ -n "$want" ] || die "the seed helper exported no sqrtP for $label — refusing to broadcast blind"
        addr=$(v3_pool_addr "$a" "$b")
        if [ "$addr" = "$ZERO_ADDR" ]; then
            printf '   %-14s does not exist yet — this run OPENS it, and its first mint IS its price\n' "$label"
            POOLS_BEFORE="${POOLS_BEFORE}${label}|${addr}|new
"
            continue
        fi
        sp=$(v3_sqrtp "$addr")
        drift=$(sqrtp_drift_pct "$want" "$sp")
        printf '   %-14s exists at %s — live price is %s%% off this run'\''s ratio\n' "$label" "$addr" "$drift"
        python3 -c 'import sys
drift, maxbps = abs(float(sys.argv[1])), int(sys.argv[2])
sys.exit(0 if drift <= maxbps / 100 else 1)' "$drift" "$POOL_DRIFT_MAX_BPS" || die \
"$label has moved $drift% away from the ratio this seed was computed at.
   The pool is live between the canary and the top-up, so someone traded it.
   Topping up at the posted ratio now deposits a mix you did not choose, and
   the slippage bounds may revert after you have paid the gas.
   Decide which is true before re-running:
     - the market is right  -> the seed ratio is stale; re-price or accept it
     - the price is wrong   -> arb it back FIRST, then top up
   Override once you have decided: POOL_DRIFT_MAX_BPS=<bps> (now $POOL_DRIFT_MAX_BPS)"
        POOLS_BEFORE="${POOLS_BEFORE}${label}|${addr}|${sp}
"
    done <<EOF
$(pool_triples)
EOF
}

# AFTER the broadcast. Every pool must exist; one we just created must sit at
# EXACTLY the sqrtP the helper computed; one that already existed must still be
# inside the drift bound. Anything else and we stop before the top-up, which is
# the entire point of seeding 0.1% first.
pools_read_after() {
    local lp_to="$1" lp_before="$2"
    say "READ-BACK: every number below came off the chain, not out of town.env"
    local bad=0 label a b want addr got was drift lp_after
    while IFS='|' read -r label a b want; do
        addr=$(v3_pool_addr "$a" "$b")
        if [ "$addr" = "$ZERO_ADDR" ]; then
            printf '   !! %-14s STILL HAS NO POOL at fee %s\n' "$label" "$POOL_FEE"
            bad=1; continue
        fi
        got=$(v3_sqrtp "$addr")
        was=$(printf '%s' "$POOLS_BEFORE" | awk -F'|' -v l="$label" '$1==l{print $3}')
        if [ "$was" = new ]; then
            if [ "$got" = "$want" ]; then
                printf '   ok %-14s %s  opened at the posted ratio (sqrtP exact)\n' "$label" "$addr"
            else
                printf '   !! %-14s %s\n      OPENED AT A PRICE THIS SCRIPT DID NOT CHOOSE\n      want %s\n      got  %s  (%s%%)\n' \
                    "$label" "$addr" "$want" "$got" "$(sqrtp_drift_pct "$want" "$got")"
                bad=1
            fi
        else
            drift=$(sqrtp_drift_pct "$want" "$got")
            if python3 -c 'import sys
sys.exit(0 if abs(float(sys.argv[1])) <= int(sys.argv[2]) / 100 else 1)' \
                "$drift" "$POOL_DRIFT_MAX_BPS"; then
                printf '   ok %-14s %s  topped up, price %s%% off the ratio (within bound)\n' "$label" "$addr" "$drift"
            else
                printf '   !! %-14s %s  price is %s%% off the ratio AFTER the top-up\n' "$label" "$addr" "$drift"
                bad=1
            fi
        fi
    done <<EOF
$(pool_triples)
EOF

    # The LP position NFT is the deed to everything just deposited. The seeder
    # only WARNS when the recipient has code; nothing confirmed the NFT
    # actually landed. Three mints, three positions.
    if [ -n "$lp_to" ] && [ -n "${NPM:-}" ]; then
        lp_after=$(erc20_bal "$NPM" "$lp_to")   # NPM is an ERC-721; balanceOf is the same selector
        if [ "$lp_after" -gt "$lp_before" ] 2>/dev/null; then
            printf '   ok LP positions   %s holds %s (was %s) — the deed landed in your wallet\n' \
                "$lp_to" "$lp_after" "$lp_before"
        else
            printf '   !! LP positions   %s holds %s, was %s — NO NEW POSITION NFT\n' \
                "$lp_to" "$lp_after" "$lp_before"
            bad=1
        fi
    fi

    [ "$bad" = 0 ] || die "the chain does not agree with what this script intended.
   STOP. Do not top up — every unit of depth added on top of a wrong opening
   price makes it more expensive to correct, and the correction is paid to
   whoever arbs it. Read the three lines above and reconcile them by hand."
    say "read-back clean — the chain matches this script"
    cat <<'EON'
   And now the half a machine cannot check: the script proved the pools match
   the ratios IT WAS GIVEN. Nothing above knows whether those ratios are the
   ones you decided. Confirm the prices in plain words before you top up:
EON
    printf '     1 OBOL  = %s $SCRY\n     1 MYRRH = %s $SCRY\n     1 MYRRH = %s OBOL\n' \
        "$SCRY_PER_OBOL" "$SCRY_PER_MYRRH" "$OBOL_PER_MYRRH"
}

# The offline suite SKIP-passes the fork mandates: OrchardFork + SeedSpoilsFork
# return early when RH_FORK_URL is unset (a plain `forge test` never touches
# the one thing no mock covers - real NPM/pool bytecode + a real tick walk).
# Those two suites are exactly what REPLACES a testnet, so the arm gate MUST
# run them against live chain and PROVE they executed rather than skipped.
fork_mandates_green() {
    say "fork mandates against live RH-Chain (the testnet-replacement suite)"
    local out
    if ! out=$(RH_FORK_URL="$RPC" forge test --match-contract 'OrchardFork|SeedSpoilsFork|ScryGachaFork' -vv 2>&1); then
        printf '%s\n' "$out"
        die "fork mandates are red - nothing broadcasts on red. Ever."
    fi
    printf '%s\n' "$out"
    if printf '%s' "$out" | grep -q 'SKIP:'; then
        die "fork mandates SKIPPED - RH_FORK_URL not honored; the testnet-replacement tests never ran"
    fi
    # The count is DERIVED from this list, not typed beside it: the old line
    # read "(4/4)" and would have kept saying 4 while the list grew.
    local t
    local mandates=(
        test_fork_real_npm_full_range_mint_stake_warp_unstake
        test_fork_thin_range_clock_stops_out_of_range
        test_fork_late_unstake_pays_no_more_than_the_close
        test_fork_defaults_match_live_chain
        test_fork_real_npm_seed_creates_pool_and_mints_lp
        # The gacha runs against a REAL collection's deployed bytecode and real
        # holders. No mock covers what actually kills this contract: an operator
        # filter that refuses a pool address, a pause hook, or a transferFrom
        # that returns without moving anything.
        test_fork_theRealCollectionIsCompatible
        test_fork_aRealDrawEndToEnd
        test_fork_theBidPathReturnsARealTokenToItsDepositor
        test_fork_whatOnePullActuallyCosts
    )
    for t in "${mandates[@]}"; do
        printf '%s' "$out" | grep -q "$t" \
            || die "fork mandate '$t' did not run - refusing to broadcast"
    done
    say "fork mandates executed live (${#mandates[@]}/${#mandates[@]}) - the testnet-replacement gate is real"
}

# forge script runner: dry-run unless --arm was passed to the phase.
run_script() { # $1 = script path, $2 = armed (yes/no)
    need_upstream_forge   # dry-run compiles too - a fork sim is a lie
    if [ "$2" = yes ]; then
        [ -n "${PRIVATE_KEY:-}" ] || die "--arm needs PRIVATE_KEY in the env"
        tests_green
        say "BROADCASTING $1 (chain $CHAIN_ID_WANT)"
        forge script "$1" --rpc-url "$RPC" --broadcast
    else
        say "dry run: $1 (no --arm, nothing moves; add --arm to broadcast)"
        # simulation still wants a key for vm.envUint; any throwaway works
        PRIVATE_KEY="${PRIVATE_KEY:-0x0000000000000000000000000000000000000000000000000000000000000001}" \
            forge script "$1" --rpc-url "$RPC"
    fi
}

armed() { [ "${1:-}" = "--arm" ] && echo yes || echo no; }

remind_ledger() {
    cat <<'EOF'

   Broadcast done. TWO records to keep, same commit (deployments.json rule):
     1. paste the new addresses into contracts/deployments.json
        (an entry there means BROADCAST AND LIVE);
     2. commit town.env alongside it.
   Then run: ./deploy_town.sh status
EOF
}

cmd="${1:-help}"
shift || true

case "$cmd" in

check)
    need_upstream_forge; need cast; need python3
    say "chain"
    got=$(cast chain-id --rpc-url "$RPC")
    [ "$got" = "$CHAIN_ID_WANT" ] || die "RPC serves chain $got, want $CHAIN_ID_WANT (RH-Chain)"
    echo "   RPC $RPC -> chain $got ok"
    say "canon \$SCRY has code on-chain (docs are never the authority)"
    code=$(cast code "$SCRY_CANON" --rpc-url "$RPC")
    [ "$code" != "0x" ] || die "no code at canon \$SCRY $SCRY_CANON"
    echo "   \$SCRY $SCRY_CANON ok ($((${#code} / 2 - 1)) bytes)"
    if [ -n "${NPM:-}" ] || [ -n "${V3_FACTORY:-}" ]; then
        [ -n "${NPM:-}" ] && [ -n "${V3_FACTORY:-}" ] || die "set NPM and V3_FACTORY together"
        say "orchard plumbing: NPM must mint against the factory we validate with"
        [ "$(cast code "$NPM" --rpc-url "$RPC")" != "0x" ] || die "no code at NPM $NPM"
        [ "$(cast code "$V3_FACTORY" --rpc-url "$RPC")" != "0x" ] || die "no code at factory $V3_FACTORY"
        fac=$(cast call "$NPM" 'factory()(address)' --rpc-url "$RPC")
        [ "$(echo "$fac" | tr 'A-F' 'a-f')" = "$(echo "$V3_FACTORY" | tr 'A-F' 'a-f')" ] \
            || die "npm.factory() = $fac, not $V3_FACTORY - wrong pairing"
        echo "   NPM/factory pairing ok"
    else
        echo "   (NPM/V3_FACTORY unset - fine until the orchard phase)"
    fi
    say "seed-helper selftest (the sqrtP/amount math the spoils seeding trusts)"
    python3 script/seed_spoils_uniswap.py --selftest \
        || die "seed_spoils_uniswap.py --selftest is red - fix it before anything arms"
    say "preflight clean"
    ;;

test)
    need forge; need python3
    tests_green
    ;;

spoils)
    need forge; need python3
    spoils_armed="$(armed "${1:-}")"
    # THE POWDER IS A DECISION, AND ITS DEFAULT IS SILENCE. `POWDER_TO` unset
    # means no allocation is minted at all, so an armed run refuses to guess:
    # name the wallet, or say `none` out loud and mean it.
    #
    # ⚠ "MINTED IN THIS PHASE AND NOWHERE ELSE" WAS TOO STRONG, and it pushed
    # the operator into paying HIMSELF FIRST. This block is where the script
    # mints it; it is not where the CHAIN stops allowing it. The deployer holds
    # OBOL's minter slot until `granary`/`gardener`/`rotate` takes it, which is
    # phases later — so `POWDER_TO=none` here and one `cast send` afterwards is
    # a perfectly good path, and it is the better one:
    #
    #   spoils → pools → harvest → DeployClaim → FUND THE CLAIMS → then powder
    #
    # Players are made whole before the house takes its working float, which is
    # the order anyone reading the chain would want to see. It costs one extra
    # transaction and nothing else.
    if [ "$spoils_armed" = yes ] && [ -z "${POWDER_TO:-}" ]; then
        die "POWDER_TO is unset. Name it, or defer it deliberately.
   The powder is the operator's own working allocation of OBOL and MYRRH — the
   coins you hand out, seed pots with, and pay people in.
   THE RULE (operator, 2026-07-28): OBOL is a POSTED FIXED allocation, read back
   from LAUNCH-DECISIONS.md; MYRRH keeps the 10%-of-drop rule because it is
   capped and its headroom is ~1,000,000 against a 40-year schedule. The drop
   stays whole either way — the powder is on top of it, never carved out.
   Defaults ${POWDER_OBOL:-1,000,000} OBOL + ${POWDER_MYRRH:-240} MYRRH;
   \`preflight\` reds if either drifts from its own rule.
     POWDER_TO=0x…  ./deploy_town.sh spoils --arm     # mint it here, now
     POWDER_TO=none ./deploy_town.sh spoils --arm     # defer, or never

   DEFERRING IS NOT LOSING IT. The deployer keeps OBOL's minter slot until
   \`granary\`/\`gardener\`/\`rotate\` moves it, so after the claim contracts
   are deployed AND FUNDED you can mint it by hand:
     cast send \$OBOL_TOKEN  'mint(address,uint256)' \$POWDER_TO <obol-wei>
     cast send \$MYRRH_TOKEN 'mint(address,uint256)' \$POWDER_TO <myrrh-wei>
   That pays the players before it pays the house. Do it BEFORE the phase that
   takes the slot, and \`status\` will tell you who holds it."
    fi
    # ── THE WELDED VALUES, GATED FROM SOURCE (2026-07-28) ───────────────────
    # Every SpoilsToken constructor value is a `vm.envOr`, and this phase
    # validated exactly one of them (POWDER_TO). A stale OBOL_CAP left over in
    # the shell from any earlier session welds an IMMUTABLE cap onto a coin that
    # is meant to ship uncapped — and preflight.py's own words for a cap on
    # OBOL are "a countdown". The names have no setter either: a wrong
    # OBOL_NAME reads as a permanent typo in every wallet. GARDEN_SEED_* set the
    # Garden's opening price, which its first add welds forever.
    #
    # Same shape as the `bank` phase's `bank_posted`: DERIVE, NEVER RETYPE. The
    # posted defaults live in exactly one place — DeploySpoils.s.sol's own
    # `vm.envOr` literals — and are read back OUT of it here, so this file
    # cannot drift from the thing it is about to broadcast. An override is
    # allowed and is a new sentence; it just has to be spoken out loud.
    spoils_posted() { # $1 = env var name -> the posted default, from the script
        # two literal shapes: uint256(<number>) and string("<text>")
        #
        # `/^[[:space:]]*\/\//d` FIRST, and it is load-bearing. The house
        # convention writes a superseded value in a comment ABOVE the live line
        # (DeploySpoils.s.sol already carries three such ⚠ blocks for the powder
        # sizings), and `head -1` takes the first match — so without this, one
        # `// was vm.envOr("OBOL_CAP", uint256(21_000_000e18)) until 2026-07-27`
        # makes this function report 21_000_000e18 as the posted default for a
        # value that is 0. A gate that names the wrong posted number is worse
        # than no gate: the operator reconciles the difference in the wrong
        # direction. Found by review, 2026-07-29.
        sed -n "/^[[:space:]]*\/\//d;\
                s/.*vm\.envOr(\"$1\", *uint256(\([0-9_e]*\)).*/\1/p;\
                s/.*vm\.envOr(\"$1\", *string(\"\([^\"]*\)\").*/\1/p" \
            script/DeploySpoils.s.sol | head -1
    }
    # NUMERIC EQUALITY, NOT STRING EQUALITY. The posted literal is Solidity
    # (`20_000e18`); the env var must be decimal wei, because that is what
    # `vm.envOr(..., uint256)` parses, and the runbook instructs exactly
    # that form. So a string compare calls the operator's byte-for-byte-correct
    # export an override, on EVERY run, and the only way past it is
    # SPOILS_FORCE=1. That is the failure that matters: the force flag becomes
    # routine, and a genuinely stale OBOL_CAP rides through inside the same
    # blanket override. Normalize both sides to an integer and compare numbers;
    # anything that is not a number (the two names) falls back to the string.
    weld_norm() { # $1 = a Solidity literal or an env value -> canonical form
        python3 -c 'import re,sys
v = sys.argv[1].replace("_", "")
m = re.fullmatch(r"([0-9]+)(?:[eE]([0-9]+))?", v)
print(str(int(m.group(1)) * 10 ** int(m.group(2) or 0)) if m else v)' "$1"
    }
    spoils_overridden=""
    for v in OBOL_CAP MYRRH_CAP OBOL_NAME MYRRH_NAME GARDEN_SEED_MYRRH GARDEN_SEED_OBOL \
             POWDER_OBOL POWDER_MYRRH; do
        posted="$(spoils_posted "$v")"
        [ -n "$posted" ] || die "could not read the posted default for $v out of
   script/DeploySpoils.s.sol - the literal must keep the form
   \`vm.envOr(\"$v\", <literal>)\` for this phase to derive it."
        # `${!v+set}`, not `[ -n "$cur" ]`: foundry's envOr falls back to the
        # default only when the variable is ABSENT. `export OBOL_NAME=` (a
        # truncated paste, or VAR=$OTHER where $OTHER was unset) is present and
        # empty, and welds a nameless ERC-20 with no setter. An emptiness test
        # skips exactly that case.
        if [ -n "${!v+set}" ]; then
            cur="${!v}"
            if [ "$(weld_norm "$cur")" != "$(weld_norm "$posted")" ]; then
                spoils_overridden="$spoils_overridden
     $v  posted ${posted}  ->  you set ${cur:-(empty)}"
            fi
        fi
    done
    if [ -n "$spoils_overridden" ]; then
        cat <<EOF

   !! YOU ARE OVERRIDING A VALUE THIS PHASE WELDS PERMANENTLY:$spoils_overridden
      Caps and names have NO setter, and the Garden's opening add sets its price
      for good. If this is deliberate, append the sentence to SENTENCES.md and
      fix the default in DeploySpoils.s.sol so the next run does not differ.
      If it is a leftover shell variable, \`unset\` it and re-run.
EOF
        [ "$spoils_armed" != yes ] || [ "${SPOILS_FORCE:-0}" = 1 ] || die \
            "refusing to weld an unposted value. Re-run with SPOILS_FORCE=1 to
   say you mean it, or unset the variable(s) above."
    fi

    if [ "${POWDER_TO:-}" = none ]; then
        # `none` is the spoken refusal; the deploy script reads an UNSET var as
        # "no allocation", and "none" is not an address it could parse.
        unset POWDER_TO
        echo "   POWDER_TO=none — no operator allocation will be minted, ever."
    elif [ -n "${POWDER_TO:-}" ]; then
        # The powder address gets a presence check and `!= address(0)` in
        # Solidity, and nothing anywhere vets the address ITSELF — preflight
        # grades only the amounts. A transposed hex character mints the whole
        # allocation to a wallet nobody holds, and those coins are unburnable
        # by anyone else: `burn` is msg.sender-scoped and `burnFrom` needs the
        # holder's allowance. Same nonce/code tell `rotate` uses; a warning
        # rather than a refusal, because a fresh, never-used wallet is a
        # legitimate destination and only the operator knows which this is.
        #
        # Runs on a DRY RUN too, and `need cast` rather than `command -v` — both
        # were the same bug in opposite directions. Gated on `armed`, the one
        # rehearsal that could catch a typo said nothing about the address; and
        # skipped when cast was missing, the check evaporated silently on a box
        # that had never installed it. A typo filter that is quiet in exactly
        # the two situations where you would look at it is not a filter.
        need cast
        p_nonce=$(cast nonce "$POWDER_TO" --rpc-url "$RPC" 2>/dev/null || echo "")
        p_code=$(cast code "$POWDER_TO" --rpc-url "$RPC" 2>/dev/null || echo "0x")
        # AN EIP-7702 ACCOUNT IS AN EOA WITH A DELEGATE, NOT A CONTRACT, and
        # `rotate` already knows that while this did not — the same fix, not
        # ported to its sibling, which is the failure mode this repo keeps
        # writing down. It is not cosmetic here: `code.length != 0` is TRUE for
        # a 7702 wallet, so the "a contract" arm swallowed the nonce check
        # entirely, and the powder is aimed at 0xb474a952…0f18d, which IS a 7702
        # account on chain 4663 (measured 2026-07-29, nonce 129;
        # snapshots/house-wallets.json says the same and says why). The typo
        # filter was disabled for precisely the address class it was aimed at.
        case "$p_code" in
        # 23 bytes of 0xef0100 || delegate. Normalize to "no code" so the nonce
        # tell below runs, and keep the label for the line that prints.
        0xef0100*) p_kind="EIP-7702 EOA, delegate 0x${p_code#0xef0100}"; p_code=0x ;;
        0x)        p_kind="EOA" ;;
        *)         p_kind="contract" ;;
        esac
        if [ "$p_code" != "0x" ]; then
            echo "   powder -> $POWDER_TO (a contract - make sure it can move ERC20s)"
        elif [ -z "$p_nonce" ]; then
            echo "   ?? powder -> $POWDER_TO: the chain would not answer nonce() - address unvetted"
        elif [ "$p_nonce" = 0 ]; then
            cat <<EOF

   ?? POWDER_TO $POWDER_TO has never signed a transaction on this chain
      (nonce 0, $p_kind). That is normal for a fresh wallet and fatal for a
      typo: the powder is minted once, here, and nobody but the holder can
      burn it back. Confirm you control it before arming.
EOF
        else
            echo "   powder -> $POWDER_TO ($p_kind, nonce $p_nonce - this key has signed before)"
        fi
    fi
    run_script script/DeploySpoils.s.sol "$spoils_armed"
    if [ "$spoils_armed" = yes ]; then
        say "recording addresses"
        creates_from_broadcast DeploySpoils.s.sol | {
            spoils_i=0; garden_i=0
            while read -r name addr; do
                case "$name" in
                SpoilsToken)
                    [ $spoils_i = 0 ] && town_set OBOL_TOKEN "$addr" || town_set MYRRH_TOKEN "$addr"
                    spoils_i=1 ;;
                # ONE Garden now, MYRRH/OBOL, both sides house-minted
                # (operator 2026-07-25): a ScryGarden never holds canonical
                # $SCRY. Real depth is the canonical v3 pool, POOLS.md 3.
                ScryGarden)
                    town_set GARDEN_MYRRH_OBOL "$addr"
                    garden_i=1 ;;
                esac
            done
        }
        remind_ledger
    fi
    ;;

gardener)
    need forge; need python3
    # THE FARM PAYS MYRRH, AND IS ITS ONLY SOURCE (operator, 2026-07-26).
    # Play mints no MYRRH at all now (SCRY_BARROW_MYRRH_BANDS is empty), so the
    # 07-25 move to OBOL is superseded: that move throttled the FARM while
    # barrow room 3 kept minting 2,509 MYRRH/day at dau=200 against 1,320/day
    # burned, and the coin diverged anyway. Cutting the SOURCE is what worked.
    # FARMING.md 3a. The farmed pool is unchanged: the MYRRH/OBOL Garden SEED,
    # because a ScryGarden never holds canonical $SCRY at all (AUDIT F5).
    #
    # TWO GRANARIES, decided 2026-07-26. ScryGranary binds ONE SpoilsToken
    # because SpoilsToken has ONE minter slot, so the coin the farm pays and the
    # coin the silo drips need one each: MYRRH here, OBOL from `./deploy_town.sh
    # granary` (script/DeployGranary.s.sol). One shared granary would make the
    # silo drip MYRRH too -- 240/day instead of the posted 120 -- and delete the
    # OBOL drip. Same audited contract twice, and no UI names a granary.
    OBOL=$(town_get OBOL_TOKEN); MYRRH=$(town_get MYRRH_TOKEN)
    SEED=$(town_get GARDEN_MYRRH_OBOL)
    [ -n "$MYRRH" ] && [ -n "$SEED" ] \
        || die "need spoils first (town.env lacks MYRRH_TOKEN / GARDEN_MYRRH_OBOL)"
    export REWARD_TOKEN="$MYRRH" SEED_MYRRH_OBOL="$SEED"
    echo "   farm reward: MYRRH $MYRRH"
    echo "   farmed pool: MYRRH/OBOL SEED $SEED"
    echo "   granary:     MYRRH-bound (the silo gets its own: ./deploy_town.sh granary)"
    powder_door gardener "$(armed "${1:-}")"
    run_script script/DeployGardener.s.sol "$(armed "${1:-}")"
    if [ "$(armed "${1:-}")" = yes ]; then
        say "recording addresses"
        creates_from_broadcast DeployGardener.s.sol | while read -r name addr; do
            case "$name" in
            ScryGranary)  town_set GRANARY_MYRRH "$addr"; town_set GRANARY "$addr" ;;
            ScryGardener) town_set GARDENER "$addr" ;;
            esac
        done
        remind_ledger
    fi
    ;;

granary)
    need forge; need python3
    # The OBOL granary. ScryGranary binds ONE SpoilsToken because SpoilsToken
    # has ONE minter slot, so the farm's coin (MYRRH, deployed by `gardener`)
    # and the silo's coin (OBOL) need one each. Same audited contract twice.
    OBOL=$(town_get OBOL_TOKEN)
    [ -n "$OBOL" ] || die "need spoils first (town.env lacks OBOL_TOKEN)"
    export GRANARY_TOKEN="$OBOL"
    echo "   granary token: OBOL $OBOL"
    echo "   (grant the silo AFTER it exists: granary.setGrant(silo, cap))"
    powder_door granary "$(armed "${1:-}")"
    run_script script/DeployGranary.s.sol "$(armed "${1:-}")"
    if [ "$(armed "${1:-}")" = yes ]; then
        say "recording addresses"
        creates_from_broadcast DeployGranary.s.sol | while read -r name addr; do
            case "$name" in
            ScryGranary) town_set GRANARY_OBOL "$addr" ;;
            esac
        done
        remind_ledger
    fi
    ;;

cashier)
    need forge
    # THE LEDGERLESS PAYOUT'S ONLY ON-CHAIN ACT. `meter/payout.py` mints a
    # winner's coin straight to their wallet through
    # `ScryGranary.mint(player, amount)`, which is bounded by a per-address DAILY
    # CAP this phase sets. So this number is the blast radius of a hot key that
    # can mint, and it is the reason a grant is an acceptable place for one:
    # a stranger reads the cap at `grants(cashier)` and the steward kills it with
    # one `revokeGrant`, no notice period (CASHIER.md).
    #
    # NOTHING IS DEFAULTED. A cap that appeared because an env var was unset is
    # not a posted cap, and the coin decides which granary - OBOL's and MYRRH's
    # are different contracts because SpoilsToken has one minter slot each.
    COIN="${SCRY_CASHIER_COIN:-OBOL}"
    case "$COIN" in
    OBOL)  GRANARY_A=$(town_get GRANARY_OBOL) ;;
    MYRRH) GRANARY_A=$(town_get GRANARY_MYRRH) ;;
    *)     die "SCRY_CASHIER_COIN must be OBOL or MYRRH (got '$COIN')" ;;
    esac
    [ -n "$GRANARY_A" ] || die "no $COIN granary in town.env - run:
   ./deploy_town.sh $([ "$COIN" = MYRRH ] && echo gardener || echo granary) --arm
   (a granary holds ONE token's minter slot; they are not interchangeable)"
    [ -n "${CASHIER:-}" ] || die "export CASHIER=0x... - the payout wallet
   (the same address meter runs as SCRY_PAYOUT_FROM). No default: this is the
    key that will be able to mint, so it gets pasted deliberately."
    [ -n "${CASHIER_DAILY_CAP:-}" ] || die "export CASHIER_DAILY_CAP=<whole $COIN/day>
   Size it as what the house can afford to lose in ONE DAY to a bug or a stolen
   key - never as what a good day pays out. A banked delve is ~740 OBOL, so a
   few hundred delves is a few hundred thousand. There is no honest default."
    export GRANARY="$GRANARY_A"
    echo "   coin:        $COIN"
    echo "   granary:     $GRANARY_A"
    echo "   cashier:     $CASHIER"
    echo "   daily cap:   $CASHIER_DAILY_CAP whole $COIN"
    run_script script/GrantCashier.s.sol "$(armed "${1:-}")"
    if [ "$(armed "${1:-}")" = yes ]; then
        cat <<EOF

   Granted. The cap is now PUBLIC - anyone can read it:
     cast call $GRANARY_A "availableToday(address)(uint256)" $CASHIER --rpc-url \$RPC

   Two more things, and the meter pays nobody until both are done:
     1. HELD-KEYS.md - add/refresh the cashier's key row (it can mint up to
        the cap; that is a new KIND of held key, not another posted knob);
     2. arm the meter: SCRY_PAYOUT=1, SCRY_PAYOUT_FROM=$CASHIER,
        SCRY_PAYOUT_GRANARIES='{"$COIN":"$GRANARY_A"}', then GET /payout and
        check cap_room_today reads back what you just set.

   To undo: granary.revokeGrant($CASHIER) from the steward. Immediate, no notice.
EOF
    fi
    ;;

silo)
    need forge; need python3
    # The silo drips OBOL, so it spends the OBOL granary's grant - NOT the
    # gardener's, which binds MYRRH. Passing the wrong one makes the silo emit
    # MYRRH silently (FARMING.md 3b). Refuse rather than guess.
    GRANARY_A=$(town_get GRANARY_OBOL); OBOL=$(town_get OBOL_TOKEN); MYRRH=$(town_get MYRRH_TOKEN)
    [ -n "$GRANARY_A" ] || die "the silo needs the OBOL granary: ./deploy_town.sh granary --arm
   (town.env lacks GRANARY_OBOL. The gardener's granary binds MYRRH and is NOT
    interchangeable - a granary holds one token's minter slot.)"
    [ -n "$OBOL" ] || die "need spoils first (town.env lacks OBOL_TOKEN)"
    export GRANARY="$GRANARY_A" OBOL_TOKEN="$OBOL"
    echo "   granary:     OBOL-bound $GRANARY_A"
    # the MYRRH self-bin (alloc 25) posts automatically when the token is known
    [ -n "$MYRRH" ] && export MYRRH_TOKEN="$MYRRH"
    run_script script/DeploySilo.s.sol "$(armed "${1:-}")"
    if [ "$(armed "${1:-}")" = yes ]; then
        say "recording addresses"
        creates_from_broadcast DeploySilo.s.sol | while read -r name addr; do
            # `case`, not `[ … ] && …`: the while body's last command is the
            # loop's exit status, so a non-matching final CREATE line made the
            # pipeline return 1 and `set -e` killed the phase AFTER a successful
            # broadcast - suppressing remind_ledger and, inside `launch`,
            # aborting every phase after it. Dormant today (one CREATE, it
            # matches); fires the moment a script gains a second deploy, or when
            # foundry writes a CREATE with a null contractName.
            case "$name" in ScrySilo) town_set SILO "$addr" ;; esac
        done
        remind_ledger
    fi
    ;;

shrine)
    need forge; need python3
    OBOL=$(town_get OBOL_TOKEN); MYRRH=$(town_get MYRRH_TOKEN)
    [ -n "$OBOL" ] && [ -n "$MYRRH" ] || die "need spoils first (town.env lacks OBOL_TOKEN / MYRRH_TOKEN)"
    export OBOL_TOKEN="$OBOL" MYRRH_TOKEN="$MYRRH"
    run_script script/DeployShrine.s.sol "$(armed "${1:-}")"
    if [ "$(armed "${1:-}")" = yes ]; then
        say "recording addresses"
        creates_from_broadcast DeployShrine.s.sol | while read -r name addr; do
            case "$name" in ScryShrine) town_set SHRINE "$addr" ;; esac
        done
        remind_ledger
    fi
    ;;

orchard)
    need forge; need python3
    # A season pays **OBOL** (SEASONS.md 4: OBOL is the wage, MYRRH is never
    # emitted to LPs). Unchanged by the 2026-07-26 farm flip -- the GARDENER
    # moved to MYRRH, the ORCHARD did not. A pot is funded by
    # granary.stewardMint and a granary binds one token, so the pot comes from
    # the **OBOL** granary (./deploy_town.sh granary), not the gardener's.
    OBOL=$(town_get OBOL_TOKEN)
    [ -n "$OBOL" ] || die "need spoils first (town.env lacks OBOL_TOKEN)"
    [ -n "${NPM:-}" ] && [ -n "${V3_FACTORY:-}" ] || die "export NPM and V3_FACTORY (then run: ./deploy_town.sh check)"
    # never silently clobber an operator's own export: a pre-set REWARD_TOKEN
    # that is not OBOL is either a typo or a deliberate act this phase refuses
    # to guess about (SEASONS.md 4: the season wage is OBOL).
    if [ -n "${REWARD_TOKEN:-}" ] && [ "$(lc "$REWARD_TOKEN")" != "$(lc "$OBOL")" ]; then
        die "REWARD_TOKEN is exported as $REWARD_TOKEN but town.env's OBOL_TOKEN is $OBOL
   (the Orchard pays OBOL. unset REWARD_TOKEN and re-run, or fix town.env)"
    fi
    export REWARD_TOKEN="$OBOL"
    echo "   season coin: OBOL $OBOL"
    # season 0 is NOT posted here by default (SEASON_POT unset = none).
    # Posting a season is its own operator act - RUNBOOK.md 2, the arming decisions.
    run_script script/DeployOrchard.s.sol "$(armed "${1:-}")"
    if [ "$(armed "${1:-}")" = yes ]; then
        say "recording addresses"
        creates_from_broadcast DeployOrchard.s.sol | while read -r name addr; do
            case "$name" in ScryOrchard) town_set ORCHARD "$addr" ;; esac
        done
        remind_ledger
    fi
    ;;

bank|economy)
    # The SCRY side of the economy: ScryBank (stake SCRY -> xSCRY) + the
    # ScryFeeSplitter that feeds it. Deliberately independent of every phase
    # above it - no minter, no granary, no spoils address, no pool. It can ride
    # anywhere in a sitting, which is exactly why it kept getting left out: it
    # never blocked anything, so nothing ever demanded it.
    #
    # THE ONE NUMBER, and it is spoken: burn 5000 / bank 4000 / prizes 0 /
    # ops 1000 (2026-07-27, SENTENCES.md - closes FEES.md 9 question 2).
    #
    # This phase used to refuse to arm unless all four were exported, because a
    # default nobody chose must not weld itself. That gate has done its job and
    # is retired: demanding four exports for a decided number is friction with
    # no safety left in it. What replaces it is the habit the powder rule
    # earned - DERIVE, NEVER RETYPE. The numbers live in exactly one place,
    # DeployScryEconomy.s.sol's own defaults, and are read back OUT of it here,
    # so this file cannot drift from the thing it is about to broadcast.
    need forge; need python3
    bank_armed="$(armed "${1:-}")"
    export SCRY_TOKEN="${SCRY_TOKEN:-$SCRY_CANON}"
    bank_posted() { # $1 = BURN|BANK|PRIZES|OPS -> the posted default, from the script
        sed -n "s/.*SCRY_BPS_$1\", uint256(\([0-9]\+\)).*/\1/p" \
            script/DeployScryEconomy.s.sol | head -1
    }
    if [ "$bank_armed" = yes ]; then
        overridden=""
        for v in BURN BANK PRIZES OPS; do
            posted="$(bank_posted "$v")"
            [ -n "$posted" ] || die "could not read the posted SCRY_BPS_$v out of
   script/DeployScryEconomy.s.sol - the literal must keep the form
   \`SCRY_BPS_$v\", uint256(N)\` for this phase to derive it."
            cur="SCRY_BPS_$v"
            if [ -z "${!cur:-}" ]; then
                printf -v "$cur" '%s' "$posted"      # take the posted sentence
            elif [ "${!cur}" != "$posted" ]; then
                overridden="$overridden
     SCRY_BPS_$v  posted $posted  ->  you set ${!cur}"
            fi
        done
        # An override is allowed and is NOT a warning to skim past: it means the
        # chain is about to carry a number the ledger does not.
        [ -z "$overridden" ] || cat <<EOF

   !! YOU ARE OVERRIDING THE POSTED SPLIT (SENTENCES.md 2026-07-27):$overridden
      That is fine, and it is a new sentence - append it to the ledger and fix
      the defaults in DeployScryEconomy.s.sol, or the next run silently differs.
EOF
        # Their sum is the constructor's own require; check it HERE so a wrong
        # total costs a shell exit instead of a reverted broadcast's gas.
        sum=$(( SCRY_BPS_BURN + SCRY_BPS_BANK + SCRY_BPS_PRIZES + SCRY_BPS_OPS ))
        [ "$sum" = 10000 ] || die "the four bps sum to $sum, not 10000 (the constructor reverts on this)"
        # POSTED DEFAULT, not a silent one. `DeployScryEconomy.s.sol` falls back
        # to `vm.addr(pk)` — the deployer — which would post a hot key that
        # exists to be retired as a permanent public fee recipient. The operator
        # named the dev wallet for exactly this class of field (2026-07-29), so
        # take it and SAY SO, loudly, every run. An override is still an
        # override; it just has to be spoken.
        if [ -z "${SCRY_OPS:-}" ]; then
            SCRY_OPS="$DEV_WALLET"
            printf '   SCRY_OPS unset -> the posted dev wallet %s\n' "$SCRY_OPS"
            printf '     (operator 2026-07-29; NOT the deployer, which is what the script would default to)\n'
        fi
        # A fee sink that IS the deployer is the thing the default above exists
        # to prevent, so an explicit one gets refused rather than shrugged at.
        if [ -n "${PRIVATE_KEY:-}" ]; then
            ops_me=$(cast wallet address --private-key "$PRIVATE_KEY" 2>/dev/null || echo "")
            [ -z "$ops_me" ] || [ "$(lc "$SCRY_OPS")" != "$(lc "$ops_me")" ] || die \
                "SCRY_OPS is the DEPLOYER ($ops_me). That posts a key meant to be
   retired as a permanent public fee recipient. Name a lasting address —
   the posted one is \$DEV_WALLET ($DEV_WALLET)."
        fi
        # Two cuts to one address merge silently, and the posted table then
        # advertises a prize programme that is really just ops. The forge script
        # requires this too; saying it here saves the gas.
        if [ "${SCRY_BPS_PRIZES}" -gt 0 ]; then
            [ -n "${SCRY_PRIZE_ESCROW:-}" ] || die "export SCRY_PRIZE_ESCROW, or set SCRY_BPS_PRIZES=0.
   Unset, it defaults to SCRY_OPS - so the split would post four outlets and
   pay three, with 'prizes' landing in the ops wallet."
            [ "$(lc "$SCRY_PRIZE_ESCROW")" != "$(lc "$SCRY_OPS")" ] \
                || die "SCRY_PRIZE_ESCROW and SCRY_OPS are the same address"
        fi
        # EXPORT WHAT WAS VALIDATED, and do not assume the caller did. Checking
        # a shell variable and then broadcasting whatever forge happens to see
        # is the `HARVEST_TOKEN` defect wearing a different name (2026-07-27,
        # the fork rehearsal): unexported, every check above passes on a value
        # the deploy script never receives, and `vm.envOr` quietly welds the
        # DEFAULT split instead of the one just printed and approved.
        export SCRY_BPS_BURN SCRY_BPS_BANK SCRY_BPS_PRIZES SCRY_BPS_OPS SCRY_OPS
        [ -n "${SCRY_PRIZE_ESCROW:-}" ] && export SCRY_PRIZE_ESCROW

        say "the split you are about to weld"
        printf '   burn   (0xdEaD)  %5s bps\n' "$SCRY_BPS_BURN"
        printf '   bank   (xSCRY)   %5s bps\n' "$SCRY_BPS_BANK"
        printf '   prizes           %5s bps  %s\n' "$SCRY_BPS_PRIZES" "${SCRY_PRIZE_ESCROW:-(no line)}"
        printf '   ops              %5s bps  %s\n' "$SCRY_BPS_OPS" "$SCRY_OPS"
        [ "${SCRY_BPS_BURN}" -eq 0 ] && echo "   (no burn line: do not advertise a burn on any surface)"
    fi
    run_script script/DeployScryEconomy.s.sol "$bank_armed"
    if [ "$bank_armed" = yes ]; then
        say "recording addresses"
        creates_from_broadcast DeployScryEconomy.s.sol | while read -r name addr; do
            case "$name" in
                ScryBank) town_set BANK "$addr" ;;
                ScryFeeSplitter) town_set FEE_SPLITTER "$addr" ;;
            esac
        done
        cat <<'EOF'

   THE BANK IS EMPTY, AND THAT IS THE CORRECT LAUNCH STATE.
   Do NOT transfer a seed tranche into it. While totalSupply() == 0 the whole
   balance belongs to whoever deposits FIRST, whatever they deposit - one SCRY
   takes the lot, and MINIMUM_SHARES is a rounding artefact, not a haircut.
   TREASURY.md P9's 9M is a trickle budget gated on a real stake existing;
   `./deploy_town.sh status` refuses to call the bank seedable until then.

   Two surfaces read these addresses (neither is on-chain, both are edits):
     meter/ecosystem.config.js   SCRY_BANK / SCRY_FEE_SPLITTER  -> GET /bank
     watchtower/bank.config.json bank / feeSplitter             -> the bank page
EOF
        remind_ledger
    fi
    ;;

preflight)
    # The contracts gate is real; the LEDGER that feeds them had no gate at all.
    # Everything checked here was actually wrong in this repo at some point in
    # the week before launch (see contracts/preflight.py).
    need python3
    say "off-chain gate"
    python3 preflight.py || die "preflight is red — the ledger that feeds the chain is not ready"
    ;;

pools)
    # THE phase that makes it a DeFi game: three canonical Uniswap v3 pools.
    # Sizes/ratios are LAUNCH-DECISIONS.md's table, not this script's opinion.
    need forge; need python3; need cast
    pools_armed="$(armed "${1:-}")"
    OBOL=$(town_get OBOL_TOKEN); MYRRH=$(town_get MYRRH_TOKEN)
    [ -n "$OBOL" ] && [ -n "$MYRRH" ] || die "need spoils first (town.env lacks OBOL_TOKEN / MYRRH_TOKEN)"
    [ -n "${NPM:-}" ] && [ -n "${V3_FACTORY:-}" ] || die "set NPM and V3_FACTORY (verified by 'check')"
    python3 script/seed_spoils_uniswap.py --selftest || die "seed math selftest red"

    # The opening RATIOS are $SCRY-denominated policy (LAUNCH-DECISIONS.md), so
    # they do not move with the $SCRY/USD price and this phase can compute the
    # whole seed deterministically instead of asking you to paste numbers.
    # SIZE is the knob, and it is now TWO knobs, one per $SCRY pool.
    #
    # It used to be one. `POOL_SCRY_BUDGET` (default 40000000) was passed to BOTH
    # --obol-scry-budget and --myrrh-scry-budget, so an armed run seeded 40M/40M
    # - the split that the operator REVISED to 60M/20M on 2026-07-26. The decision
    # doc said "the seed script takes --myrrh-scry-budget as an argument, so no
    # code changes", which was true of the helper and false of this wrapper, and
    # this wrapper is what actually broadcasts. The pools would have opened at the
    # superseded split, out of the operator's own $SCRY, with every doc green.
    # `preflight` now reads these defaults back against the decision table.
    SCRY_PER_OBOL="${SCRY_PER_OBOL:-10}"
    SCRY_PER_MYRRH="${SCRY_PER_MYRRH:-50}"
    POOL_SCRY_BUDGET_OBOL="${POOL_SCRY_BUDGET_OBOL:-76500000}"
    POOL_SCRY_BUDGET_MYRRH="${POOL_SCRY_BUDGET_MYRRH:-23500000}"
    # The canary knob: a TOTAL, split on the decided ratio so a small rehearsal
    # rehearses the real shape rather than a different one. Setting either
    # per-pool knob above wins over it.
    if [ -n "${POOL_SCRY_BUDGET:-}" ]; then
        read -r POOL_SCRY_BUDGET_OBOL POOL_SCRY_BUDGET_MYRRH <<EOSPLIT
$(python3 -c 'import sys
tot, o, m = (float(x) for x in sys.argv[1:4])
print(f"{tot * o / (o + m):.0f} {tot * m / (o + m):.0f}")' \
  "$POOL_SCRY_BUDGET" "${POOL_SCRY_BUDGET_OBOL}" "${POOL_SCRY_BUDGET_MYRRH}")
EOSPLIT
        echo "   POOL_SCRY_BUDGET=$POOL_SCRY_BUDGET split on the decided ratio ->" \
             "OBOL $POOL_SCRY_BUDGET_OBOL / MYRRH $POOL_SCRY_BUDGET_MYRRH"
    fi
    POOL_OBOL_BUDGET="${POOL_OBOL_BUDGET:-10000000}"   # the third pool's OBOL side
    # The third pool's rate is DERIVED, never defaulted (security review
    # 2026-07-25): it is the cross of the other two, so if either ratio knob
    # moves and this stays a literal 5 the launch ships with the exact genesis
    # arb the third pool exists to foreclose - paid out of the LP NFT this same
    # broadcast mints. The helper re-checks the cross and fails closed.
    OBOL_PER_MYRRH=$(python3 -c 'import sys; print(float(sys.argv[1]) / float(sys.argv[2]))' \
                     "$SCRY_PER_MYRRH" "$SCRY_PER_OBOL")

    # Before anything is DERIVED from them, and long before anything is sent.
    ratios_locked

    cat <<EON
   Three pools, from LAUNCH-DECISIONS.md (sizes here, ratios are policy):
     OBOL/\$SCRY    ${POOL_SCRY_BUDGET_OBOL} \$SCRY  @ ${SCRY_PER_OBOL} \$SCRY/OBOL
     MYRRH/\$SCRY   ${POOL_SCRY_BUDGET_MYRRH} \$SCRY  @ ${SCRY_PER_MYRRH} \$SCRY/MYRRH
     MYRRH/OBOL    ${POOL_OBOL_BUDGET} OBOL   @ ${OBOL_PER_MYRRH} OBOL/MYRRH   (costs no \$SCRY)

   WHY WE OPEN THE THIRD ONE: not because we want a third market - because we
   cannot stop one existing. Uniswap is permissionless, and once OBOL and MYRRH
   both trade against \$SCRY those prices IMPLY a MYRRH/OBOL rate that pays
   whoever opens the pool first. A v3 pool's first mint IS its price: a stranger
   opening it off-cross takes the difference out of the two positions you are
   about to mint. Opening it yourself at ${SCRY_PER_MYRRH}/${SCRY_PER_OBOL} =
   ${OBOL_PER_MYRRH} forecloses that, for free.

   CANARY FIRST. Seed small, read back ordering/price/recipient, THEN top up:
     POOL_SCRY_BUDGET=100000 POOL_OBOL_BUDGET=10000 ./deploy_town.sh pools --arm
   That is 0.1% of the 100,000,000 total, split on the same 76.5/23.5 the full
   seed uses, so the rehearsal rehearses the real shape.
   Re-running with the full size ADDS liquidity to the pools you just made.
EON

    # THE GATE RUNS BEFORE THE SEED ENV EXISTS, and the order is the fix.
    # `SeedSpoilsUniswapV3.s.sol` reads its inputs from the environment, and so
    # does one of its tests (`test_run_reads_env_roundtrip`). This phase exports
    # OBOL_SQRTP/MYRRH_SQRTP/… below and `forge test` inherits them, so the
    # suite was graded against a half-populated env it never sees in any other
    # run: MYRRH_SQRTP set, MYRRH_TOKEN absent, and the script's own
    # `require(myrrhToken != address(0), "MYRRH_SQRTP set but MYRRH_TOKEN is
    # not")` fires. `pools --arm` therefore failed its OWN pre-broadcast gate,
    # every time, with a message that reads like a contract bug rather than an
    # env leak - and the obvious move at 3am is to skip the gate. Found by a
    # fork rehearsal 2026-07-29 (invisible to a dry run: nothing arms, so
    # tests_green never runs); reproduced as
    # `MYRRH_SQRTP=79228162514264337593543950336 forge test --match-test
    # test_run_reads_env_roundtrip` -> red, and green with the var unset.
    # Proving the suite first leaves the memo set, so run_script's later call is
    # the no-op and nothing broadcasts on an unproven suite.
    [ "$pools_armed" = yes ] && tests_green

    say "computing sqrtP + raw amounts (deterministic from the posted ratios)"
    SEED_ENV=$(python3 script/seed_spoils_uniswap.py \
        --scry "$SCRY_CANON" \
        --obol  "$OBOL"  --obol-price  "$SCRY_PER_OBOL"  --obol-scry-budget  "$POOL_SCRY_BUDGET_OBOL" \
        --myrrh "$MYRRH" --myrrh-price "$SCRY_PER_MYRRH" --myrrh-scry-budget "$POOL_SCRY_BUDGET_MYRRH" \
        --myrrh-obol-price "$OBOL_PER_MYRRH" --myrrh-obol-budget "$POOL_OBOL_BUDGET") \
        || die "seed helper failed - fix the inputs before anything broadcasts"
    echo "$SEED_ENV"
    # shellcheck disable=SC2046
    eval "$(echo "$SEED_ENV" | grep '^export ')"
    export NPM V3_FACTORY
    export V3_NPM="$NPM"

    # Read the pools BEFORE the mints, so an abort costs nothing. On the first
    # run this just records "does not exist yet"; on the top-up it is the check
    # that catches a price someone moved while the canary sat there live.
    [ "$pools_armed" = yes ] && pools_read_before

    # ── THE COIN SIDES HAVE TO EXIST BEFORE THEY CAN BE POOLED ───────────────
    # Nothing else in the launch minted them, and every doc assumed something
    # else did. `DeploySpoils` mints only the Garden's opening seed (which goes
    # straight INTO the Garden in the same transaction) and the optional powder
    # (which goes to a different wallet); `SeedSpoilsUniswapV3` only approves
    # and calls NPM.mint. So the 17,650,000 OBOL / 2,470,000 MYRRH this phase
    # deposits had no source at all: an armed run died in forge's simulation
    # with an opaque revert from inside the NPM multicall, AFTER `spoils` had
    # broadcast the tokens. The fork mandate could never catch it, because
    # SeedSpoilsFork.t.sol mints and `deal`s its own balances - which is the
    # right thing for a test of the seeding mechanics and a blind spot for the
    # question "does the depositor actually hold this".
    #
    # Minted HERE rather than in DeploySpoils, and only the SHORTFALL:
    #   - what is minted always equals what enters a pool, so there is no
    #     unposted float sitting in a wallet afterwards (the powder is the one
    #     deliberate exception and it is posted as one);
    #   - the canary -> read-back -> top-up flow works, because the second run
    #     mints only the remainder;
    #   - re-running against already-seeded pools mints nothing.
    NEED_OBOL=$(add_raw "${OBOL_BASE_RAW:-0}" "${MYRRH_OBOL_QUOTE_RAW:-0}")
    NEED_MYRRH=$(add_raw "${MYRRH_BASE_RAW:-0}" "${MYRRH_OBOL_BASE_RAW:-0}")
    NEED_SCRY=$(add_raw "${OBOL_QUOTE_RAW:-0}" "${MYRRH_QUOTE_RAW:-0}")

    say "what this seed spends, and what has to exist before it can"
    echo "   \$SCRY  $(fmt_units "$NEED_SCRY")  - YOURS, transferred. This phase never mints \$SCRY"
    echo "   OBOL   $(fmt_units "$NEED_OBOL")  - house-minted below, exactly what enters the pools"
    echo "   MYRRH  $(fmt_units "$NEED_MYRRH")  - same"

    # POSTED, NOT INHERITED. `SeedSpoilsUniswapV3.s.sol` defaults the recipient
    # to `vm.addr(pk)`, so without this the deed to 100,000,000 SCRY of depth
    # lands on the deployer — the one key in this whole file that exists to be
    # used once and retired. `snapshots/house-wallets.json` has said "the powder
    # AND the LP NFT are aimed at this wallet" since 2026-07-27; it was never
    # wired, and a documented intention that no code reads is a wish. Operator
    # confirmed the class 2026-07-29. Exported, because a value this phase
    # validated and did not export is the HARVEST_TOKEN defect again (07-27).
    # Resolved HERE, above the print below, so that line can name the real
    # recipient instead of guessing at it.
    if [ -z "${LP_RECIPIENT:-}" ]; then
        LP_RECIPIENT="$DEV_WALLET"
        printf '   LP_RECIPIENT unset -> the posted dev wallet %s\n' "$LP_RECIPIENT"
        printf '     (the LP position NFT is the DEED to this depth; not the deployer)\n'
    fi
    export LP_RECIPIENT

    DEPOSITOR=""
    [ -n "${PRIVATE_KEY:-}" ] && DEPOSITOR=$(cast wallet address --private-key "$PRIVATE_KEY" 2>/dev/null || true)
    if [ -z "$DEPOSITOR" ]; then
        echo "   (no PRIVATE_KEY, so no balances read - the armed run proves all three)"
    else
        echo "   depositor    $DEPOSITOR   (spends the SCRY, mints the coin sides)"
        echo "   LP deed ->   $LP_RECIPIENT"
        SHORT_SCRY=$(sub_floor0 "$NEED_SCRY" "$(erc20_bal "$SCRY_CANON" "$DEPOSITOR")")
        if [ "$SHORT_SCRY" != 0 ]; then
            echo "   !! \$SCRY short by $(fmt_units "$SHORT_SCRY") - and \$SCRY cannot be minted, it is bought"
            [ "$pools_armed" = yes ] && die "$DEPOSITOR does not hold the \$SCRY this seed spends"
        else
            echo "   ok \$SCRY covered"
        fi

        # A mint is a broadcast, so it takes the same gate as any other. Already
        # proven at the top of the phase (before the seed env is exported, which
        # is why it moved); this and run_script's call are both the memoized
        # no-op. Kept rather than deleted: the rule is "green before every
        # broadcast", and the call site is where that rule is legible. If the
        # memo is ever removed, this still does the right thing.
        [ "$pools_armed" = yes ] && tests_green

        for triple in "OBOL:$OBOL:$NEED_OBOL" "MYRRH:$MYRRH:$NEED_MYRRH"; do
            sym="${triple%%:*}"; rest="${triple#*:}"; addr="${rest%%:*}"; need="${rest#*:}"
            short=$(sub_floor0 "$need" "$(erc20_bal "$addr" "$DEPOSITOR")")
            if [ "$short" = 0 ]; then
                echo "   ok $sym already held - nothing to mint"
                continue
            fi
            if [ "$pools_armed" != yes ]; then
                echo "   $sym would mint $(fmt_units "$short") to the depositor (the armed run does this)"
                continue
            fi
            m=$(cast call "$addr" 'minter()(address)' --rpc-url "$RPC")
            [ "$(lc "$m")" = "$(lc "$DEPOSITOR")" ] || die \
                "$sym's minter is $m, not the depositor $DEPOSITOR.
   Only the minter may mint, and \`rotate\` is one-way - if it has already run,
   the seed has to be signed by whoever holds the role now."
            say "minting $sym $(fmt_units "$short") -> $DEPOSITOR (this is exactly what the pools receive)"
            cast send "$addr" 'mint(address,uint256)' "$DEPOSITOR" "$short" \
                --rpc-url "$RPC" --private-key "$PRIVATE_KEY" >/dev/null \
                || die "$sym mint reverted - are you still the minter?"
            echo "   $sym balance now $(fmt_units "$(erc20_bal "$addr" "$DEPOSITOR")") (read back off the chain)"
        done
    fi

    # The deed to everything about to be deposited. Counted before, so the
    # read-back can prove a position actually landed rather than trusting that
    # the seeder's silence meant success.
    LP_TO=""; LP_BEFORE=0
    if [ "$pools_armed" = yes ] && [ -n "${NPM:-}" ]; then
        LP_TO="$LP_RECIPIENT"
        [ -n "$LP_TO" ] && LP_BEFORE=$(erc20_bal "$NPM" "$LP_TO")
    fi

    run_script script/SeedSpoilsUniswapV3.s.sol "$pools_armed"
    if [ "$pools_armed" = yes ]; then
        pools_read_after "$LP_TO" "$LP_BEFORE"
        cat <<'EON'

   AND THE SITE STAYS DARK UNTIL THE METER IS TOLD. Three separate variables,
   two of which are named in no other document:
     SCRY_SPOILS_ADDRS={"OBOL":"0x…","MYRRH":"0x…"}   -> /api/tokens, play.html step 5
     SCRY_OBOL_TOKEN=0x…                              -> /api/pools reads this
     SCRY_MYRRH_TOKEN=0x…                             -> and this
   Without the last two, all three pools you just opened keep reporting
   `deployed: false` on the one page this audience actually checks. They live
   in /data/apps/scry-deploy/meter/ecosystem.config.js (untracked - it is not
   in the repo), and `pm2 restart --update-env` does NOT re-read that file:
   restart from it with --only, then confirm at /proc/<pid>/environ.
EON
        remind_ledger
    fi
    ;;

harvest)
    # Closes the loop: the off-chain ledger already produces proofs in
    # ScryHarvest's exact merkle dialect (meter test_fun_layer [bridge]).
    # This is what makes them redeemable.
    need forge; need python3
    OBOL=$(town_get OBOL_TOKEN)
    [ -n "$OBOL" ] || die "need spoils first (town.env lacks OBOL_TOKEN)"
    # DeployHarvest reads OBOL_TOKEN - that is its documented interface (point
    # it at MYRRH for a MYRRH harvest). This exported HARVEST_TOKEN until
    # 2026-07-27, a name nothing reads, so an armed `harvest` died inside forge
    # on the missing variable - AFTER `pools`, with the coins live and the whole
    # gate green over it. The dry run cannot see this phase at all (no town.env,
    # refused earlier, by design); only the fork rehearsal caught it.
    export OBOL_TOKEN="$OBOL"
    run_script script/DeployHarvest.s.sol "$(armed "${1:-}")"
    if [ "$(armed "${1:-}")" = yes ]; then
        creates_from_broadcast DeployHarvest.s.sol | while read -r name addr; do
            case "$name" in ScryHarvest) town_set HARVEST "$addr" ;; esac
        done
        cat <<'EON'

   Harvest is deployed but NOT funded. The claim only works once you:
     1. GET /api/tokens/claim-root?token=OBOL      -> root + total_committed
     2. mint total_committed OBOL to the harvest contract - as the deployer
        while you still hold OBOL's minter slot; after the `granary` phase or
        `rotate`, via the OBOL granary's stewardMint(harvest, ...) instead
     3. harvest.postRoot(root, total_committed, ledgerRef)
   Players then GET /api/tokens/claim-proof and claim self-serve.
   Roots are REPUBLISHABLE: nobody forfeits by claiming late.
EON
        remind_ledger
    fi
    ;;

launch)
    # The whole launch, in the only order that works. Everything after this is
    # depth (gardener/silo/shrine/orchard) and blocks nothing.
    armed_flag="$(armed "${1:-}")"
    say "LAUNCH: preflight -> check -> test -> spoils -> pools -> harvest"
    [ "$armed_flag" = yes ] || echo "   (dry run — add --arm to broadcast each phase)"

    # EVERYTHING THE LATER PHASES NEED IS DEMANDED HERE, BEFORE THE FIRST SEND.
    # `check` treats NPM/V3_FACTORY as optional ("fine until the orchard
    # phase") and `pools` refuses without them - so an armed launch used to
    # deploy OBOL and MYRRH, then die on an unset variable with the coins
    # already welded to the chain. A precondition that fires after an
    # irreversible act is not a precondition.
    if [ "$armed_flag" = yes ]; then
        [ -n "${PRIVATE_KEY:-}" ] || die "--arm needs PRIVATE_KEY in the env (the deployer, and the wallet the LP NFT lands in)"
        [ -n "${NPM:-}" ] && [ -n "${V3_FACTORY:-}" ] || die \
            "export NPM and V3_FACTORY before an armed launch.
   \`pools\` refuses without them, and by then \`spoils\`'s tokens are
   already broadcast. POOLS.md 4.1 carries the pair; \`check\` verifies it.
     export NPM=0x73991a25C818Bf1f1128dEAaB1492D45638DE0D3
     export V3_FACTORY=0x1f7d7550B1b028f7571E69A784071F0205FD2EfA"

        # AND THE $SCRY, WHICH IS THE ONE INPUT THAT CANNOT BE MINTED OR
        # RETRIED. The paragraph above was written about unset variables and
        # stopped there; the balance is the same defect with a slower fuse.
        # `NEED_SCRY` is computed inside `pools`, so an armed launch with an
        # unfunded deployer ran preflight/check/test green, BROADCAST OBOL,
        # MYRRH and the Garden, and only then died on "does not hold the $SCRY
        # this seed spends" - three welded contracts and no pools.
        #
        # It demands the FULL size, not the canary's 0.1%, and that is the
        # point: `launch --arm` spends only the canary, but it leaves three
        # LIVE PUBLIC POOLS at a few dollars of depth that the first sitting says to
        # finish in "minutes, not hours". Passing this gate on canary money and
        # hitting the wall at the top-up is worse than dying at `spoils` - the
        # pools are already open and anyone with pennies of gas can move one,
        # after which the top-up aborts on the drift guard. You do not begin a
        # launch you cannot finish.
        #
        # Read from the same defaults `pools` resolves (:1079), never retyped.
        launch_need=$(python3 -c 'import sys; print(int(sys.argv[1]) + int(sys.argv[2]))' \
            "${POOL_SCRY_BUDGET_OBOL:-76500000}" "${POOL_SCRY_BUDGET_MYRRH:-23500000}")
        launch_who=$(cast wallet address --private-key "$PRIVATE_KEY" 2>/dev/null) \
            || die "PRIVATE_KEY is set but is not a usable key - cast could not derive an address from it"
        launch_held=$(erc20_bal "$SCRY_CANON" "$launch_who")
        launch_short=$(sub_floor0 "$(python3 -c 'import sys; print(int(sys.argv[1]) * 10**18)' "$launch_need")" "$launch_held")
        if [ "$launch_short" != 0 ]; then
            die "the deploy wallet does not hold the \$SCRY this launch seeds.

   deployer  $launch_who
   holds     $(fmt_units "$launch_held") \$SCRY
   needs     $(fmt_units "$(python3 -c 'import sys; print(int(sys.argv[1]) * 10**18)' "$launch_need")") \$SCRY
   short     $(fmt_units "$launch_short") \$SCRY

   \$SCRY is TRANSFERRED, never minted - no phase here can make it. Move it in
   before you arm. The deploy wallet holds 0 the rest of the time on purpose
   (the first sitting seeds spoils only), so this is the expected state, not a fault.

   This gate demands the full seed even though an armed launch spends only the
   canary: it stops you opening three public pools you cannot then top up."
        fi
        printf '   ok $SCRY covered - %s held, %s needed\n' \
            "$(fmt_units "$launch_held")" \
            "$(fmt_units "$(python3 -c 'import sys; print(int(sys.argv[1]) * 10**18)' "$launch_need")")"
    fi

    ./deploy_town.sh preflight
    ./deploy_town.sh check
    ./deploy_town.sh test
    ./deploy_town.sh spoils  ${1:-}
    if [ "$armed_flag" = yes ]; then
        if [ "${LAUNCH_SKIP_CANARY:-0}" = 1 ]; then
            echo "
   LAUNCH_SKIP_CANARY=1 - seeding the pools at FULL SIZE in one act, with no
   read-back in between. You are choosing this."
            ./deploy_town.sh pools   --arm
            ./deploy_town.sh harvest --arm
            say "launch phases done — ./deploy_town.sh status"
        else
            # THE CANARY IS THE LAUNCH'S DEFAULT, because a v3 pool's first
            # mint IS its price and this is the one step the script cannot undo
            # for you. LAUNCH.md 4 has said "canary first, always" since it was
            # written; `launch --arm` went straight to the full 60M/20M anyway,
            # which is the doc of record and the driver disagreeing about the
            # only number that cannot be corrected afterwards.
            POOL_SCRY_BUDGET=100000 POOL_OBOL_BUDGET=10000 ./deploy_town.sh pools --arm
            cat <<'EON'

== the canary is in. STOPPING HERE ON PURPOSE.

   Three pools now exist at 0.1% of full size. Their price is set; the
   remaining 99.9% is just depth.

   THE MACHINE HAS ALREADY CHECKED WHAT A MACHINE CAN. `pools --arm` read all
   three pools back off the chain: each one exists at the 1% tier, each sits at
   exactly the sqrtPriceX96 the seed helper computed, and the LP position NFT
   landed in your wallet. If any of that had disagreed you would not be reading
   this - it aborts. You do not need to run those cast calls by hand; the
   script has both sides of that comparison and does it itself.

   THE RATIOS ARE ALSO CHECKED NOW, and they did not used to be. `ratios_locked`
   compared the three RESOLVED prices - the actual numbers that went into sqrtP,
   not this file's text - against the table in LAUNCH-DECISIONS.md, and refused
   to run on a mismatch. It also re-derived the cross (SCRY/MYRRH over
   SCRY/OBOL) and proved it equals the posted OBOL/MYRRH, so the third pool
   cannot open with the genesis arb it exists to foreclose.

   That closes the hole this stop was written for. The old warning here said
   "only the operator can say the script matches what he decided" - true when
   nothing read the decision. LAUNCH-DECISIONS posts the three numbers in a
   table, so a machine reads it now. The receipts behind that caution were real
   (POOL_SCRY_BUDGET once fed both pools from one knob and would have opened
   them 40M/40M, a split revised the day before, with every check green) and
   they are what the gate is made of.

   WHAT NO ASSERTION STILL REACHES is one step further back:

     IS THE POSTED TABLE WHAT YOU DECIDED?  Everything above proves the chain
     matches the table. Nothing can prove the table matches your intent - it is
     a document, and you wrote it. If 10 / 50 / 5 is still the answer, you are
     done here; SENTENCES.md is where those three were spoken.

   Also worth one look while you are here: your balances fell by what you
   approved and no more.

   When the ratios are right, finish the launch:
     ./deploy_town.sh pools   --arm     # tops up to full size, mints only the remainder
     ./deploy_town.sh harvest --arm
     ./deploy_town.sh verify  --arm     # Blockscout source, before anyone reads the addresses
     ./deploy_town.sh status
EON
        fi
    else
        cat <<'EON'

== phases 4-5 (pools, harvest) cannot be dry-run from here yet

   They need OBOL/MYRRH addresses, and a dry run deliberately broadcasts
   nothing — so town.env has none to give them. That is the gate working,
   not a failure: nothing downstream can pretend to have upstream addresses.

   After `spoils --arm` records the real addresses:
     ./deploy_town.sh pools            # dry run, now that town.env is populated
     ./deploy_town.sh harvest

   Or run the whole thing for real:
     ./deploy_town.sh launch --arm

   An armed launch seeds the pools at 0.1% and STOPS, so you read the opening
   price off the chain before the other 99.9% goes in. It also mints the coin
   sides it deposits (17,650,000 OBOL / 2,470,000 MYRRH at full size) - nothing
   else in the launch does, and the amounts minted are exactly the amounts
   pooled. Set LAUNCH_SKIP_CANARY=1 to seed full size in one act instead.

== dry run complete: preflight, check, test, and the spoils simulation are green
EON
    fi
    ;;

rotate)
    # The last launch act, and the only IRREVERSIBLE one in this file.
    #
    # The `spoils` phase deploys OBOL and MYRRH with minter = the deployer
    # key, which is what lets you seed pools, fund claim contracts and fund
    # harvest. That key
    # is UNLIMITED MINT while it holds the role. LAUNCH.md called rotating it
    # "a launch step and not a someday" - and then no phase did it and no gate
    # checked it, so the launch's own closing act existed only as prose.
    #
    # setMinter is onlyMinter, so this is one-way: after it runs, the deployer
    # cannot take the role back and cannot hand it anywhere else. Everything
    # that needs minting must already be done.
    need cast
    OBOL=$(town_get OBOL_TOKEN); MYRRH=$(town_get MYRRH_TOKEN)
    [ -n "$OBOL" ] || die "need spoils first (town.env lacks OBOL_TOKEN)"
    rot_armed="$(armed "${1:-}")"

    # Per-token targets, because the two coins retire to different places.
    # MYRRH's minter is what the FARM needs later (DeployGardener calls
    # reward.setMinter(granary) with reward = MYRRH since 2026-07-26, and only
    # the current minter may), so pointing MYRRH at an offline key means that
    # key performs the handoff, not you. This said OBOL until 07-26; the farm's
    # coin has moved twice, so re-read FARMING.md 3a before trusting either.
    NEXT_OBOL="${MINTER_NEXT_OBOL:-${MINTER_NEXT:-}}"
    NEXT_MYRRH="${MINTER_NEXT_MYRRH:-${MINTER_NEXT:-}}"
    [ -n "$NEXT_OBOL$NEXT_MYRRH" ] || die \
        "set MINTER_NEXT (both coins) or MINTER_NEXT_OBOL / MINTER_NEXT_MYRRH.
   Leave one unset to rotate only the other - the two retire to different
   places, and MYRRH's slot is the one the farm still needs.

   NO DEFAULT HERE, DELIBERATELY. Every other posted address in this file falls
   back to \$DEV_WALLET when unset; this one does not, because setMinter is
   ONE-WAY and a destination that arrives by default is a destination nobody
   read. The posted answer is the dev wallet - paste it, do not infer it:
     MINTER_NEXT=$DEV_WALLET ./deploy_town.sh rotate --arm"

    say "what the chain says the minters are right now"
    for pair in "OBOL:$OBOL" "MYRRH:$MYRRH"; do
        sym="${pair%%:*}"; addr="${pair#*:}"
        [ -n "$addr" ] || continue
        # Tolerant on purpose: this is the ORIENTATION print, and an address
        # that cannot answer `minter()` must reach the precondition block below
        # as a refusal with a reason, not kill the run from inside an echo.
        echo "   $sym  $addr  minter -> $(cast call "$addr" 'minter()(address)' --rpc-url "$RPC" 2>/dev/null || echo '(unreadable — not a SpoilsToken?)')"
    done

    say "the things that must already be done, because this is one-way"
    warn=0
    powder_door rotate "$rot_armed"

    # (0) THE SEAT MUST STILL BE OURS, and this is the ONE check here that
    # refuses instead of warning, because it is the one that can be PROVEN
    # impossible rather than merely unproven. `gardener` and `granary` move
    # SpoilsToken.minter into a granary on their way past
    # (DeployGardener.s.sol:175, DeployGranary.s.sol:54), so on a depth-first
    # launch this phase is guaranteed to revert - and it used to find that out
    # from `cast send`, at the end of the sitting, as "setMinter reverted - are
    # you still the minter?", which reads as a broken key rather than as the
    # right thing having already happened. The seat that still holds unlimited
    # mint in that case is ScryGranary.steward, and the phase is `steward`.
    #
    # ROTATE_FORCE does not open this. It exists for preconditions that cannot
    # be proven FROM HERE (an offline ledger, a drop that will never exist);
    # a minter() that reads back as somebody else is proof, not absence of it.
    ME_ROT=""
    [ -n "${PRIVATE_KEY:-}" ] && \
        ME_ROT=$(cast wallet address --private-key "$PRIVATE_KEY" 2>/dev/null || echo "")
    for pair in "OBOL:$OBOL:$NEXT_OBOL:$(town_get GRANARY_OBOL)" \
                "MYRRH:$MYRRH:$NEXT_MYRRH:$(town_get GRANARY_MYRRH)"; do
        sym="${pair%%:*}"; rest="${pair#*:}"; addr="${rest%%:*}"; rest="${rest#*:}"
        next="${rest%%:*}"; gr="${rest#*:}"
        # Only judge a coin this run would actually touch.
        [ -n "$addr" ] && [ -n "$next" ] || continue
        m=$(cast call "$addr" 'minter()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
        if [ -z "$m" ]; then
            echo "   ?? $sym: minter() unreadable - cannot prove the role is still ours"
            warn=$((warn + 1))
        elif [ -z "$ME_ROT" ]; then
            echo "   ?? $sym minter is $m - PRIVATE_KEY unset, cannot confirm it is us"
            warn=$((warn + 1))
        elif [ "$(lc "$m")" = "$(lc "$ME_ROT")" ]; then
            echo "   ok $sym.minter is us ($m) - there is a role here to retire"
        elif [ -n "$gr" ] && [ "$(lc "$m")" = "$(lc "$gr")" ]; then
            die "$sym.minter is already the granary $gr, not you.

   \`gardener\` / \`granary\` took that slot on their way past, which is the
   BETTER destination - a rate-limited contract rather than another hot key,
   and \`minter()\` reads as a contract to the rug-screen. Nothing is wrong.

   But it did not retire unlimited mint: ScryGranary.stewardMint() is uncapped
   and the granary's steward is still you. The closing act on this ordering is
   the OTHER phase:

     STEWARD_NEXT=0x... ./deploy_town.sh steward          # dry run first
     STEWARD_NEXT=0x... ./deploy_town.sh steward --arm

   \`rotate\` would revert on \`minter only\` here, and ROTATE_FORCE=1 will not
   change that - the role is provably not ours."
        else
            die "$sym.minter is $m, not you ($ME_ROT).

   Whoever holds that address holds the mint; \`setMinter\` is onlyMinter, so
   an armed run reverts. If it is a granary this town does not have recorded,
   check town.env against the chain before going further."
        fi
    done

    if [ -n "${V3_FACTORY:-}" ] && [ -n "$MYRRH" ]; then
        for pair in "OBOL/\$SCRY:$OBOL:$SCRY_CANON" "MYRRH/\$SCRY:$MYRRH:$SCRY_CANON" \
                    "MYRRH/OBOL:$MYRRH:$OBOL"; do
            lbl="${pair%%:*}"; rest="${pair#*:}"; a="${rest%%:*}"; b="${rest#*:}"
            got=$(cast call "$V3_FACTORY" 'getPool(address,address,uint24)(address)' \
                  "$a" "$b" 10000 --rpc-url "$RPC" 2>/dev/null || echo "")
            case "$got" in
            0x0000000000000000000000000000000000000000|"")
                echo "   !! $lbl pool does NOT exist - seed the pools BEFORE rotating"; warn=$((warn + 1)) ;;
            *) echo "   ok $lbl pool $got" ;;
            esac
        done
    else
        echo "   ?? V3_FACTORY unset - cannot verify the pools were seeded"; warn=$((warn + 1))
    fi
    HARVEST_A=$(town_get HARVEST)
    if [ -n "$HARVEST_A" ]; then
        bal=$(cast call "$OBOL" 'balanceOf(address)(uint256)' "$HARVEST_A" --rpc-url "$RPC")
        bal=${bal%% *}
        # AGAINST WHAT IT OWES, not against zero. This asked only `> 0`, so ONE
        # WEI printed "funded" and cleared the last gate before an irreversible
        # act. Dust from a prior root, a partial funding cast, or a stewardMint
        # that clamped all pass a nonzero test. The contract publishes the
        # number to compare against — `totalCommitted()` — and the driver had
        # the ABI to ask and did not. An underfunded harvest against a PUBLIC
        # root pays the early claimants and reverts on an empty balance for the
        # rest, after the proofs are out, and the only party who can top it up
        # is whoever MINTER_NEXT just named.
        owed=$(cast call "$HARVEST_A" 'totalCommitted()(uint256)' --rpc-url "$RPC" 2>/dev/null || echo "")
        owed=${owed%% *}
        if [ -z "$owed" ]; then
            echo "   ?? harvest $HARVEST_A: could not read totalCommitted() - is it a ScryHarvest?"; warn=$((warn + 1))
        elif [ "$owed" = 0 ]; then
            case "$bal" in
            0) echo "   ok harvest $HARVEST_A has no root posted yet and holds nothing (consistent)" ;;
            *) echo "   ok harvest $HARVEST_A holds $bal with no root posted yet" ;;
            esac
        elif python3 -c "import sys; sys.exit(0 if int(sys.argv[1]) >= int(sys.argv[2]) else 1)" "$bal" "$owed"; then
            echo "   ok harvest $HARVEST_A funded ($bal >= totalCommitted $owed)"
        else
            echo "   !! harvest $HARVEST_A UNDERFUNDED: holds $bal, its posted root commits $owed"
            echo "      claims will revert once the balance runs out, and after this rotate"
            echo "      only the new minter can top it up"
            warn=$((warn + 1))
        fi
    else
        echo "   ?? no HARVEST in town.env - if a claim is planned, deploy+fund it first"; warn=$((warn + 1))
    fi
    if [ -f deployments.json ]; then
        n=$(python3 -c 'import json,sys
try: print(len(json.load(open("deployments.json")).get("chains",{}).get("4663",{}).get("claims",{}) or {}))
except Exception: print(0)')
        [ "$n" = 0 ] && { echo "   ?? deployments.json records no per-drop claim contracts - if a drop is planned, deploy+fund it first"; warn=$((warn + 1)); } \
                     || echo "   ok $n drop(s) have claim contracts recorded"
    fi

    # AND THE DESTINATION ITSELF. Every check above asks whether we are READY to
    # rotate; none of them asked WHERE TO, which is the only field here that is
    # typed by a human into a one-way call. `SpoilsToken.setMinter` rejects
    # address(0) and accepts literally anything else, so a mistyped address is
    # accepted, welded, and unrecoverable: the role can only move again from the
    # new holder, and MYRRH's only source is the Garden, which needs
    # setMinter(granary) FROM THE CURRENT MINTER. There is no admin door - that
    # is the design, not an oversight - so this is the last place it can be
    # caught.
    #
    # The tell for a fat-finger is cheap and strong: an address that has never
    # sent a transaction AND holds no code has never been used by anybody. A key
    # you actually hold has a nonce; a granary is a contract. Neither is a proof
    # of custody, and this does not pretend to be one - it is the typo filter,
    # and it fails into the same `warn` refusal as everything else here, so
    # ROTATE_FORCE=1 still covers the honest case of a fresh cold wallet.
    # THE COIN TRAVELS IN THE PAIR, like every other loop in this phase. It used
    # to read `$addr` here, which this loop never sets — it was the leftover from
    # the ME_ROT precondition loop above, whose last iteration is MYRRH. So the
    # `spoils()` comparison asked, for BOTH coins, "does this granary bind
    # MYRRH?", and the OBOL arm was inverted in both directions at once: the
    # correct OBOL granary was refused as "a granary for a DIFFERENT coin"
    # (printing OBOL's own address as the different one), and pointing OBOL at
    # the MYRRH granary — the exact two-granary footgun the check was written
    # for — printed `ok ... a ScryGranary binding OBOL`, contributed no warn, and
    # would have broadcast. Proven on an RH-Chain fork, 2026-07-29, with two real
    # granaries; found by review of the 07-28 audit fixes.
    for pair in "OBOL:$OBOL:$NEXT_OBOL" "MYRRH:$MYRRH:$NEXT_MYRRH"; do
        sym="${pair%%:*}"; rest="${pair#*:}"; coin="${rest%%:*}"; next="${rest#*:}"
        [ -n "$coin" ] && [ -n "$next" ] || continue
        n_nonce=$(cast nonce "$next" --rpc-url "$RPC" 2>/dev/null || echo "")
        n_code=$(cast code "$next" --rpc-url "$RPC" 2>/dev/null || echo "0x")
        if [ -z "$n_nonce" ]; then
            echo "   ?? $sym -> $next: the chain would not answer nonce() - cannot vet the destination"
            warn=$((warn + 1))
        elif [ "$n_nonce" = 0 ] && [ "$n_code" = "0x" ]; then
            echo "   !! $sym -> $next has NEVER sent a tx and holds no code."
            echo "      If that is a typo, the mint role is gone for good. If it is a"
            echo "      fresh cold wallet, send one tx from it first and re-run."
            warn=$((warn + 1))
        else
            # An EIP-7702 account is an EOA WITH A DELEGATE, not a contract, and
            # calling it one here would be wrong in the direction that matters:
            # the operator's own wallets are 7702 on this chain (three of them
            # are, per snapshots/house-wallets.json, and the holder snapshot has
            # a reclassification pass for exactly this). The tell is the 23-byte
            # 0xef0100 indicator.
            case "$n_code" in
            0xef0100*)
                echo "   ok $sym -> $next (EIP-7702 EOA, delegate 0x${n_code#0xef0100} - nonce $n_nonce)" ;;
            0x)
                echo "   ok $sym -> $next (nonce $n_nonce - this key has signed before)" ;;
            *)
                # A CONTRACT DESTINATION IS NOT SELF-EVIDENTLY THE GRANARY PATH,
                # and this arm blessed every one of them. The lethal case is
                # written down in a doc marked `status: live`: SPOILS-ECONOMY.md's
                # arming runbook said `SpoilsToken.setMinter(claimsContract)`
                # where the claims contract is a ScryHarvest. ScryHarvest has no
                # mint call and no setMinter passthrough - its whole external
                # surface is postRoot/claim/sweep/sweepFloor/transferOperator -
                # so that rotation ends the coin's mint authority permanently,
                # and this line printed "ok" over it.
                #
                # A ScryGranary is the only contract that can hold this slot and
                # still do anything with it, and it identifies itself: `spoils()`
                # returns the coin it binds. Ask, and require the answer to name
                # THIS coin - a granary bound to the other one is the two-granary
                # footgun arriving by a different door.
                d_spoils=$(cast call "$next" 'spoils()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
                if [ -z "$d_spoils" ]; then
                    echo "   !! $sym -> $next is a CONTRACT that does not answer spoils()."
                    echo "      Only a ScryGranary can hold a minter slot and still mint."
                    echo "      A ScryHarvest here (SPOILS-ECONOMY.md said this until 2026-07-28) ends"
                    echo "      $sym's mint authority forever - it has no mint path at all."
                    warn=$((warn + 1))
                elif [ "$(lc "$d_spoils")" != "$(lc "$coin")" ]; then
                    echo "   !! $sym -> $next is a granary for a DIFFERENT coin ($d_spoils)."
                    echo "      Rotating $sym here strands it: that granary's stewardMint"
                    echo "      would mint the other coin and $sym could never be minted again."
                    warn=$((warn + 1))
                else
                    echo "   ok $sym -> $next (a ScryGranary binding $sym - the granary path)"
                fi ;;
            esac
        fi
    done

    cat <<EON

   ONE-WAY. setMinter is onlyMinter, so after this the deployer key cannot take
   the role back and cannot hand it on. Anything still needing a mint - a pool
   top-up, a claim contract, a harvest root, the GARDENER's own handoff to the
   granary - must be done first, or done by the new holder.

   about to set:
     OBOL   ${OBOL}  ->  ${NEXT_OBOL:-(unchanged)}
     MYRRH  ${MYRRH:-(none)}  ->  ${NEXT_MYRRH:-(unchanged)}
EON
    if [ "$rot_armed" != yes ]; then
        [ "$warn" != 0 ] && echo "
   ^ at least one precondition above is unproven. An armed run will REFUSE
     while any of them is - that is the point of them."
        echo "
   dry run - nothing sent. Add --arm to rotate."
        exit 0
    fi

    # A PRECONDITION THAT PRINTS AND PROCEEDS IS NOT A PRECONDITION. This block
    # used to emit `!!` lines and rotate anyway - on the one act in this file
    # that cannot be undone, at the end of a long sitting, in the same shape as
    # a green /health over a dead Redis. If a check cannot be proven, the
    # honest move is to refuse and let the operator either fix it or say so out
    # loud with ROTATE_FORCE=1.
    if [ "$warn" != 0 ]; then
        [ "${ROTATE_FORCE:-0}" = 1 ] || die "at least one precondition above is unproven, and this is ONE-WAY.
   Fix it, or - if you know why it cannot be proven from here (an offline
   ledger, a drop that will never exist) - say so deliberately:
     ROTATE_FORCE=1 MINTER_NEXT=... ./deploy_town.sh rotate --arm"
        echo "
   ROTATE_FORCE=1 - rotating with $warn unproven precondition(s). Your call."
    fi
    [ -n "${PRIVATE_KEY:-}" ] || die "PRIVATE_KEY unset - the CURRENT minter must sign"

    # ALL OR NOTHING IS ALREADY PROVEN, by the `ME_ROT` block in the
    # preconditions above: it reads `minter()` for every coin this run would
    # touch, compares each against the signer, and `die`s on any that is not
    # us - naming the granary case and printing the `steward` commands when
    # that is what happened. ROTATE_FORCE does not open it, because a minter()
    # that reads back as somebody else is proof rather than absence of proof.
    #
    # ⚠ THAT PARAGRAPH IS TRUE OF TWO OF ME_ROT's FOUR ARMS, NOT ALL FOUR, and
    # the difference is the whole point of this block. ME_ROT `die`s when
    # `minter()` reads back as somebody else — that is proof, and ROTATE_FORCE
    # does not open it. But it only WARNS when `minter()` could not be read at
    # all, and when `cast wallet address` returned nothing; both are absence of
    # proof, and ROTATE_FORCE=1 is exactly the flag that lets absence of proof
    # through. ROTATE_FORCE is not exceptional either: `deployments.json` has no
    # `claims` key on a first launch, so the run starts at warn=1 before anything
    # is wrong. So on a flaky RPC the honest sequence is: MYRRH's `minter()`
    # times out (warn, not die) -> forced -> OBOL's setMinter LANDS -> MYRRH's
    # reverts -> the two coins now retire through two different seats and the
    # error message names neither.
    #
    # A batch of irreversible calls proves the whole batch before sending any of
    # it. This is not a duplicate of ME_ROT; it is the part ME_ROT declines to
    # assert. It re-reads every minter EXACTLY NOW rather than trusting a value
    # read before a long precondition block, and it is deliberately deaf to
    # ROTATE_FORCE. (Restored 2026-07-29 after review; removed in the 07-28
    # merge as "a second, weaker copy".)
    say "final proof, immediately before the first irreversible send"
    SIGNER=$(cast wallet address --private-key "$PRIVATE_KEY" 2>/dev/null || echo "")
    [ -n "$SIGNER" ] || die "cast wallet address could not derive the signer from PRIVATE_KEY.
   NOTHING HAS BEEN SENT. Fix the key before rotating anything."
    for pair in "OBOL:$OBOL:$NEXT_OBOL" "MYRRH:$MYRRH:$NEXT_MYRRH"; do
        sym="${pair%%:*}"; rest="${pair#*:}"; addr="${rest%%:*}"; next="${rest#*:}"
        [ -n "$addr" ] && [ -n "$next" ] || continue
        m=$(cast call "$addr" 'minter()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
        [ -n "$m" ] || die "$sym: minter() is unreadable RIGHT NOW ($addr).
   NOTHING HAS BEEN SENT. This is a batch of one-way calls and it will not
   start one it cannot finish. ROTATE_FORCE does not open this."
        [ "$(lc "$m")" = "$(lc "$SIGNER")" ] || die "$sym.minter is $m, not the signer $SIGNER.
   NOTHING HAS BEEN SENT. Rotating the other coin first would have split the
   two coins across two seats. ROTATE_FORCE does not open this."
        echo "   ok $sym.minter is the signer - safe to send"
    done
    for pair in "OBOL:$OBOL:$NEXT_OBOL" "MYRRH:$MYRRH:$NEXT_MYRRH"; do
        sym="${pair%%:*}"; rest="${pair#*:}"; addr="${rest%%:*}"; next="${rest#*:}"
        [ -n "$addr" ] && [ -n "$next" ] || continue
        say "rotating $sym"
        cast send "$addr" 'setMinter(address)' "$next" \
            --rpc-url "$RPC" --private-key "$PRIVATE_KEY" >/dev/null \
            || die "$sym setMinter reverted - are you still the minter?"
        got=$(cast call "$addr" 'minter()(address)' --rpc-url "$RPC")
        [ "$(echo "$got" | tr 'A-F' 'a-f')" = "$(echo "$next" | tr 'A-F' 'a-f')" ] \
            || die "$sym minter reads back $got, not $next"
        echo "   $sym minter is now $got (read back off the chain)"
        town_set "${sym}_MINTER" "$got"
    done
    remind_ledger
    ;;

steward)
    # THE ACT THAT ACTUALLY RETIRES UNLIMITED MINT once the depth phases have
    # run - and on that ordering it is this phase, not `rotate`. Read this
    # before reaching for that one.
    #
    # `rotate` retires SpoilsToken.minter. But `gardener` and `granary` have
    # ALREADY MOVED that slot: DeployGardener.s.sol:175 and
    # DeployGranary.s.sol:54 both do `if (token.minter() == deployer)
    # token.setMinter(granary)`. So on any run where the depth phases came
    # first, the deployer holds NEITHER minter role, and `rotate --arm` reverts
    # on `minter only` - correctly, but only after printing a confident
    # precondition block that says nothing about why.
    #
    # What the deployer holds instead is ScryGranary.steward, and
    # stewardMint(to, amount) is UNCAPPED (ScryGranary.sol:87). That is the
    # same unlimited mint the launch set out to retire, one door over. Moving
    # the minter slot into a granary bought a POSTED DAILY CAP for the farm's
    # GRANT; it bought exactly nothing against the steward seat. A launch that
    # ran the depth phases and stopped has not retired anything - it has
    # renamed the door.
    #
    # Which phase closes your launch depends only on the order you ran:
    #   depth AFTER  the closing act -> `rotate`   (deployer still holds minter)
    #   depth BEFORE the closing act -> `steward`  (a granary holds minter)
    # Depth-before is the better order - the coin's minter ends up a
    # rate-limited contract rather than another hot key, which is what a
    # rug-screen reads off `minter()` - it just moves which seat you retire.
    #
    # ONE-WAY FOR YOU, not for the protocol. transferSteward is onlySteward, so
    # the old key cannot take the seat back. The NEW steward can transfer it on
    # and can rotateMinterAway(), so nothing is bricked - but nothing returns
    # to you either.
    need cast
    OBOL=$(town_get OBOL_TOKEN); MYRRH=$(town_get MYRRH_TOKEN)
    G_MYRRH=$(town_get GRANARY_MYRRH); G_OBOL=$(town_get GRANARY_OBOL)
    GARDENER_A=$(town_get GARDENER); SILO_A=$(town_get SILO)
    [ -n "$G_MYRRH$G_OBOL" ] || die "town.env records no granary.
   This phase retires the steward seat on a granary that exists. If you have
   not run \`gardener\` / \`granary\` yet, the seat holding unlimited mint is
   still SpoilsToken.minter and the phase you want is \`rotate\`."
    st_armed="$(armed "${1:-}")"

    # Per-granary targets, for the same reason `rotate` has them: the two coins
    # do different jobs and may retire to different places.
    NEXT_MYRRH="${STEWARD_NEXT_MYRRH:-${STEWARD_NEXT:-}}"
    NEXT_OBOL="${STEWARD_NEXT_OBOL:-${STEWARD_NEXT:-}}"
    [ -n "$NEXT_MYRRH$NEXT_OBOL" ] || die \
        "set STEWARD_NEXT (both granaries) or STEWARD_NEXT_MYRRH / STEWARD_NEXT_OBOL.
   Leave one unset to retire only the other.

   NO DEFAULT HERE, DELIBERATELY - transferSteward is the one-way act and a
   seat that retires to a default is a seat nobody chose. The posted answer is
   the dev wallet; paste it:
     STEWARD_NEXT=$DEV_WALLET ./deploy_town.sh steward --arm"

    # Who we are, so `steward()` can be compared against something rather than
    # just printed. In a dry run PRIVATE_KEY is often unset; that is a warn
    # below, never a hard stop here.
    ME=""
    [ -n "${PRIVATE_KEY:-}" ] && \
        ME=$(cast wallet address --private-key "$PRIVATE_KEY" 2>/dev/null || echo "")

    say "what the chain says the seats are right now"
    for row in "MYRRH:$G_MYRRH:$MYRRH" "OBOL:$G_OBOL:$OBOL"; do
        sym="${row%%:*}"; rest="${row#*:}"; gr="${rest%%:*}"; tok="${rest#*:}"
        [ -n "$gr" ] || { echo "   $sym granary: none in town.env"; continue; }
        # Tolerant on purpose: this is the ORIENTATION print. An address that
        # cannot answer must reach the precondition block below as a refusal
        # with a reason, not kill the run from inside an echo.
        echo "   $sym granary $gr"
        echo "      steward     -> $(cast call "$gr" 'steward()(address)' --rpc-url "$RPC" 2>/dev/null || echo '(unreadable - not a ScryGranary?)')"
        [ -n "$tok" ] && \
        echo "      $sym.minter -> $(cast call "$tok" 'minter()(address)' --rpc-url "$RPC" 2>/dev/null || echo '(unreadable)')"
    done

    say "the things that must already be done, because this is one-way"
    warn=0
    powder_door steward "$st_armed"

    # (1) THE SEAT MUST BE WORTH RETIRING. If the granary does not hold the
    # coin's minter role, unlimited mint is sitting wherever minter() actually
    # points and this phase moves a seat that controls nothing - the exact
    # false-completion `rotate` produces in the other direction.
    for row in "MYRRH:$G_MYRRH:$MYRRH" "OBOL:$G_OBOL:$OBOL"; do
        sym="${row%%:*}"; rest="${row#*:}"; gr="${rest%%:*}"; tok="${rest#*:}"
        [ -n "$gr" ] && [ -n "$tok" ] || continue
        m=$(cast call "$tok" 'minter()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
        if [ -z "$m" ]; then
            echo "   ?? $sym: could not read minter() - cannot prove this seat controls anything"
            warn=$((warn + 1))
        elif [ "$(lc "$m")" = "$(lc "$gr")" ]; then
            echo "   ok $sym.minter IS the granary - retiring this seat retires the mint"
        else
            echo "   !! $sym.minter is $m, NOT the granary $gr."
            echo "      Retiring this steward seat leaves unlimited mint wherever that points."
            echo "      If it is still the deployer, the phase you want is \`rotate\`."
            warn=$((warn + 1))
        fi
    done

    # (2) WE MUST HOLD THE SEAT WE ARE GIVING AWAY. transferSteward is
    # onlySteward; without this the armed run just reverts at the end of a long
    # sitting.
    for row in "MYRRH:$G_MYRRH" "OBOL:$G_OBOL"; do
        sym="${row%%:*}"; gr="${row#*:}"
        [ -n "$gr" ] || continue
        s=$(cast call "$gr" 'steward()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
        if [ -z "$s" ]; then
            echo "   ?? $sym granary: steward() unreadable"; warn=$((warn + 1))
        elif [ -z "$ME" ]; then
            echo "   ?? $sym granary steward is $s - PRIVATE_KEY unset, cannot confirm it is us"
            warn=$((warn + 1))
        elif [ "$(lc "$s")" = "$(lc "$ME")" ]; then
            echo "   ok $sym granary steward is us ($s)"
        else
            echo "   !! $sym granary steward is $s, not $ME - transferSteward would revert"
            warn=$((warn + 1))
        fi
    done

    # (3) EVERY GRANT MUST ALREADY BE SET, because setGrant is onlySteward too.
    # This is the check with real teeth and no equivalent in `rotate`: the
    # `granary` phase prints "grant the silo AFTER it exists" and nothing
    # enforces it, so a silo deployed and never granted becomes a permanently
    # dry drip the moment the seat moves. The farm's grant is set by
    # DeployGardener itself, so it is normally green - verify it anyway, since
    # a foreign-minter NOTE would have skipped it.
    for row in "farm:$G_MYRRH:$GARDENER_A:gardener" "silo:$G_OBOL:$SILO_A:silo"; do
        lbl="${row%%:*}"; rest="${row#*:}"; gr="${rest%%:*}"; rest="${rest#*:}"
        who="${rest%%:*}"; phase="${rest#*:}"
        [ -n "$gr" ] || continue
        if [ -z "$who" ]; then
            echo "   -- no $phase in town.env - nothing to grant (fine if you have not deployed it)"
            continue
        fi
        room=$(cast call "$gr" 'availableToday(address)(uint256)' "$who" --rpc-url "$RPC" 2>/dev/null || echo "")
        case "$room" in
        "")     echo "   ?? $lbl: availableToday($who) unreadable"; warn=$((warn + 1)) ;;
        0|0\ *) echo "   !! $lbl $who has NO grant on granary $gr."
                echo "      setGrant is onlySteward - after this phase only the new holder can"
                echo "      set it, and the $lbl can never pay. Run: cast send $gr \\"
                echo "        'setGrant(address,uint256)' $who <dailyCap> --private-key ..."
                warn=$((warn + 1)) ;;
        *)      echo "   ok $lbl $who granted ($room per day)" ;;
        esac
    done

    # (4) EVERYTHING THAT STILL NEEDS A MINT MUST BE FUNDED. Same list `rotate`
    # proves, for the same reason - after this, minting runs through the new
    # steward, not you.
    HARVEST_A=$(town_get HARVEST)
    if [ -n "$HARVEST_A" ] && [ -n "$OBOL" ]; then
        bal=$(cast call "$OBOL" 'balanceOf(address)(uint256)' "$HARVEST_A" --rpc-url "$RPC" 2>/dev/null || echo "")
        case "$bal" in
        "")     echo "   ?? harvest $HARVEST_A: balance unreadable"; warn=$((warn + 1)) ;;
        0|0\ *) echo "   !! harvest $HARVEST_A holds 0 OBOL - fund it BEFORE retiring the seat"
                warn=$((warn + 1)) ;;
        *)      echo "   ok harvest $HARVEST_A funded ($bal)" ;;
        esac
    else
        echo "   ?? no HARVEST in town.env - if a claim is planned, deploy+fund it first"
        warn=$((warn + 1))
    fi
    if [ -f deployments.json ]; then
        n=$(python3 -c 'import json,sys
try: print(len(json.load(open("deployments.json")).get("chains",{}).get("4663",{}).get("claims",{}) or {}))
except Exception: print(0)')
        [ "$n" = 0 ] && { echo "   ?? deployments.json records no per-drop claim contracts - if a drop is planned, deploy+fund it first"; warn=$((warn + 1)); } \
                     || echo "   ok $n drop(s) have claim contracts recorded"
    fi

    # (5) AND THE DESTINATION ITSELF - the only field here typed by a human
    # into a one-way call. Same typo filter as `rotate`, same reasoning: an
    # address with no nonce AND no code has never been used by anybody.
    # EIP-7702 accounts are EOAs WITH A DELEGATE, and three of the operator's
    # own wallets are 7702 on this chain, so the 0xef0100 indicator is read
    # explicitly rather than being mistaken for a contract.
    for row in "MYRRH:$G_MYRRH:$NEXT_MYRRH" "OBOL:$G_OBOL:$NEXT_OBOL"; do
        sym="${row%%:*}"; rest="${row#*:}"; gr="${rest%%:*}"; next="${rest#*:}"
        [ -n "$gr" ] && [ -n "$next" ] || continue
        cur=$(cast call "$gr" 'steward()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
        if [ -n "$cur" ] && [ "$(lc "$cur")" = "$(lc "$next")" ]; then
            echo "   !! $sym -> $next is ALREADY the steward - this would be a no-op"
            warn=$((warn + 1))
            continue
        fi
        n_nonce=$(cast nonce "$next" --rpc-url "$RPC" 2>/dev/null || echo "")
        n_code=$(cast code "$next" --rpc-url "$RPC" 2>/dev/null || echo "0x")
        if [ -z "$n_nonce" ]; then
            echo "   ?? $sym -> $next: the chain would not answer nonce() - cannot vet the destination"
            warn=$((warn + 1))
        elif [ "$n_nonce" = 0 ] && [ "$n_code" = "0x" ]; then
            echo "   !! $sym -> $next has NEVER sent a tx and holds no code."
            echo "      If that is a typo, the steward seat is gone for good. If it is a"
            echo "      fresh cold wallet, send one tx from it first and re-run."
            warn=$((warn + 1))
        else
            case "$n_code" in
            0xef0100*) echo "   ok $sym -> $next (EIP-7702 EOA, delegate 0x${n_code#0xef0100} - nonce $n_nonce)" ;;
            0x)        echo "   ok $sym -> $next (nonce $n_nonce - this key has signed before)" ;;
            *)         echo "   ok $sym -> $next (contract - a successor hub, no address typed by hand)" ;;
            esac
        fi
    done

    cat <<EON

   THIS PROPOSES; IT DOES NOT HAND OVER. transferSteward is two-step since
   2026-07-28: you stay the steward until the destination calls
   acceptSteward(), and until then you can cancelStewardTransfer(). So this
   act is reversible - it is the ACCEPT that is one-way.
   (This banner said the opposite until 2026-07-29, and read literally it
   invited destroying the deployer key right after the propose, which strands
   the granary exactly as badly as the typo the two-step was added to survive.)

   Still true: once the handoff completes, anything needing an unbudgeted mint
   - funding a harvest root, funding a claim contract, granting a new organ -
   is the new holder's to do. Do it first, or hand them the list.

   What this does NOT touch: the granted daily mints keep working. The farm
   goes on paying under the grant already set; only the unbudgeted
   stewardMint() and the knobs (setGrant / revokeGrant / rotateMinterAway)
   move.

   about to set:
     MYRRH granary  ${G_MYRRH:-(none)}  steward ->  ${NEXT_MYRRH:-(unchanged)}
     OBOL  granary  ${G_OBOL:-(none)}  steward ->  ${NEXT_OBOL:-(unchanged)}
EON
    if [ "$st_armed" != yes ]; then
        [ "$warn" != 0 ] && echo "
   ^ at least one precondition above is unproven. An armed run will REFUSE
     while any of them is - that is the point of them."
        echo "
   dry run - nothing sent. Add --arm to retire the seat."
        exit 0
    fi

    # A PRECONDITION THAT PRINTS AND PROCEEDS IS NOT A PRECONDITION - the same
    # defect `rotate` carried until 2026-07-27, on the same kind of act.
    if [ "$warn" != 0 ]; then
        [ "${STEWARD_FORCE:-0}" = 1 ] || die "at least one precondition above is unproven, and this is ONE-WAY.
   Fix it, or - if you know why it cannot be proven from here - say so
   deliberately:
     STEWARD_FORCE=1 STEWARD_NEXT=0x... ./deploy_town.sh steward --arm"
        echo "
   STEWARD_FORCE=1 - retiring with $warn unproven precondition(s). Your call."
    fi
    [ -n "${PRIVATE_KEY:-}" ] || die "PRIVATE_KEY unset - the CURRENT steward must sign"
    for row in "MYRRH:$G_MYRRH:$NEXT_MYRRH" "OBOL:$G_OBOL:$NEXT_OBOL"; do
        sym="${row%%:*}"; rest="${row#*:}"; gr="${rest%%:*}"; next="${rest#*:}"
        [ -n "$gr" ] && [ -n "$next" ] || continue
        say "proposing the $sym granary steward handoff"
        cast send "$gr" 'transferSteward(address)' "$next" \
            --rpc-url "$RPC" --private-key "$PRIVATE_KEY" >/dev/null \
            || die "$sym transferSteward reverted - are you still the steward?"
        # TWO-STEP since 2026-07-28, so the read-back is `pendingSteward`, NOT
        # `steward`. This block asserted `steward() == next` and would have
        # died here on a transaction that in fact succeeded.
        #
        # Why the contract changed: this seat is uncapped mint over a whole
        # coin, and `transferSteward` was one-way for the incumbent. The typo
        # filter above catches a destination that has NEVER transacted, which
        # is the common typo - it cannot catch a transposed character that
        # lands on a live address somebody else controls, and that outcome is
        # identical to losing the coin. Requiring the successor to sign is the
        # only check that proves the address is reachable.
        got=$(cast call "$gr" 'pendingSteward()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
        [ -n "$got" ] || die "$sym granary does not answer pendingSteward() - this granary
   predates the two-step handoff. Deploy the current ScryGranary, or accept
   that its seat transfers in one irreversible call."
        [ "$(lc "$got")" = "$(lc "$next")" ] \
            || die "$sym granary pendingSteward reads back $got, not $next"
        still=$(cast call "$gr" 'steward()(address)' --rpc-url "$RPC")
        echo "   $sym granary: handoff PROPOSED to $got"
        echo "     the seat is still $still until the successor accepts"
        town_set "GRANARY_${sym}_STEWARD_PENDING" "$got"
    done
    cat <<'EON'

   NOTHING IS RETIRED YET, AND THAT IS THE POINT. The seat moves when the
   SUCCESSOR signs, which is what proves the address is reachable at all:

     cast send <granary> 'acceptSteward()' --rpc-url $RPC --private-key <SUCCESSOR KEY>

   Until then you are still the steward and can withdraw the whole thing:

     cast send <granary> 'cancelStewardTransfer()' --rpc-url $RPC --private-key $PRIVATE_KEY

   `./deploy_town.sh status` prints a `!! handoff PENDING` line for any granary
   in this state, so an unaccepted proposal cannot be mistaken for a retirement.
   Record the seat in deployments.json only after it has been accepted.
EON
    remind_ledger
    ;;

# Publish the source of everything we broadcast, to the explorer the audience
# actually opens. This had no phase and no gate, and the habit had already
# lapsed: ScryVowRegistry is verified on Blockscout and ScryNotary - broadcast
# eight days later - is not. `forge script --broadcast` carries no --verify
# here, so nothing ever did it automatically.
#
# It matters more than it sounds for this launch specifically. The audience
# screens for rugs (DEGEN.md 1b: liquidity/FDV, holder concentration), and an
# unverified token contract is the first thing that screen fails on. Everything
# needed is derived - the constructor arguments come from foundry's own
# broadcast receipt and the types from the compiled artifact - so this cannot
# drift from what was actually deployed.
verify)
    need python3; need cast
    verify_armed="$(armed "${1:-}")"
    say "source verification against ${EXPLORER:-https://robinhoodchain.blockscout.com}"
    EXPLORER="${EXPLORER:-https://robinhoodchain.blockscout.com}" \
    ARMED="$verify_armed" RPC="$RPC" CHAIN="$CHAIN_ID_WANT" python3 - <<'PY'
import glob, json, os, subprocess, sys, urllib.request

explorer = os.environ["EXPLORER"].rstrip("/")
armed = os.environ["ARMED"] == "yes"
rpc, chain = os.environ["RPC"], os.environ["CHAIN"]

# Only real broadcasts. `broadcast/<script>/<chain>/dry-run/` holds SIMULATED
# deploys at addresses that were never sent, and verifying one would be
# verifying a contract that does not exist.
found = {}
for f in sorted(glob.glob(f"broadcast/*/{chain}/run-latest.json"), key=os.path.getmtime):
    for t in json.load(open(f)).get("transactions", []):
        if t.get("transactionType") != "CREATE":
            continue
        addr = (t.get("contractAddress") or "").lower()
        if addr:
            found[addr] = (t.get("contractName"), t.get("arguments") or [])

if not found:
    print("   no broadcast receipts on this chain yet - nothing deployed to verify.")
    sys.exit(0)


def artifact(name):
    for p in glob.glob(f"out/*/{name}.json"):
        d = json.load(open(p))
        tgt = ((d.get("metadata") or {}).get("settings") or {}).get("compilationTarget") or {}
        for src, cname in tgt.items():
            if cname == name:
                ctor = next((i for i in d["abi"] if i.get("type") == "constructor"), None)
                return src, [i["type"] for i in (ctor or {}).get("inputs", [])], d
    return None, None, None


def is_verified(addr):
    # Blockscout 403s the default python user-agent, which made every contract
    # read as "state unreadable" and would have re-submitted work already done.
    req = urllib.request.Request(f"{explorer}/api/v2/smart-contracts/{addr}",
                                 headers={"User-Agent": "scry-deploy_town/1.0"})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return bool(json.load(r).get("is_verified"))
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return False  # known: the explorer has no source for it
        return None
    except Exception:
        return None  # unknown: never report "verified" from a failed read


def same_code(onchain_hex, art):
    """Does the address actually hold THIS contract? Compared with immutables
    masked out, because they are baked in at construction and differ per deploy.

    This is not paranoia about the explorer. `broadcast/<script>/4663/` is keyed
    on CHAIN ID, and a local anvil fork of RH-Chain has the same chain id - so a
    rehearsal against a fork writes a receipt indistinguishable from a real one,
    at addresses that mostly do not exist on mainnet. Several such receipts are
    sitting in this repo right now. Submitting one would be publishing source
    for a contract that is not ours."""
    db = art["deployedBytecode"]
    want = bytearray(bytes.fromhex(db["object"][2:]))
    got_hex = onchain_hex[2:] if onchain_hex.startswith("0x") else onchain_hex
    got = bytearray(bytes.fromhex(got_hex))
    if len(want) != len(got):
        return False
    for spans in (db.get("immutableReferences") or {}).values():
        for s in spans:
            a, b = s["start"], s["start"] + s["length"]
            want[a:b] = got[a:b] = b"\0" * s["length"]
    return want == got


todo, bad = [], 0
for addr, (name, args) in found.items():
    if not name:
        continue
    r = subprocess.run(["cast", "code", addr, "--rpc-url", rpc],
                       capture_output=True, text=True)
    code = r.stdout.strip()
    if r.returncode != 0:
        # An RPC that will not answer is NOT an empty address. Saying "no code"
        # here would be the never-raises-reader trap: one value for "nothing
        # there" and "we could not look".
        print(f"   ?? {name} {addr}: the chain would not answer - {r.stderr.strip()[:80]}")
        bad += 1
        continue
    if code in ("", "0x"):
        print(f"   -- {name} {addr}: no code on chain {chain} - a rehearsal receipt, skipped")
        continue
    src, types, art = artifact(name)
    if not src:
        print(f"   !! {name} {addr}: no compiled artifact - run `forge build` first")
        bad += 1
        continue
    if not same_code(code, art):
        print(f"   -- {name} {addr}: the code there is not this contract's - "
              f"skipped (a fork rehearsal, or the address was reused)")
        continue
    v = is_verified(addr)
    if v is True:
        print(f"   ok {name} {addr} already verified")
        continue
    enc = ""
    if types:
        sig = "constructor(" + ",".join(types) + ")"
        out = subprocess.run(["cast", "abi-encode", sig, *[str(a) for a in args]],
                             capture_output=True, text=True)
        if out.returncode != 0:
            print(f"   !! {name} {addr}: could not encode {sig} from the receipt's arguments")
            print("     ", out.stderr.strip()[:200])
            bad += 1
            continue
        enc = out.stdout.strip()
    todo.append((name, addr, f"{src}:{name}", enc,
                 "unverified" if v is False else "verification state unreadable"))

for name, addr, target, enc, why in todo:
    cmd = ["forge", "verify-contract", addr, target,
           "--verifier", "blockscout", "--verifier-url", f"{explorer}/api/",
           "--chain-id", chain, "--watch"]
    if enc:
        cmd += ["--constructor-args", enc]
    print(f"\n   {name} {addr}  ({why})")
    print("     " + " ".join(cmd))
    if not armed:
        continue
    r = subprocess.run(cmd)
    if r.returncode != 0:
        print(f"   !! {name} verification failed - the address is live either way, "
              f"so re-run this phase; nothing else is blocked")
        bad += 1

if not todo:
    print("\n   every deployed contract on this chain carries verified source.")
elif not armed:
    print(f"\n   {len(todo)} contract(s) would be submitted. Nothing sent - add --arm.")
sys.exit(1 if bad else 0)
PY
    ;;

status)
    need cast
    say "the wiring, read back from the chain (town.env is a claim; this is the check)"
    MYRRH=$(town_get MYRRH_TOKEN); OBOL=$(town_get OBOL_TOKEN)
    GRANARY_A=$(town_get GRANARY); GRANARY_OBOL_A=$(town_get GRANARY_OBOL)
    GARDENER_A=$(town_get GARDENER)
    SILO_A=$(town_get SILO); SHRINE_A=$(town_get SHRINE); ORCHARD_A=$(town_get ORCHARD)
    # The granary check belongs to whichever coin the FARM pays. That coin has
    # moved twice and this block has been wrong in BOTH directions, each time
    # naming the wrong coin in the one tool you read to confirm a broadcast
    # worked. Derived, never typed - see farm_coin_sym() and FARMING.md 3a.
    FARM_COIN_SYM=$(farm_coin_sym)
    # EVERY COIN GETS A VERDICT, not just the farm's. This loop gated the whole
    # comparison on `$sym = $FARM_COIN_SYM`, so OBOL's minter was printed as a
    # bare address with nothing beside it — and OBOL's granary is what funds the
    # silo drip, every orchard season pot, and any post-launch harvest top-up.
    # With that slot pointing anywhere else, `ScrySilo.reap` still succeeds
    # paying zero (its try/catch carries the shortfall to `owedOf`), the orchard
    # fails on its first stewardMint, `availableToday` reads a healthy grant
    # because it consults the Grant struct and never `spoils.minter()` — and
    # status was entirely green over it. That is a false green in the one tool
    # RUNBOOK says to trust over town.env, read immediately before irreversible
    # acts. The MYRRH half of exactly this bug was caught on a fork rehearsal
    # 2026-07-27 and fixed for the grant-room lines only.
    for pair in "MYRRH:$MYRRH:$GRANARY_A" "OBOL:$OBOL:$GRANARY_OBOL_A"; do
        sym="${pair%%:*}"; rest="${pair#*:}"; addr="${rest%%:*}"; gran="${rest#*:}"
        [ -n "$addr" ] || continue
        m=$(cast call "$addr" 'minter()(address)' --rpc-url "$RPC")
        printf '   %-5s minter      %s\n' "$sym" "$m"
        if [ -n "$gran" ]; then
            if [ "$(lc "$m")" = "$(lc "$gran")" ]; then
                echo "                     = its own granary (correct - the hub holds the slot)"
            else
                echo "   !! $sym minter is NOT $sym's granary ($gran)"
                if [ "$sym" = "$FARM_COIN_SYM" ]; then
                    echo "      farm emissions cannot pay: _settle catches the revert and stashes"
                else
                    echo "      the silo drip and every orchard season pot cannot be funded;"
                    echo "      reap() still succeeds paying ZERO and nothing goes red"
                fi
            fi
            # The coin the granary BINDS is immutable and never read anywhere
            # else in this repo. Two granaries exist and they are not
            # interchangeable, so a stewardMint on the wrong one mints the wrong
            # coin while the approve beside it spends one you do not hold.
            gs=$(cast call "$gran" 'spoils()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
            if [ -n "$gs" ] && [ "$(lc "$gs")" != "$(lc "$addr")" ]; then
                echo "   !! $sym's recorded granary BINDS A DIFFERENT COIN ($gs)"
                echo "      town.env has GRANARY/GRANARY_OBOL crossed, or the wrong one was deployed"
            fi
        else
            echo "                     ($sym has no granary in town.env yet - deployer holds it"
            echo "                      until the gardener/granary phase or 'rotate' moves it)"
        fi
        printf '   %-5s supply      %s\n' "$sym" \
            "$(cast call "$addr" 'totalSupply()(uint256)' --rpc-url "$RPC")"
    done
    if [ -n "$GRANARY_A$GRANARY_OBOL_A" ]; then
        # BOTH stewards, because there are two granaries and each one holds
        # unbudgeted mint authority over a whole coin — the successor to the
        # deployer's unlimited mint key. Printing only the farm's hid the OBOL
        # one, which is the key the silo and every orchard season run on.
        #
        # Gated on EITHER granary, not on the MYRRH one. `GRANARY_A` is written
        # only by the `gardener` phase, while `granary` (OBOL) depends on nothing
        # and `steward` explicitly supports retiring one granary alone. So a town
        # that ran `granary` without `gardener` printed no OBOL steward, no
        # pending-handoff warning and no silo grant room — silently, with no
        # `??`. That is the same false green, in the same tool, one scope out
        # from the line this block was written to fix. (Review, 2026-07-29.)
        for gpair in "MYRRH:$GRANARY_A" "OBOL:$GRANARY_OBOL_A"; do
            gsym="${gpair%%:*}"; gaddr="${gpair#*:}"
            [ -n "$gaddr" ] || { echo "   granary steward   $gsym: (no granary in town.env)"; continue; }
            echo "   granary steward   $gsym: $(cast call "$gaddr" 'steward()(address)' --rpc-url "$RPC")"
            pend=$(cast call "$gaddr" 'pendingSteward()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
            case "$pend" in
                ""|0x0000000000000000000000000000000000000000) ;;
                *) echo "   !! $gsym steward handoff PENDING to $pend (unaccepted; two-step)" ;;
            esac
        done
        # Each organ's room is asked of ITS OWN granary. The silo's grant lives
        # on the OBOL granary (FARMING.md 3b); this loop used to ask the
        # gardener's MYRRH one — the `GRANARY` alias — for both, so a healthy
        # 250/day silo grant read as 0 in the one tool the runbook says to
        # trust over town.env. Proven on a fork rehearsal, 2026-07-27.
        for triple in "gardener:$GRANARY_A:$GARDENER_A" "silo:$GRANARY_OBOL_A:$SILO_A"; do
            who="${triple%%:*}"; rest="${triple#*:}"; gran="${rest%%:*}"; addr="${rest#*:}"
            [ -n "$addr" ] || continue
            if [ -n "$gran" ]; then
                echo "   grant room today  $who: $(cast call "$gran" 'availableToday(address)(uint256)' "$addr" --rpc-url "$RPC")"
            else
                # "no granary recorded" is not a room of zero — say which.
                echo "   grant room today  $who: ?? (its granary is not in town.env)"
            fi
        done
    fi
    [ -n "$GARDENER_A" ] && echo "   gardener pools    $(cast call "$GARDENER_A" 'poolCount()(uint256)' --rpc-url "$RPC")"
    [ -n "$SILO_A" ]     && echo "   silo bins/tiers   $(cast call "$SILO_A" 'binCount()(uint256)' --rpc-url "$RPC") / $(cast call "$SILO_A" 'tierCount()(uint256)' --rpc-url "$RPC")"
    [ -n "$SHRINE_A" ]   && echo "   shrine altars     $(cast call "$SHRINE_A" 'offeringCount()(uint256)' --rpc-url "$RPC")"
    if [ -n "$ORCHARD_A" ]; then
        echo "   orchard seasons   $(cast call "$ORCHARD_A" 'incentiveCount()(uint256)' --rpc-url "$RPC")"
        # the wage tripwire: a MYRRH-bound Orchard would violate SEASONS.md 4
        # silently and forever (the binding is immutable). Read, do not trust.
        ocoin=$(cast call "$ORCHARD_A" 'rewardToken()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
        OBOL_T=$(town_get OBOL_TOKEN)
        if [ -n "$ocoin" ] && [ -n "$OBOL_T" ] && [ "$(lc "$ocoin")" != "$(lc "$OBOL_T")" ]; then
            echo "   !! orchard rewardToken $ocoin is NOT town.env's OBOL_TOKEN $OBOL_T"
            echo "   !! seasons pay the wrong coin - SEASONS.md 4. Redeploy the orchard."
        fi
    fi

    # ── the bank + the posted split ──────────────────────────────────────
    # Read the SPLIT back rather than quoting the env that set it: those four
    # numbers weld at broadcast and the whole point of posting them is that a
    # stranger can check them without trusting us. Same reason burnBps() is
    # read off the contract instead of echoed.
    BANK_A=$(town_get BANK); SPLIT_A=$(town_get FEE_SPLITTER)
    if [ -n "$BANK_A" ]; then
        bank_pool=$(cast call "$SCRY_CANON" 'balanceOf(address)(uint256)' "$BANK_A" --rpc-url "$RPC" 2>/dev/null || echo "")
        bank_ts=$(cast call "$BANK_A" 'totalSupply()(uint256)' --rpc-url "$RPC" 2>/dev/null || echo "")
        bank_pool="${bank_pool%% *}"; bank_ts="${bank_ts%% *}"
        echo "   bank pool         $([ -n "$bank_pool" ] && fmt_units "$bank_pool" || echo '??') \$SCRY"
        echo "   xSCRY supply      $([ -n "$bank_ts" ] && fmt_units "$bank_ts" || echo '??')"
        echo "   redemption rate   $(cast call "$BANK_A" 'redemptionRate()(uint256)' --rpc-url "$RPC" 2>/dev/null || echo '??') (1e18 = 1.0000)"
        # THE SEED GATE. A transfer into an empty bank is not a seed, it is a
        # gift to the next depositor - the whole balance, for any deposit at
        # all. So the tool that would tell you to trickle says the opposite
        # until a real stake exists (TREASURY.md P9).
        #
        # ⚠ AN UNREADABLE SUPPLY MUST NOT READ AS SEEDABLE, and the naive
        # `[ "$bank_ts" = "0" ]` fails exactly that way: an empty string is not
        # "0", so a dead RPC would have printed "seedable" over a bank that is
        # empty. Same lesson as the silo's grant room printing 0 for "could not
        # look" (2026-07-27) - except this one points at 9,000,000 of the
        # purse, so it fails CLOSED.
        if [ -z "$bank_ts" ]; then
            echo "                     ?? COULD NOT READ totalSupply - treat as NOT seedable"
        elif [ "$bank_ts" = "0" ]; then
            echo "                     NOT SEEDABLE: no stakers. A transfer now is"
            echo "                     captured whole by the first depositor."
        else
            echo "                     seedable: a stake exists, so a tranche accrues pro rata"
        fi
    fi
    if [ -n "$SPLIT_A" ]; then
        n=$(cast call "$SPLIT_A" 'splitLength()(uint256)' --rpc-url "$RPC" 2>/dev/null || echo "")
        n="${n%% *}"
        # Same rule as the bank above: `??` for a read that did not happen,
        # never a number that looks like an answer. An empty `$n` would also
        # abort the loop's arithmetic under `set -u`.
        if [ -z "$n" ]; then
            echo "   posted split      ?? could not read splitLength()"
        else
            echo "   posted split      $n outlet(s), burn $(cast call "$SPLIT_A" 'burnBps()(uint16)' --rpc-url "$RPC" 2>/dev/null || echo '??') bps"
            i=0
            while [ "$i" -lt "$n" ]; do
                cut=$(cast call "$SPLIT_A" 'cuts(uint256)(address,uint16)' "$i" --rpc-url "$RPC" 2>/dev/null | tr '\n' ' ' || echo '??')
                printf '                     %s\n' "${cut:-??}"
                i=$((i + 1))
            done
        fi
        split_held=$(cast call "$SCRY_CANON" 'balanceOf(address)(uint256)' "$SPLIT_A" --rpc-url "$RPC" 2>/dev/null | cut -d' ' -f1 || echo "")
        split_burned=$(cast call "$SPLIT_A" 'totalBurned()(uint256)' --rpc-url "$RPC" 2>/dev/null | cut -d' ' -f1 || echo "")
        echo "   undistributed     $([ -n "$split_held" ] && fmt_units "$split_held" || echo '??') \$SCRY (anyone may call distribute())"
        echo "   burned to date    $([ -n "$split_burned" ] && fmt_units "$split_burned" || echo '??') \$SCRY"
    fi

    # ScryVowRegistry is LIVE and UNPATCHABLE, and `setOracle` has no
    # zero-check (AUDIT-2026-07-25.md §3; the full hazard note and the safe
    # rotation sequence are RUNBOOK.md §4b). One mistyped address ends daily
    # anchoring FOREVER with no owner path back — and it would fail silently,
    # because nothing else on this box ever reads the slot. So read it here,
    # every time someone checks the wiring, rather than only in an incident.
    say "the live registry's oracle (unpatchable - RUNBOOK 4b)"
    orc=$(cast call "$VOW_REGISTRY" 'oracle()(address)' --rpc-url "$RPC" 2>/dev/null || echo "")
    if [ -z "$orc" ]; then
        echo "   !! could not read oracle() from $VOW_REGISTRY"
    elif [ "$(echo "$orc" | tr 'A-F' 'a-f')" = "0x0000000000000000000000000000000000000000" ]; then
        echo "   !! ORACLE IS THE ZERO ADDRESS - anchoring is dead and cannot be restored"
    else
        echo "   vow oracle        $orc"
        [ "$(cast code "$orc" --rpc-url "$RPC")" = "0x" ] \
            && echo "                     (an EOA - it must be a key you still hold)" \
            || echo "                     (a contract)"
    fi
    ;;

# §M4's on-chain half. `postRoot` takes `totalCommitted_` from the operator and
# cannot check it against the tree — a merkle root commits to its leaves, never
# to their sum. `preflight.py::gate_merkle_totals_are_derived` proves the
# ARTIFACT is self-consistent offline; this proves the CHAIN carries that same
# artifact. Between the two, the number the sweep floor is armed to is derived
# end to end instead of asserted.
claims)
    need cast; need python3
    say "posted roots vs the merkle artifacts they should have come from"
    RPC="$RPC" python3 - <<'PY'
import json, os, subprocess, sys, pathlib

here = pathlib.Path(os.getcwd())
repo = here.parent if here.name == "contracts" else here
rpc = os.environ["RPC"]

dep = json.loads((repo / "contracts" / "deployments.json").read_text())
claims = (dep.get("chains", {}).get("4663", {}) or {}).get("claims") or {}
if not claims:
    print("   no claim contracts recorded in deployments.json yet.")
    print("   This arms after `forge script script/DeployClaim.s.sol --broadcast`,")
    print("   which LAUNCH.md requires you to record under chains.4663.claims.")
    sys.exit(0)


def call(addr, sig):
    out = subprocess.run(["cast", "call", addr, sig, "--rpc-url", rpc],
                         capture_output=True, text=True)
    return out.stdout.strip() if out.returncode == 0 else None


bad = 0
for drop_id, tokens in claims.items():
    print(f"   drop {drop_id}")
    for token, addr in (tokens or {}).items():
        if not isinstance(addr, str) or not addr.startswith("0x"):
            continue
        art = repo / "snapshots" / f"{drop_id}.{token.lower()}.merkle.json"
        onchain_root = call(addr, "root()(bytes32)")
        onchain_total = call(addr, "totalCommitted()(uint256)")
        floor = call(addr, "sweepFloor()(uint256)")
        if onchain_root is None:
            print(f"     !! {token} {addr}: unreadable")
            bad += 1
            continue
        if not art.exists():
            print(f"     !! {token} {addr}: root {onchain_root} posted, but NO artifact"
                  f" at {art.name} to check it against")
            bad += 1
            continue
        a = json.loads(art.read_text())
        want_total = int(a["total_wei"])
        got_total = int((onchain_total or "0").split()[0].replace(",", ""))
        ok_root = onchain_root.lower() == a["root"].lower()
        ok_total = got_total == want_total
        print(f"     {token} {addr}")
        print(f"       root  {'OK ' if ok_root else '!! MISMATCH'} {onchain_root}")
        print(f"       total {'OK ' if ok_total else '!! MISMATCH'} {got_total}"
              f"{'' if ok_total else f'  (artifact says {want_total})'}")
        print(f"       sweep floor {floor}")
        if not (ok_root and ok_total):
            bad += 1
if bad:
    print(f"\n   {bad} claim contract(s) disagree with their artifact - do NOT open claims")
    sys.exit(1)
print("\n   every posted root and total matches the artifact it came from")
PY
    ;;

config)
    need python3
    say "emitting watchtower/gardens.config.json from town.env"
    OBOL=$(town_get OBOL_TOKEN) MYRRH=$(town_get MYRRH_TOKEN) \
    GARDEN=$(town_get GARDEN_MYRRH_OBOL) GRANARY_A=$(town_get GRANARY) GARDENER_A=$(town_get GARDENER) \
    FARM_SYM=$(farm_coin_sym) \
    python3 - <<PY
import json, os, sys
need = {k: os.environ.get(v) or None for k, v in {
    "obol": "OBOL", "myrrh": "MYRRH", "garden": "GARDEN",
    "granary": "GRANARY_A", "gardener": "GARDENER_A"}.items()}
missing = [k for k, v in need.items() if not v]
if missing:
    sys.exit("ABORT: town.env is missing " + ", ".join(missing) + " - deploy those phases first")
# The farm's crop, derived by farm_coin_sym() from the gardener phase itself.
# This line said OBOL for a day after the coin moved to MYRRH, which would have
# published the wrong symbol AND the wrong token address to the gardens page -
# a card naming a coin the farm does not pay. The farmed LP is unaffected: it
# is the MYRRH/OBOL Garden SEED either way, because no ScryGarden holds \$SCRY.
farm_sym = os.environ["FARM_SYM"]
farm_addr = need[farm_sym.lower()]
cfg = {
  "chainId": 4663, "chainHex": "0x1237", "chainName": "Robinhood Chain",
  "rpcUrls": ["https://rpc.mainnet.chain.robinhood.com"],
  "explorer": "https://robinhoodchain.blockscout.com",
  "nativeCurrency": {"name": "Ether", "symbol": "ETH", "decimals": 18},
  "gardener": need["gardener"], "granary": need["granary"],
  "reward": {"sym": farm_sym, "addr": farm_addr, "dec": 18},
  "farms": [{
    "pid": 0, "label": "MYRRH / OBOL SEED", "garden": need["garden"],
    "token0": {"sym": "MYRRH", "addr": need["myrrh"], "dec": 18},
    "token1": {"sym": "OBOL", "addr": need["obol"], "dec": 18}}],
}
path = os.path.join("..", "watchtower", "gardens.config.json")
json.dump(cfg, open(path, "w"), indent=2)
print("   wrote", path, "- the farm page lights up on next serve")
print("   verify every address on-chain before trusting it (POOLS.md 4.1)")
PY
    ;;

help|*)
    # Print the whole leading comment block, however long it grows. It used to
    # be a hardcoded line range and the header outgrew it, so `help` silently
    # stopped showing the last third of the usage - including two variables
    # that change what an armed run does.
    awk 'NR == 1 { next }
         /^#/    { sub(/^# ?/, ""); print; next }
                 { exit }' "$0"
    ;;
esac
