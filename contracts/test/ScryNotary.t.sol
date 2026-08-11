// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryNotary.sol";

contract ScryNotaryTest is Test {
    ScryNotary notary;
    address momo = address(0xA6E27);
    address rival = address(0xBAD);

    function setUp() public {
        notary = new ScryNotary();
    }

    function test_notarizeRecordsFirstSeen() public {
        bytes32 h = keccak256("ETH closes above 3500 friday");
        vm.warp(1000);
        vm.prank(momo);
        uint256 seq = notary.notarize(h, "prediction: ETH > 3500 fri", "sworn by momo-9");
        (address by, uint64 at, uint256 s) = notary.firstSeen(h);
        assertEq(by, momo);
        assertEq(at, 1000);
        assertEq(s, seq);
    }

    function test_firstSeenNeverChanges() public {
        bytes32 h = keccak256("the same claim");
        vm.prank(momo);
        notary.notarize(h, "claim", "first");
        vm.warp(block.timestamp + 100);
        vm.prank(rival);
        notary.notarize(h, "claim", "late copy");
        (address by,,) = notary.firstSeen(h);
        assertEq(by, momo); // priority holds
        assertEq(notary.count(), 2); // but both notarizations are logged
    }

    function test_labelBounds() public {
        vm.expectRevert(bytes("label 1..128 chars"));
        notary.notarize(bytes32(0), "", "x");
    }

    function test_checkReveal() public view {
        bytes32 h = keccak256(bytes("the seed"));
        assertTrue(notary.checkReveal(h, "the seed"));
        assertFalse(notary.checkReveal(h, "not the seed"));
    }

    // ── the product lives in the log: full Notarized payload asserted ───────
    // First commit carries firstForHash=true; a rival re-commit of the SAME
    // hash still logs (seq 2) but carries firstForHash=false — the explorer
    // line itself shows who was first.
    function test_notarizedEventFirstCommitAndRivalRecommit() public {
        bytes32 h = keccak256("ETH closes above 3500 friday");
        vm.warp(1000);
        vm.expectEmit(true, true, false, true);
        emit ScryNotary.Notarized(momo, h, "prediction: ETH > 3500 fri", "sworn by momo-9", 1000, 1, true);
        vm.prank(momo);
        uint256 seq1 = notary.notarize(h, "prediction: ETH > 3500 fri", "sworn by momo-9");
        assertEq(seq1, 1); // absolute — the first notarization ever is seq 1

        vm.warp(1100);
        vm.expectEmit(true, true, false, true);
        emit ScryNotary.Notarized(rival, h, "prediction: ETH > 3500 fri", "late echo", 1100, 2, false);
        vm.prank(rival);
        uint256 seq2 = notary.notarize(h, "prediction: ETH > 3500 fri", "late echo");
        assertEq(seq2, 2);
    }

    // ── posted length bounds, exact revert strings ──────────────────────────
    function test_labelUpperBound() public {
        bytes32 h = keccak256("bounded");
        string memory label129 = _fill(129, "L");
        vm.expectRevert(bytes("label 1..128 chars"));
        notary.notarize(h, label129, "memo");
    }

    function test_memoUpperBound() public {
        bytes32 h = keccak256("bounded");
        string memory memo513 = _fill(513, "m");
        vm.expectRevert(bytes("memo <= 512 chars"));
        notary.notarize(h, "label", memo513);
    }

    function _fill(uint256 n, bytes1 c) internal pure returns (string memory) {
        bytes memory b = new bytes(n);
        for (uint256 i = 0; i < n; i++) {
            b[i] = c;
        }
        return string(b);
    }
}
