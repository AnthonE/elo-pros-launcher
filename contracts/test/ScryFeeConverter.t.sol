// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryFeeConverter.sol";
import "../src/ScryLaunch.sol";
import "./MockToken.sol";
import "./MockUniswapV3.sol";

/// A studio's fees, without the bag of its own coin.
///
/// What these pin:
///   - the coin leaves ONLY as a swap; SCRY leaves ONLY to the immutable payee
///   - there is no withdraw, same as the vault
///   - the steward chooses a floor and gains nothing by calling anything
///   - renouncing leaves `pay()` working
contract ScryFeeConverterTest is Test {
    ScryFeeConverter conv;
    MockToken scry;
    MockToken coin;
    MockSwapRouter router;

    address steward = address(0x5D10);
    address payee = address(0xBEEF);
    address stranger = address(0x57A);

    function setUp() public {
        scry = new MockToken("Scry", "SCRY");
        coin = new MockToken("Gates Coin", "GATE");
        router = new MockSwapRouter();
        conv = new ScryFeeConverter(
            IERC20(address(scry)), IERC20(address(coin)), ISwapRouter(address(router)), 10000, payee, steward
        );
    }

    function test_there_is_no_way_out_except_the_two_posted_ones() public {
        string[6] memory forbidden = [
            "withdraw()",
            "withdraw(uint256)",
            "rescue(address,uint256)",
            "sweep()",
            "sweepTo(address)",
            "emergencyWithdraw()"
        ];
        for (uint256 i; i < forbidden.length; ++i) {
            (bool ok,) = address(conv).call(abi.encodeWithSignature(forbidden[i]));
            assertFalse(ok, forbidden[i]);
        }
    }

    function test_the_coin_leaves_only_as_a_swap_and_scry_only_to_the_payee() public {
        coin.mint(address(conv), 100e18);

        vm.prank(steward);
        uint256 out = conv.convert(100e18, 90e18, block.timestamp + 1);
        assertEq(out, 100e18);
        assertEq(coin.balanceOf(address(conv)), 0, "the coin left as a swap");
        assertEq(scry.balanceOf(address(conv)), 100e18, "and landed back here, not with the steward");
        assertEq(scry.balanceOf(steward), 0);

        vm.prank(stranger); // permissionless: the destination is immutable
        uint256 paid = conv.pay();
        assertEq(paid, 100e18);
        assertEq(scry.balanceOf(payee), 100e18);
        assertEq(conv.totalPaid(), 100e18);
    }

    function test_only_the_steward_converts_and_a_floor_is_required() public {
        coin.mint(address(conv), 10e18);
        vm.prank(stranger);
        vm.expectRevert(bytes("not steward"));
        conv.convert(10e18, 1, block.timestamp + 1);

        vm.prank(steward);
        vm.expectRevert(bytes("name a floor"));
        conv.convert(10e18, 0, block.timestamp + 1);
    }

    function test_the_floor_bites() public {
        coin.mint(address(conv), 100e18);
        router.setRate(5000);
        vm.prank(steward);
        vm.expectRevert(bytes("Too little received"));
        conv.convert(100e18, 90e18, block.timestamp + 1);
    }

    function test_it_cannot_swap_coin_it_does_not_hold() public {
        coin.mint(address(conv), 1e18);
        vm.prank(steward);
        vm.expectRevert(bytes("not that much coin held"));
        conv.convert(5e18, 1, block.timestamp + 1);
    }

    function test_paying_nothing_refuses_rather_than_emitting_a_zero() public {
        vm.expectRevert(bytes("nothing to pay"));
        conv.pay();
    }

    function test_scry_that_arrives_directly_needs_no_swap() public {
        // the position's SCRY leg arrives already in the reserve
        scry.mint(address(conv), 42e18);
        conv.pay();
        assertEq(scry.balanceOf(payee), 42e18);
    }

    function test_renouncing_leaves_pay_working_forever() public {
        scry.mint(address(conv), 5e18);
        vm.prank(steward);
        conv.transferSteward(address(0));
        assertEq(conv.steward(), address(0));

        coin.mint(address(conv), 1e18);
        vm.prank(stranger);
        vm.expectRevert(bytes("not steward"));
        conv.convert(1e18, 1, block.timestamp + 1);

        vm.prank(stranger);
        conv.pay();
        assertEq(scry.balanceOf(payee), 5e18, "the studio still gets what was already converted");
    }

    function test_handover_is_two_steps() public {
        vm.prank(steward);
        conv.transferSteward(stranger);
        assertEq(conv.steward(), steward);
        vm.prank(stranger);
        conv.acceptSteward();
        assertEq(conv.steward(), stranger);
    }

    function test_the_payee_cannot_be_repointed() public view {
        // there is no setter; this asserts the shape the claim rests on
        assertEq(conv.payee(), payee);
    }

    function test_it_refuses_to_be_built_with_scry_as_the_coin() public {
        vm.expectRevert(bytes("scry cannot be the coin"));
        new ScryFeeConverter(
            IERC20(address(scry)), IERC20(address(scry)), ISwapRouter(address(router)), 10000, payee, steward
        );
    }
}
