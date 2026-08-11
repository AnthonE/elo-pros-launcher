// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryCovenant.sol";

contract ScryCovenantTest is Test {
    ScryCovenant cov;
    bytes32 cid = keccak256("momo-fleet-2026");
    string oath = "never risk more than 5% of the book on one position; report every 24h";
    bytes32 th;

    address momo1 = address(0xA6E27);
    address momo2 = address(0xB0B);
    address momo3 = address(0xC0FFEE);

    function setUp() public {
        cov = new ScryCovenant();
        th = sha256(bytes(oath));
    }

    function _open() internal {
        vm.warp(1000);
        vm.prank(momo1);
        cov.open(cid, th, 24, "momo fleet - the 5% oath", oath);
    }

    function test_openEmitsFullText() public {
        vm.warp(1000);
        vm.expectEmit(true, true, false, true);
        emit ScryCovenant.CovenantOpened(cid, momo1, th, 24, "momo fleet - the 5% oath", oath);
        vm.prank(momo1);
        cov.open(cid, th, 24, "momo fleet - the 5% oath", oath);
        (address opener,, uint32 cadence, uint64 openedAt, uint32 members, uint32 active) = cov.covenants(cid);
        assertEq(opener, momo1);
        assertEq(cadence, 24);
        assertEq(openedAt, 1000);
        assertEq(members, 0);
        assertEq(active, 0);
    }

    function test_openRejectsBadTextHash() public {
        vm.prank(momo1);
        vm.expectRevert(ScryCovenant.BadTextHash.selector);
        cov.open(cid, keccak256("wrong"), 24, "label", oath);
    }

    function test_openIsOnce() public {
        _open();
        vm.prank(momo2);
        vm.expectRevert(ScryCovenant.CovenantExists.selector);
        cov.open(cid, th, 24, "label", oath);
    }

    function test_fleetSwearsInOrder() public {
        _open();
        vm.prank(momo1);
        assertEq(cov.swear(cid), 1);
        vm.prank(momo2);
        assertEq(cov.swear(cid), 2);
        vm.prank(momo3);
        assertEq(cov.swear(cid), 3);
        (,,,, uint32 members, uint32 active) = cov.covenants(cid);
        assertEq(members, 3);
        assertEq(active, 3);
        (uint32 seq, uint64 swornAt, uint64 renouncedAt) = cov.membership(cid, momo2);
        assertEq(seq, 2);
        assertEq(swornAt, 1000);
        assertEq(renouncedAt, 0);
        assertTrue(cov.isActiveMember(cid, momo2));
    }

    function test_cannotSwearTwice() public {
        _open();
        vm.prank(momo1);
        cov.swear(cid);
        vm.prank(momo1);
        vm.expectRevert(ScryCovenant.AlreadySworn.selector);
        cov.swear(cid);
    }

    function test_cannotSwearUnopened() public {
        vm.prank(momo1);
        vm.expectRevert(ScryCovenant.NoSuchCovenant.selector);
        cov.swear(keccak256("ghost"));
    }

    function test_renounceIsRecordedNotErased() public {
        _open();
        vm.prank(momo1);
        cov.swear(cid);
        vm.prank(momo2);
        cov.swear(cid);
        vm.warp(2000);
        vm.expectEmit(true, true, false, true);
        emit ScryCovenant.Renounced(cid, momo1, 1, "found a better book", 2000);
        vm.prank(momo1);
        cov.renounce(cid, "found a better book");
        // history preserved: seq + swornAt stay, renouncedAt stamped
        (uint32 seq, uint64 swornAt, uint64 renouncedAt) = cov.membership(cid, momo1);
        assertEq(seq, 1);
        assertEq(swornAt, 1000);
        assertEq(renouncedAt, 2000);
        // memberCount unchanged; activeCount drops
        (,,,, uint32 members, uint32 active) = cov.covenants(cid);
        assertEq(members, 2);
        assertEq(active, 1);
        assertFalse(cov.isActiveMember(cid, momo1));
        assertTrue(cov.everSwore(cid, momo1)); // still on the historical roster
    }

    function test_cannotRenounceTwiceOrReSwear() public {
        _open();
        vm.prank(momo1);
        cov.swear(cid);
        vm.prank(momo1);
        cov.renounce(cid, "walking");
        vm.prank(momo1);
        vm.expectRevert(ScryCovenant.AlreadyRenounced.selector);
        cov.renounce(cid, "walking again");
        // the breach is a fact, not a phase: cannot re-swear
        vm.prank(momo1);
        vm.expectRevert(ScryCovenant.AlreadySworn.selector);
        cov.swear(cid);
    }

    function test_cannotRenounceUnsworn() public {
        _open();
        vm.prank(momo2);
        vm.expectRevert(ScryCovenant.NotAMember.selector);
        cov.renounce(cid, "never swore");
    }

    // ── Sworn event payload (the register's per-member line) ────────────────
    function test_swearEmitsSworn() public {
        _open();
        vm.expectEmit(true, true, false, true);
        emit ScryCovenant.Sworn(cid, momo2, 1, 1000); // _open warped to 1000
        vm.prank(momo2);
        cov.swear(cid);
    }

    // ── covenantCount (never asserted before) ───────────────────────────────
    function test_covenantCountIncrements() public {
        assertEq(cov.covenantCount(), 0);
        _open();
        assertEq(cov.covenantCount(), 1);
        bytes32 cid2 = keccak256("second-fleet-2026");
        vm.prank(momo2);
        cov.open(cid2, th, 12, "second fleet", oath);
        assertEq(cov.covenantCount(), 2);
    }

    // ── posted length bounds, exact revert strings ──────────────────────────
    function test_openRejectsZeroTextHash() public {
        vm.expectRevert(ScryCovenant.BadTextHash.selector);
        cov.open(cid, bytes32(0), 24, "label", "");
    }

    function test_labelBounds() public {
        vm.expectRevert(bytes("label 1..128 chars"));
        cov.open(cid, th, 24, "", oath);
        string memory label129 = _fill(129, "L");
        vm.expectRevert(bytes("label 1..128 chars"));
        cov.open(cid, th, 24, label129, oath);
    }

    function test_textBounds() public {
        // sha256 is a precompile CALL — computed before any pending cheatcode
        bytes32 emptyHash = sha256(bytes(""));
        vm.expectRevert(bytes("text 1..2000 chars"));
        cov.open(cid, emptyHash, 24, "label", "");
        string memory text2001 = _fill(2001, "t");
        bytes32 longHash = sha256(bytes(text2001));
        vm.expectRevert(bytes("text 1..2000 chars"));
        cov.open(cid, longHash, 24, "label", text2001);
    }

    function test_renounceReasonBound() public {
        _open();
        vm.prank(momo1);
        cov.swear(cid);
        string memory reason513 = _fill(513, "r");
        vm.prank(momo1);
        vm.expectRevert(bytes("reason <= 512 chars"));
        cov.renounce(cid, reason513);
    }

    function _fill(uint256 n, bytes1 c) internal pure returns (string memory) {
        bytes memory b = new bytes(n);
        for (uint256 i = 0; i < n; i++) {
            b[i] = c;
        }
        return string(b);
    }
}
