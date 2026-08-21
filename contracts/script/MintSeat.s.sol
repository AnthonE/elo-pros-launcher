// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloSeat.sol";

/// Mint seats to named wallets through the treasury door — the operator's own
/// mint, run by the operator.
///
/// WHAT THIS IS FOR. `EloSeat` deploys with every door SHUT: no root is posted
/// and no price is set, so `claim` and `buyWithEth` both refuse and there is no
/// way for anyone — including the owner — to walk a seat out of doors 1–4. The
/// fifth door is the only one open on day zero, and this script is it. Its two
/// real uses are the team allocation and putting real seats in real wallets so
/// the mint surface can be exercised against a live collection instead of a
/// mock.
///
/// WHY IT IS A SCRIPT AND NOT A PASTED `cast send`. An address typed once into a
/// shell is a seat minted to a wallet nobody holds, and nothing burns a seat —
/// `EloSeat` has no burn, so a fat-fingered recipient is permanent and it has
/// spent an allocation bounded by an immutable cap. So the recipients are
/// parsed with named errors, echoed back before the broadcast, and every count
/// is read off the chain afterwards rather than assumed.
///
///   export PRIVATE_KEY=0x...            # MUST be the collection's owner()
///   export SEAT=0x...                   # the deployed EloSeat
///   export SEAT_MINT_TO=0xaaa...,0xbbb...   # recipients, comma separated
///   # export SEAT_MINT_EACH=1           # seats per recipient; default 1
///   forge script script/MintSeat.s.sol --rpc-url $RPC            # DRY RUN
///   forge script script/MintSeat.s.sol --rpc-url $RPC --broadcast
///
/// ⚠ RUN THE DRY RUN FIRST AND READ THE ECHOED ADDRESSES. It performs every
/// check this file has against real chain state and prints the recipient list
/// parsed rather than as typed, which is the only place a transposed character
/// is still free to catch.
///
/// WHAT A SEAT MINTED HERE IS, and it is not the same object as a claimed one:
///   · door 5 — `doorOf` reads 5, the treasury door, distinct from the four
///     public ones on purpose so a team seat is never miscounted as a cohort's
///   · tier 0 — BENCHED. `mintTreasury` does not activate, and it could not:
///     `activate` is the holder's own call and is paid in SCRY by whoever holds
///     the seat. A seat minted here weighs nothing until its holder sits it
///   · counted — `treasuryMinted` goes up and is public, which is the whole
///     reason a capped team mint is a different object from an owner who mints
///     at will
///
/// WHAT THIS SCRIPT CANNOT DO: post a root, set a price, open a door, move the
/// cap, or reach past `treasuryCap`. It mints inside an allocation that was
/// public before mint #1.
contract MintSeat is Script {
    /// A comma-separated env list -> address[]. Written out rather than using
    /// `vm.envAddress(name, ",")` for the reason `DeploySeat._split` gives about
    /// the ladder: the cheatcode reverts with an unreadable message on a
    /// trailing comma or a stray space, and this is an input where a typo does
    /// not fail the run — it succeeds against the wrong wallet, permanently.
    function _addrs(string memory csv) internal pure returns (address[] memory out) {
        bytes memory b = bytes(csv);
        uint256 n = b.length == 0 ? 0 : 1;
        for (uint256 i = 0; i < b.length; i++) {
            if (b[i] == ",") n++;
        }
        out = new address[](n);

        uint256 k;
        uint160 acc;
        uint256 digits;
        bool sawPrefix;
        for (uint256 i = 0; i <= b.length; i++) {
            if (i == b.length || b[i] == ",") {
                require(sawPrefix, "SEAT_MINT_TO: an entry does not start with 0x");
                require(digits == 40, "SEAT_MINT_TO: an entry is not 40 hex digits after 0x");
                out[k++] = address(acc);
                acc = 0;
                digits = 0;
                sawPrefix = false;
                continue;
            }
            bytes1 c = b[i];
            if (c == " ") continue;
            if (!sawPrefix) {
                require(c == "0", "SEAT_MINT_TO: an entry does not start with 0x");
                require(i + 1 < b.length && (b[i + 1] == "x" || b[i + 1] == "X"),
                    "SEAT_MINT_TO: an entry does not start with 0x");
                sawPrefix = true;
                i++; // step past the x
                continue;
            }
            uint8 v;
            if (c >= "0" && c <= "9") v = uint8(c) - 48;
            else if (c >= "a" && c <= "f") v = uint8(c) - 87;
            else if (c >= "A" && c <= "F") v = uint8(c) - 55;
            else revert("SEAT_MINT_TO: not a hex digit");
            require(digits < 40, "SEAT_MINT_TO: an entry is longer than 40 hex digits");
            acc = acc * 16 + v;
            digits++;
        }
    }

    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address me = vm.addr(pk);
        EloSeat seat = EloSeat(payable(vm.envAddress("SEAT")));
        address[] memory to = _addrs(vm.envString("SEAT_MINT_TO"));
        uint256 each = vm.envOr("SEAT_MINT_EACH", uint256(1));

        require(to.length > 0, "SEAT_MINT_TO is empty - name at least one recipient");
        require(each > 0, "SEAT_MINT_EACH must be > 0");

        // Refuse rather than guess. Each of these reverts inside `mintTreasury`
        // anyway; a named refusal off a dry run beats a revert nobody can read
        // with gas already spent.
        require(seat.owner() == me, "this key is not the collection's owner()");
        require(!seat.mintClosed(), "the mint is closed - no door opens again, including this one");

        uint256 want = to.length * each;
        uint256 minted = seat.treasuryMinted();
        uint256 cap = seat.treasuryCap();
        require(cap > 0,
            "treasuryCap is 0 - this collection was deployed with NO team allocation, and "
            "the cap is immutable. There is no treasury mint on this contract");
        require(minted + want <= cap, "not enough treasury allocation left for this list");
        require(seat.totalMinted() + want <= seat.maxSupply(), "not enough supply left");

        uint256 firstId = seat.totalMinted() + 1;

        console2.log("collection   ", address(seat));
        console2.log("  name       ", seat.name());
        console2.log("owner (this key)", me);
        console2.log("treasury cap ", cap);
        console2.log("  minted so far", minted);
        console2.log("  this run     ", want);
        console2.log("  left after   ", cap - minted - want);
        console2.log("");
        console2.log("recipients - PARSED, not as typed. Read them:");
        for (uint256 i = 0; i < to.length; i++) {
            require(to[i] != address(0), "SEAT_MINT_TO: the zero address");
            // Nothing burns a seat, so a duplicate is not an error — it is a
            // wallet getting two. Said out loud because it is far more often a
            // pasted line than an intention.
            for (uint256 j = 0; j < i; j++) {
                if (to[j] == to[i]) console2.log("  (repeat of an earlier entry)");
            }
            console2.log("  ", to[i], seat.balanceOf(to[i]));
        }
        console2.log("(the number beside each is what it holds NOW, before this run)");
        console2.log("");
        console2.log("ids this run will hand out:", firstId, "..", firstId + want - 1);
        console2.log("every one at door 5, tier 0 - BENCHED until its holder activates it");

        vm.startBroadcast(pk);
        for (uint256 i = 0; i < to.length; i++) {
            seat.mintTreasury(to[i], each);
        }
        vm.stopBroadcast();

        // Read it BACK off the chain rather than echoing the input — the same
        // discipline `GrantCashier` ends on, and the only proof the seats landed
        // where the list said.
        console2.log("");
        console2.log("read back off chain:");
        for (uint256 i = 0; i < to.length; i++) {
            console2.log("  ", to[i], seat.balanceOf(to[i]));
        }
        uint256 after_ = seat.treasuryMinted();
        console2.log("treasuryMinted", minted, "->", after_);
        require(after_ == minted + want, "read-back disagrees with what was asked for");

        // The ids are sequential because `_mint` hands them out in order, so
        // this is derived rather than guessed — and it is what to paste into a
        // marketplace or into /api/seat/<id> to see the row.
        for (uint256 i = 0; i < want; i++) {
            uint256 id = firstId + i;
            require(seat.doorOf(id) == 5, "a minted seat did not land at the treasury door");
            console2.log("  seat", id, "->", seat.ownerOf(id));
        }

        console2.log("");
        console2.log("Next: GET /api/seat/gallery and /api/seat/of/<wallet> now answer with");
        console2.log("      these rows. The faces are the SEALED card until revealSalt().");
    }
}
