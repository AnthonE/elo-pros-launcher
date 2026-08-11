// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IScryArbiter} from "./IScryArbiter.sol";

/// The single thing this panel needs from the board it serves: how many jobs
/// that board has ever issued. A vote on `jobId >= jobCount()` is a vote on a
/// case that does not exist.
interface IJobBoardCases {
    function jobCount() external view returns (uint256);
}

/// @title ScryArbiter — the staked, metered dispute panel (reference impl)
/// @notice A real `IScryArbiter` for `ScryJobBoard`. The whole point of the
///         interface is that a verdict is INDEPENDENT of who pays: the board
///         pays a FLAT fee (identical whatever the ruling) BEFORE it calls
///         `rule`, and here the fee AND its recipient are `immutable` — no
///         owner, no case, and no verdict can move them. That is the
///         anti-Bar-Hadya guarantee in code (AGENT-ECONOMY.md §9.1): a court
///         you can pay for a verdict is Bar Hadya with extra steps, so the fee
///         cannot vary with the verdict, and here it structurally cannot.
///
///         It is a PANEL, not an oracle (§9.2). A set of independent arbiters,
///         each of them a metered vow (scry's own instrument turns on the
///         court: an arbiter caught channel-switching is removed here and, off
///         chain, loses stake and reputation). Members deliberate off-chain
///         against the committed spec, then each casts ONE on-chain vote
///         carrying a pointer to their signed reasoning (kept on the public
///         record, §9.4). Parties cannot choose their arbiter and no party can
///         vote. A `quorum` is a strict majority of the panel, so at most one
///         verdict can ever reach it; whichever does is the ruling. If the
///         whole bench votes without a majority, the ruling is `Undecided`,
///         which the board treats as a refund.
///
///         THE BENCH IS PINNED PER CASE at the first vote: `caseQuorum` and
///         `casePanel` snapshot the then-current quorum and panel size, and at
///         most `casePanel` votes will ever be accepted on that case. This is
///         what keeps membership churn honest mid-case: votes already cast
///         survive a member's removal, a replacement seated later may cast one
///         of the remaining slots, and because caseQuorum is a strict majority
///         of casePanel while total votes never exceed casePanel, two verdicts
///         can never both reach quorum on one case — the ruling is
///         order-independent.
///
///         ⚠ THE LIVENESS BOUND, STATED EXACTLY (audit 2026-07-27 — this
///         paragraph used to end "so no case can be stranded unfinalizable by a
///         removal", which holds only if a replacement is actually seated).
///         **Keep the bench at its pinned size while a case is open.** Shrink it
///         below `casePanel` and an open case can strand: with every remaining
///         member already voted, no further vote is possible, `cast_` never
///         reaches `casePanel` so the exhaustion branch cannot fire, and no
///         verdict holds `caseQuorum`. `finalized` stays false and `rule`
///         reverts forever. Recipe, for the avoidance of doubt: panel 5, quorum
///         3, three members vote one-each-way, `setQuorum(2)`, remove the two
///         who never voted.
///
///         **No money is trapped by that, and that is the part worth knowing.**
///         `ScryJobBoard.dispute` closes the job BEFORE calling `rule`, so
///         `rule`'s revert rolls the status change back with it: the job stays
///         open and `close()` after the deadline still releases the escrow. What
///         is lost is the arbitration path for that one job — the parties get the
///         timeout outcome instead of a verdict. That is a softer form of the
///         inversion the 2026-07-22 rebuild was written to kill, and the fix is
///         operational, not structural: seat a replacement, or leave the bench
///         alone until the case closes. It is deliberately NOT an owner-callable
///         force-finalize, which would hand the owner exactly the power over
///         verdicts this contract exists to withhold.
///
///         `rule` returns the finalized verdict and REVERTS if the panel has
///         not ruled yet — so a party can never open a dispute and force a
///         settlement the panel never decided (a premature `dispute` reverts
///         whole, the flat fee rolled back with it). "No checkable basis" is a
///         real panel verdict (`Undecided` → refund, §9.3), deliberately
///         distinct from "not ruled yet" (a revert). Taste is never
///         adjudicated here; taste-heavy jobs fall back to refund + reputation
///         off the board's own timeout path.
///
///         THE PANEL IS BOUND TO ONE BOARD, once, before it can vote at all
///         (`setBoard`, audit 2026-07-25). A vote is only accepted on a jobId
///         that board has really issued. Before this, `vote` took any uint256:
///         a panel could rule on a job that did not exist yet and `dispute()`
///         would settle instantly on a verdict cast before the evidence.
///
///         The owner governs membership, the quorum, and that one-time board
///         binding — never a fund path and never a verdict. Retuning the flat
///         fee is a redeploy + `board.setArbiter(new)`, which keeps the fee's
///         independence airtight rather than leaving a lever an owner could
///         pull per-case.
///
///         Written in an env without foundry — `forge test -vv` before any
///         broadcast, as with every contract here.
contract ScryArbiter is IScryArbiter {
    address public immutable feeRecipient; // where the FLAT fee lands; fixed forever
    uint256 public immutable flatFee; // the FLAT fee (in SCRY); fixed forever

    address public owner; // governs panel membership + quorum ONLY
    uint256 public quorum; // votes to finalize; always a strict majority
    uint256 public arbiterCount; // live panel size
    mapping(address => bool) public isArbiter;

    /// THE BOARD THIS PANEL SERVES, bound ONCE and never again (audit
    /// 2026-07-25). Until it is set, `vote` reverts: the panel is not yet a
    /// court of anything. Once set, a vote is only accepted on a jobId that
    /// board has actually issued — `vote()` used to take ANY uint256, so a
    /// panel could rule on job #9,999 before it existed and `dispute()` would
    /// settle on that verdict the instant the job was posted, on evidence that
    /// did not exist when the panel decided.
    ///
    /// One-time on purpose. It cannot go in the constructor (the board takes
    /// the arbiter's address, so the arbiter is deployed first), and an owner
    /// who could REPOINT it could point at a puppet whose `jobCount` returns
    /// whatever makes a stale verdict look fresh. Bind once, then it is as
    /// immutable as the fee.
    address public board;

    event BoardBound(address indexed board);

    mapping(uint256 => bool) public finalized; // job => ruled?
    mapping(uint256 => Verdict) private _verdict; // job => the ruling
    mapping(uint256 => mapping(address => bool)) public voted; // job => arbiter => voted?
    mapping(uint256 => uint256) public forBuyer; // job => tally
    mapping(uint256 => uint256) public forSeller;
    mapping(uint256 => uint256) public forUndecided;
    mapping(uint256 => uint256) public caseQuorum; // job => quorum pinned at first vote
    mapping(uint256 => uint256) public casePanel; // job => bench size pinned at first vote

    event OwnerSet(address indexed owner);
    event ArbiterSet(address indexed arbiter, bool active);
    event QuorumSet(uint256 quorum);
    event Voted(uint256 indexed jobId, address indexed arbiter, Verdict verdict, bytes32 reasonHash);
    event Ruled(uint256 indexed jobId, Verdict verdict, uint256 nBuyer, uint256 nSeller, uint256 nUndecided);

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    /// @param _feeRecipient where the flat arbitration fee lands (fixed forever)
    /// @param _flatFee the flat fee in SCRY the board pays to open a case
    /// @param panel the initial arbiter addresses (each a metered vow)
    /// @param _quorum votes to finalize; must be a strict majority of `panel`
    constructor(address _feeRecipient, uint256 _flatFee, address[] memory panel, uint256 _quorum) {
        require(_feeRecipient != address(0), "zero fee recipient");
        feeRecipient = _feeRecipient;
        flatFee = _flatFee;
        owner = msg.sender;
        emit OwnerSet(msg.sender);
        for (uint256 i = 0; i < panel.length; i++) {
            address a = panel[i];
            require(a != address(0) && !isArbiter[a], "bad panel member");
            isArbiter[a] = true;
            emit ArbiterSet(a, true);
        }
        arbiterCount = panel.length;
        require(_quorum > 0 && _quorum <= arbiterCount && 2 * _quorum > arbiterCount, "quorum not a majority");
        quorum = _quorum;
        emit QuorumSet(_quorum);
    }

    // ── governance: membership + quorum only (never a fund path, never a verdict) ──
    function setArbiter(address a, bool active) external onlyOwner {
        require(a != address(0), "zero");
        require(isArbiter[a] != active, "no change");
        isArbiter[a] = active;
        if (active) arbiterCount++;
        else arbiterCount--;
        // the majority invariant must survive the change; raise the quorum
        // first (setQuorum) if adding a member would break it.
        require(arbiterCount > 0 && quorum <= arbiterCount && 2 * quorum > arbiterCount, "fix quorum first");
        emit ArbiterSet(a, active);
    }

    function setQuorum(uint256 q) external onlyOwner {
        require(q > 0 && q <= arbiterCount && 2 * q > arbiterCount, "quorum not a majority");
        quorum = q;
        emit QuorumSet(q);
    }

    function transferOwnership(address o) external onlyOwner {
        require(o != address(0), "zero");
        owner = o;
        emit OwnerSet(o);
    }

    /// Bind this panel to the board it serves. Callable ONCE, by the owner,
    /// and never re-pointed — see `board` above for why re-pointing would undo
    /// the guarantee. The panel cannot vote before this is called.
    function setBoard(address b) external onlyOwner {
        require(board == address(0), "board already bound");
        require(b != address(0), "zero board");
        require(b.code.length > 0, "board is not a contract");
        board = b;
        emit BoardBound(b);
    }

    // ── the panel votes: off-chain deliberation, on-chain record ──
    /// Cast one vote on `jobId`. `reasonHash` points at the arbiter's signed
    /// off-chain reasoning (the public record). Only a panel member may vote;
    /// one vote each; nothing more once the job is finalized. The first vote
    /// pins the case's bench (caseQuorum/casePanel) so mid-case membership
    /// churn can neither strand the case nor move its bar; a member seated
    /// after the pin may still fill one of the bench's remaining slots.
    function vote(uint256 jobId, Verdict verdict, bytes32 reasonHash) external {
        require(isArbiter[msg.sender], "not an arbiter");
        // The case must EXIST on the bound board before anyone rules on it.
        // Without this the panel could pre-rule a future job and `dispute()`
        // would settle on a verdict cast before the evidence existed.
        address b = board;
        require(b != address(0), "board not bound");
        require(jobId < IJobBoardCases(b).jobCount(), "no such case on the bound board");
        require(!finalized[jobId], "already ruled");
        require(!voted[jobId][msg.sender], "already voted");
        if (casePanel[jobId] == 0) {
            caseQuorum[jobId] = quorum;
            casePanel[jobId] = arbiterCount;
        }
        uint256 cast_ = forBuyer[jobId] + forSeller[jobId] + forUndecided[jobId];
        require(cast_ < casePanel[jobId], "bench full for this case");
        voted[jobId][msg.sender] = true;
        if (verdict == Verdict.ForSeller) forSeller[jobId]++;
        else if (verdict == Verdict.ForBuyer) forBuyer[jobId]++;
        else forUndecided[jobId]++;
        emit Voted(jobId, msg.sender, verdict, reasonHash);

        // caseQuorum is a strict majority of casePanel and total votes are
        // capped at casePanel, so at most one verdict can ever reach quorum —
        // the branch order below is provably immaterial.
        uint256 q = caseQuorum[jobId];
        if (forSeller[jobId] >= q) {
            _finalize(jobId, Verdict.ForSeller);
        } else if (forBuyer[jobId] >= q) {
            _finalize(jobId, Verdict.ForBuyer);
        } else if (forUndecided[jobId] >= q) {
            _finalize(jobId, Verdict.Undecided);
        } else if (cast_ + 1 == casePanel[jobId]) {
            // the whole bench voted and no verdict won a majority → refund
            _finalize(jobId, Verdict.Undecided);
        }
    }

    function _finalize(uint256 jobId, Verdict v) internal {
        _verdict[jobId] = v;
        finalized[jobId] = true;
        emit Ruled(jobId, v, forBuyer[jobId], forSeller[jobId], forUndecided[jobId]);
    }

    // ── the board reads the ruling here (evidence args unused: the panel
    //    already weighed them off-chain; the fee is flat and already paid) ──
    /// ONLY the bound board may read a ruling as a SETTLEMENT (security review
    /// 2026-07-25). Without this, a verdict leaks across boards: job ids are
    /// per-board counters, so if a second board is ever pointed at this panel
    /// via `setArbiter`, board B's job #N would settle instantly on board A's
    /// verdict for #N — a ruling made about a different job, on evidence from
    /// a different case. Binding `vote` to one board is only half the fence;
    /// this is the other half. Anyone wanting to READ a verdict uses
    /// `verdictOf`, which is public and settles nothing.
    function rule(uint256 jobId, bytes32, bytes32) external view returns (Verdict) {
        require(msg.sender == board, "not the bound board");
        require(finalized[jobId], "panel has not ruled");
        return _verdict[jobId];
    }

    /// A transparency read: is the job ruled, and to what.
    function verdictOf(uint256 jobId) external view returns (bool isFinal, Verdict verdict) {
        return (finalized[jobId], _verdict[jobId]);
    }
}
