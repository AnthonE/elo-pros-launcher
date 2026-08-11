// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

/// @title ScryInsurancePool — a premium pool for the trust menu
/// @notice The lightest trust mode: instead of locking escrow, parties pay a
///         small premium into this pool; a covered dispute ruled for the buyer
///         pays out from the pool, capped. Funds leave only as claims-out, to
///         an authorized claimant (the job board), capped per payout.
///
///         THE OPERATOR'S REACH, stated exactly (audit 2026-07-25 — this
///         notice used to claim "there is NO owner-drain path", which was
///         false). There is no `withdraw` and no owner transfer here, but the
///         operator holds `setClaimant` and `setMaxPayout`, and those two
///         together ARE a drain: authorize any address, raise the cap, claim.
///         That is inherent — an insurance pool must be able to name who may
///         pay claims — so the honest posture is disclosure plus a door out,
///         not a false absolute. Both knobs emit events, so an unexpected
///         claimant is visible on-chain the moment it is set; the intended end
///         state is `transferOperator` to a multisig or a governor, and the
///         key belongs in HELD-KEYS.md's ledger like every other held key.
///
///         This prices RISK, never a measurement. Charging a premium to cover
///         a job does not move any meter number or ruling — and no meter number
///         keys a premium, a cap, or a payout.
contract ScryInsurancePool is ReentrancyGuard {
    IERC20 public immutable scry;
    address public operator;
    uint256 public maxPayout; // per-claim cap (posted)

    /// PER-BENEFICIARY ROLLING WINDOW (audit 2026-07-25). A per-CLAIM cap
    /// bounds one payout and nothing else: repeat it and the pool still
    /// empties. This is the bound that actually caps the loss when the claims
    /// are manufactured — one address can draw at most `windowCap` per
    /// `windowSeconds`, whatever the job board believes about the job. It
    /// defaults to exactly one full claim per day, so the honest case is
    /// unaffected and a drain becomes a slow, visible, rate-limited thing
    /// rather than a single transaction batch.
    uint256 public windowCap;
    uint64 public windowSeconds = 1 days;
    mapping(address => uint256) public paidInWindow;
    mapping(address => uint64) public windowStart;

    mapping(address => bool) public claimant; // market contracts allowed to claim

    event PremiumPaid(address indexed from, uint256 amount);
    event Claimed(address indexed to, uint256 amount, uint256 indexed jobId);
    event ClaimantSet(address indexed who, bool allowed);
    event MaxPayoutSet(uint256 maxPayout);
    event WindowSet(uint256 windowCap, uint64 windowSeconds);
    event OperatorTransferred(address indexed from, address indexed to);

    modifier onlyOperator() {
        require(msg.sender == operator, "not operator");
        _;
    }

    constructor(IERC20 _scry, uint256 _maxPayout) {
        scry = _scry;
        operator = msg.sender;
        maxPayout = _maxPayout;
        windowCap = _maxPayout; // one full claim per beneficiary per day
        emit MaxPayoutSet(_maxPayout);
        emit WindowSet(_maxPayout, windowSeconds);
    }

    /// Anyone can top the pool (a premium). Pull via transferFrom so the
    /// payer keeps control of approval.
    function payPremium(uint256 amount) external {
        require(amount > 0, "zero");
        SafeERC20.safeTransferFrom(address(scry), msg.sender, address(this), amount, "pull failed");
        emit PremiumPaid(msg.sender, amount);
    }

    /// The pool balance available to cover claims.
    function reserves() external view returns (uint256) {
        return scry.balanceOf(address(this));
    }

    /// Pay a covered claim. Only an authorized claimant (the job board), only
    /// up to the posted cap and available reserves. Never an owner withdraw.
    function claim(address to, uint256 amount, uint256 jobId) external nonReentrant returns (uint256 paid) {
        require(claimant[msg.sender], "not claimant");
        require(to != address(0), "zero");
        paid = amount > maxPayout ? maxPayout : amount;

        // roll `to`'s window forward before reading its room
        if (block.timestamp >= uint256(windowStart[to]) + windowSeconds) {
            windowStart[to] = uint64(block.timestamp);
            paidInWindow[to] = 0;
        }
        uint256 room = windowCap > paidInWindow[to] ? windowCap - paidInWindow[to] : 0;
        if (paid > room) paid = room;

        uint256 bal = scry.balanceOf(address(this));
        if (paid > bal) paid = bal; // best-effort; a thin pool pays what it has
        if (paid > 0) {
            paidInWindow[to] += paid;
            SafeERC20.safeTransfer(address(scry), to, paid, "payout failed");
            emit Claimed(to, paid, jobId);
        }
    }

    /// Room left for `who` in the current window (view, for UIs and the board).
    function claimableThisWindow(address who) external view returns (uint256) {
        if (block.timestamp >= uint256(windowStart[who]) + windowSeconds) return windowCap;
        return windowCap > paidInWindow[who] ? windowCap - paidInWindow[who] : 0;
    }

    function setClaimant(address who, bool allowed) external onlyOperator {
        claimant[who] = allowed;
        emit ClaimantSet(who, allowed);
    }

    function setMaxPayout(uint256 _maxPayout) external onlyOperator {
        maxPayout = _maxPayout;
        emit MaxPayoutSet(_maxPayout);
    }

    function setWindow(uint256 _windowCap, uint64 _windowSeconds) external onlyOperator {
        require(_windowSeconds > 0, "zero window");
        windowCap = _windowCap;
        windowSeconds = _windowSeconds;
        emit WindowSet(_windowCap, _windowSeconds);
    }

    function transferOperator(address to) external onlyOperator {
        require(to != address(0), "zero");
        emit OperatorTransferred(operator, to);
        operator = to;
    }
}
