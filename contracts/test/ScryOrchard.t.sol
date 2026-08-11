// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryOrchard.sol";
import {INonfungiblePositionManagerOrchard, IUniswapV3FactoryOrchard} from "../src/interfaces/IUniswapV3Orchard.sol";

// -- mocks: just enough canonical-v3 surface to drive the Orchard ----------
// The real referee is the pool's snapshotCumulativesInside clock; the mock
// lets the test SET that clock per range, standing in for "price sat inside
// this range for N seconds against the range's active liquidity".

interface IERC721ReceiverMock {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

contract MockV3Pool {
    mapping(bytes32 => uint160) public clock;

    function setClock(int24 lower, int24 upper, uint160 v) external {
        clock[keccak256(abi.encode(lower, upper))] = v;
    }

    function snapshotCumulativesInside(int24 lower, int24 upper) external view returns (int56, uint160, uint32) {
        return (0, clock[keccak256(abi.encode(lower, upper))], 0);
    }
}

contract MockFactory {
    mapping(address => mapping(address => mapping(uint24 => address))) public pools;

    function setPool(address a, address b, uint24 fee, address pool) external {
        pools[a][b][fee] = pool;
        pools[b][a][fee] = pool;
    }

    function getPool(address a, address b, uint24 fee) external view returns (address) {
        return pools[a][b][fee];
    }
}

contract MockNPM {
    struct Pos {
        address token0;
        address token1;
        uint24 fee;
        int24 tickLower;
        int24 tickUpper;
        uint128 liquidity;
    }

    mapping(uint256 => Pos) public pos;
    mapping(uint256 => address) public ownerOf;
    mapping(uint256 => address) public approved;
    address public factory;

    constructor(address f) {
        factory = f;
    }

    function mintPos(address to, uint256 id, address t0, address t1, uint24 fee, int24 lo, int24 hi, uint128 liq)
        external
    {
        pos[id] = Pos(t0, t1, fee, lo, hi, liq);
        ownerOf[id] = to;
    }

    function positions(uint256 id)
        external
        view
        returns (uint96, address, address, address, uint24, int24, int24, uint128, uint256, uint256, uint128, uint128)
    {
        Pos memory p = pos[id];
        return (0, address(0), p.token0, p.token1, p.fee, p.tickLower, p.tickUpper, p.liquidity, 0, 0, 0, 0);
    }

    function approve(address to, uint256 id) external {
        require(ownerOf[id] == msg.sender, "not owner");
        approved[id] = to;
    }

    function safeTransferFrom(address from, address to, uint256 id) external {
        require(ownerOf[id] == from, "not owner");
        require(msg.sender == from || approved[id] == msg.sender, "not approved");
        ownerOf[id] = to;
        approved[id] = address(0);
        if (to.code.length > 0) {
            require(
                IERC721ReceiverMock(to).onERC721Received(msg.sender, from, id, "")
                    == IERC721ReceiverMock.onERC721Received.selector,
                "bad receiver"
            );
        }
    }

    /// plain ERC-721 transfer - the hook-less path every real NPM also has;
    /// mirrors safeTransferFrom minus the receiver callback. Used to pin the
    /// stranded-NFT behavior below.
    function transferFrom(address from, address to, uint256 id) external {
        require(ownerOf[id] == from, "not owner");
        ownerOf[id] = to;
    }
}

/// The Orchard (FARMING.md): stake canonical-v3 position NFTs into posted,
/// finite seasons of the bound game coin; shares are liquidity x
/// seconds-IN-RANGE from the pool's own clock, so concentration is never
/// overpaid. Emission can never exceed the posted pot. In production the
/// wage is OBOL (SEASONS.md 4, since 2026-07-25); this suite deliberately
/// binds a MYRRH-named instance to pin that the coin is a deploy-time
/// constructor choice, not a contract assumption. `forge test -vv` before
/// any broadcast.
contract ScryOrchardTest is Test {
    SpoilsToken myrrh;
    MockV3Pool pool;
    MockFactory factory;
    MockNPM npm;
    ScryOrchard orchard;

    address op = address(0xC0FFEE);
    address alice = address(0xA11CE);
    address bob = address(0xB0B);

    address t0 = address(0x1111);
    address t1 = address(0x2222);
    uint24 constant FEE = 3000;

    uint128 constant LIQ = uint128(1) << 64; // power of two: clock math lands exact
    uint256 constant POT = 100e18;
    uint256 constant WINDOW = 1000;

    uint256 start;
    uint256 end;

    function setUp() public {
        myrrh = new SpoilsToken("myrrh", "MYRRH", 0, op);
        pool = new MockV3Pool();
        factory = new MockFactory();
        npm = new MockNPM(address(factory));
        factory.setPool(t0, t1, FEE, address(pool));

        orchard = new ScryOrchard(
            myrrh, INonfungiblePositionManagerOrchard(address(npm)), IUniswapV3FactoryOrchard(address(factory))
        ); // owner = this

        vm.prank(op);
        myrrh.mint(address(this), 10 * POT);
        myrrh.approve(address(orchard), type(uint256).max);

        start = block.timestamp;
        end = start + WINDOW;
        orchard.createIncentive(address(pool), POT, start, end); // incentive 0

        npm.mintPos(alice, 1, t0, t1, FEE, -100, 100, LIQ);
        npm.mintPos(bob, 2, t0, t1, FEE, -10, 10, LIQ); // the thin range
    }

    /// clock value meaning "N seconds in range at LIQ active liquidity"
    function _splFor(uint256 secondsInside) internal pure returns (uint160) {
        return uint160((secondsInside << 128) / LIQ);
    }

    function _depositAndStake(address who, uint256 id) internal {
        vm.startPrank(who);
        npm.approve(address(orchard), id);
        orchard.depositToken(id);
        orchard.stakeToken(id, 0);
        vm.stopPrank();
    }

    // -- custody ------------------------------------------------------------

    function test_deposit_records_owner_only_via_the_npm() public {
        vm.prank(alice);
        npm.safeTransferFrom(alice, address(orchard), 1);
        (address owner_, bool staked,,,,,) = orchard.deposits(1);
        assertEq(owner_, alice);
        assertFalse(staked);
        vm.expectRevert("not the position manager");
        orchard.onERC721Received(address(this), alice, 99, ""); // no side door
    }

    function test_holderDepositRequiresNpmApprovalAndRecordsOwner() public {
        vm.prank(alice);
        vm.expectRevert("not approved");
        orchard.depositToken(1);

        vm.startPrank(alice);
        npm.approve(address(orchard), 1);
        orchard.depositToken(1);
        vm.stopPrank();

        assertEq(npm.ownerOf(1), address(orchard));
        (address owner_, bool staked,,,,,) = orchard.deposits(1);
        assertEq(owner_, alice);
        assertFalse(staked);
    }

    function test_withdraw_returns_the_nft_and_clears_the_record() public {
        _depositAndStake(alice, 1);
        vm.expectRevert("unstake first");
        vm.prank(alice);
        orchard.withdrawToken(1, alice);
        vm.prank(alice);
        orchard.unstakeToken(1);
        vm.prank(bob);
        vm.expectRevert("not your deposit");
        orchard.withdrawToken(1, bob); // only the depositor takes it out
        vm.prank(alice);
        orchard.withdrawToken(1, alice);
        assertEq(npm.ownerOf(1), alice);
        (address owner_,,,,,,) = orchard.deposits(1);
        assertEq(owner_, address(0));
    }

    // -- the range clock pays the season ------------------------------------

    function test_sole_staker_in_range_all_season_earns_the_whole_pot() public {
        _depositAndStake(alice, 1);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(WINDOW)); // in range the whole window
        vm.prank(alice);
        uint256 reward = orchard.unstakeToken(1);
        assertEq(reward, POT);
        vm.prank(alice);
        orchard.claimReward(alice);
        assertEq(myrrh.balanceOf(alice), POT);
    }

    /// AFTER THE CLOSE the denominator grows with wall time (canonical
    /// v3-staker shape). This test's predecessor pinned the opposite - a
    /// frozen window - and its mock clock could not show why that was wrong:
    /// a REAL pool's clock keeps extrapolating for a position still in range,
    /// so the frozen denominator paid a camper 2.000x its at-close amount
    /// (measured on a live fork, 2026-07-27) out of the sweepable remainder.
    /// Three pins of the new shape, one per case:
    ///
    /// 1. still IN RANGE after the close -> exactly neutral. Numerator and
    ///    denominator grow together, so waiting buys nothing and loses
    ///    nothing. This is the property the old comment claimed ("never paid
    ///    less merely because they unstaked late"), now true without the
    ///    subsidy. The fork mandate proves it against real bytecode.
    function test_late_unstake_in_range_after_the_close_is_neutral() public {
        _depositAndStake(alice, 1);
        vm.warp(end + WINDOW); // one full window late
        // in range the whole season AND the whole overtime: the clock kept
        // running, exactly what a real pool reports for an in-range position
        pool.setClock(-100, 100, _splFor(2 * WINDOW));
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), POT); // same as unstaking at the close
    }

    /// 2. OUT OF RANGE after the close -> the share decays with the wait
    ///    (the clock froze at the close; the denominator did not). The
    ///    canonical trade: a position whose price left its range after the
    ///    season is no longer working depth, and camping on it earns less,
    ///    never more. Prompt exits are never touched.
    function test_late_unstake_out_of_range_after_the_close_decays() public {
        _depositAndStake(alice, 1);
        vm.warp(end + 30 days);
        pool.setClock(-100, 100, _splFor(WINDOW)); // froze the moment the season closed
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), (POT * WINDOW) / (WINDOW + 30 days));
    }

    /// 3. the law the frozen denominator broke: a camper sharing the range
    ///    with unstaked liquidity (every real pool - the house LP is never
    ///    staked) can no longer beat its own at-close share. alice cashes
    ///    400/1000 at the close; bob camped one extra window at a ~10%
    ///    active-liquidity share (his clock adds 100s over the 1000s wait).
    ///    Old code: bob took the WHOLE 60e18 remainder. Now: strictly under
    ///    his 40e18 at-close share.
    function test_late_unstake_cannot_beat_its_at_close_share() public {
        _depositAndStake(alice, 1);
        _depositAndStake(bob, 2);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(400));
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), 40e18); // 400/1000 of the pot, at the close
        vm.warp(end + WINDOW);
        pool.setClock(-10, 10, _splFor(500)); // 400 in-window + 100 of overtime
        vm.prank(bob);
        uint256 rB = orchard.unstakeToken(2);
        assertEq(rB, (60e18 * 500) / 1600); // 18.75e18: (pot - paid) x s / grown denominator
        assertLe(rB, 40e18); // never more than the 60e18 x 400/600 he had at the close
    }

    function test_half_a_season_in_range_earns_half_rest_sweepable() public {
        _depositAndStake(alice, 1);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(WINDOW / 2));
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), POT / 2);
        vm.warp(end + 1);
        uint256 before = myrrh.balanceOf(address(this));
        orchard.endIncentive(0);
        assertEq(myrrh.balanceOf(address(this)) - before, POT / 2); // the unpaid half
    }

    function test_thin_range_earns_only_its_seconds_inside() public {
        // the honest-range property, the reason the MasterChef fence existed:
        // alice full-width sat in range 500s-worth; bob's razor range only 200s.
        _depositAndStake(alice, 1);
        _depositAndStake(bob, 2);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(500));
        pool.setClock(-10, 10, _splFor(200));
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), 50e18); // 500/1000 of the pot
        vm.prank(bob);
        assertEq(orchard.unstakeToken(2), 20e18); // 200 of the 500 seconds left, of 50e18
        vm.warp(end + 1);
        orchard.endIncentive(0); // 30e18 unearned sweeps home
        assertEq(myrrh.balanceOf(address(orchard)), 70e18); // credited, unclaimed rewards stay
    }

    function test_reward_is_clamped_by_the_pot_never_beyond() public {
        _depositAndStake(alice, 1);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(2 * WINDOW)); // absurd clock: 2x the window
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), POT); // clamped, no revert, no overdraw
    }

    function test_fully_cashed_season_still_lets_the_last_staker_exit() public {
        // Two concurrent in-range stakes sum to more liquidity-seconds than
        // the window holds; the first unstake cashes every unclaimed second.
        // The second must pay 0 - never divide by zero and trap the NFT.
        _depositAndStake(alice, 1);
        _depositAndStake(bob, 2);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(WINDOW)); // both ranges in range the
        pool.setClock(-10, 10, _splFor(WINDOW)); // whole window, concurrently
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), POT); // takes the whole pot
        vm.prank(bob);
        assertEq(orchard.unstakeToken(2), 0); // pays 0, exits cleanly
        vm.prank(bob);
        orchard.withdrawToken(2, bob); // custody out, nothing trapped
        assertEq(npm.ownerOf(2), bob);
        vm.warp(end + 1);
        vm.expectRevert("nothing left"); // pot fully paid; sweep is empty
        orchard.endIncentive(0);
    }

    function test_restake_into_a_new_season_uses_a_fresh_snapshot() public {
        _depositAndStake(alice, 1);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(WINDOW));
        vm.prank(alice);
        orchard.unstakeToken(1); // season 0 fully cashed
        uint256 start2 = block.timestamp;
        orchard.createIncentive(address(pool), POT, start2, start2 + WINDOW); // incentive 1
        vm.prank(alice);
        orchard.stakeToken(1, 1);
        vm.warp(start2 + WINDOW);
        pool.setClock(-100, 100, _splFor(WINDOW + WINDOW / 4)); // +250s this season
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), POT / 4); // only the delta counts
    }

    // -- season boundaries --------------------------------------------------

    function test_stake_outside_the_window_refused() public {
        uint256 later = block.timestamp + 5000;
        orchard.createIncentive(address(pool), POT, later, later + WINDOW); // incentive 1
        vm.startPrank(alice);
        npm.safeTransferFrom(alice, address(orchard), 1);
        vm.expectRevert("season not started");
        orchard.stakeToken(1, 1);
        vm.stopPrank();
        vm.warp(later + WINDOW);
        vm.expectRevert("season over");
        vm.prank(alice);
        orchard.stakeToken(1, 1);
    }

    function test_wrong_pool_and_empty_positions_refused() public {
        npm.mintPos(alice, 3, t0, t1, 500, -100, 100, LIQ); // fee tier with no pool
        npm.mintPos(alice, 4, t0, t1, FEE, -100, 100, 0); // no liquidity
        vm.startPrank(alice);
        npm.safeTransferFrom(alice, address(orchard), 3);
        vm.expectRevert("wrong pool");
        orchard.stakeToken(3, 0);
        npm.safeTransferFrom(alice, address(orchard), 4);
        vm.expectRevert("no liquidity");
        orchard.stakeToken(4, 0);
        npm.safeTransferFrom(alice, address(orchard), 1);
        orchard.stakeToken(1, 0);
        vm.expectRevert("already staked");
        orchard.stakeToken(1, 0);
        vm.stopPrank();
    }

    function test_after_the_window_anyone_may_unstake_reward_still_credits_owner() public {
        _depositAndStake(alice, 1);
        vm.expectRevert("not your deposit");
        vm.prank(bob);
        orchard.unstakeToken(1); // mid-season: owner only
        vm.warp(end + 1);
        pool.setClock(-100, 100, _splFor(WINDOW));
        vm.prank(bob);
        orchard.unstakeToken(1); // season over: anyone may clear the way
        assertGt(orchard.rewardsOf(alice), 0);
        assertEq(orchard.rewardsOf(bob), 0); // the caller earns nothing
    }

    function test_end_incentive_gates() public {
        _depositAndStake(alice, 1);
        vm.prank(alice);
        vm.expectRevert("owner only");
        orchard.endIncentive(0); // the sweep is the operator's alone
        vm.expectRevert("season running");
        orchard.endIncentive(0);
        vm.warp(end + 1);
        vm.expectRevert("stakes remain");
        orchard.endIncentive(0); // never swept out from under a staker
        vm.prank(alice);
        orchard.unstakeToken(1); // clock never moved: zero reward
        orchard.endIncentive(0);
        assertEq(myrrh.balanceOf(address(orchard)), 0);
        vm.expectRevert("nothing left");
        orchard.endIncentive(0);
    }

    // -- posting + claiming -------------------------------------------------

    function test_a_season_is_funded_when_posted_or_not_at_all() public {
        uint256 before = myrrh.balanceOf(address(this));
        orchard.createIncentive(address(pool), POT, block.timestamp, block.timestamp + 1);
        assertEq(before - myrrh.balanceOf(address(this)), POT); // pot moved NOW
        vm.expectRevert("zero pool");
        orchard.createIncentive(address(0), POT, block.timestamp, block.timestamp + 1);
        vm.expectRevert("zero pot");
        orchard.createIncentive(address(pool), 0, block.timestamp, block.timestamp + 1);
        vm.expectRevert("start in past");
        orchard.createIncentive(address(pool), POT, block.timestamp - 1, block.timestamp + 1);
        vm.expectRevert("bad window");
        orchard.createIncentive(address(pool), POT, block.timestamp + 2, block.timestamp + 1);
        vm.expectRevert("owner only");
        vm.prank(alice);
        orchard.createIncentive(address(pool), POT, block.timestamp, block.timestamp + 1);
    }

    /// The uint160 accumulator holds (window << 128) only while the window
    /// fits 32 bits, so createIncentive refuses anything wider - pinned at
    /// the exact boundary: 2^32 - 1 seconds accepted, 2^32 refused.
    function test_window_boundary_uint32_max_accepted_one_over_refused() public {
        uint256 startAt = block.timestamp;
        uint256 id = orchard.createIncentive(address(pool), POT, startAt, startAt + type(uint32).max);
        (,,,, uint256 endStored,) = orchard.incentives(id);
        assertEq(endStored, startAt + type(uint32).max); // widest legal season
        vm.expectRevert("window too long");
        orchard.createIncentive(address(pool), POT, startAt, startAt + uint256(type(uint32).max) + 1);
    }

    function test_claim_pays_and_zeroes() public {
        _depositAndStake(alice, 1);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(WINDOW));
        vm.prank(alice);
        orchard.unstakeToken(1);
        vm.prank(alice);
        assertEq(orchard.claimReward(alice), POT);
        assertEq(orchard.rewardsOf(alice), 0);
        vm.expectRevert("nothing accrued");
        vm.prank(alice);
        orchard.claimReward(alice);
    }

    // -- the mid-season denominator (TEST-AUDIT.md HIGH: unstake-early ------
    // -- shape pinned exactly) ----------------------------------------------

    /// A mid-season unstake divides by the FULL posted window, never the
    /// elapsed slice: with `now < end`, `_max(endTime, now) - startTime` is
    /// the whole WINDOW, so
    ///   reward = pot * secondsInside / WINDOW = 100e18 * 400/1000 = 40e18
    /// and NOT the elapsed-time fraction (400 of 500 elapsed -> 80e18).
    /// That denominator is the anti-drain shape: a sole in-range staker who
    /// leaves mid-season is paid against the whole season, and the rest of
    /// the pot stays for whoever farms the back half (or the sweep). Exact
    /// pin of the audit's drain-shape question.
    function test_mid_season_unstake_pays_full_window_fraction_exact() public {
        _depositAndStake(alice, 1);
        vm.warp(start + WINDOW / 2); // now < end: mid-season
        pool.setClock(-100, 100, _splFor(400)); // 400s in range so far
        vm.prank(alice);
        uint256 reward = orchard.unstakeToken(1);
        assertEq(reward, (POT * 400) / WINDOW); // the full-window denominator
        assertEq(reward, 40e18); // pinned as a literal too
        (, uint256 unclaimed,,,,) = orchard.incentives(0);
        assertEq(unclaimed, 60e18); // the back half stays potted
    }

    // -- the plain-transferFrom trap, now RESCUABLE (operator, 2026-07-25) --

    /// WAS PINNED, NOT ENDORSED: an NFT sent by plain `transferFrom` stranded
    /// FOREVER. Only safeTransferFrom fires onERC721Received, and that hook is
    /// the ONLY writer of the deposits record, so a bare transfer moved custody
    /// without one — owner address(0), `unstakeToken` "not staked",
    /// `withdrawToken` "not your deposit" for every caller alive.
    ///
    /// The old comment said the fix was "a src/ change and an operator
    /// decision". The operator made it: `rescueOrphan`. The trap itself is
    /// unchanged and still pinned below — a bare transfer still records
    /// nothing, and the normal reclaim paths still refuse. What changed is
    /// that the position is no longer lost.
    function test_plain_transferFrom_still_records_nothing_but_is_now_rescuable() public {
        address orchardAddr = address(orchard);
        vm.prank(alice);
        npm.transferFrom(alice, orchardAddr, 1); // no hook fires
        assertEq(npm.ownerOf(1), orchardAddr); // custody moved...
        (address owner_, bool staked,,,,,) = orchard.deposits(1);
        assertEq(owner_, address(0)); // ...but no record was written
        assertFalse(staked);
        vm.prank(alice);
        vm.expectRevert("not staked");
        orchard.unstakeToken(1);
        vm.prank(alice);
        vm.expectRevert("not your deposit");
        orchard.withdrawToken(1, alice); // the sender still cannot reclaim it
        vm.expectRevert("not your deposit");
        orchard.withdrawToken(1, alice); // nor can anyone else (owner is 0)

        // ...and it is no longer lost: the operator hands it back.
        orchard.rescueOrphan(1, alice);
        assertEq(npm.ownerOf(1), alice, "returned to the sender");
    }

    /// THE WALL that makes the rescue safe, and it is structural rather than a
    /// promise: `rescueOrphan` requires `deposits[id].owner == address(0)`, and
    /// every properly deposited position has a non-zero owner from the instant
    /// the receiver hook runs. So the function provably cannot reach an NFT
    /// anyone deposited correctly — staked or merely parked. "The owner cannot
    /// take my staked position" survives intact.
    function test_rescueOrphan_cannot_touch_a_real_deposit_or_a_stake() public {
        vm.prank(alice);
        npm.approve(address(orchard), 1);
        vm.prank(alice);
        orchard.depositToken(1); // the PROPER path: the hook records alice

        vm.expectRevert("not an orphan - it has a depositor");
        orchard.rescueOrphan(1, address(this));

        // and once it is staked in the live season, still no
        vm.prank(alice);
        orchard.stakeToken(1, 0);
        vm.expectRevert("not an orphan - it has a depositor");
        orchard.rescueOrphan(1, address(this));
        assertEq(npm.ownerOf(1), address(orchard), "alice's position is untouched");
    }

    function test_rescueOrphan_is_owner_only_and_checks_custody() public {
        vm.prank(alice);
        npm.transferFrom(alice, address(orchard), 1);

        vm.prank(alice);
        vm.expectRevert("owner only");
        orchard.rescueOrphan(1, alice);

        vm.expectRevert("bad to");
        orchard.rescueOrphan(1, address(0));
        vm.expectRevert("bad to");
        orchard.rescueOrphan(1, address(orchard));

        // an id this contract does not hold cannot be "rescued" out of thin air
        vm.expectRevert("this contract does not hold it");
        orchard.rescueOrphan(2, alice);
    }

    // -- guard strings (TEST-AUDIT.md: the four reverts, exact bytes) -------

    function test_stake_guards_not_your_deposit_and_pot_empty() public {
        // not-your-deposit: bob cannot stake alice's parked NFT
        vm.prank(alice);
        npm.safeTransferFrom(alice, address(orchard), 1);
        vm.prank(bob);
        vm.expectRevert("not your deposit");
        orchard.stakeToken(1, 0);

        // pot-empty: drain the whole pot mid-season (absurd clock - the
        // unclaimed-seconds clamp cashes everything), then a fresh staker
        // is refused while the window is still open
        vm.prank(alice);
        orchard.stakeToken(1, 0);
        vm.warp(start + WINDOW / 2);
        pool.setClock(-100, 100, _splFor(WINDOW));
        vm.prank(alice);
        uint256 reward = orchard.unstakeToken(1);
        assertEq(reward, POT); // pot fully cashed mid-season
        vm.prank(bob);
        npm.safeTransferFrom(bob, address(orchard), 2);
        vm.prank(bob);
        vm.expectRevert("pot empty");
        orchard.stakeToken(2, 0);
    }

    function test_unstake_fresh_id_not_staked() public {
        vm.expectRevert("not staked");
        orchard.unstakeToken(77); // never deposited, never staked
    }

    function test_withdraw_bad_to_guards() public {
        address orchardAddr = address(orchard); // hoisted: nothing external after a cheatcode
        vm.prank(alice);
        npm.safeTransferFrom(alice, orchardAddr, 1);
        vm.prank(alice);
        vm.expectRevert("bad to");
        orchard.withdrawToken(1, address(0));
        vm.prank(alice);
        vm.expectRevert("bad to");
        orchard.withdrawToken(1, orchardAddr); // the orchard itself is no exit
    }

    // -- governance (TEST-AUDIT.md: every transferOwner, one test) ----------

    function test_transferOwner_old_refused_new_works() public {
        vm.expectRevert("zero owner");
        orchard.transferOwner(address(0));
        vm.prank(bob);
        vm.expectRevert("owner only");
        orchard.transferOwner(bob); // a stranger cannot take the keys
        orchard.transferOwner(alice);
        assertEq(orchard.owner(), alice);
        vm.expectRevert("owner only");
        orchard.transferOwner(bob); // the old owner (this) is refused
        vm.prank(alice);
        orchard.transferOwner(bob); // the new owner works
        assertEq(orchard.owner(), bob);
    }

    // -- custody records are written once (audit hardening, 2026-07-27) -----

    /// The record gate is the RECORD, not custody: a second depositToken for a
    /// token already in the Orchard refuses whoever calls it, and the door
    /// re-opens only when withdrawToken deletes the record.
    function test_depositToken_refuses_a_token_already_in_custody() public {
        vm.startPrank(alice);
        npm.approve(address(orchard), 1);
        orchard.depositToken(1);
        vm.expectRevert("already deposited");
        orchard.depositToken(1); // the depositor herself is refused
        vm.stopPrank();
        vm.prank(bob);
        vm.expectRevert("already deposited");
        orchard.depositToken(1); // and so is anyone else
        vm.prank(alice);
        orchard.withdrawToken(1, alice); // delete re-opens the door
        vm.startPrank(alice);
        npm.approve(address(orchard), 1);
        orchard.depositToken(1);
        vm.stopPrank();
        (address owner_,,,,,,) = orchard.deposits(1);
        assertEq(owner_, alice);
    }

    /// The receiver hook refuses to overwrite a live record even when the
    /// caller IS the position manager - the one line that makes "a deposit
    /// can never be reset out from under its owner" true by construction
    /// rather than by the external no-approvals invariant.
    function test_hook_never_overwrites_a_live_record() public {
        vm.prank(alice);
        npm.safeTransferFrom(alice, address(orchard), 1); // record: alice
        vm.prank(address(npm));
        vm.expectRevert("already recorded");
        orchard.onERC721Received(address(this), bob, 1, ""); // bob cannot replace her
    }

    // -- leaving stops the clock; returning restarts it ----------------------

    /// "It is not a lock": unstake mid-season, sit out, re-stake into the SAME
    /// running season. The gap seconds pay nothing (fresh snapshot at restake)
    /// and both legs settle against the posted window with the pot conserved.
    function test_restake_same_running_season_gap_seconds_pay_nothing() public {
        _depositAndStake(alice, 1);
        vm.warp(start + WINDOW / 2);
        pool.setClock(-100, 100, _splFor(300)); // 300s in range so far
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), 30e18); // POT * 300/1000
        pool.setClock(-100, 100, _splFor(600)); // 300 more range-seconds pass UNstaked
        vm.prank(alice);
        orchard.stakeToken(1, 0); // same season, fresh snapshot at 600
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(900)); // +300 staked seconds
        vm.prank(alice);
        // remaining pot 70e18 against remaining window 700s: 70e18 * 300/700
        assertEq(orchard.unstakeToken(1), 30e18);
        (, uint256 unclaimed,,,,) = orchard.incentives(0);
        assertEq(unclaimed, 40e18); // the 300 unstaked + 100 unfarmed seconds stay potted
    }

    /// Two overlapping seasons on ONE pool: a position stakes in one at a
    /// time ("already staked" even across seasons), and each season settles
    /// against its own snapshot, window, and pot.
    function test_two_concurrent_seasons_same_pool_one_nft_sequential() public {
        orchard.createIncentive(address(pool), POT, start, end); // incentive 1, same window+pool
        _depositAndStake(alice, 1); // into season 0
        vm.prank(alice);
        vm.expectRevert("already staked");
        orchard.stakeToken(1, 1); // one stake at a time, even into another season
        vm.warp(start + WINDOW / 2);
        pool.setClock(-100, 100, _splFor(400));
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), 40e18); // season 0: POT * 400/1000
        vm.prank(alice);
        orchard.stakeToken(1, 1); // now season 1, snapshot 400
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(900));
        vm.prank(alice);
        assertEq(orchard.unstakeToken(1), 50e18); // season 1: POT * 500/1000, own baseline
        vm.warp(end + 1);
        uint256 before = myrrh.balanceOf(address(this));
        orchard.endIncentive(0);
        orchard.endIncentive(1);
        assertEq(myrrh.balanceOf(address(this)) - before, 60e18 + 50e18); // both sweep independently
    }

    /// claimReward pays the NAMED recipient and zeroes the CALLER's accrual -
    /// the payment address never becomes an accrual address.
    function test_claimReward_pays_named_recipient_and_zeroes_caller() public {
        _depositAndStake(alice, 1);
        vm.warp(end);
        pool.setClock(-100, 100, _splFor(WINDOW));
        vm.prank(alice);
        orchard.unstakeToken(1);
        vm.prank(alice);
        assertEq(orchard.claimReward(bob), POT); // alice directs the payout to bob
        assertEq(myrrh.balanceOf(bob), POT);
        assertEq(myrrh.balanceOf(alice), 0);
        assertEq(orchard.rewardsOf(alice), 0);
        vm.prank(bob);
        vm.expectRevert("nothing accrued");
        orchard.claimReward(bob); // being paid granted bob no accrual
    }

    // -- fuzz: the load-bearing money law ------------------------------------
    /// EMISSION IS BOUNDED BY THE POT, for ANY clock the pool reports. The
    /// contract's headline promise ("emission can never exceed the posted
    /// pot") is pinned only at hand-picked points in the unit tests above;
    /// here two stakers cash arbitrary - even adversarial, over-window -
    /// seconds-in-range, unstaking in a fuzzed order at fuzzed times, and the
    /// Orchard must (a) never pay more than POT in total and (b) conserve the
    /// pot to the wei: everything paid plus everything still unclaimed equals
    /// exactly POT. A single over-emission or an underflow fails this. Bounds
    /// stay inside the contract's own "sane pot x window" 256-bit envelope
    /// (its unstake-math comment) so a legitimately-scoped season is covered
    /// without manufacturing a spurious overflow the contract never promises.
    function testFuzz_emission_never_exceeds_pot(uint256 secA, uint256 secB, uint256 gap) public {
        secA = bound(secA, 0, 1_000_000); // 0..1000x the window: exercises
        secB = bound(secB, 0, 1_000_000); // both the normal and cap branches
        gap = bound(gap, 0, 3 * WINDOW); // unstake inside or well past the season

        _depositAndStake(alice, 1); // range [-100, 100]
        _depositAndStake(bob, 2); // the thin range [-10, 10]

        // the pool reports whatever seconds-in-range it likes for each range.
        pool.setClock(-100, 100, _splFor(secA));
        pool.setClock(-10, 10, _splFor(secB));

        vm.warp(start + gap);
        vm.prank(alice);
        uint256 rA = orchard.unstakeToken(1);
        vm.warp(block.timestamp + 1);
        vm.prank(bob);
        uint256 rB = orchard.unstakeToken(2);

        (, uint256 unclaimed,,,,) = orchard.incentives(0);
        assertLe(rA + rB, POT, "over-emitted the pot");
        assertEq(rA + rB + unclaimed, POT, "pot not conserved to the wei");
        assertEq(orchard.rewardsOf(alice), rA);
        assertEq(orchard.rewardsOf(bob), rB);
        assertEq(myrrh.balanceOf(address(orchard)), POT); // nothing left the vault yet
    }
}
