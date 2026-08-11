// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryArbiter.sol";
import "../src/IScryArbiter.sol";
import "../src/ScryReputation.sol";
import "../src/ScryInsurancePool.sol";
import "../src/ScryJobBoard.sol";

/// Minimal ERC-20 standing in for the fair-launched SCRY. (The real token is
/// a plain OZ ERC-20 — verified on RH-Chain 2026-07-22: PonsLauncherToken,
/// launch window long expired — so always-revert-on-shortfall is faithful.)
contract MockSCRY {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 a) external {
        balanceOf[to] += a;
    }

    function transfer(address to, uint256 a) external returns (bool) {
        require(balanceOf[msg.sender] >= a, "bal");
        balanceOf[msg.sender] -= a;
        balanceOf[to] += a;
        return true;
    }

    function transferFrom(address f, address t, uint256 a) external returns (bool) {
        require(allowance[f][msg.sender] >= a, "allow");
        require(balanceOf[f] >= a, "bal");
        allowance[f][msg.sender] -= a;
        balanceOf[f] -= a;
        balanceOf[t] += a;
        return true;
    }

    function approve(address s, uint256 a) external returns (bool) {
        allowance[msg.sender][s] = a;
        return true;
    }
}

/// A stand-in for the board a panel is bound to. All the arbiter ever asks it
/// is how many cases exist, so a vote cannot be cast on one that does not.
contract MockCaseBook {
    uint256 public jobCount;

    function setJobCount(uint256 n) external {
        jobCount = n;
    }
}

/// The arbiter in isolation: the flat-fee guarantee, the pinned bench, panel
/// voting, governance transitions, and the procedural "not ruled yet" revert.
/// Green here is NECESSARY, not sufficient, for a broadcast — see
/// contracts/TEST-AUDIT.md for the standing gaps across the whole tree.
contract ScryArbiterTest is Test {
    ScryArbiter arb;
    MockCaseBook cases;
    address feeSink = address(0xA6B1);
    address a1 = address(0xA1);
    address a2 = address(0xA2);
    address a3 = address(0xA3);
    address a4 = address(0xA4);
    address stranger = address(0xBAD);
    uint256 constant FEE = 2e18;
    // the board's IMMUTABLE arbitration-fee ceiling (audit 2026-07-25):
    // an arbiter cannot charge a disputing party more than this, ever.
    uint256 constant MAX_ARB_FEE = 10e18;

    event Voted(uint256 indexed jobId, address indexed arbiter, IScryArbiter.Verdict verdict, bytes32 reasonHash);
    event Ruled(
        uint256 indexed jobId, IScryArbiter.Verdict verdict, uint256 nBuyer, uint256 nSeller, uint256 nUndecided
    );

    function setUp() public {
        address[] memory p = new address[](3);
        p[0] = a1;
        p[1] = a2;
        p[2] = a3;
        arb = new ScryArbiter(feeSink, FEE, p, 2); // quorum 2 of 3 (a strict majority)
        cases = new MockCaseBook();
        cases.setJobCount(1000); // the ids these tests rule on all really exist
        arb.setBoard(address(cases));
    }

    // ── the anti-Bar-Hadya crux: the fee and its recipient are immutable ──
    function test_feeAndRecipientAreFixed() public view {
        assertEq(arb.feeRecipient(), feeSink);
        assertEq(arb.flatFee(), FEE);
        assertEq(arb.arbiterCount(), 3);
        assertEq(arb.quorum(), 2);
    }

    // ── the panel is bound to ONE board, once (audit 2026-07-25) ──────────
    //    WAS: `vote()` took any uint256, so a panel could rule on a job that
    //    did not exist yet and `dispute()` would settle on that verdict the
    //    instant the job was posted — decided before the evidence existed.
    function test_voteRefusesACaseTheBoardHasNotIssued() public {
        cases.setJobCount(3); // jobs 0,1,2 exist
        vm.prank(a1);
        vm.expectRevert(bytes("no such case on the bound board"));
        arb.vote(3, IScryArbiter.Verdict.ForSeller, "reason");

        vm.prank(a1);
        vm.expectRevert(bytes("no such case on the bound board"));
        arb.vote(9999, IScryArbiter.Verdict.ForSeller, "reason");

        vm.prank(a1); // the last id that DOES exist is fine
        arb.vote(2, IScryArbiter.Verdict.ForSeller, "reason");
        assertEq(arb.forSeller(2), 1);
    }

    function test_unboundPanelCannotVoteAtAll() public {
        address[] memory p = new address[](3);
        p[0] = a1;
        p[1] = a2;
        p[2] = a3;
        ScryArbiter fresh = new ScryArbiter(feeSink, FEE, p, 2);
        assertEq(fresh.board(), address(0), "a fresh panel serves no board");
        vm.prank(a1);
        vm.expectRevert(bytes("board not bound"));
        fresh.vote(0, IScryArbiter.Verdict.ForSeller, "reason");
    }

    /// A verdict must not leak across boards. Job ids are PER-BOARD counters,
    /// so if a second board is ever pointed at this panel (`setArbiter` on the
    /// board side is an operator knob), board B's job #N would otherwise
    /// settle instantly on board A's verdict for #N — a ruling about a
    /// different job, on evidence from a different case. Binding `vote` to one
    /// board was only half the fence.
    function test_aSecondBoardCannotSettleOnThisPanelsVerdicts() public {
        vm.prank(a1);
        arb.vote(4, IScryArbiter.Verdict.ForSeller, "r1");
        vm.prank(a2);
        arb.vote(4, IScryArbiter.Verdict.ForSeller, "r2");
        assertTrue(arb.finalized(4), "the bound board's case is ruled");

        // a second board points itself at this panel and asks for job #4
        MockCaseBook other = new MockCaseBook();
        other.setJobCount(1000);
        vm.prank(address(other));
        vm.expectRevert(bytes("not the bound board"));
        arb.rule(4, bytes32(0), bytes32(0));

        // ...and so does a plain stranger
        vm.prank(stranger);
        vm.expectRevert(bytes("not the bound board"));
        arb.rule(4, bytes32(0), bytes32(0));

        // reading the verdict is still open to everyone — it settles nothing
        vm.prank(stranger);
        (bool isFinal, IScryArbiter.Verdict v) = arb.verdictOf(4);
        assertTrue(isFinal);
        assertEq(uint256(v), uint256(IScryArbiter.Verdict.ForSeller));
    }

    function test_boardBindingIsOneTimeAndOwnerOnly() public {
        MockCaseBook other = new MockCaseBook();
        // already bound in setUp — it can never be re-pointed at a puppet
        vm.expectRevert(bytes("board already bound"));
        arb.setBoard(address(other));

        ScryArbiter fresh = _freshPanel();
        vm.prank(stranger);
        vm.expectRevert(bytes("not owner"));
        fresh.setBoard(address(other));

        vm.expectRevert(bytes("zero board"));
        fresh.setBoard(address(0));

        vm.expectRevert(bytes("board is not a contract"));
        fresh.setBoard(address(0xDEAD)); // an EOA cannot answer jobCount()

        fresh.setBoard(address(other));
        assertEq(fresh.board(), address(other));
    }

    function _freshPanel() internal returns (ScryArbiter) {
        address[] memory p = new address[](3);
        p[0] = a1;
        p[1] = a2;
        p[2] = a3;
        return new ScryArbiter(feeSink, FEE, p, 2);
    }

    // ── constructor guards ──
    function test_constructorRejectsZeroFeeRecipient() public {
        address[] memory p = new address[](1);
        p[0] = a1;
        vm.expectRevert(bytes("zero fee recipient"));
        new ScryArbiter(address(0), FEE, p, 1);
    }

    function test_constructorRejectsDuplicateAndZeroMembers() public {
        address[] memory dup = new address[](2);
        dup[0] = a1;
        dup[1] = a1;
        vm.expectRevert(bytes("bad panel member"));
        new ScryArbiter(feeSink, FEE, dup, 2);
        address[] memory zero = new address[](1);
        zero[0] = address(0);
        vm.expectRevert(bytes("bad panel member"));
        new ScryArbiter(feeSink, FEE, zero, 1);
    }

    function test_constructorRejectsEmptyPanelAndNonMajorityQuorum() public {
        address[] memory none = new address[](0);
        vm.expectRevert(bytes("quorum not a majority"));
        new ScryArbiter(feeSink, FEE, none, 1);
        address[] memory p = new address[](3);
        p[0] = a1;
        p[1] = a2;
        p[2] = a3;
        vm.expectRevert(bytes("quorum not a majority"));
        new ScryArbiter(feeSink, FEE, p, 1); // 1 of 3 is not a majority
        vm.expectRevert(bytes("quorum not a majority"));
        new ScryArbiter(feeSink, FEE, p, 4); // > panel
    }

    function test_constructorAcceptsOneOfOne() public {
        // PINNED ON PURPOSE: the code accepts a 1-of-1 panel; the deploy
        // script's runbook warns "never a single oracle" — that guard is
        // procedural, not enforced here. See TEST-AUDIT.md.
        address[] memory p = new address[](1);
        p[0] = a1;
        ScryArbiter solo = new ScryArbiter(feeSink, FEE, p, 1);
        assertEq(solo.arbiterCount(), 1);
    }

    // ── majority verdicts finalize at quorum ──
    function test_majorityForSellerFinalizesWithEvents() public {
        vm.expectEmit(true, true, true, true);
        emit Voted(1, a1, IScryArbiter.Verdict.ForSeller, bytes32("r1"));
        vm.prank(a1);
        arb.vote(1, IScryArbiter.Verdict.ForSeller, bytes32("r1"));
        assertFalse(arb.finalized(1)); // one vote, not yet a majority
        (bool fin0,) = arb.verdictOf(1);
        assertFalse(fin0);
        assertEq(arb.casePanel(1), 3); // the bench pinned at first vote
        assertEq(arb.caseQuorum(1), 2);

        vm.expectEmit(true, true, true, true);
        emit Ruled(1, IScryArbiter.Verdict.ForSeller, 0, 2, 0); // tallies ride the event
        vm.prank(a2);
        arb.vote(1, IScryArbiter.Verdict.ForSeller, bytes32("r2"));
        assertTrue(arb.finalized(1));
        assertEq(arb.forSeller(1), 2);
        (bool fin, IScryArbiter.Verdict v) = arb.verdictOf(1);
        assertTrue(fin);
        assertEq(uint256(v), uint256(IScryArbiter.Verdict.ForSeller));
        vm.prank(address(cases));
        assertEq(uint256(arb.rule(1, bytes32(0), bytes32(0))), uint256(IScryArbiter.Verdict.ForSeller));
    }

    function test_majorityForBuyerFinalizes() public {
        vm.prank(a1);
        arb.vote(7, IScryArbiter.Verdict.ForBuyer, "r1");
        vm.prank(a2);
        arb.vote(7, IScryArbiter.Verdict.ForBuyer, "r2");
        vm.prank(address(cases));
        assertEq(uint256(arb.rule(7, bytes32(0), bytes32(0))), uint256(IScryArbiter.Verdict.ForBuyer));
    }

    // ── a full split with no majority → Undecided (the board refunds) ──
    function test_splitPanelRulesUndecided() public {
        vm.prank(a1);
        arb.vote(3, IScryArbiter.Verdict.ForSeller, "r1");
        vm.prank(a2);
        arb.vote(3, IScryArbiter.Verdict.ForBuyer, "r2");
        assertFalse(arb.finalized(3)); // 1-1, no verdict reached quorum
        vm.prank(a3); // whole bench voted, no majority
        arb.vote(3, IScryArbiter.Verdict.Undecided, "r3");
        assertTrue(arb.finalized(3));
        vm.prank(address(cases));
        assertEq(uint256(arb.rule(3, bytes32(0), bytes32(0))), uint256(IScryArbiter.Verdict.Undecided));
    }

    // ── rule() reverts before the panel has ruled (procedural, not a verdict) ──
    function test_ruleRevertsBeforeQuorum() public {
        vm.prank(a1);
        arb.vote(9, IScryArbiter.Verdict.ForSeller, "r1");
        vm.expectRevert(bytes("panel has not ruled"));
        vm.prank(address(cases));
        arb.rule(9, bytes32(0), bytes32(0));
    }

    // ── access + double-vote guards ──
    function test_nonArbiterCannotVote() public {
        vm.prank(stranger);
        vm.expectRevert(bytes("not an arbiter"));
        arb.vote(1, IScryArbiter.Verdict.ForSeller, "x");
    }

    function test_doubleVoteReverts() public {
        vm.prank(a1);
        arb.vote(1, IScryArbiter.Verdict.ForSeller, "r1");
        vm.prank(a1);
        vm.expectRevert(bytes("already voted"));
        arb.vote(1, IScryArbiter.Verdict.ForBuyer, "r1b");
    }

    function test_voteAfterFinalizedReverts() public {
        vm.prank(a1);
        arb.vote(1, IScryArbiter.Verdict.ForSeller, "r1");
        vm.prank(a2); // finalized
        arb.vote(1, IScryArbiter.Verdict.ForSeller, "r2");
        vm.prank(a3);
        vm.expectRevert(bytes("already ruled"));
        arb.vote(1, IScryArbiter.Verdict.ForBuyer, "r3");
    }

    function test_removedArbiterCannotVote() public {
        arb.setArbiter(a3, false); // count 3→2; quorum 2 of 2 still a majority
        vm.prank(a3);
        vm.expectRevert(bytes("not an arbiter"));
        arb.vote(1, IScryArbiter.Verdict.ForSeller, "x");
    }

    // ── governance is owner-only and preserves the strict-majority invariant ──
    function test_setArbiterOnlyOwner() public {
        vm.prank(stranger);
        vm.expectRevert(bytes("not owner"));
        arb.setArbiter(a4, true);
        vm.prank(stranger);
        vm.expectRevert(bytes("not owner"));
        arb.setQuorum(2);
    }

    function test_addingArbiterNeedsQuorumRaisedFirst() public {
        // owner == this test contract (the deployer). Adding a 4th member makes
        // quorum 2 no longer a majority of 4 (2*2 == 4), so it must revert until
        // the quorum is raised.
        vm.expectRevert(bytes("fix quorum first"));
        arb.setArbiter(a4, true);
        arb.setQuorum(3); // majority of 3
        arb.setArbiter(a4, true); // now majority of 4
        assertEq(arb.arbiterCount(), 4);
        assertEq(arb.quorum(), 3);
    }

    function test_removeKeepsMajorityOrReverts() public {
        arb.setArbiter(a3, false); // 3→2 with quorum 2: 2*2>2 — fine
        assertEq(arb.arbiterCount(), 2);
        // 2→1 with quorum 2 breaks quorum<=count → must revert. And note the
        // deliberate deadlock: setQuorum(1) at count 2 is not a majority
        // (2*1 == 2), so a 2-member panel can never legally shrink to 1 —
        // dissolving a panel means replacing it, not decaying to an oracle.
        vm.expectRevert(bytes("fix quorum first"));
        arb.setArbiter(a2, false);
        vm.expectRevert(bytes("quorum not a majority"));
        arb.setQuorum(1);
    }

    function test_setArbiterNoChangeReverts() public {
        vm.expectRevert(bytes("no change"));
        arb.setArbiter(a1, true); // already a member
    }

    function test_transferOwnership() public {
        vm.expectRevert(bytes("zero"));
        arb.transferOwnership(address(0));
        arb.transferOwnership(stranger);
        assertEq(arb.owner(), stranger);
        vm.expectRevert(bytes("not owner")); // old owner locked out
        arb.setQuorum(2);
        vm.prank(stranger);
        arb.setQuorum(2); // new owner governs
    }

    // ── the pinned bench: churn mid-case can neither strand nor re-bar a case ──
    function test_churnMidCaseStillFinalizes() public {
        // A and B split; C (who never voted) is removed; D is seated; D's vote
        // fills the pinned bench of 3 → exhaustion → Undecided. Without the
        // per-case pin this case would hang forever (sum could never equal the
        // shrunken live count) and the deadline path would decide the job.
        vm.prank(a1);
        arb.vote(5, IScryArbiter.Verdict.ForSeller, "r1");
        assertEq(arb.casePanel(5), 3);
        arb.setArbiter(a3, false); // count 2 (quorum 2 still a majority)
        vm.prank(a2);
        arb.vote(5, IScryArbiter.Verdict.ForBuyer, "r2");
        assertFalse(arb.finalized(5)); // 1-1 on a bench of 3
        arb.setArbiter(a4, true); // count 3 again (quorum 2 fine)
        vm.prank(a4);
        arb.vote(5, IScryArbiter.Verdict.Undecided, "r4");
        assertTrue(arb.finalized(5)); // bench of 3 exhausted, no majority
        vm.prank(address(cases));
        assertEq(uint256(arb.rule(5, bytes32(0), bytes32(0))), uint256(IScryArbiter.Verdict.Undecided));
    }

    function test_caseQuorumPinnedAgainstLaterRaise() public {
        // 5-member panel, quorum 3. Two FS votes land; the quorum is then
        // raised to 4 globally; the case still finalizes at its PINNED 3.
        address[] memory p = new address[](5);
        p[0] = a1;
        p[1] = a2;
        p[2] = a3;
        p[3] = a4;
        p[4] = address(0xA5);
        ScryArbiter big = new ScryArbiter(feeSink, FEE, p, 3);
        big.setBoard(address(cases)); // every panel serves a board before it rules
        vm.prank(a1);
        big.vote(11, IScryArbiter.Verdict.ForSeller, "r1");
        vm.prank(a2);
        big.vote(11, IScryArbiter.Verdict.ForSeller, "r2");
        big.setQuorum(4); // 4 of 5 is a majority; global bar moves
        vm.prank(a3);
        big.vote(11, IScryArbiter.Verdict.ForSeller, "r3");
        assertTrue(big.finalized(11)); // pinned caseQuorum 3 ruled, not the new 4
        vm.prank(address(cases));
        assertEq(uint256(big.rule(11, bytes32(0), bytes32(0))), uint256(IScryArbiter.Verdict.ForSeller));
    }
}

/// The integration the deploy path needs: a REAL ScryArbiter wired into a REAL
/// ScryJobBoard settles disputes, with the flat fee independent of the verdict
/// across ALL THREE verdicts. This is what DeployScryMarket.s.sol expects
/// SCRY_ARBITER to be. Green here is necessary for a broadcast, not
/// sufficient — the standing board-side gaps (Insured-mode disputes, operator
/// knobs) are catalogued in contracts/TEST-AUDIT.md.
contract ScryArbiterBoardTest is Test {
    MockSCRY scry;
    ScryReputation reputation;
    ScryInsurancePool pool;
    ScryJobBoard board;
    ScryArbiter arb;

    address buyer = address(0xB0B);
    address seller = address(0x5E11E5);
    address splitter = address(0xFEE5);
    address arbFeeSink = address(0xA6B1);
    address a1 = address(0xA1);
    address a2 = address(0xA2);
    address a3 = address(0xA3);

    bytes32 constant SPEC = keccak256("output matches schema X");
    bytes32 constant DELIV = keccak256("here is the output");
    uint256 constant FEE = 2e18;
    // the board's IMMUTABLE arbitration-fee ceiling (audit 2026-07-25):
    // an arbiter cannot charge a disputing party more than this, ever.
    uint256 constant MAX_ARB_FEE = 10e18;

    function setUp() public {
        scry = new MockSCRY();
        reputation = new ScryReputation(50);
        pool = new ScryInsurancePool(IERC20(address(scry)), 1_000e18);
        address[] memory p = new address[](3);
        p[0] = a1;
        p[1] = a2;
        p[2] = a3;
        arb = new ScryArbiter(arbFeeSink, FEE, p, 2);
        board = new ScryJobBoard(
            IERC20(address(scry)),
            IReputation(address(reputation)),
            IInsurancePool(address(pool)),
            splitter,
            IScryArbiter(address(arb)),
            MAX_ARB_FEE
        );
        reputation.setAuthority(address(board), true);
        pool.setClaimant(address(board), true);
        arb.setBoard(address(board)); // ONE-TIME binding; the panel cannot vote before it
        scry.mint(buyer, 1_000e18);
        scry.mint(seller, 1_000e18);
    }

    function _postEscrow(uint256 amount) internal returns (uint256 id) {
        vm.startPrank(buyer);
        scry.approve(address(board), amount);
        id = board.post(seller, amount, ScryJobBoard.Mode.Escrow, SPEC, uint64(block.timestamp + 1 days), 0);
        vm.stopPrank();
    }

    function _panelRules(uint256 id, IScryArbiter.Verdict v) internal {
        vm.prank(a1);
        arb.vote(id, v, "r1");
        vm.prank(a2); // 2 of 3 → the ruling is finalized
        arb.vote(id, v, "r2");
    }

    function _panelSplits(uint256 id) internal {
        // 1-1-1 across the bench → exhaustion → Undecided
        vm.prank(a1);
        arb.vote(id, IScryArbiter.Verdict.ForSeller, "r1");
        vm.prank(a2);
        arb.vote(id, IScryArbiter.Verdict.ForBuyer, "r2");
        vm.prank(a3);
        arb.vote(id, IScryArbiter.Verdict.Undecided, "r3");
    }

    // ── a real panel ruling ForSeller pays the seller ──
    function test_disputeRuledForSellerPays() public {
        uint256 id = _postEscrow(100e18);
        _panelRules(id, IScryArbiter.Verdict.ForSeller);
        vm.startPrank(buyer);
        scry.approve(address(board), FEE);
        board.dispute(id);
        vm.stopPrank();
        assertEq(scry.balanceOf(seller), 1_000e18 + 95e18); // 100 - 5% fee
        assertEq(scry.balanceOf(arbFeeSink), FEE); // the flat fee landed
    }

    // ── ruling ForBuyer refunds the escrow and slashes the seller ──
    function test_disputeRuledForBuyerRefundsAndSlashes() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        uint256 id = _postEscrow(100e18);
        _panelRules(id, IScryArbiter.Verdict.ForBuyer);
        vm.startPrank(buyer);
        scry.approve(address(board), FEE);
        board.dispute(id);
        vm.stopPrank();
        assertEq(scry.balanceOf(buyer), 1_000e18 - FEE); // escrow refunded, minus the arb fee it paid
        assertEq(reputation.rep(seller), 75); // 100 - 25 penalty
    }

    // ── Undecided through the board: refund, NEVER a slash (operator decision
    //    2026-07-22). The panel declined to decide — rep moves only on an
    //    EARNED outcome (CREED #9), and slashing here would pay spurious
    //    disputes in seller damage. The job closes under its own honest
    //    reason, "undecided", not "ruled-for-buyer". ──
    function test_disputeUndecidedRefundsWithoutSlash() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        uint256 id = _postEscrow(100e18);
        _panelSplits(id);
        vm.startPrank(buyer);
        scry.approve(address(board), FEE);
        vm.expectEmit(true, false, false, true, address(board));
        emit ScryJobBoard.Closed(id, "undecided", 0, 0);
        board.dispute(id);
        vm.stopPrank();
        assertEq(scry.balanceOf(buyer), 1_000e18 - FEE); // escrow refunded
        assertEq(scry.balanceOf(seller), 1_000e18); // seller paid nothing, got nothing
        assertEq(reputation.rep(seller), 100); // and is NOT slashed on a no-basis case
    }

    // ── the anti-Bar-Hadya property end-to-end: SAME fee, ALL THREE verdicts ──
    function test_arbiterFeeIndependentOfVerdictOnBoard() public {
        uint256 id1 = _postEscrow(100e18);
        _panelRules(id1, IScryArbiter.Verdict.ForSeller);
        vm.startPrank(buyer);
        scry.approve(address(board), FEE);
        board.dispute(id1);
        vm.stopPrank();
        uint256 after1 = scry.balanceOf(arbFeeSink);

        uint256 id2 = _postEscrow(100e18);
        _panelRules(id2, IScryArbiter.Verdict.ForBuyer);
        vm.startPrank(buyer);
        scry.approve(address(board), FEE);
        board.dispute(id2);
        vm.stopPrank();
        uint256 after2 = scry.balanceOf(arbFeeSink);

        uint256 id3 = _postEscrow(100e18);
        _panelSplits(id3); // → Undecided
        vm.startPrank(buyer);
        scry.approve(address(board), FEE);
        board.dispute(id3);
        vm.stopPrank();
        uint256 after3 = scry.balanceOf(arbFeeSink);

        assertEq(after1, FEE);
        assertEq(after2 - after1, FEE); // identical fee, opposite verdicts
        assertEq(after3 - after2, FEE); // and identical again when nobody won
    }

    // ── the SELLER may open the case too, and pays the same flat fee ──
    function test_sellerInitiatedDispute() public {
        uint256 id = _postEscrow(100e18);
        vm.prank(seller);
        board.deliver(id, DELIV);
        _panelRules(id, IScryArbiter.Verdict.ForSeller);
        vm.startPrank(seller);
        scry.approve(address(board), FEE);
        board.dispute(id); // a Delivered job, disputed by the seller
        vm.stopPrank();
        assertEq(scry.balanceOf(seller), 1_000e18 + 95e18 - FEE); // paid, minus the fee they fronted
        assertEq(scry.balanceOf(arbFeeSink), FEE);
    }

    // ── ReputationOnly dispute, ForSeller, buyer never funds: no capital
    //    moves by DESIGN (RepOnly's honest line), and since 2026-07-22 the
    //    event says so — "completed-unfunded" with zeros, never fictional
    //    amounts (CREED #4). Rep is still earned: the work was done. ──
    function test_repOnlyDisputeForSellerUnfundedPaysNothing() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed"); // meets threshold 50
        vm.prank(buyer);
        uint256 id =
            board.post(seller, 100e18, ScryJobBoard.Mode.ReputationOnly, SPEC, uint64(block.timestamp + 1 days), 0);
        vm.prank(seller);
        board.deliver(id, DELIV);
        _panelRules(id, IScryArbiter.Verdict.ForSeller);
        vm.startPrank(seller);
        scry.approve(address(board), FEE);
        vm.expectEmit(true, false, false, true, address(board));
        emit ScryJobBoard.Closed(id, "completed-unfunded", 0, 0);
        board.dispute(id);
        vm.stopPrank();
        assertEq(scry.balanceOf(seller), 1_000e18 - FEE); // won the case; zero SCRY arrived
        assertEq(reputation.rep(seller), 110); // 100 + 10 completion rep
    }

    // ── a premature dispute (panel silent) reverts WHOLE: no settlement, fee rolled back ──
    function test_disputeBeforePanelRulesRevertsAndKeepsFee() public {
        uint256 id = _postEscrow(100e18);
        vm.startPrank(buyer);
        scry.approve(address(board), FEE);
        vm.expectRevert(bytes("panel has not ruled"));
        board.dispute(id);
        vm.stopPrank();
        assertEq(scry.balanceOf(arbFeeSink), 0); // the fee never left
        assertEq(scry.balanceOf(address(board)), 100e18); // escrow still held, the job still open
    }

    // ── only a party may open a case ──
    function test_disputeByStrangerReverts() public {
        uint256 id = _postEscrow(100e18);
        _panelRules(id, IScryArbiter.Verdict.ForSeller);
        vm.prank(address(0xDEAD));
        vm.expectRevert(bytes("not a party"));
        board.dispute(id);
    }
}
