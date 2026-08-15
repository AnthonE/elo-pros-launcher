// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryShrine.sol";

/// The Shrine: burn spoils as votive offerings, climb the
/// Mithraic seven-grade ladder on triangular thresholds. A rank is a public
/// record and nothing else - no payout, no buff, no reputation, no access.
/// `forge test -vv` before any broadcast.
contract ScryShrineTest is Test {
    SpoilsToken obol;
    SpoilsToken myrrh;
    ScryShrine shrine;

    address op = address(0xC0FFEE);
    address alice = address(0xA11CE);
    address bob = address(0xB0B);

    uint256 constant BASE_STEP = 10e18; // rank 1 costs 10 points (10 OBOL)

    // the posted menu: OBOL 1 point/token, MYRRH 5 (the Book's base rate)
    uint256 constant ALTAR_OBOL = 0;
    uint256 constant ALTAR_MYRRH = 1;

    function setUp() public {
        obol = new SpoilsToken("obol", "OBOL", 0, op);
        myrrh = new SpoilsToken("myrrh", "MYRRH", 0, op);
        shrine = new ScryShrine(BASE_STEP); // owner = this
        shrine.addOffering(obol, 1);
        shrine.addOffering(myrrh, 5);

        vm.startPrank(op);
        obol.mint(alice, 1000e18);
        myrrh.mint(alice, 1000e18);
        obol.mint(bob, 1000e18);
        vm.stopPrank();
        vm.startPrank(alice);
        obol.approve(address(shrine), type(uint256).max);
        myrrh.approve(address(shrine), type(uint256).max);
        vm.stopPrank();
        vm.prank(bob);
        obol.approve(address(shrine), type(uint256).max);
    }

    // -- offerings burn, points accrue -------------------------------------

    function test_offer_burns_supply_forever_and_grants_points_only() public {
        vm.prank(alice);
        shrine.offer(ALTAR_OBOL, 10e18);
        assertEq(obol.balanceOf(alice), 990e18);
        assertEq(obol.totalBurned(), 10e18); // retired, not parked
        assertEq(obol.balanceOf(address(shrine)), 0); // the shrine holds nothing
        assertEq(shrine.votiveOf(alice), 10e18);
        assertEq(shrine.totalVotive(), 10e18);
    }

    function test_myrrh_counts_five_to_one_the_books_base_rate() public {
        vm.prank(alice);
        shrine.offer(ALTAR_MYRRH, 2e18);
        assertEq(shrine.votiveOf(alice), 10e18); // 2 MYRRH = 10 points
        assertEq(shrine.levelOf(alice), 1);
    }

    // -- the triangular ladder ---------------------------------------------

    function test_rank_thresholds_are_triangular() public {
        assertEq(shrine.pointsForLevel(1), 10e18);
        assertEq(shrine.pointsForLevel(2), 30e18);
        assertEq(shrine.pointsForLevel(3), 60e18);
        assertEq(shrine.pointsForLevel(7), 280e18);

        vm.startPrank(alice);
        shrine.offer(ALTAR_OBOL, 10e18 - 1);
        assertEq(shrine.levelOf(alice), 0); // one wei short of Corax
        shrine.offer(ALTAR_OBOL, 1);
        assertEq(shrine.levelOf(alice), 1); // Corax exactly at the post
        shrine.offer(ALTAR_OBOL, 19e18);
        assertEq(shrine.levelOf(alice), 1); // 29: one point short of rank 2
        shrine.offer(ALTAR_OBOL, 1e18);
        assertEq(shrine.levelOf(alice), 2); // 30: Nymphus
        shrine.offer(ALTAR_OBOL, 250e18);
        assertEq(shrine.levelOf(alice), 7); // 280 cumulative: Pater
        vm.stopPrank();
    }

    function test_ladder_keeps_counting_above_pater() public {
        vm.prank(op);
        obol.mint(alice, 10_000e18);
        vm.startPrank(alice);
        obol.approve(address(shrine), type(uint256).max);
        shrine.offer(ALTAR_OBOL, 10_000e18); // 1000 steps banked
        vm.stopPrank();
        // L(L+1)/2 <= 1000 => L = 44
        assertEq(shrine.levelOf(alice), 44);
        assertEq(shrine.rankName(shrine.levelOf(alice)), "Pater");
    }

    function test_rank_names_are_the_mithraic_seven() public {
        assertEq(shrine.rankName(0), "Profane");
        assertEq(shrine.rankName(1), "Corax");
        assertEq(shrine.rankName(2), "Nymphus");
        assertEq(shrine.rankName(3), "Miles");
        assertEq(shrine.rankName(4), "Leo");
        assertEq(shrine.rankName(5), "Perses");
        assertEq(shrine.rankName(6), "Heliodromus");
        assertEq(shrine.rankName(7), "Pater");
        assertEq(shrine.rankName(100), "Pater"); // the last new name, not a cap
    }

    function test_ranks_are_per_address_never_pooled() public {
        vm.prank(alice);
        shrine.offer(ALTAR_OBOL, 10e18);
        vm.prank(bob);
        shrine.offer(ALTAR_OBOL, 30e18);
        assertEq(shrine.levelOf(alice), 1);
        assertEq(shrine.levelOf(bob), 2);
        assertEq(shrine.totalVotive(), 40e18);
    }

    // -- guardrails ---------------------------------------------------------

    function test_closed_altar_refuses_new_offerings_only() public {
        vm.prank(alice);
        shrine.offer(ALTAR_OBOL, 10e18);
        shrine.setOfferingEnabled(ALTAR_OBOL, false);
        vm.expectRevert("altar closed");
        vm.prank(alice);
        shrine.offer(ALTAR_OBOL, 1e18);
        assertEq(shrine.levelOf(alice), 1); // standing already held is untouched
        vm.prank(alice);
        shrine.offer(ALTAR_MYRRH, 4e18); // other altars unaffected
        assertEq(shrine.levelOf(alice), 2);
    }

    function test_no_allowance_no_burn() public {
        vm.expectRevert("allowance");
        vm.prank(address(0xDEED));
        shrine.offer(ALTAR_OBOL, 1e18);
    }

    function test_knobs_owner_only_menu_append_only() public {
        vm.startPrank(alice);
        vm.expectRevert("owner only");
        shrine.addOffering(obol, 1);
        vm.expectRevert("owner only");
        shrine.setOfferingEnabled(0, false);
        vm.stopPrank();
        vm.expectRevert("altar exists");
        shrine.addOffering(obol, 2); // rates are posted once, never edited
        vm.expectRevert("zero step");
        new ScryShrine(0);
        vm.expectRevert(bytes("zero"));
        vm.prank(alice);
        shrine.offer(ALTAR_OBOL, 0);
    }

    // -- governance + menu guards (TEST-AUDIT.md: operator knobs) -----------

    function test_zero_rate_altar_refused() public {
        SpoilsToken salt = new SpoilsToken("salt", "SALT", 0, op);
        vm.expectRevert("zero rate");
        shrine.addOffering(salt, 0); // a rate is posted at add time, never zero
    }

    /// A closed altar is a pause, not a demolition: setOfferingEnabled(true)
    /// reopens it and offerings burn + count exactly as before.
    function test_reenabling_a_closed_altar_takes_offerings_again() public {
        shrine.setOfferingEnabled(ALTAR_OBOL, false);
        vm.expectRevert("altar closed");
        vm.prank(alice);
        shrine.offer(ALTAR_OBOL, 1e18);
        shrine.setOfferingEnabled(ALTAR_OBOL, true);
        vm.prank(alice);
        shrine.offer(ALTAR_OBOL, 10e18);
        assertEq(shrine.votiveOf(alice), 10e18); // points accrue as before
        assertEq(shrine.levelOf(alice), 1);
        assertEq(obol.totalBurned(), 10e18); // and the burn still retires supply
    }

    function test_transferOwner_old_refused_new_works() public {
        vm.expectRevert("zero owner");
        shrine.transferOwner(address(0));
        vm.prank(bob);
        vm.expectRevert("owner only");
        shrine.transferOwner(bob); // a stranger cannot take the keys
        shrine.transferOwner(alice);
        assertEq(shrine.owner(), alice);
        SpoilsToken salt = new SpoilsToken("salt", "SALT", 0, op);
        vm.expectRevert("owner only");
        shrine.addOffering(salt, 1); // the old owner (this) is refused
        vm.prank(alice);
        shrine.addOffering(salt, 1); // the new owner works
        assertEq(shrine.offeringCount(), 3);
    }
}
