// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

interface IWETH9 {
    function deposit() external payable;
    function approve(address spender, uint256 amount) external returns (bool);
    function balanceOf(address who) external view returns (uint256);
}

interface ISwapRouter {
    struct ExactInputSingleParams {
        address tokenIn;
        address tokenOut;
        uint24 fee;
        address recipient;
        uint256 deadline;
        uint256 amountIn;
        uint256 amountOutMinimum;
        uint160 sqrtPriceLimitX96;
    }

    function exactInputSingle(ExactInputSingleParams calldata params) external payable returns (uint256 amountOut);
}

interface ILaunchPositionManager {
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

    struct IncreaseLiquidityParams {
        uint256 tokenId;
        uint256 amount0Desired;
        uint256 amount1Desired;
        uint256 amount0Min;
        uint256 amount1Min;
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
    function increaseLiquidity(IncreaseLiquidityParams calldata params)
        external
        payable
        returns (uint128 liquidity, uint256 amount0, uint256 amount1);
    function collect(CollectParams calldata params) external payable returns (uint256 amount0, uint256 amount1);
}

interface ILaunchFactory {
    function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address pool);
    function feeAmountTickSpacing(uint24 fee) external view returns (int24);
}

interface ILaunchPool {
    function slot0()
        external
        view
        returns (
            uint160 sqrtPriceX96,
            int24 tick,
            uint16 observationIndex,
            uint16 observationCardinality,
            uint16 observationCardinalityNext,
            bool unlocked
        );
    function observe(uint32[] calldata secondsAgos)
        external
        view
        returns (int56[] memory tickCumulatives, uint160[] memory secondsPerLiquidityCumulativeX128s);
    function increaseObservationCardinalityNext(uint16 observationCardinalityNext) external;
}

/// @title ScryLaunch — the copy sale's money, with only one door and it leads to the pool
/// @notice `COPIES.md`'s launch, as the contract that was missing. The ticket
///         takes money and mints a copy; it knows nothing about pools, and
///         should not. This is the layer above it — named as the ticket's
///         `proceeds` at deploy, because that address is IMMUTABLE and so the
///         decision about who holds a raise is made once and never again.
///
///         ⭐ THE ONE PROPERTY, AND EVERYTHING ELSE SERVES IT: **value that
///         enters this contract can only ever leave as liquidity.** There is no
///         withdraw, no rescue, no sweep-to-owner, no `decreaseLiquidity`, and
///         no path that moves the position NFT. Not "we promise not to" —
///         there is no function. That is the honest answer to the only
///         question this audience actually asks (*"can the team take my
///         money"*, `DEGEN.md` §1b), and it is the reason the design does not
///         need an escrow: an escrow holds a refund, and the refund is the coin.
///
///         ⚠ THIS IS NOT THE DELETED ESCROW WEARING A NEW NAME. What died on
///         2026-08-09 was a contract holding the buyer's money against a
///         promise to give it back. This holds it against a promise to POOL it,
///         and it cannot do anything else, which is a different object. If a
///         future edit ever adds a way to take value out other than pool fees,
///         it has become the escrow and the whole design is void.
///
///         CONTINUOUS, NOT THRESHOLDED (2026-08-10). The launchpads that this
///         is measured against do one of two things and never a third: pons V1
///         deploys the pool in the launch transaction so a token trades from
///         block zero, and curve launchpads make the pre-graduation phase
///         itself a two-way venue. **Nobody runs a phase where a buyer has paid
///         and cannot sell.** `COPIES.md` §3's warning is exactly that — *"just
///         sell your coins needs somewhere to sell them"* — so there is no
///         threshold here, no graduation event and no waiting: the pool opens
///         at seed and every later sale deepens it.
///
///         WHO MAY DO WHAT, and the split is about MEV rather than trust:
///           · `pair()` is PERMISSIONLESS. Anyone may push the vault's holdings
///             into the pool, because the thing being protected is a deposit
///             ratio and the guard for that is on chain (see the TWAP check).
///           · `convert()` is STEWARD-gated, because a swap needs a slippage
///             bound somebody chose, and a permissionless swap with a
///             caller-supplied `minOut` is a permissionless sandwich.
///           · `collectFees()` is PERMISSIONLESS and sends POOL FEES ONLY, one
///             leg to each of two destinations fixed at deploy. Fees are not
///             liquidity; taking them removes nothing.
///
///         WHY TWO FEE DESTINATIONS AND NOT ONE (operator, 2026-08-10: *"the
///         game economy should handle this… they can setup fee stuff just like
///         we can"*). A game's coin coming back as fee income is not a problem
///         to be swapped away — a studio is supposed to be long its own coin,
///         and buying it at launch is the expected behaviour, so treating any
///         coin inventory as dump risk imports a memecoin-creator assumption
///         onto somebody who is meant to be aligned. What the coin leg wants is
///         an ECONOMY: burn part, bank part, pay part. The house already runs
///         both instruments and both take their token as a constructor
///         argument, so a game deploys its own — `ScryFeeSplitter` (posted
///         basis points, `BURN` is an ordinary recipient) and `ScryBank` (stake
///         the coin, fee income swells the pool). A splitter handles ONE token
///         and a position pays TWO, which is the only reason this needed a
///         change at all.
///
///         So "liquidity grows as copies sell" is operational — it depends on
///         someone calling `pair`, and anyone can. "The raise cannot be taken"
///         is structural and depends on nobody.
///
///         ⚠ THE COIN SIDE IS FUNDED, NEVER MINTED. Like the ticket's grant
///         reserve: somebody transfers the game's coin here. This contract has
///         no authority over anybody's token, because a launch vault that could
///         mint would make every listing a trust question.
///
///         ⚠ PACED IN `block.timestamp`, NEVER `block.number`. On chain 4663 —
///         Arbitrum Nitro — Solidity's `block.number` is the PARENT chain's and
///         advances once per ~12s while the chain runs at 101ms. `ScryGacha`
///         shipped a "~10 second" window that was really ~20 minutes and
///         nothing failed. Every duration here is a timestamp.
contract ScryLaunch is ReentrancyGuard {

    string public constant NOTICE = "the copy sale's raise, paired into the game's pool and never withdrawable - "
        "no threshold, no graduation event, no exit: this contract has no function that removes liquidity, and "
        "pool fees are the only value that ever leaves";

    // ── the wiring, all welded ───────────────────────────────────────────────
    IERC20 public immutable scry; // the reserve, and the only pair asset
    IERC20 public immutable coin; // the game's coin
    IWETH9 public immutable weth;
    ILaunchPositionManager public immutable positionManager;
    ILaunchFactory public immutable v3Factory;
    ISwapRouter public immutable router;
    uint24 public immutable poolFee; // the pool's fee tier
    uint24 public immutable convertFee; // the fee tier `convert` swaps through
    /// Where POOL FEES go — ONE DESTINATION PER LEG, both fixed at deploy.
    ///
    /// It was a single address, and that was wrong for the shape a game economy
    /// actually wants (operator, 2026-08-10: *"the game economy should handle
    /// this… they can setup fee stuff just like we can"*). The house already
    /// runs both instruments and both are token-generic: `ScryFeeSplitter`
    /// routes one token by posted basis points with `BURN` available as an
    /// ordinary recipient, and `ScryBank` stakes one token so fee income swells
    /// a staking pool. A game deploys its own of each. But a splitter handles
    /// ONE token, and a v3 position pays TWO — so one recipient forced both
    /// legs through the same instrument and neither could be a splitter.
    ///
    /// Two immutable destinations is also strictly MORE constrained than one
    /// arbitrary address in front of a router, which is the other way to do it.
    address public immutable scryFeeRecipient;
    address public immutable coinFeeRecipient;

    /// May `convert` and may seed. Never may withdraw — there is nothing to
    /// withdraw with. Two-step handover, as everywhere here.
    address public steward;
    address public pendingSteward;

    address public pool;
    uint256 public positionId; // 0 until seeded
    int24 public tickLower;
    int24 public tickUpper;

    /// How far spot may sit from the TWAP before `pair` refuses, in bps.
    uint16 public constant MAX_TWAP_DEVIATION_BPS = 500; // 5%
    /// The window the TWAP is taken over. A timestamp duration, not blocks.
    uint32 public constant TWAP_WINDOW = 900; // 15 minutes

    uint256 public totalScryPaired;
    uint256 public totalCoinPaired;

    event Seeded(address indexed pool, uint256 indexed positionId, uint128 liquidity, uint256 scryUsed, uint256 coinUsed);
    event Paired(uint256 indexed positionId, uint128 liquidity, uint256 scryUsed, uint256 coinUsed);
    event Converted(address indexed tokenIn, uint256 amountIn, uint256 scryOut);
    event Wrapped(uint256 amount);
    event FeesCollected(address indexed scryTo, address indexed coinTo, uint256 scryFees, uint256 coinFees);
    event StewardOffered(address indexed from, address indexed to);
    event StewardTransferred(address indexed from, address indexed to);

    modifier onlySteward() {
        require(msg.sender == steward, "not steward");
        _;
    }

    constructor(
        IERC20 _scry,
        IERC20 _coin,
        IWETH9 _weth,
        ILaunchPositionManager _positionManager,
        ISwapRouter _router,
        uint24 _poolFee,
        uint24 _convertFee,
        address _scryFeeRecipient,
        address _coinFeeRecipient,
        address _steward
    ) {
        require(address(_scry) != address(0) && address(_coin) != address(0), "zero token");
        require(address(_scry) != address(_coin), "scry cannot pair with itself");
        require(address(_weth) != address(0), "zero weth");
        require(address(_positionManager) != address(0) && address(_router) != address(0), "zero v3");
        require(_scryFeeRecipient != address(0) && _coinFeeRecipient != address(0), "zero fee recipient");
        require(_steward != address(0), "zero steward");

        scry = _scry;
        coin = _coin;
        weth = _weth;
        positionManager = _positionManager;
        router = _router;
        v3Factory = ILaunchFactory(_positionManager.factory());
        poolFee = _poolFee;
        convertFee = _convertFee;
        scryFeeRecipient = _scryFeeRecipient;
        coinFeeRecipient = _coinFeeRecipient;
        steward = _steward;

        int24 spacing = v3Factory.feeAmountTickSpacing(_poolFee);
        require(spacing > 0, "unknown fee tier");
        // Full range, aligned to the tier's spacing. Full range because a
        // concentrated launch position is a range somebody has to manage, and
        // an unmanaged one silently stops being liquidity the moment price
        // leaves it — which would make "the pool is deep" false while every
        // number still looked right.
        tickUpper = (887272 / spacing) * spacing;
        tickLower = -tickUpper;
    }

    /// The ticket's `sweep()` sends ETH here, so this must accept it.
    receive() external payable {}

    // ── the raise arrives, and is made poolable ──────────────────────────────

    /// Wrap held ETH. Permissionless: wrapping changes nothing about who owns
    /// what, and it is a step somebody has to be able to take without asking.
    function wrap() external nonReentrant returns (uint256 amount) {
        amount = address(this).balance;
        require(amount > 0, "no eth");
        weth.deposit{value: amount}();
        emit Wrapped(amount);
    }

    /// Swap a non-SCRY rail into SCRY so it can be paired.
    ///
    /// STEWARD-GATED, and the reason is MEV rather than trust: a swap needs a
    /// slippage bound that somebody chose, and a permissionless swap taking a
    /// caller-supplied `minOut` is just a sandwich with extra steps. The
    /// steward cannot steal by calling this — the output lands here, and here
    /// has no exit.
    function convert(address tokenIn, uint256 amountIn, uint256 minScryOut, uint256 deadline)
        external
        onlySteward
        nonReentrant
        returns (uint256 scryOut)
    {
        require(tokenIn != address(scry), "already scry");
        require(tokenIn != address(coin), "the coin side is not the raise");
        require(amountIn > 0, "nothing in");
        require(minScryOut > 0, "name a floor");

        SafeERC20.safeApprove(tokenIn, address(router), amountIn, "approve failed");
        scryOut = router.exactInputSingle(
            ISwapRouter.ExactInputSingleParams({
                tokenIn: tokenIn,
                tokenOut: address(scry),
                fee: convertFee,
                recipient: address(this),
                deadline: deadline,
                amountIn: amountIn,
                amountOutMinimum: minScryOut,
                sqrtPriceLimitX96: 0
            })
        );
        SafeERC20.safeApprove(tokenIn, address(router), 0, "approve reset failed");
        emit Converted(tokenIn, amountIn, scryOut);
    }

    // ── the pool ─────────────────────────────────────────────────────────────

    /// Open the pool and mint the position. Steward-gated because the OPENING
    /// PRICE is a posted decision — there is no market to read yet, so nothing
    /// on chain can check it, and a permissionless seed would let a stranger
    /// pick a title's launch price.
    ///
    /// `sqrtPriceX96` is computed off chain and must be expressed for the pool's
    /// own token ordering (token0 < token1 by address). It is passed rather than
    /// derived because a square root on chain buys nothing here.
    function seed(uint160 sqrtPriceX96, uint256 scryAmount, uint256 coinAmount, uint256 deadline)
        external
        onlySteward
        nonReentrant
        returns (uint256 tokenId, uint128 liquidity)
    {
        require(positionId == 0, "already seeded");
        require(scryAmount > 0 && coinAmount > 0, "both sides");
        require(scry.balanceOf(address(this)) >= scryAmount, "not enough scry held");
        require(coin.balanceOf(address(this)) >= coinAmount, "the coin side is not funded");

        (address token0, address token1) = _sorted();
        pool = positionManager.createAndInitializePoolIfNecessary(token0, token1, poolFee, sqrtPriceX96);
        // Make the TWAP `pair()` depends on actually available. A fresh pool
        // has cardinality 1, and `observe` over a window reverts on that — so
        // the guard would be dead exactly when the pool is youngest and most
        // manipulable.
        ILaunchPool(pool).increaseObservationCardinalityNext(16);

        (uint256 amount0, uint256 amount1) =
            token0 == address(scry) ? (scryAmount, coinAmount) : (coinAmount, scryAmount);

        SafeERC20.safeApprove(address(scry), address(positionManager), scryAmount, "approve failed");
        SafeERC20.safeApprove(address(coin), address(positionManager), coinAmount, "approve failed");

        uint256 used0;
        uint256 used1;
        (tokenId, liquidity, used0, used1) = positionManager.mint(
            ILaunchPositionManager.MintParams({
                token0: token0,
                token1: token1,
                fee: poolFee,
                tickLower: tickLower,
                tickUpper: tickUpper,
                amount0Desired: amount0,
                amount1Desired: amount1,
                // The seed sets the price it is minting at, so there is nothing
                // to slip against; a min here would only be a self-imposed
                // rounding failure.
                amount0Min: 0,
                amount1Min: 0,
                recipient: address(this),
                deadline: deadline
            })
        );

        SafeERC20.safeApprove(address(scry), address(positionManager), 0, "approve reset failed");
        SafeERC20.safeApprove(address(coin), address(positionManager), 0, "approve reset failed");

        positionId = tokenId;
        (uint256 scryUsed, uint256 coinUsed) =
            token0 == address(scry) ? (used0, used1) : (used1, used0);
        totalScryPaired += scryUsed;
        totalCoinPaired += coinUsed;
        emit Seeded(pool, tokenId, liquidity, scryUsed, coinUsed);
    }

    /// Push everything the vault currently holds into the position.
    ///
    /// PERMISSIONLESS on purpose. This is what makes "the pool deepens as copies
    /// sell" a thing anybody can cause rather than a thing the studio has to be
    /// trusted to do. What a caller could otherwise abuse — depositing at a
    /// manipulated ratio — is guarded on chain below rather than by a number the
    /// caller supplies.
    function pair(uint256 deadline) external nonReentrant returns (uint128 liquidity) {
        require(positionId != 0, "not seeded");
        _requireNearTwap();

        uint256 scryAmount = scry.balanceOf(address(this));
        uint256 coinAmount = coin.balanceOf(address(this));
        require(scryAmount > 0 && coinAmount > 0, "nothing to pair");

        (address token0,) = _sorted();
        (uint256 amount0, uint256 amount1) =
            token0 == address(scry) ? (scryAmount, coinAmount) : (coinAmount, scryAmount);

        SafeERC20.safeApprove(address(scry), address(positionManager), scryAmount, "approve failed");
        SafeERC20.safeApprove(address(coin), address(positionManager), coinAmount, "approve failed");

        uint256 used0;
        uint256 used1;
        (liquidity, used0, used1) = positionManager.increaseLiquidity(
            ILaunchPositionManager.IncreaseLiquidityParams({
                tokenId: positionId,
                amount0Desired: amount0,
                amount1Desired: amount1,
                // The TWAP guard above is the slippage bound. Mins here would
                // have to come from the caller, and a caller-chosen min on a
                // permissionless function bounds nothing.
                amount0Min: 0,
                amount1Min: 0,
                deadline: deadline
            })
        );

        SafeERC20.safeApprove(address(scry), address(positionManager), 0, "approve reset failed");
        SafeERC20.safeApprove(address(coin), address(positionManager), 0, "approve reset failed");

        (uint256 scryUsed, uint256 coinUsed) =
            token0 == address(scry) ? (used0, used1) : (used1, used0);
        totalScryPaired += scryUsed;
        totalCoinPaired += coinUsed;
        emit Paired(positionId, liquidity, scryUsed, coinUsed);
    }

    /// Refuse to deposit while spot is far from the TWAP. A full-range add at a
    /// manipulated price hands the manipulator the difference, and this is the
    /// only defence available to a function that takes no slippage argument.
    function _requireNearTwap() internal view {
        (, int24 spot,,,,) = ILaunchPool(pool).slot0();
        uint32[] memory ago = new uint32[](2);
        ago[0] = TWAP_WINDOW;
        ago[1] = 0;
        (int56[] memory cumulatives,) = ILaunchPool(pool).observe(ago);
        int24 mean = int24((cumulatives[1] - cumulatives[0]) / int56(uint56(TWAP_WINDOW)));

        int24 diff = spot > mean ? spot - mean : mean - spot;
        // A tick is 1.0001x, so bps of price ≈ ticks. Comparing in ticks keeps
        // this exact-integer and avoids a pow() on chain; it is conservative,
        // which is the direction a guard should err in.
        require(uint256(uint24(diff)) <= MAX_TWAP_DEVIATION_BPS, "spot is far from the twap");
    }

    /// Collect POOL FEES to the recipient fixed at deploy.
    ///
    /// This is the only value that ever leaves, and it is not liquidity: v3
    /// `collect` returns what is owed, and nothing here ever calls
    /// `decreaseLiquidity`, so what is owed can only be fees. Permissionless —
    /// the destination is immutable, so there is nothing for a caller to aim.
    /// ⚠ THE FEES LAND HERE FIRST, AND THAT IS THE ONE PLACE THIS CONTRACT
    ///    COULD LEAK. The position pays both legs to a single recipient, so
    ///    splitting them means collecting to this address and forwarding — and
    ///    this address is also holding the raise. Two bounds, both required:
    ///
    ///      1. only the DELTA across the collect is forwarded, never a balance;
    ///      2. that delta is capped by what `collect` said it owed.
    ///
    ///    (1) alone would let a token that moves value during its own transfer
    ///    inflate the delta and walk raise out as "fees". (2) alone would trust
    ///    a return value over what actually arrived. Together the amount
    ///    forwarded is at most what the position paid, so a raise sitting here
    ///    is untouchable by this function no matter how the tokens behave.
    function collectFees() external nonReentrant returns (uint256 scryFees, uint256 coinFees) {
        require(positionId != 0, "not seeded");

        uint256 scryBefore = scry.balanceOf(address(this));
        uint256 coinBefore = coin.balanceOf(address(this));

        (address token0,) = _sorted();
        (uint256 owed0, uint256 owed1) = positionManager.collect(
            ILaunchPositionManager.CollectParams({
                tokenId: positionId,
                recipient: address(this),
                amount0Max: type(uint128).max,
                amount1Max: type(uint128).max
            })
        );
        (uint256 owedScry, uint256 owedCoin) = token0 == address(scry) ? (owed0, owed1) : (owed1, owed0);

        scryFees = _bounded(scry.balanceOf(address(this)) - scryBefore, owedScry);
        coinFees = _bounded(coin.balanceOf(address(this)) - coinBefore, owedCoin);

        if (scryFees > 0) SafeERC20.safeTransfer(address(scry), scryFeeRecipient, scryFees, "scry fee pay failed");
        if (coinFees > 0) SafeERC20.safeTransfer(address(coin), coinFeeRecipient, coinFees, "coin fee pay failed");
        emit FeesCollected(scryFeeRecipient, coinFeeRecipient, scryFees, coinFees);
    }

    function _bounded(uint256 arrived, uint256 owed) internal pure returns (uint256) {
        return arrived < owed ? arrived : owed;
    }

    // ── steward handover ─────────────────────────────────────────────────────

    function transferSteward(address to) external onlySteward {
        if (to == address(0)) {
            // Renouncing only removes power: nobody can seed or convert after
            // this, and `pair` and `collectFees` still work forever because
            // neither needed a steward.
            emit StewardTransferred(steward, address(0));
            steward = address(0);
            pendingSteward = address(0);
            return;
        }
        pendingSteward = to;
        emit StewardOffered(steward, to);
    }

    function acceptSteward() external {
        require(msg.sender == pendingSteward, "not the pending steward");
        emit StewardTransferred(steward, msg.sender);
        steward = msg.sender;
        pendingSteward = address(0);
    }

    // ── reading ──────────────────────────────────────────────────────────────

    function _sorted() internal view returns (address token0, address token1) {
        (token0, token1) =
            address(scry) < address(coin) ? (address(scry), address(coin)) : (address(coin), address(scry));
    }

    /// What is sitting here waiting to be paired. A card should read this
    /// rather than claim a number.
    function pending() external view returns (uint256 ethHeld, uint256 scryHeld, uint256 coinHeld) {
        return (address(this).balance, scry.balanceOf(address(this)), coin.balanceOf(address(this)));
    }

    function seeded() external view returns (bool) {
        return positionId != 0;
    }
}
