// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryItem.sol";
import "../src/ScryKiln.sol";

/// `CARDS.md` §7. The kiln burns one of each card in a declared set and mints
/// one crest — so what this suite has to prove is mostly about what it
/// REFUSES: cards you do not own, cards outside the set, two of the same card,
/// the same token passed twice, and a set whose composition moved after
/// somebody assembled it.
///
/// This suite has run and passed — under Hardhat 3's EDR, which implements the
/// forge-std cheatcodes it uses; the authoring box has no foundry (`CARDS.md`
/// §8). The first execution surfaced four bugs in the tests themselves, all
/// one shape: a `KIND_*()` getter inside a pranked call's argument list is a
/// staticcall, and it consumes the one-shot prank before the call under test.
/// The canonical gate is still owed on a box with foundry:
///   forge test --match-contract ScryKilnTest -vv
contract ScryKilnTest is Test {
    ScryItem cards;
    ScryItem crests;
    ScryKiln kiln;

    address creator = address(0xC0FFEE);
    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    address payout = address(0xFEE5);

    uint32 constant SET = 1;
    uint32 constant CREST_CAT = 900;

    // A five-card set, which is the small end of Steam's 5–15.
    uint32[] setIds;

    function setUp() public {
        vm.startPrank(creator);
        cards = new ScryItem("Gates Cards", "GCARD", payout, 0, "https://scry.moreright.xyz/cards/");
        crests = new ScryItem("Gates Crests", "GCREST", payout, 0, "https://scry.moreright.xyz/crests/");
        kiln = new ScryKiln(address(cards), address(crests));

        // The kiln mints crests and the creator keeps minting cards.
        crests.setMinter(address(kiln), true);
        cards.setMinter(creator, true);

        setIds = new uint32[](5);
        for (uint32 i = 0; i < 5; i++) {
            setIds[i] = 10 + i;
        }
        kiln.defineSet(SET, setIds, CREST_CAT, 5);
        vm.stopPrank();
    }

    function _ref(uint256 i) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked("card", i));
    }

    /// Mint one of each card in the set to `to`, and approve the kiln.
    function _fullSet(address to, uint256 salt) internal returns (uint256[] memory ids) {
        ids = new uint256[](5);
        vm.startPrank(creator);
        for (uint256 i = 0; i < 5; i++) {
            ids[i] = cards.mint(to, setIds[i], cards.KIND_PLAY(), _ref(salt * 100 + i));
        }
        vm.stopPrank();
        vm.prank(to);
        cards.setApprovalForAll(address(kiln), true);
    }

    // ── the constructor's one wall ───────────────────────────────────────────

    function test_constructor_refuses_one_contract_for_both() public {
        vm.expectRevert(ScryKiln.OneContractForBoth.selector);
        new ScryKiln(address(cards), address(cards));
    }

    function test_constructor_refuses_zero() public {
        vm.expectRevert(ScryKiln.ZeroAddress.selector);
        new ScryKiln(address(0), address(crests));
        vm.expectRevert(ScryKiln.ZeroAddress.selector);
        new ScryKiln(address(cards), address(0));
    }

    // ── defining a set ───────────────────────────────────────────────────────

    function test_only_owner_defines() public {
        vm.prank(alice);
        vm.expectRevert(ScryKiln.NotOwner.selector);
        kiln.defineSet(2, setIds, CREST_CAT, 5);
    }

    function test_rejects_a_set_that_names_a_card_twice() public {
        uint32[] memory dupes = new uint32[](3);
        dupes[0] = 1;
        dupes[1] = 2;
        dupes[2] = 1;
        vm.prank(creator);
        vm.expectRevert(ScryKiln.DuplicateCatalog.selector);
        kiln.defineSet(2, dupes, CREST_CAT, 5);
    }

    function test_rejects_bad_sizes_and_zero_levels() public {
        uint32[] memory one = new uint32[](1);
        one[0] = 1;
        vm.prank(creator);
        vm.expectRevert(ScryKiln.BadSetSize.selector);
        kiln.defineSet(2, one, CREST_CAT, 5);

        uint32[] memory huge = new uint32[](33);
        for (uint32 i = 0; i < 33; i++) {
            huge[i] = 1000 + i;
        }
        vm.prank(creator);
        vm.expectRevert(ScryKiln.BadSetSize.selector);
        kiln.defineSet(3, huge, CREST_CAT, 5);

        vm.prank(creator);
        vm.expectRevert(ScryKiln.BadMaxLevel.selector);
        kiln.defineSet(4, setIds, CREST_CAT, 0);
    }

    // ── the happy path ───────────────────────────────────────────────────────

    function test_fire_burns_the_set_and_mints_one_crest() public {
        uint256[] memory ids = _fullSet(alice, 1);
        assertEq(cards.balanceOf(alice), 5);

        vm.prank(alice);
        uint256 crestId = kiln.fire(SET, ids);

        // the cards are gone
        assertEq(cards.balanceOf(alice), 0);
        assertEq(cards.totalSupply(), 0);
        for (uint256 i = 0; i < ids.length; i++) {
            vm.expectRevert(bytes("no such token"));
            cards.ownerOf(ids[i]);
        }

        // the crest is theirs, and it says how it was made
        assertEq(crests.ownerOf(crestId), alice);
        assertEq(crests.balanceOf(alice), 1);
        (uint32 catalogId, uint8 kind,) = crests.instanceOf(crestId);
        assertEq(catalogId, CREST_CAT);
        assertEq(kind, crests.KIND_CRAFT());
        assertEq(kind, 3);

        assertEq(kiln.firedBy(SET, alice), 1);
        assertEq(kiln.firedTotal(SET), 1);
    }

    function test_the_crest_mintref_is_the_level() public {
        uint256[] memory first = _fullSet(alice, 1);
        vm.prank(alice);
        uint256 a = kiln.fire(SET, first);

        uint256[] memory second = _fullSet(alice, 2);
        vm.prank(alice);
        uint256 b = kiln.fire(SET, second);

        assertTrue(a != b);
        (,, bytes32 refA) = crests.instanceOf(a);
        (,, bytes32 refB) = crests.instanceOf(b);
        assertEq(refA, keccak256(abi.encodePacked(SET, alice, uint8(1))));
        assertEq(refB, keccak256(abi.encodePacked(SET, alice, uint8(2))));
        assertEq(kiln.firedTotal(SET), 2);
    }

    function test_levels_are_per_wallet_not_global() public {
        uint256[] memory hers = _fullSet(alice, 1);
        vm.prank(alice);
        kiln.fire(SET, hers);

        uint256[] memory his = _fullSet(bob, 2);
        vm.prank(bob);
        kiln.fire(SET, his);

        assertEq(kiln.firedBy(SET, alice), 1);
        assertEq(kiln.firedBy(SET, bob), 1);
        assertEq(kiln.levelsLeft(SET, alice), 4);
        assertEq(kiln.levelsLeft(SET, bob), 4);
    }

    function test_level_cap_binds() public {
        for (uint256 n = 1; n <= 5; n++) {
            uint256[] memory ids = _fullSet(alice, n);
            vm.prank(alice);
            kiln.fire(SET, ids);
        }
        assertEq(kiln.levelsLeft(SET, alice), 0);

        uint256[] memory sixth = _fullSet(alice, 6);
        vm.prank(alice);
        vm.expectRevert(ScryKiln.LevelCapReached.selector);
        kiln.fire(SET, sixth);
    }

    // ── what it refuses ──────────────────────────────────────────────────────

    function test_refuses_an_unknown_set() public {
        uint256[] memory ids = _fullSet(alice, 1);
        vm.prank(alice);
        vm.expectRevert(ScryKiln.SetUnknown.selector);
        kiln.fire(99, ids);
    }

    function test_refuses_the_wrong_count() public {
        uint256[] memory ids = _fullSet(alice, 1);
        uint256[] memory four = new uint256[](4);
        for (uint256 i = 0; i < 4; i++) {
            four[i] = ids[i];
        }
        vm.prank(alice);
        vm.expectRevert(ScryKiln.WrongCardCount.selector);
        kiln.fire(SET, four);
    }

    /// The one that matters: approval is not ownership. Bob approving the kiln
    /// must never let Alice burn Bob's cards through it.
    function test_refuses_cards_you_do_not_own() public {
        uint256[] memory his = _fullSet(bob, 1);
        vm.prank(alice);
        vm.expectRevert(ScryKiln.NotYourCard.selector);
        kiln.fire(SET, his);
        assertEq(cards.balanceOf(bob), 5);
    }

    function test_refuses_a_card_from_outside_the_set() public {
        uint256[] memory ids = _fullSet(alice, 1);
        // read the kind BEFORE pranking — a getter in the argument list is a
        // staticcall, and it consumes the one-shot prank before the mint runs
        uint8 kindPlay = cards.KIND_PLAY();
        vm.prank(creator);
        uint256 stranger = cards.mint(alice, 777, kindPlay, _ref(999));
        ids[4] = stranger;
        vm.prank(alice);
        vm.expectRevert(ScryKiln.CardNotInSet.selector);
        kiln.fire(SET, ids);
    }

    function test_refuses_two_of_the_same_card() public {
        uint256[] memory ids = _fullSet(alice, 1);
        uint8 kindPlay = cards.KIND_PLAY(); // before the prank — see above
        vm.prank(creator);
        uint256 secondOfFirst = cards.mint(alice, setIds[0], kindPlay, _ref(555));
        ids[4] = secondOfFirst; // four distinct + a duplicate of the first
        vm.prank(alice);
        vm.expectRevert(ScryKiln.DuplicateCard.selector);
        kiln.fire(SET, ids);
    }

    function test_refuses_the_same_token_listed_twice() public {
        uint256[] memory ids = _fullSet(alice, 1);
        ids[4] = ids[0];
        vm.prank(alice);
        vm.expectRevert(ScryKiln.DuplicateCard.selector);
        kiln.fire(SET, ids);
    }

    function test_without_approval_the_burn_refuses() public {
        uint256[] memory ids = _fullSet(alice, 1);
        vm.prank(alice);
        cards.setApprovalForAll(address(kiln), false);
        assertFalse(kiln.approvedByAll(alice));

        vm.prank(alice);
        vm.expectRevert(bytes("not authorized"));
        kiln.fire(SET, ids);
        assertEq(cards.balanceOf(alice), 5);
    }

    function test_kiln_must_be_a_minter_on_the_crests() public {
        vm.prank(creator);
        crests.setMinter(address(kiln), false);

        uint256[] memory ids = _fullSet(alice, 1);
        vm.prank(alice);
        vm.expectRevert(bytes("not a minter"));
        kiln.fire(SET, ids);
    }

    // ── the freeze ───────────────────────────────────────────────────────────

    function test_a_set_freezes_the_first_time_it_fires() public {
        (,,,, bool frozenBefore) = kiln.setOf(SET);
        assertFalse(frozenBefore);

        uint256[] memory ids = _fullSet(alice, 1);
        vm.prank(alice);
        kiln.fire(SET, ids);

        (,,,, bool frozenAfter) = kiln.setOf(SET);
        assertTrue(frozenAfter);

        vm.prank(creator);
        vm.expectRevert(ScryKiln.SetIsFrozen.selector);
        kiln.defineSet(SET, setIds, CREST_CAT, 9);
    }

    function test_a_set_may_be_corrected_before_it_fires() public {
        uint32[] memory six = new uint32[](6);
        for (uint32 i = 0; i < 6; i++) {
            six[i] = 10 + i;
        }
        vm.prank(creator);
        kiln.defineSet(SET, six, CREST_CAT, 3);
        assertEq(kiln.setSize(SET), 6);
        (,, uint8 maxLevel,,) = kiln.setOf(SET);
        assertEq(maxLevel, 3);
    }

    // ── the kiln never holds anything ────────────────────────────────────────

    function test_the_kiln_holds_no_items_at_any_point() public {
        uint256[] memory ids = _fullSet(alice, 1);
        vm.prank(alice);
        kiln.fire(SET, ids);
        assertEq(cards.balanceOf(address(kiln)), 0);
        assertEq(crests.balanceOf(address(kiln)), 0);
        assertEq(address(kiln).balance, 0);
    }

    function test_levels_left_is_zero_for_an_unknown_set() public view {
        assertEq(kiln.levelsLeft(4242, alice), 0);
    }
}
