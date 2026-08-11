// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "../src/IERC20.sol";
import {ScryHarvest} from "../src/ScryHarvest.sol";
import {ScryBank} from "../src/ScryBank.sol";
import {SpoilsToken} from "../src/SpoilsToken.sol";

/// The payments pass, §M4–§M7. Each test states the property, and each was run
/// against the code BEFORE the fix and confirmed to fail there — a law that
/// passes on the broken contract is a comment, not a law.
contract PaymentsPassTest is Test {
    // ─────────────────────────────────────────────────────────────────────────
    // §M5 · "token-generic; nothing below assumes a particular ERC-20" was
    //       false. Every fund mover did `require(token.transfer(...))` against
    //       an interface declaring a bool return, so a token that returns
    //       NOTHING (the USDT shape) reverted on the ABI decode — the transfer
    //       succeeded and the contract said it failed.
    // ─────────────────────────────────────────────────────────────────────────
    function test_M5_a_no_return_token_can_be_claimed() public {
        NoReturnToken t = new NoReturnToken();
        ScryHarvest h = new ScryHarvest(IERC20(address(t)));
        t.mint(address(h), 150e18);

        address a = address(0xA);
        bytes32 leaf = keccak256(abi.encodePacked(a, uint256(100e18)));
        h.postRoot(leaf, 100e18, "ledger/usdt-shaped");

        bytes32[] memory proof = new bytes32[](0);
        h.claim(a, 100e18, proof);
        assertEq(t.balanceOf(a), 100e18, "M5: a no-return token pays out");
        assertEq(h.owedUnderRoot(), 0, "M5: and the obligation is discharged");
    }

    function test_M5_a_token_that_returns_false_still_reverts() public {
        FalseToken t = new FalseToken();
        ScryHarvest h = new ScryHarvest(IERC20(address(t)));
        t.mint(address(h), 150e18);
        address a = address(0xA);
        bytes32 leaf = keccak256(abi.encodePacked(a, uint256(100e18)));
        h.postRoot(leaf, 100e18, "ledger/liar");
        bytes32[] memory proof = new bytes32[](0);
        vm.expectRevert(bytes("transfer failed"));
        h.claim(a, 100e18, proof);
    }

    function test_M5_the_tokens_own_revert_reason_survives() public {
        SpoilsToken obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        ScryHarvest h = new ScryHarvest(IERC20(address(obol)));
        obol.mint(address(h), 1e18); // funded for 1, root promises 100
        address a = address(0xA);
        bytes32 leaf = keccak256(abi.encodePacked(a, uint256(100e18)));
        h.postRoot(leaf, 100e18, "ledger/underfunded");
        bytes32[] memory proof = new bytes32[](0);
        // NOT "transfer failed": a low-level call swallows the callee's reason
        // unless you hand it back, and losing SpoilsToken's own "balance" for a
        // flat "transfer failed" would make every failure look alike.
        vm.expectRevert(bytes("balance"));
        h.claim(a, 100e18, proof);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // §M4 · `postRoot` takes `totalCommitted_` from the operator and cannot
    //       check it against the tree. Post a real root with a total of 0,
    //       sweep, and every claimant's claim reverts on an empty contract.
    // ─────────────────────────────────────────────────────────────────────────
    function test_M4_retracting_an_obligation_cannot_drain_in_the_same_breath() public {
        SpoilsToken obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        ScryHarvest h = new ScryHarvest(IERC20(address(obol)));
        obol.mint(address(h), 100e18);

        address a = address(0xA);
        bytes32 leaf = keccak256(abi.encodePacked(a, uint256(100e18)));
        h.postRoot(leaf, 100e18, "ledger/honest");
        assertEq(h.sweepFloor(), 100e18, "M4: the honest obligation is the floor");

        // the move: re-post the SAME real root with a total of zero.
        h.postRoot(leaf, 0, "ledger/zeroed");
        assertEq(h.owedUnderRoot(), 0, "M4: the stated obligation really is zero");
        assertEq(h.sweepFloor(), 100e18, "M4: but the floor still holds the old one");
        vm.expectRevert(bytes("would break the posted root"));
        h.sweep(address(this), 100e18);

        // …and inside the window the claimant is still paid in full.
        bytes32[] memory proof = new bytes32[](0);
        h.claim(a, 100e18, proof);
        assertEq(obol.balanceOf(a), 100e18, "M4: the retracted claim is still payable");
    }

    /// A claim does NOT release the carried floor early, and that is the fix
    /// rather than the defect. `claim` used to draw `priorOwed` down by any
    /// payment under the CURRENT root — but `priorOwed` carries what the
    /// PREVIOUS root owed, and the contract cannot know whether the claimant
    /// was in both. See the honest-error case pinned in
    /// test_a_claim_under_the_new_root_does_not_discharge_the_old_one below.
    /// The cost of the conservative direction is bounded: surplus waits out
    /// SWEEP_DELAY instead of releasing the moment someone claims.
    function test_M4_the_window_is_finite_and_holds_until_it_expires() public {
        SpoilsToken obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        ScryHarvest h = new ScryHarvest(IERC20(address(obol)));
        obol.mint(address(h), 100e18);
        address a = address(0xA);
        bytes32 leaf = keccak256(abi.encodePacked(a, uint256(60e18)));
        h.postRoot(leaf, 60e18, "ledger/1");
        h.postRoot(leaf, 0, "ledger/2"); // obligation retracted
        assertEq(h.sweepFloor(), 60e18, "the window holds the previous obligation");

        bytes32[] memory proof = new bytes32[](0);
        h.claim(a, 60e18, proof);
        assertEq(h.priorOwed(), 60e18, "a claim never walks the carried floor down");
        assertEq(h.sweepFloor(), 60e18, "so the floor stands for the rest of the window");

        // 40e18 of real surplus is left, and it is held — conservative in the
        // one safe direction (ScryHarvest.sol: over-stating means sweep LESS).
        vm.expectRevert(bytes("would break the posted root"));
        h.sweep(address(this), 40e18);

        // the window releases it, on its own, on schedule.
        vm.warp(vm.getBlockTimestamp() + 7 days + 1);
        assertEq(h.sweepFloor(), 0, "released after SWEEP_DELAY: a delay, not a lock");
        h.sweep(address(this), 40e18);
        assertEq(obol.balanceOf(address(h)), 0, "surplus sweeps once the window closes");

        // and the window expires on its own for the un-claimed case.
        ScryHarvest h2 = new ScryHarvest(IERC20(address(obol)));
        obol.mint(address(h2), 10e18);
        h2.postRoot(leaf, 10e18, "ledger/1");
        h2.postRoot(leaf, 0, "ledger/2");
        assertEq(h2.sweepFloor(), 10e18, "held during the window");
        vm.warp(vm.getBlockTimestamp() + 7 days + 1);
        assertEq(h2.sweepFloor(), 0, "and released after SWEEP_DELAY: a delay, not a lock");
    }

    /// The honest-error case that the `priorOwed` drawdown broke. Nobody is
    /// attacking: a ledger regeneration drops a wallet and adds a new one, and
    /// a legitimate claim by the NEW wallet used to discharge the OLD wallet's
    /// carried obligation — collapsing the very floor the window exists to
    /// hold, while the operator read `sweepFloor()` and believed it.
    /// RED before the 2026-07-28 fix (floor read 500e18 instead of 1500e18).
    function test_a_claim_under_the_new_root_does_not_discharge_the_old_one() public {
        SpoilsToken obol = new SpoilsToken("obol", "OBOL", 0, address(this));
        ScryHarvest h = new ScryHarvest(IERC20(address(obol)));
        obol.mint(address(h), 3_000e18);
        address alice = address(0xA11CE);
        address bob = address(0xB0B);
        bytes32[] memory proof = new bytes32[](0);

        // root one owes alice 1500
        h.postRoot(keccak256(abi.encodePacked(alice, uint256(1_500e18))), 1_500e18, "ledger/1");

        // the ledger is regenerated; a generator bug drops alice, bob is added
        h.postRoot(keccak256(abi.encodePacked(bob, uint256(1_000e18))), 1_000e18, "ledger/2");
        assertEq(h.priorOwed(), 1_500e18, "alice's obligation is carried");
        assertEq(h.sweepFloor(), 1_500e18, "and it is the floor");

        // bob claims his own, entirely legitimate, 1000
        assertEq(h.claim(bob, 1_000e18, proof), 1_000e18);

        // bob was never part of root one, so alice's floor must be untouched
        assertEq(h.priorOwed(), 1_500e18, "bob's claim discharges none of alice's");
        assertEq(h.sweepFloor(), 1_500e18, "the floor alice relies on still stands");

        // and the operator cannot sweep into it, which is the whole point
        uint256 bal = obol.balanceOf(address(h));
        vm.expectRevert(bytes("would break the posted root"));
        h.sweep(address(this), bal - 1_500e18 + 1);
        h.sweep(address(this), bal - 1_500e18); // the true surplus is free
    }

    // ─────────────────────────────────────────────────────────────────────────
    // §M6/§M7 · There was no reentrancy guard anywhere in src/. Sound, but
    //           conditionally on a token with no transfer callback — and the
    //           token is a CONSTRUCTOR ARGUMENT, so that condition is a
    //           deployment-time assumption a future deployer cannot see.
    //           ScryBank.enter() also minted shares BEFORE pulling the deposit.
    // ─────────────────────────────────────────────────────────────────────────
    function test_M7_a_callback_token_cannot_reenter_the_bank() public {
        HookToken t = new HookToken();
        ScryBank bank = new ScryBank(IERC20(address(t)));
        Reenterer evil = new Reenterer(bank, t);
        t.mint(address(evil), 1000e18);
        t.setHook(address(evil));

        // seed the bank honestly first, so the share price is well defined.
        t.mint(address(this), 1000e18);
        t.approve(address(bank), type(uint256).max);
        bank.enter(1000e18);

        evil.arm();
        vm.expectRevert(bytes("reentrant"));
        evil.attack(100e18);
        assertEq(bank.balanceOf(address(evil)), 0, "M7: no shares minted through the callback");
    }

    function test_M7_the_bank_pulls_before_it_mints() public {
        HookToken t = new HookToken();
        ScryBank bank = new ScryBank(IERC20(address(t)));
        Observer obs = new Observer(bank);
        t.mint(address(obs), 1000e18);
        t.setHook(address(obs));

        obs.enter(1000e18);
        // the hook fires DURING the pull; at that moment the deposit must
        // already be the bank's and no shares may exist for the depositor yet.
        assertEq(obs.sharesSeenDuringPull(), 0, "M7: shares are minted after the pull, not before");
        assertGt(bank.balanceOf(address(obs)), 0, "and they exist once it returns");
    }
}

// ── tokens shaped to break the old assumptions ──────────────────────────────

/// The USDT shape: succeeds, returns nothing at all.
contract NoReturnToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function transfer(address to, uint256 v) external {
        require(balanceOf[msg.sender] >= v, "bal");
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v;
    }

    function transferFrom(address f, address t, uint256 v) external {
        require(balanceOf[f] >= v, "bal");
        balanceOf[f] -= v;
        balanceOf[t] += v;
    }
}

/// Returns false instead of reverting — must still be treated as a failure.
contract FalseToken {
    mapping(address => uint256) public balanceOf;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function transfer(address, uint256) external pure returns (bool) {
        return false;
    }

    function transferFrom(address, address, uint256) external pure returns (bool) {
        return false;
    }
}

/// An ERC-777-ish token: calls a hook on the sender during transferFrom.
contract HookToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    address public hook;

    function mint(address to, uint256 v) external {
        balanceOf[to] += v;
    }

    function setHook(address h) external {
        hook = h;
    }

    function approve(address s, uint256 v) external returns (bool) {
        allowance[msg.sender][s] = v;
        return true;
    }

    function transfer(address to, uint256 v) external returns (bool) {
        require(balanceOf[msg.sender] >= v, "bal");
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v;
        return true;
    }

    function transferFrom(address f, address t, uint256 v) external returns (bool) {
        require(balanceOf[f] >= v, "bal");
        balanceOf[f] -= v;
        balanceOf[t] += v;
        if (hook != address(0) && f == hook) IHooked(hook).tokensReceived();
        return true;
    }
}

interface IHooked {
    function tokensReceived() external;
}

/// Reenters enter() from inside the token's transfer hook.
contract Reenterer {
    ScryBank public bank;
    HookToken public token;
    bool public armed;
    bool internal inside;

    constructor(ScryBank b, HookToken t) {
        bank = b;
        token = t;
        t.approve(address(b), type(uint256).max);
    }

    function arm() external {
        armed = true;
    }

    function attack(uint256 amount) external {
        bank.enter(amount);
    }

    function tokensReceived() external {
        if (armed && !inside) {
            inside = true;
            bank.enter(1e18); // the second bite
        }
    }
}

/// Watches the bank's state from inside the pull, without reentering.
contract Observer {
    ScryBank public bank;
    uint256 public sharesSeenDuringPull = type(uint256).max;

    constructor(ScryBank b) {
        bank = b;
    }

    function enter(uint256 amount) external {
        HookToken(address(bank.scry())).approve(address(bank), type(uint256).max);
        bank.enter(amount);
    }

    function tokensReceived() external {
        sharesSeenDuringPull = bank.balanceOf(address(this));
    }
}
