// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloEidolon.sol";
import "../src/EloTicket.sol";

/// ERC-721 receivers for the safeTransfer surface: one accepts (returns the
/// magic selector), one rejects (wrong selector -> "unsafe recipient").
contract EidolonGoodReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0x150b7a02; // IERC721Receiver.onERC721Received.selector
    }
}

contract EidolonBadReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0xdeadbeef; // not the magic value
    }
}

contract MockSCRY {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 a) external {
        balanceOf[to] += a;
    }

    function approve(address s, uint256 a) external returns (bool) {
        allowance[msg.sender][s] = a;
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

/// A ticket contract whose cap disagrees with the vessels' — the eidolon
/// constructor must refuse it, because tickets:vessels is 1:1 by design.
contract WrongCapTicket {
    uint256 public constant MAX_SUPPLY = 999;
}

contract EloEidolonTest is Test {
    MockSCRY reserve;
    EloTicket tick;
    EloEidolon eid;
    address splitter = address(0xFEE);
    address proceeds = address(0xCA5);
    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    bytes32 commitment = sha256("test-salt");

    function setUp() public {
        reserve = new MockSCRY();
        // trials carries the whole run in these tests: the tranche split is
        // the ticket suite's business, the vessel suite just needs tickets.
        tick = new EloTicket(1000, 0, 0, 2 hours, proceeds, 0, "https://elopros.com/api");
        eid = new EloEidolon(
            IERC20(address(reserve)), IEloTicket(address(tick)), splitter, 0, commitment, 500,
            "https://elopros.com/api"
        );
        tick.setEidolon(address(eid));
    }

    // ── merkle helpers (mirror eidolon_passes.py: sorted-pair keccak) ──────────
    function _leaf(address a, uint256 al) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, al));
    }

    /// Hand `who` `n` tickets through a solo trials root (root == leaf).
    function _giveTickets(address who, uint256 n) internal returns (uint256 firstId) {
        tick.setTrialsRoot(_leaf(who, 1001)); // allowance high enough for any test
        bytes32[] memory empty = new bytes32[](0);
        for (uint256 i = 0; i < n; i++) {
            vm.prank(who);
            uint256 id = tick.claimTrials(1001, empty);
            if (i == 0) firstId = id;
        }
    }

    // ── the ticket gate ────────────────────────────────────────────────────────
    function test_mintRevertsWithoutATicket() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(bob); // bob holds nothing — alice's ticket is not his to burn
        vm.expectRevert(bytes("not your ticket"));
        eid.mint(id);
        vm.prank(bob);
        vm.expectRevert(bytes("no such ticket")); // and a ticket that never existed
        eid.mint(777);
    }

    function test_mintBurnsTheTicketAndSummons() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(alice);
        uint256 tokenId = eid.mint(id);
        assertEq(tokenId, 1);
        assertEq(eid.ownerOf(1), alice);
        assertEq(eid.totalMinted(), 1);
        assertGt(eid.mintedAt(1), 0);
        assertEq(tick.totalBurned(), 1); // the ticket is gone…
        vm.expectRevert(bytes("no such ticket"));
        tick.ownerOf(id); // …not parked, gone
    }

    function test_aBurnedTicketNeverMintsAgain() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(alice);
        eid.mint(id);
        vm.prank(alice);
        vm.expectRevert(bytes("no such ticket"));
        eid.mint(id);
        assertEq(eid.totalMinted(), 1);
    }

    /// The OpenSea path: a ticket bought secondhand summons exactly like an
    /// earned one — the vessel contract never asks where a ticket came from.
    function test_aTradedTicketSummons() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(alice);
        tick.transferFrom(alice, bob, id);
        vm.prank(bob);
        uint256 tokenId = eid.mint(id);
        assertEq(eid.ownerOf(tokenId), bob);
    }

    function test_priceZeroPullsNoReserve() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(alice); // no SCRY balance, no approval — the ticket is the whole price
        eid.mint(id);
        assertEq(reserve.balanceOf(splitter), 0);
    }

    function test_pricedMintStillPullsEloToTheSplitter() public {
        uint256 price = 25_000e18;
        EloTicket t2 = new EloTicket(1000, 0, 0, 2 hours, proceeds, 0, "x");
        EloEidolon priced = new EloEidolon(
            IERC20(address(reserve)), IEloTicket(address(t2)), splitter, price, commitment, 500, "x"
        );
        t2.setEidolon(address(priced));
        t2.setTrialsRoot(_leaf(alice, 2));
        bytes32[] memory empty = new bytes32[](0);
        vm.startPrank(alice);
        uint256 id = t2.claimTrials(2, empty);
        vm.stopPrank();
        reserve.mint(alice, price);
        vm.prank(alice);
        reserve.approve(address(priced), price);
        vm.prank(alice);
        priced.mint(id);
        assertEq(reserve.balanceOf(splitter), price);
        // a second ticket without SCRY behind it fails before any state moves
        vm.prank(alice);
        uint256 id2 = t2.claimTrials(2, empty);
        vm.prank(alice);
        vm.expectRevert();
        priced.mint(id2);
        assertEq(priced.totalMinted(), 1);
    }

    // ── supply / provenance (unchanged invariants) ─────────────────────────────
    function test_hardCapAtTheThousand() public {
        _giveTickets(alice, 1000); // the full run of tickets…
        vm.startPrank(alice);
        for (uint256 i = 1; i <= 1000; i++) {
            eid.mint(i);
        }
        vm.stopPrank();
        assertEq(eid.totalMinted(), 1000);
        assertEq(tick.totalBurned(), 1000);
        // …and no 1001st ticket can exist to summon with, through any door
        tick.setPublicSale(1 ether, 10);
        vm.deal(bob, 1 ether);
        vm.prank(bob);
        vm.expectRevert(bytes("the thousand tickets exist"));
        tick.buyPublic{value: 1 ether}();
    }

    function test_idsAreSequentialAcrossWallets() public {
        tick.setTrialsRoot(_pair(_leaf(alice, 2), _leaf(bob, 1)));
        bytes32[] memory aProof = new bytes32[](1);
        aProof[0] = _leaf(bob, 1);
        bytes32[] memory bProof = new bytes32[](1);
        bProof[0] = _leaf(alice, 2);
        vm.prank(alice);
        uint256 tA1 = tick.claimTrials(2, aProof);
        vm.prank(bob);
        uint256 tB = tick.claimTrials(1, bProof);
        vm.prank(alice);
        uint256 tA2 = tick.claimTrials(2, aProof);
        vm.prank(alice);
        assertEq(eid.mint(tA1), 1);
        vm.prank(bob);
        assertEq(eid.mint(tB), 2);
        vm.prank(alice);
        assertEq(eid.mint(tA2), 3);
        assertEq(eid.totalMinted(), 3);
    }

    function _pair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return a <= b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
    }

    function test_commitmentIsImmutablySet() public view {
        assertEq(eid.saltCommitment(), commitment);
    }

    function test_constructorRefusesZeroCommitment() public {
        vm.expectRevert(bytes("zero commitment"));
        new EloEidolon(IERC20(address(reserve)), IEloTicket(address(tick)), splitter, 0, bytes32(0), 500, "x");
    }

    function test_constructorRefusesFatRoyalty() public {
        vm.expectRevert(bytes("royalty > 10%"));
        new EloEidolon(IERC20(address(reserve)), IEloTicket(address(tick)), splitter, 0, commitment, 1001, "x");
    }

    function test_constructorRefusesZeroTicket() public {
        vm.expectRevert(bytes("zero ticket"));
        new EloEidolon(IERC20(address(reserve)), IEloTicket(address(0)), splitter, 0, commitment, 500, "x");
    }

    /// tickets:vessels is 1:1 — a ticket contract capped at anything but 1000
    /// would either strand tickets or leave vessels unreachable.
    function test_constructorRefusesMismatchedTicketSupply() public {
        WrongCapTicket wrong = new WrongCapTicket();
        vm.expectRevert(bytes("ticket supply mismatch"));
        new EloEidolon(IERC20(address(reserve)), IEloTicket(address(wrong)), splitter, 0, commitment, 500, "x");
    }

    function test_royaltyInfoPaysTheSplitter() public view {
        (address to, uint256 amt) = eid.royaltyInfo(1, 10_000e18);
        assertEq(to, splitter);
        assertEq(amt, 500e18); // 5% of 10k
    }

    // ── ERC-721 surface (unchanged) ────────────────────────────────────────────
    function test_tokenURIAndContractURI() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(alice);
        eid.mint(id);
        assertEq(eid.tokenURI(1), "https://elopros.com/api/eidolon/1/metadata");
        assertEq(eid.contractURI(), "https://elopros.com/api/eidolon/collection");
        vm.expectRevert(bytes("no such token"));
        eid.tokenURI(2); // unminted → no URI
    }

    function test_transferFlow() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(alice);
        eid.mint(id);
        vm.prank(bob);
        vm.expectRevert(bytes("not authorized")); // a stranger can't move it
        eid.transferFrom(alice, bob, 1);
        vm.prank(alice);
        eid.transferFrom(alice, bob, 1);
        assertEq(eid.ownerOf(1), bob);
        assertEq(eid.balanceOf(bob), 1);
        assertEq(eid.balanceOf(alice), 0);
    }

    function test_approvedOperatorCanMove() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(alice);
        eid.mint(id);
        vm.prank(alice);
        eid.approve(bob, 1);
        vm.prank(bob);
        eid.transferFrom(alice, bob, 1);
        assertEq(eid.ownerOf(1), bob);
        assertEq(eid.getApproved(1), address(0)); // approval cleared on transfer
    }

    function test_supportsInterfaces() public view {
        assertTrue(eid.supportsInterface(0x01ffc9a7)); // ERC-165
        assertTrue(eid.supportsInterface(0x80ac58cd)); // ERC-721
        assertTrue(eid.supportsInterface(0x5b5e139f)); // ERC-721 Metadata
        assertTrue(eid.supportsInterface(0x49064906)); // ERC-4906 (metadata update)
        assertTrue(eid.supportsInterface(0x2a55205a)); // ERC-2981
        assertFalse(eid.supportsInterface(0xdeadbeef));
    }

    function test_ownerCanRepointBaseURI() public {
        uint256 id = _giveTickets(alice, 1);
        vm.prank(alice);
        eid.mint(id);
        assertEq(eid.owner(), address(this)); // deployer is owner
        eid.setBaseURI("https://ipfs.example/reserve");
        assertEq(eid.tokenURI(1), "https://ipfs.example/reserve/eidolon/1/metadata");
        assertEq(eid.contractURI(), "https://ipfs.example/reserve/eidolon/collection");
    }

    function test_onlyOwnerAdmin() public {
        vm.prank(alice);
        vm.expectRevert(bytes("not owner")); // a stranger can't set metadata
        eid.setBaseURI("x");
        eid.transferOwnership(bob);
        assertEq(eid.owner(), bob);
        vm.expectRevert(bytes("not owner")); // old owner lost admin
        eid.setBaseURI("y");
    }

    /// safeTransferFrom into a CONTRACT: accepted by a good receiver, refused
    /// by a bad one. mint() uses plain _mint, but any escrow/marketplace
    /// moving a vessel calls safeTransferFrom — the receiver check must hold.
    function test_safeTransferFrom_receiver_accept_and_reject() public {
        uint256 first = _giveTickets(alice, 2);
        vm.prank(alice);
        uint256 id1 = eid.mint(first);
        vm.prank(alice);
        uint256 id2 = eid.mint(first + 1);

        EidolonGoodReceiver good = new EidolonGoodReceiver();
        vm.prank(alice);
        eid.safeTransferFrom(alice, address(good), id1);
        assertEq(eid.ownerOf(id1), address(good)); // accepted

        EidolonBadReceiver bad = new EidolonBadReceiver();
        vm.prank(alice);
        vm.expectRevert(bytes("unsafe recipient"));
        eid.safeTransferFrom(alice, address(bad), id2);
        assertEq(eid.ownerOf(id2), alice); // refused, custody unmoved
    }
}
