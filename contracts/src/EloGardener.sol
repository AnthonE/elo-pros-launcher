// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {EloGranary} from "./EloGranary.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";
import {Math} from "./Math.sol";

/// @title EloGardener - the LP farm (Master Gardener homage, FARMING.md)
/// @notice Stake Garden SEED shares (or any posted LP token), earn MYRRH by
///         the posted emission schedule. This is DFK's original Harmony
///         Gardens mechanic with reserve's one structural correction: the coin
///         being emitted is a GAME coin (elastic, distributor-gated,
///         burned by the town's sinks), never SCRY. SCRY is the base
///         pairing asset, the JEWEL role, and it is never farm-minted, so
///         the money token does not inflate to pay for its own liquidity.
///
///         The DFK trademarks, kept on purpose (the older version):
///         - allocation points split one emission stream across pools;
///         - a share of every harvest is LOCKED and cliff-unlocks at a
///           posted timestamp (anti mercenary-farming);
///         - a withdrawal slash that decays with time staked, 25% for a
///           same-block exit down to 0.01% for a patient one. Slashed LP
///           shares go to 0xdEaD: flash-farmers donate permanent liquidity
///           to everyone who stayed.
///
///         Emission is a pure function of (staked share, elapsed time,
///         posted rate). It reads no meter number, no vow, no score, and
///         never will: rewarding LP time is participation, and the posted
///         schedule is the whole formula (`TOKENOMICS.md`, the one line that
///         never bends). Posted
///         mechanics, not promised APY; the outcome floats with the pool.
///
///         MYRRH is minted through the EloGranary, clamped to the farm's
///         posted daily cap. When the granary clamps, the shortfall stays
///         credited in the staker's stash and pays out on a later harvest;
///         nothing is lost, nothing bricks.
contract EloGardener is ReentrancyGuard {
    // -- accounting ---------------------------------------------------------
    struct UserInfo {
        uint256 amount; // staked LP shares
        uint256 rewardDebt; // amount * accPerShare at last settle (1e12)
        uint256 stash; // earned, unpaid (granary clamped) immediate MYRRH
        uint256 lastDeposit; // slash clock; RESET on every deposit (DFK rule)
    }

    struct PoolInfo {
        IERC20 lp; // the staked token (Garden SEED shares)
        uint256 allocPoint; // share of the emission stream
        uint256 lastRewardTime; // last accPerShare update
        uint256 accPerShare; // accumulated MYRRH per LP share, 1e12 scaled
    }

    uint256 private constant ACC = 1e12;
    address public constant DEAD = 0x000000000000000000000000000000000000dEaD;

    EloGranary public immutable granary; // MYRRH mints go through here
    uint256 public immutable unlockTime; // the cliff for locked rewards
    address public owner;

    uint256 public rewardPerSecond; // ERA-0 emission rate (MYRRH wei/s); halves — see below
    uint256 public lockBps; // share of each harvest that locks
    uint256 public totalAllocPoint;

    // ── THE HALVING SCHEDULE ────────────────────────────────────────────────
    // Operator, 2026-07-28: "i want people to farm MYRRH like bitcoin ... early
    // users get way more than people later, max 40 year run way".
    //
    // So emission is not flat. It HALVES every four years and then STOPS DEAD,
    // which is what makes "max 40 year run" a fact about the contract rather
    // than a projection: after `emissionEnd()` this farm mints nothing, ever,
    // and no owner call can restart it — the terminal is a constant, not a knob.
    //
    // Both constants are welded on purpose. A halving schedule an owner could
    // reschedule is not a halving schedule, it is a promise; the whole reason
    // this shape is worth copying is that the holder does not have to trust
    // anyone about the future supply.
    //
    // ⚠ WHAT THE OWNER CAN STILL DO, said plainly so no surface overclaims:
    // `setRewardPerSecond` moves the ERA-0 BASE, and every era is derived from
    // it, so the owner can still scale the whole curve up or down. The SHAPE
    // (halve every 4y, stop at 40y) is immutable; the HEIGHT is not. This is
    // weaker than Bitcoin and must never be described as equivalent. Making it
    // equivalent is one line — a require() that the base only ever decreases —
    // and it is deliberately NOT taken here, because the rate has already been
    // moved in both directions once (halved 2026-07-26, restored 07-27) and a
    // ratchet would have blocked the restore. That is an operator call, not a
    // contract one; see FARMING.md for the tradeoff.
    uint256 public constant HALVING_PERIOD = 4 * 365 days;
    uint256 public constant HALVINGS = 10; // 10 * 4y = the 40-year run

    /// @notice Upper bound on the era-0 base, and the one knob-guard that is
    ///         about liveness rather than policy. `updatePool` computes
    ///         `reward * ACC` (:211) where `reward` is bounded by the whole
    ///         remaining lifetime emission, so a large enough base overflows
    ///         there — and `withdraw` and `emergencyWithdraw` BOTH route
    ///         through `updatePool`, so the revert takes every exit with it.
    ///         `setRewardPerSecond` mass-updates before mutating, which means
    ///         the owner cannot even lower the rate afterwards: there is no
    ///         path home and staked principal is trapped.
    ///         1e30 wei/s is ~14 orders of magnitude above the shipped era-0
    ///         base (6,000/day is ~6.94e16) and ~26 below the overflow, so it
    ///         constrains no reachable policy while removing the brick.
    uint256 public constant MAX_REWARD_PER_SECOND = 1e30;
    uint256 public immutable startTime; // era 0 opens at deploy

    PoolInfo[] public pools;
    mapping(uint256 => mapping(address => UserInfo)) public userInfo;
    mapping(address => bool) public lpAdded; // no duplicate pools
    mapping(address => uint256) public lockedOf; // per staker, cliff-gated

    /// Sum of every UserInfo.amount in a pool — the emission denominator.
    /// Deliberately NOT `lp.balanceOf(address(this))`: LP shares sent in by a
    /// plain transfer have no UserInfo row, so they can never claim, but a
    /// balance-based denominator would still hand them a share of the stream —
    /// stranding it and diluting every real staker, permanently, for the price
    /// of one transfer. (The MasterChef-V1 donation bug; audit 2026-07-25.)
    /// A donation is now simply inert: it earns nothing and moves nothing.
    /// There is no path back out for it, and deliberately no owner sweep —
    /// an owner able to move this token could move staked principal.
    mapping(uint256 => uint256) public stakedSupply;

    event PoolAdded(uint256 indexed pid, address indexed lp, uint256 allocPoint);
    event AllocSet(uint256 indexed pid, uint256 allocPoint);
    event RateSet(uint256 rewardPerSecond);
    event LockBpsSet(uint256 lockBps);
    event Deposit(address indexed user, uint256 indexed pid, uint256 amount);
    event Withdraw(address indexed user, uint256 indexed pid, uint256 amount, uint256 slashed);
    event Harvest(address indexed user, uint256 indexed pid, uint256 paid, uint256 locked, uint256 stashed);
    event LockedClaim(address indexed user, uint256 paid, uint256 remaining);
    event EmergencyWithdraw(
        address indexed user, uint256 indexed pid, uint256 amount, uint256 slashed, uint256 forfeited
    );
    event OwnerTransferred(address indexed from, address indexed to);

    modifier onlyOwner() {
        require(msg.sender == owner, "owner only");
        _;
    }

    constructor(EloGranary _granary, uint256 _rewardPerSecond, uint256 _lockBps, uint256 _unlockTime) {
        require(_lockBps <= 9000, "lock too high");
        require(_rewardPerSecond <= MAX_REWARD_PER_SECOND, "rate too high");
        require(_unlockTime > block.timestamp, "unlock in past");
        granary = _granary;
        rewardPerSecond = _rewardPerSecond;
        lockBps = _lockBps;
        unlockTime = _unlockTime;
        startTime = block.timestamp;
        owner = msg.sender;
    }

    // -- the schedule, read-only ---------------------------------------------

    /// The second emission stops for good. Nothing mints after this.
    function emissionEnd() public view returns (uint256) {
        return startTime + HALVING_PERIOD * HALVINGS;
    }

    /// The rate right now, after however many halvings have elapsed. Zero once
    /// the run is over. This is the number a UI should show — `rewardPerSecond`
    /// is the era-0 base and is only equal to it during the first four years.
    function currentRewardPerSecond() public view returns (uint256) {
        if (block.timestamp >= emissionEnd()) return 0;
        return rewardPerSecond >> ((block.timestamp - startTime) / HALVING_PERIOD);
    }

    /// Total MYRRH the schedule emits across [from, to), summed era by era.
    ///
    /// This replaces `elapsed * rewardPerSecond`, which is only correct inside
    /// a single era. An interval spanning a halving boundary must be split, or
    /// a pool left un-updated across one would be paid the OLD rate for the
    /// whole span — the farm's largest single overpayment, arriving exactly at
    /// the moment the schedule exists to prevent it.
    ///
    /// The loop is bounded by HALVINGS + 1 = 11 iterations (both ends are
    /// clamped into the run), and costs one iteration in every ordinary call.
    function emittedBetween(uint256 from, uint256 to) public view returns (uint256 total) {
        uint256 end = emissionEnd();
        if (from < startTime) from = startTime;
        if (to > end) to = end;
        if (to <= from) return 0;
        while (from < to) {
            uint256 era = (from - startTime) / HALVING_PERIOD; // < HALVINGS, so the shift is safe
            uint256 eraEnd = startTime + (era + 1) * HALVING_PERIOD;
            uint256 seg = (to < eraEnd ? to : eraEnd) - from;
            total += seg * (rewardPerSecond >> era);
            from += seg;
        }
    }

    // -- the DFK slash ladder (the older Harmony schedule, verbatim) --------
    /// bps slashed off a withdrawal, by time since the LAST deposit.
    function withdrawFeeBps(uint256 sinceDeposit) public pure returns (uint256) {
        if (sinceDeposit == 0) return 2500; // same block: 25%
        if (sinceDeposit < 1 hours) return 800; // 8%
        if (sinceDeposit < 1 days) return 400; // 4%
        if (sinceDeposit < 3 days) return 200; // 2%
        if (sinceDeposit < 5 days) return 100; // 1%
        if (sinceDeposit < 2 weeks) return 50; // 0.5%
        if (sinceDeposit < 4 weeks) return 25; // 0.25%
        return 1; // 0.01%
    }

    // -- pool bookkeeping ---------------------------------------------------
    function poolCount() external view returns (uint256) {
        return pools.length;
    }

    /// Scaled remainder per pool: reward*ACC not yet attributable per-share.
    /// Without it, any interval where (reward*ACC)/lpSupply floors to 0 loses
    /// that emission FOREVER while lastRewardTime still advances — at
    /// lpSupply > rps*ACC a per-second caller (updatePool is public) zeroes
    /// the stream permanently. The carry makes per-second and patient callers
    /// arrive at identical state. Operator decision 2026-07-22.
    mapping(uint256 => uint256) public carryScaled;

    function updatePool(uint256 pid) public {
        PoolInfo storage p = pools[pid];
        if (block.timestamp <= p.lastRewardTime) return;
        uint256 lpSupply = stakedSupply[pid];
        if (lpSupply == 0 || p.allocPoint == 0 || totalAllocPoint == 0) {
            p.lastRewardTime = block.timestamp;
            return;
        }
        // mulDiv, not `a * b / c`. The quotient is bounded (allocPoint <=
        // totalAllocPoint, so reward <= emitted) but the 256-bit intermediate
        // is not — and a revert here takes every exit with it, including the
        // setAllocPoint that would undo it, because that mass-updates first.
        // Same shape as the rate cap; the cap bounds the other factor only.
        uint256 reward = Math.mulDiv(emittedBetween(p.lastRewardTime, block.timestamp), p.allocPoint, totalAllocPoint);
        uint256 scaled = reward * ACC + carryScaled[pid];
        uint256 inc = scaled / lpSupply;
        p.accPerShare += inc;
        carryScaled[pid] = scaled - inc * lpSupply;
        p.lastRewardTime = block.timestamp;
    }

    function massUpdatePools() public {
        for (uint256 i = 0; i < pools.length; i++) {
            updatePool(i);
        }
    }

    /// Unsettled accrual (pre lock-split) plus carried stash for a staker,
    /// as of now. On settle, the fresh part splits lockBps/rest; the stash
    /// pays out whole. Already-locked balances live in lockedOf, not here.
    function pendingMyrrh(uint256 pid, address who) external view returns (uint256) {
        PoolInfo storage p = pools[pid];
        UserInfo storage u = userInfo[pid][who];
        uint256 acc = p.accPerShare;
        uint256 lpSupply = stakedSupply[pid];
        if (block.timestamp > p.lastRewardTime && lpSupply > 0 && totalAllocPoint > 0) {
            uint256 reward = Math.mulDiv(emittedBetween(p.lastRewardTime, block.timestamp), p.allocPoint, totalAllocPoint);
            acc += (reward * ACC) / lpSupply;
        }
        return (u.amount * acc) / ACC - u.rewardDebt + u.stash;
    }

    // -- settle: split fresh accrual, mint through the granary, carry -------
    // The stash is already post-split (it is the clamped remainder of an
    // earlier immediate portion), so only FRESH accrual takes the lock cut;
    // re-splitting the carry would double-lock it.
    // The granary call is try/catch on purpose (the EloSilo rule): a revoked
    // grant or rotated-away minter must defer rewards (carried in the stash),
    // never trap principal behind a revert. Deposits and withdrawals depend
    // on nothing but this contract's own LP balance.
    function _settle(uint256 pid, address who) internal {
        PoolInfo storage p = pools[pid];
        UserInfo storage u = userInfo[pid][who];
        uint256 fresh = (u.amount * p.accPerShare) / ACC - u.rewardDebt;
        uint256 lockedPart = (fresh * lockBps) / 10_000;
        uint256 immediate = fresh - lockedPart + u.stash;
        u.stash = 0;
        if (lockedPart > 0) lockedOf[who] += lockedPart;
        uint256 paid = 0;
        if (immediate > 0) {
            try granary.mint(who, immediate) returns (uint256 minted) {
                paid = minted;
            } catch {
                paid = 0;
            }
            if (paid < immediate) u.stash = immediate - paid; // carry, never lose
        }
        if (fresh > 0 || paid > 0) {
            emit Harvest(who, pid, paid, lockedPart, u.stash);
        }
    }

    // -- staker actions -----------------------------------------------------
    function deposit(uint256 pid, uint256 amount) external nonReentrant {
        updatePool(pid);
        PoolInfo storage p = pools[pid];
        UserInfo storage u = userInfo[pid][msg.sender];
        _settle(pid, msg.sender);
        if (amount > 0) {
            // `pullExact`: `u.amount` and `stakedSupply` are credited the
            // STATED amount on the next two lines, so a token delivering less
            // mints farm weight that no deposit backs. Same class as the
            // EloBank fix; also the second half of the codeless-LP guard in
            // `addPool` — a tolerant transfer is what let an empty account
            // report success at all.
            SafeERC20.pullExact(address(p.lp), msg.sender, address(this), amount, "lp in");
            u.amount += amount;
            stakedSupply[pid] += amount;
            u.lastDeposit = block.timestamp; // DFK rule: deposits reset the slash clock
        }
        u.rewardDebt = (u.amount * p.accPerShare) / ACC;
        emit Deposit(msg.sender, pid, amount);
    }

    function withdraw(uint256 pid, uint256 amount) external nonReentrant {
        updatePool(pid);
        PoolInfo storage p = pools[pid];
        UserInfo storage u = userInfo[pid][msg.sender];
        require(u.amount >= amount, "too much");
        _settle(pid, msg.sender);
        uint256 slashed = 0;
        if (amount > 0) {
            u.amount -= amount;
            stakedSupply[pid] -= amount; // the whole amount leaves: user share + the DEAD slash
            slashed = (amount * withdrawFeeBps(block.timestamp - u.lastDeposit)) / 10_000;
            if (slashed > 0) {
                // permanent liquidity: the slash deepens the pool floor forever
                SafeERC20.safeTransfer(address(p.lp), DEAD, slashed, "lp slash");
            }
            SafeERC20.safeTransfer(address(p.lp), msg.sender, amount - slashed, "lp out");
        }
        u.rewardDebt = (u.amount * p.accPerShare) / ACC;
        emit Withdraw(msg.sender, pid, amount, slashed);
    }

    /// Claim cliff-unlocked rewards. Clamped by the granary like everything
    /// else; call again if the daily budget ran dry mid-claim.
    function claimLocked() external nonReentrant returns (uint256 paid) {
        require(block.timestamp >= unlockTime, "still locked");
        uint256 owed = lockedOf[msg.sender];
        require(owed > 0, "nothing locked");
        // ZERO BEFORE THE CALL, re-credit the shortfall after — the twin of
        // `EloSilo.claimOwed` (audit 2026-07-27). This was the cleanest of the
        // four read-mint-write sites: a reentrant claim read the full locked
        // balance again and minted it a second time, bounded only by the
        // granary's daily cap. `_settle` above is NOT in that class — it is only
        // reachable from `deposit`/`withdraw`, both of which are guarded.
        lockedOf[msg.sender] = 0;
        paid = granary.mint(msg.sender, owed);
        if (paid < owed) lockedOf[msg.sender] = owed - paid;
        emit LockedClaim(msg.sender, paid, owed - paid);
    }

    /// Exit without rewards: forfeits pending + stash, keeps the locked
    /// balance (it was already earned), still pays the slash ladder.
    function emergencyWithdraw(uint256 pid) external nonReentrant {
        PoolInfo storage p = pools[pid];
        UserInfo storage u = userInfo[pid][msg.sender];
        uint256 amount = u.amount;
        require(amount > 0, "nothing staked");
        updatePool(pid);
        uint256 forfeited = (amount * p.accPerShare) / ACC - u.rewardDebt + u.stash;
        u.amount = 0;
        u.rewardDebt = 0;
        u.stash = 0;
        stakedSupply[pid] -= amount;
        uint256 slashed = (amount * withdrawFeeBps(block.timestamp - u.lastDeposit)) / 10_000;
        if (slashed > 0) {
            SafeERC20.safeTransfer(address(p.lp), DEAD, slashed, "lp slash");
        }
        SafeERC20.safeTransfer(address(p.lp), msg.sender, amount - slashed, "lp out");
        emit EmergencyWithdraw(msg.sender, pid, amount, slashed, forfeited);
    }

    // -- operator knobs (posted, deliberate, massUpdate first) --------------
    function addPool(IERC20 lp, uint256 allocPoint) external onlyOwner {
        require(!lpAdded[address(lp)], "pool exists");
        // An LP address with NO CODE mints a phantom stake. `SafeERC20._check`
        // treats `ok == true && ret.length == 0` as success, and a CALL to an
        // empty account returns exactly that - so `deposit` would credit
        // `u.amount` and `stakedSupply[pid]` against a token that never moved,
        // and the position would farm a real emission stream out of nothing.
        // The reachable trigger is SEED_SECONDARY (DeployGardener.s.sol:121),
        // an operator-supplied raw address whose only other guard is
        // `!= address(reward)`; nothing exports or validates it, and preflight
        // checks no farm LP for code. One extcodesize at pool-add time is the
        // whole fix, and it is free - this is an onlyOwner call made once.
        require(address(lp).code.length > 0, "lp has no code");
        massUpdatePools(); // never dilute already-accrued rewards retroactively
        lpAdded[address(lp)] = true;
        totalAllocPoint += allocPoint;
        pools.push(PoolInfo({lp: lp, allocPoint: allocPoint, lastRewardTime: block.timestamp, accPerShare: 0}));
        emit PoolAdded(pools.length - 1, address(lp), allocPoint);
    }

    function setAllocPoint(uint256 pid, uint256 allocPoint) external onlyOwner {
        massUpdatePools();
        totalAllocPoint = totalAllocPoint - pools[pid].allocPoint + allocPoint;
        pools[pid].allocPoint = allocPoint;
        emit AllocSet(pid, allocPoint);
    }

    /// Move the ERA-0 BASE. Every era is `base >> era`, so this rescales the
    /// whole remaining curve; it cannot reschedule or extend it. The halving
    /// period and the 40-year terminal are constants and are not reachable
    /// from here. `massUpdatePools` settles at the old base first, so accrual
    /// already earned is never repriced.
    function setRewardPerSecond(uint256 rate) external onlyOwner {
        require(rate <= MAX_REWARD_PER_SECOND, "rate too high");
        massUpdatePools();
        rewardPerSecond = rate;
        emit RateSet(rate);
    }

    /// ⚠ THIS KNOB IS RETROACTIVE, and no amount of `massUpdatePools()` makes
    /// it otherwise. `_settle` splits `fresh` using the CURRENT `lockBps`, and
    /// `fresh` is `(u.amount * p.accPerShare)/ACC - u.rewardDebt` — accrual a
    /// staker earned under the OLD value but has not settled yet. Raising the
    /// lock re-prices it on the way out.
    ///
    /// AUDIT-2026-07-25.md filed this as "the one owner knob that does not
    /// massUpdatePools()" and prescribed adding the call. That prescription is
    /// wrong and is recorded here so it is not re-adopted: `massUpdatePools`
    /// writes `p.accPerShare`, `carryScaled` and `p.lastRewardTime` and touches
    /// no `UserInfo` row, so a staker's `fresh` is unchanged in magnitude by
    /// it. It would not prevent one wei of the retroactive lock. The analogy to
    /// `setAllocPoint` / `setRewardPerSecond` is false — there the mass-update
    /// genuinely freezes the old RATE into `accPerShare` before the knob moves.
    ///
    /// There is no O(1) fix. The real remedies are removing this setter or
    /// snapshotting lockBps per user at settle time, both of which are changes
    /// to the posted farm terms rather than bug fixes. Posted as a held power
    /// in HELD-KEYS.md instead, which is the honest handling.
    function setLockBps(uint256 bps) external onlyOwner {
        require(bps <= 9000, "lock too high");
        lockBps = bps;
        emit LockBpsSet(bps);
    }

    function transferOwner(address to) external onlyOwner {
        require(to != address(0), "zero owner");
        emit OwnerTransferred(owner, to);
        owner = to;
    }
}
