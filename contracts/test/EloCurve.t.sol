// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloCurve.sol";
import "./MockToken.sol";
import "./MockUniswapV3.sol";

/// The launch curve: the two-way venue that exists before there is a pool.
///
/// What these pin, by section:
///   - THE claim the reversal was for: you can SELL BACK from the first trade,
///     and you can still sell after the curve closes
///   - the seed comes from BUYS — no SCRY is required to open the curve, and
///     the pool's quote side is exactly what buyers put in
///   - the exits are the three named in the header and there is no fourth: no
///     withdraw, no rescue, no admin, no owner
///   - the curve is a market in what it SOLD, not a bid for the whole supply
///   - graduation takes no price argument, burns what it did not use, and the
///     position it mints cannot be moved
///
/// ⚠ These run against MOCK v3. They prove this contract's OWN logic and
/// nothing about real Uniswap — tick math, the ratio a real mint consumes, what
/// `collect` returns on a position that never decreased liquidity. A fork test
/// against the canonical addresses is owed before any broadcast, exactly as it
/// is for `EloLaunchpad` (`LAUNCHPAD.md` §gates).
contract EloCurveTest is Test {
    EloCurve curve;
    MockToken coin;
    MockToken reserve;
    MockV3Factory factory;
    MockV3Pool pool;
    MockPositionManager npm;

    address creatorFees = address(0xFEE1);
    address protocolFees = address(0xFEE2);
    address quotePoolFees = address(0xFEE3);
    address coinPoolFees = address(0xFEE4);

    address alice = address(0xA11CE);
    /// The wallet that pushes the curve to its threshold, so a test can hold a
    /// balance of its own across the fill and still assert on it afterwards.
    address filler = address(0xF111E5);
    address bob = address(0xB0B);

    uint24 constant POOL_FEE = 10000;
    uint256 constant VIRTUAL_QUOTE = 100_000e18;
    uint256 constant CURVE_ALLOC = 700_000_000e18;
    uint256 constant POOL_ALLOC = 200_000_000e18;
    uint256 constant THRESHOLD = 400_000e18;
    uint16 constant FEE_BPS = 100; // 1%
    // A FIXTURE, NOT THE POSTED RATE. The posted default is 10%
    // (`curve.env.example`); this stays at a less round 30% because what
    // `payFees` owes is the arithmetic at whatever share it was built
    // with, and a test that re-asserts the policy number proves less.
    uint16 constant PROTOCOL_SHARE = 3000; // 30% of the fee

    function setUp() public {
        coin = new MockToken("Gates Coin", "GATE");
        reserve = new MockToken("Scry", "SCRY");
        factory = new MockV3Factory();
        pool = new MockV3Pool();
        npm = new MockPositionManager(address(factory), pool);

        curve = new EloCurve(
            IERC20(address(coin)),
            IERC20(address(reserve)),
            ICurvePositionManager(address(npm)),
            POOL_FEE,
            VIRTUAL_QUOTE,
            CURVE_ALLOC,
            POOL_ALLOC,
            THRESHOLD,
            FEE_BPS,
            PROTOCOL_SHARE,
            creatorFees,
            protocolFees,
            quotePoolFees,
            coinPoolFees
        );

        (address t0, address t1) =
            address(reserve) < address(coin) ? (address(reserve), address(coin)) : (address(coin), address(reserve));
        npm.setTokens(t0, t1);
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    function _fundAndArm() internal {
        coin.mint(address(curve), CURVE_ALLOC + POOL_ALLOC);
        curve.arm();
    }

    function _buy(address who, uint256 amount) internal returns (uint256 out) {
        reserve.mint(who, amount);
        vm.startPrank(who);
        reserve.approve(address(curve), amount);
        out = curve.buy(amount, 0, block.timestamp + 1);
        vm.stopPrank();
    }

    function _sell(address who, uint256 amount) internal returns (uint256 out) {
        vm.startPrank(who);
        coin.approve(address(curve), amount);
        out = curve.sell(amount, 0, block.timestamp + 1);
        vm.stopPrank();
    }

    /// Buy in chunks until the curve closes, so the threshold is crossed the
    /// way a real launch crosses it rather than in one whale-shaped call.
    function _fillToThreshold() internal {
        uint256 guard;
        while (!curve.closed()) {
            _buy(filler, 50_000e18);
            guard++;
            require(guard < 50, "did not close");
        }
    }

    // ── the coin is funded, never minted ─────────────────────────────────────

    function test_it_refuses_to_arm_before_the_coin_arrives() public {
        vm.expectRevert(bytes("the coin side is not funded"));
        curve.arm();
    }

    function test_it_refuses_to_arm_on_a_partial_transfer() public {
        coin.mint(address(curve), CURVE_ALLOC + POOL_ALLOC - 1);
        vm.expectRevert(bytes("the coin side is not funded"));
        curve.arm();
    }

    function test_anyone_may_arm_because_it_grants_nothing() public {
        coin.mint(address(curve), CURVE_ALLOC + POOL_ALLOC);
        vm.prank(bob);
        curve.arm();
        assertTrue(curve.armed(), "a stranger may start a funded launch");
        assertEq(curve.coinReserve(), CURVE_ALLOC, "only the curve's allocation is sellable");
    }

    function test_the_pool_allocation_is_never_sellable_on_the_curve() public {
        _fundAndArm();
        // Everything a buyer can ever take out is bounded by curveAlloc, so the
        // held-back share cannot be drawn down no matter how much is spent.
        _buy(alice, 10_000_000e18);
        assertLe(CURVE_ALLOC - curve.coinReserve(), CURVE_ALLOC, "cannot sell past the allocation");
        assertGe(coin.balanceOf(address(curve)), POOL_ALLOC, "the pool side is still here");
    }

    // ── the seed comes from buys ─────────────────────────────────────────────

    function test_the_curve_opens_with_no_scry_in_it_at_all() public {
        _fundAndArm();
        assertEq(reserve.balanceOf(address(curve)), 0, "no quote is required to open");
        assertGt(curve.spotPrice(), 0, "and it still quotes a price");
    }

    function test_every_wei_of_the_pools_quote_side_came_from_buyers() public {
        _fundAndArm();
        _fillToThreshold();
        uint256 raised = curve.quoteReserve();
        curve.graduate(block.timestamp + 1);

        (address t0,) = address(reserve) < address(coin) ? (address(reserve), address(coin)) : (address(coin), address(reserve));
        uint256 quoteIntoPool = t0 == address(reserve) ? npm.totalAmount0() : npm.totalAmount1();
        assertEq(quoteIntoPool, raised, "the pool's quote side is exactly what buyers paid in");
    }

    // ── you can always sell back ─────────────────────────────────────────────

    function test_a_buyer_can_sell_back_on_the_very_next_block() public {
        _fundAndArm();
        uint256 got = _buy(alice, 10_000e18);
        assertGt(got, 0, "bought something");

        uint256 before = reserve.balanceOf(alice);
        uint256 back = _sell(alice, got);
        assertGt(back, 0, "the curve bid for it");
        assertEq(reserve.balanceOf(alice), before + back, "and paid in reserve");
    }

    function test_selling_back_immediately_costs_only_the_two_fees() public {
        _fundAndArm();
        uint256 spent = 10_000e18;
        uint256 got = _buy(alice, spent);
        uint256 back = _sell(alice, got);

        // Two 1% fees and the curve's own rounding. Nothing else may be taken.
        assertLt(back, spent, "a round trip is not free");
        assertGt(back, (spent * 9700) / 10_000, "and it costs about the two fees, not more");
    }

    function test_selling_still_works_after_the_curve_closes() public {
        _fundAndArm();
        uint256 got = _buy(alice, 20_000e18);
        _fillToThreshold();
        assertTrue(curve.closed(), "closed");

        vm.expectRevert(bytes("the curve is closed"));
        vm.prank(bob);
        curve.buy(1e18, 0, block.timestamp + 1);

        uint256 back = _sell(alice, got);
        assertGt(back, 0, "but the exit is still open right up to graduation");
    }

    function test_selling_stops_only_once_a_real_pool_exists() public {
        _fundAndArm();
        uint256 got = _buy(alice, 20_000e18);
        _fillToThreshold();
        curve.graduate(block.timestamp + 1);

        vm.startPrank(alice);
        coin.approve(address(curve), got);
        vm.expectRevert(bytes("graduated - trade the pool"));
        curve.sell(got, 0, block.timestamp + 1);
        vm.stopPrank();
    }

    // ── the curve is a market in what it sold ────────────────────────────────

    function test_it_refuses_coin_the_curve_never_sold() public {
        _fundAndArm();
        _buy(alice, 10_000e18);

        // bob holds coin from somewhere else — a grant reserve, an airdrop, the
        // studio's own bag — and tries to draw down a reserve he never funded.
        coin.mint(bob, CURVE_ALLOC);
        vm.startPrank(bob);
        coin.approve(address(curve), CURVE_ALLOC);
        vm.expectRevert(bytes("more coin than the curve sold"));
        curve.sell(CURVE_ALLOC, 0, block.timestamp + 1);
        vm.stopPrank();
    }

    function test_the_reserve_cannot_be_drained_below_zero_by_selling() public {
        _fundAndArm();
        uint256 a = _buy(alice, 30_000e18);
        uint256 b = _buy(bob, 30_000e18);

        _sell(bob, b);
        _sell(alice, a);
        // Everyone out. What is left is the fees, and the reserve is flat to
        // within floor dust: `sell` computes `gross` through a flooring
        // `mulDiv`, so each exit leaves under a wei behind instead of paying it
        // out. The rounding runs TOWARD the reserve, which is what makes "never
        // past zero" structural rather than lucky. Two sells, so at most two wei.
        assertLe(curve.quoteReserve(), 2, "the last seller out empties the reserve, never past it");
        assertEq(curve.coinReserve(), CURVE_ALLOC, "and every coin is back on the curve");
    }

    // ── price ────────────────────────────────────────────────────────────────

    function test_the_price_rises_on_a_buy_and_falls_on_a_sell() public {
        _fundAndArm();
        uint256 opening = curve.spotPrice();
        uint256 got = _buy(alice, 50_000e18);
        uint256 afterBuy = curve.spotPrice();
        assertGt(afterBuy, opening, "buying raises it");

        _sell(alice, got);
        assertLt(curve.spotPrice(), afterBuy, "selling lowers it");
    }

    function test_a_later_buyer_pays_more_than_an_earlier_one() public {
        _fundAndArm();
        uint256 first = _buy(alice, 25_000e18);
        uint256 second = _buy(bob, 25_000e18);
        assertLt(second, first, "the same money buys less after the price moved");
    }

    function test_preview_matches_what_a_buy_actually_returns() public {
        _fundAndArm();
        (uint256 previewed,) = curve.previewBuy(30_000e18);
        uint256 actual = _buy(alice, 30_000e18);
        assertEq(actual, previewed, "a card may quote this");
    }

    function test_preview_sell_names_the_wall_instead_of_returning_zero() public {
        _fundAndArm();
        coin.mint(bob, 10e18);
        (,, string memory why) = curve.previewSell(10e18);
        assertEq(why, "more coin than the curve sold", "a surface can say which wall it hit");
    }

    // ── graduation ───────────────────────────────────────────────────────────

    function test_it_will_not_graduate_before_the_threshold() public {
        _fundAndArm();
        _buy(alice, 10_000e18);
        vm.expectRevert(bytes("not closed"));
        curve.graduate(block.timestamp + 1);
    }

    function test_anyone_may_graduate_and_it_takes_no_price() public {
        _fundAndArm();
        _fillToThreshold();
        vm.prank(bob);
        (uint256 tokenId,) = curve.graduate(block.timestamp + 1);
        assertGt(tokenId, 0, "a stranger closed the launch");
        assertEq(curve.positionId(), tokenId, "and the position is this contract's");
    }

    function test_graduating_twice_is_refused() public {
        _fundAndArm();
        _fillToThreshold();
        curve.graduate(block.timestamp + 1);
        vm.expectRevert(bytes("already graduated"));
        curve.graduate(block.timestamp + 1);
    }

    function test_what_the_pool_did_not_take_is_burned() public {
        _fundAndArm();
        _fillToThreshold();
        uint256 leftOnCurve = curve.coinReserve();
        // a real mint consumes at the pool's ratio, not everything offered
        npm.setConsumeBps(8_000);
        curve.graduate(block.timestamp + 1);

        uint256 expected = leftOnCurve + (POOL_ALLOC - (POOL_ALLOC * 8_000) / 10_000);
        assertEq(curve.coinBurnedAtGraduation(), expected, "unsold coin and unused pool coin both burn");
        assertEq(coin.balanceOf(0x000000000000000000000000000000000000dEaD), expected, "to the dead address");
        assertEq(curve.coinReserve(), 0, "and nothing reads as a live market afterwards");
    }

    function test_graduation_raises_the_pools_observation_cardinality() public {
        _fundAndArm();
        _fillToThreshold();
        curve.graduate(block.timestamp + 1);
        assertEq(pool.cardinalityNext(), 16, "so a TWAP exists while the pool is youngest");
    }

    function test_the_two_prices_are_published_before_anyone_deploys() public view {
        (uint256 closePrice, uint256 openPrice, uint256 coinLeft) = curve.graduationPreview();
        assertGt(closePrice, 0, "the curve's closing price is computable at deploy time");
        assertGt(openPrice, 0, "so is the pool's opening price");
        assertGt(coinLeft, 0, "and what would be left on the curve");
    }

    // ── fees ─────────────────────────────────────────────────────────────────

    function test_trading_fees_split_by_the_posted_share() public {
        _fundAndArm();
        _buy(alice, 100_000e18);
        uint256 owed = curve.feesAccrued();
        assertEq(owed, (100_000e18 * uint256(FEE_BPS)) / 10_000, "1% of the buy");

        vm.prank(bob);
        curve.payFees();
        assertEq(reserve.balanceOf(protocolFees), (owed * PROTOCOL_SHARE) / 10_000, "the protocol's share");
        assertEq(reserve.balanceOf(creatorFees), owed - (owed * PROTOCOL_SHARE) / 10_000, "the creator takes the rest");
        assertEq(curve.feesAccrued(), 0, "and the ledger zeroes");
    }

    function test_fees_are_held_apart_from_the_reserve() public {
        _fundAndArm();
        _buy(alice, 100_000e18);
        uint256 fee = (100_000e18 * uint256(FEE_BPS)) / 10_000;
        assertEq(curve.quoteReserve(), 100_000e18 - fee, "the reserve is net of the fee");
        assertEq(reserve.balanceOf(address(curve)), 100_000e18, "both sit here, counted separately");
    }

    function test_paying_fees_does_not_touch_the_reserve() public {
        _fundAndArm();
        _buy(alice, 100_000e18);
        uint256 reserve = curve.quoteReserve();
        curve.payFees();
        assertEq(curve.quoteReserve(), reserve, "the raise is not a fee and never pays like one");
    }

    function test_pool_fees_go_to_the_two_welded_recipients() public {
        _fundAndArm();
        _fillToThreshold();
        curve.graduate(block.timestamp + 1);

        npm.setFeesOwed(5e18, 7e18);
        vm.prank(bob);
        curve.collectPoolFees();

        (address t0,) = address(reserve) < address(coin) ? (address(reserve), address(coin)) : (address(coin), address(reserve));
        (uint256 quoteOwed, uint256 coinOwed) = t0 == address(reserve) ? (uint256(5e18), uint256(7e18)) : (uint256(7e18), uint256(5e18));
        assertEq(reserve.balanceOf(quotePoolFees), quoteOwed, "the quote leg");
        assertEq(coin.balanceOf(coinPoolFees), coinOwed, "and the coin leg, to a different address");
    }

    function test_collecting_pool_fees_before_graduation_is_refused() public {
        _fundAndArm();
        _buy(alice, 10_000e18);
        vm.expectRevert(bytes("not graduated"));
        curve.collectPoolFees();
    }

    // ── the exits, and there is no fourth ────────────────────────────────────

    /// The test that should fail if anyone ever adds a way out. It asserts the
    /// ABSENCE directly: no selector on this contract moves the reserve
    /// anywhere but a seller, the position, or the two fee addresses.
    function test_there_is_no_withdraw_rescue_or_sweep() public view {
        string[6] memory forbidden =
            ["withdraw()", "rescue(address,uint256)", "sweep()", "sweep(address,uint256)", "emergencyWithdraw()", "renounceOwnership()"];
        for (uint256 i = 0; i < forbidden.length; i++) {
            bytes4 sel = bytes4(keccak256(bytes(forbidden[i])));
            (bool ok,) = address(curve).staticcall(abi.encodePacked(sel));
            assertFalse(ok, forbidden[i]);
        }
    }

    function test_nobody_owns_this_contract() public view {
        (bool ok,) = address(curve).staticcall(abi.encodeWithSignature("owner()"));
        assertFalse(ok, "there is no owner to ask");
        (bool ok2,) = address(curve).staticcall(abi.encodeWithSignature("steward()"));
        assertFalse(ok2, "and no steward either - every knob is welded");
    }

    function test_a_donation_cannot_move_the_price() public {
        _fundAndArm();
        _buy(alice, 20_000e18);
        uint256 before = curve.spotPrice();

        reserve.mint(address(curve), 500_000e18); // a gift, straight to the balance
        assertEq(curve.spotPrice(), before, "accounting is tracked, never read off balanceOf");
        assertEq(curve.quoteReserve(), curve.quoteReserve(), "and the reserve is untouched");
    }

    // ── the wiring refuses to be built wrong ─────────────────────────────────

    function test_a_coin_cannot_quote_itself() public {
        vm.expectRevert(bytes("the coin cannot quote itself"));
        new EloCurve(
            IERC20(address(coin)), IERC20(address(coin)), ICurvePositionManager(address(npm)), POOL_FEE,
            VIRTUAL_QUOTE, CURVE_ALLOC, POOL_ALLOC, THRESHOLD, FEE_BPS, PROTOCOL_SHARE,
            creatorFees, protocolFees, quotePoolFees, coinPoolFees
        );
    }

    function test_an_unknown_fee_tier_refuses_rather_than_minting_a_bad_range() public {
        vm.expectRevert(bytes("unknown fee tier"));
        new EloCurve(
            IERC20(address(coin)), IERC20(address(reserve)), ICurvePositionManager(address(npm)), uint24(1234),
            VIRTUAL_QUOTE, CURVE_ALLOC, POOL_ALLOC, THRESHOLD, FEE_BPS, PROTOCOL_SHARE,
            creatorFees, protocolFees, quotePoolFees, coinPoolFees
        );
    }

    function test_a_fee_a_seller_could_not_survive_is_refused() public {
        vm.expectRevert(bytes("fee above 10%"));
        new EloCurve(
            IERC20(address(coin)), IERC20(address(reserve)), ICurvePositionManager(address(npm)), POOL_FEE,
            VIRTUAL_QUOTE, CURVE_ALLOC, POOL_ALLOC, THRESHOLD, uint16(1_001), PROTOCOL_SHARE,
            creatorFees, protocolFees, quotePoolFees, coinPoolFees
        );
    }

    function test_a_launch_with_no_held_back_share_is_refused() public {
        vm.expectRevert(bytes("both allocations"));
        new EloCurve(
            IERC20(address(coin)), IERC20(address(reserve)), ICurvePositionManager(address(npm)), POOL_FEE,
            VIRTUAL_QUOTE, CURVE_ALLOC, 0, THRESHOLD, FEE_BPS, PROTOCOL_SHARE,
            creatorFees, protocolFees, quotePoolFees, coinPoolFees
        );
    }

    function test_a_zero_virtual_reserve_is_refused_because_it_has_no_opening_price() public {
        vm.expectRevert(bytes("zero virtual quote"));
        new EloCurve(
            IERC20(address(coin)), IERC20(address(reserve)), ICurvePositionManager(address(npm)), POOL_FEE,
            0, CURVE_ALLOC, POOL_ALLOC, THRESHOLD, FEE_BPS, PROTOCOL_SHARE,
            creatorFees, protocolFees, quotePoolFees, coinPoolFees
        );
    }

    function test_the_range_is_full_and_aligned_to_the_tiers_spacing() public view {
        // ⚠ A LITERAL EXPRESSION IS FOLDED AS AN EXACT RATIONAL, NOT AS INTEGER
        // DIVISION. `(887272 / 200) * 200` is 887272 to solc — the truncation the
        // constructor performs happens only because it divides through an `int24`
        // VARIABLE. Written the literal way this asserted the UNTRUNCATED bound
        // and failed against a correct contract. `EloLaunchpad.t.sol` hit this
        // first and carries the same note; the fix did not travel to this file
        // when the curve was written, and nothing caught it because this suite
        // had never been executed.
        int24 spacing = factory.feeAmountTickSpacing(POOL_FEE);
        int24 expected = (int24(887272) / spacing) * spacing;
        assertEq(expected, int24(887200), "the tier's aligned bound");
        assertEq(curve.tickUpper(), expected);
        assertEq(curve.tickLower(), -expected);
    }

    // ── the whole shape, once ────────────────────────────────────────────────

    function test_a_launch_end_to_end() public {
        // the studio funds the coin. nothing mints, and no reserve is needed.
        _fundAndArm();
        assertEq(reserve.balanceOf(address(curve)), 0);

        // strangers buy, and one of them changes their mind and gets out
        uint256 aliceGot = _buy(alice, 60_000e18);
        uint256 bobGot = _buy(bob, 60_000e18);
        uint256 bobBack = _sell(bob, bobGot);
        assertGt(bobBack, 0, "the exit worked before any pool existed");

        // buying continues until the posted line
        _fillToThreshold();
        assertTrue(curve.closed());

        uint256 raised = curve.quoteReserve();
        curve.graduate(block.timestamp + 1);

        // the pool exists, the position is this contract's, and the raise is in it
        assertGt(curve.positionId(), 0);
        assertEq(reserve.balanceOf(address(curve)), curve.feesAccrued() + curve.quoteReserve(), "only fees and dust are left here");
        assertGt(raised, 0);

        // alice's coin is still hers, and the venue is now the pool
        assertGt(coin.balanceOf(alice), 0);
        assertEq(aliceGot, coin.balanceOf(alice));
    }
}
