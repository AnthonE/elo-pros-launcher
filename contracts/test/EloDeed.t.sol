// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloDeed.sol";
import {SpoilsToken} from "../src/SpoilsToken.sol";

/// A recipient that accepts safe transfers (returns the magic selector).
contract DeedReceiver is IERC721Receiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return IERC721Receiver.onERC721Received.selector;
    }
}

/// A recipient that rejects safe transfers (wrong selector).
contract DeedRejector {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0xdeadbeef;
    }
}

/// The suite runs against the REAL SpoilsToken (elastic OBOL: cap 0, this test
/// contract as minter) — not a mock. The deed's sink rides on the real token's
/// burnFrom semantics: allowance checked FIRST ("allowance", then "balance"),
/// and an infinite allowance is never decremented (SpoilsToken.sol:72-77).
contract EloDeedTest is Test {
    SpoilsToken obol;
    EloDeed deed;
    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    address steward = address(0x57E);
    address operator = address(0x09E7A707);
    uint256 constant FOUND = 10e18;
    uint256 constant CONVEY = 5e18;

    // scope bits hoisted ONCE — never call deed getters in an argument list
    // after vm.prank/vm.expectRevert (the staticcall would consume the
    // cheatcode and the test would misfire).
    uint32 SC; // craft
    uint32 SP; // price
    uint32 SD; // deliver

    // events redeclared for vm.expectEmit
    event Founded(uint256 indexed tokenId, uint16 streetIndex, uint16 plot, address indexed to, uint256 burned);
    event Conveyed(uint256 indexed tokenId, address indexed from, address indexed to, uint256 burned);
    event KeyCut(uint256 indexed tokenId, address indexed steward, uint32 scopes, uint64 until);
    event KeyRevoked(uint256 indexed tokenId);

    function setUp() public {
        obol = new SpoilsToken("obol", "OBOL", 0, address(this)); // elastic, we mint
        deed = new EloDeed(ISpoils(address(obol)), FOUND, CONVEY, "https://elopros.com/api");
        obol.mint(alice, 1_000e18);
        obol.mint(bob, 1_000e18);
        obol.mint(operator, 1_000e18);
        SC = deed.SCOPE_CRAFT();
        SP = deed.SCOPE_PRICE();
        SD = deed.SCOPE_DELIVER();
    }

    function _found(address who, uint16 street, uint16 plot) internal returns (uint256 id) {
        vm.startPrank(who);
        obol.approve(address(deed), FOUND);
        id = deed.found(street, plot);
        vm.stopPrank();
    }

    // ── the toll must not make deeds untradeable (audit 2026-07-25) ───────
    //    WAS: the toll burned from `msg.sender`, which on any marketplace is
    //    the ROUTER. Routers hold no OBOL and grant no allowance, so every
    //    deed was effectively non-tradeable off-site — killing the churn that
    //    IS the burn (SENTENCES.md 2026-07-22). It now falls on the departing
    //    owner when they have prepared, and only otherwise on the caller.
    // ── the seller-pays path is a COUNTED CREDIT, not an allowance ────────
    //    SECURITY REVIEW 2026-07-25. The first cut read "holds OBOL + has an
    //    allowance to this contract" as the seller's consent. It is not:
    //    `found()` needs that allowance anyway, and an ERC-721 operator
    //    approval is UNBOUNDED and survives transfers. Together they let
    //    anyone approved to MOVE a deed instead SPEND the owner's whole OBOL
    //    balance — `transferFrom(alice, alice, id)` was a valid, repeatable,
    //    state-preserving "conveyance" that burned her toll every iteration
    //    while the attacker paid nothing and she kept the deed.
    function test_operatorCannotDrainTheOwnersObolBySelfConveyance() public {
        uint256 id = _found(alice, 9, 9);
        address mkt = address(0x4B17); // a marketplace/manager alice approved
        vm.startPrank(alice);
        obol.approve(address(deed), type(uint256).max); // the unbounded approve
        deed.setApprovalForAll(mkt, true);
        vm.stopPrank();

        uint256 aliceBefore = obol.balanceOf(alice);
        // WAS: this looped until alice's OBOL hit zero. NOW: a deed cannot be
        // conveyed to its own owner at all, so there is no repeatable charge.
        vm.prank(mkt);
        vm.expectRevert(bytes("self-conveyance"));
        deed.transferFrom(alice, alice, id);

        // and a real transfer still cannot spend her OBOL without her having
        // agreed to THIS conveyance: she granted no credit, so the toll falls
        // back to the caller, who holds no OBOL and cannot pay.
        assertFalse(deed.sellerCoversToll(alice), "an allowance alone is not consent");
        vm.prank(mkt);
        vm.expectRevert();
        deed.transferFrom(alice, bob, id);

        assertEq(obol.balanceOf(alice), aliceBefore, "not one wei of hers was burned");
        assertEq(deed.ownerOf(id), alice);
    }

    function test_aConveyanceCreditPaysForExactlyOneConveyance() public {
        uint256 id = _found(alice, 8, 8);
        uint256 id2 = _found(alice, 8, 9);
        address mkt = address(0x4B17);
        vm.startPrank(alice);
        obol.approve(address(deed), type(uint256).max);
        deed.setApprovalForAll(mkt, true);
        deed.allowConveyances(1); // she agrees to pay ONE toll
        vm.stopPrank();

        uint256 aliceBefore = obol.balanceOf(alice);
        vm.prank(mkt);
        deed.transferFrom(alice, bob, id);
        assertEq(obol.balanceOf(alice), aliceBefore - CONVEY, "the one she agreed to");
        assertEq(deed.conveyanceCredits(alice), 0, "and it is spent");

        // the second deed cannot ride on the same agreement
        assertFalse(deed.sellerCoversToll(alice));
        vm.prank(mkt);
        vm.expectRevert();
        deed.transferFrom(alice, bob, id2);
        assertEq(obol.balanceOf(alice), aliceBefore - CONVEY, "no second burn");
    }

    function test_deedSellsThroughARouterThatHoldsNoObol() public {
        uint256 id = _found(alice, 1, 1);
        address router = address(0x40D7E7); // holds no OBOL, grants no allowance
        assertEq(obol.balanceOf(router), 0);

        // alice prepares exactly as a listing UI would tell her to
        vm.startPrank(alice);
        obol.approve(address(deed), CONVEY);
        deed.allowConveyances(1);
        deed.setApprovalForAll(router, true);
        vm.stopPrank();
        assertTrue(deed.sellerCoversToll(alice), "the seller is prepared to pay her own toll");

        uint256 aliceBefore = obol.balanceOf(alice);
        uint256 burnedBefore = obol.totalBurned();
        vm.prank(router); // the router moves the deed, exactly as OpenSea would
        deed.transferFrom(alice, bob, id);

        assertEq(deed.ownerOf(id), bob, "the sale went through");
        assertEq(obol.balanceOf(alice), aliceBefore - CONVEY, "the departing owner paid the toll");
        assertEq(obol.totalBurned(), burnedBefore + CONVEY, "and it was BURNED - the sink still fires");
    }

    function test_unpreparedSellerLeavesTheTollOnTheCaller() public {
        uint256 id = _found(alice, 2, 2);
        vm.prank(alice);
        deed.setApprovalForAll(operator, true); // no OBOL allowance from alice
        assertFalse(deed.sellerCoversToll(alice), "not prepared - the UI should say so");

        uint256 aliceBefore = obol.balanceOf(alice);
        vm.startPrank(operator);
        obol.approve(address(deed), CONVEY);
        deed.transferFrom(alice, bob, id);
        vm.stopPrank();

        assertEq(obol.balanceOf(alice), aliceBefore, "the seller paid nothing");
        assertEq(obol.balanceOf(operator), 1_000e18 - CONVEY, "the caller covered it, as before");
    }

    function test_aRouterCannotMoveADeedNobodyPaidFor() public {
        uint256 id = _found(alice, 3, 3);
        address router = address(0x40D7E7);
        vm.prank(alice);
        deed.setApprovalForAll(router, true); // approved to move it, but no OBOL anywhere
        vm.prank(router);
        vm.expectRevert(); // burnFrom(router) - no allowance, no balance
        deed.transferFrom(alice, bob, id);
        assertEq(deed.ownerOf(id), alice, "no toll, no conveyance - the sink is not optional");
    }

    // ── founding mints the title and burns OBOL (the sink) ──
    function test_foundMintsAndBurnsWithEvent() public {
        vm.startPrank(alice);
        obol.approve(address(deed), FOUND);
        vm.expectEmit(true, true, true, true);
        emit Founded(1, 2, 7, alice, FOUND);
        uint256 id = deed.found(2, 7);
        vm.stopPrank();
        assertEq(id, 1);
        assertEq(deed.ownerOf(1), alice);
        assertEq(deed.balanceOf(alice), 1);
        assertEq(obol.balanceOf(alice), 1_000e18 - FOUND); // burned from the founder
        assertEq(obol.totalSupply(), 3_000e18 - FOUND); // supply retired — a true sink
        assertEq(obol.totalBurned(), FOUND); // the real token's burn ledger agrees
        (uint16 s, uint16 p, uint64 t) = deed.deeds(1);
        assertEq(s, 2);
        assertEq(p, 7);
        assertTrue(t > 0);
    }

    function test_foundRefusesTakenPlot() public {
        _found(alice, 1, 1);
        vm.startPrank(bob);
        obol.approve(address(deed), FOUND);
        vm.expectRevert(bytes("plot taken"));
        deed.found(1, 1);
        vm.stopPrank();
    }

    function test_plotZeroZeroIsARealPlot() public {
        // coord (0,0) packs to key 0 — the free-sentinel read stays correct
        // because the STORED tokenId is nonzero; pin it.
        uint256 id = _found(alice, 0, 0);
        assertEq(deed.ownerOf(id), alice);
        vm.startPrank(bob);
        obol.approve(address(deed), FOUND);
        vm.expectRevert(bytes("plot taken"));
        deed.found(0, 0);
        vm.stopPrank();
    }

    function test_foundRequiresObolApproval() public {
        // the REAL token checks allowance first — its dialect, not a mock's
        vm.prank(alice);
        vm.expectRevert(bytes("allowance"));
        deed.found(3, 3);
    }

    function test_foundInsufficientBalanceReverts() public {
        address pauper = address(0xBAD);
        vm.startPrank(pauper);
        obol.approve(address(deed), FOUND); // approval without funds
        vm.expectRevert(bytes("balance"));
        deed.found(3, 4);
        vm.stopPrank();
    }

    function test_infiniteAllowanceIsNotDecremented() public {
        vm.startPrank(alice);
        obol.approve(address(deed), type(uint256).max);
        deed.found(5, 5);
        vm.stopPrank();
        assertEq(obol.allowance(alice, address(deed)), type(uint256).max); // real-token carve-out
    }

    function test_zeroBurnFoundsWithoutApproval() public {
        EloDeed free = new EloDeed(ISpoils(address(obol)), 0, 0, "u");
        vm.prank(alice);
        uint256 id = free.found(5, 5); // burn 0 → no approval needed
        assertEq(free.ownerOf(id), alice);
    }

    // ── the mancipatio: conveyance burns OBOL, moves the title, clears any key ──
    function test_conveyanceBurnsMovesAndClearsKey() public {
        uint256 id = _found(alice, 4, 2);
        vm.prank(alice);
        deed.grantKey(id, steward, SC, uint64(block.timestamp + 1 days));

        uint256 supplyBefore = obol.totalSupply();
        vm.startPrank(alice);
        obol.approve(address(deed), CONVEY); // the caller pays the toll
        vm.expectEmit(true, true, true, true);
        emit KeyRevoked(id);
        vm.expectEmit(true, true, true, true);
        emit Conveyed(id, alice, bob, CONVEY);
        deed.transferFrom(alice, bob, id);
        vm.stopPrank();

        assertEq(deed.ownerOf(id), bob);
        assertEq(deed.balanceOf(alice), 0);
        assertEq(deed.balanceOf(bob), 1);
        assertEq(obol.balanceOf(alice), 1_000e18 - FOUND - CONVEY); // both burns fired
        assertEq(obol.totalSupply(), supplyBefore - CONVEY); // toll retired, not moved
        (address st,, uint64 until) = deed.keys(id);
        assertEq(st, address(0)); // the key was cleared on conveyance
        assertEq(until, 0);
    }

    function test_approvedOperatorConveysAndPaysTheToll() public {
        // a marketplace-style operator: the TOLL BURNS FROM THE CALLER — the
        // operator — never from the deed owner.
        uint256 id = _found(alice, 4, 3);
        vm.prank(alice);
        deed.approve(operator, id);
        assertEq(deed.getApproved(id), operator);

        vm.startPrank(operator);
        obol.approve(address(deed), CONVEY);
        deed.transferFrom(alice, bob, id);
        vm.stopPrank();

        assertEq(deed.ownerOf(id), bob);
        assertEq(obol.balanceOf(operator), 1_000e18 - CONVEY); // operator paid
        assertEq(obol.balanceOf(alice), 1_000e18 - FOUND); // owner paid only the founding
        assertEq(deed.getApproved(id), address(0)); // approval cleared by transfer
    }

    function test_operatorForAllConveysAndKeyClears() public {
        uint256 id = _found(alice, 4, 4);
        vm.prank(alice);
        deed.grantKey(id, steward, SC, uint64(block.timestamp + 1 days));
        vm.prank(alice);
        deed.setApprovalForAll(operator, true);

        vm.startPrank(operator);
        obol.approve(address(deed), CONVEY);
        deed.transferFrom(alice, bob, id);
        vm.stopPrank();

        assertEq(deed.ownerOf(id), bob);
        assertFalse(deed.keyAllows(id, steward, SC)); // key cleared via the operator path too
    }

    function test_transferRequiresTollObol() public {
        uint256 id = _found(alice, 6, 6);
        vm.prank(alice); // owner, but never approved the conveyance toll
        vm.expectRevert(bytes("allowance"));
        deed.transferFrom(alice, bob, id);
    }

    function test_transferAuthorization() public {
        uint256 id = _found(alice, 7, 7);
        vm.prank(bob); // not owner, not approved
        vm.expectRevert(bytes("not authorized"));
        deed.transferFrom(alice, bob, id);
    }

    function test_transferToZeroAndWrongFromRevert() public {
        uint256 id = _found(alice, 7, 8);
        vm.startPrank(alice);
        obol.approve(address(deed), CONVEY * 2);
        vm.expectRevert(bytes("zero to"));
        deed.transferFrom(alice, address(0), id);
        vm.expectRevert(bytes("from != owner"));
        deed.transferFrom(bob, alice, id);
        vm.stopPrank();
    }

    function test_plotStaysTakenAfterConveyance() public {
        uint256 id = _found(alice, 7, 9);
        vm.startPrank(alice);
        obol.approve(address(deed), CONVEY);
        deed.transferFrom(alice, bob, id);
        vm.stopPrank();
        vm.startPrank(alice);
        obol.approve(address(deed), FOUND);
        vm.expectRevert(bytes("plot taken")); // the plot is the deed's forever
        deed.found(7, 9);
        vm.stopPrank();
    }

    // ── safe transfers: the marketplace path ──
    function test_safeTransferToAcceptingReceiver() public {
        uint256 id = _found(alice, 10, 1);
        DeedReceiver rcv = new DeedReceiver();
        vm.startPrank(alice);
        obol.approve(address(deed), CONVEY);
        deed.safeTransferFrom(alice, address(rcv), id);
        vm.stopPrank();
        assertEq(deed.ownerOf(id), address(rcv));
    }

    function test_safeTransferToRejectingReceiverReverts() public {
        uint256 id = _found(alice, 10, 2);
        DeedRejector bad = new DeedRejector();
        vm.startPrank(alice);
        obol.approve(address(deed), CONVEY);
        vm.expectRevert(bytes("unsafe recipient"));
        deed.safeTransferFrom(alice, address(bad), id);
        vm.stopPrank();
        assertEq(deed.ownerOf(id), alice); // nothing moved
    }

    // ── the key: scoped, revocable, owner-only, never a private key ──
    function test_grantKeyOwnerOnly() public {
        uint256 id = _found(alice, 8, 1);
        uint64 until = uint64(block.timestamp + 1 days);
        vm.prank(bob);
        vm.expectRevert(bytes("not owner"));
        deed.grantKey(id, steward, SP, until);
    }

    function test_keyAllowsScopeAndExpiry() public {
        uint256 id = _found(alice, 8, 2);
        uint32 scopes = SC | SP;
        uint64 until = uint64(block.timestamp + 1 days);
        vm.startPrank(alice);
        vm.expectEmit(true, true, true, true);
        emit KeyCut(id, steward, scopes, until);
        deed.grantKey(id, steward, scopes, until);
        vm.stopPrank();
        assertTrue(deed.keyAllows(id, steward, SC));
        assertTrue(deed.keyAllows(id, steward, SP));
        assertFalse(deed.keyAllows(id, steward, SD)); // scope not granted
        assertFalse(deed.keyAllows(id, bob, SC)); // not the steward
        vm.warp(until); // exactly at expiry: `until > now` fails → key is dead
        assertFalse(deed.keyAllows(id, steward, SC));
    }

    function test_regrantReplacesKey() public {
        uint256 id = _found(alice, 8, 3);
        uint64 until = uint64(block.timestamp + 1 days);
        vm.startPrank(alice);
        deed.grantKey(id, steward, SC, until);
        deed.grantKey(id, bob, SP, until); // cutting again replaces wholesale
        vm.stopPrank();
        assertFalse(deed.keyAllows(id, steward, SC));
        assertTrue(deed.keyAllows(id, bob, SP));
    }

    function test_revokeKey() public {
        uint256 id = _found(alice, 9, 1);
        uint64 until = uint64(block.timestamp + 1 days);
        vm.prank(alice);
        deed.grantKey(id, steward, SC, until);
        assertTrue(deed.keyAllows(id, steward, SC));
        vm.prank(bob);
        vm.expectRevert(bytes("not owner")); // revoke is owner-only too
        deed.revokeKey(id);
        vm.startPrank(alice);
        vm.expectEmit(true, true, true, true);
        emit KeyRevoked(id);
        deed.revokeKey(id);
        vm.stopPrank();
        assertFalse(deed.keyAllows(id, steward, SC));
    }

    function test_grantKeyBadArgsRevert() public {
        uint256 id = _found(alice, 9, 2);
        uint64 future = uint64(block.timestamp + 1 days);
        uint64 now_ = uint64(block.timestamp);
        vm.startPrank(alice);
        vm.expectRevert(bytes("bad key"));
        deed.grantKey(id, address(0), SC, future); // zero steward
        vm.expectRevert(bytes("bad key"));
        deed.grantKey(id, steward, SC, now_); // until not in the future
        vm.stopPrank();
    }

    // ── metadata is fully on-chain — pinned byte-for-byte ──
    // The expected string is generated OFFLINE by transcribing the contract's
    // _svg/_u/_b64 and json assembly exactly (street 3, plot 9, tokenId 1,
    // foundedAt 1000), then json-validated. If this test reds, the contract's
    // metadata changed — update the fixture deliberately.
    string constant EXPECTED_URI =
        "data:application/json;base64,eyJuYW1lIjoic2NyeSBkZWVkICMxIiwiZGVzY3JpcHRpb24iOiJUaXRsZSB0byBvbmUgcGxvdCBvbiB0aGUgc3RyZWV0cyBvZiBzY3J5IC0gc3RyZWV0IDMsIHBsb3QgOS4gQSBkZWVkIGNhcnJpZXMgdGhlIHdhbGxzIGFuZCB0aGUgcGxvdCwgbmV2ZXIgdGhlIG93bmVyIHJlcHV0YXRpb246IHJlcHV0YXRpb24gaXMgc291bGJvdW5kIHRvIHRoZSB2b3cgYXQgU2NyeVJlcHV0YXRpb24gYW5kIGRvZXMgTk9UIG1vdmUgd2l0aCB0aGlzIHRpdGxlLiBGb3VuZGluZyBhbmQgZXZlcnkgY29udmV5YW5jZSBidXJuIE9CT0wgLSB0aGUgZGVlZCBpcyBhIHNpbmsgYnkgZGVzaWduLiIsImV4dGVybmFsX3VybCI6Imh0dHBzOi8vZWxvcHJvcy5jb20vYXBpL3RhYmVybmFlIiwiaW1hZ2UiOiJkYXRhOmltYWdlL3N2Zyt4bWw7YmFzZTY0LFBITjJaeUI0Yld4dWN6MGlhSFIwY0RvdkwzZDNkeTUzTXk1dmNtY3ZNakF3TUM5emRtY2lJSFpwWlhkQ2IzZzlJakFnTUNBek5UQWdNelV3SWo0OGNtVmpkQ0IzYVdSMGFEMGlNelV3SWlCb1pXbG5hSFE5SWpNMU1DSWdabWxzYkQwaUl6QmpNR013WlNJdlBqeHlaV04wSUhnOUlqRTRJaUI1UFNJeE9DSWdkMmxrZEdnOUlqTXhOQ0lnYUdWcFoyaDBQU0l6TVRRaUlHWnBiR3c5SW01dmJtVWlJSE4wY205clpUMGlJMk01WVRJeU55SWdjM1J5YjJ0bExYZHBaSFJvUFNJeExqVWlMejQ4Y21WamRDQjRQU0l4TWpBaUlIazlJakUxTUNJZ2QybGtkR2c5SWpFeE1DSWdhR1ZwWjJoMFBTSTVNQ0lnWm1sc2JEMGlibTl1WlNJZ2MzUnliMnRsUFNJall6bGhNakkzSWlCemRISnZhMlV0ZDJsa2RHZzlJaklpTHo0OGNHRjBhQ0JrUFNKTk1URXdJREUxTUNCTU1UYzFJREV3TUNCTU1qUXdJREUxTUNCYUlpQm1hV3hzUFNKdWIyNWxJaUJ6ZEhKdmEyVTlJaU5qT1dFeU1qY2lJSE4wY205clpTMTNhV1IwYUQwaU1pSXZQanh5WldOMElIZzlJakUyTUNJZ2VUMGlNVGsySWlCM2FXUjBhRDBpTXpBaUlHaGxhV2RvZEQwaU5EUWlJR1pwYkd3OUltNXZibVVpSUhOMGNtOXJaVDBpSXpoaE56QXhPU0lnYzNSeWIydGxMWGRwWkhSb1BTSXhMalVpTHo0OFkybHlZMnhsSUdONFBTSXhPRElpSUdONVBTSXlNakFpSUhJOUlqTWlJR1pwYkd3OUlpTmpPV0V5TWpjaUx6NDhkR1Y0ZENCNFBTSXhOelVpSUhrOUlqSTNPQ0lnZEdWNGRDMWhibU5vYjNJOUltMXBaR1JzWlNJZ1ptbHNiRDBpSTJVNFpUWmtaaUlnWm05dWRDMW1ZVzFwYkhrOUltMXZibTl6Y0dGalpTSWdabTl1ZEMxemFYcGxQU0l4TkNJK1lTQm9iMjFsSUc5bUlIbHZkWElnYjNkdVBDOTBaWGgwUGp4MFpYaDBJSGc5SWpFM05TSWdlVDBpTXpBd0lpQjBaWGgwTFdGdVkyaHZjajBpYldsa1pHeGxJaUJtYVd4c1BTSWpZems1WVRNMElpQm1iMjUwTFdaaGJXbHNlVDBpYlc5dWIzTndZV05sSWlCbWIyNTBMWE5wZW1VOUlqRXlJajV6ZEhKbFpYUWdNeUJ3Ykc5MElEazhMM1JsZUhRK1BIUmxlSFFnZUQwaU1UYzFJaUI1UFNJek1qSWlJSFJsZUhRdFlXNWphRzl5UFNKdGFXUmtiR1VpSUdacGJHdzlJaU0yWWpZM05XTWlJR1p2Ym5RdFptRnRhV3g1UFNKdGIyNXZjM0JoWTJVaUlHWnZiblF0YzJsNlpUMGlNVEFpUG1SbFpXUWdJekVnSmlNeE9ETTdJSFJvWlNCdVlXMWxJSE4wWVhseklIZHBkR2dnZEdobElIWnZkend2ZEdWNGRENDhMM04yWno0PSIsImF0dHJpYnV0ZXMiOlt7InRyYWl0X3R5cGUiOiJzdHJlZXRfaW5kZXgiLCJ2YWx1ZSI6M30seyJ0cmFpdF90eXBlIjoicGxvdCIsInZhbHVlIjo5fSx7ImRpc3BsYXlfdHlwZSI6ImRhdGUiLCJ0cmFpdF90eXBlIjoiZm91bmRlZF9hdCIsInZhbHVlIjoxMDAwfV19";

    function test_tokenURIKnownAnswer() public {
        vm.warp(1000); // pin foundedAt so the fixture is deterministic
        uint256 id = _found(alice, 3, 9);
        assertEq(deed.tokenURI(id), EXPECTED_URI);
    }

    function test_tokenURINoSuchTokenReverts() public {
        vm.expectRevert(bytes("no such token"));
        deed.tokenURI(99);
    }

    function test_supportsErc721() public view {
        assertTrue(deed.supportsInterface(0x01ffc9a7)); // ERC-165
        assertTrue(deed.supportsInterface(0x80ac58cd)); // ERC-721
        assertTrue(deed.supportsInterface(0x5b5e139f)); // ERC-721 Metadata
    }
}
