// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloLaunchpad.sol";
import "../src/EloGameTicket.sol";
import "./MockToken.sol";
import "./MockUniswapV3.sol";

/// The launch vault: the copy sale's money, with only one door.
///
/// What these pin, by section:
///   - THE claim: there is no way out. Not "the owner shouldn't" — no function
///   - continuous, not thresholded: the pool opens at seed and deepens per sale
///   - the MEV split: `pair` permissionless under a TWAP guard, `convert`
///     steward-gated with a floor
///   - fees leave, liquidity does not, and the fee destination cannot be aimed
///   - renouncing the steward does not freeze the pool
///
/// ⚠ These run against MOCK v3. They prove this contract's logic and nothing
/// about real Uniswap. A fork test against the canonical addresses is still
/// owed before any broadcast (`LAUNCHPAD.md` §gates).
contract EloLaunchTest is Test {
    EloLaunchpad vault;
    MockToken reserve;
    MockToken coin;
    MockToken usdg;
    MockWETH weth;
    MockV3Factory factory;
    MockV3Pool pool;
    MockPositionManager npm;
    MockSwapRouter router;

    address steward = address(0x5D10);
    address reserveFeeTo = address(0xFEE5); // in practice: the studio, or a splitter
    address coinFeeTo = address(0xFEE6); // in practice: the game's OWN EloFeeSplitter
    address stranger = address(0x57A);

    uint24 constant POOL_FEE = 10000;

    function setUp() public {
        reserve = new MockToken("Scry", "SCRY");
        coin = new MockToken("Gates Coin", "GATE");
        usdg = new MockToken("USD Glo", "USDG");
        weth = new MockWETH();
        factory = new MockV3Factory();
        pool = new MockV3Pool();
        npm = new MockPositionManager(address(factory), pool);
        router = new MockSwapRouter();

        vault = new EloLaunchpad(
            IERC20(address(reserve)),
            IERC20(address(coin)),
            IWETH9(address(weth)),
            ILaunchPositionManager(address(npm)),
            ISwapRouter(address(router)),
            POOL_FEE,
            POOL_FEE,
            reserveFeeTo,
            coinFeeTo,
            steward
        );

        (address t0, address t1) =
            address(reserve) < address(coin) ? (address(reserve), address(coin)) : (address(coin), address(reserve));
        npm.setTokens(t0, t1);
        factory.setPool(address(reserve), address(coin), POOL_FEE, address(pool));
    }

    function _fund(uint256 reserveAmt, uint256 coinAmt) internal {
        reserve.mint(address(vault), reserveAmt);
        coin.mint(address(vault), coinAmt);
    }

    function _seed() internal {
        _fund(1000e18, 5000e18);
        vm.prank(steward);
        vault.seed(uint160(1) << 96, 1000e18, 5000e18, block.timestamp + 1);
    }

    // ── THE claim: there is no exit ──────────────────────────────────────────

    function test_there_is_no_function_that_takes_value_out() public {
        // If a withdraw is ever added this test is the one that should fail:
        // the whole design rests on the absence, not on a modifier.
        string[8] memory forbidden = [
            "withdraw()",
            "withdraw(uint256)",
            "rescue(address,uint256)",
            "sweep()",
            "sweepTo(address)",
            "decreaseLiquidity(uint256)",
            "emergencyWithdraw()",
            "transferPosition(address)"
        ];
        for (uint256 i; i < forbidden.length; ++i) {
            (bool ok,) = address(vault).call(abi.encodeWithSignature(forbidden[i]));
            assertFalse(ok, forbidden[i]);
        }
    }

    function test_the_steward_cannot_move_the_raise_anywhere_but_the_pool() public {
        _fund(1000e18, 5000e18);
        uint256 before = reserve.balanceOf(steward);
        // every steward-callable path, run by the steward, moves nothing to them
        vm.startPrank(steward);
        vault.seed(uint160(1) << 96, 1000e18, 5000e18, block.timestamp + 1);
        vm.stopPrank();
        assertEq(reserve.balanceOf(steward), before, "the steward gained nothing");
        assertEq(reserve.balanceOf(address(vault)), 0, "and it went to the position");
        assertGt(npm.totalAmount0() + npm.totalAmount1(), 0);
    }

    function test_each_fee_leg_goes_to_its_own_welded_destination() public {
        _seed();
        (address t0,) = address(reserve) < address(coin)
            ? (address(reserve), address(coin)) : (address(coin), address(reserve));
        // owed0/owed1 are in pool ordering; give the reserve leg 3 and the coin leg 7
        if (t0 == address(reserve)) npm.setFeesOwed(3e18, 7e18);
        else npm.setFeesOwed(7e18, 3e18);

        vm.prank(stranger); // permissionless: both destinations are immutable
        (uint256 reserveFees, uint256 coinFees) = vault.collectFees();

        assertEq(reserveFees, 3e18);
        assertEq(coinFees, 7e18);
        assertEq(reserve.balanceOf(reserveFeeTo), 3e18, "the reserve leg");
        assertEq(coin.balanceOf(coinFeeTo), 7e18, "the coin leg - a game's own splitter/bank");
        assertEq(vault.reserveFeeRecipient(), reserveFeeTo);
        assertEq(vault.coinFeeRecipient(), coinFeeTo);
    }

    /// ⚠ THE ONE PLACE THIS CONTRACT COULD LEAK. Fees land here before being
    /// forwarded, and the raise is sitting in the same balance. If the forward
    /// used a balance rather than the collect's delta, a vault holding a raise
    /// would pay that raise out as "fees" the first time anyone called this.
    function test_a_raise_sitting_in_the_vault_is_never_paid_out_as_fees() public {
        _seed();
        _fund(500e18, 900e18); // an unpaired raise, waiting for pair()
        (address t0,) = address(reserve) < address(coin)
            ? (address(reserve), address(coin)) : (address(coin), address(reserve));
        if (t0 == address(reserve)) npm.setFeesOwed(1e18, 2e18);
        else npm.setFeesOwed(2e18, 1e18);

        (uint256 reserveFees, uint256 coinFees) = vault.collectFees();

        assertEq(reserveFees, 1e18, "only what the position owed");
        assertEq(coinFees, 2e18);
        assertEq(reserve.balanceOf(reserveFeeTo), 1e18);
        assertEq(coin.balanceOf(coinFeeTo), 2e18);
        // and the raise is still here, still poolable
        assertEq(reserve.balanceOf(address(vault)), 500e18, "the raise did not move");
        assertEq(coin.balanceOf(address(vault)), 900e18);
    }

    function test_collecting_nothing_moves_nothing() public {
        _seed();
        vault.collectFees();
        assertEq(reserve.balanceOf(reserveFeeTo), 0);
        assertEq(coin.balanceOf(coinFeeTo), 0);
        assertEq(npm.collects(), 1, "it still called through");
    }

    // ── continuous, not thresholded ──────────────────────────────────────────

    function test_the_pool_opens_at_seed_with_no_threshold_to_reach() public {
        assertFalse(vault.seeded());
        _seed();
        assertTrue(vault.seeded(), "there is no waiting and nothing to cross");
        assertGt(vault.positionId(), 0);
        assertEq(vault.totalEloPaired(), 1000e18);
        assertEq(vault.totalCoinPaired(), 5000e18);
    }

    function test_every_later_sale_deepens_the_same_position() public {
        _seed();
        _fund(100e18, 500e18); // a later copy sale, converted and funded
        vm.prank(stranger); // ANYONE may deepen it
        vault.pair(block.timestamp + 1);
        assertEq(npm.increases(), 1);
        assertEq(vault.totalEloPaired(), 1100e18);
        assertEq(reserve.balanceOf(address(vault)), 0, "nothing is left sitting");
    }

    function test_pair_before_seed_refuses() public {
        _fund(1e18, 1e18);
        vm.expectRevert(bytes("not seeded"));
        vault.pair(block.timestamp + 1);
    }

    function test_seeding_twice_refuses() public {
        _seed();
        _fund(1e18, 1e18);
        vm.prank(steward);
        vm.expectRevert(bytes("already seeded"));
        vault.seed(uint160(1) << 96, 1e18, 1e18, block.timestamp + 1);
    }

    function test_seeding_without_the_coin_side_funded_refuses() public {
        reserve.mint(address(vault), 1000e18);
        vm.prank(steward);
        vm.expectRevert(bytes("the coin side is not funded"));
        vault.seed(uint160(1) << 96, 1000e18, 5000e18, block.timestamp + 1);
    }

    // ── the MEV split ────────────────────────────────────────────────────────

    function test_pair_refuses_while_spot_is_far_from_the_twap() public {
        _seed();
        _fund(100e18, 500e18);
        pool.setTicks(int24(6000), int24(0)); // spot yanked away from the mean
        vm.expectRevert(bytes("spot is far from the twap"));
        vault.pair(block.timestamp + 1);
    }

    function test_pair_allows_a_deviation_inside_the_band_in_both_directions() public {
        _seed();
        _fund(100e18, 500e18);
        pool.setTicks(int24(-400), int24(0)); // below the mean, inside 500
        vault.pair(block.timestamp + 1);
        assertEq(npm.increases(), 1);
    }

    function test_only_the_steward_converts_and_a_floor_is_required() public {
        usdg.mint(address(vault), 100e18);
        vm.prank(stranger);
        vm.expectRevert(bytes("not steward"));
        vault.convert(address(usdg), 100e18, 1, block.timestamp + 1);

        vm.prank(steward);
        vm.expectRevert(bytes("name a floor"));
        vault.convert(address(usdg), 100e18, 0, block.timestamp + 1);
    }

    function test_convert_output_lands_in_the_vault_never_the_caller() public {
        usdg.mint(address(vault), 100e18);
        vm.prank(steward);
        uint256 out = vault.convert(address(usdg), 100e18, 90e18, block.timestamp + 1);
        assertEq(out, 100e18);
        assertEq(reserve.balanceOf(address(vault)), 100e18, "the swap cannot pay the steward");
        assertEq(reserve.balanceOf(steward), 0);
    }

    function test_convert_honours_its_floor() public {
        usdg.mint(address(vault), 100e18);
        router.setRate(5000); // half
        vm.prank(steward);
        vm.expectRevert(bytes("Too little received"));
        vault.convert(address(usdg), 100e18, 90e18, block.timestamp + 1);
    }

    function test_convert_refuses_the_two_assets_that_are_not_the_raise() public {
        vm.startPrank(steward);
        vm.expectRevert(bytes("already reserve"));
        vault.convert(address(reserve), 1, 1, block.timestamp + 1);
        vm.expectRevert(bytes("the coin side is not the raise"));
        vault.convert(address(coin), 1, 1, block.timestamp + 1);
        vm.stopPrank();
    }

    // ── the raise arrives ────────────────────────────────────────────────────

    function test_it_takes_the_tickets_eth_sweep_and_anyone_may_wrap_it() public {
        vm.deal(address(this), 5 ether);
        (bool ok,) = address(vault).call{value: 5 ether}("");
        assertTrue(ok, "the ticket's sweep() must land");
        vm.prank(stranger);
        vault.wrap();
        assertEq(weth.balanceOf(address(vault)), 5 ether);
        assertEq(address(vault).balance, 0);
    }

    function test_pending_reads_what_is_waiting_rather_than_claiming_it() public {
        vm.deal(address(this), 1 ether);
        (bool ok,) = address(vault).call{value: 1 ether}("");
        assertTrue(ok);
        _fund(2e18, 3e18);
        (uint256 e, uint256 s, uint256 c) = vault.pending();
        assertEq(e, 1 ether);
        assertEq(s, 2e18);
        assertEq(c, 3e18);
    }

    // ── the steward is an operator, never a holder ───────────────────────────

    function test_steward_handover_is_two_steps() public {
        vm.prank(steward);
        vault.transferSteward(stranger);
        assertEq(vault.steward(), steward, "still theirs until accepted");
        vm.prank(stranger);
        vault.acceptSteward();
        assertEq(vault.steward(), stranger);
    }

    function test_renouncing_the_steward_does_not_freeze_the_pool() public {
        _seed();
        vm.prank(steward);
        vault.transferSteward(address(0));
        assertEq(vault.steward(), address(0));

        // seeding and converting are gone forever...
        vm.prank(stranger);
        vm.expectRevert(bytes("not steward"));
        vault.convert(address(usdg), 1, 1, block.timestamp + 1);

        // ...but the two that matter to a holder still work, for anyone
        _fund(10e18, 50e18);
        vm.prank(stranger);
        vault.pair(block.timestamp + 1);
        npm.setFeesOwed(1e18, 1e18);
        vm.prank(stranger);
        vault.collectFees();
        assertEq(npm.lastCollectRecipient(), address(vault), "collected here, then forwarded");
    }

    // ── the wiring refuses to be built wrong ─────────────────────────────────

    function test_a_launch_cannot_pair_scry_with_scry() public {
        vm.expectRevert(bytes("elo cannot pair with itself"));
        new EloLaunchpad(
            IERC20(address(reserve)), IERC20(address(reserve)), IWETH9(address(weth)),
            ILaunchPositionManager(address(npm)), ISwapRouter(address(router)),
            POOL_FEE, POOL_FEE, reserveFeeTo, coinFeeTo, steward
        );
    }

    function test_an_unknown_fee_tier_refuses_rather_than_minting_a_bad_range() public {
        vm.expectRevert(bytes("unknown fee tier"));
        new EloLaunchpad(
            IERC20(address(reserve)), IERC20(address(coin)), IWETH9(address(weth)),
            ILaunchPositionManager(address(npm)), ISwapRouter(address(router)),
            uint24(1234), POOL_FEE, reserveFeeTo, coinFeeTo, steward
        );
    }

    function test_the_range_is_full_and_aligned_to_the_tiers_spacing() public view {
        // ⚠ A LITERAL EXPRESSION IS FOLDED AS AN EXACT RATIONAL, NOT AS INTEGER
        // DIVISION. `(887272 / 200) * 200` is 887272 to solc — the truncation
        // the constructor performs only happens because it divides through an
        // `int24` VARIABLE. Written the literal way this asserted the
        // untruncated bound and failed against a correct contract.
        int24 spacing = factory.feeAmountTickSpacing(POOL_FEE);
        int24 expected = (int24(887272) / spacing) * spacing;
        assertEq(expected, int24(887200), "the tier's aligned bound");
        assertEq(vault.tickUpper(), expected);
        assertEq(vault.tickLower(), -expected);
    }

    // ── the whole shape, once ────────────────────────────────────────────────

    function test_a_copy_sale_becomes_liquidity_end_to_end() public {
        // the coin side is FUNDED by a transfer, never minted here
        coin.mint(address(vault), 10_000e18);
        reserve.mint(address(vault), 1_000e18);
        vm.prank(steward);
        vault.seed(uint160(1) << 96, 1_000e18, 10_000e18, block.timestamp + 1);

        // a copy sells in USDG; the ticket routes it here
        usdg.mint(address(vault), 10e18);
        vm.prank(steward);
        vault.convert(address(usdg), 10e18, 9e18, block.timestamp + 1);
        coin.mint(address(vault), 100e18);

        uint256 pairedBefore = vault.totalEloPaired();
        vm.prank(stranger);
        vault.pair(block.timestamp + 1);

        assertEq(vault.totalEloPaired(), pairedBefore + 10e18);
        assertEq(usdg.balanceOf(address(vault)), 0);
        assertEq(reserve.balanceOf(address(vault)), 0);
        assertEq(coin.balanceOf(address(vault)), 0);
    }
}
