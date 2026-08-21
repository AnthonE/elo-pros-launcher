// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

/// @title EloBank — stake SCRY, receive xSCRY, game fees swell the pool
/// @notice The DFK-Jeweler / SushiBar pattern, unchanged because it is the
///         boring one that works: xSCRY is a pro-rata share of every SCRY
///         the bank holds. SERVICE fees (the job-board cut, stele-edition
///         mints, familiar labor — routed here by EloFeeSplitter) are plain
///         transfers into this contract, so the xSCRY→SCRY redemption rate
///         only ever ratchets up. No admin over the money: no pause, no
///         emission schedule, no APY promise, and nothing that can move a
///         deposit. The one owner door is `metadataOwner`, which reaches
///         name/symbol and nothing else — the yield is whatever the services
///         actually collect, which at current scale is small and we say so.
///
///         ⚠ SUPERSEDED, 2026-07-25 — do not restore. This header used to read:
///         "The GAMES run on game tokens (OBOL/MYRRH), so no game fee flows
///         here; the Bank is fed by service revenue, never a game buy-in or a
///         game rake (those are OBOL now)." The operator sentence "reserve can
///         enter" (2026-07-25, `SENTENCES.md`) REVERSED it: SCRY is taken on the
///         game surfaces as fees, entries and bonds, so a SCRY game rake CAN
///         reach this Bank through EloFeeSplitter. Corrected before broadcast
///         so the on-chain record does not contradict the live fee policy in a
///         place it can never be edited.
///
///         What the Bank is fed by is therefore whatever the posted split
///         sends it — and since 2026-07-26 that split also carries a BURN line
///         (EloFeeSplitter.BURN), so the Bank and the furnace are two outlets
///         of one number rather than competing designs. They are not
///         exclusive: the Bank accrues to those who STAKE, the burn to every
///         holder pro rata. Neither is deployed, so the weighting is a knob.
///
///         Economy rules, inherited (`TOKENOMICS.md`): fee inflows are
///         score-blind by construction (the splitter neither knows nor cares
///         about meter output), and the bank never touches meter revenue —
///         measurement USDG/USDC funds research infra, the bank sees service
///         fees only.
contract EloBank is ReentrancyGuard {
    string public name = "staked elo";
    string public symbol = "xELO";

    // ── metadata admin: name/symbol, and NOTHING else ──────────────────────
    // This is the only owner door in this contract. It cannot move funds,
    // pause anything, change a rate or touch what is escrowed — it reaches
    // exactly what a wallet prints. It exists because a constant here welds
    // whichever name was current on broadcast day in forever, and this
    // platform has been renamed more than once.
    address public metadataOwner;

    event MetadataOwnerTransferred(address indexed from, address indexed to);
    event MetadataUpdated(string name, string symbol);

    modifier onlyMetadataOwner() {
        require(msg.sender == metadataOwner, "not the metadata owner");
        _;
    }

    /// Pass address(0) to renounce and freeze name/symbol forever.
    function transferMetadataOwnership(address newOwner) external onlyMetadataOwner {
        emit MetadataOwnerTransferred(metadataOwner, newOwner);
        metadataOwner = newOwner;
    }

    function setName(string calldata newName) external onlyMetadataOwner {
        name = newName;
        emit MetadataUpdated(name, symbol);
    }

    function setSymbol(string calldata newSymbol) external onlyMetadataOwner {
        symbol = newSymbol;
        emit MetadataUpdated(name, symbol);
    }
    uint8 public constant decimals = 18;

    IERC20 public immutable reserve;

    /// First-deposit inflation guard (Uniswap-style): the first enter() locks
    /// this many shares to the bank itself, making donate-then-round-out
    /// attacks on later depositors uneconomic. Faucet-scale pots make the
    /// attack pointless anyway; the guard costs one storage write.
    uint256 public constant MINIMUM_SHARES = 1000;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    event Enter(address indexed who, uint256 reserveIn, uint256 sharesOut);
    event Leave(address indexed who, uint256 sharesIn, uint256 reserveOut);

    constructor(IERC20 _reserve) {
        reserve = _reserve;
        metadataOwner = msg.sender;
    }

    // ── the two moves ───────────────────────────────────────────────────────

    /// Stake `amount` SCRY, mint pro-rata xSCRY.
    /// @dev §M7. Shares used to be minted BEFORE the pull, which the comment
    ///      here called safe "ONLY because SCRY is a plain no-callback
    ///      ERC-20". That is the right disclosure and not an enforcement: the
    ///      token is a constructor argument, so a future deployment can pair
    ///      this with a hook-bearing one, and the callback fires with shares
    ///      already minted against a deposit that has not landed. Pull first,
    ///      then mint — the share price is read BEFORE the pull either way, so
    ///      the arithmetic is unchanged. `nonReentrant` makes the class safe
    ///      rather than just this instance.
    function enter(uint256 amount) external nonReentrant returns (uint256 shares) {
        require(amount > 0, "zero");
        uint256 pool = reserve.balanceOf(address(this));
        if (totalSupply == 0) {
            require(amount > MINIMUM_SHARES, "seed too small");
            shares = amount - MINIMUM_SHARES;
        } else {
            shares = (amount * totalSupply) / pool;
            require(shares > 0, "amount too small for pool");
        }
        // `pullExact`, not `safeTransferFrom`: shares are priced off the STATED
        // `amount` two lines up, so a token that delivers less than it is told
        // to would over-credit the depositor and dilute every existing xSCRY
        // holder by amount*pool*fee/(pool+amount). Nobody profits — the
        // shortfall goes to the token's own fee sink — but the books would be
        // wrong, silently, forever. This is the same class the 2026-07-28
        // sweep fixed in EloGarden; enter() priced before pulling and so was
        // missed by it. Refusing outright is correct: SCRY is not a
        // fee-on-transfer token, and a bank whose accounting cannot survive one
        // should say so at the door rather than mis-account past it.
        SafeERC20.pullExact(address(reserve), msg.sender, address(this), amount, "transferFrom failed");
        if (totalSupply == 0) _mint(address(this), MINIMUM_SHARES); // locked forever
        _mint(msg.sender, shares);
        emit Enter(msg.sender, amount, shares);
    }

    /// Burn `shares` xSCRY, withdraw the pro-rata slice of the pool.
    function leave(uint256 shares) external nonReentrant returns (uint256 amount) {
        require(shares > 0, "zero");
        // Before the first enter() this would be a bare division-by-zero panic
        // rather than a message. MINIMUM_SHARES is locked forever after the
        // seed, so totalSupply can never return to 0 once it is non-zero.
        require(totalSupply > 0, "bank is empty");
        amount = (shares * reserve.balanceOf(address(this))) / totalSupply;
        _burn(msg.sender, shares);
        SafeERC20.safeTransfer(address(reserve), msg.sender, amount, "transfer failed");
        emit Leave(msg.sender, shares, amount);
    }

    /// How much SCRY one full xSCRY redeems for right now (1e18-scaled).
    /// This number only goes up between fee inflows and stake/unstake — the
    /// whole flywheel on one view function.
    function redemptionRate() external view returns (uint256) {
        if (totalSupply == 0) return 1e18;
        return (reserve.balanceOf(address(this)) * 1e18) / totalSupply;
    }

    // ── minimal ERC-20 so xSCRY is composable/visible in wallets ────────────
    function transfer(address to, uint256 value) external returns (bool) {
        _move(msg.sender, to, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= value, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - value;
        _move(from, to, value);
        return true;
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        emit Approval(msg.sender, spender, value);
        return true;
    }

    function _move(address from, address to, uint256 value) internal {
        // Matches SpoilsToken._move, which is otherwise byte-identical to this
        // one. xSCRY sent to address(0) would leave `totalSupply` unchanged
        // while the shares became unredeemable, so the burn would be invisible
        // to `redemptionRate()` — a silent, permanent donation to every other
        // holder wearing the shape of a transfer. This contract welds with no
        // admin, so the guard goes in before the broadcast or not at all.
        require(to != address(0), "zero to");
        require(balanceOf[from] >= value, "balance");
        balanceOf[from] -= value;
        balanceOf[to] += value;
        emit Transfer(from, to, value);
    }

    function _mint(address to, uint256 value) internal {
        totalSupply += value;
        balanceOf[to] += value;
        emit Transfer(address(0), to, value);
    }

    function _burn(address from, uint256 value) internal {
        require(balanceOf[from] >= value, "balance");
        balanceOf[from] -= value;
        totalSupply -= value;
        emit Transfer(from, address(0), value);
    }
}
