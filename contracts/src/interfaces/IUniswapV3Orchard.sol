// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Minimal canonical-Uniswap-v3 surface the Orchard staker touches.
/// @notice Same rule as IUniswapV3.sol (the seed-flow file, kept separate on
///         purpose): deliberately small, inlined, no vendored dependency, and
///         never the authority - the chain is. These signatures match
///         v3-core (UniswapV3Pool) and v3-periphery
///         (NonfungiblePositionManager) exactly.

/// The one pool read the whole farm stands on: the cumulative
/// seconds-per-liquidity clock inside a tick range. It only advances while
/// the pool's current tick is inside [tickLower, tickUpper), which is what
/// makes range-aware farming honest: out-of-range liquidity earns nothing.
interface IUniswapV3PoolState {
    function snapshotCumulativesInside(int24 tickLower, int24 tickUpper)
        external
        view
        returns (int56 tickCumulativeInside, uint160 secondsPerLiquidityInsideX128, uint32 secondsInside);
}

/// The position-manager subset: read a position's identity + liquidity, and
/// move the NFT. The Orchard never mints, burns, or edits liquidity through
/// the manager - custody in, custody out, nothing else.
interface INonfungiblePositionManagerOrchard {
    function positions(uint256 tokenId)
        external
        view
        returns (
            uint96 nonce,
            address operator,
            address token0,
            address token1,
            uint24 fee,
            int24 tickLower,
            int24 tickUpper,
            uint128 liquidity,
            uint256 feeGrowthInside0LastX128,
            uint256 feeGrowthInside1LastX128,
            uint128 tokensOwed0,
            uint128 tokensOwed1
        );

    function safeTransferFrom(address from, address to, uint256 tokenId) external;
    /// Used only by rescueOrphan, to prove this contract really holds the NFT
    /// before trying to return it.
    function ownerOf(uint256 tokenId) external view returns (address);

    function factory() external view returns (address);
}

/// Factory lookup used to verify a staked position actually belongs to the
/// incentive's pool (token0/token1/fee -> pool, or zero).
interface IUniswapV3FactoryOrchard {
    function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address pool);
}
