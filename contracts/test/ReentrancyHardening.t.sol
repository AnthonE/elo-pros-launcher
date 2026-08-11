// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryGranary.sol";
import "../src/ScrySilo.sol";
import "../src/ScryGardener.sol";
import "../src/ScryGarden.sol";
import "../src/ScryJobBoard.sol";
import "../src/ScryReputation.sol";
import "../src/ScryInsurancePool.sol";
import "../src/IScryArbiter.sol";
import "../src/IERC20.sol";
import {MockToken} from "./MockToken.sol";

/// The §M6/§M7 sweep, finished (audit 2026-07-27).
///
/// `ReentrancyGuard.sol`'s own header states the rule this file enforces:
/// *"several of these contracts take their token as a CONSTRUCTOR ARGUMENT, so
/// 'the token has no callback' is a deployment-time assumption a future deployer
/// cannot see."* That argument was applied to `ScryBank.enter` and then not to
/// four functions that have exactly the same shape:
///
///   - `ScrySilo.reap` / `unseal`  — `_settle` advanced its checkpoint AFTER
///     `granary.mint`, so a reentrant call recomputed the same accrual from a
///     stale `rewardDebt` and minted it twice
///   - `ScrySilo.claimOwed`        — read-mint-write across the same call
///   - `ScryGardener.claimLocked`  — the same, on the cliff balance
///
/// plus two consistency gaps: `ScryJobBoard.complete` was the one state-changing
/// entry point on the board without a guard (safe, but only because two lines
/// stayed in order), and `ScryGarden.addLiquidity` minted SEED before pulling
/// either side.
///
/// **None of these was live.** `SpoilsToken.mint` has no hook, so the chain
/// Silo/Gardener → `Granary.mint` → `SpoilsToken.mint` cannot call back today.
/// The point of the mocks below is to build the deployment that CAN — a
/// SpoilsToken-shaped coin that notifies its recipient, the ERC-777 shape — and
/// prove the guards rather than assert them. That is the difference between a
/// contract that is safe and a contract that is safe *for a reason you tested*.
///
/// Blast radius if one had gone live, so the severity is not overstated:
/// `ScryGranary.mint` debits `mintedToday` BEFORE calling the token, so a
/// reentrancy loop correctly consumes the daily budget and clamps out. The loss
/// was bounded at the grantee's remaining daily cap (RUNBOOK.md §0: gardener
/// 500/day, silo 250/day), and honest stakers were delayed rather than
/// deprived — a clamped settle carries the shortfall. Bounded, visible in
/// `GrantMint`, and a permanent leak while it lasted.

// ═══════════════════════════════════════════════════════════════════════════
// THE MOCKS — the deployment the guard header warns about
// ═══════════════════════════════════════════════════════════════════════════

/// A `SpoilsToken`-shaped coin that NOTIFIES ITS RECIPIENT on mint. Matches the
/// ABI `ScryGranary` actually uses (`mint`, `setMinter`), so it can be cast to
/// `SpoilsToken` and handed to a real granary. One-shot, and armed for a single
/// address, so the harness itself is never hooked.
contract HookSpoils {
    string public constant name = "hook myrrh";
    string public constant symbol = "hMYRRH";
    uint8 public constant decimals = 18;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    address public minter;
    address public armedFor;

    event Transfer(address indexed from, address indexed to, uint256 value);

    constructor(address _minter) {
        minter = _minter;
    }

    /// Arm the hook for one address. Cleared by the first firing.
    function arm(address who) external {
        armedFor = who;
    }

    function setMinter(address next) external {
        require(msg.sender == minter, "minter only");
        minter = next;
    }

    function mint(address to, uint256 amount) external {
        require(msg.sender == minter, "minter only");
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
        if (to == armedFor && to.code.length > 0) {
            armedFor = address(0); // one shot: proving a guard, not looping
            (bool ok,) = to.call(abi.encodeWithSignature("onSpoilsReceived()"));
            ok; // the recipient's own failure is not the token's business
        }
    }

    function transfer(address to, uint256 value) external returns (bool) {
        require(balanceOf[msg.sender] >= value, "balance");
        balanceOf[msg.sender] -= value;
        balanceOf[to] += value;
        emit Transfer(msg.sender, to, value);
        return true;
    }
}

/// A plain ERC-20 that notifies the PAYER during `transferFrom` — the shape a
/// pull-based contract meets. Used against the job board (whose `_tryPull`
/// pulls from the buyer) and the Garden (whose `addLiquidity` pulls from the
/// minter). Armed for one address, one shot.
contract HookToken {
    string public constant name = "hook coin";
    string public constant symbol = "hCOIN";
    uint8 public constant decimals = 18;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    address public armedFor;

    event Transfer(address indexed from, address indexed to, uint256 value);

    function arm(address who) external {
        armedFor = who;
    }

    function mint(address to, uint256 value) external {
        totalSupply += value;
        balanceOf[to] += value;
        emit Transfer(address(0), to, value);
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        return true;
    }

    function transfer(address to, uint256 value) external returns (bool) {
        _move(msg.sender, to, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= value, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - value;
        _move(from, to, value);
        if (from == armedFor && from.code.length > 0) {
            armedFor = address(0);
            (bool ok,) = from.call(abi.encodeWithSignature("onTokenPull()"));
            ok;
        }
        return true;
    }

    function _move(address from, address to, uint256 value) internal {
        require(balanceOf[from] >= value, "balance");
        balanceOf[from] -= value;
        balanceOf[to] += value;
        emit Transfer(from, to, value);
    }
}

/// Shared bookkeeping for the four attackers. `didReenter` exists so a green
/// assertion cannot be mistaken for a hook that never fired — the vacuity check
/// every one of these tests needs first.
abstract contract Reenterer {
    bool public didReenter;
    bool public reentryReverted;
    string public revertReason;

    /// Fire the reentrant call and record what came back. A low-level call
    /// rather than try/catch so the Error(string) payload can be read: WHICH
    /// guard refuses the call is the whole point of these fixes, and a bare
    /// "it reverted" would pass just as well against the old ordering.
    function _fire(address target, bytes memory data) internal {
        didReenter = true;
        (bool ok, bytes memory ret) = target.call(data);
        reentryReverted = !ok;
        revertReason = _reason(ret);
    }

    function _reason(bytes memory ret) internal pure returns (string memory) {
        if (ret.length < 68) return "<no reason>";
        assembly {
            ret := add(ret, 0x04)
        }
        return abi.decode(ret, (string));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SILO — reap and claimOwed
// ═══════════════════════════════════════════════════════════════════════════

contract SiloReenterer is Reenterer {
    ScrySilo public silo;
    SpoilsToken public sealToken;
    uint256 public sealId;
    uint8 public mode; // 0 off · 1 reap · 2 claimOwed

    constructor(ScrySilo s, SpoilsToken t) {
        silo = s;
        sealToken = t;
    }

    function doSeal(uint256 tier, uint256 amount) external {
        sealToken.approve(address(silo), type(uint256).max);
        sealId = silo.seal(0, tier, amount);
    }

    function arm(uint8 m) external {
        mode = m;
    }

    function doReap() external {
        silo.reap(sealId);
    }

    function doUnseal() external {
        silo.unseal(sealId);
    }

    function doClaimOwed() external {
        silo.claimOwed();
    }

    function onSpoilsReceived() external {
        uint8 m = mode;
        if (m == 0) return;
        mode = 0;
        if (m == 1) _fire(address(silo), abi.encodeCall(ScrySilo.reap, (sealId)));
        else _fire(address(silo), abi.encodeCall(ScrySilo.claimOwed, ()));
    }
}

contract SiloReentrancyTest is Test {
    SpoilsToken obol;
    HookSpoils myrrh;
    ScryGranary granary;
    ScrySilo silo;
    SiloReenterer attacker;

    uint256 constant RPS = 1e18; // 1 MYRRH/sec
    uint256 constant T_LONG = 0; // 7 days, 1x
    uint256 constant T_SHORT = 1; // 100 seconds, 1x

    function setUp() public {
        obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        myrrh = new HookSpoils(address(this));
        // The granary's coin is typed `SpoilsToken` but resolved at runtime by
        // address — which IS the deployment risk, so the harness builds it.
        granary = new ScryGranary(SpoilsToken(address(myrrh)));
        myrrh.setMinter(address(granary));
        silo = new ScrySilo(granary, RPS, 5000);
        silo.addTier(7 days, 10_000);
        silo.addTier(100, 10_000);
        silo.addBin(obol, 100);

        attacker = new SiloReenterer(silo, obol);
        obol.mint(address(attacker), 1000e18);
    }

    /// `reap` → `_settle` → `granary.mint` → the coin notifies the staker → the
    /// staker reaps again. Before the fix `_settle`'s checkpoint was written
    /// after the mint, so the second pass recomputed the SAME 100 MYRRH from a
    /// stale `rewardDebt` and minted it: 200 out for 100 earned.
    function test_reap_cannot_be_reentered_through_a_hook_bearing_coin() public {
        granary.setGrant(address(silo), 1_000e18);
        attacker.doSeal(T_LONG, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 seconds × 1 MYRRH/s

        myrrh.arm(address(attacker));
        attacker.arm(1);
        attacker.doReap();

        assertTrue(attacker.didReenter(), "the hook never fired - the test proves nothing");
        assertTrue(attacker.reentryReverted(), "the reentrant reap was NOT refused");
        assertEq(attacker.revertReason(), "reentrant", "refused by the guard, not by luck");
        assertEq(myrrh.balanceOf(address(attacker)), 100e18, "the accrual was paid exactly once");
    }

    /// The same, on the balance-claimer. A clamped `unseal` leaves `owedOf`
    /// standing; `claimOwed` read it, minted, then wrote — so a reentrant claim
    /// saw the whole balance a second time.
    function test_claimOwed_cannot_be_reentered_through_a_hook_bearing_coin() public {
        // a cap tight enough that the unseal clamps and leaves an owed balance
        granary.setGrant(address(silo), 10e18);
        attacker.doSeal(T_SHORT, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // matured, 100 MYRRH accrued

        attacker.doUnseal();
        assertEq(silo.owedOf(address(attacker)), 90e18, "the clamp left 90 owing");
        assertEq(myrrh.balanceOf(address(attacker)), 10e18, "and paid the day's room");

        granary.setGrant(address(silo), 1_000e18); // budget returns
        myrrh.arm(address(attacker));
        attacker.arm(2);
        attacker.doClaimOwed();

        assertTrue(attacker.didReenter(), "the hook never fired - the test proves nothing");
        assertTrue(attacker.reentryReverted(), "the reentrant claim was NOT refused");
        assertEq(attacker.revertReason(), "reentrant", "refused by the guard, not by luck");
        assertEq(myrrh.balanceOf(address(attacker)), 100e18, "100 earned, 100 paid, no more");
        assertEq(silo.owedOf(address(attacker)), 0, "and the owed balance is discharged");
    }

    /// The arithmetic half, independent of the guard: `_settle` now advances the
    /// checkpoint BEFORE the external call, so a reentrant settle would compute
    /// `due == 0` even if the modifier were removed. Pinned by observing that a
    /// second reap in the same second pays nothing.
    function test_a_second_settle_in_the_same_second_pays_nothing() public {
        granary.setGrant(address(silo), 1_000e18);
        attacker.doSeal(T_LONG, 100e18);
        vm.warp(vm.getBlockTimestamp() + 100);

        attacker.doReap();
        assertEq(myrrh.balanceOf(address(attacker)), 100e18);
        attacker.doReap(); // same block, no fresh accrual
        assertEq(myrrh.balanceOf(address(attacker)), 100e18, "the checkpoint held");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GARDENER — claimLocked
// ═══════════════════════════════════════════════════════════════════════════

contract GardenerReenterer is Reenterer {
    ScryGardener public gardener;
    MockToken public lp;
    bool public armed;

    constructor(ScryGardener g, MockToken l) {
        gardener = g;
        lp = l;
    }

    function doStake(uint256 amount) external {
        lp.approve(address(gardener), type(uint256).max);
        gardener.deposit(0, amount);
    }

    function doSettle() external {
        gardener.deposit(0, 0);
    }

    function arm() external {
        armed = true;
    }

    function doClaimLocked() external {
        gardener.claimLocked();
    }

    function onSpoilsReceived() external {
        if (!armed) return;
        armed = false;
        _fire(address(gardener), abi.encodeCall(ScryGardener.claimLocked, ()));
    }
}

contract GardenerReentrancyTest is Test {
    HookSpoils myrrh;
    MockToken lp;
    ScryGranary granary;
    ScryGardener gardener;
    GardenerReenterer attacker;

    uint256 constant RPS = 1e18;
    uint256 constant LOCK_BPS = 6700;
    uint256 unlockAt;

    function setUp() public {
        myrrh = new HookSpoils(address(this));
        lp = new MockToken("lp", "SEED");
        granary = new ScryGranary(SpoilsToken(address(myrrh)));
        myrrh.setMinter(address(granary));
        unlockAt = vm.getBlockTimestamp() + 90 days;
        gardener = new ScryGardener(granary, RPS, LOCK_BPS, unlockAt);
        granary.setGrant(address(gardener), 1_000e18);
        gardener.addPool(IERC20(address(lp)), 100);

        attacker = new GardenerReenterer(gardener, lp);
        lp.mint(address(attacker), 1000e18);
    }

    /// The cleanest of the four read-mint-write sites: `claimLocked` read the
    /// cliff balance, minted it, and only then zeroed it — so a reentrant claim
    /// took the whole locked balance a second time, bounded only by the
    /// granary's daily cap.
    function test_claimLocked_cannot_be_reentered_through_a_hook_bearing_coin() public {
        attacker.doStake(100e18);
        vm.warp(vm.getBlockTimestamp() + 100); // 100 MYRRH: 33 immediate, 67 locked
        attacker.doSettle();
        assertEq(myrrh.balanceOf(address(attacker)), 33e18, "the immediate share");
        assertEq(gardener.lockedOf(address(attacker)), 67e18, "the cliff share");

        vm.warp(unlockAt + 1); // the cliff opens
        myrrh.arm(address(attacker));
        attacker.arm();
        attacker.doClaimLocked();

        assertTrue(attacker.didReenter(), "the hook never fired - the test proves nothing");
        assertTrue(attacker.reentryReverted(), "the reentrant claim was NOT refused");
        assertEq(attacker.revertReason(), "reentrant", "refused by the guard, not by luck");
        assertEq(myrrh.balanceOf(address(attacker)), 100e18, "33 + 67, and not a wei more");
        assertEq(gardener.lockedOf(address(attacker)), 0, "the cliff balance is discharged");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// JOB BOARD — complete()
// ═══════════════════════════════════════════════════════════════════════════

contract BoardReenterer is Reenterer {
    ScryJobBoard public board;
    HookToken public coin;
    uint256 public jobId;
    bool public armed;

    constructor(ScryJobBoard b, HookToken c) {
        board = b;
        coin = c;
    }

    function doPost(address seller, uint256 amount, uint64 deadline) external {
        coin.approve(address(board), type(uint256).max);
        jobId = board.post(seller, amount, ScryJobBoard.Mode.ReputationOnly, keccak256("spec"), deadline, 0);
    }

    function arm() external {
        armed = true;
    }

    function doComplete() external {
        board.complete(jobId);
    }

    function onTokenPull() external {
        if (!armed) return;
        armed = false;
        _fire(address(board), abi.encodeCall(ScryJobBoard.complete, (jobId)));
    }
}

contract JobBoardReentrancyTest is Test {
    HookToken scry;
    ScryReputation rep;
    ScryInsurancePool pool;
    ScryJobBoard board;
    BoardReenterer buyer;
    address seller = address(0x5E11E4);
    address splitter = address(0x5911);

    function setUp() public {
        scry = new HookToken();
        rep = new ScryReputation(0); // threshold 0: everyone meets it
        pool = new ScryInsurancePool(IERC20(address(scry)), 100e18);
        board = new ScryJobBoard(
            IERC20(address(scry)), IReputation(address(rep)), IInsurancePool(address(pool)), splitter, IScryArbiter(address(0)), 1e18
        );
        rep.setAuthority(address(board), true);
        buyer = new BoardReenterer(board, scry);
        scry.mint(address(buyer), 1000e18);
    }

    /// `complete` was the only state-changing entry point on the board without
    /// `nonReentrant`. It was never exploitable — the status flip to `Closed`
    /// happens before `_payoutSeller` reaches a token, so the reentrant call
    /// bounced off the status guard. That is precisely what this test pins: the
    /// refusal now comes from the GUARD ("reentrant"), not from two lines
    /// happening to stay in the right order ("not delivered"). Red before the
    /// fix on the reason, green on the fact — which is the honest shape for a
    /// change that hardens a property rather than repairing a hole.
    function test_complete_is_refused_by_the_guard_not_by_the_status_ordering() public {
        uint64 deadline = uint64(vm.getBlockTimestamp() + 7 days);
        buyer.doPost(seller, 100e18, deadline);
        // Read the id BEFORE the prank: `vm.prank` arms exactly one call, and a
        // getter between the prank and the target consumes it — `deliver` then
        // runs as the test contract and reverts "not seller".
        uint256 id = buyer.jobId();
        vm.prank(seller);
        board.deliver(id, keccak256("done"));

        scry.arm(address(buyer));
        buyer.arm();
        buyer.doComplete();

        assertTrue(buyer.didReenter(), "the hook never fired - the test proves nothing");
        assertTrue(buyer.reentryReverted(), "the reentrant complete was NOT refused");
        assertEq(buyer.revertReason(), "reentrant", "the guard refuses it, not the status ordering");
        // and the job settled exactly once: 100 less 5% house fee
        assertEq(scry.balanceOf(seller), 95e18, "the seller was paid once");
        assertEq(scry.balanceOf(splitter), 5e18, "and the house took its cut once");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// GARDEN — pull before mint
// ═══════════════════════════════════════════════════════════════════════════

contract GardenObserver {
    ScryGarden public garden;
    HookToken public t0;
    MockToken public t1;
    uint256 public supplyDuringPull;
    bool public observed;

    constructor(ScryGarden g, HookToken a, MockToken b) {
        garden = g;
        t0 = a;
        t1 = b;
    }

    function addLiquidity(uint256 a, uint256 b) external {
        t0.approve(address(garden), type(uint256).max);
        t1.approve(address(garden), type(uint256).max);
        garden.addLiquidity(a, b, 0, block.timestamp + 1);
    }

    /// Fires mid-pull, which is the only moment the ordering is observable.
    function onTokenPull() external {
        observed = true;
        supplyDuringPull = garden.totalSupply();
    }
}

contract GardenOrderingTest is Test {
    HookToken t0;
    MockToken t1;
    ScryGarden garden;
    GardenObserver observer;

    function setUp() public {
        t0 = new HookToken();
        t1 = new MockToken("t1", "T1");
        garden = new ScryGarden(IERC20(address(t0)), IERC20(address(t1)));

        // seed the pool from the harness, unhooked
        t0.mint(address(this), 1000e18);
        t1.mint(address(this), 1000e18);
        t0.approve(address(garden), type(uint256).max);
        t1.approve(address(garden), type(uint256).max);
        garden.addLiquidity(100e18, 100e18, 0, block.timestamp + 1);

        observer = new GardenObserver(garden, t0, t1);
        t0.mint(address(observer), 100e18);
        t1.mint(address(observer), 100e18);
    }

    /// §M7 applied to the Garden. SEED used to be minted before either side was
    /// pulled, so mid-pull the shares already existed against a deposit that had
    /// not landed — and this contract's ERC-20 surface sits OUTSIDE
    /// `nonReentrant`, so they were movable. The pull now comes first, which is
    /// observable exactly here: `totalSupply` during the pull is the pre-add
    /// value, not the post-mint one.
    function test_seed_is_not_minted_before_the_deposit_lands() public {
        uint256 before = garden.totalSupply();
        t0.arm(address(observer));
        observer.addLiquidity(10e18, 10e18);

        assertTrue(observer.observed(), "the hook never fired - the test proves nothing");
        assertEq(observer.supplyDuringPull(), before, "SEED existed against an unlanded deposit");
        assertEq(garden.balanceOf(address(observer)), 10e18, "and the add still works");
        assertEq(garden.totalSupply(), before + 10e18, "supply moves after the pull, not before");
    }

    /// The seeding branch moved too — `MINIMUM_LIQUIDITY` is locked after the
    /// pulls now, not before. Same reserves, same shares, same lock.
    function test_the_opening_add_still_locks_minimum_liquidity() public {
        ScryGarden fresh = new ScryGarden(IERC20(address(t0)), IERC20(address(t1)));
        t0.approve(address(fresh), type(uint256).max);
        t1.approve(address(fresh), type(uint256).max);
        uint256 seeds = fresh.addLiquidity(100e18, 100e18, 0, block.timestamp + 1);

        assertEq(seeds, 100e18 - 1000, "sqrt(k) less the locked minimum");
        assertEq(fresh.balanceOf(address(fresh)), 1000, "locked forever at the pool itself");
        assertEq(fresh.totalSupply(), 100e18, "and the two sum to sqrt(k)");
        (uint112 r0, uint112 r1) = fresh.getReserves();
        assertEq(r0, 100e18);
        assertEq(r1, 100e18);
    }
}
