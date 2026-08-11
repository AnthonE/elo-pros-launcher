// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "./MockToken.sol";
import "../src/ScryGarden.sol";
import "../src/ScryBurrow.sol";
import "../src/ScryBank.sol";
import "../src/ScryGardener.sol";
import "../src/ScryGranary.sol";
import "../src/SpoilsToken.sol";
import "../src/SafeERC20.sol";

/// @title FeeOnTransfer — the pull that lies, and the guard that catches it
/// @notice `SafeERC20.safeTransferFrom` proves a token CALL succeeded. It cannot
///         prove the tokens ARRIVED. A fee-on-transfer token debits the sender
///         in full, credits the receiver less, and returns `true` — so a caller
///         that already credited the stated amount to its own books is now short,
///         silently, and the shortfall lands on whoever withdraws last.
///
///         Found while reading FWA Token Packs (`0x727C…3d74A`, Ethereum
///         mainnet, 2026-07-28), which carries an `IncorrectAmountReceived()`
///         error at the same position. We had no equivalent.
///
///         The sharp one is `ScryGarden.swap`: `out` is priced off the STATED
///         `amountIn` and paid before anything checks what landed, so a short
///         input buys a full output. `test_swap_was_a_drain_and_now_reverts`
///         puts a number on it.
contract FeeOnTransferTest is Test {
    FeeToken fee; // skims on every move
    MockToken plain; // honest control
    ScryGarden garden;
    ScryBurrow burrow;

    address alice = address(0xA11CE);
    address whale = address(0x5A1E);

    function setUp() public {
        fee = new FeeToken("fee token", "FEE", 100); // 1%
        plain = new MockToken("plain", "PLN");
        garden = new ScryGarden(IERC20(address(fee)), IERC20(address(plain)));
        burrow = new ScryBurrow(IERC20(address(fee)), IERC20(address(plain)), IGardenOracle(address(garden)));

        fee.mint(whale, 10_000e18);
        plain.mint(whale, 10_000e18);
        fee.mint(alice, 10_000e18);
        plain.mint(alice, 10_000e18);

        // Seed the garden while the guard is out of the way, so the pool the
        // tests trade against is real. `seedExempt` is a fixture lever on the
        // MOCK, not a production door — it exists so setUp can create the very
        // state the guard is meant to protect.
        fee.setExempt(address(garden), true);
        vm.startPrank(whale);
        fee.approve(address(garden), type(uint256).max);
        plain.approve(address(garden), type(uint256).max);
        garden.addLiquidity(2000e18, 2000e18, 0, type(uint256).max);
        plain.transfer(address(burrow), 1000e18);
        vm.stopPrank();
        fee.setExempt(address(garden), false);

        vm.startPrank(alice);
        fee.approve(address(garden), type(uint256).max);
        plain.approve(address(garden), type(uint256).max);
        fee.approve(address(burrow), type(uint256).max);
        plain.approve(address(burrow), type(uint256).max);
        vm.stopPrank();
    }

    // ── the drain ───────────────────────────────────────────────────────────

    /// The swap leg priced `out` off the stated `amountIn`. Against a 1% skim
    /// the pool paid for tokens it never received — free value, every call,
    /// repeatable. This asserts both halves: the size of what was extractable,
    /// and that the call now refuses.
    function test_swap_was_a_drain_and_now_reverts() public {
        uint256 amountIn = 100e18;
        uint256 quoted = garden.getAmountOut(amountIn, true);
        uint256 landed = amountIn - (amountIn * 100) / 10_000; // 1% skimmed
        uint256 honest = garden.getAmountOut(landed, true);

        // The pool would have paid the quote for the smaller deposit.
        assertGt(quoted, honest, "no gap means no drain to fix");
        emit log_named_uint("overpay per 100e18 swap (wei)", quoted - honest);

        vm.prank(alice);
        vm.expectRevert(bytes("inexact transfer"));
        garden.swap(amountIn, true, 0, type(uint256).max);
    }

    // ── the dilution ────────────────────────────────────────────────────────

    /// SEED is minted against the stated deposit, so a short deposit issues
    /// shares nobody funded and every existing holder pays for them.
    function test_addLiquidity_refuses_short_deposit() public {
        vm.prank(alice);
        vm.expectRevert(bytes("inexact transfer"));
        garden.addLiquidity(100e18, 100e18, 0, type(uint256).max);
    }

    // ── the lending book ────────────────────────────────────────────────────

    function test_burrow_deposit_refuses_short_collateral() public {
        vm.prank(alice);
        vm.expectRevert(bytes("inexact transfer"));
        burrow.depositCollateral(100e18);
    }

    /// The debt leg is the honest token here, so repay must still work — the
    /// guard has to catch the lying token without breaking the other one.
    function test_burrow_repay_unaffected_when_debt_token_is_honest() public {
        fee.setExempt(alice, true);
        vm.prank(alice);
        burrow.depositCollateral(500e18);
        fee.setExempt(alice, false);

        vm.startPrank(alice);
        burrow.borrow(100e18);
        uint256 owed = plain.balanceOf(alice);
        burrow.repay(50e18);
        vm.stopPrank();
        assertEq(plain.balanceOf(alice), owed - 50e18, "honest repay must land");
    }

    // ── the three books that priced BEFORE pulling (2026-07-28) ─────────────
    //
    // The original sweep caught `ScryGarden` and `ScryBurrow`, both of which
    // pull and then act. These three credit a STATED amount to their own books
    // and pull afterwards, so a short delivery mints unbacked accounting rather
    // than buying a free output — no extractable drain, but permanently wrong
    // books that dilute whoever withdraws last. Same guard, same message.

    /// `ScryBank.enter` prices shares off the stated `amount` two lines before
    /// the pull, so a 1% skim would credit the depositor a claim on tokens that
    /// never arrived and dilute every existing xSCRY holder.
    function test_bank_enter_refuses_short_deposit() public {
        ScryBank bank = new ScryBank(IERC20(address(fee)));
        vm.startPrank(alice);
        fee.approve(address(bank), type(uint256).max);
        vm.expectRevert(bytes("inexact transfer"));
        bank.enter(100e18);
        vm.stopPrank();
    }

    /// And it must still work for the honest token — the bank is deployed
    /// against canonical SCRY, which does not skim.
    function test_bank_enter_unaffected_by_an_honest_token() public {
        ScryBank bank = new ScryBank(IERC20(address(plain)));
        vm.startPrank(alice);
        plain.approve(address(bank), type(uint256).max);
        uint256 shares = bank.enter(100e18);
        vm.stopPrank();
        assertEq(shares, 100e18 - bank.MINIMUM_SHARES(), "honest deposit mints in full");
        assertEq(plain.balanceOf(address(bank)), 100e18, "and the pool holds it");
    }

    /// `ScryGardener.deposit` credits `u.amount` and `stakedSupply` the stated
    /// amount, so a short LP delivery farms a real emission stream against
    /// weight nothing backs.
    function test_gardener_deposit_refuses_short_lp() public {
        SpoilsToken reward = new SpoilsToken("reward", "RWD", 0, address(this));
        ScryGranary granary = new ScryGranary(reward);
        reward.setMinter(address(granary));
        ScryGardener gardener = new ScryGardener(granary, 1e18, 0, block.timestamp + 365 days);
        granary.setGrant(address(gardener), 1_000e18);
        gardener.addPool(IERC20(address(fee)), 100);

        vm.startPrank(alice);
        fee.approve(address(gardener), type(uint256).max);
        vm.expectRevert(bytes("inexact transfer"));
        gardener.deposit(0, 100e18);
        vm.stopPrank();
    }

    /// `addPool` itself must refuse an address with no code: `SafeERC20`'s
    /// tolerant call reads `ok && ret.length == 0` from an empty account as
    /// success, so a codeless "LP" would mint a phantom stake that farms real
    /// emissions. The reachable trigger is `SEED_SECONDARY`, an operator-typed
    /// raw address nothing else validates.
    function test_gardener_addPool_refuses_a_codeless_lp() public {
        SpoilsToken reward = new SpoilsToken("reward", "RWD", 0, address(this));
        ScryGranary granary = new ScryGranary(reward);
        ScryGardener gardener = new ScryGardener(granary, 1e18, 0, block.timestamp + 365 days);
        vm.expectRevert(bytes("lp has no code"));
        gardener.addPool(IERC20(address(0xDEADBEEF)), 100);
    }

    // `ScrySilo.seal` is deliberately NOT in this list. Its bins are typed
    // `SpoilsToken`, whose `_move` cannot skim, so the type is the guard —
    // the same call ScryOrchard makes and states in its own comment. A test
    // cannot even be written for it: `addBin` takes `SpoilsToken`, so a lying
    // token is unrepresentable at the type level. That is the point.

    // ── the guard must not fire on honest tokens ────────────────────────────

    /// Regression: the whole point is that nothing changes for a normal ERC-20.
    /// A garden of two honest tokens swaps and mints exactly as before.
    function test_plain_tokens_are_untouched() public {
        MockToken a = new MockToken("a", "A");
        MockToken b = new MockToken("b", "B");
        ScryGarden g = new ScryGarden(IERC20(address(a)), IERC20(address(b)));
        a.mint(alice, 1000e18);
        b.mint(alice, 1000e18);

        vm.startPrank(alice);
        a.approve(address(g), type(uint256).max);
        b.approve(address(g), type(uint256).max);
        g.addLiquidity(500e18, 500e18, 0, type(uint256).max);
        uint256 got = g.swap(10e18, true, 0, type(uint256).max);
        vm.stopPrank();
        assertGt(got, 0, "honest swap must pay out");
    }

    /// A token that credits the receiver MORE than asked is refused too. The
    /// accounting models one number; a transfer that quietly beats it is still
    /// a transfer this contract cannot reason about.
    function test_over_delivery_is_refused() public {
        BonusToken bonus = new BonusToken();
        MockToken other = new MockToken("other", "OTH");
        ScryGarden g = new ScryGarden(IERC20(address(bonus)), IERC20(address(other)));
        bonus.mint(alice, 1000e18);
        other.mint(alice, 1000e18);

        vm.startPrank(alice);
        bonus.approve(address(g), type(uint256).max);
        other.approve(address(g), type(uint256).max);
        vm.expectRevert(bytes("inexact transfer"));
        g.addLiquidity(100e18, 100e18, 0, type(uint256).max);
        vm.stopPrank();
    }

    /// A token that cannot report a balance cannot be measured. `pullExact`
    /// has no tolerant case for that and must say so rather than assume zero.
    function test_unmeasurable_token_is_refused() public {
        MuteToken mute = new MuteToken();
        MockToken other = new MockToken("other", "OTH");
        ScryGarden g = new ScryGarden(IERC20(address(mute)), IERC20(address(other)));
        other.mint(alice, 1000e18);

        vm.startPrank(alice);
        other.approve(address(g), type(uint256).max);
        vm.expectRevert(bytes("balanceOf failed"));
        g.addLiquidity(100e18, 100e18, 0, type(uint256).max);
        vm.stopPrank();
    }
}

/// @notice ERC-20 that burns `feeBps` of every move. TEST-ONLY. `exempt`
///         suppresses the skim for one receiver so a fixture can build the
///         pool state the guard is meant to defend.
contract FeeToken {
    string public name;
    string public symbol;
    uint8 public constant decimals = 18;
    uint256 public totalSupply;
    uint256 public feeBps;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    mapping(address => bool) public exempt;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    constructor(string memory n, string memory s, uint256 bps) {
        name = n;
        symbol = s;
        feeBps = bps;
    }

    function setExempt(address who, bool on) external {
        exempt[who] = on;
    }

    function mint(address to, uint256 v) public {
        totalSupply += v;
        balanceOf[to] += v;
        emit Transfer(address(0), to, v);
    }

    function transfer(address to, uint256 v) external returns (bool) {
        _move(msg.sender, to, v);
        return true;
    }

    function transferFrom(address from, address to, uint256 v) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= v, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - v;
        _move(from, to, v);
        return true;
    }

    function approve(address s, uint256 v) external returns (bool) {
        allowance[msg.sender][s] = v;
        emit Approval(msg.sender, s, v);
        return true;
    }

    function _move(address from, address to, uint256 v) internal {
        require(balanceOf[from] >= v, "balance");
        uint256 cut = (exempt[to] || exempt[from]) ? 0 : (v * feeBps) / 10_000;
        unchecked {
            balanceOf[from] -= v;
            balanceOf[to] += v - cut;
            totalSupply -= cut;
        }
        emit Transfer(from, to, v - cut);
    }
}

/// @notice ERC-20 that credits the receiver one wei MORE than asked. TEST-ONLY.
contract BonusToken {
    string public constant name = "bonus";
    string public constant symbol = "BON";
    uint8 public constant decimals = 18;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);

    function mint(address to, uint256 v) public {
        totalSupply += v;
        balanceOf[to] += v;
        emit Transfer(address(0), to, v);
    }

    function transfer(address to, uint256 v) external returns (bool) {
        _move(msg.sender, to, v);
        return true;
    }

    function transferFrom(address from, address to, uint256 v) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= v, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - v;
        _move(from, to, v);
        return true;
    }

    function approve(address s, uint256 v) external returns (bool) {
        allowance[msg.sender][s] = v;
        return true;
    }

    function _move(address from, address to, uint256 v) internal {
        require(balanceOf[from] >= v, "balance");
        unchecked {
            balanceOf[from] -= v;
            balanceOf[to] += v + 1; // the lie
            totalSupply += 1;
        }
        emit Transfer(from, to, v);
    }
}

/// @notice ERC-20 whose `balanceOf` reverts — unmeasurable. TEST-ONLY.
contract MuteToken {
    mapping(address => mapping(address => uint256)) public allowance;

    function balanceOf(address) external pure returns (uint256) {
        revert("no");
    }

    function transferFrom(address, address, uint256) external pure returns (bool) {
        return true;
    }

    function approve(address s, uint256 v) external returns (bool) {
        allowance[msg.sender][s] = v;
        return true;
    }
}
