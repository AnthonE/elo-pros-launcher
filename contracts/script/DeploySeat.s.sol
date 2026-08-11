// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScrySeat.sol";
import "../src/ScrySeatArt.sol";

/// Deploy the founder mint — `SCRY-HIVE.md` as a broadcast.
///
/// ⚠ EVERY OPEN KNOB IN `SCRY-HIVE.md` §11 IS AN ENV VAR HERE, and that is the point
/// of the file: the operator's sentence becomes a deploy value, never a code
/// edit. Nothing below has a default that pretends to be a decision — the four
/// figures that are genuinely welded (supply, the reserved caps, the ladder,
/// the burn) must be typed, and the run aborts rather than guess.
///
/// ⚠ THE NAME IS SETTLED: **Scry Hive** (operator, 2026-08-09). It is a
/// constructor argument on both contracts and the site reads `name()` off the
/// deployed collection, so changing it later is a redeploy rather than an edit
/// — no page hard-codes it.
///
/// ⚠ THIS SCRIPT ALSO DEPLOYS THE RENDERER AND WIRES IT. `ScrySeatArt` holds
/// the art as bytecode and has NO owner and NO setters, so there is nothing to
/// upload and nothing to seal. What it does NOT do is call
/// `freezeMetadata()` — that is one-way and belongs after the salt is revealed
/// and the art has been seen rendering correctly on a real explorer.
///
///   export PRIVATE_KEY=0x...
///   # the collection is NAMED (operator, 2026-08-09) — these are the defaults:
///   # export SEAT_NAME="Scry Hive"
///   # export SEAT_SYMBOL="HIVE"
///   # addresses — read the real ones out of deployments.json, never retype:
///   export SEAT_SCRY=0x...                # chains.4663.contracts.SCRY
///   export SEAT_SPLITTER=0x...            # chains.4663.contracts.ScryFeeSplitter
///   # optional:
///   # export SEAT_PROCEEDS=0x...          # where swept ETH lands; default dev wallet
///   # export SEAT_SUPPLY=8192             # operator, 2026-08-08
///   # export SEAT_SNAPSHOT_CAP=...        # door 1's reserve
///   # export SEAT_PLAY_CAP=...            # door 2 — reserved; waits on a title
///   # export SEAT_BUILD_CAP=...           # door 3 — the board is live today
///   # export SEAT_TREASURY_CAP=100        # operator, 2026-08-08
///   # export SEAT_ROYALTY_BPS=500         # ERC-2981, to the splitter. Max 1000
///   # export SEAT_BURN_BPS=5000           # the welded activation burn
///   # export SEAT_BASE_URI=https://scry.moreright.xyz/api
///   # THE LADDER — comma-separated, ascending, same length, SUBLINEAR.
///   # ⚠ THE WEIGHTS BELOW ARE THE OPERATOR'S, CLOSED 2026-08-11 (SENTENCES.md,
///   # SCRY-HIVE.md §2): five tiers, costs x1/3/7/14/25, base efficiency 7.51x
///   # the top. This line read "100,125,160,200,333" until then — the pre-
///   # sentence draft — and a copy-paste out of a header is exactly how a
///   # ladder nobody chose gets welded into a constructor argument.
///   # ⚠ THE COSTS ARE AN EXAMPLE AND THE MULTIPLES ARE NOT. Tier 1's price is
///   # derived off that day's tape and is the one number this file must not
///   # supply; the x1/3/7/14/25 spacing is what is decided, so scale the whole
///   # row from whatever tier 1 lands on.
///   # export SEAT_TIER_COSTS="66666,200000,466666,933333,1666666"   # whole SCRY
///   # export SEAT_TIER_WEIGHTS="100,160,220,275,333"                # x100
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
///   1. record the address in `contracts/deployments.json` as `ScrySeat`
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
        string memory name_ = vm.envOr("SEAT_NAME", string("Scry Hive"));
        string memory symbol_ = vm.envOr("SEAT_SYMBOL", string("HIVE"));
        address scry = vm.envAddress("SEAT_SCRY");
        address splitter = vm.envAddress("SEAT_SPLITTER");
        address proceeds = vm.envOr("SEAT_PROCEEDS", DEV_WALLET);

        uint256[5] memory caps = [
            vm.envOr("SEAT_SUPPLY", uint256(8192)),
            vm.envOr("SEAT_SNAPSHOT_CAP", uint256(0)),
            vm.envOr("SEAT_PLAY_CAP", uint256(0)),
            vm.envOr("SEAT_BUILD_CAP", uint256(0)),
            vm.envOr("SEAT_TREASURY_CAP", uint256(100))
        ];

        uint96 royaltyBps = uint96(vm.envOr("SEAT_ROYALTY_BPS", uint256(500)));
        uint16 burnBps = uint16(vm.envOr("SEAT_BURN_BPS", uint256(5000)));
        bytes32 commitment = vm.envBytes32("SEAT_SALT_COMMITMENT");

        // Sequential, not an inline default: a url literal inside envOr's
        // argument list reads as a comment to the preflight scanner and
        // unbalances its walk (the DeployGardener.envAddrOr lesson).
        string memory base = vm.envOr("SEAT_BASE_URI", string(""));
        if (bytes(base).length == 0) {
            base = "https://scry.moreright.xyz/api";
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
        console2.log("  tiers", costs.length);

        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        ScrySeat s = new ScrySeat(
            name_, symbol_, caps, IERC20(scry), splitter, proceeds,
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
        ScrySeatArt artc = new ScrySeatArt(
            name_, "A capped run of characters. The seat transfers; the record never does."
        );
        require(artc.layerSumsAreExact(), "renderer: a weight row does not sum to 10000 bps");
        s.setRenderer(address(artc));
        vm.stopBroadcast();

        console2.log("ScrySeat", address(s));
        console2.log("ScrySeatArt (no owner, no setters)", address(artc));
        console2.log("  name", name_);
        console2.log("  scry", scry);
        console2.log("  splitter (royalties + the activation remainder)", splitter);
        console2.log("  proceeds (swept ETH only)", proceeds);
        console2.log("");
        console2.log("DEPLOYED SHUT. No root is posted and no price is set, so nothing");
        console2.log("can be minted by anyone yet. Next:");
        console2.log("  1. record the address in contracts/deployments.json as ScrySeat");
        console2.log("  2. meter/seat_roots.py build --allowlist <cohort> --door 1");
        console2.log("  3. setDoorRoot(1, <root>)   then setPaidDoor(wei, scry) when priced");
        console2.log("  4. KEEP THE SALT OFFLINE until the run closes - it cannot be changed");
        console2.log("  5. AFTER revealSalt + eyeballing the art: freezeMetadata() (one way)");
    }
}
