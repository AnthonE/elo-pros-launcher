// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryOrchard.sol";
import {INonfungiblePositionManagerOrchard, IUniswapV3FactoryOrchard} from "../src/interfaces/IUniswapV3Orchard.sol";
import {INonfungiblePositionManager, IUniswapV3Factory, IERC20Seed} from "../src/interfaces/IUniswapV3.sol";

/// slot0 + liquidity reads for the live pool. IUniswapV3Orchard.sol carries
/// only the snapshot clock on purpose (the Orchard never reads price); the
/// fork test needs the current tick to prove the minted range brackets it.
interface IUniswapV3PoolSlot0 {
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

    function liquidity() external view returns (uint128);
}

/// Plain ERC-721 owner read so custody-out can be asserted against the real NPM.
interface IERC721OwnerOf {
    function ownerOf(uint256 tokenId) external view returns (address);
}

/// The one write the price-movement test needs: a direct pool swap so a real
/// tick walk across a thin range's bounds can be driven with no SwapRouter
/// dependency (same inline-not-vendored rule as IUniswapV3Orchard.sol). The
/// caller implements uniswapV3SwapCallback to pay what the swap owes.
interface IUniswapV3PoolSwap {
    function swap(
        address recipient,
        bool zeroForOne,
        int256 amountSpecified,
        uint160 sqrtPriceLimitX96,
        bytes calldata data
    ) external returns (int256 amount0, int256 amount1);
}

/// The one ERC-20 write the swap callback needs (IERC20Seed is read+approve
/// only). Kept local to the test - the src interfaces stay untouched.
interface IERC20Transfer {
    function transfer(address to, uint256 amount) external returns (bool);
}

/// The fork harness TEST-AUDIT.md mandates for the Orchard ("the pool clock
/// is a hand-set mock; mispricing/stranding vs the real NPM is invisible to
/// green... a fork test on RH-Chain is the one item no mock replaces").
///
/// What this run proves that the unit suite cannot:
///   - the hand-inlined interface transcriptions (IUniswapV3Orchard.sol's
///     12-tuple `positions()` decode, `snapshotCumulativesInside`,
///     `getPool`, `safeTransferFrom`) line up with the REAL deployed
///     bytecode on RH-Chain 4663 - a mis-ordered tuple field or wrong type
///     would revert or mis-decode here and nowhere else;
///   - a REAL position NFT minted through the real NonfungiblePositionManager
///     round-trips: mint -> safeTransferFrom in (the onERC721Received path)
///     -> stakeToken (real factory.getPool check, real liquidity read) ->
///     warp -> unstakeToken (real pool clock delta) -> claim -> withdraw out;
///   - rewards computed from the real clock stay within [0, pot].
///
/// The second test (test_fork_thin_range_clock_stops_out_of_range) closes what
/// TEST-AUDIT.md flagged as the operator's remaining MANUAL step: price-
/// MOVEMENT-in-range coverage. It stakes a ONE-TICK-SPACING (thin) position,
/// then drives the pool's own tick across that range's bounds with real
/// swaps (direct pool.swap, no SwapRouter needed) and proves the clock the
/// Orchard trusts - snapshotCumulativesInside - FREEZES the instant price
/// leaves the range and RESUMES the instant it re-enters. A full-range
/// position (the first test) is always in range, so its clock runs on warp
/// alone; a thin range is where the whole "out-of-range liquidity earns
/// nothing" claim either holds against real bytecode or does not.
///
/// Run:
///   RH_FORK_URL=https://rpc.mainnet.chain.robinhood.com \
///     forge test --match-contract OrchardForkTest -vv
/// Without RH_FORK_URL the test logs a SKIP and passes - the suite stays
/// green offline and the harness stays compiled.
contract OrchardForkTest is Test {
    // Real RH-Chain (4663) addresses, COPIED from the repo - never guessed:
    //   FACTORY / NPM / SCRY: script/SeedSpoilsUniswapV3.s.sol
    //     DEFAULT_FACTORY / DEFAULT_NPM / DEFAULT_SCRY (POOLS.md §1
    //     verified-live table; re-verify per §4.1 before trusting further).
    //   WETH:  meter/tape.py SCRY_TAPE_WETH default == POOLS.md §1 WETH row.
    //   POOL:  the live SCRY/WETH v3 1% pool - meter/tape.py SCRY_TAPE_POOL
    //     default == POOLS.md ("fee() = 10000, tickSpacing() = 200,
    //     token0 = WETH", created 2026-07-15).
    address constant FACTORY = 0x1f7d7550B1b028f7571E69A784071F0205FD2EfA;
    address constant NPM = 0x73991a25C818Bf1f1128dEAaB1492D45638DE0D3;
    address constant SCRY = 0xDa2a4b23459e9ca88183e990802be644AcA7C4B0;
    address constant WETH = 0x0Bd7D308f8E1639FAb988df18A8011f41EAcAD73;
    address constant POOL = 0xEA8D769eEdEC0eaF19698aEAF4fb3e24f77d7462;
    uint24 constant FEE = 10000; // the pool's 1% tier (POOLS.md: matches the incumbent)
    int24 constant MAX_TICK = 887272; // v3 bound, as in SeedSpoilsUniswapV3.s.sol

    address user = address(0xFA2A); // the staker; NFT custody round-trips here
    uint256 constant POT = 1_000e18; // season pot, fresh MYRRH

    function test_fork_real_npm_full_range_mint_stake_warp_unstake() public {
        string memory url = vm.envOr("RH_FORK_URL", string(""));
        if (bytes(url).length == 0) {
            emit log("SKIP: set RH_FORK_URL to run the Orchard fork test");
            return;
        }
        vm.createSelectFork(url);

        // -- 0. the wiring guards DeployOrchard.s.sol runs, against live code
        address npmFactory = INonfungiblePositionManagerOrchard(NPM).factory();
        assertEq(npmFactory, FACTORY, "NPM/factory mismatch - re-verify POOLS.md 4.1");
        address livePool = IUniswapV3FactoryOrchard(FACTORY).getPool(WETH, SCRY, FEE);
        assertEq(livePool, POOL, "SCRY/WETH 1% pool moved - re-verify POOLS.md 1");
        int24 spacing = IUniswapV3Factory(FACTORY).feeAmountTickSpacing(FEE);
        assertEq(int256(spacing), 200, "1% tier spacing changed"); // POOLS.md pin

        // -- 1. fresh reward coin + Orchard against the REAL NPM/factory.
        // The test contract plays the granary steward's role from the deploy
        // script's season-0 shape: mint the pot -> approve -> createIncentive
        // (the Orchard itself can never mint; a season is funded or absent).
        SpoilsToken myrrh = new SpoilsToken("myrrh", "MYRRH", 0, address(this));
        ScryOrchard orchard =
            new ScryOrchard(myrrh, INonfungiblePositionManagerOrchard(NPM), IUniswapV3FactoryOrchard(FACTORY));
        myrrh.mint(address(this), POT);
        myrrh.approve(address(orchard), POT);
        uint256 startAt = block.timestamp + 1 hours; // DeployOrchard defaults
        uint256 endAt = startAt + 30 days;
        orchard.createIncentive(POOL, POT, startAt, endAt); // incentive 0

        // -- 2. fund the staker with both pool tokens. forge-std `deal`
        // writes the balance slot directly - fine for canonical WETH9 and the
        // plain-OZ SCRY (TEST-AUDIT.md's grounding: PonsLauncherToken is a
        // vanilla ERC-20, restrictions long expired).
        deal(WETH, user, 5e18);
        deal(SCRY, user, 5e26); // 500M SCRY; the pool holds ~508M total

        // -- 3. a REAL full-range position minted through the real NPM,
        // built against the live pool's slot0 tick (full range must bracket
        // it or the mint would be one-sided).
        (uint160 sqrtP, int24 tick,,,,,) = IUniswapV3PoolSlot0(POOL).slot0();
        assertGt(uint256(sqrtP), 0, "pool uninitialized?");
        int24 usable = (MAX_TICK / spacing) * spacing; // 887200 for spacing 200
        assertGt(int256(tick), int256(-usable));
        assertLt(int256(tick), int256(usable));

        INonfungiblePositionManager.MintParams memory mp = INonfungiblePositionManager.MintParams({
            token0: WETH, // 0x0Bd7... < 0xDa2a...: WETH sorts first (POOLS.md: token0 = WETH)
            token1: SCRY,
            fee: FEE,
            tickLower: -usable,
            tickUpper: usable,
            amount0Desired: 5e18,
            amount1Desired: 5e26,
            amount0Min: 0, // a test position, not a launch - no slippage stakes
            amount1Min: 0,
            recipient: user,
            deadline: block.timestamp + 900
        });
        vm.startPrank(user);
        IERC20Seed(WETH).approve(NPM, type(uint256).max);
        IERC20Seed(SCRY).approve(NPM, type(uint256).max);
        (uint256 tokenId, uint128 liq,,) = INonfungiblePositionManager(NPM).mint(mp);
        vm.stopPrank();
        assertGt(uint256(liq), 0, "mint produced no liquidity");

        // -- 4. the decode check that IS the point: the Orchard's inlined
        // 12-field positions() tuple against real bytecode. A transcription
        // slip (order, width, missing field) surfaces exactly here.
        (,, address p0, address p1, uint24 pFee, int24 pLo, int24 pHi, uint128 pLiq,,,,) =
            INonfungiblePositionManagerOrchard(NPM).positions(tokenId);
        assertEq(p0, WETH);
        assertEq(p1, SCRY);
        assertEq(uint256(pFee), uint256(FEE));
        assertEq(int256(pLo), int256(-usable));
        assertEq(int256(pHi), int256(usable));
        assertEq(uint256(pLiq), uint256(liq));

        // -- 5. season opens; custody in by the staker's own safeTransferFrom
        // (the real NPM's ERC-721 hook path), then stake against the real
        // factory.getPool check and the real snapshotCumulativesInside.
        vm.warp(startAt);
        address orchardAddr = address(orchard);
        vm.prank(user);
        INonfungiblePositionManagerOrchard(NPM).safeTransferFrom(user, orchardAddr, tokenId);
        (address depOwner, bool depStaked,,,,,) = orchard.deposits(tokenId);
        assertEq(depOwner, user);
        assertFalse(depStaked);
        vm.prank(user);
        orchard.stakeToken(tokenId, 0);

        // -- 6. a week of the season passes. Full-range liquidity is always
        // in range, and v3's clock extrapolates to block.timestamp at read
        // time, so warp alone advances secondsPerLiquidityInside here. (Real
        // swaps moving price across a THIN range's bounds - the start/stop
        // behavior - are the operator's manual verification, see header.)
        vm.warp(startAt + 7 days);
        vm.prank(user);
        uint256 reward = orchard.unstakeToken(tokenId);

        // -- 7. the money laws: bounded by the pot, credited to the owner,
        // claimable, and the NFT comes home. (No exact-value assert: the
        // real pool's other LPs share the clock's liquidity denominator, so
        // the exact figure floats with live state - the bound is the law.)
        assertLe(reward, POT);
        assertEq(orchard.rewardsOf(user), reward);
        if (reward > 0) {
            vm.prank(user);
            orchard.claimReward(user);
            assertEq(myrrh.balanceOf(user), reward);
        }
        vm.prank(user);
        orchard.withdrawToken(tokenId, user);
        assertEq(IERC721OwnerOf(NPM).ownerOf(tokenId), user); // custody out
    }

    // v3 sqrt-price bounds (from TickMath) - the extreme is never used here;
    // both swaps stop at a bound we set, but the pool requires the limit sit
    // strictly inside [MIN, MAX] and on the correct side of the current price.
    uint160 constant MIN_SQRT_RATIO = 4295128739;
    uint160 constant MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342;

    /// The mandated price-MOVEMENT-in-range run. A thin (one tick-spacing)
    /// position is staked, then the pool's real tick is walked OUT of that
    /// range and back with direct pool swaps. The proof is on the exact clock
    /// the Orchard reads at stake and unstake - snapshotCumulativesInside's
    /// secondsPerLiquidityInsideX128:
    ///   - it grows while the tick sits inside the range,
    ///   - it FREEZES exactly (byte-for-byte across a warp) once a swap pushes
    ///     the tick above tickUpper - the out-of-range interval adds zero,
    ///   - it RESUMES once a swap brings the tick back inside.
    /// That is "out-of-range liquidity earns nothing" proven against live
    /// RH-Chain bytecode, not a hand-set mock clock. The reward the Orchard
    /// credits then reflects only the in-range seconds.
    function test_fork_thin_range_clock_stops_out_of_range() public {
        string memory url = vm.envOr("RH_FORK_URL", string(""));
        if (bytes(url).length == 0) {
            emit log("SKIP: set RH_FORK_URL to run the Orchard thin-range fork test");
            return;
        }
        vm.createSelectFork(url);

        // Fork-reality fix: SCRY (PonsLauncherToken) had a launch max-wallet
        // cap that expired on the LIVE chain at L1 block restrictionEndBlock
        // (25,539,589; TEST-AUDIT.md grounding: "afterward a plain ERC-20").
        // But forge sets the fork's block.number to the RPC's height, which is
        // BELOW that end block, so the long-dead cap springs back to life and
        // reverts the large real-swap payout this test drives (the pool is
        // whitelisted; a plain staker contract is not). Roll block.number past
        // the end block to match live reality. block.timestamp is untouched
        // (the pool clock this test measures runs on vm.warp, not the roll).
        vm.roll(25_539_590);

        int24 spacing = IUniswapV3Factory(FACTORY).feeAmountTickSpacing(FEE);
        assertEq(int256(spacing), 200, "1% tier spacing changed");

        // fresh reward coin + Orchard + a funded season, same shape as above.
        SpoilsToken myrrh = new SpoilsToken("myrrh", "MYRRH", 0, address(this));
        ScryOrchard orchard =
            new ScryOrchard(myrrh, INonfungiblePositionManagerOrchard(NPM), IUniswapV3FactoryOrchard(FACTORY));
        myrrh.mint(address(this), POT);
        myrrh.approve(address(orchard), POT);
        uint256 startAt = block.timestamp + 1 hours;
        uint256 endAt = startAt + 30 days;
        orchard.createIncentive(POOL, POT, startAt, endAt); // incentive 0

        // -- 1. the THIN range: exactly one tick-spacing bracketing the live
        // tick (floor to spacing; correct for a negative tick). tickLower is
        // inclusive, tickUpper exclusive - the live tick sits inside.
        (uint160 sqrtP0, int24 tick0,,,,,) = IUniswapV3PoolSlot0(POOL).slot0();
        assertGt(uint256(sqrtP0), 0, "pool uninitialized?");
        int24 tickLower = (tick0 / spacing) * spacing;
        if (tick0 < 0 && tick0 % spacing != 0) tickLower -= spacing;
        int24 tickUpper = tickLower + spacing;
        assertLe(int256(tickLower), int256(tick0));
        assertLt(int256(tick0), int256(tickUpper));

        // -- 2. mint the thin position through the real NPM. Both tokens are
        // needed (price sits inside the range); deal generously and let the
        // NPM take the ratio it wants.
        deal(WETH, user, 50e18);
        deal(SCRY, user, 5e26);
        INonfungiblePositionManager.MintParams memory mp = INonfungiblePositionManager.MintParams({
            token0: WETH,
            token1: SCRY,
            fee: FEE,
            tickLower: tickLower,
            tickUpper: tickUpper,
            amount0Desired: 50e18,
            amount1Desired: 5e26,
            amount0Min: 0,
            amount1Min: 0,
            recipient: user,
            deadline: block.timestamp + 900
        });
        vm.startPrank(user);
        IERC20Seed(WETH).approve(NPM, type(uint256).max);
        IERC20Seed(SCRY).approve(NPM, type(uint256).max);
        (uint256 tokenId, uint128 liq,,) = INonfungiblePositionManager(NPM).mint(mp);
        vm.stopPrank();
        assertGt(uint256(liq), 0, "thin mint produced no liquidity");

        // -- 3. custody in + stake at season open (tick still inside range).
        vm.warp(startAt);
        vm.prank(user);
        INonfungiblePositionManagerOrchard(NPM).safeTransferFrom(user, address(orchard), tokenId);
        vm.prank(user);
        orchard.stakeToken(tokenId, 0);
        (, uint160 spInit,) = IUniswapV3PoolState(POOL).snapshotCumulativesInside(tickLower, tickUpper);

        // -- 4. IN RANGE: a day passes with the tick inside -> the clock grows.
        vm.warp(startAt + 1 days);
        (, uint160 spInRange1,) = IUniswapV3PoolState(POOL).snapshotCumulativesInside(tickLower, tickUpper);
        assertGt(uint256(spInRange1), uint256(spInit), "clock did not advance in range");

        // -- 5. fund THIS contract to drive real swaps, then walk the tick UP
        // and OUT of the range: sell token1 (SCRY) for token0 (WETH), which
        // raises the price (SCRY-per-WETH) and the tick. Bounded up-limit
        // (+2.5% on sqrtPrice ~= +~490 ticks) clears one 200-tick spacing with
        // margin, and stops the walk somewhere controlled above tickUpper.
        deal(SCRY, address(this), 1e30);
        deal(WETH, address(this), 1e27);
        uint160 limitUp = uint160((uint256(sqrtP0) * 1025) / 1000);
        assertLt(uint256(limitUp), uint256(MAX_SQRT_RATIO), "up-limit out of bounds");
        IUniswapV3PoolSwap(POOL).swap(address(this), false, int256(1e29), limitUp, "");
        (, int24 tickOut,,,,,) = IUniswapV3PoolSlot0(POOL).slot0();
        assertGe(int256(tickOut), int256(tickUpper), "swap failed to leave the range (up)");
        (, uint160 spAtExit,) = IUniswapV3PoolState(POOL).snapshotCumulativesInside(tickLower, tickUpper);

        // -- 6. OUT OF RANGE: three days pass with NO swap. The inside clock is
        // a difference of the two boundary ticks' frozen outside-snapshots when
        // price sits outside - the global term cancels - so it must not move by
        // a single unit. THIS is the load-bearing assertion.
        vm.warp(startAt + 4 days);
        (, uint160 spOutFrozen,) = IUniswapV3PoolState(POOL).snapshotCumulativesInside(tickLower, tickUpper);
        assertEq(uint256(spOutFrozen), uint256(spAtExit), "clock ran while OUT of range");

        // -- 7. walk the tick back DOWN and INTO the range: sell token0 (WETH)
        // for token1 (SCRY). Set the price limit to the EXACT starting price so
        // the swap stops precisely where it began - back inside the range, no
        // TickMath needed. A large exact-input makes the limit bind.
        IUniswapV3PoolSwap(POOL).swap(address(this), true, int256(1e27), sqrtP0, "");
        (uint160 sqrtBack, int24 tickBack,,,,,) = IUniswapV3PoolSlot0(POOL).slot0();
        assertEq(uint256(sqrtBack), uint256(sqrtP0), "did not return to the start price");
        assertLe(int256(tickLower), int256(tickBack));
        assertLt(int256(tickBack), int256(tickUpper)); // back inside
        (, uint160 spReentry,) = IUniswapV3PoolState(POOL).snapshotCumulativesInside(tickLower, tickUpper);

        // -- 8. IN RANGE again: a day passes -> the clock RESUMES.
        vm.warp(startAt + 5 days);
        (, uint160 spInRange2,) = IUniswapV3PoolState(POOL).snapshotCumulativesInside(tickLower, tickUpper);
        assertGt(uint256(spInRange2), uint256(spReentry), "clock did not resume in range");

        // -- 9. the money law: unstake credits a reward built from ONLY the
        // in-range seconds (the frozen out-of-range window added nothing), and
        // it stays bounded by the pot. The whole "razor-thin range earns only
        // while price sits inside it" claim, proven end to end.
        vm.prank(user);
        uint256 reward = orchard.unstakeToken(tokenId);
        assertGt(reward, 0, "in-range time should have earned something");
        assertLe(reward, POT);
        assertEq(orchard.rewardsOf(user), reward);

        // and the NFT still comes home.
        vm.prank(user);
        orchard.withdrawToken(tokenId, user);
        assertEq(IERC721OwnerOf(NPM).ownerOf(tokenId), user);
    }

    /// THE CAMPING MANDATE. Two identical full-range positions, both staked
    /// for a whole season on the live pool; one unstakes at the close, the
    /// other a FULL EXTRA WINDOW late. On real bytecode the pool clock keeps
    /// extrapolating for an in-range position after endTime, so against the
    /// old frozen-window denominator the late exit cashed exactly 2.000x the
    /// prompt one (measured 2026-07-27) - paid out of the remainder the
    /// operator sweeps back, which made camping strictly dominant and let one
    /// sleeping staker hold `endIncentive` hostage profitably. The unit suite
    /// could never see this: its mock clock only moves when a test sets it.
    /// The law, against the denominator that now grows past the close: a late
    /// in-range exit is NEUTRAL - within a whisker of the prompt exit's pay,
    /// and never above it.
    function test_fork_late_unstake_pays_no_more_than_the_close() public {
        string memory url = vm.envOr("RH_FORK_URL", string(""));
        if (bytes(url).length == 0) {
            emit log("SKIP: set RH_FORK_URL to run the Orchard camping fork test");
            return;
        }
        vm.createSelectFork(url);

        int24 spacing = IUniswapV3Factory(FACTORY).feeAmountTickSpacing(FEE);
        int24 usable = (MAX_TICK / spacing) * spacing;

        SpoilsToken myrrh = new SpoilsToken("myrrh", "MYRRH", 0, address(this));
        ScryOrchard orchard =
            new ScryOrchard(myrrh, INonfungiblePositionManagerOrchard(NPM), IUniswapV3FactoryOrchard(FACTORY));
        myrrh.mint(address(this), POT);
        myrrh.approve(address(orchard), POT);
        uint256 startAt = block.timestamp + 1 hours;
        uint256 endAt = startAt + 30 days;
        orchard.createIncentive(POOL, POT, startAt, endAt); // incentive 0

        // two stakers, two positions minted back to back with the same
        // desireds at the same price - as identical as the real NPM makes
        // them. SMALL against the pool's own depth on purpose: the house LP
        // is never staked, which is exactly the shape every real season has.
        address userA = address(0xA11CE);
        address userB = address(0xB0B);
        uint256[2] memory ids;
        address[2] memory users = [userA, userB];
        for (uint256 i = 0; i < 2; i++) {
            deal(WETH, users[i], 1e18);
            deal(SCRY, users[i], 1e25);
            INonfungiblePositionManager.MintParams memory mp = INonfungiblePositionManager.MintParams({
                token0: WETH,
                token1: SCRY,
                fee: FEE,
                tickLower: -usable,
                tickUpper: usable,
                amount0Desired: 1e18,
                amount1Desired: 1e25,
                amount0Min: 0,
                amount1Min: 0,
                recipient: users[i],
                deadline: block.timestamp + 900
            });
            vm.startPrank(users[i]);
            IERC20Seed(WETH).approve(NPM, type(uint256).max);
            IERC20Seed(SCRY).approve(NPM, type(uint256).max);
            (uint256 tokenId,,,) = INonfungiblePositionManager(NPM).mint(mp);
            vm.stopPrank();
            ids[i] = tokenId;
        }

        vm.warp(startAt);
        for (uint256 i = 0; i < 2; i++) {
            vm.prank(users[i]);
            INonfungiblePositionManagerOrchard(NPM).safeTransferFrom(users[i], address(orchard), ids[i]);
            vm.prank(users[i]);
            orchard.stakeToken(ids[i], 0);
        }

        // prompt exit at the close...
        vm.warp(endAt + 1);
        vm.prank(userA);
        uint256 rewardPrompt = orchard.unstakeToken(ids[0]);
        assertGt(rewardPrompt, 0, "a full season in range earned nothing?");

        // ...and the camper, one whole extra window later, still in range
        // (full range always is). The old code paid this exit 2x the prompt one.
        vm.warp(endAt + 30 days);
        vm.prank(userB);
        uint256 rewardLate = orchard.unstakeToken(ids[1]);
        assertGt(rewardLate, 0);
        assertLe(rewardLate, rewardPrompt, "camping past the close must never pay more");
        // and neutral, not merely bounded: within 2% of the prompt exit
        // (the residual drift is the tiny share the staked pair holds of the
        // pool's mostly-unstaked liquidity).
        assertApproxEqRel(rewardLate, rewardPrompt, 2e16, "late in-range exit should be neutral");
    }

    /// Pay what a direct pool.swap owes. Only the pool being swapped calls
    /// this; positive deltas are the amounts this contract must send in.
    function uniswapV3SwapCallback(int256 amount0Delta, int256 amount1Delta, bytes calldata) external {
        require(msg.sender == POOL, "callback not from pool");
        if (amount0Delta > 0) IERC20Transfer(WETH).transfer(msg.sender, uint256(amount0Delta));
        if (amount1Delta > 0) IERC20Transfer(SCRY).transfer(msg.sender, uint256(amount1Delta));
    }
}
