// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryReputation.sol";
import "../src/ScryInsurancePool.sol";
import "../src/ScryJobBoard.sol";
import "../src/IScryArbiter.sol";

/// Minimal ERC-20 standing in for the fair-launched SCRY.
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

/// A mock arbiter whose verdict is set by the test — and, crucially, whose
/// FEE does not depend on the verdict (the property we assert).
contract MockArbiter is IScryArbiter {
    address public feeRecipient;
    uint256 public flatFee;
    Verdict public next = Verdict.ForSeller;
    uint256 public feesCollectedForBuyer;
    uint256 public feesCollectedForSeller;

    constructor(address _fee, uint256 _flat) {
        feeRecipient = _fee;
        flatFee = _flat;
    }

    function setVerdict(Verdict v) external {
        next = v;
    }

    function rule(uint256, bytes32, bytes32) external view returns (Verdict) {
        return next;
    }
}

/// The OTHER ERC-20 dialect: transfer/transferFrom RETURN FALSE on a
/// shortfall instead of reverting. The real SCRY is a plain OZ ERC-20 that
/// reverts (TEST-AUDIT.md grounding), so this is the robustness leg for
/// self-hosters wiring a different token — the board's `require(...)` wraps
/// must catch a false return, and `_tryPull` must decode it as "unfunded".
contract MockSCRYFalse {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 a) external {
        balanceOf[to] += a;
    }

    function transfer(address to, uint256 a) external returns (bool) {
        if (balanceOf[msg.sender] < a) return false;
        balanceOf[msg.sender] -= a;
        balanceOf[to] += a;
        return true;
    }

    function transferFrom(address f, address t, uint256 a) external returns (bool) {
        if (allowance[f][msg.sender] < a || balanceOf[f] < a) return false;
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

contract ScryMarketTest is Test {
    // the board's IMMUTABLE arbitration-fee ceiling (audit 2026-07-25):
    // an arbiter cannot charge a disputing party more than this, ever.
    uint256 constant MAX_ARB_FEE = 10e18;
    MockSCRY scry;
    ScryReputation reputation;
    ScryInsurancePool pool;
    ScryJobBoard board;
    MockArbiter arbiter;

    address buyer = address(0xB0B);
    address seller = address(0x5E11E5);
    address splitter = address(0xFEE5);
    address arbFeeSink = address(0xA6B1);

    bytes32 constant SPEC = keccak256("output matches schema X");
    bytes32 constant DELIV = keccak256("here is the output");

    function setUp() public {
        scry = new MockSCRY();
        reputation = new ScryReputation(50); // threshold
        pool = new ScryInsurancePool(IERC20(address(scry)), 1_000e18);
        arbiter = new MockArbiter(arbFeeSink, 2e18);
        board = new ScryJobBoard(
            IERC20(address(scry)),
            IReputation(address(reputation)),
            IInsurancePool(address(pool)),
            splitter,
            IScryArbiter(address(arbiter)),
            MAX_ARB_FEE
        );
        reputation.setAuthority(address(board), true);
        pool.setClaimant(address(board), true);
        // Insured mode ships CLOSED (operator, 2026-07-25) — the suite opens it
        // deliberately so the mode's own mechanics stay covered. That the door
        // is shut by default is pinned separately, in
        // testInsuredModeIsClosedOnAFreshBoard below.
        board.setInsuredOpen(true);
        scry.mint(buyer, 1_000e18);
        scry.mint(seller, 1_000e18);
    }

    function _postEscrow(uint256 amount) internal returns (uint256 id) {
        vm.startPrank(buyer);
        scry.approve(address(board), amount);
        id = board.post(seller, amount, ScryJobBoard.Mode.Escrow, SPEC, uint64(block.timestamp + 1 days), 0);
        vm.stopPrank();
    }

    // ── the happy path: escrow → deliver → complete, cut + rep ───────────
    function testEscrowCompletePaysSellerMinusFeeAndEarnsRep() public {
        uint256 id = _postEscrow(100e18);
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.prank(buyer);
        board.complete(id);
        assertEq(scry.balanceOf(seller), 1_000e18 + 95e18); // 100 - 5% fee
        assertEq(scry.balanceOf(splitter), 5e18);
        assertEq(reputation.rep(seller), 10);
    }

    // ── timeout with no delivery → refund buyer, slash seller ────────────
    function testSellerDefaultRefundsBuyerAndSlashes() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        uint256 id = _postEscrow(100e18);
        vm.warp(block.timestamp + 2 days);
        board.close(id);
        assertEq(scry.balanceOf(buyer), 1_000e18); // fully refunded
        assertEq(reputation.rep(seller), 75); // 100 - 25 penalty
    }

    // ── delivered but buyer goes silent → seller paid from escrow ────────
    function testBuyerSilentOnDeliveredReleasesToSeller() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(buyer, 100, "seed");
        uint256 id = _postEscrow(100e18);
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.warp(block.timestamp + 2 days);
        board.close(id);
        assertEq(scry.balanceOf(seller), 1_000e18 + 95e18);
        assertEq(reputation.rep(buyer), 75); // buyer default slashed
    }

    // ── the anti-Bar-Hadya property: the arbiter's fee is the SAME whichever
    //    way it rules, and it's paid before ruling ──────────────────────────
    function testArbiterFeeIsIndependentOfVerdict() public {
        // case 1: rules for seller
        uint256 id1 = _postEscrow(100e18);
        arbiter.setVerdict(IScryArbiter.Verdict.ForSeller);
        vm.startPrank(buyer);
        scry.approve(address(board), 2e18);
        board.dispute(id1);
        vm.stopPrank();
        uint256 feeAfterCase1 = scry.balanceOf(arbFeeSink);

        // case 2: rules for buyer — same flat fee lands
        uint256 id2 = _postEscrow(100e18);
        arbiter.setVerdict(IScryArbiter.Verdict.ForBuyer);
        vm.startPrank(buyer);
        scry.approve(address(board), 2e18);
        board.dispute(id2);
        vm.stopPrank();
        uint256 feeAfterCase2 = scry.balanceOf(arbFeeSink);

        assertEq(feeAfterCase1, 2e18);
        assertEq(feeAfterCase2 - feeAfterCase1, 2e18); // identical fee, opposite verdicts
    }

    // ── ruling for seller pays them; for buyer refunds + slashes ─────────
    function testRulingForSellerPays() public {
        uint256 id = _postEscrow(100e18);
        arbiter.setVerdict(IScryArbiter.Verdict.ForSeller);
        vm.startPrank(buyer);
        scry.approve(address(board), 2e18);
        board.dispute(id);
        vm.stopPrank();
        assertEq(scry.balanceOf(seller), 1_000e18 + 95e18);
    }

    function testRulingForBuyerRefundsAndSlashes() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        uint256 id = _postEscrow(100e18);
        arbiter.setVerdict(IScryArbiter.Verdict.ForBuyer);
        vm.startPrank(buyer);
        scry.approve(address(board), 2e18);
        board.dispute(id);
        vm.stopPrank();
        assertEq(scry.balanceOf(buyer), 1_000e18 - 2e18); // refunded amount, minus the arb fee it paid
        assertEq(reputation.rep(seller), 75);
    }

    // ── reputation is soulbound: no transfer path exists, gate works ─────
    function testRepGateBlocksLowRepSellerFromRepOnlyJob() public {
        vm.startPrank(buyer);
        vm.expectRevert(bytes("seller below rep threshold"));
        board.post(seller, 10e18, ScryJobBoard.Mode.ReputationOnly, SPEC, uint64(block.timestamp + 1 days), 0);
        vm.stopPrank();
    }

    // ── measurement can't be auctioned is enforced off-chain (market.py);
    //    on-chain, money never buys rep — only authorized market logic mutates
    function testOnlyAuthorizedCanMutateRep() public {
        vm.expectRevert(bytes("not authorized"));
        reputation.earn(seller, 1000, "cheat");
    }

    // ── insured mode ships CLOSED (operator, 2026-07-25) ──────────────────
    //    The rep gate + priced premium + per-beneficiary window raise the cost
    //    of self-dealing and cap its rate; none of them makes it unprofitable
    //    in the limit, because nothing on-chain tells a real hire from one
    //    person wearing two addresses (AUDIT-2026-07-25.md §4b). So a freshly
    //    deployed board refuses the mode outright, and Escrow + ReputationOnly
    //    are untouched by that. This test is the tripwire on the default: it
    //    goes red the day someone ships `insuredOpen = true`.
    function testInsuredModeIsClosedOnAFreshBoard() public {
        ScryJobBoard fresh = new ScryJobBoard(
            IERC20(address(scry)),
            IReputation(address(reputation)),
            IInsurancePool(address(pool)),
            splitter,
            IScryArbiter(address(arbiter)),
            MAX_ARB_FEE
        );
        assertEq(fresh.insuredOpen(), false, "insured mode must ship closed");
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed"); // over the bar, and it still must not post
        vm.startPrank(buyer);
        scry.approve(address(fresh), type(uint256).max);
        vm.expectRevert(bytes("insured mode is closed"));
        fresh.post(seller, 100e18, ScryJobBoard.Mode.Insured, SPEC, uint64(block.timestamp + 1 days), 3e18);
        // the other two modes are NOT gated by this — the door is on Insured only
        fresh.post(seller, 100e18, ScryJobBoard.Mode.Escrow, SPEC, uint64(block.timestamp + 1 days), 0);
        fresh.post(seller, 100e18, ScryJobBoard.Mode.ReputationOnly, SPEC, uint64(block.timestamp + 1 days), 0);
        vm.stopPrank();
    }

    function testInsuredOpenIsOperatorOnlyAndReclosable() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        board.setInsuredOpen(false);

        board.setInsuredOpen(false); // the operator may shut it again
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        vm.startPrank(buyer);
        scry.approve(address(board), type(uint256).max);
        vm.expectRevert(bytes("insured mode is closed"));
        board.post(seller, 100e18, ScryJobBoard.Mode.Insured, SPEC, uint64(block.timestamp + 1 days), 3e18);
        vm.stopPrank();
    }

    // ── insured mode: buyer confirms completion WITHOUT funding → must revert,
    //    never close as "completed" while paying the seller nothing ─────────
    function testInsuredCompleteWithoutFundingReverts() public {
        // Insured mode now demands the same reputation bar as an unsecured
        // hire — coverage is the pool's money, and a burner seller was the
        // whole shape of the drain (audit 2026-07-25, test/PoolDrain.t.sol).
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        vm.startPrank(buyer);
        scry.approve(address(board), 3e18); // premium only, amount NOT approved
        uint256 id = board.post(seller, 100e18, ScryJobBoard.Mode.Insured, SPEC, uint64(block.timestamp + 1 days), 3e18);
        vm.stopPrank();
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.prank(buyer);
        vm.expectRevert(bytes("buyer has not funded - approve SCRY or await deadline"));
        board.complete(id);
    }

    // ── insured mode: buyer funds then completes → seller paid, no pool touch
    function testInsuredCompleteWithFundingPaysSeller() public {
        // Insured mode now demands the same reputation bar as an unsecured
        // hire — coverage is the pool's money, and a burner seller was the
        // whole shape of the drain (audit 2026-07-25, test/PoolDrain.t.sol).
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        vm.startPrank(buyer);
        scry.approve(address(board), 103e18); // premium + amount
        uint256 id = board.post(seller, 100e18, ScryJobBoard.Mode.Insured, SPEC, uint64(block.timestamp + 1 days), 3e18);
        vm.stopPrank();
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.prank(buyer);
        board.complete(id);
        assertEq(scry.balanceOf(seller), 1_000e18 + 95e18);
        assertEq(scry.balanceOf(splitter), 5e18);
        // 100 seed + 10 reward: the BUYER funded, so this is an honest
        // completion and it earns rep like any other.
        assertEq(reputation.rep(seller), 110);
    }

    // ── insured mode: buyer default after delivery → seller covered by pool
    function testInsuredBuyerDefaultCoversSellerFromPool() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        scry.mint(address(this), 500e18);
        scry.approve(address(pool), 500e18);
        pool.payPremium(500e18); // fund the pool

        vm.startPrank(buyer);
        scry.approve(address(board), 3e18); // premium only; amount is NOT escrowed
        uint256 id = board.post(seller, 100e18, ScryJobBoard.Mode.Insured, SPEC, uint64(block.timestamp + 1 days), 3e18);
        vm.stopPrank();
        vm.prank(seller);
        board.deliver(id, DELIV);
        // buyer never approves the amount; goes silent past the deadline
        vm.warp(block.timestamp + 2 days);
        board.close(id);
        // seller made whole from the pool (net of fee), buyer slashed
        assertEq(scry.balanceOf(seller), 1_000e18 + 95e18);
    }

    // ── reputation-only mode: the positive path (TEST-AUDIT: zero positive
    //    tests existed) — nothing locked at post; buyer funds at completion ──
    function testRepOnlyCompleteWithFundingPaysSeller() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed"); // over the 50 threshold
        vm.prank(buyer);
        uint256 id =
            board.post(seller, 100e18, ScryJobBoard.Mode.ReputationOnly, SPEC, uint64(block.timestamp + 1 days), 0); // no escrow, no premium
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.startPrank(buyer);
        scry.approve(address(board), 100e18); // fund at completion time
        board.complete(id);
        vm.stopPrank();
        assertEq(scry.balanceOf(seller), 1_000e18 + 95e18); // net of the 5% cut
        assertEq(scry.balanceOf(splitter), 5e18);
        assertEq(scry.balanceOf(buyer), 1_000e18 - 100e18);
        assertEq(reputation.rep(seller), 110); // 100 seed + 10 reward
    }

    // ── reputation-only default at deadline: never delivered → nothing was
    //    ever locked, so nothing moves; the seller pays in soulbound rep ────
    function testRepOnlySellerDefaultNothingLockedSlashesSeller() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        vm.prank(buyer);
        uint256 id =
            board.post(seller, 100e18, ScryJobBoard.Mode.ReputationOnly, SPEC, uint64(block.timestamp + 1 days), 0);
        vm.warp(block.timestamp + 2 days);
        board.close(id);
        assertEq(scry.balanceOf(buyer), 1_000e18); // no refund needed: no lock
        assertEq(scry.balanceOf(seller), 1_000e18);
        assertEq(reputation.rep(seller), 75); // 100 - 25 penalty
    }

    // ── reputation-only, delivered, buyer never funds, deadline passes:
    //    close() emits Closed(id, "completed", 95e18, 5e18) — amounts that
    //    describe money that NEVER MOVED — and the seller still earns rep.
    //    PINNED, NOT ENDORSED — see contracts/TEST-AUDIT.md (operator
    //    question #2: RepOnly "completed" with zero transfer). ─────────────
    function testRepOnlyBuyerSilentClosesUnfundedWithHonestZeros() public {
        // Operator decision 2026-07-22: the settlement (no capital recovery,
        // rep both ways) is RepOnly's design; the EVENT now tells the truth —
        // "completed-unfunded" with the amounts that actually moved (CREED #4).
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        reputation.earn(buyer, 100, "seed");
        vm.prank(buyer);
        uint256 id =
            board.post(seller, 100e18, ScryJobBoard.Mode.ReputationOnly, SPEC, uint64(block.timestamp + 1 days), 0);
        vm.prank(seller);
        board.deliver(id, DELIV);
        // buyer neither approves nor completes; anyone closes at the deadline
        vm.warp(block.timestamp + 2 days);
        vm.expectEmit(true, false, false, true, address(board));
        emit ScryJobBoard.Closed(id, "completed-unfunded", 0, 0); // honest amounts
        board.close(id);
        // NO transfer occurred anywhere:
        assertEq(scry.balanceOf(seller), 1_000e18);
        assertEq(scry.balanceOf(buyer), 1_000e18);
        assertEq(scry.balanceOf(splitter), 0);
        // the seller earned rep for work done; the defaulting buyer is slashed:
        assertEq(reputation.rep(seller), 110); // 100 + 10 "completed"
        assertEq(reputation.rep(buyer), 75); // 100 - 25 "buyer-default"
    }

    // ── insured cover reports what the POOL actually paid (cap / thin pool),
    //    never the nominal net (CREED #4) — operator decision 2026-07-22 ──
    function testInsuredThinPoolCoverEmitsActualPaidAmount() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        scry.mint(address(this), 10e18);
        scry.approve(address(pool), 10e18);
        pool.payPremium(10e18); // the pool holds only 10 of the 95 owed
        vm.startPrank(buyer);
        scry.approve(address(board), 3e18); // premium only; amount never funded
        uint256 id = board.post(seller, 100e18, ScryJobBoard.Mode.Insured, SPEC, uint64(block.timestamp + 1 days), 3e18);
        vm.stopPrank();
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.warp(block.timestamp + 2 days);
        vm.expectEmit(true, false, false, true, address(board));
        emit ScryJobBoard.Closed(id, "completed-insured", 13e18, 0); // 10 + the 3 premium in the pool
        board.close(id);
        assertEq(scry.balanceOf(seller), 1_000e18 + 13e18); // paid exactly what the pool held
    }

    // ── deliver guards ───────────────────────────────────────────────────
    function testDeliverByNonSellerReverts() public {
        uint256 id = _postEscrow(100e18);
        vm.prank(buyer);
        vm.expectRevert(bytes("not seller"));
        board.deliver(id, DELIV);
    }

    function testDeliverOnClosedJobReverts() public {
        uint256 id = _postEscrow(100e18);
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.prank(buyer);
        board.complete(id); // job now Closed
        vm.prank(seller);
        vm.expectRevert(bytes("not open"));
        board.deliver(id, DELIV);
    }

    function testDeliverTwiceReverts() public {
        uint256 id = _postEscrow(100e18);
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.prank(seller);
        vm.expectRevert(bytes("not open"));
        board.deliver(id, DELIV); // Delivered is not Posted
    }

    // ── double-settle: every second settlement path must dead-end ────────
    function testCloseAfterCompleteReverts() public {
        uint256 id = _postEscrow(100e18);
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.prank(buyer);
        board.complete(id);
        vm.warp(block.timestamp + 2 days);
        vm.expectRevert(bytes("closed"));
        board.close(id);
    }

    function testCloseTwiceReverts() public {
        uint256 id = _postEscrow(100e18);
        vm.warp(block.timestamp + 2 days);
        board.close(id);
        vm.expectRevert(bytes("closed"));
        board.close(id);
    }

    function testDisputeAfterCloseReverts() public {
        uint256 id = _postEscrow(100e18);
        vm.warp(block.timestamp + 2 days);
        board.close(id);
        vm.prank(buyer);
        vm.expectRevert(bytes("closed"));
        board.dispute(id);
    }

    // ── deadline boundary: `close` needs block.timestamp >= deadline ─────
    function testCloseDeadlineBoundary() public {
        uint64 deadline = uint64(block.timestamp + 1 days); // == _postEscrow's
        uint256 id = _postEscrow(100e18);
        vm.warp(deadline - 1);
        vm.expectRevert(bytes("not yet"));
        board.close(id);
        vm.warp(deadline);
        board.close(id); // exactly at the deadline settles
        assertEq(scry.balanceOf(buyer), 1_000e18); // refunded, never delivered
    }

    // ── operator knobs (TEST-AUDIT: "near-universally untested") ─────────
    function testSetFeeBpsOverCapReverts() public {
        vm.expectRevert(bytes("fee > 10%"));
        board.setFeeBps(1001);
    }

    function testSetFeeBpsNonOperatorReverts() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        board.setFeeBps(100);
    }

    // ── the admin has a door out: the operator can be handed to a new key,
    //    a multisig, or a governor. Same shape as ScryReputation /
    //    ScryInsurancePool / ScryHarvest / ScryFeeSplitter. ───────────────
    function testTransferOperatorHandsOverTheKnobs() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        board.transferOperator(buyer); // a stranger cannot take the keys

        vm.expectRevert(bytes("zero"));
        board.transferOperator(address(0)); // params-only role: never strand it

        board.transferOperator(buyer);
        assertEq(board.operator(), buyer);

        vm.expectRevert(bytes("not operator"));
        board.setFeeBps(100); // the old operator is refused

        vm.prank(buyer);
        board.setFeeBps(100); // the new operator works
        assertEq(board.feeBps(), 100);
    }

    // ── the fee is read at SETTLEMENT, not locked at post: an operator can
    //    move the cut on an already-open job (up to the 10% cap).
    //    PINNED, NOT ENDORSED — see contracts/TEST-AUDIT.md ("decide: lock
    //    fee at post?" — an operator call, deliberately not changed here). ─
    function testFeeBpsChangeMidJobAppliesAtSettlement() public {
        uint256 id = _postEscrow(100e18); // posted under the 5% fee
        vm.prank(seller);
        board.deliver(id, DELIV);
        board.setFeeBps(1000); // operator raises to 10% while the job is open
        vm.prank(buyer);
        board.complete(id);
        assertEq(scry.balanceOf(seller), 1_000e18 + 90e18); // NEW fee applied
        assertEq(scry.balanceOf(splitter), 10e18);
    }

    function testSetRepRewardsTakesEffectAtSettlement() public {
        board.setRepRewards(7, 13);
        assertEq(board.repReward(), 7);
        assertEq(board.repPenalty(), 13);
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        uint256 id1 = _postEscrow(100e18);
        uint256 id2 = _postEscrow(100e18);
        vm.prank(seller);
        board.deliver(id1, DELIV);
        vm.prank(buyer);
        board.complete(id1);
        assertEq(reputation.rep(seller), 107); // 100 + the new reward
        vm.warp(block.timestamp + 2 days);
        board.close(id2); // seller default on the second job
        assertEq(reputation.rep(seller), 94); // 107 - the new penalty
    }

    function testSetRepRewardsNonOperatorReverts() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        board.setRepRewards(1, 1);
    }

    function testSetArbiterTakesEffect() public {
        address newSink = address(0xA6B2);
        MockArbiter newArb = new MockArbiter(newSink, 4e18);
        board.setArbiter(IScryArbiter(address(newArb)));
        assertEq(address(board.arbiter()), address(newArb));
        uint256 id = _postEscrow(100e18);
        vm.startPrank(buyer);
        scry.approve(address(board), 4e18);
        board.dispute(id); // flat fee flows to the NEW arbiter's sink
        vm.stopPrank();
        assertEq(scry.balanceOf(newSink), 4e18);
        assertEq(scry.balanceOf(arbFeeSink), 0);
    }

    function testSetArbiterNonOperatorReverts() public {
        MockArbiter newArb = new MockArbiter(arbFeeSink, 1e18);
        address newArbAddr = address(newArb); // hoisted: no calls after the prank
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        board.setArbiter(IScryArbiter(newArbAddr));
    }

    function testSetFeeSplitterTakesEffect() public {
        address newSplit = address(0xFEE6);
        board.setFeeSplitter(newSplit);
        assertEq(board.feeSplitter(), newSplit);
        uint256 id = _postEscrow(100e18);
        vm.prank(seller);
        board.deliver(id, DELIV);
        vm.prank(buyer);
        board.complete(id);
        assertEq(scry.balanceOf(newSplit), 5e18); // cut lands at the new sink
        assertEq(scry.balanceOf(splitter), 0);
    }

    function testSetFeeSplitterZeroReverts() public {
        vm.expectRevert(bytes("zero"));
        board.setFeeSplitter(address(0));
    }

    function testSetFeeSplitterNonOperatorReverts() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        board.setFeeSplitter(address(0xFEE6));
    }

    // ── the insurance pool, addressed directly (TEST-AUDIT: cap / auth /
    //    thin-pool untested) ─────────────────────────────────────────────
    function testPoolClaimUnauthorizedReverts() public {
        // this test contract is NOT a claimant (only the board is)
        vm.expectRevert(bytes("not claimant"));
        pool.claim(seller, 1e18, 0);
    }

    function testPoolClaimCappedAtMaxPayout() public {
        pool.setClaimant(address(this), true);
        scry.mint(address(this), 5_000e18);
        scry.approve(address(pool), 5_000e18);
        pool.payPremium(5_000e18);
        assertEq(pool.reserves(), 5_000e18); // reserves == pool's token balance
        uint256 paid = pool.claim(seller, 3_000e18, 7); // above the 1_000e18 cap
        assertEq(paid, 1_000e18); // pays exactly the posted cap
        assertEq(scry.balanceOf(seller), 1_000e18 + 1_000e18);
        assertEq(pool.reserves(), 4_000e18);
    }

    function testPoolThinPoolPaysWhatItHas() public {
        pool.setClaimant(address(this), true);
        scry.mint(address(this), 10e18);
        scry.approve(address(pool), 10e18);
        pool.payPremium(10e18);
        uint256 paid = pool.claim(seller, 500e18, 8); // under the cap, over reserves
        assertEq(paid, 10e18); // best-effort: a thin pool pays what it has
        assertEq(scry.balanceOf(seller), 1_000e18 + 10e18);
        assertEq(pool.reserves(), 0);
    }

    // ── the false-return token dialect (TEST-AUDIT HIGH: "only one token
    //    behavior ever tested") ──────────────────────────────────────────
    function testFalseReturnEscrowPostUnfundedReverts() public {
        MockSCRYFalse fscry = new MockSCRYFalse();
        ScryInsurancePool fpool = new ScryInsurancePool(IERC20(address(fscry)), 1_000e18);
        ScryJobBoard fboard = new ScryJobBoard(
            IERC20(address(fscry)),
            IReputation(address(reputation)),
            IInsurancePool(address(fpool)),
            splitter,
            IScryArbiter(address(arbiter)),
            MAX_ARB_FEE
        );
        // buyer holds no fscry and gave no approval → transferFrom returns
        // FALSE (does not revert); the board's require must wrap it
        vm.startPrank(buyer);
        vm.expectRevert(bytes("escrow pull failed"));
        fboard.post(seller, 100e18, ScryJobBoard.Mode.Escrow, SPEC, uint64(block.timestamp + 1 days), 0);
        vm.stopPrank();
    }

    function testFalseReturnInsuredBuyerDefaultCoveredByPool() public {
        // Insured mode now demands the same reputation bar as an unsecured
        // hire — coverage is the pool's money, and a burner seller was the
        // whole shape of the drain (audit 2026-07-25, test/PoolDrain.t.sol).
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 100, "seed");
        MockSCRYFalse fscry = new MockSCRYFalse();
        ScryInsurancePool fpool = new ScryInsurancePool(IERC20(address(fscry)), 1_000e18);
        ScryJobBoard fboard = new ScryJobBoard(
            IERC20(address(fscry)),
            IReputation(address(reputation)),
            IInsurancePool(address(fpool)),
            splitter,
            IScryArbiter(address(arbiter)),
            MAX_ARB_FEE
        );
        reputation.setAuthority(address(fboard), true);
        fpool.setClaimant(address(fboard), true);
        fboard.setInsuredOpen(true); // ships closed; opened deliberately for this case
        // fund the pool
        fscry.mint(address(this), 500e18);
        fscry.approve(address(fpool), 500e18);
        fpool.payPremium(500e18);
        // buyer HOLDS the amount but approves only the premium — _tryPull's
        // transferFrom will return FALSE, and must read as "unfunded"
        fscry.mint(buyer, 1_000e18);
        vm.startPrank(buyer);
        fscry.approve(address(fboard), 3e18);
        uint256 id =
            fboard.post(seller, 100e18, ScryJobBoard.Mode.Insured, SPEC, uint64(block.timestamp + 1 days), 3e18);
        vm.stopPrank();
        vm.prank(seller);
        fboard.deliver(id, DELIV);
        vm.warp(block.timestamp + 2 days);
        fboard.close(id);
        // the false return routed to pool coverage, not a bogus pull:
        assertEq(fscry.balanceOf(seller), 95e18); // net, from the pool
        assertEq(fscry.balanceOf(buyer), 1_000e18 - 3e18); // only the premium left
        assertEq(fpool.reserves(), 500e18 + 3e18 - 95e18);
        // UNCHANGED at the seed (audit 2026-07-25): a covered claim is not an
        // honest completion — the counterparty defaulted and strangers'
        // premiums paid — so it makes the seller whole without also making
        // them more trusted. Earning here was the flywheel that let a
        // self-dealing pair build the reputation gating Insured mode while
        // draining the pool funding it.
        assertEq(reputation.rep(seller), 100);
    }

    // ── the governance surface TEST-AUDIT flagged as "near-universally
    //    untested" — the mis-wired-operator/ownership paths a testnet's real
    //    value catches. All pure-unit: effect + auth for each knob. ─────────

    /// dispute → close: the named double-settle ordering the suite had only
    /// in reverse (close → dispute). A disputed job is off the settle path,
    /// so close() must refuse it — else a dispute could be silently closed.
    function testCloseAfterDisputeReverts() public {
        uint256 id = _postEscrow(100e18);
        vm.startPrank(buyer);
        scry.approve(address(board), 2e18); // the flat arbiter fee
        board.dispute(id); // rules + closes the job
        vm.stopPrank();
        vm.warp(block.timestamp + 2 days);
        vm.expectRevert(bytes("closed"));
        board.close(id); // double-settle refused
    }

    /// ScryReputation.setThreshold — the gate RepOnly mode reads. Effect
    /// (meets() flips across the new threshold) + non-operator revert.
    function testReputationSetThresholdEffectAndAuth() public {
        reputation.setAuthority(address(this), true);
        reputation.earn(seller, 60, "seed");
        assertTrue(reputation.meets(seller)); // 60 >= 50 (setUp threshold)
        reputation.setThreshold(61);
        assertFalse(reputation.meets(seller)); // the knob actually moved the gate
        assertEq(reputation.threshold(), 61);
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        reputation.setThreshold(1);
    }

    /// ScryReputation.setAuthority — who may mint/slash rep. The effect path
    /// runs in setUp; here the non-operator revert (an open setAuthority is a
    /// rep-minting hole).
    function testReputationSetAuthorityNonOperatorReverts() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        reputation.setAuthority(buyer, true);
    }

    /// ScryReputation.transferOperator — hand-off correctness end to end.
    function testReputationTransferOperator() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        reputation.transferOperator(buyer);
        vm.expectRevert(bytes("zero"));
        reputation.transferOperator(address(0));
        reputation.transferOperator(buyer); // this (operator) hands off
        assertEq(reputation.operator(), buyer);
        vm.expectRevert(bytes("not operator")); // old operator refused now
        reputation.setThreshold(1);
        vm.prank(buyer);
        reputation.setThreshold(7); // new operator works
        assertEq(reputation.threshold(), 7);
    }

    /// ScryInsurancePool.setMaxPayout — the per-claim cap. Effect (a claim
    /// caps at the NEW value) + non-operator revert.
    function testPoolSetMaxPayoutEffectAndAuth() public {
        pool.setClaimant(address(this), true);
        scry.mint(address(this), 5_000e18);
        scry.approve(address(pool), 5_000e18);
        pool.payPremium(5_000e18);
        pool.setMaxPayout(200e18);
        assertEq(pool.maxPayout(), 200e18);
        uint256 paid = pool.claim(seller, 3_000e18, 1); // asks big; capped at new max
        assertEq(paid, 200e18);
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        pool.setMaxPayout(1);
    }

    /// ScryInsurancePool.setClaimant — who may drain the pool. Non-operator
    /// revert (the effect is exercised in setUp).
    function testPoolSetClaimantNonOperatorReverts() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        pool.setClaimant(buyer, true);
    }

    /// ScryInsurancePool.transferOperator — hand-off + guards.
    function testPoolTransferOperator() public {
        vm.prank(buyer);
        vm.expectRevert(bytes("not operator"));
        pool.transferOperator(buyer);
        vm.expectRevert(bytes("zero"));
        pool.transferOperator(address(0));
        pool.transferOperator(buyer);
        assertEq(pool.operator(), buyer);
        vm.expectRevert(bytes("not operator"));
        pool.setMaxPayout(1);
        vm.prank(buyer);
        pool.setMaxPayout(42e18);
        assertEq(pool.maxPayout(), 42e18);
    }
}
