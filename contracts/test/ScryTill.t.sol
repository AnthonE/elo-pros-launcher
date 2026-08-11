// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryTill.sol";
import "../src/IERC20.sol";
import {MockToken} from "./MockToken.sol";

/// ScryTill — the game claim (`docs/money/CASHIER.md`).
///
/// What this suite is actually defending, in rough order of what it would cost
/// to get wrong:
///   1. the cumulative high-water mark — a replayed or stale draft must pay
///      NOTHING, by arithmetic rather than by a nonce map;
///   2. the daily cap, which is the only thing standing between a stolen
///      signing key and the float — including that it is paced on
///      `block.timestamp` and NOT on `block.number`, which on chain 4663 is the
///      parent chain's counter (`RUNBOOK.md` §0c). There is a test below that
///      rolls a million blocks and asserts the cap does NOT reset;
///   3. pay-the-wallet-never-the-submitter, without which gasless agents can
///      win and never collect;
///   4. the two delayed acts — raising the cap and sweeping the float — which
///      are the only ways the operator can take value back off a player.
contract ScryTillTest is Test {
    MockToken tok;
    ScryTill till;

    uint256 constant SIGNER_PK = 0x5163;
    uint256 constant WRONG_PK = 0xBAD;
    address signer;
    address alice = address(0xA11CE0);
    address bob = address(0xB0B0);
    address relayer = address(0x9E1A); // submits for others; must never be paid
    address treasury = address(0x7EA);

    uint256 constant CAP = 1_000e18;
    uint256 constant FLOAT = 100_000e18;

    function setUp() public {
        signer = vm.addr(SIGNER_PK);
        tok = new MockToken("Obol", "OBOL");
        till = new ScryTill(IERC20(address(tok)), signer, CAP);
        tok.mint(address(till), FLOAT);
        // a real timestamp: day 0 makes an off-by-one in the day index invisible
        vm.warp(1_800_000_000);
    }

    // ── helpers ─────────────────────────────────────────────────────────────
    function _draft(address who, uint256 cumulative) internal view returns (ScryTill.Draft memory d) {
        d = ScryTill.Draft({
            wallet: who,
            cumulative: cumulative,
            expiry: uint64(block.timestamp + 1 hours),
            epoch: till.signerEpoch()
        });
    }

    function _sign(ScryTill.Draft memory d, uint256 pk) internal view returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, _digest(d));
        return abi.encodePacked(r, s, v);
    }

    /// Rebuilt here from the EIP-712 pieces rather than read back off the
    /// contract: asking the contract for the digest it is about to verify would
    /// prove only that it agrees with itself.
    function _digest(ScryTill.Draft memory d) internal view returns (bytes32) {
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256("Draft(address wallet,uint256 cumulative,uint64 expiry,uint64 epoch)"),
                d.wallet,
                d.cumulative,
                d.expiry,
                d.epoch
            )
        );
        bytes32 domain = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("ScryTill"),
                keccak256("1"),
                block.chainid,
                address(till)
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domain, structHash));
    }

    function _pay(address who, uint256 cumulative) internal returns (uint256) {
        ScryTill.Draft memory d = _draft(who, cumulative);
        return till.withdraw(d, _sign(d, SIGNER_PK));
    }

    // ── the happy path, and who gets the money ──────────────────────────────
    function test_withdraw_pays_the_wallet_named_in_the_draft() public {
        assertEq(till.draftDigest(_draft(alice, 100e18)), _digest(_draft(alice, 100e18)), "digest dialect drift");
        uint256 paid = _pay(alice, 100e18);
        assertEq(paid, 100e18);
        assertEq(tok.balanceOf(alice), 100e18);
        assertEq(till.withdrawn(alice), 100e18);
        assertEq(till.totalWithdrawn(), 100e18);
    }

    /// The gasless-agent property: a familiar ships `wallet: null` and faucet
    /// cap "0", so somebody else has to push the transaction. If the till ever
    /// pays `msg.sender`, the lockout moves from the door to the cashier.
    function test_anyone_may_submit_for_anyone_and_the_submitter_is_never_paid() public {
        ScryTill.Draft memory d = _draft(alice, 250e18);
        bytes memory sig = _sign(d, SIGNER_PK);
        vm.prank(relayer);
        till.withdraw(d, sig);
        assertEq(tok.balanceOf(alice), 250e18, "the wallet in the draft is paid");
        assertEq(tok.balanceOf(relayer), 0, "the submitter is never paid");
    }

    // ── the high-water mark: what makes a nonce map unnecessary ─────────────
    function test_replaying_the_same_draft_pays_nothing() public {
        ScryTill.Draft memory d = _draft(alice, 100e18);
        bytes memory sig = _sign(d, SIGNER_PK);
        till.withdraw(d, sig);
        vm.expectRevert("nothing to withdraw");
        till.withdraw(d, sig);
        assertEq(tok.balanceOf(alice), 100e18, "paid once, not twice");
    }

    function test_a_later_draft_pays_only_the_difference() public {
        _pay(alice, 100e18);
        uint256 paid = _pay(alice, 175e18);
        assertEq(paid, 75e18, "the delta, not the whole cumulative");
        assertEq(tok.balanceOf(alice), 175e18);
    }

    /// Two drafts outstanding at once, submitted out of order. This is the case
    /// a per-withdrawal amount plus a nonce would either double-pay or wedge.
    function test_out_of_order_drafts_need_no_ordering() public {
        ScryTill.Draft memory small = _draft(alice, 100e18);
        ScryTill.Draft memory big = _draft(alice, 300e18);
        bytes memory sigSmall = _sign(small, SIGNER_PK);
        bytes memory sigBig = _sign(big, SIGNER_PK);

        till.withdraw(big, sigBig);
        assertEq(tok.balanceOf(alice), 300e18);
        vm.expectRevert("nothing to withdraw"); // the superseded one is inert
        till.withdraw(small, sigSmall);
        assertEq(tok.balanceOf(alice), 300e18, "the stale draft added nothing");
    }

    function test_a_lost_draft_costs_nothing_the_next_one_supersedes_it() public {
        _sign(_draft(alice, 50e18), SIGNER_PK); // written, never submitted
        assertEq(_pay(alice, 120e18), 120e18, "no nonce was consumed by the lost draft");
    }

    // ── what a draft must be to count ───────────────────────────────────────
    function test_expired_draft_is_refused() public {
        ScryTill.Draft memory d = _draft(alice, 100e18);
        bytes memory sig = _sign(d, SIGNER_PK);
        vm.warp(block.timestamp + 2 hours);
        vm.expectRevert("draft expired");
        till.withdraw(d, sig);
    }

    function test_a_draft_from_the_wrong_key_is_refused() public {
        ScryTill.Draft memory d = _draft(alice, 100e18);
        vm.expectRevert("bad signature");
        till.withdraw(d, _sign(d, WRONG_PK));
    }

    /// The leaked-key move: one `setSigner` voids every draft already written.
    function test_rotating_the_signer_voids_outstanding_drafts() public {
        ScryTill.Draft memory d = _draft(alice, 100e18);
        bytes memory sig = _sign(d, SIGNER_PK);
        till.setSigner(vm.addr(WRONG_PK));
        assertEq(till.signerEpoch(), 2);
        vm.expectRevert("stale signer epoch");
        till.withdraw(d, sig);
    }

    /// `ecrecover` accepts both s and n−s, so without the half-order check one
    /// draft has a second valid encoding.
    function test_a_malleable_signature_is_refused() public {
        ScryTill.Draft memory d = _draft(alice, 100e18);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(SIGNER_PK, _digest(d));
        uint256 N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141;
        bytes32 flipped = bytes32(N - uint256(s));
        uint8 flippedV = v == 27 ? 28 : 27;
        vm.expectRevert("bad signature");
        till.withdraw(d, abi.encodePacked(r, flipped, flippedV));
    }

    /// The domain separator carries `block.chainid` and the contract address,
    /// so a draft written for one deployment is not a draft for another — which
    /// matters the moment OBOL and MYRRH each get an instance.
    function test_a_draft_for_another_instance_does_not_verify_here() public {
        ScryTill other = new ScryTill(IERC20(address(tok)), signer, CAP);
        tok.mint(address(other), FLOAT);
        ScryTill.Draft memory d = _draft(alice, 100e18);
        bytes memory sigForThis = _sign(d, SIGNER_PK);
        vm.expectRevert("bad signature");
        other.withdraw(d, sigForThis);
    }

    // ── the daily cap ───────────────────────────────────────────────────────
    function test_cap_fills_partially_rather_than_refusing() public {
        uint256 paid = _pay(alice, CAP + 400e18);
        assertEq(paid, CAP, "paid what the day allowed");
        assertEq(till.withdrawn(alice), CAP, "the high-water advanced by what was PAID");
        assertEq(till.remainingToday(), 0);
    }

    function test_the_rest_is_still_owed_the_next_day() public {
        _pay(alice, CAP + 400e18);
        vm.warp(block.timestamp + 1 days);
        assertEq(till.remainingToday(), CAP, "a new day, a new cap");
        assertEq(_pay(alice, CAP + 400e18), 400e18, "the remainder, not the whole draft again");
        assertEq(tok.balanceOf(alice), CAP + 400e18);
    }

    function test_the_cap_is_shared_across_wallets_and_refuses_when_spent() public {
        _pay(alice, CAP);
        ScryTill.Draft memory d = _draft(bob, 10e18);
        vm.expectRevert("daily cap reached, try tomorrow");
        till.withdraw(d, _sign(d, SIGNER_PK));
    }

    /// ⚠ THE NITRO REGRESSION TEST. `block.number` on chain 4663 is the PARENT
    /// chain's counter, ~one per 12s, so a cap paced on it would reset roughly
    /// every twelve days while claiming to be daily. `ScryGacha` shipped that
    /// exact bug and its tests passed. Rolling a million blocks with the clock
    /// held still must change nothing.
    function test_the_cap_is_paced_on_the_timestamp_not_the_block_number() public {
        _pay(alice, CAP);
        assertEq(till.remainingToday(), 0);
        vm.roll(block.number + 1_000_000);
        assertEq(till.remainingToday(), 0, "block height must not reset the day");
        ScryTill.Draft memory d = _draft(bob, 10e18);
        vm.expectRevert("daily cap reached, try tomorrow");
        till.withdraw(d, _sign(d, SIGNER_PK));
    }

    // ── the two delayed acts ────────────────────────────────────────────────
    function test_lowering_the_cap_is_immediate() public {
        till.setDailyCap(10e18);
        assertEq(till.dailyCap(), 10e18);
        assertEq(_pay(alice, 500e18), 10e18, "the lower cap bound this withdrawal");
    }

    function test_raising_the_cap_waits_out_the_notice() public {
        till.setDailyCap(CAP * 10);
        assertEq(till.dailyCap(), CAP, "not raised yet");
        vm.expectRevert("not yet");
        till.applyCapRaise();
        vm.warp(block.timestamp + till.NOTICE());
        till.applyCapRaise();
        assertEq(till.dailyCap(), CAP * 10);
    }

    /// Lower-then-apply must not walk the cap back up through a raise announced
    /// before anyone had reason to watch for it.
    function test_lowering_cancels_a_pending_raise() public {
        till.setDailyCap(CAP * 10);
        till.setDailyCap(5e18);
        vm.warp(block.timestamp + till.NOTICE());
        vm.expectRevert("not yet");
        till.applyCapRaise();
        assertEq(till.dailyCap(), 5e18);
    }

    function test_sweep_needs_an_announcement_and_the_notice() public {
        vm.expectRevert("sweep not announced, or too soon");
        till.sweep(treasury, 1e18);
        till.announceSweep();
        vm.expectRevert("sweep not announced, or too soon");
        till.sweep(treasury, 1e18);
        vm.warp(block.timestamp + till.NOTICE());
        till.sweep(treasury, 1e18);
        assertEq(tok.balanceOf(treasury), 1e18);
    }

    /// One notice licenses ONE sweep. Otherwise an announcement made once and
    /// long forgotten licenses draining the float forever.
    function test_a_sweep_consumes_its_announcement() public {
        till.announceSweep();
        vm.warp(block.timestamp + till.NOTICE());
        till.sweep(treasury, 1e18);
        vm.expectRevert("sweep not announced, or too soon");
        till.sweep(treasury, 1e18);
    }

    function test_a_player_can_outrun_an_announced_sweep() public {
        till.announceSweep();
        vm.warp(block.timestamp + till.NOTICE() - 1);
        assertEq(_pay(alice, 100e18), 100e18, "the week is the player's, not the operator's");
    }

    // ── roles ───────────────────────────────────────────────────────────────
    function test_operator_handoff_is_two_step() public {
        till.transferOperator(bob);
        assertEq(till.operator(), address(this), "still ours until accepted");
        vm.prank(alice);
        vm.expectRevert("not pending operator");
        till.acceptOperator();
        vm.prank(bob);
        till.acceptOperator();
        assertEq(till.operator(), bob);
    }

    function test_only_the_operator_moves_the_knobs() public {
        vm.startPrank(alice);
        vm.expectRevert("not operator");
        till.setSigner(alice);
        vm.expectRevert("not operator");
        till.setDailyCap(1);
        vm.expectRevert("not operator");
        till.announceSweep();
        vm.stopPrank();
    }

    /// An empty float is an OPERATIONAL failure, not a posted policy, so it
    /// reverts rather than filling partially — paying less because the drawer
    /// is empty would hide it behind what looks like a cap.
    function test_an_underfunded_till_reverts_rather_than_paying_less() public {
        till.announceSweep();
        vm.warp(block.timestamp + till.NOTICE());
        till.sweep(treasury, FLOAT); // drain it
        ScryTill.Draft memory d = _draft(alice, 10e18);
        vm.expectRevert();
        till.withdraw(d, _sign(d, SIGNER_PK));
        assertEq(till.withdrawn(alice), 0, "a failed withdrawal advances nothing");
    }

    // ── the property the whole design rests on ──────────────────────────────
    /// However many drafts are written, in whatever order, at whatever
    /// cumulative values, a wallet is never paid more than the highest
    /// cumulative any valid draft named.
    function testFuzz_never_pays_more_than_the_highest_cumulative(uint96[8] calldata amounts) public {
        // Take the cap out of the way — through the notice, because raising it
        // is delayed by design and skipping that here silently left the cap at
        // 1000e18 and failed this test for the wrong reason.
        till.setDailyCap(type(uint256).max);
        vm.warp(block.timestamp + till.NOTICE());
        till.applyCapRaise();
        // fund past the top of the fuzz range: this test is about the
        // high-water mark, and an underfunded float would fail it for the
        // unrelated reason covered by `test_an_underfunded_till_reverts…`
        tok.mint(address(till), uint256(type(uint96).max));
        uint256 high;
        for (uint256 i = 0; i < amounts.length; i++) {
            uint256 c = uint256(amounts[i]);
            if (c == 0) continue;
            if (c > high) high = c;
            ScryTill.Draft memory d = _draft(alice, c);
            bytes memory sig = _sign(d, SIGNER_PK);
            try till.withdraw(d, sig) {} catch {}
        }
        assertEq(tok.balanceOf(alice), high, "paid exactly the high-water mark");
        assertEq(till.withdrawn(alice), high);
    }
}
