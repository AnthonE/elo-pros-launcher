// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SpoilsToken} from "./SpoilsToken.sol";
import {EloGranary} from "./EloGranary.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";
import {Math} from "./Math.sol";

/// @title EloSilo - sealed spoils lockers (the xJEWEL/cJEWEL homage, FARMING.md)
/// @notice The lever POOLS.md 3.5 names for when emission runs hot: "add
///         lockups (an xOBOL bank, DFK-style, parks supply out of
///         circulation)". This is that bank. Seal spoils (OBOL, and any bin
///         the operator posts) in the silo for a chosen season from a POSTED
///         tier menu; while sealed the supply is out of circulation and the
///         seal earns a reward drip, weighted by seal length - the cJEWEL
///         rule: longer commitment, more weight.
///
///         The DFK trademarks, kept on purpose:
///         - duration-weighted emission (the cJEWEL multiplier ladder,
///           captured at seal time and never re-priced);
///         - MasterChef accounting: allocation points split one posted
///           emission stream across bins;
///         - a painful early exit: breaking a seal before maturity BURNS a
///           slice of the principal (linear from `maxBreakBps` at seal time
///           down to zero at maturity) and forfeits unpaid rewards. DFK
///           routed penalties to project pots; reserve retires them -
///           SpoilsToken.burn is open, so the slice leaves supply forever.
///           An impatient exit is itself a sink.
///
///         WHICH COIN THIS PAYS is not decided in this file. The reward is
///         whatever `granary.spoils()` binds, fixed at construction, and every
///         deploy path binds the OBOL granary: `deploy_town.sh silo` dies
///         without GRANARY_OBOL, and DeploySilo.s.sol asserts the binding
///         on-chain before it broadcasts. This natspec said MYRRH in nine
///         places until 2026-07-28, and `pendingMyrrh` was in the public ABI -
///         the coin the operator explicitly REJECTED for the silo when the
///         farm moved to MYRRH and the two granaries split (FARMING.md 3a).
///         Read the granary, never this comment.
///
///         reserve's structural corrections, same as the Gardener:
///         - the reward is a game coin - never SCRY (the money token
///           does not inflate to pay for parking);
///         - every mint goes through the EloGranary behind a posted daily
///           cap, clamped-and-carried, never lost, never bricking;
///         - NO lock-split on harvests here: the seal IS the lock, and
///           double-locking rewards on top of a principal lock is the kind
///           of compounding fine print reserve does not do;
///         - no governance weight. cJEWEL voted; sealed spoils do not.
///
///         Score-blind by construction: emission is a pure function of
///         (sealed amount, tier weight, elapsed time, posted rate). No meter
///         number, vow trajectory, or breach flag keys any mint, weight, or
///         penalty (CLAUDE.md, the line that never moves). Posted mechanics,
///         not promised APY.
contract EloSilo is ReentrancyGuard {
    // -- accounting ---------------------------------------------------------
    struct Tier {
        uint256 duration; // seal length in seconds
        uint256 weightBps; // emission weight; 10_000 = 1x
        bool enabled; // a closed tier refuses NEW seals only
    }

    struct Bin {
        SpoilsToken spoils; // the sealed token (burnable - penalties retire)
        uint256 allocPoint; // share of the emission stream
        uint256 lastRewardTime; // last accPerShare update
        uint256 accPerShare; // accumulated reward per weighted unit, 1e12
        uint256 totalWeighted; // sum of open seals' weighted amounts
    }

    struct Seal {
        uint256 bin;
        uint256 tier; // menu index at seal time; the weight is captured
        uint256 amount; // principal sealed
        uint256 weighted; // amount * weightBps / 10_000 - the earning size
        uint256 sealedAt;
        uint256 maturesAt;
        uint256 rewardDebt; // weighted * accPerShare at last settle (1e12)
        uint256 stash; // earned, unpaid (granary clamped), carried
        bool open;
    }

    uint256 private constant ACC = 1e12;

    EloGranary public immutable granary; // reward mints go through here
    uint256 public immutable maxBreakBps; // seal-break burn at t=0; <= 5000
    address public owner;

    /// @notice The same brick EloGardener.MAX_REWARD_PER_SECOND removes on
    ///         2026-07-28, in a contract that is strictly MORE exposed to it and
    ///         got neither half of the fix. `updateBin` computes
    ///         `elapsed * rewardPerSecond * allocPoint` and then `reward * ACC`;
    ///         `reap`, `unseal` and `breakSeal` all route through `updateBin`,
    ///         so an overflow there takes every exit with it; and
    ///         `setRewardPerSecond` mass-updates BEFORE mutating, so the owner
    ///         cannot lower the rate afterwards either. There is no path home
    ///         and sealed principal is trapped for good.
    ///         Worse here than in the Gardener: the Silo has no emission end, so
    ///         `elapsed` grows without bound where `emittedBetween` is clamped
    ///         to the 40-year run. Same 1e30 ceiling, for the same reason - far
    ///         above any reachable policy, far below the overflow.
    ///         Added 2026-07-29 after review of the 07-28 audit fixes.
    uint256 public constant MAX_REWARD_PER_SECOND = 1e30;

    uint256 public rewardPerSecond; // posted emission rate (reward wei/s)
    uint256 public totalAllocPoint;

    Tier[] public tiers;
    Bin[] public bins;
    mapping(address => bool) public binAdded; // no duplicate bins
    mapping(address => Seal[]) public sealsOf; // every seal, closed ones kept
    mapping(address => uint256) public owedOf; // clamped payout that outlived its seal

    event TierAdded(uint256 indexed tier, uint256 duration, uint256 weightBps);
    event TierEnabled(uint256 indexed tier, bool enabled);
    event BinAdded(uint256 indexed bin, address indexed spoils, uint256 allocPoint);
    event AllocSet(uint256 indexed bin, uint256 allocPoint);
    event RateSet(uint256 rewardPerSecond);
    event Sealed(
        address indexed who,
        uint256 indexed sealId,
        uint256 indexed bin,
        uint256 tier,
        uint256 amount,
        uint256 maturesAt
    );
    event Reaped(address indexed who, uint256 indexed sealId, uint256 paid, uint256 stashed);
    event Unsealed(address indexed who, uint256 indexed sealId, uint256 amount, uint256 paid, uint256 owed);
    event SealBroken(address indexed who, uint256 indexed sealId, uint256 returned, uint256 burned, uint256 forfeited);
    event OwedClaim(address indexed who, uint256 paid, uint256 remaining);
    event OwnerTransferred(address indexed from, address indexed to);

    modifier onlyOwner() {
        require(msg.sender == owner, "owner only");
        _;
    }

    constructor(EloGranary _granary, uint256 _rewardPerSecond, uint256 _maxBreakBps) {
        require(_maxBreakBps <= 5000, "break too high");
        require(_rewardPerSecond <= MAX_REWARD_PER_SECOND, "rate too high");
        granary = _granary;
        rewardPerSecond = _rewardPerSecond;
        maxBreakBps = _maxBreakBps;
        owner = msg.sender;
    }

    // -- views --------------------------------------------------------------
    function tierCount() external view returns (uint256) {
        return tiers.length;
    }

    function binCount() external view returns (uint256) {
        return bins.length;
    }

    function sealCount(address who) external view returns (uint256) {
        return sealsOf[who].length;
    }

    /// The burn taken if `who` broke seal `sealId` right now, in bps of the
    /// principal: maxBreakBps at seal time, straight line to 0 at maturity.
    function breakFeeBps(address who, uint256 sealId) public view returns (uint256) {
        Seal storage s = sealsOf[who][sealId];
        require(s.open, "seal closed");
        if (block.timestamp >= s.maturesAt) return 0;
        uint256 duration = s.maturesAt - s.sealedAt;
        return (maxBreakBps * (s.maturesAt - block.timestamp)) / duration;
    }

    /// Unsettled accrual plus carried stash for one seal, as of now.
    function pendingReward(address who, uint256 sealId) external view returns (uint256) {
        Seal storage s = sealsOf[who][sealId];
        if (!s.open) return 0;
        Bin storage b = bins[s.bin];
        uint256 acc = b.accPerShare;
        if (block.timestamp > b.lastRewardTime && b.totalWeighted > 0 && totalAllocPoint > 0) {
            uint256 elapsed = block.timestamp - b.lastRewardTime;
            uint256 reward = Math.mulDiv(elapsed * rewardPerSecond, b.allocPoint, totalAllocPoint);
            acc += (reward * ACC) / b.totalWeighted;
        }
        return (s.weighted * acc) / ACC - s.rewardDebt + s.stash;
    }

    // -- bin bookkeeping ----------------------------------------------------
    /// Scaled remainder per bin: reward*ACC not yet attributable per weighted
    /// unit. Without it, any interval where (reward*ACC)/totalWeighted floors
    /// to 0 loses that emission FOREVER while lastRewardTime still advances —
    /// and updateBin is PUBLIC, so at totalWeighted > rewardPerSecond*ACC any
    /// address can call it every second and zero the stream permanently. The
    /// carry makes per-second and patient callers arrive at identical state.
    /// Ported verbatim from EloGardener.carryScaled (operator decision
    /// 2026-07-22, which the Silo was missed out of; audit 2026-07-25).
    mapping(uint256 => uint256) public carryScaled;

    function updateBin(uint256 binId) public {
        Bin storage b = bins[binId];
        if (block.timestamp <= b.lastRewardTime) return;
        if (b.totalWeighted == 0 || b.allocPoint == 0 || totalAllocPoint == 0) {
            b.lastRewardTime = block.timestamp;
            return;
        }
        uint256 elapsed = block.timestamp - b.lastRewardTime;
        // mulDiv, for the same reason as EloGardener.updatePool: bounded
        // quotient, unbounded intermediate, and every exit (reap / unseal /
        // breakSeal) routes through here.
        uint256 reward = Math.mulDiv(elapsed * rewardPerSecond, b.allocPoint, totalAllocPoint);
        uint256 scaled = reward * ACC + carryScaled[binId];
        uint256 inc = scaled / b.totalWeighted;
        b.accPerShare += inc;
        carryScaled[binId] = scaled - inc * b.totalWeighted;
        b.lastRewardTime = block.timestamp;
    }

    function massUpdateBins() public {
        for (uint256 i = 0; i < bins.length; i++) {
            updateBin(i);
        }
    }

    // -- settle: mint fresh accrual through the granary, carry the clamp ----
    // The granary call is try/catch on purpose: a revoked grant must defer
    // rewards (carried in the stash), never trap principal behind a revert.
    // Exits depend on nothing but this contract's own balance.
    function _settle(address who, uint256 sealId) internal returns (uint256 paid) {
        Seal storage s = sealsOf[who][sealId];
        // CHECKPOINT BEFORE THE CALL (audit 2026-07-27). `granary.mint` below is
        // external and the checkpoint used to be written AFTER it, so inside
        // that window `rewardDebt` was stale and `stash` was already zero: a
        // reentrant `reap`/`unseal` recomputed the SAME accrual and minted it
        // again, until the granary's daily cap clamped the loop. Advancing it
        // here makes a reentrant call compute `due == 0` and mint nothing, so
        // the property holds by arithmetic and the `nonReentrant` guards on the
        // callers are the second line rather than the only one.
        //
        // Reading `accPerShare` ONCE also closes a quieter version of the same
        // hole: `updateBin` is public, so a reentrant call could advance the
        // accumulator between the two reads and the checkpoint would jump past
        // accrual that was never paid — silently forfeiting it.
        uint256 acc = bins[s.bin].accPerShare;
        uint256 due = (s.weighted * acc) / ACC - s.rewardDebt + s.stash;
        s.rewardDebt = (s.weighted * acc) / ACC;
        s.stash = 0;
        if (due > 0) {
            try granary.mint(who, due) returns (uint256 minted) {
                paid = minted;
            } catch {
                paid = 0;
            }
            if (paid < due) s.stash = due - paid; // carry, never lose
        }
    }

    // -- staker actions -----------------------------------------------------
    /// Seal `amount` of bin `binId`'s spoils for tier `tierId`'s duration.
    /// Every seal is its own position with its own maturity; the tier weight
    /// is captured here and never re-priced by later menu changes.
    function seal(uint256 binId, uint256 tierId, uint256 amount) external nonReentrant returns (uint256 sealId) {
        require(amount > 0, "zero");
        Tier storage t = tiers[tierId];
        require(t.enabled, "tier closed");
        updateBin(binId);
        Bin storage b = bins[binId];
        uint256 weighted = (amount * t.weightBps) / 10_000;
        require(weighted > 0, "too small");
        // Deliberately `safeTransferFrom`, not `pullExact`, and the reason is
        // the same one EloOrchard states: `b.spoils` is TYPED `SpoilsToken`,
        // whose `_move` is a plain move — burning is an explicit `burn` call,
        // never a transfer-side skim — so the pull always lands exactly. The
        // type is the guard; a balance measurement would buy nothing and cost
        // a staticcall. EloBank and EloGardener DID need `pullExact` because
        // their pulled token is typed `IERC20` and can be anything.
        SafeERC20.safeTransferFrom(address(b.spoils), msg.sender, address(this), amount, "spoils in");
        b.totalWeighted += weighted;
        sealId = sealsOf[msg.sender].length;
        sealsOf[msg.sender].push(
            Seal({
                bin: binId,
                tier: tierId,
                amount: amount,
                weighted: weighted,
                sealedAt: block.timestamp,
                maturesAt: block.timestamp + t.duration,
                rewardDebt: (weighted * b.accPerShare) / ACC,
                stash: 0,
                open: true
            })
        );
        emit Sealed(msg.sender, sealId, binId, tierId, amount, block.timestamp + t.duration);
    }

    /// Harvest one seal's accrued reward without touching the principal.
    function reap(uint256 sealId) external nonReentrant returns (uint256 paid) {
        Seal storage s = sealsOf[msg.sender][sealId];
        require(s.open, "seal closed");
        updateBin(s.bin);
        paid = _settle(msg.sender, sealId);
        emit Reaped(msg.sender, sealId, paid, s.stash);
    }

    /// Withdraw a matured seal: full principal back, rewards settled. A
    /// granary-clamped remainder credits owedOf and stays claimable after
    /// the seal closes - closing a position never voids earned reward.
    function unseal(uint256 sealId) external nonReentrant {
        Seal storage s = sealsOf[msg.sender][sealId];
        require(s.open, "seal closed");
        require(block.timestamp >= s.maturesAt, "still sealed");
        updateBin(s.bin);
        uint256 paid = _settle(msg.sender, sealId);
        uint256 owed = s.stash;
        if (owed > 0) {
            s.stash = 0;
            owedOf[msg.sender] += owed;
        }
        s.open = false;
        Bin storage b = bins[s.bin];
        b.totalWeighted -= s.weighted;
        SafeERC20.safeTransfer(address(b.spoils), msg.sender, s.amount, "spoils out");
        emit Unsealed(msg.sender, sealId, s.amount, paid, owed);
    }

    /// Break a seal before maturity: a slice of the principal is BURNED
    /// (linear from maxBreakBps down to 0 at maturity - supply retired
    /// forever, the impatience sink) and all unpaid rewards are forfeited
    /// (never minted; forfeited emission is simply less inflation).
    function breakSeal(uint256 sealId) external nonReentrant {
        Seal storage s = sealsOf[msg.sender][sealId];
        require(s.open, "seal closed");
        require(block.timestamp < s.maturesAt, "matured - unseal");
        updateBin(s.bin);
        Bin storage b = bins[s.bin];
        uint256 forfeited = (s.weighted * b.accPerShare) / ACC - s.rewardDebt + s.stash;
        uint256 burned = (s.amount * breakFeeBps(msg.sender, sealId)) / 10_000;
        s.open = false;
        s.stash = 0;
        b.totalWeighted -= s.weighted;
        if (burned > 0) b.spoils.burn(burned); // open burn: the slice leaves supply
        SafeERC20.safeTransfer(address(b.spoils), msg.sender, s.amount - burned, "spoils out");
        emit SealBroken(msg.sender, sealId, s.amount - burned, burned, forfeited);
    }

    /// Claim reward that a granary clamp left owing after its seal closed.
    /// Clamped like everything else; call again when the budget refills.
    function claimOwed() external nonReentrant returns (uint256 paid) {
        uint256 owed = owedOf[msg.sender];
        require(owed > 0, "nothing owed");
        // ZERO BEFORE THE CALL, re-credit the shortfall after (audit
        // 2026-07-27). Read-mint-write across `granary.mint` let a reentrant
        // claim see the whole balance a second time. A clamped mint still
        // carries what it could not pay, exactly as before.
        owedOf[msg.sender] = 0;
        paid = granary.mint(msg.sender, owed);
        if (paid < owed) owedOf[msg.sender] = owed - paid;
        emit OwedClaim(msg.sender, paid, owed - paid);
    }

    // -- operator knobs (posted, deliberate, massUpdate first) --------------
    /// The tier menu is append-only: a posted commitment is never edited,
    /// only closed to new seals. Open seals captured their weight already.
    function addTier(uint256 duration, uint256 weightBps) external onlyOwner {
        require(duration > 0, "zero duration");
        require(weightBps > 0 && weightBps <= 50_000, "weight out of range");
        tiers.push(Tier({duration: duration, weightBps: weightBps, enabled: true}));
        emit TierAdded(tiers.length - 1, duration, weightBps);
    }

    function setTierEnabled(uint256 tierId, bool enabled) external onlyOwner {
        tiers[tierId].enabled = enabled;
        emit TierEnabled(tierId, enabled);
    }

    function addBin(SpoilsToken spoils, uint256 allocPoint) external onlyOwner {
        require(!binAdded[address(spoils)], "bin exists");
        // THE SAME ONE-LINE GUARD EloGardener.addPool GOT ON 2026-07-28, and
        // not porting it here is the sibling-drift this repo has been bitten by
        // before. `SpoilsToken spoils` is a CAST over a caller-supplied address,
        // not a proof - and `seal` deliberately uses `safeTransferFrom` rather
        // than `pullExact` on the argument that "the type is the guard", so
        // there is no second line of defence the way the Gardener has one.
        //
        // Against a codeless address the pull SUCCEEDS: SafeERC20._check accepts
        // `ok == true, returndata.length == 0`, which is exactly what a CALL to
        // an empty account returns. So one transposed character in DeploySilo's
        // MYRRH_TOKEN (bin 1 is validated by nothing; only bin 0's granary is
        // checked) opens a bin where ANY stranger can `seal` an arbitrary
        // `amount`, move no tokens, and then `reap` that bin's share of real
        // OBOL emissions forever. Found by review of the 07-28 audit fixes,
        // 2026-07-29.
        require(address(spoils).code.length > 0, "spoils has no code");
        massUpdateBins(); // never dilute already-accrued rewards retroactively
        binAdded[address(spoils)] = true;
        totalAllocPoint += allocPoint;
        bins.push(
            Bin({
                spoils: spoils,
                allocPoint: allocPoint,
                lastRewardTime: block.timestamp,
                accPerShare: 0,
                totalWeighted: 0
            })
        );
        emit BinAdded(bins.length - 1, address(spoils), allocPoint);
    }

    function setAllocPoint(uint256 binId, uint256 allocPoint) external onlyOwner {
        massUpdateBins();
        totalAllocPoint = totalAllocPoint - bins[binId].allocPoint + allocPoint;
        bins[binId].allocPoint = allocPoint;
        emit AllocSet(binId, allocPoint);
    }

    function setRewardPerSecond(uint256 rate) external onlyOwner {
        require(rate <= MAX_REWARD_PER_SECOND, "rate too high");
        massUpdateBins();
        rewardPerSecond = rate;
        emit RateSet(rate);
    }

    function transferOwner(address to) external onlyOwner {
        require(to != address(0), "zero owner");
        emit OwnerTransferred(owner, to);
        owner = to;
    }
}
