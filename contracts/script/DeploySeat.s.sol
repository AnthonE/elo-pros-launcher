// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloSeat.sol";
import "../src/EloSeatArt.sol";

/// Deploy the founder mint — the Elo Pros as a broadcast.
///
/// ⚠ EVERY OPEN KNOB IN THE DESIGN IS AN ENV VAR HERE, and that is the point
/// of the file: the operator's sentence becomes a deploy value, never a code
/// edit. Nothing below has a default that pretends to be a decision — the four
/// figures that are genuinely welded (supply, the reserved caps, the ladder,
/// the burn) must be typed, and the run aborts rather than guess.
///
/// ⚠ THE NAME IS SETTLED: **Elo Pros** (operator, 2026-08-20, superseding
/// "Scry Hive" of 2026-08-09 — it interlocks with the ELO ticker and
/// elopros.com). It is a
/// constructor argument on both contracts and the site reads `name()` off the
/// deployed collection, so changing it later is a redeploy rather than an edit
/// — no page hard-codes it.
///
/// ⚠ THIS SCRIPT ALSO DEPLOYS THE RENDERER AND WIRES IT. `EloSeatArt` holds
/// the art as bytecode and has NO owner and NO setters, so there is nothing to
/// upload and nothing to seal. What it does NOT do is call
/// `freezeMetadata()` — that is one-way and belongs after the salt is revealed
/// and the art has been seen rendering correctly on a real explorer.
///
///   export PRIVATE_KEY=0x...
///   # the collection is NAMED (operator, 2026-08-09) — these are the defaults:
///   # export SEAT_NAME="Elo Pros"
///   # export SEAT_SYMBOL="PROS"
///   # addresses — read the real ones out of deployments.json, never retype:
///   export SEAT_SCRY=0x...                # chains.4663.contracts.ELO -- NOT .SCRY,
///                                         #   which is the RETIRED reserve. IMMUTABLE:
///                                         #   welding it strands the whole ladder
///                                         #   (ONE-SHOT.md 1). No ELO key exists until
///                                         #   the pons launch is recorded.
///   export SEAT_SPLITTER=0x...            # chains.4663.contracts.EloFeeSplitter
///   # THE WELDED FIGURES — no defaults, and the run aborts naming the one you
///   # missed. A cap of 0 is a real answer (it collapses the scheduling knob to
///   # "wait for a playable title"); it just has to be typed, because an
///   # unclaimable door is immutable and looks exactly like an open one.
///   export SEAT_SUPPLY=8192               # operator, 2026-08-08
///   # ⚠ SPOKEN 2026-08-12: ONE DOOR. "we only have one door this one and i need
///   # like max amount of people LOL just not dust". Door 1 takes the whole
///   # float; doors 2, 3 and the PAID door are all 0. The four must sum to
///   # <= supply (the constructor requires it): 7692 + 0 + 0 + 500 = 8192.
///   # ⚠ RE-CUT 2026-08-21 AND THIS BLOCK CARRIED 8092/100 UNTIL THEN — the
///   # superseded pair. It is a copy-pasteable header, so a stale figure here
///   # welds a collection nobody chose. contracts/hive.env.example is the
///   # staged record; read the figures from there, never from this comment.
///   # ⚠ WHAT A 0 CAP MEANS, since it is immutable: setDoorRoot still succeeds,
///   # /mint.html still renders the door, and every claim reverts for as long as
///   # the contract exists. For the PAID door it means no seat is ever sold, so
///   # the mint raises nothing and the free seats are the whole collection.
///   export SEAT_SNAPSHOT_CAP=7692         # door 1 — the whole free float
///   export SEAT_PLAY_CAP=0                # door 2 — welded shut
///   export SEAT_BUILD_CAP=0               # door 3 — welded shut
///   export SEAT_TREASURY_CAP=500          # operator, 2026-08-21 (400 of it is
///                                         # pre-declared gacha prize inventory,
///                                         # so it is never a post-close sweep)
///   export SEAT_RESERVED_UNTIL=...        # unix ts: when the three free doors
///                                         # shut and their unclaimed remainder
///                                         # becomes paid float. WELDED, public
///                                         # before mint #1, and REQUIRED since
///                                         # 2026-08-12 — `=0` is still the way
///                                         # to say "never expires", it just has
///                                         # to be said. It was optional, and an
///                                         # omission welded that answer in
///                                         # silence.
///   export SEAT_BURN_BPS=5000             # the welded activation burn
///   # optional:
///   # export SEAT_PROCEEDS=0x...          # where swept ETH lands; default dev wallet
///   # export SEAT_ROYALTY_BPS=500         # ERC-2981, to the splitter. Max 1000
///   # export SEAT_BASE_URI=https://elopros.com/api
///   # THE LADDER — comma-separated, ascending, same length, SUBLINEAR.
///   # ⚠ THE WEIGHTS BELOW ARE THE OPERATOR'S, CLOSED 2026-08-11 (SENTENCES.md):
///   # five tiers, "100,125,160,200,333" until then — the pre-sentence draft —
///   # and a copy-paste out of a header is exactly how a ladder nobody chose
///   # gets welded into a constructor argument. The WEIGHTS are all that
///   # survives from 08-11; this line also carried "costs x1/3/7/14/25, base
///   # efficiency 7.51x the top" and both figures died the next day, three
///   # lines above the corrected costs that replaced them.
///   # Over the locked x20 span the base rung buys 6.01x the weight per token
///   # that the top does — derive it, do not quote it: it moves with the ladder.
///   # ⚠ THE COSTS ARE NO LONGER AN EXAMPLE — they were LOCKED 2026-08-12
///   # (SENTENCES.md; docs/money/SEAT-MONEY.md §3 carries the table and the
///   # arithmetic). 20,000 -> 400,000 whole SCRY, which at the tape that
///   # day is $0.35 -> $7.04, and $20 -> $400 at a $1M cap. The old row read
///   # "66666,200000,466666,933333,1666666" — StonkBrokers' rungs, copied
///   # before anyone priced ours, and worth about a dollar at tier 1.
///   # ⚠ THE SPACING IS 1/3/6/12/20 AND NOT 1/3/7/14/20, deliberately: the
///   # second gives upgrade steps of 1/2/4/7/6M, so the jump to the TOP rung
///   # costs less than the jump below it and tier 4 is strictly dominated.
///   export SEAT_TIER_COSTS="20000,60000,120000,240000,400000"
///   export SEAT_TIER_WEIGHTS="100,160,220,275,333"                # x100
///   # THE SEAL — sha256 of a salt you generate ONCE and keep offline until the
///   # run closes. There is no way to change it later and no way to recover it:
///   #   SALT=$(openssl rand -hex 32); echo "$SALT" > seat.salt.KEEP-OFFLINE
///   #   export SEAT_SALT_COMMITMENT=0x$(printf %s "$SALT" | openssl dgst -sha256 -r | cut -d' ' -f1)
///
///   forge script script/DeploySeat.s.sol --rpc-url $RPC --broadcast
///
/// ⚠ IT DEPLOYS WITH EVERY DOOR SHUT, deliberately. No root is posted and no
/// price is set, so nothing can be minted by anyone — including by whoever is
/// watching the mempool during the broadcast. Opening a door is its own later
/// transaction (`setDoorRoot`, `setPaidDoor`) taken when the cohort is pinned
/// and the price is derived off the tape that day.
///
/// After the broadcast, in order:
///   1. record the address in `contracts/deployments.json` as `EloSeat`
///      (`broadcast/` is gitignored, so that file is the durable record)
///   2. `python3 meter/seat_roots.py build --allowlist <cohort> --door 1 --cap <n>`
///   3. `cast send $SEAT 'setDoorRoot(uint8,bytes32)' 1 <root>`
///   4. verify the source (`contracts/RUNBOOK.md` §verify — the v2 API recipe;
///      `forge verify-contract` cannot do it from this box any more)
///   5. `/api/seat` and `/mint.html` light up with no code change
contract DeploySeat is Script {
    address constant DEV_WALLET = 0xb474a95200bC5de8950E445B1E9d524f4de0f18D;

    /// A comma-separated env list -> uint256[]. Written out rather than using
    /// `vm.envUint(name, ",")` because that reverts with an unreadable message
    /// on a trailing comma or a stray space, and this is the one input where a
    /// typo produces a WELDED wrong ladder rather than a failed run.
    function _split(string memory csv) internal pure returns (uint256[] memory out) {
        bytes memory b = bytes(csv);
        uint256 n = b.length == 0 ? 0 : 1;
        for (uint256 i = 0; i < b.length; i++) {
            if (b[i] == ",") n++;
        }
        out = new uint256[](n);
        uint256 k;
        uint256 acc;
        bool any;
        for (uint256 i = 0; i <= b.length; i++) {
            if (i == b.length || b[i] == ",") {
                require(any, "ladder: empty field");
                out[k++] = acc;
                acc = 0;
                any = false;
            } else if (b[i] == " ") {
                continue;
            } else {
                require(b[i] >= "0" && b[i] <= "9", "ladder: not a number");
                acc = acc * 10 + (uint8(b[i]) - 48);
                any = true;
            }
        }
    }

    /// ⚠ A WELDED FIGURE HAS NO DEFAULT, AND THIS IS THE FUNCTION THAT MEANS IT.
    /// The header above has always promised that supply, the reserved caps and
    /// the burn "must be typed, and the run aborts rather than guess" — but only
    /// the ladder actually aborted; the rest were `vm.envOr(..., 0)`. A deploy
    /// that forgot `SEAT_SNAPSHOT_CAP` therefore welded a collection where door 1
    /// can NEVER mint: `setDoorRoot(1, …)` still succeeds, the card still reads
    /// the door as open, and every `claim` reverts "door's reserve spent" for as
    /// long as the contract exists, because the caps are immutable. A cap of 0
    /// is a legitimate and documented choice (§4's scheduling knob) — it just has
    /// to be a choice somebody made, which is exactly the difference a default
    /// erases. `type(uint256).max` is the sentinel because no cap can be it.
    function _mustUint(string memory name) internal view returns (uint256 v) {
        v = vm.envOr(name, type(uint256).max);
        require(
            v != type(uint256).max,
            string.concat(
                "set ", name, " - it is welded at construction and immutable afterwards, so this "
                "script will not guess it. 0 is a valid answer; it has to be typed"
            )
        );
    }

    /// Whole SCRY in the env, 18-decimal units into the constructor. The env is
    /// typed by a person, so it takes the unit a person says out loud.
    function _costs(string memory csv) internal pure returns (uint256[] memory out) {
        out = _split(csv);
        for (uint256 i = 0; i < out.length; i++) {
            require(out[i] > 0, "ladder: a zero-cost tier");
            out[i] *= 1e18;
        }
    }

    /// Weights are already x100 — 100 is 1.00x. No scaling, just a range check.
    function _weights(string memory csv) internal pure returns (uint32[] memory out) {
        uint256[] memory raw = _split(csv);
        out = new uint32[](raw.length);
        for (uint256 i = 0; i < raw.length; i++) {
            require(raw[i] > 0 && raw[i] <= type(uint32).max, "ladder: weight out of range");
            out[i] = uint32(raw[i]);
        }
    }

    function run() external {
        string memory name_ = vm.envOr("SEAT_NAME", string("Elo Pros"));
        string memory symbol_ = vm.envOr("SEAT_SYMBOL", string("PROS"));
        address reserve = vm.envAddress("SEAT_SCRY");
        address splitter = vm.envAddress("SEAT_SPLITTER");
        address proceeds = vm.envOr("SEAT_PROCEEDS", DEV_WALLET);

        // ⚠ SEAT_RESERVED_UNTIL IS DEMANDED, AND IT WAS NOT UNTIL 2026-08-12.
        // This block used to argue the opposite: that zero is a real answer
        // here — strand an unclaimed reserve forever — so demanding it "would
        // force a date on an operator who meant no window."
        //
        // That reasoning does not survive reading `_mustUint`. Its sentinel is
        // `type(uint256).max`, chosen precisely so that ZERO STAYS TYPEABLE; its
        // own error message says "0 is a valid answer; it has to be typed." So
        // demanding this forces a DECISION, never a date, and `=0` remains the
        // one-character way to say "no window."
        //
        // What the old default actually bought was silence. Forgetting the line
        // entirely welded "the free doors never expire" — permanently, with no
        // setter — and looked identical to having chosen it. `NOW.md` carried
        // that as a standing hazard and named it the one class of decision where
        // being forgotten is permanent. The past-timestamp check the old comment
        // pointed at catches a DIFFERENT mistake (a stale date) and cannot catch
        // this one, because an omission has no timestamp to check.
        uint256[6] memory caps = [
            _mustUint("SEAT_SUPPLY"),
            _mustUint("SEAT_SNAPSHOT_CAP"),
            _mustUint("SEAT_PLAY_CAP"),
            _mustUint("SEAT_BUILD_CAP"),
            _mustUint("SEAT_TREASURY_CAP"),
            _mustUint("SEAT_RESERVED_UNTIL")
        ];

        // ⚠ A RESERVE WINDOW CAN BE STALE WITHOUT BEING INVALID, and the
        // constructor cannot catch that. Its only check is `caps[5] == 0 ||
        // caps[5] > block.timestamp` — so a timestamp two days out passes
        // exactly like one sixty days out. The operator sentence (2026-08-12) is
        // "60 days from DEPLOY", which is RELATIVE, while this argument is
        // ABSOLUTE: a value computed when the env file was first filled in and
        // then broadcast three weeks later silently welds a 39-day window, and
        // there is no setter to widen it afterwards.
        //
        // A warning rather than a `require`, deliberately: a deliberately short
        // window is a legitimate choice and this script does not get to overrule
        // one. What it can do is make the stale case impossible to broadcast
        // without having read the number out loud.
        if (caps[5] != 0 && caps[5] < block.timestamp + 14 days) {
            console2.log("");
            console2.log("!! RESERVE WINDOW IS UNDER 14 DAYS FROM NOW - is that what you meant?");
            console2.log("   welded, no setter. If this was computed days ago, recompute it:");
            console2.log("     date -d '+60 days' +%s      (the 2026-08-12 sentence)");
            console2.log("   window closes at (unix)", caps[5]);
            console2.log("   ...which is this many seconds away:", caps[5] - block.timestamp);
            console2.log("");
        }

        // ⚠ THE ROYALTY *IS* WELDED -- `EloSeat.royaltyBps` is `immutable` with no
        // setter, and hive.env.example marks it [WELDED]. This comment read "not
        // in the welded set" until 2026-08-18, which is literally false and made
        // the default below look free. What is true is the softer thing it meant:
        // ERC-2981 is a REQUEST a marketplace may ignore, so being wrong here is
        // cheap in a way the reserve address and the caps are not. 500 is the
        // posted figure rather than a guess -- but it is welded at 500.
        uint256 royaltyRaw = vm.envOr("SEAT_ROYALTY_BPS", uint256(500));
        uint256 burnRaw = _mustUint("SEAT_BURN_BPS");
        // ⚠ RANGE-CHECK BEFORE THE CAST, because a narrowing cast in Solidity is
        // silent. `uint16(70000)` is 4464 — which passes the constructor's
        // `<= 10_000` and welds a burn nobody typed, on the one figure this
        // script refuses to guess. A fat-fingered extra zero has to fail here,
        // not become an immutable.
        require(burnRaw <= 10_000, "SEAT_BURN_BPS is basis points - max 10000 (100%)");
        require(royaltyRaw <= 1000, "SEAT_ROYALTY_BPS is basis points - max 1000 (10%)");
        uint96 royaltyBps = uint96(royaltyRaw);
        uint16 burnBps = uint16(burnRaw);
        bytes32 commitment = vm.envBytes32("SEAT_SALT_COMMITMENT");

        // Sequential, not an inline default: a url literal inside envOr's
        // argument list reads as a comment to the preflight scanner and
        // unbalances its walk (the DeployGardener.envAddrOr lesson).
        string memory base = vm.envOr("SEAT_BASE_URI", string(""));
        if (bytes(base).length == 0) {
            base = "https://elopros.com/api";
        }

        string memory costCsv = vm.envOr("SEAT_TIER_COSTS", string(""));
        string memory weightCsv = vm.envOr("SEAT_TIER_WEIGHTS", string(""));
        require(bytes(costCsv).length > 0 && bytes(weightCsv).length > 0,
            "set SEAT_TIER_COSTS and SEAT_TIER_WEIGHTS - the ladder is not guessed here");
        uint256[] memory costs = _costs(costCsv);
        uint32[] memory weights = _weights(weightCsv);

        // ⚠ The pre-flight the CONSTRUCTOR also enforces, run here first so a
        // bad ladder costs a script run instead of a failed broadcast with gas
        // spent. Same inequality, cross-multiplied: w[i]/w[0] < c[i]/c[0].
        require(costs.length == weights.length, "ladder: length mismatch");
        for (uint256 i = 1; i < costs.length; i++) {
            require(costs[i] > costs[i - 1], "ladder: cost not ascending");
            require(weights[i] > weights[i - 1], "ladder: weight not ascending");
            require(uint256(weights[i]) * costs[0] < uint256(weights[0]) * costs[i],
                "ladder: NOT SUBLINEAR - a tier weighs at least what it costs, which hands "
                "the distribution to whoever arrives richest");
        }

        console2.log("about to deploy - every door SHUT, nothing mintable:");
        console2.log("  supply", caps[0]);
        console2.log("  snapshot / play / build reserved", caps[1] + caps[2] + caps[3]);
        console2.log("  treasury", caps[4]);
        console2.log("  paid float", caps[0] - caps[1] - caps[2] - caps[3] - caps[4]);
        if (caps[5] == 0) {
            console2.log("  reserve window: NONE - unclaimed free seats strand forever");
        } else {
            console2.log("  reserve window closes at (unix)", caps[5]);
            console2.log("  ...after which unclaimed free seats join the paid float");
        }
        console2.log("  tiers", costs.length);

        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        EloSeat s = new EloSeat(
            name_, symbol_, caps, IERC20(reserve), splitter, proceeds,
            royaltyBps, burnBps, commitment, costs, weights, base
        );
        // ⚠ THIS STRING IS WELDED AND IT IS WHAT A MARKETPLACE PRINTS. The
        // renderer has no setter of any kind, so the description is fixed at
        // construction and changing it later is a redeploy, exactly like the
        // name above. It read "a capped run of agents" while the art was a
        // hooded figure; the art is a creature now (2026-08-10), and a
        // description that names something the picture does not show is the
        // same drift as a trait whose colour moved out from under its name.
        // "characters" is the word the origin's own /seat/collection already
        // uses, so the two surfaces now agree.
        // The third argument is the base for the collection's banner and its
        // featured card — `<base>/banner.jpg` and `<base>/featured.jpg`, the
        // two frames `watchtower/art/make_hive_banner.py` composes. The avatar
        // is not here because the renderer draws it. Weld "" and the document
        // simply carries no banner, which is the honest shape for a fork that
        // has not drawn one.
        EloSeatArt artc = new EloSeatArt(
            name_,
            "A capped run of characters. The seat transfers; the record never does.",
            vm.envOr("SEAT_ART_BASE", string("https://elopros.com/art/hive"))
        );
        require(artc.layerSumsAreExact(), "renderer: a weight row does not sum to 10000 bps");
        s.setRenderer(address(artc));
        vm.stopBroadcast();

        console2.log("EloSeat", address(s));
        console2.log("EloSeatArt (no owner, no setters)", address(artc));
        console2.log("  name", name_);
        console2.log("  reserve", reserve);
        console2.log("  splitter (royalties + the activation remainder)", splitter);
        console2.log("  proceeds (swept ETH only)", proceeds);
        console2.log("");
        console2.log("DEPLOYED SHUT. No root is posted and no price is set, so nothing");
        console2.log("can be minted by anyone yet. Next:");
        console2.log("  1. record the address in contracts/deployments.json as EloSeat");
        console2.log("  2. meter/seat_roots.py build --allowlist <cohort> --door 1");
        console2.log("  3. setDoorRoot(1, <root>)   then setPaidDoor(wei, reserve) when priced");
        console2.log("  4. KEEP THE SALT OFFLINE until the run closes - it cannot be changed");
        console2.log("  5. AFTER revealSalt + eyeballing the art: freezeMetadata() (one way)");
    }
}
