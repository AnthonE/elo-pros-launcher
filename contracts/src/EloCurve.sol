// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";
import {Math} from "./Math.sol";

interface ICurvePositionManager {
    struct MintParams {
        address token0;
        address token1;
        uint24 fee;
        int24 tickLower;
        int24 tickUpper;
        uint256 amount0Desired;
        uint256 amount1Desired;
        uint256 amount0Min;
        uint256 amount1Min;
        address recipient;
        uint256 deadline;
    }

    struct CollectParams {
        uint256 tokenId;
        address recipient;
        uint128 amount0Max;
        uint128 amount1Max;
    }

    function factory() external view returns (address);
    function createAndInitializePoolIfNecessary(address token0, address token1, uint24 fee, uint160 sqrtPriceX96)
        external
        payable
        returns (address pool);
    function mint(MintParams calldata params)
        external
        payable
        returns (uint256 tokenId, uint128 liquidity, uint256 amount0, uint256 amount1);
    function collect(CollectParams calldata params) external payable returns (uint256 amount0, uint256 amount1);
}

interface ICurveFactory {
    function feeAmountTickSpacing(uint24 fee) external view returns (int24);
}

interface ICurvePool {
    function increaseObservationCardinalityNext(uint16 observationCardinalityNext) external;
}

/// @title EloCurve — the two-way venue a launch needs before it has a pool
/// @notice ⭐ WHY THIS EXISTS, AND IT IS A REVERSAL. `LAUNCHPAD.md` chose
///         CONTINUOUS liquidity on 2026-08-10 — no curve, no threshold, no
///         graduation — on one premise: that a copy sale is already the money,
///         so no phase exists where a buyer has paid and cannot sell. **That
///         premise was false, and the hole is in `EloLaunchpad.seed()`**, which
///         requires the vault to already hold SCRY (`reserveAmount > 0`, balance
///         checked). No copies have sold at deploy, so the vault holds nothing,
///         so either the studio funds the SCRY side themselves — reintroducing
///         the capital requirement the whole design removed — or the pool opens
///         only after N copies have sold, and every buyer before that holds a
///         coin with no venue. That is exactly the dead phase the same doc says
///         nobody runs.
///
///         Operator, 2026-08-13: *"the reserve seed needs to come from buys buying
///         the coin… i think it should just be like how pons family v2 does it
///         on rh."* This is that, in our own bytecode.
///
///         THE SHAPE, and it is pons V2's, adapted to the one thing that does
///         not bend (`GATES.md` §2.1 — a game coin pairs against SCRY and only
///         there):
///
///           - the curve holds the game's coin and quotes it in **SCRY**
///           - anyone buys from it, and **anyone can sell back to it from the
///             first trade.** That is the property the continuous design could
///             not offer, and it is the whole reason to take a curve
///           - a share of supply is **held back from the start**, and that share
///             is what becomes the pool's coin side. So every launch on the same
///             settings graduates into a pool of the same size
///           - at a threshold posted in SCRY, the curve **closes**, and a
///             permissionless `graduate()` mints a full-range v3 position that
///             **nothing can ever move**
///
///         ⚠ WHAT LEAVES, STATED EXACTLY — because `EloLaunchpad`'s headline is
///         "no value leaves except pool fees" and that sentence is NOT true of
///         this contract, by design. SCRY here leaves in exactly three ways:
///
///           1. **to a seller**, along the posted curve, at a price that is a
///              function of state and nothing else;
///           2. **into the locked position** at graduation;
///           3. **as posted trading fees**, to two addresses fixed at deploy.
///
///         There is no withdraw, no rescue, no sweep-to-owner, no pause and no
///         admin. Nobody — not the studio, not us — has a call that takes the
///         raise. (1) is the buyer's own money coming back and is strictly
///         friendlier than the design it replaces; it is not a leak, but it
///         must never be described with `EloLaunchpad`'s sentence.
///
///         ⚠ THE COIN IS FUNDED, NEVER MINTED. Same rule as the ticket's grant
///         reserve and the launch vault: somebody transfers `curveAlloc +
///         poolAlloc` here and `arm()` checks it arrived. This contract has no
///         authority over anybody's token. A launch venue that could mint would
///         make every listing a trust question, and would make `liquidity ÷
///         FDV` — the number this audience actually computes — meaningless
///         rather than merely worse.
///
///         ⚠ THE TWO PRICES DIFFER AND THE GAP IS A DEPLOY DECISION. The curve
///         closes at a marginal price set by `virtualQuote`, `curveAlloc` and
///         `threshold`; the pool opens at `threshold / poolAlloc`. Those are
///         different numbers unless somebody sizes them to match.
///         `graduationPreview()` returns both so a deploy script can refuse a
///         bad pair and a card can publish the gap. **A buyer who bought at the
///         curve's top and finds the pool opening below it was not defrauded —
///         they were not told.** Tell them.
///
///         ⚠ IT COMPOSES WITH `EloLaunchpad` WITH NO CHANGE TO IT. The curve
///         seeds the pool; the vault deepens it. `EloLaunchpad.seed()` calls
///         `createAndInitializePoolIfNecessary`, which returns the pool that
///         already exists, and then mints the vault its OWN full-range position
///         at the live price — so after graduation the copy sale's raise pairs
///         into the same pool through the vault's normal path, and the vault's
///         `sqrtPriceX96` argument is ignored because the pool is already
///         initialised.
///
///         ⚠ PACED IN `block.timestamp`, NEVER `block.number`. On chain 4663 —
///         Arbitrum Nitro — Solidity's `block.number` is the PARENT chain's and
///         advances once per ~12s while the chain runs at 101ms. Every duration
///         here is a deadline in timestamps.
contract EloCurve is ReentrancyGuard {

    string public constant NOTICE =
        "a game coin's launch curve, quoted in the reserve - buy or SELL BACK from the first trade, then at a posted "
        "threshold the curve closes and anyone may graduate it into a uniswap v3 position this contract can "
        "never move. The reserve leaves here three ways and no others: to a seller along the curve, into the locked "
        "position, or as posted fees to two addresses welded at deploy. there is no withdraw and no admin";

    uint256 internal constant BPS = 10_000;
    address internal constant DEAD = 0x000000000000000000000000000000000000dEaD;

    /// v3's own sqrt-price bounds. A pool cannot initialise outside them, and
    /// failing here with a readable reason beats failing inside the manager.
    uint160 internal constant MIN_SQRT_RATIO = 4295128739;
    uint160 internal constant MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342;

    // ── the wiring, all welded ───────────────────────────────────────────────
    IERC20 public immutable coin; // the game's coin
    IERC20 public immutable quote; // SCRY. The road in and the road out
    ICurvePositionManager public immutable positionManager;
    ICurveFactory public immutable v3Factory;
    uint24 public immutable poolFee;

    /// The curve's virtual quote reserve — what sets the OPENING price without
    /// anybody having to deposit SCRY. Opening price is `virtualQuote /
    /// curveAlloc`, and the first buyer pays it. This is the number that makes
    /// "the seed comes from buys" possible at all: there is no real SCRY here
    /// until someone buys, and the curve still quotes a price.
    uint256 public immutable virtualQuote;
    /// Coin the curve may sell. It never sells past this.
    uint256 public immutable curveAlloc;
    /// Coin held back from the first block, which becomes the pool's coin side.
    /// Never sellable on the curve — `sell` cannot reach it and `buy` cannot
    /// draw on it.
    uint256 public immutable poolAlloc;
    /// SCRY raised at which the curve closes. Measured on `quoteReserve`, which
    /// is net of fees, so it is the amount that actually becomes liquidity.
    uint256 public immutable threshold;

    /// Trading fee on every buy and every sell, taken in SCRY on both sides so
    /// fee accounting never touches coin accounting.
    uint16 public immutable feeBps;
    /// The protocol's share OF THAT FEE. The remainder is the creator's.
    uint16 public immutable protocolShareBps;
    address public immutable creatorFeeTo;
    address public immutable protocolFeeTo;

    /// Where POOL fees go once graduated — one destination per leg, both fixed
    /// at deploy, exactly as `EloLaunchpad` does it and for the same reason: a
    /// splitter handles ONE token and a v3 position pays TWO.
    address public immutable quoteFeeRecipient;
    address public immutable coinFeeRecipient;

    int24 public immutable tickLower;
    int24 public immutable tickUpper;

    // ── state ────────────────────────────────────────────────────────────────
    /// True once the coin has actually arrived. Permissionless, because it
    /// checks a fact rather than granting anything.
    bool public armed;
    /// True once `threshold` was crossed. Buys stop; SELLS DO NOT — see `sell`.
    bool public closed;

    /// Real SCRY held for the curve. Tracked, never read off `balanceOf`, so a
    /// donation cannot move a price and fees cannot be double-counted.
    uint256 public quoteReserve;
    /// Coin the curve still holds for selling.
    uint256 public coinReserve;
    /// Fees owed to the two fee addresses, held apart from the reserve.
    uint256 public feesAccrued;

    address public pool;
    uint256 public positionId; // 0 until graduated
    uint256 public coinBurnedAtGraduation;

    event Armed(uint256 curveAlloc, uint256 poolAlloc);
    event Bought(address indexed who, uint256 quoteIn, uint256 fee, uint256 coinOut, uint256 quoteReserve);
    event Sold(address indexed who, uint256 coinIn, uint256 fee, uint256 quoteOut, uint256 quoteReserve);
    event Closed(uint256 quoteReserve, uint256 coinLeftOnCurve);
    event Graduated(
        address indexed pool, uint256 indexed positionId, uint128 liquidity, uint256 quoteUsed, uint256 coinUsed, uint256 coinBurned
    );
    event FeesPaid(address indexed creatorTo, address indexed protocolTo, uint256 creatorAmount, uint256 protocolAmount);
    event PoolFeesCollected(address indexed quoteTo, address indexed coinTo, uint256 quoteFees, uint256 coinFees);

    constructor(
        IERC20 _coin,
        IERC20 _quote,
        ICurvePositionManager _positionManager,
        uint24 _poolFee,
        uint256 _virtualQuote,
        uint256 _curveAlloc,
        uint256 _poolAlloc,
        uint256 _threshold,
        uint16 _feeBps,
        uint16 _protocolShareBps,
        address _creatorFeeTo,
        address _protocolFeeTo,
        address _quoteFeeRecipient,
        address _coinFeeRecipient
    ) {
        require(address(_coin) != address(0) && address(_quote) != address(0), "zero token");
        require(address(_coin) != address(_quote), "the coin cannot quote itself");
        require(address(_positionManager) != address(0), "zero v3");
        require(_virtualQuote > 0, "zero virtual quote");
        require(_curveAlloc > 0 && _poolAlloc > 0, "both allocations");
        require(_threshold > 0, "zero threshold");
        // A fee this contract cannot honour is worse than none: the sell side
        // pays out of the reserve, and a fee at or near 100% would let a sell
        // return nothing while still moving the price.
        require(_feeBps <= 1_000, "fee above 10%");
        require(_protocolShareBps <= BPS, "protocol share above 100%");
        require(_creatorFeeTo != address(0) && _protocolFeeTo != address(0), "zero fee address");
        require(_quoteFeeRecipient != address(0) && _coinFeeRecipient != address(0), "zero pool fee recipient");

        coin = _coin;
        quote = _quote;
        positionManager = _positionManager;
        v3Factory = ICurveFactory(_positionManager.factory());
        poolFee = _poolFee;
        virtualQuote = _virtualQuote;
        curveAlloc = _curveAlloc;
        poolAlloc = _poolAlloc;
        threshold = _threshold;
        feeBps = _feeBps;
        protocolShareBps = _protocolShareBps;
        creatorFeeTo = _creatorFeeTo;
        protocolFeeTo = _protocolFeeTo;
        quoteFeeRecipient = _quoteFeeRecipient;
        coinFeeRecipient = _coinFeeRecipient;

        int24 spacing = v3Factory.feeAmountTickSpacing(_poolFee);
        require(spacing > 0, "unknown fee tier");
        // Full range, aligned to the tier's spacing. Full range because a
        // concentrated position is a range somebody has to manage, and an
        // unmanaged one silently stops being liquidity the moment price leaves
        // it — which would make "the pool is deep" false while every number
        // still looked right. Nobody can manage this one: it is locked.
        int24 usable = (887272 / spacing) * spacing;
        tickUpper = usable;
        tickLower = -usable;
    }

    // ── arming ───────────────────────────────────────────────────────────────

    /// The coin arrived; open the curve. PERMISSIONLESS on purpose — it grants
    /// nothing and decides nothing, it only observes that the studio funded
    /// what the constructor already welded. Anyone may cause a funded launch to
    /// start trading, including a buyer who is tired of waiting.
    function arm() external nonReentrant {
        require(!armed, "already armed");
        uint256 required = curveAlloc + poolAlloc;
        require(coin.balanceOf(address(this)) >= required, "the coin side is not funded");
        armed = true;
        coinReserve = curveAlloc;
        emit Armed(curveAlloc, poolAlloc);
    }

    // ── the curve ────────────────────────────────────────────────────────────

    /// Buy the coin with SCRY.
    ///
    /// `x·y=k` over (virtual + real) quote against the curve's coin. The caller
    /// names their own floor, which is the correct place for it here: unlike
    /// `EloLaunchpad.pair`, this call moves the caller's OWN money, so a bound
    /// they choose bounds their own risk rather than pretending to bound
    /// somebody else's.
    function buy(uint256 quoteIn, uint256 minCoinOut, uint256 deadline)
        external
        nonReentrant
        returns (uint256 coinOut)
    {
        require(armed, "not armed");
        require(!closed, "the curve is closed");
        require(block.timestamp <= deadline, "deadline passed");
        require(quoteIn > 0, "nothing in");

        SafeERC20.pullExact(address(quote), msg.sender, address(this), quoteIn, "quote pull failed");

        uint256 fee = (quoteIn * feeBps) / BPS;
        uint256 net = quoteIn - fee;
        require(net > 0, "all fee");

        // coinOut = coinReserve * net / (virtualQuote + quoteReserve + net).
        // Strictly less than coinReserve for any finite net, so the curve can
        // never be drained to zero and the price is always defined.
        coinOut = Math.mulDiv(coinReserve, net, virtualQuote + quoteReserve + net);
        require(coinOut > 0, "dust in");
        require(coinOut >= minCoinOut, "below your floor");

        quoteReserve += net;
        coinReserve -= coinOut;
        feesAccrued += fee;

        SafeERC20.safeTransfer(address(coin), msg.sender, coinOut, "coin pay failed");
        emit Bought(msg.sender, quoteIn, fee, coinOut, quoteReserve);

        if (quoteReserve >= threshold) {
            closed = true;
            emit Closed(quoteReserve, coinReserve);
        }
    }

    /// Sell the coin back for SCRY.
    ///
    /// ⭐ THIS IS THE FUNCTION THE WHOLE REVERSAL IS FOR. It works from the
    /// first trade, and it keeps working after the curve closes — right up
    /// until `graduate()` puts a real pool in its place. There is never a
    /// moment when a holder has paid and cannot get out.
    ///
    /// ⚠ THE ONE GUARD THAT MATTERS: only coin the curve actually SOLD may come
    /// back to it. Without `coinReserve + coinIn <= curveAlloc`, anybody
    /// holding the coin from elsewhere — the studio's own bag, the ticket's
    /// grant reserve, a later airdrop — could push coin in and draw down a
    /// reserve their money never funded. The curve is a market in what it sold,
    /// not a bid for the whole supply.
    function sell(uint256 coinIn, uint256 minQuoteOut, uint256 deadline)
        external
        nonReentrant
        returns (uint256 quoteOut)
    {
        require(armed, "not armed");
        require(positionId == 0, "graduated - trade the pool");
        require(block.timestamp <= deadline, "deadline passed");
        require(coinIn > 0, "nothing in");
        require(coinReserve + coinIn <= curveAlloc, "more coin than the curve sold");

        SafeERC20.pullExact(address(coin), msg.sender, address(this), coinIn, "coin pull failed");

        // gross = (virtualQuote + quoteReserve) * coinIn / (coinReserve + coinIn)
        uint256 gross = Math.mulDiv(virtualQuote + quoteReserve, coinIn, coinReserve + coinIn);
        // The guard above makes this hold arithmetically; it is asserted anyway
        // because the alternative to a revert here is paying out of the virtual
        // reserve, which does not exist.
        require(gross <= quoteReserve, "curve cannot pay that");

        uint256 fee = (gross * feeBps) / BPS;
        quoteOut = gross - fee;
        require(quoteOut > 0, "dust in");
        require(quoteOut >= minQuoteOut, "below your floor");

        quoteReserve -= gross;
        coinReserve += coinIn;
        feesAccrued += fee;

        SafeERC20.safeTransfer(address(quote), msg.sender, quoteOut, "quote pay failed");
        emit Sold(msg.sender, coinIn, fee, quoteOut, quoteReserve);
    }

    // ── graduation ───────────────────────────────────────────────────────────

    /// Close the curve into a v3 position nothing can move.
    ///
    /// PERMISSIONLESS, and it takes no price argument. The opening price is
    /// derived from this contract's own state — `quoteReserve / poolAlloc` —
    /// so there is nothing for a caller to aim, which is what lets this be open
    /// to anybody. (`EloLaunchpad.seed` is steward-gated for the opposite reason:
    /// it opens a pool with no prior state to derive from.)
    ///
    /// ⚠ THE COIN LEFT ON THE CURVE IS BURNED. It is supply that was held for
    /// selling and never sold. Burning it is the honest end state: it cannot be
    /// quietly rerouted to the studio, and holders are not diluted by a bag
    /// nobody paid for. ⚠ A burn is a transfer to a dead address, so the coins
    /// become unspendable but `totalSupply()` does not fall — never write
    /// "supply reduced".
    function graduate(uint256 deadline) external nonReentrant returns (uint256 tokenId, uint128 liquidity) {
        require(closed, "not closed");
        require(positionId == 0, "already graduated");
        require(block.timestamp <= deadline, "deadline passed");

        uint256 quoteForPool = quoteReserve;
        require(quoteForPool > 0, "nothing raised");

        (address token0, address token1) = _sorted();
        uint160 sqrtPriceX96 = _sqrtPriceX96(quoteForPool, poolAlloc, token0);

        pool = positionManager.createAndInitializePoolIfNecessary(token0, token1, poolFee, sqrtPriceX96);
        // A fresh pool has observation cardinality 1 and `observe` over a
        // window reverts on it. Anything downstream that wants a TWAP — the
        // launch vault's `pair` guard, most obviously — needs this to have been
        // done at birth, when the pool is youngest and easiest to push.
        ICurvePool(pool).increaseObservationCardinalityNext(16);

        (uint256 amount0, uint256 amount1) =
            token0 == address(quote) ? (quoteForPool, poolAlloc) : (poolAlloc, quoteForPool);

        SafeERC20.safeApprove(address(quote), address(positionManager), quoteForPool, "approve failed");
        SafeERC20.safeApprove(address(coin), address(positionManager), poolAlloc, "approve failed");

        uint256 used0;
        uint256 used1;
        (tokenId, liquidity, used0, used1) = positionManager.mint(
            ICurvePositionManager.MintParams({
                token0: token0,
                token1: token1,
                fee: poolFee,
                tickLower: tickLower,
                tickUpper: tickUpper,
                amount0Desired: amount0,
                amount1Desired: amount1,
                // The price being minted at is the one derived two lines above,
                // from this contract's own state. There is nothing to slip
                // against, and a min here would only be a self-imposed rounding
                // failure.
                amount0Min: 0,
                amount1Min: 0,
                recipient: address(this),
                deadline: deadline
            })
        );

        SafeERC20.safeApprove(address(quote), address(positionManager), 0, "approve reset failed");
        SafeERC20.safeApprove(address(coin), address(positionManager), 0, "approve reset failed");

        (uint256 quoteUsed, uint256 coinUsed) =
            token0 == address(quote) ? (used0, used1) : (used1, used0);

        positionId = tokenId;
        quoteReserve = quoteForPool - quoteUsed;

        // Everything the pool did not take, plus everything the curve never
        // sold, burns. `coinReserve` is zeroed first: nothing may read it as a
        // live market after this.
        uint256 burned = coinReserve + (poolAlloc - coinUsed);
        coinReserve = 0;
        coinBurnedAtGraduation = burned;
        if (burned > 0) SafeERC20.safeTransfer(address(coin), DEAD, burned, "burn failed");

        emit Graduated(pool, tokenId, liquidity, quoteUsed, coinUsed, burned);
    }

    // ── fees ─────────────────────────────────────────────────────────────────

    /// Push the curve's trading fees to the two addresses fixed at deploy.
    /// Permissionless — the destinations are immutable, so there is nothing for
    /// a caller to aim.
    function payFees() external nonReentrant returns (uint256 creatorAmount, uint256 protocolAmount) {
        uint256 owed = feesAccrued;
        require(owed > 0, "no fees");
        feesAccrued = 0;

        protocolAmount = (owed * protocolShareBps) / BPS;
        creatorAmount = owed - protocolAmount;

        if (creatorAmount > 0) {
            SafeERC20.safeTransfer(address(quote), creatorFeeTo, creatorAmount, "creator fee failed");
        }
        if (protocolAmount > 0) {
            SafeERC20.safeTransfer(address(quote), protocolFeeTo, protocolAmount, "protocol fee failed");
        }
        emit FeesPaid(creatorFeeTo, protocolFeeTo, creatorAmount, protocolAmount);
    }

    /// Collect POOL fees to the recipients fixed at deploy. Same two bounds as
    /// `EloLaunchpad.collectFees` and for the same reason: the legs land here
    /// first, and this address is also holding `quoteReserve`'s remainder and
    /// `feesAccrued`. Only the DELTA across the collect is forwarded, and that
    /// delta is capped by what `collect` said it owed. Delta alone would let a
    /// token that moves value during its own transfer inflate it; the cap alone
    /// would trust a return value over what arrived.
    function collectPoolFees() external nonReentrant returns (uint256 quoteFees, uint256 coinFees) {
        require(positionId != 0, "not graduated");

        uint256 quoteBefore = quote.balanceOf(address(this));
        uint256 coinBefore = coin.balanceOf(address(this));

        (address token0,) = _sorted();
        (uint256 owed0, uint256 owed1) = positionManager.collect(
            ICurvePositionManager.CollectParams({
                tokenId: positionId,
                recipient: address(this),
                amount0Max: type(uint128).max,
                amount1Max: type(uint128).max
            })
        );
        (uint256 owedQuote, uint256 owedCoin) = token0 == address(quote) ? (owed0, owed1) : (owed1, owed0);

        quoteFees = _bounded(quote.balanceOf(address(this)) - quoteBefore, owedQuote);
        coinFees = _bounded(coin.balanceOf(address(this)) - coinBefore, owedCoin);

        if (quoteFees > 0) {
            SafeERC20.safeTransfer(address(quote), quoteFeeRecipient, quoteFees, "quote fee pay failed");
        }
        if (coinFees > 0) SafeERC20.safeTransfer(address(coin), coinFeeRecipient, coinFees, "coin fee pay failed");
        emit PoolFeesCollected(quoteFeeRecipient, coinFeeRecipient, quoteFees, coinFees);
    }

    // ── reads a card and a deploy script need ────────────────────────────────

    /// Quote per whole coin, scaled 1e18, at the curve's current margin.
    function spotPrice() external view returns (uint256) {
        if (coinReserve == 0) return 0;
        return Math.mulDiv(virtualQuote + quoteReserve, 1e18, coinReserve);
    }

    /// What a buy would return, without sending anything.
    function previewBuy(uint256 quoteIn) external view returns (uint256 coinOut, uint256 fee) {
        if (!armed || closed || quoteIn == 0) return (0, 0);
        fee = (quoteIn * feeBps) / BPS;
        uint256 net = quoteIn - fee;
        if (net == 0) return (0, fee);
        coinOut = Math.mulDiv(coinReserve, net, virtualQuote + quoteReserve + net);
    }

    /// What a sell would return, without sending anything. `why` is non-empty
    /// when the sell would revert, so a surface can say which wall it hit
    /// instead of rendering a zero.
    function previewSell(uint256 coinIn) external view returns (uint256 quoteOut, uint256 fee, string memory why) {
        if (!armed) return (0, 0, "not armed");
        if (positionId != 0) return (0, 0, "graduated - trade the pool");
        if (coinIn == 0) return (0, 0, "nothing in");
        if (coinReserve + coinIn > curveAlloc) return (0, 0, "more coin than the curve sold");
        uint256 gross = Math.mulDiv(virtualQuote + quoteReserve, coinIn, coinReserve + coinIn);
        if (gross > quoteReserve) return (0, 0, "curve cannot pay that");
        fee = (gross * feeBps) / BPS;
        quoteOut = gross - fee;
    }

    /// ⭐ THE ONE A DEPLOY SCRIPT MUST READ. The price the curve would close at
    /// against the price the pool would open at, both scaled 1e18, computed
    /// from the constructor's own figures. They are different numbers unless
    /// `poolAlloc` was sized to make them agree, and a large gap is a markdown
    /// the last buyer on the curve eats.
    ///
    /// At close: `coinLeft = k / (virtualQuote + threshold)` where
    /// `k = virtualQuote * curveAlloc`, so the marginal price is
    /// `(virtualQuote + threshold) / coinLeft`. At open: `threshold / poolAlloc`.
    function graduationPreview()
        external
        view
        returns (uint256 curveClosePrice, uint256 poolOpenPrice, uint256 coinLeftOnCurve)
    {
        uint256 k = virtualQuote * curveAlloc;
        uint256 vt = virtualQuote + threshold;
        coinLeftOnCurve = k / vt;
        curveClosePrice = coinLeftOnCurve == 0 ? 0 : Math.mulDiv(vt, 1e18, coinLeftOnCurve);
        poolOpenPrice = Math.mulDiv(threshold, 1e18, poolAlloc);
    }

    /// How much SCRY still has to arrive before the curve closes.
    function remainingToThreshold() external view returns (uint256) {
        return quoteReserve >= threshold ? 0 : threshold - quoteReserve;
    }

    // ── internals ────────────────────────────────────────────────────────────

    function _sorted() internal view returns (address token0, address token1) {
        return address(quote) < address(coin) ? (address(quote), address(coin)) : (address(coin), address(quote));
    }

    function _bounded(uint256 arrived, uint256 owed) internal pure returns (uint256) {
        return arrived < owed ? arrived : owed;
    }

    /// sqrt(amount1 / amount0) · 2^96, for the pool's own token ordering.
    ///
    /// Done at full precision through `mulDiv` rather than by multiplying up:
    /// `amount1 * 2^192` overflows a 256-bit word for any realistic launch
    /// size, and the naive version fails by reverting at graduation, which is
    /// the single worst place in this contract to discover an overflow.
    function _sqrtPriceX96(uint256 quoteAmount, uint256 coinAmount, address token0)
        internal
        view
        returns (uint160)
    {
        (uint256 amount0, uint256 amount1) =
            token0 == address(quote) ? (quoteAmount, coinAmount) : (coinAmount, quoteAmount);
        require(amount0 > 0 && amount1 > 0, "both sides");

        uint256 ratioX192 = Math.mulDiv(amount1, 1 << 192, amount0);
        uint256 root = _sqrt(ratioX192);
        require(root >= MIN_SQRT_RATIO && root <= MAX_SQRT_RATIO, "price outside v3 range");
        return uint160(root);
    }

    /// Babylonian square root, floor. Seeded by bit-length so it converges in a
    /// fixed handful of steps rather than iterating from 1.
    function _sqrt(uint256 x) internal pure returns (uint256 y) {
        if (x == 0) return 0;
        uint256 z = x;
        y = 1;
        if (z >= 0x100000000000000000000000000000000) {
            z >>= 128;
            y <<= 64;
        }
        if (z >= 0x10000000000000000) {
            z >>= 64;
            y <<= 32;
        }
        if (z >= 0x100000000) {
            z >>= 32;
            y <<= 16;
        }
        if (z >= 0x10000) {
            z >>= 16;
            y <<= 8;
        }
        if (z >= 0x100) {
            z >>= 8;
            y <<= 4;
        }
        if (z >= 0x10) {
            z >>= 4;
            y <<= 2;
        }
        if (z >= 0x4) {
            y <<= 1;
        }
        unchecked {
            y = (y + x / y) >> 1;
            y = (y + x / y) >> 1;
            y = (y + x / y) >> 1;
            y = (y + x / y) >> 1;
            y = (y + x / y) >> 1;
            y = (y + x / y) >> 1;
            y = (y + x / y) >> 1;
            uint256 down = x / y;
            return y < down ? y : down;
        }
    }
}
