// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Minimal canonical-Uniswap-v3 surface used to seed the spoils pools.
/// @notice Just the periphery/factory functions the SeedSpoilsUniswapV3 script
///         touches — kept deliberately small and inlined so the repo carries no
///         Uniswap dependency and nothing here can drift from the audited
///         on-chain contracts. Every address is verified live at deploy time
///         (POOLS.md §4.1); an interface file is never the authority, the chain
///         is. These signatures match Uniswap v3-periphery
///         (NonfungiblePositionManager) and v3-core (UniswapV3Factory) exactly.

/// The subset of ERC-20 the seed flow needs (approve + reads). Separate from the
/// repo's IERC20 so this file stands alone and carries `approve`/`decimals`,
/// which the market-spine IERC20 intentionally omits.
interface IERC20Seed {
    function approve(address spender, uint256 amount) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
    function balanceOf(address who) external view returns (uint256);
    function decimals() external view returns (uint8);
    function symbol() external view returns (string memory);
}

interface IUniswapV3Factory {
    /// tick spacing enabled for a fee tier (0 = tier not enabled). 1% => 200.
    function feeAmountTickSpacing(uint24 fee) external view returns (int24);
    /// existing pool for (tokenA, tokenB, fee), or address(0) if none.
    function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address pool);
}

/// Just enough of a v3 pool to read the price it ALREADY trades at.
/// Used only to refuse a seed whose numbers were computed against a different
/// price than the live one — never to price anything, and never as an oracle
/// (a spot read is manipulable within a block; this one only ever says "stop").
interface IUniswapV3PoolSeed {
    function slot0()
        external
        view
        returns (
            uint160 sqrtPriceX96,
            int24 tick,
            uint16 observationIndex,
            uint16 observationCardinality,
            uint16 observationCardinalityNext,
            uint8 feeProtocol,
            bool unlocked
        );
}

interface INonfungiblePositionManager {
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

    /// The v3 factory this manager mints against — checked in the script so a
    /// wrong NPM/factory pairing fails before any value moves.
    function factory() external view returns (address);

    /// Deploy + initialize the (token0, token1, fee) pool in one call. Idempotent:
    /// if the pool already exists it just returns it; the passed sqrtPriceX96 is
    /// ignored for an existing, already-initialized pool. token0 < token1 (sorted).
    function createAndInitializePoolIfNecessary(address token0, address token1, uint24 fee, uint160 sqrtPriceX96)
        external
        payable
        returns (address pool);

    /// Mint a new position NFT to `recipient`. Returns the position tokenId and
    /// the liquidity + amounts actually consumed.
    function mint(MintParams calldata params)
        external
        payable
        returns (uint256 tokenId, uint128 liquidity, uint256 amount0, uint256 amount1);

    /// Bundle calls into one transaction — used to land initialize + mint
    /// atomically so no bot can trade the gap (POOLS.md §2.2(b)).
    function multicall(bytes[] calldata data) external payable returns (bytes[] memory results);
}
