// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IScryArbiter} from "./IScryArbiter.sol";
import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

interface IReputation {
    function earn(address who, uint256 amount, bytes32 reason) external;
    function slash(address who, uint256 amount, bytes32 reason) external;
    function meets(address who) external view returns (bool);
}

interface IInsurancePool {
    function claim(address to, uint256 amount, uint256 jobId) external returns (uint256);
}

/// @title ScryJobBoard — hire a named worker for a task, with a trust mode
/// @notice The on-chain spine of the agent-labor economy (AGENT-ECONOMY.md).
///         A buyer hires a named seller for a task carrying a HASHED completion
///         criterion and a deadline, under one of three trust modes:
///
///           - Escrow          buyer locks the amount; released on completion,
///                             refunded on timeout. Full protection both ways.
///           - Insured         buyer locks nothing but pays a premium to the
///                             pool; on buyer default the seller is covered
///                             from the pool (capped). Capital-lite — and
///                             SHIPS CLOSED (`insuredOpen` = false), because
///                             mutual insurance against a counterparty you
///                             chose yourself is exploitable by construction
///                             until an underwriting step exists. See the
///                             field's own note and AUDIT-2026-07-25.md §4b.
///           - ReputationOnly  nothing locked; seller must meet the rep
///                             threshold; default just slashes soulbound rep.
///
///         Disputes go to an arbiter that is paid a FLAT fee INDEPENDENT of its
///         verdict (the anti-Bar-Hadya guarantee, in code). The house takes a
///         posted fee off each sale to the splitter; completion earns soulbound
///         rep, default/adverse rulings slash it. Money moves labor, coverage,
///         and fees — never a measurement, never a ruling.
///
///         THE OPERATOR'S REACH, stated exactly (audit 2026-07-25 — the older
///         "no owner-drain" line was simply false). The operator holds NO
///         withdraw, seize, or sweep path: escrowed funds move only along a
///         job's own outcome. What it DOES hold is `setArbiter`, and an arbiter
///         is external code on the settlement path, so a hostile one is the
///         board's real trust boundary. Two structural bounds, not promises:
///           - the job is CLOSED before the arbiter is called, so a reentrant
///             arbiter cannot make one job settle twice out of the pooled
///             escrow (it used to be able to — that was the bug);
///           - the arbitration fee is capped by the IMMUTABLE `maxArbFee`, so
///             an arbiter cannot name an unbounded fee against whatever the
///             disputing party has approved (it used to be able to).
///         An arbiter can still RULE badly. That is what a court is; it is
///         bounded by the panel design in ScryArbiter, not by this contract.
contract ScryJobBoard is ReentrancyGuard {
    enum Mode {
        Escrow,
        Insured,
        ReputationOnly
    }
    enum Status {
        None,
        Posted,
        Delivered,
        Closed
    }

    struct Job {
        address buyer;
        address seller;
        uint256 amount; // SCRY paid to the seller on completion (pre-fee)
        Mode mode;
        Status status;
        bytes32 specHash; // the committed completion criterion — the court rules vs THIS
        bytes32 deliveredHash;
        uint64 deadline;
    }

    IERC20 public immutable scry;
    IReputation public immutable rep;
    IInsurancePool public insurancePool;
    address public feeSplitter; // ScryFeeSplitter — every sale's cut lands here
    IScryArbiter public arbiter;
    address public operator;

    /// The most a dispute may ever cost the party opening it, whatever arbiter
    /// the operator points at. Immutable on purpose: a ceiling the operator can
    /// raise is not a ceiling, and this is the only thing standing between a
    /// swapped arbiter and a disputing party's standing SCRY approval.
    uint256 public immutable maxArbFee;

    uint16 public feeBps = 500; // 5%, posted; capped at 10%
    uint256 public repReward = 10; // rep earned on an honest completion
    uint256 public repPenalty = 25; // rep slashed on default / adverse ruling

    /// The floor a premium must clear, as bps of the amount it may claim.
    /// Insured coverage is the POOL's money, not the buyer's, so a premium
    /// unrelated to the sum insured is free money for anyone willing to be both
    /// sides of a job. Posted, capped at 50%. (audit 2026-07-25 — see post().)
    uint16 public minPremiumBps = 300;

    /// INSURED MODE SHIPS DISARMED (operator, 2026-07-25). The rep gate, the
    /// priced premium and the pool's per-beneficiary window raise the cost of
    /// self-dealing and cap its rate; none of them makes it unprofitable in
    /// the limit, because nothing on-chain distinguishes a real hire from one
    /// person wearing two addresses. The general fix is an UNDERWRITING step —
    /// somebody accepts the specific risk — and that does not exist yet. So
    /// the mode is compiled, tested, and closed at the door: `post()` in
    /// `Mode.Insured` reverts until the operator flips this. Escrow and
    /// ReputationOnly are unaffected and ship open. Flipping it is a
    /// deliberate act against a pool whose size IS the exposure limit.
    bool public insuredOpen; // default false — see AUDIT-2026-07-25.md §4b

    Job[] public jobs;

    event Posted(
        uint256 indexed id,
        address indexed buyer,
        address indexed seller,
        uint256 amount,
        Mode mode,
        bytes32 specHash,
        uint64 deadline
    );
    event Delivered(uint256 indexed id, bytes32 deliveredHash);
    event Closed(uint256 indexed id, bytes32 outcome, uint256 toSeller, uint256 fee);
    event Ruled(uint256 indexed id, IScryArbiter.Verdict verdict);
    event ParamSet(bytes32 what, uint256 value);
    event OperatorTransferred(address indexed from, address indexed to);

    modifier onlyOperator() {
        require(msg.sender == operator, "not operator");
        _;
    }

    constructor(
        IERC20 _scry,
        IReputation _rep,
        IInsurancePool _pool,
        address _feeSplitter,
        IScryArbiter _arbiter,
        uint256 _maxArbFee
    ) {
        require(_feeSplitter != address(0), "zero splitter");
        scry = _scry;
        rep = _rep;
        insurancePool = _pool;
        feeSplitter = _feeSplitter;
        arbiter = _arbiter;
        maxArbFee = _maxArbFee;
        operator = msg.sender;
    }

    // ── post ─────────────────────────────────────────────────────────────
    function post(address seller, uint256 amount, Mode mode, bytes32 specHash, uint64 deadline, uint256 premium)
        external nonReentrant
        returns (uint256 id)
    {
        require(seller != address(0) && seller != msg.sender, "bad seller");
        require(amount > 0 && specHash != bytes32(0), "bad job");
        require(deadline > block.timestamp, "deadline passed");
        if (mode == Mode.ReputationOnly) require(rep.meets(seller), "seller below rep threshold");
        if (mode == Mode.Escrow) {
            SafeERC20.safeTransferFrom(address(scry), msg.sender, address(this), amount, "escrow pull failed");
        } else if (mode == Mode.Insured) {
            // WHY INSURED IS GATED LIKE REPUTATION-ONLY (audit 2026-07-25).
            // Escrow risks the buyer's own money, so the board can be
            // indifferent to who they are. Insured risks the POOL's money —
            // other people's premiums — on a buyer default. Nothing here can
            // tell a real hire from a (buyer, seller) pair that is one person:
            // post, deliver garbage, let the deadline pass, and the pool covers
            // the "seller" in full. Measured before the fix: ten rounds of a
            // ONE WEI premium with a throwaway pair each time took 9,500 SCRY
            // out of a 10,000 SCRY pool, and the rep slash landed on burner
            // addresses that never intended to be hired again. So an insured
            // hire must clear BOTH bars a self-dealer cannot cheaply clear:
            //   - the seller carries the same soulbound reputation a wholly
            //     unsecured hire demands (burners have none, and rep is earned
            //     over real completed jobs — never bought);
            //   - the premium is priced against the sum it may claim, so a
            //     round costs money proportional to what it can take.
            // Neither is sufficient alone; the pool's own per-beneficiary
            // window cap is the third bound and the one that caps the loss.
            // And none of the three is a WALL — see `insuredOpen` above; the
            // mode is closed at the door until an underwriting step exists.
            require(insuredOpen, "insured mode is closed");
            require(rep.meets(seller), "seller below rep threshold");
            require(premium > 0 && premium >= (amount * minPremiumBps) / 10_000, "premium below the posted floor");
            SafeERC20.safeTransferFrom(address(scry), msg.sender, address(insurancePool), premium, "premium pull failed");
        }
        id = jobs.length;
        jobs.push(Job(msg.sender, seller, amount, mode, Status.Posted, specHash, bytes32(0), deadline));
        emit Posted(id, msg.sender, seller, amount, mode, specHash, deadline);
    }

    // ── deliver ──────────────────────────────────────────────────────────
    function deliver(uint256 id, bytes32 deliveredHash) external nonReentrant {
        Job storage j = jobs[id];
        require(msg.sender == j.seller, "not seller");
        require(j.status == Status.Posted, "not open");
        j.deliveredHash = deliveredHash;
        j.status = Status.Delivered;
        emit Delivered(id, deliveredHash);
    }

    // ── complete (buyer confirms the delivery meets the spec) ────────────
    /// @dev `nonReentrant` added 2026-07-27 — it was the ONE state-changing entry
    ///      point without it, which made "every fund path on this board is
    ///      guarded" untrue by grep. No exploit was found: the status flip below
    ///      closes the job before `_payoutSeller` reaches a token, so a reentrant
    ///      `complete`/`close`/`dispute` on this job hits a status guard either
    ///      way. But that safety rested entirely on those two lines staying in
    ///      that order, unstated and unenforced — which is the condition the
    ///      guard exists to stop depending on.
    function complete(uint256 id) external nonReentrant {
        Job storage j = jobs[id];
        require(msg.sender == j.buyer, "not buyer");
        require(j.status == Status.Delivered, "not delivered");
        j.status = Status.Closed; // checks-effects: close before paying
        _payoutSeller(j, id, false);
    }

    // ── close after deadline (no human needed) ───────────────────────────
    /// Anyone may call once the deadline passes:
    ///   - not delivered  → seller defaulted → refund/settle to buyer, slash seller
    ///   - delivered      → buyer went silent → seller is paid/covered, slash buyer
    function close(uint256 id) external nonReentrant {
        Job storage j = jobs[id];
        require(j.status == Status.Posted || j.status == Status.Delivered, "closed");
        require(block.timestamp >= j.deadline, "not yet");
        if (j.status == Status.Posted) {
            j.status = Status.Closed;
            _refundBuyer(j, id, "seller-default");
            rep.slash(j.seller, repPenalty, "seller-default");
        } else {
            j.status = Status.Closed;
            _payoutSeller(j, id, true); // buyer silent on a delivered job → seller made whole
            rep.slash(j.buyer, repPenalty, "buyer-default");
        }
    }

    // ── dispute → the flat-fee court ─────────────────────────────────────
    function dispute(uint256 id) external nonReentrant {
        Job storage j = jobs[id];
        require(msg.sender == j.buyer || msg.sender == j.seller, "not a party");
        require(j.status == Status.Posted || j.status == Status.Delivered, "closed");

        // CHECKS-EFFECTS-INTERACTIONS. The arbiter is external, operator-set
        // code, and `rule` is non-view on the interface, so it is a CALL and it
        // can reenter. Close the job FIRST: an arbiter that reenters close() or
        // dispute() on this job now hits the "closed" guard instead of making
        // it settle a second time out of the pooled escrow (which is another
        // buyer's money). Everything below reads `j` from storage and does not
        // depend on the status field.
        j.status = Status.Closed;

        // Guarantee §9.1: the arbiter is paid a FLAT fee BEFORE ruling, from
        // the party opening the case, identical whatever the verdict — and
        // never more than the immutable ceiling this board was deployed with.
        uint256 fee = arbiter.flatFee();
        require(fee <= maxArbFee, "arb fee over the posted ceiling");
        if (fee > 0) SafeERC20.safeTransferFrom(address(scry), msg.sender, arbiter.feeRecipient(), fee, "arb fee failed");

        // Reverts if the panel has not ruled — the whole dispute rolls back,
        // fee and status change with it.
        IScryArbiter.Verdict v = arbiter.rule(id, j.specHash, j.deliveredHash);
        emit Ruled(id, v);

        if (v == IScryArbiter.Verdict.ForSeller) {
            _payoutSeller(j, id, true); // deliverable met the committed spec
        } else if (v == IScryArbiter.Verdict.ForBuyer) {
            // an EARNED adverse outcome: refund the buyer, slash the seller
            _refundBuyer(j, id, "ruled-for-buyer");
            rep.slash(j.seller, repPenalty, "ruled-for-buyer");
        } else {
            // Undecided: the panel DECLINED to decide — refund, never a slash.
            // A slash is a verdict-like act and rep moves only on an earned
            // outcome (CREED #9); punishing a no-basis case would also pay
            // spurious disputes in seller damage. Operator decision 2026-07-22.
            _refundBuyer(j, id, "undecided");
        }
    }

    // ── internals: the only paths funds may take ─────────────────────────
    function _payoutSeller(Job storage j, uint256 id, bool coverIfUnfunded) internal {
        uint256 fee = (j.amount * feeBps) / 10_000;
        uint256 net = j.amount - fee;
        // The event reports what ACTUALLY moved — never the nominal amounts
        // (CREED #4: never fake a number; the record is the product).
        uint256 paid = net;
        uint256 feePaid = fee;
        bytes32 outcome = "completed";
        bool earnsRep = true;
        if (j.mode == Mode.Escrow) {
            SafeERC20.safeTransfer(address(scry), j.seller, net, "pay seller failed");
            if (fee > 0) SafeERC20.safeTransfer(address(scry), feeSplitter, fee, "fee failed");
        } else {
            // Insured / ReputationOnly: the amount was never escrowed. Try to
            // pull it from the buyer now (voluntary completion path)…
            bool pulled = _tryPull(j.buyer, address(this), j.amount);
            if (pulled) {
                SafeERC20.safeTransfer(address(scry), j.seller, net, "pay seller failed");
                if (fee > 0) SafeERC20.safeTransfer(address(scry), feeSplitter, fee, "fee failed");
            } else if (coverIfUnfunded && j.mode == Mode.Insured) {
                // …else, on buyer default, cover the seller from the pool. The
                // pool may pay LESS than net (cap / thin pool / the
                // per-beneficiary window) — report the amount it actually paid,
                // and no house fee was taken.
                paid = insurancePool.claim(j.seller, net, id);
                feePaid = 0;
                outcome = "completed-insured";
                // NO REP IS EARNED HERE (audit 2026-07-25). A covered claim is
                // not an honest completion: the counterparty defaulted and
                // strangers' premiums paid instead. Rewarding it let a
                // self-dealing pair BUILD the very reputation that gates
                // Insured mode while draining the pool that funds it — a
                // flywheel, not a market. Cover makes the seller whole; it
                // never also makes them more trusted.
                earnsRep = false;
            } else {
                // Not pulled and not covered. On the VOLUNTARY complete() path
                // (coverIfUnfunded == false) a buyer must NOT be able to close a
                // job as "completed" while paying the seller nothing — revert so
                // they approve SCRY first (or let it reach the deadline, where
                // Insured is covered by the pool). On the close()-after-deadline
                // path (coverIfUnfunded == true), an uncovered ReputationOnly
                // seller ate the loss by design — say so with honest zeros.
                require(coverIfUnfunded, "buyer has not funded - approve SCRY or await deadline");
                paid = 0;
                feePaid = 0;
                outcome = "completed-unfunded";
            }
            // ReputationOnly with a defaulting buyer at deadline: no capital
            // recovery — the seller ate the loss; the buyer's rep was slashed.
        }
        if (earnsRep) rep.earn(j.seller, repReward, "completed");
        emit Closed(id, outcome, paid, feePaid);
    }

    function _refundBuyer(Job storage j, uint256 id, bytes32 reason) internal {
        if (j.mode == Mode.Escrow) {
            SafeERC20.safeTransfer(address(scry), j.buyer, j.amount, "refund failed");
        }
        // Insured / ReputationOnly: nothing was locked, nothing to refund.
        emit Closed(id, reason, 0, 0);
    }

    function _tryPull(address from, address to, uint256 amount) internal returns (bool) {
        (bool ok, bytes memory ret) =
            address(scry).call(abi.encodeWithSelector(IERC20.transferFrom.selector, from, to, amount));
        return ok && (ret.length == 0 || abi.decode(ret, (bool)));
    }

    // ── operator knobs (params only; never a fund path) ──────────────────
    function setFeeBps(uint16 bps) external onlyOperator {
        require(bps <= 1_000, "fee > 10%");
        feeBps = bps;
        emit ParamSet("feeBps", bps);
    }

    function setMinPremiumBps(uint16 bps) external onlyOperator {
        require(bps <= 5_000, "premium floor > 50%");
        minPremiumBps = bps;
        emit ParamSet("minPremiumBps", bps);
    }

    /// Open or close Insured mode. Ships CLOSED; opening it is a deliberate
    /// act, and the insurance pool's balance is the real exposure limit
    /// whatever this says. Closing it never touches a job already posted —
    /// live insured jobs settle exactly as they were posted to.
    function setInsuredOpen(bool open) external onlyOperator {
        insuredOpen = open;
        emit ParamSet("insuredOpen", open ? 1 : 0);
    }

    function setRepRewards(uint256 reward, uint256 penalty) external onlyOperator {
        repReward = reward;
        repPenalty = penalty;
        emit ParamSet("repReward", reward);
        emit ParamSet("repPenalty", penalty);
    }

    function setArbiter(IScryArbiter _arbiter) external onlyOperator {
        arbiter = _arbiter;
        emit ParamSet("arbiter", uint256(uint160(address(_arbiter))));
    }

    function setFeeSplitter(address _s) external onlyOperator {
        require(_s != address(0), "zero");
        feeSplitter = _s;
        emit ParamSet("feeSplitter", uint256(uint160(_s)));
    }

    /// @notice Rotate the operator. Same shape as ScryReputation /
    /// ScryInsurancePool / ScryHarvest / ScryFeeSplitter: the admin can be
    /// handed to a new key, a multisig, or a governor. The zero-check is
    /// deliberate — the operator governs params only (never a fund path), so
    /// stranding it would freeze the posted fee/arbiter knobs with no way back.
    function transferOperator(address to) external onlyOperator {
        require(to != address(0), "zero");
        emit OperatorTransferred(operator, to);
        operator = to;
    }

    function jobCount() external view returns (uint256) {
        return jobs.length;
    }
}
