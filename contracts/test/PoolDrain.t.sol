// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {EloJobBoard, IReputation, IInsurancePool} from "../src/EloJobBoard.sol";
import {EloInsurancePool} from "../src/EloInsurancePool.sol";
import {EloReputation} from "../src/EloReputation.sol";
import {IEloArbiter} from "../src/IEloArbiter.sol";
import {IERC20} from "../src/IERC20.sol";

contract MockSCRY {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 a) external {
        balanceOf[to] += a;
    }

    function approve(address s, uint256 a) external returns (bool) {
        allowance[msg.sender][s] = a;
        return true;
    }

    function transfer(address to, uint256 a) external returns (bool) {
        require(balanceOf[msg.sender] >= a, "bal");
        balanceOf[msg.sender] -= a;
        balanceOf[to] += a;
        return true;
    }

    function transferFrom(address f, address t, uint256 a) external returns (bool) {
        require(balanceOf[f] >= a, "bal");
        require(allowance[f][msg.sender] >= a, "allow");
        allowance[f][msg.sender] -= a;
        balanceOf[f] -= a;
        balanceOf[t] += a;
        return true;
    }
}

contract MockArbiter is IEloArbiter {
    function flatFee() external pure returns (uint256) {
        return 0;
    }

    function feeRecipient() external view returns (address) {
        return address(this);
    }

    function rule(uint256, bytes32, bytes32) external pure returns (Verdict) {
        return Verdict.Undecided;
    }
}

/// WAS: proof that EloJobBoard's Insured mode let a colluding (buyer, seller)
/// pair drain EloInsurancePool for the price of a 1-wei premium per round.
/// MEASURED before the fix: ten rounds took 9,500 SCRY out of a 10,000 SCRY
/// pool for 10 wei of premiums, using a fresh throwaway pair each time so the
/// soulbound rep slash landed on addresses nobody intended to reuse.
///
/// FIXED 2026-07-25 with three independent bounds (contracts/AUDIT-2026-07-25.md):
///   1. an Insured hire needs a seller over the reputation threshold — burners
///      have none, and rep is earned over real jobs, never bought;
///   2. the premium is priced against the sum insured (minPremiumBps);
///   3. EloInsurancePool caps what any ONE beneficiary draws per rolling
///      window — the bound that actually caps the loss when claims are
///      manufactured rather than merely expensive.
/// The assertions below are INVERTED in place rather than deleted, so this
/// still fails the day any of the three is removed.
///
/// (Also fixed: the loop read `block.timestamp` around `vm.warp`. via_ir
/// legally CSEs the TIMESTAMP opcode within a call, so the post-warp read
/// collapsed to the pre-warp value and every round after the first reverted
/// "deadline passed" — a red for the wrong reason. Use vm.getBlockTimestamp().)
contract PoolDrainTest is Test {
    MockSCRY reserve;
    EloReputation rep;
    EloInsurancePool pool;
    EloJobBoard board;

    function setUp() public {
        reserve = new MockSCRY();
        rep = new EloReputation(50);
        // per-claim cap 1_000e18, same as the shipped test harness
        pool = new EloInsurancePool(IERC20(address(reserve)), 1_000e18);
        board = new EloJobBoard(
            IERC20(address(reserve)),
            IReputation(address(rep)),
            IInsurancePool(address(pool)),
            address(0xFEE5),
            IEloArbiter(address(new MockArbiter())),
            10e18
        );
        rep.setAuthority(address(board), true);
        pool.setClaimant(address(board), true);
        // The mode SHIPS CLOSED (operator, 2026-07-25) — precisely because the
        // three bounds proved below raise the cost of this attack and cap its
        // rate without ever making it unprofitable in the limit. This suite
        // opens the door on purpose so the bounds themselves stay under test;
        // that a shipped board refuses the mode outright is pinned in
        // EloMarket.t.sol::testInsuredModeIsClosedOnAFreshBoard.
        board.setInsuredOpen(true);

        // honest LPs fund the pool with 10,000 SCRY of premiums
        reserve.mint(address(this), 10_000e18);
        reserve.approve(address(pool), 10_000e18);
        pool.payPremium(10_000e18);
    }

    /// Bounds 1 and 2: the throwaway pair cannot open the job at all now.
    function test_insured_refuses_a_throwaway_seller_and_a_token_premium() public {
        assertEq(reserve.balanceOf(address(pool)), 10_000e18, "pool funded");
        uint256 t = block.timestamp;

        address mark = address(uint160(0xBAD0000)); // the "buyer"
        address sink = address(uint160(0xBAD0001)); // the "seller"
        reserve.mint(mark, 1_000e18);
        vm.startPrank(mark);
        reserve.approve(address(board), 1_000e18);

        // WAS: this posted fine, and two seconds later the pool covered 950e18.
        vm.expectRevert(bytes("seller below rep threshold"));
        board.post(sink, 1_000e18, EloJobBoard.Mode.Insured, keccak256("anything"), uint64(t + 1), 1);
        vm.stopPrank();

        // and even once the seller really has cleared the bar, a token premium
        // is refused — coverage is the pool's money, so it is priced against
        // the sum it may claim.
        rep.setAuthority(address(this), true);
        rep.earn(sink, 100, "seed");
        vm.startPrank(mark);
        vm.expectRevert(bytes("premium below the posted floor"));
        board.post(sink, 1_000e18, EloJobBoard.Mode.Insured, keccak256("anything"), uint64(t + 1), 1);
        vm.stopPrank();

        assertEq(reserve.balanceOf(address(pool)), 10_000e18, "not a wei left the pool");
    }

    /// Bound 3, the one that actually caps the loss: a seller who really did
    /// clear the bar, paying the full priced premium every round, still cannot
    /// draw more than one window's cap out of the pool.
    function test_insured_cover_is_rate_limited_per_beneficiary() public {
        uint256 startingReserves = reserve.balanceOf(address(pool));
        address mark = address(uint160(0xBAD0000));
        address sink = address(uint160(0xBAD0001));
        rep.setAuthority(address(this), true);
        rep.earn(sink, 100, "seed");

        // `block.timestamp` is loop-invariant inside one tx, so the compiler
        // caches it — track the clock ourselves and warp explicitly.
        uint256 t = block.timestamp;
        uint256 premiums;
        for (uint256 i = 0; i < 10; i++) {
            reserve.mint(mark, 30e18); // 3% of the sum insured, the posted floor
            premiums += 30e18;
            vm.startPrank(mark);
            reserve.approve(address(board), 30e18);
            uint256 id =
                board.post(sink, 1_000e18, EloJobBoard.Mode.Insured, keccak256("anything"), uint64(t + 1), 30e18);
            vm.stopPrank();

            vm.prank(sink);
            board.deliver(id, keccak256("garbage"));

            t += 2;
            vm.warp(t);
            board.close(id);
        }
        uint256 drawn = reserve.balanceOf(sink);

        emit log_named_uint("premiums paid", premiums);
        emit log_named_uint("drawn from the pool", drawn);
        emit log_named_uint("pool reserves left", reserve.balanceOf(address(pool)));

        // WAS: ten rounds x 950e18 emptied the pool. NOW the per-beneficiary
        // window is one full claim cap per day, so ten rounds inside one day
        // pay out that cap ONCE (950e18 on the first, the 50e18 remainder on
        // the second, nothing after) instead of ten times.
        assertEq(drawn, 1_000e18, "exactly one window's cap across all ten rounds");
        assertEq(reserve.balanceOf(address(pool)), startingReserves + premiums - 1_000e18, "the LPs' money stayed put");
        assertEq(pool.claimableThisWindow(sink), 0, "the window is spent down, not reset per job");

        // and the drain no longer builds the reputation that gates the mode:
        // a covered claim is not an honest completion, so it earns nothing.
        assertEq(rep.rep(sink), 100, "seeded rep only - cover never earns rep");
    }
}
