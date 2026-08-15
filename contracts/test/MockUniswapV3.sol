// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../src/ScryLaunchpad.sol";
import "./MockToken.sol";

/// Stand-ins for the canonical v3 pieces, for `ScryLaunchpad.t.sol`.
///
/// ⚠ WHAT THESE CAN AND CANNOT PROVE. They pin `ScryLaunchpad`'s OWN logic — who
/// may call what, that the TWAP guard bites, that fees go to an address nobody
/// can aim, that no exit exists. They prove NOTHING about real Uniswap
/// behaviour: tick math, the ratio a mint actually consumes, router pathing.
/// That needs a fork test against the canonical addresses in
/// `deployments.json`, and this file is not a substitute for one.

contract MockWETH is MockToken {
    constructor() MockToken("Wrapped Ether", "WETH") {}

    function deposit() external payable {
        mint(msg.sender, msg.value);
    }
}

contract MockV3Factory {
    mapping(uint24 => int24) public feeAmountTickSpacing;
    mapping(bytes32 => address) internal _pools;

    constructor() {
        feeAmountTickSpacing[500] = 10;
        feeAmountTickSpacing[3000] = 60;
        feeAmountTickSpacing[10000] = 200;
    }

    function setPool(address a, address b, uint24 fee, address pool) external {
        _pools[keccak256(abi.encode(a, b, fee))] = pool;
        _pools[keccak256(abi.encode(b, a, fee))] = pool;
    }

    function getPool(address a, address b, uint24 fee) external view returns (address) {
        return _pools[keccak256(abi.encode(a, b, fee))];
    }
}

/// A pool whose spot and TWAP are settable, because the only thing `ScryLaunchpad`
/// asks a pool is "how far is spot from the mean".
contract MockV3Pool {
    int24 public spotTick;
    int24 public meanTick;
    uint16 public cardinalityNext;
    bool public observeReverts;

    function setTicks(int24 spot, int24 mean) external {
        spotTick = spot;
        meanTick = mean;
    }

    function setObserveReverts(bool v) external {
        observeReverts = v;
    }

    function slot0() external view returns (uint160, int24, uint16, uint16, uint16, bool) {
        return (uint160(1) << 96, spotTick, 0, 16, cardinalityNext, true);
    }

    function observe(uint32[] calldata secondsAgos)
        external
        view
        returns (int56[] memory tickCumulatives, uint160[] memory secondsPer)
    {
        require(!observeReverts, "OLD");
        tickCumulatives = new int56[](2);
        secondsPer = new uint160[](2);
        // cumulative[1] - cumulative[0] == meanTick * window
        tickCumulatives[0] = 0;
        tickCumulatives[1] = int56(meanTick) * int56(uint56(secondsAgos[0]));
    }

    function increaseObservationCardinalityNext(uint16 n) external {
        cardinalityNext = n;
    }
}

/// Records what it was asked to do and consumes tokens, so a test can assert
/// that value actually left the vault into a position rather than a wallet.
contract MockPositionManager {
    address public immutable factory;
    MockV3Pool public pool;

    uint256 public nextId = 1;
    uint256 public lastTokenId;
    uint256 public totalAmount0;
    uint256 public totalAmount1;
    uint256 public increases;
    uint256 public collects;
    /// What `collect` will hand back — the fees a real pool would owe.
    uint256 public feesOwed0;
    uint256 public feesOwed1;
    address public lastCollectRecipient;
    /// Set to make a mint consume less than desired, the way a real ratio does.
    uint256 public consumeBps = 10_000;

    constructor(address _factory, MockV3Pool _pool) {
        factory = _factory;
        pool = _pool;
    }

    function setFeesOwed(uint256 a0, uint256 a1) external {
        feesOwed0 = a0;
        feesOwed1 = a1;
    }

    function setConsumeBps(uint256 bps) external {
        consumeBps = bps;
    }

    function createAndInitializePoolIfNecessary(address, address, uint24, uint160)
        external
        payable
        returns (address)
    {
        return address(pool);
    }

    function _pull(address token0, address token1, uint256 a0, uint256 a1)
        internal
        returns (uint256 used0, uint256 used1)
    {
        used0 = a0 * consumeBps / 10_000;
        used1 = a1 * consumeBps / 10_000;
        if (used0 > 0) MockToken(token0).transferFrom(msg.sender, address(this), used0);
        if (used1 > 0) MockToken(token1).transferFrom(msg.sender, address(this), used1);
        totalAmount0 += used0;
        totalAmount1 += used1;
    }

    function mint(ILaunchPositionManager.MintParams calldata p)
        external
        payable
        returns (uint256 tokenId, uint128 liquidity, uint256 amount0, uint256 amount1)
    {
        (amount0, amount1) = _pull(p.token0, p.token1, p.amount0Desired, p.amount1Desired);
        tokenId = nextId++;
        lastTokenId = tokenId;
        liquidity = uint128(amount0 + amount1);
    }

    function increaseLiquidity(ILaunchPositionManager.IncreaseLiquidityParams calldata p)
        external
        payable
        returns (uint128 liquidity, uint256 amount0, uint256 amount1)
    {
        require(p.tokenId == lastTokenId, "unknown position");
        increases++;
        (amount0, amount1) = _pull(token0Of(), token1Of(), p.amount0Desired, p.amount1Desired);
        liquidity = uint128(amount0 + amount1);
    }

    address public t0;
    address public t1;

    function setTokens(address a, address b) external {
        t0 = a;
        t1 = b;
    }

    function token0Of() public view returns (address) {
        return t0;
    }

    function token1Of() public view returns (address) {
        return t1;
    }

    function collect(ILaunchPositionManager.CollectParams calldata p)
        external
        payable
        returns (uint256 amount0, uint256 amount1)
    {
        lastCollectRecipient = p.recipient;
        amount0 = feesOwed0;
        amount1 = feesOwed1;
        feesOwed0 = 0;
        feesOwed1 = 0;
        if (amount0 > 0) MockToken(t0).mint(p.recipient, amount0);
        if (amount1 > 0) MockToken(t1).mint(p.recipient, amount1);
        collects++;
    }
}

/// Swaps at a settable rate so `convert`'s floor can be tested from both sides.
contract MockSwapRouter {
    uint256 public rateBps = 10_000; // out = in * rateBps / 10_000

    function setRate(uint256 bps) external {
        rateBps = bps;
    }

    function exactInputSingle(ISwapRouter.ExactInputSingleParams calldata p)
        external
        payable
        returns (uint256 amountOut)
    {
        require(p.deadline >= block.timestamp, "expired");
        MockToken(p.tokenIn).transferFrom(msg.sender, address(this), p.amountIn);
        amountOut = p.amountIn * rateBps / 10_000;
        require(amountOut >= p.amountOutMinimum, "Too little received");
        MockToken(p.tokenOut).mint(p.recipient, amountOut);
    }
}
