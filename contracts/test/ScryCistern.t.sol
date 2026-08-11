// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryCistern.sol";
import "../src/ScrySeat.sol";

contract MockCoin {
    string public name;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(string memory n) {
        name = n;
    }

    function mint(address to, uint256 a) external {
        balanceOf[to] += a;
    }

    function approve(address s, uint256 a) external returns (bool) {
        allowance[msg.sender][s] = a;
        return true;
    }

    function transfer(address t, uint256 a) external returns (bool) {
        require(balanceOf[msg.sender] >= a, "balance");
        balanceOf[msg.sender] -= a;
        balanceOf[t] += a;
        return true;
    }

    function transferFrom(address f, address t, uint256 a) external returns (bool) {
        require(balanceOf[f] >= a, "balance");
        require(allowance[f][msg.sender] >= a, "allowance");
        allowance[f][msg.sender] -= a;
        balanceOf[f] -= a;
        balanceOf[t] += a;
        return true;
    }
}

/// A router that actually moves tokens, at a posted rate, so a claim's coin leg
/// is exercised end to end rather than mocked away.
contract MockRouter {
    uint256 public rateNum = 2; // 1 SCRY -> 2 coin, by default
    uint256 public rateDen = 1;

    function setRate(uint256 n, uint256 d) external {
        rateNum = n;
        rateDen = d;
    }

    function exactInputSingle(ISwapRouter.ExactInputSingleParams calldata p) external returns (uint256 out) {
        MockCoin(p.tokenIn).transferFrom(msg.sender, address(this), p.amountIn);
        out = p.amountIn * rateNum / rateDen;
        require(out >= p.amountOutMinimum, "Too little received");
        MockCoin(p.tokenOut).mint(p.recipient, out);
    }
}

contract ScryCisternTest is Test {
    ScrySeat seat;
    ScryCistern cistern;
    MockCoin scry;
    MockCoin obol;
    MockCoin myrrh;
    MockRouter router;

    address splitter = address(0xFEE);
    address proceeds = address(0xB0B);
    address alice = address(0xA11CE);
    address bob = address(0xB0BB1E);
    address carol = address(0xCA401);

    uint256 constant THRESHOLD = 1_000e18;
    uint32 constant WINDOW = 7 days;
    string constant SALT = "cistern-salt";
    bytes32 saltCommit;

    function _ladder() internal pure returns (uint256[] memory costs, uint32[] memory weights) {
        costs = new uint256[](3);
        costs[0] = 100e18;
        costs[1] = 400e18;
        costs[2] = 2_500e18;
        weights = new uint32[](3);
        weights[0] = 100;
        weights[1] = 160;
        weights[2] = 300;
    }

    function setUp() public {
        saltCommit = sha256(bytes(SALT));
        scry = new MockCoin("SCRY");
        obol = new MockCoin("OBOL");
        myrrh = new MockCoin("MYRRH");
        router = new MockRouter();

        (uint256[] memory costs, uint32[] memory weights) = _ladder();
        seat = new ScrySeat(
            "scry seats",
            "SEAT",
            [uint256(100), uint256(0), uint256(0), uint256(0), uint256(0)],
            IERC20(address(scry)),
            splitter,
            proceeds,
            500,
            5000,
            saltCommit,
            costs,
            weights,
            "https://scry.moreright.xyz/api"
        );
        seat.setPaidDoor(1 ether, 0);

        cistern = new ScryCistern(
            IERC20(address(scry)), IScrySeat(address(seat)), ISwapRouter(address(router)), THRESHOLD, 100, WINDOW
        );
    }

    // ── helpers ──────────────────────────────────────────────────────────────
    function _seatFor(address who) internal returns (uint256 id) {
        vm.deal(who, 10 ether);
        vm.prank(who);
        id = seat.buyWithEth{value: 1 ether}();
    }

    function _activate(address who, uint256 id, uint8 tier) internal {
        (uint256[] memory costs,) = _ladder();
        scry.mint(who, costs[tier - 1]);
        vm.startPrank(who);
        scry.approve(address(seat), costs[tier - 1]);
        seat.activate(id, tier);
        vm.stopPrank();
    }

    function _fill(uint256 amount) internal {
        scry.mint(address(this), amount);
        scry.approve(address(cistern), amount);
        cistern.fill(amount);
    }

    function _zeros() internal pure returns (uint256[3] memory z) {
        return z;
    }

    /// Roll past the round's open so `tierSetAt < openedAt` holds for seats
    /// activated in the same block. Real usage never notices; a test does.
    ///
    /// ⚠ `vm.getBlockTimestamp()`, NEVER `block.timestamp`, and this is the
    /// trap that cost two tests here. Within one transaction `block.timestamp`
    /// CANNOT change, so solc is entitled to read `TIMESTAMP` once and reuse
    /// the value for the whole function — and under `via_ir` it does.
    /// `vm.warp` breaks that assumption because it mutates the clock mid-call,
    /// something no real EVM execution can do. The contract under test reads
    /// the true clock (its own call frame, its own read); the TEST keeps the
    /// stale one, so an assertion compares a fresh value against a cached one
    /// and prints an impossible pair like `101 != 1`. The cheatcode is an
    /// external call and can never be folded away.
    function _now() internal view returns (uint256) {
        return vm.getBlockTimestamp();
    }

    function _tick() internal {
        vm.warp(_now() + 1);
    }

    // ── the seat's side of the contract ══════════════════════════════════════

    function test_seat_total_weight_tracks_activation_and_clearing() public {
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        assertEq(seat.totalWeight(), 0, "benched seats weigh nothing");

        _activate(alice, a, 1);
        assertEq(seat.totalWeight(), 100);

        _activate(bob, b, 3);
        assertEq(seat.totalWeight(), 400);

        // an upgrade replaces, never adds
        _activate(alice, a, 2);
        assertEq(seat.totalWeight(), 460, "upgrade swaps 100 for 160");

        // a sale clears the tier and the weight with it
        vm.prank(alice);
        seat.transferFrom(alice, carol, a);
        assertEq(seat.totalWeight(), 300, "a sold seat leaves the roster");
        assertEq(seat.weightOf(a), 0);
    }

    function test_seat_records_when_the_tier_moved() public {
        uint256 a = _seatFor(alice);
        assertEq(seat.tierSetAt(a), 0);
        _activate(alice, a, 1);
        assertEq(seat.tierSetAt(a), uint64(_now()));

        vm.warp(_now() + 100);
        vm.prank(alice);
        seat.transferFrom(alice, bob, a);
        assertEq(seat.tierSetAt(a), uint64(_now()), "the clear is a tier move too");
    }

    // ── the bar ══════════════════════════════════════════════════════════════

    function test_the_bar_fills_from_a_bare_transfer() public {
        assertEq(cistern.pooled(), 0);
        // no integration, no approval — the fee splitter's path
        scry.mint(address(this), 250e18);
        scry.transfer(address(cistern), 250e18);
        assertEq(cistern.pooled(), 250e18);
        assertEq(cistern.fillBps(), 2500, "a page can draw the bar off one call");
    }

    function test_fill_bps_caps_at_full() public {
        _fill(THRESHOLD * 3);
        assertEq(cistern.fillBps(), 10_000);
    }

    function test_open_refuses_a_bar_that_is_not_full() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD - 1);
        vm.expectRevert("the bar is not full");
        cistern.open();
    }

    function test_open_refuses_an_empty_roster() public {
        _fill(THRESHOLD);
        vm.expectRevert("nobody is on the roster");
        cistern.open();
    }

    function test_anyone_opens_and_the_caller_keeps_the_tip() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();

        // carol holds no seat and has never touched this system
        vm.prank(carol);
        uint256 roundId = cistern.open();

        assertEq(roundId, 0);
        assertEq(scry.balanceOf(carol), THRESHOLD / 100, "1% tip to whoever pulled the lever");
        ScryCistern.Round memory r = cistern.rounds(0);
        assertEq(r.pot, THRESHOLD - THRESHOLD / 100);
        assertEq(r.totalWeight, 100);
        assertEq(cistern.pooled(), 0, "the pot moved into the round");
    }

    function test_the_house_cannot_lower_the_bar_to_time_a_buy() public {
        // The invariant-9 control: a threshold change takes KNOB_DELAY to land.
        _fill(THRESHOLD / 2);
        cistern.proposeThreshold(THRESHOLD / 2);
        assertEq(cistern.threshold(), THRESHOLD, "not yet");

        vm.expectRevert("still timelocked");
        cistern.commitThreshold();

        vm.warp(_now() + cistern.KNOB_DELAY() - 1);
        vm.expectRevert("still timelocked");
        cistern.commitThreshold();

        vm.warp(_now() + 2);
        // permissionless: a timelock only the proposer can execute is not one
        vm.prank(carol);
        cistern.commitThreshold();
        assertEq(cistern.threshold(), THRESHOLD / 2);
    }

    function test_routes_are_timelocked_too() public {
        cistern.proposeRoute(1, address(obol), 10_000);
        assertEq(cistern.coinOf(1), address(0));
        vm.expectRevert("still timelocked");
        cistern.commitRoute(1);
        vm.warp(_now() + cistern.KNOB_DELAY());
        cistern.commitRoute(1);
        assertEq(cistern.coinOf(1), address(obol));
        assertEq(cistern.feeOf(1), 10_000);
    }

    function test_track_zero_is_scry_and_cannot_be_repointed() public {
        vm.expectRevert("track 0 is SCRY");
        cistern.proposeRoute(0, address(obol), 10_000);
        vm.expectRevert("track 0 is SCRY");
        cistern.proposeRoute(2, address(scry), 10_000);
    }

    function test_tip_is_capped_in_bytecode() public {
        vm.expectRevert("tip too fat");
        cistern.setTip(501);
        cistern.setTip(500); // the ceiling itself is fine
        assertEq(cistern.tipBps(), 500);
    }

    function test_only_the_operator_holds_the_knobs() public {
        vm.startPrank(alice);
        vm.expectRevert("not operator");
        cistern.proposeThreshold(1);
        vm.expectRevert("not operator");
        cistern.proposeRoute(1, address(obol), 10_000);
        vm.expectRevert("not operator");
        cistern.setTip(10);
        vm.expectRevert("not operator");
        cistern.setRoundWindow(1);
        vm.stopPrank();
    }

    // ── the draw ═════════════════════════════════════════════════════════════

    function test_shares_are_tier_weighted() public {
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        _activate(alice, a, 1); // weight 100
        _activate(bob, b, 3); // weight 300
        _fill(THRESHOLD);
        _tick();
        cistern.open();

        ScryCistern.Round memory r = cistern.rounds(0);
        uint256 before_a = scry.balanceOf(alice);
        uint256 before_b = scry.balanceOf(bob);

        vm.prank(alice);
        cistern.claim(0, a, _zeros());
        vm.prank(bob);
        cistern.claim(0, b, _zeros());

        uint256 gotA = scry.balanceOf(alice) - before_a;
        uint256 gotB = scry.balanceOf(bob) - before_b;
        assertEq(gotA, r.pot * 100 / 400);
        assertEq(gotB, r.pot * 300 / 400);
        assertEq(gotA + gotB, r.pot, "the whole pot went out");
        // and the sublinear ladder is visible in the payout: bob paid 25x the
        // capital for 3x the share
        assertEq(gotB / gotA, 3);
    }

    function test_no_election_pays_scry_with_no_router_involved() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();

        vm.prank(alice);
        uint256 share = cistern.claim(0, a, _zeros());
        assertEq(scry.balanceOf(alice), share, "paid in SCRY, no swap, no route needed");
        assertEq(obol.balanceOf(alice), 0);
    }

    function test_an_election_buys_the_coins_at_market() public {
        cistern.proposeRoute(1, address(obol), 10_000);
        cistern.proposeRoute(2, address(myrrh), 10_000);
        vm.warp(_now() + cistern.KNOB_DELAY());
        cistern.commitRoute(1);
        cistern.commitRoute(2);

        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        vm.prank(alice);
        seat.setElection(a, [uint16(1), uint16(2), uint16(0)], [uint16(6000), uint16(4000), uint16(0)]);

        _fill(THRESHOLD);
        _tick();
        cistern.open();
        ScryCistern.Round memory r = cistern.rounds(0);

        vm.prank(alice);
        cistern.claim(0, a, _zeros());

        // sole seat: the whole pot, split 60/40 and swapped 1 -> 2
        assertEq(obol.balanceOf(alice), r.pot * 6000 / 10_000 * 2);
        assertEq(myrrh.balanceOf(alice), r.pot * 4000 / 10_000 * 2);
        assertEq(scry.balanceOf(alice), 0, "nothing came back as SCRY");
        assertEq(scry.balanceOf(address(cistern)), 0, "nothing stuck to the cistern");
    }

    function test_the_claimer_sets_their_own_slippage_bound() public {
        cistern.proposeRoute(1, address(obol), 10_000);
        vm.warp(_now() + cistern.KNOB_DELAY());
        cistern.commitRoute(1);

        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        vm.prank(alice);
        seat.setElection(a, [uint16(1), uint16(0), uint16(0)], [uint16(10_000), uint16(0), uint16(0)]);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        ScryCistern.Round memory r = cistern.rounds(0);

        uint256[3] memory tooHigh;
        tooHigh[0] = r.pot * 3; // demands 3x when the pool pays 2x
        vm.prank(alice);
        vm.expectRevert("Too little received");
        cistern.claim(0, a, tooHigh);

        uint256[3] memory ok;
        ok[0] = r.pot * 2;
        vm.prank(alice);
        cistern.claim(0, a, ok);
        assertEq(obol.balanceOf(alice), r.pot * 2);
    }

    function test_a_track_with_no_posted_route_fails_open_to_scry() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        // track 7 was never posted
        vm.prank(alice);
        seat.setElection(a, [uint16(7), uint16(0), uint16(0)], [uint16(10_000), uint16(0), uint16(0)]);
        _fill(THRESHOLD);
        _tick();
        cistern.open();

        vm.prank(alice);
        uint256 share = cistern.claim(0, a, _zeros());
        assertEq(scry.balanceOf(alice), share, "a claim is never blocked on an unposted pool");
    }

    function test_only_the_holder_claims() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();

        vm.prank(bob);
        vm.expectRevert("not your seat");
        cistern.claim(0, a, _zeros());
    }

    function test_a_seat_claims_a_round_once() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();

        vm.startPrank(alice);
        cistern.claim(0, a, _zeros());
        vm.expectRevert("already claimed");
        cistern.claim(0, a, _zeros());
        vm.stopPrank();
    }

    function test_a_benched_seat_draws_nothing() public {
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();

        vm.prank(bob);
        vm.expectRevert("seat is benched");
        cistern.claim(0, b, _zeros());
    }

    /// ⭐ THE ONE THAT PROTECTS EVERY OTHER HOLDER. Upgrading after a round
    /// opens must not mint a bigger share out of the round's fixed pot — the
    /// denominator was already fixed at `open()`.
    function test_a_tier_upgraded_after_the_open_sits_the_round_out() public {
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        _activate(alice, a, 1);
        _activate(bob, b, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open(); // totalWeight snapshotted at 200

        _tick();
        _activate(bob, b, 3); // bob jumps to weight 300 AFTER the snapshot

        vm.prank(bob);
        vm.expectRevert("tier moved after this round opened");
        cistern.claim(0, b, _zeros());

        // and alice is untouched: still exactly her half
        ScryCistern.Round memory r = cistern.rounds(0);
        vm.prank(alice);
        uint256 share = cistern.claim(0, a, _zeros());
        assertEq(share, r.pot * 100 / 200, "the denominator did not move under her");
    }

    function test_selling_a_seat_between_open_and_claim_costs_the_round() public {
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        _activate(alice, a, 1);
        _activate(bob, b, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        _tick();

        vm.prank(alice);
        seat.transferFrom(alice, carol, a); // clears the tier

        // ⚠ The clear IS a tier move, so `tierSetAt` catches it before the
        // weight check does — and that ordering is the useful one: it tells the
        // buyer the round predates them rather than that their seat is broken.
        vm.prank(carol);
        vm.expectRevert("tier moved after this round opened");
        cistern.claim(0, a, _zeros());

        // and re-tiering inside the same round does not buy back into it
        _activate(carol, a, 3);
        vm.prank(carol);
        vm.expectRevert("tier moved after this round opened");
        cistern.claim(0, a, _zeros());
    }

    function test_claims_never_exceed_the_round() public {
        // three seats, one never claims — the sum still cannot overdraw
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        uint256 c = _seatFor(carol);
        _activate(alice, a, 1);
        _activate(bob, b, 2);
        _activate(carol, c, 3);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        ScryCistern.Round memory r = cistern.rounds(0);

        vm.prank(alice);
        cistern.claim(0, a, _zeros());
        vm.prank(bob);
        cistern.claim(0, b, _zeros());

        ScryCistern.Round memory after_ = cistern.rounds(0);
        assertLe(after_.paid, r.pot);
        assertEq(scry.balanceOf(address(cistern)), cistern.outstanding(), "books match the balance");
    }

    // ── recycling ════════════════════════════════════════════════════════════

    function test_unclaimed_value_recycles_into_the_next_round() public {
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        _activate(alice, a, 1);
        _activate(bob, b, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        ScryCistern.Round memory r = cistern.rounds(0);

        vm.prank(alice);
        cistern.claim(0, a, _zeros()); // bob never claims

        vm.expectRevert("round still open");
        cistern.recycle(0);

        vm.warp(_now() + WINDOW);
        vm.prank(carol); // permissionless
        uint256 remainder = cistern.recycle(0);
        assertEq(remainder, r.pot - r.pot * 100 / 200);
        assertEq(cistern.pooled(), remainder, "it went back to the bar, not to an operator");

        vm.expectRevert("already recycled");
        cistern.recycle(0);
    }

    function test_a_closed_round_cannot_be_claimed() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        vm.warp(_now() + WINDOW);

        vm.prank(alice);
        vm.expectRevert("round window closed");
        cistern.claim(0, a, _zeros());
    }

    function test_rounds_stack_and_the_bar_refills() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        vm.prank(alice);
        cistern.claim(0, a, _zeros());

        _fill(THRESHOLD);
        _tick();
        cistern.open();
        assertEq(cistern.roundCount(), 2);
        vm.prank(alice);
        cistern.claim(1, a, _zeros());
    }

    // ── the round is a SNAPSHOT, in both of its terms ════════════════════════
    // ⚠ THE BUG THESE FOUR EXIST FOR. `claim` and `recycle` tested
    // `openedAt + roundWindow` against the CURRENT storage knob rather than the
    // round's own, so `setRoundWindow` — untimelocked, because it cannot time a
    // buy — reached backwards into every round already running. Nothing could
    // leave the contract, but an operator holding seats could cancel everybody
    // else's unclaimed share on demand and recapture a weighted slice of it in
    // the next round. `Round.totalWeight` had always been snapshotted for the
    // mirror-image reason; `Round.window` now is too.

    function test_shortening_the_window_cannot_close_a_round_already_open() public {
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        _activate(alice, a, 1);
        _activate(bob, b, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        assertEq(cistern.rounds(0).window, WINDOW, "the round carries the term it opened under");

        // alice takes hers; bob has not moved yet
        vm.prank(alice);
        cistern.claim(0, a, _zeros());

        cistern.setRoundWindow(1); // the operator slams it shut
        vm.warp(_now() + 2); // past the NEW window, nowhere near the round's own

        assertEq(cistern.roundWindow(), 1, "the knob did move");
        vm.expectRevert("round still open");
        cistern.recycle(0);

        (uint256 share, string memory why) = cistern.previewClaim(0, b);
        assertGt(share, 0, "and preview agrees the round is still live");
        assertEq(why, "");

        vm.prank(bob);
        uint256 got = cistern.claim(0, b, _zeros());
        assertGt(got, 0, "bob's share was never the operator's to cancel");
    }

    function test_lengthening_the_window_cannot_reopen_a_round_already_closed() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();

        vm.warp(_now() + WINDOW); // the round's own term is up
        cistern.setRoundWindow(365 days);

        vm.prank(alice);
        vm.expectRevert("round window closed");
        cistern.claim(0, a, _zeros());
        cistern.recycle(0); // and it is still recyclable, by anyone
    }

    function test_the_new_window_governs_the_NEXT_round() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        assertEq(cistern.rounds(0).window, WINDOW);

        cistern.setRoundWindow(1 days); // a real, forward-looking policy change
        vm.prank(alice);
        cistern.claim(0, a, _zeros());

        _fill(THRESHOLD);
        _tick();
        cistern.open();
        assertEq(cistern.rounds(1).window, 1 days, "round 1 opened under the new term");
        assertEq(cistern.rounds(0).window, WINDOW, "and round 0 kept its own, forever");

        vm.warp(_now() + 1 days);
        cistern.recycle(1); // round 1 closes on ITS window
    }

    // ── the wall: no admin path to anyone's money ════════════════════════════

    function test_there_is_no_rescue_pause_or_admin_withdrawal() public {
        _fill(THRESHOLD);
        uint256 held = scry.balanceOf(address(cistern));
        assertEq(held, THRESHOLD);

        // The operator's ENTIRE surface is the four knobs; none of them moves a
        // token. Asserted by absence — if a withdrawal is ever added, this is
        // the test that should stop it.
        cistern.setTip(200);
        cistern.setRoundWindow(1 days);
        cistern.proposeThreshold(1);
        cistern.proposeRoute(1, address(obol), 10_000);
        assertEq(scry.balanceOf(address(cistern)), held, "not one wei moved");
    }

    function test_renouncing_the_operator_freezes_the_knobs() public {
        cistern.transferOperator(address(0));
        vm.expectRevert("not operator");
        cistern.proposeThreshold(1);
        // and the machine still runs, because everything that matters is open
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        vm.prank(carol);
        cistern.open();
        vm.prank(alice);
        cistern.claim(0, a, _zeros());
    }

    function test_the_cistern_mints_nothing() public {
        // Every coin the seats receive came out of the pot that was filled.
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        uint256 supplyBefore = scry.balanceOf(address(cistern)) + scry.balanceOf(alice) + scry.balanceOf(address(this));
        _tick();
        cistern.open();
        vm.prank(alice);
        cistern.claim(0, a, _zeros());
        uint256 supplyAfter = scry.balanceOf(address(cistern)) + scry.balanceOf(alice) + scry.balanceOf(address(this));
        assertEq(supplyAfter, supplyBefore, "value was moved, never created");
    }

    // ── the preview a page draws ═════════════════════════════════════════════

    function test_preview_agrees_with_the_claim_and_says_why_not() public {
        uint256 a = _seatFor(alice);
        uint256 b = _seatFor(bob);
        _activate(alice, a, 1);
        _activate(bob, b, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();

        (uint256 share, string memory why) = cistern.previewClaim(0, a);
        assertGt(share, 0);
        assertEq(why, "");

        vm.prank(alice);
        uint256 actual = cistern.claim(0, a, _zeros());
        assertEq(actual, share, "the preview is the claim");

        (uint256 after_, string memory why2) = cistern.previewClaim(0, a);
        assertEq(after_, 0);
        assertEq(why2, "already claimed");

        (, string memory why3) = cistern.previewClaim(99, a);
        assertEq(why3, "no such round");
    }

    // ── RUNBOOK §0c ══════════════════════════════════════════════════════════

    function test_nothing_here_is_paced_in_block_number() public {
        uint256 a = _seatFor(alice);
        _activate(alice, a, 1);
        _fill(THRESHOLD);
        _tick();
        cistern.open();
        vm.roll(block.number + 1_000_000); // the parent chain's clock, ~100x slow
        vm.prank(alice);
        cistern.claim(0, a, _zeros()); // still inside the window, which is a timestamp
    }
}
