// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import {INonfungiblePositionManager} from "../src/interfaces/IUniswapV3.sol";

/// Local and minimal on purpose. `src/IERC20.sol` carries no `approve`, and
/// adding one there would oblige every implementer of it — including a test
/// mock — to grow the function too. A script needs two methods; it declares
/// two methods.
interface IERC20Approve {
    function balanceOf(address who) external view returns (uint256);
    function approve(address spender, uint256 amount) external returns (bool);
}

/// Collect a v3 position's trading fees and put them straight back in.
///
///   export PRIVATE_KEY=0x…
///   POSITION_ID=1234 V3_NPM=0x… \
///   forge script script/CompoundPosition.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// ── WHY THIS EXISTS ────────────────────────────────────────────────────
/// Trading fees accrue to a v3 position and stay there until somebody calls
/// `collect`. Nothing in this repo ever did — `POOLS.md` calls it "the
/// quarterly ritual", which is another way of saying a manual act nobody
/// automated. Uncollected fees compound at exactly zero, so a pool that
/// trades well pays nothing extra until this runs.
///
/// The whole return on doing it is compounding: at a 1% tier, a position
/// seeing daily volume worth v of its own depth earns 0.01·v per day, and
/// putting that back makes the depth itself grow. Depth is also what caps
/// how much volume the pool can absorb before slippage sends traders
/// elsewhere, so this is the lever that raises the ceiling rather than
/// just harvesting under it.
///
/// ── WHAT IT DOES NOT DO ────────────────────────────────────────────────
/// It never changes the range and never mints a second position. Both are
/// real levers (`POOLS.md` §2.3 holds a wide band in reserve for the day
/// volume justifies one) and both are decisions rather than a maintenance
/// job, so they do not belong in something safe to run on a schedule.
///
/// ⚠ FEES DO NOT ARRIVE IN THE POOL'S RATIO. They accrue in the ratio of
/// directional volume, and `increaseLiquidity` can only take them at the
/// current price. Whatever it cannot use is refunded and stays in the
/// wallet — that dust is expected, it is not a failure, and the next run
/// picks it up because this script offers the wallet's whole balance of
/// both sides rather than only what it just collected.
contract CompoundPosition is Script {
    struct Inputs {
        uint256 pk;
        address npm;
        uint256 tokenId;
        uint256 slipBps;
        uint256 deadlineSecs;
        bool collectOnly; // sweep to the wallet and stop; do not put it back
    }

    function run() external {
        Inputs memory inp;
        inp.pk = vm.envUint("PRIVATE_KEY");
        inp.npm = vm.envAddress("V3_NPM");
        // No default. A token id that lands on 0 because an env var was unset
        // is not a failure anyone would see — `positions(0)` simply reverts or
        // reads empty, and the run prints a clean nothing-to-do.
        inp.tokenId = vm.envUint("POSITION_ID");
        inp.slipBps = vm.envOr("SLIP_BPS", uint256(100));
        inp.deadlineSecs = vm.envOr("DEADLINE_SECS", uint256(900));
        inp.collectOnly = vm.envOr("COLLECT_ONLY", false);
        runWith(inp);
    }

    function runWith(Inputs memory inp) public returns (uint256 got0, uint256 got1, uint128 added) {
        INonfungiblePositionManager npm = INonfungiblePositionManager(inp.npm);
        address me = vm.addr(inp.pk);

        require(inp.slipBps <= 2000, "slippage guard too loose (>20%)");
        // Fail before broadcasting rather than reverting inside collect: the
        // NPM gates on ownership, and "not yours" is a config mistake worth
        // naming plainly.
        require(npm.ownerOf(inp.tokenId) == me, "signer does not own this position");

        (,, address t0, address t1,,,, uint128 liquidity,,,,) = npm.positions(inp.tokenId);
        require(liquidity > 0, "position holds no liquidity");

        vm.startBroadcast(inp.pk);

        (got0, got1) = npm.collect(
            INonfungiblePositionManager.CollectParams({
                tokenId: inp.tokenId,
                recipient: me,
                amount0Max: type(uint128).max,
                amount1Max: type(uint128).max
            })
        );

        if (!inp.collectOnly) {
            // Offer the WALLET's whole balance of both sides, not just what
            // this call collected. Last run's un-usable remainder is sitting
            // right here, and leaving it out would strand a little more of it
            // every time until the dust was worth more than the run.
            uint256 have0 = IERC20Approve(t0).balanceOf(me);
            uint256 have1 = IERC20Approve(t1).balanceOf(me);
            if (have0 > 0 && have1 > 0) {
                IERC20Approve(t0).approve(inp.npm, have0);
                IERC20Approve(t1).approve(inp.npm, have1);
                uint256 used0;
                uint256 used1;
                (added, used0, used1) = npm.increaseLiquidity(
                    INonfungiblePositionManager.IncreaseLiquidityParams({
                        tokenId: inp.tokenId,
                        amount0Desired: have0,
                        amount1Desired: have1,
                        // The mins bound the RATIO the add settles at, the same
                        // job they do on a mint. Zero here would let a position
                        // be topped up at a price somebody just moved.
                        amount0Min: (have0 * (10_000 - inp.slipBps)) / 10_000,
                        amount1Min: (have1 * (10_000 - inp.slipBps)) / 10_000,
                        deadline: block.timestamp + inp.deadlineSecs
                    })
                );
                // Leave no standing allowance behind a job that runs on a
                // schedule; the next run sets its own.
                IERC20Approve(t0).approve(inp.npm, 0);
                IERC20Approve(t1).approve(inp.npm, 0);
                console2.log("put back    ", used0, used1);
            } else {
                console2.log("one side is zero - nothing to add this run, fees stay in the wallet");
            }
        }

        vm.stopBroadcast();

        console2.log("position    ", inp.tokenId);
        console2.log("collected 0 ", got0);
        console2.log("collected 1 ", got1);
        console2.log("liquidity + ", uint256(added));
    }
}
