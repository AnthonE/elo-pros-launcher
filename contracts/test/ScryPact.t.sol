// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryPact.sol";

contract ScryPactTest is Test {
    ScryPact p;
    bytes32 pid = keccak256("audit-for-pay-1");
    string terms = "A pays B 10 USDG on delivery of the signed audit; B delivers within 72h.";
    bytes32 th;

    address A = address(0xA); // payer + proposer
    address B = address(0xB); // auditor
    address C = address(0xC); // outsider

    function setUp() public {
        p = new ScryPact();
        th = sha256(bytes(terms));
    }

    function _parties() internal view returns (address[] memory a, string[] memory r, string[] memory o) {
        a = new address[](2);
        a[0] = A;
        a[1] = B;
        r = new string[](2);
        r[0] = "payer";
        r[1] = "auditor";
        o = new string[](2);
        o[0] = "pay 10 USDG on delivery";
        o[1] = "deliver a signed audit within 72h";
    }

    function _propose() internal {
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        vm.warp(1000);
        vm.prank(A);
        p.propose(pid, th, "audit-for-pay", terms, a, r, o);
    }

    function test_proposeRegistersPartiesAndAutoSignsProposer() public {
        _propose();
        (address proposer, bytes32 termsHash,, uint32 partyCount, uint32 signedCount) = p.pacts(pid);
        assertEq(proposer, A);
        assertEq(termsHash, th);
        assertEq(partyCount, 2);
        assertEq(signedCount, 1); // proposer A is a party → auto-signed
        assertTrue(p.isParty(pid, A) && p.isParty(pid, B));
        assertTrue(p.hasSigned(pid, A));
        assertFalse(p.hasSigned(pid, B));
        assertFalse(p.isActive(pid)); // B has not signed yet
    }

    function test_proposeRejectsBadTermsHash() public {
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        vm.prank(A);
        vm.expectRevert(ScryPact.BadTextHash.selector);
        p.propose(pid, keccak256("wrong"), "t", terms, a, r, o);
    }

    function test_proposeRejectsTooFewParties() public {
        address[] memory a = new address[](1);
        a[0] = A;
        string[] memory r = new string[](1);
        r[0] = "only";
        string[] memory o = new string[](1);
        o[0] = "do it";
        vm.prank(A);
        vm.expectRevert(ScryPact.BadParties.selector);
        p.propose(pid, th, "solo", terms, a, r, o);
    }

    function test_proposeRejectsDuplicateParty() public {
        address[] memory a = new address[](2);
        a[0] = A;
        a[1] = A;
        string[] memory r = new string[](2);
        r[0] = "x";
        r[1] = "y";
        string[] memory o = new string[](2);
        o[0] = "a";
        o[1] = "b";
        vm.prank(A);
        vm.expectRevert(ScryPact.DuplicateParty.selector);
        p.propose(pid, th, "dup", terms, a, r, o);
    }

    function test_secondPartySignsActivates() public {
        _propose();
        vm.prank(B);
        assertEq(p.sign(pid), 2);
        assertTrue(p.isActive(pid));
    }

    function test_outsiderCannotSignOrComment() public {
        _propose();
        vm.prank(C);
        vm.expectRevert(ScryPact.NotAParty.selector);
        p.sign(pid);
        vm.prank(C);
        vm.expectRevert(ScryPact.NotAParty.selector);
        p.comment(pid, "meddling");
    }

    function test_partyCannotSignTwice() public {
        _propose();
        vm.prank(A);
        vm.expectRevert(ScryPact.AlreadySigned.selector);
        p.sign(pid); // A auto-signed at propose
    }

    function test_commentEmitsText() public {
        _propose();
        vm.warp(1500);
        vm.expectEmit(true, true, false, true);
        emit ScryPact.PactComment(pid, B, "audit underway, on track for 48h", 1500);
        vm.prank(B);
        p.comment(pid, "audit underway, on track for 48h");
    }

    function test_eachPartyAssertsOwnStatus() public {
        _propose();
        vm.prank(B);
        p.sign(pid);
        vm.prank(B);
        p.assertStatus(pid, "fulfilled");
        vm.prank(A);
        p.assertStatus(pid, "disputed");
        // both views coexist on-chain; scry never reduces them to one verdict
        assertEq(p.statusOf(pid, B), "fulfilled");
        assertEq(p.statusOf(pid, A), "disputed");
    }

    function test_proposeIsOnce() public {
        _propose();
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        vm.prank(A);
        vm.expectRevert(ScryPact.PactExists.selector);
        p.propose(pid, th, "again", terms, a, r, o);
    }

    // ── helpers for the audited additions ───────────────────────────────────
    function _fill(uint256 n, bytes1 c) internal pure returns (string memory) {
        bytes memory b = new bytes(n);
        for (uint256 i = 0; i < n; i++) {
            b[i] = c;
        }
        return string(b);
    }

    function _partiesN(uint256 n, uint256 obLen)
        internal
        pure
        returns (address[] memory a, string[] memory r, string[] memory o)
    {
        a = new address[](n);
        r = new string[](n);
        o = new string[](n);
        for (uint256 i = 0; i < n; i++) {
            a[i] = address(uint160(0x5000 + i));
            r[i] = "role";
            o[i] = _fill(obLen, "o");
        }
    }

    // ── the broker pattern: a NON-party proposer must not auto-sign anyone ──
    // Closes the auto-sign-regression HIGH from contracts/TEST-AUDIT.md: the
    // only auto-signature propose() may mint is the proposer's own, and only
    // when the proposer is itself a named party.
    function test_brokerProposeNoAutoSign_activatesOnlyWhenAllSign() public {
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        vm.prank(C); // C is NOT among the parties
        p.propose(pid, th, "brokered", terms, a, r, o);
        (,,, uint32 partyCount, uint32 signedCount) = p.pacts(pid);
        assertEq(partyCount, 2);
        assertEq(signedCount, 0); // nobody auto-signed
        assertFalse(p.hasSigned(pid, A));
        assertFalse(p.hasSigned(pid, B));
        assertFalse(p.isParty(pid, C)); // brokering grants no standing…
        assertFalse(p.isActive(pid));

        vm.prank(C); // …and no signature rights
        vm.expectRevert(ScryPact.NotAParty.selector);
        p.sign(pid);

        vm.prank(A);
        uint32 afterA = p.sign(pid);
        assertEq(afterA, 1);
        assertFalse(p.isActive(pid)); // 1 of 2 — not yet

        vm.prank(B);
        uint32 afterB = p.sign(pid);
        assertEq(afterB, 2);
        assertTrue(p.isActive(pid)); // ACTIVE exactly when the last party signs
    }

    // ── the product lives in the log: full PactProposed payload asserted ────
    function test_brokerProposeEmitsFullPayload() public {
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        vm.warp(1000);
        vm.expectEmit(true, true, false, true);
        emit ScryPact.PactProposed(pid, C, th, "brokered", terms, a, r, o);
        vm.prank(C);
        p.propose(pid, th, "brokered", terms, a, r, o);
    }

    // The auto-sign propose emits PactSigned BEFORE PactProposed — a signature
    // event for a pactId no indexer has yet seen proposed (consumers must
    // buffer same-tx PactSigned until PactProposed lands).
    // PINNED, NOT ENDORSED — see contracts/TEST-AUDIT.md
    function test_autoSignEmitsSignedBeforeProposed() public {
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        vm.warp(1000);
        vm.expectEmit(true, true, false, true);
        emit ScryPact.PactSigned(pid, A, 1, 1000);
        vm.expectEmit(true, true, false, true);
        emit ScryPact.PactProposed(pid, A, th, "audit-for-pay", terms, a, r, o);
        vm.prank(A);
        p.propose(pid, th, "audit-for-pay", terms, a, r, o);
    }

    // ── the via_ir reason: propose + emit at every posted maximum at once ───
    // 10 parties, title = 160 bytes, terms = 4000 bytes, obligations = 1000
    // bytes each. foundry.toml enables via_ir because this function hits
    // stack-too-deep on the legacy pipeline; this exercises the shipped
    // codepath at the full advertised sizes and asserts the event lands.
    function test_proposeAtPostedMaxSizes() public {
        string memory maxTitle = _fill(160, "T");
        string memory maxTerms = _fill(4000, "m");
        bytes32 maxHash = sha256(bytes(maxTerms));
        (address[] memory a, string[] memory r, string[] memory o) = _partiesN(10, 1000);
        vm.warp(1000);
        vm.expectEmit(true, true, false, true);
        emit ScryPact.PactProposed(pid, C, maxHash, maxTitle, maxTerms, a, r, o);
        vm.prank(C);
        p.propose(pid, maxHash, maxTitle, maxTerms, a, r, o);
        (, bytes32 termsHash,, uint32 partyCount, uint32 signedCount) = p.pacts(pid);
        assertEq(termsHash, maxHash);
        assertEq(partyCount, 10);
        assertEq(signedCount, 0); // broker proposer → no auto-sign
        assertEq(p.pactCount(), 1);
    }

    // ── BadParties branches beyond the existing n<2 test ────────────────────
    function test_proposeRejectsTooManyParties() public {
        (address[] memory a, string[] memory r, string[] memory o) = _partiesN(11, 5);
        vm.expectRevert(ScryPact.BadParties.selector);
        p.propose(pid, th, "crowd", terms, a, r, o);
    }

    function test_proposeRejectsRolesLengthMismatch() public {
        (address[] memory a,, string[] memory o) = _partiesN(2, 5);
        string[] memory r = new string[](1);
        r[0] = "solo-role";
        vm.expectRevert(ScryPact.BadParties.selector);
        p.propose(pid, th, "mismatch", terms, a, r, o);
    }

    function test_proposeRejectsObligationsLengthMismatch() public {
        (address[] memory a, string[] memory r,) = _partiesN(2, 5);
        string[] memory o = new string[](3);
        o[0] = "x";
        o[1] = "y";
        o[2] = "z";
        vm.expectRevert(ScryPact.BadParties.selector);
        p.propose(pid, th, "mismatch", terms, a, r, o);
    }

    // ── posted length bounds, exact revert strings ──────────────────────────
    function test_titleBounds() public {
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        vm.expectRevert(bytes("title 1..160"));
        p.propose(pid, th, "", terms, a, r, o);
        string memory longTitle = _fill(161, "T");
        vm.expectRevert(bytes("title 1..160"));
        p.propose(pid, th, longTitle, terms, a, r, o);
    }

    function test_termsBounds() public {
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        // sha256 is a precompile CALL — computed before any pending cheatcode
        bytes32 emptyHash = sha256(bytes(""));
        vm.expectRevert(bytes("terms 1..4000"));
        p.propose(pid, emptyHash, "t", "", a, r, o);
        string memory longTerms = _fill(4001, "m");
        bytes32 longHash = sha256(bytes(longTerms));
        vm.expectRevert(bytes("terms 1..4000"));
        p.propose(pid, longHash, "t", longTerms, a, r, o);
    }

    function test_obligationBounds() public {
        (address[] memory a, string[] memory r, string[] memory o) = _parties();
        o[1] = "";
        vm.expectRevert(bytes("obligation 1..1000"));
        p.propose(pid, th, "t", terms, a, r, o);
        o[1] = _fill(1001, "o");
        vm.expectRevert(bytes("obligation 1..1000"));
        p.propose(pid, th, "t", terms, a, r, o);
    }
}
