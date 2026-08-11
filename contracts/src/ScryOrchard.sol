// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SpoilsToken} from "./SpoilsToken.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";
import {
    IUniswapV3PoolState,
    INonfungiblePositionManagerOrchard,
    IUniswapV3FactoryOrchard
} from "./interfaces/IUniswapV3Orchard.sol";

/// @title ScryOrchard - the v3-staker farm for the canonical pools (FARMING.md)
/// @notice The fence FARMING.md 7 kept up - "a plain MasterChef overpays
///         concentrated ranges" - taken down the only honest way: by not
///         using MasterChef math. The Orchard is scry's rewrite of the
///         Uniswap v3 staker's incentive design. A position's share of a
///         season's pot is proportional to LIQUIDITY x SECONDS-IN-RANGE,
///         read from the pool's own `snapshotCumulativesInside` clock:
///         a razor-thin range only earns while price actually sits inside
///         it, and a full-range position earns all the time at its lower
///         liquidity. Nobody is overpaid; the pool is the referee.
///
///         The shape is SEASONS, not a drip: each incentive is a posted,
///         finite pot of the game coin over a posted [start, end] window, funded by a
///         real transfer at creation (in production: a deliberate
///         `granary.stewardMint` -> approve -> create, the same human-act
///         funding rule as the harvest ledger - the Orchard never holds a
///         granary grant and can never mint). What a season does not pay
///         out, the operator sweeps back after it ends. Emission can never
///         exceed the posted pot.
///
///         Custody: the staked NFT is the staker's own deposit, moved by
///         their own signature, returned on demand once unstaked. The
///         Orchard never mints, burns, or edits liquidity through the
///         position manager - custody in, custody out.
///
///         Score-blind, same line as every farm in town: the reward is a
///         pure function of (liquidity, seconds-in-range, posted pot,
///         posted window). No meter number, vow, or score keys anything.
///         The pool's price is still never an oracle for any scry system -
///         the Orchard reads the range CLOCK, never the price. Posted
///         mechanics, not promised APY. And the reward is a GAME COIN -
///         **OBOL** since 2026-07-25, when MYRRH became the premium/IAP coin
///         and the rule that always kept SCRY off the farm came to point at
///         MYRRH too. Which coin is bound is a DEPLOY-TIME choice (the
///         constructor takes one SpoilsToken); what never bends is that
///         SCRY is never farm-emitted, here or anywhere.
contract ScryOrchard is ReentrancyGuard {
    struct Incentive {
        address pool; // the canonical v3 pool this season pays
        uint256 totalRewardUnclaimed; // reward coin still in the pot
        uint160 totalSecondsClaimedX128; // liquidity-seconds already cashed
        uint256 startTime;
        uint256 endTime;
        uint96 numberOfStakes;
    }

    struct Deposit {
        address owner; // who parked the NFT (and who rewards credit to)
        bool staked;
        uint256 incentiveId;
        int24 tickLower; // captured at stake; the NFT is frozen in custody
        int24 tickUpper;
        uint128 liquidity;
        uint160 secondsPerLiquidityInsideInitialX128;
    }

    SpoilsToken public immutable rewardToken;
    INonfungiblePositionManagerOrchard public immutable npm;
    IUniswapV3FactoryOrchard public immutable factory;
    address public owner;

    Incentive[] public incentives;
    mapping(uint256 => Deposit) public deposits; // tokenId -> deposit
    mapping(address => uint256) public rewardsOf; // accrued reward coin, claimable

    event IncentiveCreated(
        uint256 indexed incentiveId, address indexed pool, uint256 reward, uint256 startTime, uint256 endTime
    );
    event IncentiveEnded(uint256 indexed incentiveId, uint256 refund);
    event TokenDeposited(uint256 indexed tokenId, address indexed owner);
    event TokenStaked(uint256 indexed tokenId, uint256 indexed incentiveId, uint128 liquidity);
    event TokenUnstaked(uint256 indexed tokenId, uint256 indexed incentiveId, uint256 reward);
    event TokenWithdrawn(uint256 indexed tokenId, address indexed to);
    event RewardClaimed(address indexed who, uint256 amount);
    event OwnerTransferred(address indexed from, address indexed to);
    /// An NFT that arrived without the receiver hook, handed back.
    event OrphanRescued(uint256 indexed tokenId, address indexed to);

    modifier onlyOwner() {
        require(msg.sender == owner, "owner only");
        _;
    }

    constructor(SpoilsToken _rewardToken, INonfungiblePositionManagerOrchard _npm, IUniswapV3FactoryOrchard _factory) {
        // Third of the same shape, after ScryGardener.addPool (07-28) and
        // ScrySilo.addBin (07-29). `SpoilsToken` here is a cast over a raw
        // deploy-time address whose only other guard is `!= address(0)`, and
        // `createIncentive` pulls with `safeTransferFrom` on the "the type is
        // the guard" rationale - which SafeERC20._check does not provide,
        // because a CALL to an empty account returns ok with empty returndata.
        // With a codeless REWARD_TOKEN nothing anywhere refuses: the season
        // reports itself funded while holding nothing, stakers accrue normally,
        // and `claimReward` reports success while paying zero, from
        // IncentiveCreated all the way to RewardClaimed.
        require(address(_rewardToken).code.length > 0, "reward has no code");
        require(address(_npm).code.length > 0, "npm has no code");
        require(address(_factory).code.length > 0, "factory has no code");
        rewardToken = _rewardToken;
        npm = _npm;
        factory = _factory;
        owner = msg.sender;
    }

    function incentiveCount() external view returns (uint256) {
        return incentives.length;
    }

    // -- seasons (operator-posted, pot-funded, sweepable) -------------------
    /// Post a season: `reward` of the bound coin over [startTime, endTime] on `pool`.
    /// The pot moves here NOW, by transferFrom - a season is funded the
    /// moment it is posted or it does not exist.
    function createIncentive(address pool, uint256 reward, uint256 startTime, uint256 endTime)
        external
        onlyOwner
        nonReentrant
        returns (uint256 incentiveId)
    {
        require(pool != address(0), "zero pool");
        require(reward > 0, "zero pot");
        require(startTime >= block.timestamp, "start in past");
        require(endTime > startTime, "bad window");
        // totalSecondsClaimedX128 is a uint160 and the window is shifted 128
        // bits into it, so a window wider than 2^32 seconds (~136 years) would
        // silently truncate the accumulator. Refuse it instead.
        //
        // ⚠ THIS GUARD IS NARROWER THAN THE CAST IT WAS WRITTEN FOR, and the
        // gap is guard drift rather than an oversight. It was exact when the
        // denominator was the FROZEN `(endTime - startTime) << 128`. The
        // camper fix made the post-close denominator grow with wall time:
        // `unstake` uses `clockEnd = max(block.timestamp, endTime)`, so the
        // value reaching `uint160(secondsInsideX128)` is bounded by
        // `clockEnd - startTime`, which has no upper bound here. The bound is
        // still ~2^32 seconds measured FROM startTime, so a season left
        // un-ended for ~136 years after it opened is what it would take —
        // recorded rather than re-required because tightening the require
        // would refuse legitimate windows without closing the real path.
        require(endTime - startTime <= type(uint32).max, "window too long");
        // Deliberately `safeTransferFrom`, not `pullExact`: `rewardToken` is
        // typed `SpoilsToken`, whose `_move` is a plain move — burning is an
        // explicit `burn`/`burnFrom` call, never a transfer-side skim — so the
        // pull always lands exactly. The TYPE is the guard here; a balance
        // measurement would buy nothing and cost a staticcall.
        SafeERC20.safeTransferFrom(address(rewardToken), msg.sender, address(this), reward, "pot in");
        incentiveId = incentives.length;
        incentives.push(
            Incentive({
                pool: pool,
                totalRewardUnclaimed: reward,
                totalSecondsClaimedX128: 0,
                startTime: startTime,
                endTime: endTime,
                numberOfStakes: 0
            })
        );
        emit IncentiveCreated(incentiveId, pool, reward, startTime, endTime);
    }

    /// Sweep what a finished season did not pay out. Only after the window
    /// closes and every stake has left - a staker can never be swept out
    /// from under their own accrual.
    function endIncentive(uint256 incentiveId) external onlyOwner nonReentrant returns (uint256 refund) {
        Incentive storage inc = incentives[incentiveId];
        require(block.timestamp > inc.endTime, "season running");
        require(inc.numberOfStakes == 0, "stakes remain");
        refund = inc.totalRewardUnclaimed;
        require(refund > 0, "nothing left");
        inc.totalRewardUnclaimed = 0;
        SafeERC20.safeTransfer(address(rewardToken), msg.sender, refund, "refund out");
        emit IncentiveEnded(incentiveId, refund);
    }

    // -- custody in ---------------------------------------------------------
    /// The receiver hook records an NFT only when the canonical position
    /// manager's safe-transfer path reaches this contract. The preferred
    /// holder flow is `depositToken()` below; it keeps the recipient fixed in
    /// the contract call an interface or agent prepares, rather than asking a
    /// user to manually type an NFT transfer. A bare ERC-721 `transferFrom`
    /// never calls this hook and is deliberately not presented as a deposit.
    function onERC721Received(address, address from, uint256 tokenId, bytes calldata) external returns (bytes4) {
        require(msg.sender == address(npm), "not the position manager");
        // Unreachable today (the Orchard grants no approvals on NFTs it holds,
        // so the hook cannot re-fire for a token already in custody), but a
        // record must never be overwritten while it exists: safe by
        // construction, not by that external invariant.
        require(deposits[tokenId].owner == address(0), "already recorded");
        deposits[tokenId] = Deposit({
            owner: from,
            staked: false,
            incentiveId: 0,
            tickLower: 0,
            tickUpper: 0,
            liquidity: 0,
            secondsPerLiquidityInsideInitialX128: 0
        });
        emit TokenDeposited(tokenId, from);
        return this.onERC721Received.selector;
    }

    /// Holder-initiated deposit. The holder first approves this Orchard for
    /// `tokenId` in the canonical NPM, then calls this function. The Orchard
    /// invokes `safeTransferFrom`, which must call the hook above and record
    /// the same holder as owner; otherwise the whole transaction reverts.
    /// This is a custody move, never an approval for spending token balances.
    function depositToken(uint256 tokenId) external nonReentrant {
        require(deposits[tokenId].owner == address(0), "already deposited");
        npm.safeTransferFrom(msg.sender, address(this), tokenId);
        require(deposits[tokenId].owner == msg.sender, "deposit failed");
    }

    // -- stake / unstake ----------------------------------------------------
    /// Enter a deposited position into a running season. The position must
    /// belong to the season's pool (checked against the factory, never
    /// trusted from calldata) and carry live liquidity.
    function stakeToken(uint256 tokenId, uint256 incentiveId) external nonReentrant {
        Deposit storage d = deposits[tokenId];
        require(d.owner == msg.sender, "not your deposit");
        require(!d.staked, "already staked");
        Incentive storage inc = incentives[incentiveId];
        require(block.timestamp >= inc.startTime, "season not started");
        require(block.timestamp < inc.endTime, "season over");
        require(inc.totalRewardUnclaimed > 0, "pot empty");

        (,, address token0, address token1, uint24 fee, int24 tickLower, int24 tickUpper, uint128 liquidity,,,,) =
            npm.positions(tokenId);
        require(factory.getPool(token0, token1, fee) == inc.pool, "wrong pool");
        require(liquidity > 0, "no liquidity");

        (, uint160 secondsPerLiquidityInsideX128,) =
            IUniswapV3PoolState(inc.pool).snapshotCumulativesInside(tickLower, tickUpper);

        d.staked = true;
        d.incentiveId = incentiveId;
        d.tickLower = tickLower;
        d.tickUpper = tickUpper;
        d.liquidity = liquidity;
        d.secondsPerLiquidityInsideInitialX128 = secondsPerLiquidityInsideX128;
        inc.numberOfStakes += 1;
        emit TokenStaked(tokenId, incentiveId, liquidity);
    }

    /// Leave a season and cash the accrued share. The owner may unstake any
    /// time; once the season's window has closed, ANYONE may - so a sleeping
    /// staker can never block the season's sweep - and the reward still
    /// credits the deposit's owner, never the caller.
    function unstakeToken(uint256 tokenId) external nonReentrant returns (uint256 reward) {
        Deposit storage d = deposits[tokenId];
        require(d.staked, "not staked");
        Incentive storage inc = incentives[d.incentiveId];
        require(d.owner == msg.sender || block.timestamp > inc.endTime, "not your deposit");

        (, uint160 secondsPerLiquidityInsideX128,) =
            IUniswapV3PoolState(inc.pool).snapshotCumulativesInside(d.tickLower, d.tickUpper);

        // The pool clock is designed to be compared by wrapping subtraction.
        uint160 deltaX128;
        unchecked {
            deltaX128 = secondsPerLiquidityInsideX128 - d.secondsPerLiquidityInsideInitialX128;
        }
        uint256 secondsInsideX128 = uint256(deltaX128) * d.liquidity;

        // Canonical v3-staker sharing: the pot pays out proportional to
        // liquidity-seconds against the season's remaining, still-unclaimed
        // liquidity-seconds. AFTER the close the denominator grows with wall
        // time, exactly as the canonical staker's does - it HAS to, because
        // the pool clock this contract just read keeps extrapolating for a
        // position that stays in range after endTime. Against a frozen
        // window denominator that extrapolation was a subsidy for camping:
        // an identical position unstaked one window late cashed exactly
        // 2.000x the at-close amount (measured on a live fork, 2026-07-27),
        // taken out of the remainder the operator sweeps back - so not
        // leaving was strictly dominant and `endIncentive` waited on the
        // camper. Growing the denominator makes a late in-range exit pay
        // its unchanged share (the "never paid less for unstaking late"
        // this comment used to claim - the frozen shape paid them MORE) and
        // a late out-of-range exit decay, the canonical trade. The residual
        // canonical case - a staker who IS the range's whole active
        // liquidity can still absorb the remainder asymptotically - is why
        // unstake is permissionless after the close: clear the stakes, then
        // sweep. A stake still can never cash more seconds than the season
        // has left, so the share (and the uint160 accumulator) stays
        // bounded and the reward by the pot. Products here fit 256 bits for
        // any sane pot x window (pot 1e27 x a century of seconds<<128 is
        // still short of 2^256).
        uint256 clockEnd = block.timestamp > inc.endTime ? block.timestamp : inc.endTime;
        uint256 totalSecondsUnclaimedX128 = ((clockEnd - inc.startTime) << 128) - inc.totalSecondsClaimedX128;
        // A fully-cashed season (several concurrent in-range stakes can sum to
        // more liquidity-seconds than the window holds) pays later unstakers 0
        // - but it must never divide by zero and trap their NFT staked, which
        // would also block endIncentive forever. Exits always complete.
        if (totalSecondsUnclaimedX128 > 0) {
            if (secondsInsideX128 > totalSecondsUnclaimedX128) secondsInsideX128 = totalSecondsUnclaimedX128;
            reward = (inc.totalRewardUnclaimed * secondsInsideX128) / totalSecondsUnclaimedX128;
            inc.totalSecondsClaimedX128 += uint160(secondsInsideX128);
            inc.totalRewardUnclaimed -= reward;
        }
        inc.numberOfStakes -= 1;
        d.staked = false;
        if (reward > 0) rewardsOf[d.owner] += reward;
        emit TokenUnstaked(tokenId, d.incentiveId, reward);
    }

    /// Take a resting (unstaked) position back out of custody.
    function withdrawToken(uint256 tokenId, address to) external nonReentrant {
        Deposit storage d = deposits[tokenId];
        require(d.owner == msg.sender, "not your deposit");
        require(!d.staked, "unstake first");
        require(to != address(0) && to != address(this), "bad to");
        delete deposits[tokenId];
        npm.safeTransferFrom(address(this), to, tokenId);
        emit TokenWithdrawn(tokenId, to);
    }

    /// Cash out accrued reward coin. Plain transfer from the pots this contract
    /// already holds - the Orchard cannot mint.
    function claimReward(address to) external nonReentrant returns (uint256 amount) {
        amount = rewardsOf[msg.sender];
        require(amount > 0, "nothing accrued");
        rewardsOf[msg.sender] = 0;
        SafeERC20.safeTransfer(address(rewardToken), to, amount, "reward out");
        emit RewardClaimed(msg.sender, amount);
    }

    function transferOwner(address to) external onlyOwner {
        require(to != address(0), "zero owner");
        emit OwnerTransferred(owner, to);
        owner = to;
    }

    /// Return an ORPHANED position NFT — one this contract holds but has no
    /// deposit record for. That state has exactly one cause: someone moved
    /// their position in with a bare `transferFrom` instead of
    /// `safeTransferFrom`, so `onERC721Received` never fired and no owner was
    /// recorded. Before this existed the NFT was lost forever: `withdrawToken`
    /// needs `deposits[id].owner == msg.sender` and nobody satisfies that,
    /// while `depositToken` reverts because the caller no longer holds it.
    ///
    /// WHY THIS IS NOT AN OWNER-SEIZE PATH, structurally rather than by
    /// promise: the require below is `owner == address(0)`, and EVERY position
    /// deposited properly has a non-zero owner from the moment the hook runs.
    /// A staked position has one too. So this function provably cannot reach a
    /// single NFT anyone deposited correctly — the property stakers actually
    /// care about ("the owner cannot take my staked position") is untouched.
    /// What it costs is the honest admission that returning an orphan is a
    /// SOCIAL act: the owner must send it to the right person, identified from
    /// the transfer that brought it in. The alternative is certain loss, on a
    /// chain where a bare transferFrom is a click away in most wallets.
    function rescueOrphan(uint256 tokenId, address to) external onlyOwner nonReentrant {
        require(deposits[tokenId].owner == address(0), "not an orphan - it has a depositor");
        require(to != address(0) && to != address(this), "bad to");
        require(npm.ownerOf(tokenId) == address(this), "this contract does not hold it");
        emit OrphanRescued(tokenId, to);
        npm.safeTransferFrom(address(this), to, tokenId);
    }
}
