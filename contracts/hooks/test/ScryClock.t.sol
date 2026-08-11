// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Test, console2} from "forge-std/Test.sol";
import {Deployers} from "v4-core/test/utils/Deployers.sol";
import {IPoolManager} from "v4-core/src/interfaces/IPoolManager.sol";
import {PoolKey} from "v4-core/src/types/PoolKey.sol";
import {PoolId, PoolIdLibrary} from "v4-core/src/types/PoolId.sol";
import {Currency} from "v4-core/src/types/Currency.sol";
import {IHooks} from "v4-core/src/interfaces/IHooks.sol";
import {Hooks} from "v4-core/src/libraries/Hooks.sol";
import {TickMath} from "v4-core/src/libraries/TickMath.sol";
import {SwapParams, ModifyLiquidityParams} from "v4-core/src/types/PoolOperation.sol";
import {PoolSwapTest} from "v4-core/src/test/PoolSwapTest.sol";
import {BaseHook} from "@openzeppelin/uniswap-hooks/src/base/BaseHook.sol";
import {ScryClock} from "../src/ScryClock.sol";

/// @title ScryClock tests — the spec, written before anything compiled
///
/// @dev ⚠ **These have never run.** Written in a container with no `forge`
///      (docs/onchain/HOOKS.md / ../BRINGUP.md). They are the specification of intended
///      behaviour, and the first job of whoever gets a compiler is to make them
///      run and then to disbelieve the ones that pass on the first try.
///
///      What matters most and is hardest to prove with a mock: the
///      delta-return path in `_afterSwap`. A unit test can assert `pot` moved;
///      only a real swap through a real PoolManager proves the accounting
///      balances. `test_fork_*` is the one that counts.
contract ScryClockTest is Test, Deployers {
    using PoolIdLibrary for PoolKey;

    ScryClock clock;
    // `key` is Deployers', inherited. Declaring our own shadows it and the
    // helpers below would then fill one while `Deployers` reads the other.

    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");

    uint256 constant POT_BPS = 100; // 1%
    uint64 constant EXTENSION = 30 minutes;
    uint64 constant MAX_HORIZON = 24 hours;
    uint256 constant ROLLOVER_BPS = 1_000; // 10%

    function setUp() public {
        deployFreshManagerAndRouters();
        deployMintAndApprove2Currencies();

        uint160 flags = uint160(
            Hooks.BEFORE_INITIALIZE_FLAG | Hooks.AFTER_SWAP_FLAG | Hooks.AFTER_SWAP_RETURNS_DELTA_FLAG
        );
        // currency1 is the prize; buying it is zeroForOne.
        deployCodeTo(
            "ScryClock.sol:ScryClock",
            abi.encode(manager, currency1, true, POT_BPS, EXTENSION, MAX_HORIZON, ROLLOVER_BPS),
            address(flags)
        );
        clock = ScryClock(address(flags));

        key = PoolKey(currency0, currency1, 10_000, 200, IHooks(address(clock)));
        manager.initialize(key, TickMath.getSqrtPriceAtTick(0));

        // Full range, and it has to be. Ticks must be multiples of the key's
        // tickSpacing (200) or v4 reverts TickMisaligned — ±120 is Uniswap's
        // stock LIQUIDITY_PARAMS, written for spacing 60. Aligning it is not
        // enough on its own: `test_the_horizon_caps_the_deadline` makes 60
        // one-ether buys, which walks the price straight out of any narrow
        // band and then reverts PriceLimitAlreadyExceeded on the next swap.
        // Depth is not what these tests are about, so take it off the table.
        modifyLiquidityRouter.modifyLiquidity(
            key,
            ModifyLiquidityParams(
                TickMath.minUsableTick(key.tickSpacing), TickMath.maxUsableTick(key.tickSpacing), 100 ether, 0
            ),
            ""
        );
    }

    // ------------------------------------------------------------------- bind

    function test_binds_to_the_first_pool_only() public view {
        assertTrue(clock.bound(), "should bind on initialize");
        assertEq(PoolId.unwrap(clock.poolId()), PoolId.unwrap(key.toId()));
    }

    /// @dev The attack this closes: anyone may open a pool against our hook —
    ///      the hook address is part of PoolKey — including one full of
    ///      worthless tokens purely to drive our clock.
    function test_a_second_pool_cannot_bind() public {
        PoolKey memory squatter = PoolKey(currency0, currency1, 3000, 60, IHooks(address(clock)));
        vm.expectRevert();
        manager.initialize(squatter, TickMath.getSqrtPriceAtTick(0));
    }

    // -------------------------------------------------------- opting in / out

    /// @dev The property that makes this pool safe for any router to route
    ///      through: no named beneficiary means an ordinary, untaxed swap.
    function test_swap_without_hookData_pays_nothing_and_wins_nothing() public {
        _buy(1 ether, "");
        assertEq(clock.pot(), 0, "no pot without a ticket");
        assertEq(clock.lastBuyer(), address(0), "no ticket without a contribution");
        assertEq(clock.deadline(), 0, "clock should not be running");
    }

    function test_a_named_buy_feeds_the_pot_and_takes_the_crown() public {
        _buy(1 ether, abi.encode(alice));
        assertGt(clock.pot(), 0, "pot should have grown");
        assertEq(clock.lastBuyer(), alice);
        assertEq(clock.deadline(), uint64(block.timestamp) + EXTENSION);
    }

    /// @dev `sender` in a v4 callback is the ROUTER, so a hook that credited it
    ///      would hand the router the pot. Nothing may credit the router.
    function test_the_router_never_becomes_the_last_buyer() public {
        _buy(1 ether, abi.encode(alice));
        assertTrue(clock.lastBuyer() != address(swapRouter), "router must never hold the crown");
        assertEq(clock.lastBuyer(), alice);
    }

    function test_selling_is_not_a_ticket() public {
        _buy(1 ether, abi.encode(alice));
        address holder = clock.lastBuyer();
        uint256 potBefore = clock.pot();

        _sell(1 ether, abi.encode(bob));

        assertEq(clock.pot(), potBefore, "a sell must not feed the pot");
        assertEq(clock.lastBuyer(), holder, "a sell must not take the crown");
    }

    /// @dev The dodge this closes: route as exact-output, pay no skim, still
    ///      take the crown. No contribution, no ticket.
    function test_exact_output_buy_pays_nothing_and_takes_nothing() public {
        _buy(-1 ether, abi.encode(alice)); // positive amountSpecified = exactOutput
        assertEq(clock.pot(), 0, "exact-output must not feed the pot");
        assertEq(clock.lastBuyer(), address(0), "and must not take the crown");
    }

    /// @dev A dust buy rounds its cut to zero. It must not buy the crown free.
    function test_a_dust_buy_earns_no_crown() public {
        _buy(1, abi.encode(alice));
        assertEq(clock.lastBuyer(), address(0), "a zero contribution is not a ticket");
    }

    // -------------------------------------------------------------- the clock

    function test_each_ticket_pushes_the_deadline_out() public {
        _buy(1 ether, abi.encode(alice));
        uint64 first = clock.deadline();

        skip(10 minutes);
        _buy(1 ether, abi.encode(bob));

        assertGt(clock.deadline(), first, "a ticket must extend the clock");
        assertEq(clock.lastBuyer(), bob, "and take the crown");
    }

    /// @dev Without the horizon a whale spams dust tickets and pushes the
    ///      deadline past anyone's patience for near-zero cost.
    function test_the_horizon_caps_the_deadline() public {
        for (uint256 i = 0; i < 60; i++) {
            _buy(1 ether, abi.encode(alice));
        }
        assertLe(
            clock.deadline(),
            uint64(block.timestamp) + MAX_HORIZON,
            "deadline must never exceed the horizon"
        );
    }

    /// @dev R6, and the one that would fail silently. On Nitro `block.number`
    ///      is the PARENT chain's and moves ~119x slower than the real chain.
    ///      A clock written in blocks would be unplayable and nothing would
    ///      error. Advancing the block number alone must not end a round.
    function test_the_clock_is_wall_time_not_block_number() public {
        _buy(1 ether, abi.encode(alice));

        vm.roll(block.number + 100_000);
        assertFalse(clock.settleable(), "block height must not move the clock");

        skip(EXTENSION + 1);
        assertTrue(clock.settleable(), "wall time must");
    }

    // ------------------------------------------------------------- settlement

    function test_cannot_settle_while_running() public {
        _buy(1 ether, abi.encode(alice));
        vm.expectRevert(ScryClock.StillRunning.selector);
        clock.settle();
    }

    function test_cannot_settle_an_empty_game() public {
        vm.expectRevert(ScryClock.NoRound.selector);
        clock.settle();
    }

    /// @dev Permissionless on purpose: a winner-only settle wedges the pot the
    ///      moment the winner loses their key. Carol has never touched the game.
    function test_anyone_may_ring_the_bell_but_only_the_winner_is_paid() public {
        _buy(1 ether, abi.encode(alice));
        uint256 potAtEnd = clock.pot();
        skip(EXTENSION + 1);

        vm.prank(carol);
        clock.settle();

        uint256 expectedRollover = (potAtEnd * ROLLOVER_BPS) / 10_000;
        assertEq(clock.owed(alice), potAtEnd - expectedRollover, "winner credited");
        assertEq(clock.owed(carol), 0, "the bell-ringer earns nothing");
        assertEq(clock.pot(), expectedRollover, "rollover seeds the next round");
        assertEq(clock.lastBuyer(), address(0), "round is cleared");
        assertEq(clock.round(), 1);
    }

    function test_the_pot_survives_into_the_next_round() public {
        _buy(1 ether, abi.encode(alice));
        skip(EXTENSION + 1);
        clock.settle();
        uint256 seeded = clock.pot();
        assertGt(seeded, 0, "next round should start with something on the table");

        _buy(1 ether, abi.encode(bob));
        assertGt(clock.pot(), seeded, "and keep growing");
        assertEq(clock.lastBuyer(), bob);
    }

    // ------------------------------------------------------------------ claim

    function test_winner_claims_real_tokens() public {
        _buy(1 ether, abi.encode(alice));
        skip(EXTENSION + 1);
        clock.settle();

        uint256 credited = clock.owed(alice);
        uint256 before = currency1.balanceOf(alice);

        vm.prank(alice);
        clock.claim();

        assertEq(currency1.balanceOf(alice) - before, credited, "paid in the prize coin");
        assertEq(clock.owed(alice), 0);
        assertEq(clock.owedTotal(), 0);
    }

    function test_claiming_nothing_reverts() public {
        vm.prank(carol);
        vm.expectRevert(ScryClock.NothingOwed.selector);
        clock.claim();
    }

    function test_cannot_claim_twice() public {
        _buy(1 ether, abi.encode(alice));
        skip(EXTENSION + 1);
        clock.settle();

        vm.startPrank(alice);
        clock.claim();
        vm.expectRevert(ScryClock.NothingOwed.selector);
        clock.claim();
        vm.stopPrank();
    }

    function test_unlockCallback_is_only_for_the_pool_manager() public {
        vm.prank(carol);
        // BaseHook's error, inherited rather than redeclared — see ScryClock.sol.
        vm.expectRevert(BaseHook.NotPoolManager.selector);
        clock.unlockCallback(abi.encode(carol, 1 ether));
    }

    // ------------------------------------------------------------ no admin

    /// @dev R3. If this contract ever grows a setter, this test is where it
    ///      should be caught. The parameters are welded by construction.
    function test_no_parameter_can_move() public view {
        assertEq(clock.POT_BPS(), POT_BPS);
        assertEq(clock.EXTENSION(), EXTENSION);
        assertEq(clock.MAX_HORIZON(), MAX_HORIZON);
        assertEq(clock.ROLLOVER_BPS(), ROLLOVER_BPS);
        assertTrue(clock.BUY_ZERO_FOR_ONE());
        // There is deliberately no owner(), no setter, and nothing to assert
        // about them. Adding one should break the build, not this assertion.
    }

    /// Run the constructor AT an address carrying the right permission bits,
    /// and hand back what it reverted with.
    ///
    /// ⚠ This cannot be written as `vm.expectRevert(...); new ScryClock(...)`,
    /// and the reason is the finding: `BaseHook`'s constructor validates the
    /// address bits BEFORE our constructor body runs, so a `new` — which lands
    /// wherever CREATE puts it — reverts `HookAddressNotValid` whatever the
    /// parameters say. Written that way the test passes for the wrong reason
    /// and proves nothing about any parameter.
    function _constructAt(bytes memory args) internal returns (bool ok, bytes memory ret) {
        uint160 flags =
            uint160(Hooks.BEFORE_INITIALIZE_FLAG | Hooks.AFTER_SWAP_FLAG | Hooks.AFTER_SWAP_RETURNS_DELTA_FLAG);
        // The live hook's permission bits at a different address, so this
        // never etches over `clock`.
        address where = address(flags | (uint160(1) << 40));
        vm.etch(where, abi.encodePacked(vm.getCode("ScryClock.sol:ScryClock"), args));
        (ok, ret) = where.call("");
    }

    function _rejects(bytes memory args, string memory why) internal {
        (bool ok, bytes memory ret) = _constructAt(args);
        assertFalse(ok, why);
        assertEq(bytes4(ret), ScryClock.BadParameter.selector, "refused, but not for the parameter");
    }

    function test_constructor_rejects_unplayable_parameters() public {
        // A skim above 10% is a tax nobody trades through.
        _rejects(
            abi.encode(manager, currency1, true, uint256(1_001), EXTENSION, MAX_HORIZON, uint256(0)),
            "a skim above 10% must be refused"
        );

        // Seconds, not minutes, is the block-stuffing hole.
        _rejects(
            abi.encode(manager, currency1, true, POT_BPS, uint64(30), MAX_HORIZON, uint256(0)),
            "a sub-minute extension must be refused"
        );

        // A horizon shorter than one extension would bite on every ticket.
        _rejects(
            abi.encode(manager, currency1, true, POT_BPS, EXTENSION, EXTENSION - 1, uint256(0)),
            "a horizon under one extension must be refused"
        );
    }

    /// The other half of the same fact: the address check is real, and a hook
    /// deployed without the salt grind never reaches its own parameters.
    function test_a_wrong_address_is_refused_before_the_parameters_are_read() public {
        vm.expectRevert();
        new ScryClock(manager, currency1, true, POT_BPS, EXTENSION, MAX_HORIZON, ROLLOVER_BPS);
    }

    // ------------------------------------------------------------------ fuzz

    /// @dev The accounting invariant: the contract can never credit more than
    ///      the prize it actually holds.
    function testFuzz_never_credits_more_than_it_holds(uint96 a, uint96 b) public {
        a = uint96(bound(a, 0.001 ether, 10 ether));
        b = uint96(bound(b, 0.001 ether, 10 ether));

        _buy(int256(uint256(a)), abi.encode(alice));
        skip(1 minutes);
        _buy(int256(uint256(b)), abi.encode(bob));
        skip(EXTENSION + 1);
        clock.settle();

        uint256 held = manager.balanceOf(address(clock), currency1.toId());
        assertLe(clock.owedTotal() + clock.pot(), held, "credited more than held");
    }

    // ------------------------------------------------------------------ fork

    /// @dev ⚠ THE ONE THAT COUNTS. Unit tests with the local manager will not
    ///      prove the `_afterSwap` delta-return path against the real deployed
    ///      singleton. Run this before the hook holds a cent.
    ///      `forge test --match-test test_fork --fork-url rhchain`
    function test_fork_real_swap_through_the_real_singleton() public {
        vm.skip(true); // unskip once RPC + a funded fork account are wired
        // TODO: fork 4663, point at PoolManager 0x8366a39c…, open a small
        // MYRRH/OBOL pool through a mined ScryClock, swap, and assert the pot
        // grew by exactly POT_BPS of the MYRRH received.
    }

    // -------------------------------------------------------------- internals

    /// @param amountSpecified POSITIVE spends that much (exact input);
    ///        NEGATIVE receives that much (exact output). The sign is flipped
    ///        on the way into `SwapParams`, where v4's own convention is the
    ///        opposite — this reads the way a player would say it.
    function _buy(int256 amountSpecified, bytes memory hookData) internal {
        swapRouter.swap(
            key,
            SwapParams({
                zeroForOne: true,
                amountSpecified: -amountSpecified,
                sqrtPriceLimitX96: TickMath.MIN_SQRT_PRICE + 1
            }),
            PoolSwapTest.TestSettings({takeClaims: false, settleUsingBurn: false}),
            hookData
        );
    }

    function _sell(int256 amountSpecified, bytes memory hookData) internal {
        swapRouter.swap(
            key,
            SwapParams({
                zeroForOne: false,
                amountSpecified: -amountSpecified,
                sqrtPriceLimitX96: TickMath.MAX_SQRT_PRICE - 1
            }),
            PoolSwapTest.TestSettings({takeClaims: false, settleUsingBurn: false}),
            hookData
        );
    }
}
