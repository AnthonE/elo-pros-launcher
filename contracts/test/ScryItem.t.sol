// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryItem.sol";

/// A receiver that accepts, and one that does not — the `safeTransferFrom`
/// surface. The gacha uses plain `transferFrom` and therefore neither.
contract ItemGoodReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0x150b7a02;
    }
}

contract ItemBadReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0xdeadbeef;
    }
}

/// Stands in for `ScryGacha`'s custody path: pull with plain `transferFrom`,
/// assert we hold it, hand it on later. If ScryItem ever grows a hook, a pause
/// or a soulbind, this is what breaks first — which is the point of it.
contract GachaLike {
    function pull(ScryItem item, address from, uint256 tokenId) external {
        item.transferFrom(from, address(this), tokenId);
        require(item.ownerOf(tokenId) == address(this), "NftTransferFailed");
    }

    function push(ScryItem item, address to, uint256 tokenId) external {
        item.transferFrom(address(this), to, tokenId);
    }
}

contract ScryItemTest is Test {
    ScryItem item;

    address creator = address(0xC0FFEE);
    address minter = address(0xBEEF);
    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    address payout = address(0xFEE5);

    uint32 constant CAT = 7;

    function setUp() public {
        vm.prank(creator);
        item = new ScryItem("Gates Items", "GATE", payout, 500, "https://scry.moreright.xyz");
        vm.prank(creator);
        item.setMinter(minter, true);
    }

    function _ref(uint256 i) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked("mintref", i));
    }

    // ── the creator owns it ──────────────────────────────────────────────────

    function test_creator_is_owner_not_deployer_of_record() public view {
        assertEq(item.owner(), creator);
        assertEq(item.minterCount(), 1);
    }

    function test_stranger_cannot_set_minter_or_root() public {
        vm.prank(alice);
        vm.expectRevert(bytes("not owner"));
        item.setMinter(alice, true);

        vm.prank(alice);
        vm.expectRevert(bytes("not owner"));
        item.setDropRoot(bytes32(uint256(1)));
    }

    function test_stranger_cannot_mint() public {
        vm.prank(alice);
        vm.expectRevert(bytes("not a minter"));
        item.mint(alice, CAT, 0, _ref(1));
    }

    // ── what it stores, and what it refuses to store ─────────────────────────

    function test_mint_stores_the_reference_and_nothing_else() public {
        // read the kind BEFORE pranking — a getter in the argument list is a
        // staticcall, and it consumes the one-shot prank before the mint runs
        uint8 kindPlay = item.KIND_PLAY();
        vm.prank(minter);
        uint256 id = item.mint(alice, CAT, kindPlay, _ref(1));

        (uint32 catalogId, uint8 kind, bytes32 ref) = item.instanceOf(id);
        assertEq(catalogId, CAT);
        assertEq(kind, 0);
        assertEq(ref, _ref(1));
        assertEq(item.mintRefOf(id), _ref(1));
        assertEq(item.ownerOf(id), alice);
        assertEq(item.totalSupply(), 1);
    }

    function test_a_dupe_is_unmintable_not_merely_detectable() public {
        vm.startPrank(minter);
        item.mint(alice, CAT, 0, _ref(1));
        vm.expectRevert(bytes("mintRef used"));
        item.mint(bob, CAT, 0, _ref(1));
        vm.stopPrank();
    }

    function test_burn_does_not_free_the_mintref() public {
        vm.prank(minter);
        uint256 id = item.mint(alice, CAT, 0, _ref(1));

        vm.prank(alice);
        item.burn(id);
        assertEq(item.totalSupply(), 0);
        assertEq(item.totalMinted(), 1);

        vm.prank(minter);
        vm.expectRevert(bytes("mintRef used"));
        item.mint(alice, CAT, 0, _ref(1));

        // and the record of what it was survives the burn
        (uint32 catalogId,, bytes32 ref) = item.instanceOf(id);
        assertEq(catalogId, CAT);
        assertEq(ref, _ref(1));
    }

    function test_burn_is_only_for_the_holder() public {
        vm.prank(minter);
        uint256 id = item.mint(alice, CAT, 0, _ref(1));
        vm.prank(bob);
        vm.expectRevert(bytes("not authorized"));
        item.burn(id);
    }

    function test_rejects_zero_ref_and_unknown_kind() public {
        vm.startPrank(minter);
        vm.expectRevert(bytes("zero mintRef"));
        item.mint(alice, CAT, 0, bytes32(0));
        vm.expectRevert(bytes("bad kind"));
        item.mint(alice, CAT, 4, _ref(1));
        vm.stopPrank();
    }

    // ── the store shape: N copies of one catalog ─────────────────────────────

    function test_batch_is_the_store_edition_shape() public {
        bytes32[] memory refs = new bytes32[](3);
        for (uint256 i = 0; i < 3; i++) {
            refs[i] = keccak256(abi.encodePacked(CAT, i));
        }
        uint8 kindStore = item.KIND_STORE(); // before the prank — see above
        vm.prank(minter);
        uint256 first = item.mintBatch(alice, CAT, kindStore, refs);

        assertEq(first, 1);
        assertEq(item.totalSupply(), 3);
        assertEq(item.balanceOf(alice), 3);
        // identical catalog, distinct tokens — what CS and Steam already render
        for (uint256 i = 0; i < 3; i++) {
            (uint32 catalogId, uint8 kind,) = item.instanceOf(first + i);
            assertEq(catalogId, CAT);
            assertEq(kind, 2);
        }
    }

    // ── mint authority: disclosed, and renounceable for real ─────────────────

    function test_renounce_closes_the_mint_forever() public {
        vm.prank(creator);
        item.renounceMint();

        vm.prank(minter);
        vm.expectRevert(bytes("mint closed"));
        item.mint(alice, CAT, 0, _ref(1));

        vm.startPrank(creator);
        vm.expectRevert(bytes("mint closed"));
        item.setMinter(alice, true);
        vm.expectRevert(bytes("mint closed"));
        item.setDropRoot(bytes32(uint256(1)));
        vm.stopPrank();

        assertTrue(item.mintClosed());
    }

    /// The caveat in the header, held to: renouncing must not strand a
    /// claimant who already holds a valid proof.
    function test_renounce_does_not_strand_a_standing_root() public {
        (bytes32 root, bytes32[] memory proof) = _twoLeafTree(alice, CAT, 0, _ref(1), bob, CAT, 0, _ref(2));
        vm.startPrank(creator);
        item.setDropRoot(root);
        item.renounceMint();
        vm.stopPrank();

        uint256 id = item.claim(alice, CAT, 0, _ref(1), proof);
        assertEq(item.ownerOf(id), alice);
    }

    function test_mintStatus_says_bounded_not_final_while_a_root_lives() public {
        vm.startPrank(creator);
        item.setDropRoot(bytes32(uint256(0xabc)));
        item.renounceMint();
        vm.stopPrank();

        (bool closed, uint256 minters, uint256 supply, bool rootLive, bytes32 root) = item.mintStatus();
        assertTrue(closed);
        assertEq(minters, 1);
        assertEq(supply, 0);
        assertTrue(rootLive);
        assertEq(root, bytes32(uint256(0xabc)));
    }

    function test_minter_count_tracks_both_ways() public {
        vm.startPrank(creator);
        item.setMinter(alice, true);
        assertEq(item.minterCount(), 2);
        item.setMinter(alice, true); // idempotent, no double count
        assertEq(item.minterCount(), 2);
        item.setMinter(alice, false);
        assertEq(item.minterCount(), 1);
        item.setMinter(alice, false); // idempotent, no underflow
        assertEq(item.minterCount(), 1);
        vm.stopPrank();
    }

    // ── the drop rail ────────────────────────────────────────────────────────

    function _leaf(address w, uint32 c, uint8 k, bytes32 r) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(w, c, k, r));
    }

    /// A two-leaf tree, sorted-pair hashed — the repo's one merkle dialect.
    /// Returns the root and the proof for the FIRST leaf.
    function _twoLeafTree(
        address w1,
        uint32 c1,
        uint8 k1,
        bytes32 r1,
        address w2,
        uint32 c2,
        uint8 k2,
        bytes32 r2
    ) internal pure returns (bytes32 root, bytes32[] memory proof) {
        bytes32 a = _leaf(w1, c1, k1, r1);
        bytes32 b = _leaf(w2, c2, k2, r2);
        root = a < b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
        proof = new bytes32[](1);
        proof[0] = b;
    }

    function test_claim_mints_to_the_wallet_in_the_leaf() public {
        (bytes32 root, bytes32[] memory proof) = _twoLeafTree(alice, CAT, 0, _ref(1), bob, CAT, 0, _ref(2));
        vm.prank(creator);
        item.setDropRoot(root);

        // callable by anyone FOR anyone — bob pays gas, alice gets the token
        vm.prank(bob);
        uint256 id = item.claim(alice, CAT, 0, _ref(1), proof);
        assertEq(item.ownerOf(id), alice);
    }

    function test_claim_rejects_a_bad_proof_and_a_swapped_wallet() public {
        (bytes32 root, bytes32[] memory proof) = _twoLeafTree(alice, CAT, 0, _ref(1), bob, CAT, 0, _ref(2));
        vm.prank(creator);
        item.setDropRoot(root);

        vm.expectRevert(bytes("bad proof"));
        item.claim(bob, CAT, 0, _ref(1), proof); // alice's leaf, bob's name

        vm.expectRevert(bytes("bad proof"));
        item.claim(alice, CAT, 0, _ref(3), proof); // a ref nobody committed to

        vm.expectRevert(bytes("bad proof"));
        item.claim(alice, CAT + 1, 0, _ref(1), proof); // a catalog nobody committed to
    }

    function test_claim_cannot_be_replayed() public {
        (bytes32 root, bytes32[] memory proof) = _twoLeafTree(alice, CAT, 0, _ref(1), bob, CAT, 0, _ref(2));
        vm.prank(creator);
        item.setDropRoot(root);

        item.claim(alice, CAT, 0, _ref(1), proof);
        vm.expectRevert(bytes("mintRef used"));
        item.claim(alice, CAT, 0, _ref(1), proof);
    }

    function test_claim_needs_a_root_at_all() public {
        bytes32[] memory proof = new bytes32[](0);
        vm.expectRevert(bytes("no drop root"));
        item.claim(alice, CAT, 0, _ref(1), proof);
    }

    // ── the compatibility rule: a boring ERC-721 ─────────────────────────────

    function test_the_gacha_custody_path_works_end_to_end() public {
        GachaLike gacha = new GachaLike();
        vm.prank(minter);
        uint256 id = item.mint(alice, CAT, 0, _ref(1));

        vm.prank(alice);
        item.approve(address(gacha), id);

        // deposit: plain transferFrom, then the ownerOf assertion
        gacha.pull(item, alice, id);
        assertEq(item.ownerOf(id), address(gacha));

        // and back out to a buyer, the way settle does
        gacha.push(item, bob, id);
        assertEq(item.ownerOf(id), bob);
    }

    function test_transfer_clears_the_approval() public {
        vm.prank(minter);
        uint256 id = item.mint(alice, CAT, 0, _ref(1));
        vm.prank(alice);
        item.approve(bob, id);
        vm.prank(bob);
        item.transferFrom(alice, bob, id);
        assertEq(item.getApproved(id), address(0));
        assertEq(item.balanceOf(alice), 0);
        assertEq(item.balanceOf(bob), 1);
    }

    function test_transfer_refuses_a_wrong_from_and_an_unapproved_caller() public {
        vm.prank(minter);
        uint256 id = item.mint(alice, CAT, 0, _ref(1));

        vm.prank(alice);
        vm.expectRevert(bytes("from != owner"));
        item.transferFrom(bob, bob, id);

        vm.prank(bob);
        vm.expectRevert(bytes("not authorized"));
        item.transferFrom(alice, bob, id);
    }

    function test_operator_approval_works() public {
        vm.prank(minter);
        uint256 id = item.mint(alice, CAT, 0, _ref(1));
        vm.prank(alice);
        item.setApprovalForAll(bob, true);
        assertTrue(item.isApprovedForAll(alice, bob));
        vm.prank(bob);
        item.transferFrom(alice, bob, id);
        assertEq(item.ownerOf(id), bob);
    }

    function test_safe_transfer_surface() public {
        ItemGoodReceiver good = new ItemGoodReceiver();
        ItemBadReceiver bad = new ItemBadReceiver();
        vm.startPrank(minter);
        uint256 id1 = item.mint(alice, CAT, 0, _ref(1));
        uint256 id2 = item.mint(alice, CAT, 0, _ref(2));
        vm.stopPrank();

        vm.startPrank(alice);
        item.safeTransferFrom(alice, address(good), id1);
        assertEq(item.ownerOf(id1), address(good));

        vm.expectRevert(bytes("unsafe recipient"));
        item.safeTransferFrom(alice, address(bad), id2);
        vm.stopPrank();
    }

    // ── royalty and interfaces ───────────────────────────────────────────────

    function test_royalty_is_what_was_deployed_and_cannot_move() public view {
        (address receiver, uint256 amount) = item.royaltyInfo(1, 1 ether);
        assertEq(receiver, payout);
        assertEq(amount, 0.05 ether); // 500 bps
    }

    function test_royalty_cap_and_zero_receiver_rule() public {
        vm.expectRevert(bytes("royalty > 10%"));
        new ScryItem("x", "X", payout, 1001, "");

        vm.expectRevert(bytes("zero receiver"));
        new ScryItem("x", "X", address(0), 1, "");

        // the house shape: zero bps, and then the receiver may be anything
        ScryItem free = new ScryItem("x", "X", address(0), 0, "");
        (, uint256 amount) = free.royaltyInfo(1, 1 ether);
        assertEq(amount, 0);
    }

    function test_supports_the_interfaces_a_marketplace_probes() public view {
        assertTrue(item.supportsInterface(0x01ffc9a7)); // ERC-165
        assertTrue(item.supportsInterface(0x80ac58cd)); // ERC-721
        assertTrue(item.supportsInterface(0x5b5e139f)); // metadata
        assertTrue(item.supportsInterface(0x49064906)); // ERC-4906
        assertTrue(item.supportsInterface(0x2a55205a)); // ERC-2981
        assertFalse(item.supportsInterface(0xd9b67a26)); // ERC-1155 — deliberately not
    }

    // ── metadata: art moves, truth does not ──────────────────────────────────

    function test_base_uri_repoints_art_without_touching_the_reference() public {
        vm.prank(minter);
        uint256 id = item.mint(alice, CAT, 0, _ref(1));
        assertEq(item.tokenURI(id), "https://scry.moreright.xyz/item/1/metadata");

        vm.prank(creator);
        item.setBaseURI("ipfs://somewhere");
        assertEq(item.tokenURI(id), "ipfs://somewhere/item/1/metadata");
        assertEq(item.mintRefOf(id), _ref(1)); // unmoved, and unmovable
    }

    function test_no_such_token_reads_revert() public {
        vm.expectRevert(bytes("no such token"));
        item.tokenURI(1);
        vm.expectRevert(bytes("no such token"));
        item.ownerOf(1);
        vm.expectRevert(bytes("no such token"));
        item.mintRefOf(1);
    }
}
