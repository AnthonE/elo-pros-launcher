// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";
import {ISwapRouter} from "./EloLaunchpad.sol";

/// @title EloFeeConverter — a studio's pool fees, without the game-coin bag
/// @notice OPT-IN, AND NOT THE DEFAULT (demoted 2026-08-10 — `LAUNCHPAD.md`
///         §4b). The default for a game's fee coin is its OWN economy: a
///         `EloFeeSplitter` routing to `RESERVE_BURN`, its own `EloBank`, and the
///         studio. A studio is supposed to be LONG its own coin — buying it at
///         launch is the expected behaviour — so converting that leg by default
///         would have the platform selling a game's coin on the studio's behalf,
///         which is the sell pressure this was meant to avoid. This contract is
///         for the studio that genuinely wants paying in the reserve, and for
///         nobody else.
///
///         A launch vault's `coinFeeRecipient` is an arbitrary immutable
///         address; point it here instead of at a splitter and the fee coin
///         becomes SCRY on the way to the studio.
///
///         WHAT IT IS FOR. `EloLaunchpad.collectFees()` pays both legs of the v3
///         position — SCRY and the game's coin — to two immutable destinations.
///         This is one thing the COIN leg can be pointed at.
///
///         The shape is borrowed from pons V2, which pays creators in **ETH**
///         rather than the launched token so they are not handed small balances
///         of a memecoin they would sell. ⚠ That reasoning fits a MEMECOIN
///         CREATOR and does not automatically fit a game studio, which is why
///         this is an option rather than the rule. Where it does fit — a studio
///         that wants operating income in the reserve — the fee coin becomes
///         SCRY on the way through.
///
///         ⚠ IT DOES NOT REMOVE SELL PRESSURE, AND SAYING SO WOULD BE A LIE.
///         Converting fee coin into SCRY is still a sale of that coin into the
///         pool. What it removes is **discretion and inventory**: the sells are
///         small, continuous and visible instead of lumpy and timed, and the
///         studio never holds a position it could choose a moment to exit. That
///         is a real improvement and it is a smaller claim than "no dumping."
///
///         WHAT CAN LEAVE, AND WHERE IT CAN GO. Two paths and no others:
///           · the game's coin leaves ONLY through the router, as a swap into
///             SCRY that lands back here;
///           · SCRY leaves ONLY to `payee`, which is immutable.
///         There is no withdraw, no rescue and no sweep, for the same reason
///         the vault has none. A converter with a back door would just be the
///         wallet it replaces, wearing a contract.
///
///         ⚠ THIS MUST NEVER BE THE TICKET'S PROCEEDS. It handles FEES. Pointing
///         a ticket's `proceeds` here would route the RAISE through a contract
///         that pays it out to a person, which is precisely what `EloLaunchpad`
///         exists to make impossible.
contract EloFeeConverter is ReentrancyGuard {
    string public constant NOTICE = "converts a launch position's fee income into the reserve before paying it out, so a "
        "studio never holds a bag of its own game's coin - the coin leaves only as a swap, the reserve leaves only to the "
        "immutable payee, and nothing else leaves at all";

    IERC20 public immutable reserve;
    IERC20 public immutable coin;
    ISwapRouter public immutable router;
    uint24 public immutable convertFee;
    /// Where the SCRY goes. Immutable, so `pay()` can be permissionless: there
    /// is nothing for a caller to aim.
    address public immutable payee;

    /// May choose the swap's floor. May not withdraw — there is no withdraw.
    address public steward;
    address public pendingSteward;

    uint256 public totalConverted; // coin in
    uint256 public totalPaid; // reserve out

    event Converted(uint256 coinIn, uint256 reserveOut);
    event Paid(address indexed payee, uint256 amount);
    event StewardOffered(address indexed from, address indexed to);
    event StewardTransferred(address indexed from, address indexed to);

    modifier onlySteward() {
        require(msg.sender == steward, "not steward");
        _;
    }

    constructor(IERC20 _reserve, IERC20 _coin, ISwapRouter _router, uint24 _convertFee, address _payee, address _steward) {
        require(address(_reserve) != address(0) && address(_coin) != address(0), "zero token");
        // ⚠ ROLE, NOT TICKER. This refusal is about the RESERVE — the asset the
        // converter pays out in — and it outlives whatever the reserve is
        // called. The SCRY -> ELO sweep wrote a ticker into it; the same sweep
        // is why `_reserve` beside it is not named `_elo` either.
        require(address(_reserve) != address(_coin), "reserve cannot be the coin");
        require(address(_router) != address(0), "zero router");
        require(_payee != address(0), "zero payee");
        require(_steward != address(0), "zero steward");
        reserve = _reserve;
        coin = _coin;
        router = _router;
        convertFee = _convertFee;
        payee = _payee;
        steward = _steward;
    }

    /// Swap the fee coin held here into SCRY.
    ///
    /// Steward-gated for the same reason `EloLaunchpad.convert` is: a swap needs
    /// a floor somebody chose, and a permissionless swap taking a
    /// caller-supplied `minOut` is a permissionless sandwich. The steward
    /// cannot profit by calling it — the output lands here and here pays only
    /// the immutable payee.
    function convert(uint256 amountIn, uint256 minEloOut, uint256 deadline)
        external
        onlySteward
        nonReentrant
        returns (uint256 reserveOut)
    {
        require(amountIn > 0, "nothing in");
        require(minEloOut > 0, "name a floor");
        require(coin.balanceOf(address(this)) >= amountIn, "not that much coin held");

        SafeERC20.safeApprove(address(coin), address(router), amountIn, "approve failed");
        reserveOut = router.exactInputSingle(
            ISwapRouter.ExactInputSingleParams({
                tokenIn: address(coin),
                tokenOut: address(reserve),
                fee: convertFee,
                recipient: address(this),
                deadline: deadline,
                amountIn: amountIn,
                amountOutMinimum: minEloOut,
                sqrtPriceLimitX96: 0
            })
        );
        SafeERC20.safeApprove(address(coin), address(router), 0, "approve reset failed");
        totalConverted += amountIn;
        emit Converted(amountIn, reserveOut);
    }

    /// Push all held SCRY to the payee. Permissionless — the destination is
    /// immutable, so a stranger calling this can only do the studio a favour.
    function pay() external nonReentrant returns (uint256 amount) {
        amount = reserve.balanceOf(address(this));
        require(amount > 0, "nothing to pay");
        totalPaid += amount;
        SafeERC20.safeTransfer(address(reserve), payee, amount, "pay failed");
        emit Paid(payee, amount);
    }

    function transferSteward(address to) external onlySteward {
        if (to == address(0)) {
            // Renouncing leaves `pay()` working forever and converting gone.
            // The failure mode is fee coin sitting here uncollected, which is
            // the studio's loss and nobody else's risk.
            emit StewardTransferred(steward, address(0));
            steward = address(0);
            pendingSteward = address(0);
            return;
        }
        pendingSteward = to;
        emit StewardOffered(steward, to);
    }

    function acceptSteward() external {
        require(msg.sender == pendingSteward, "not the pending steward");
        emit StewardTransferred(steward, msg.sender);
        steward = msg.sender;
        pendingSteward = address(0);
    }

    /// What is waiting. A card reads this rather than claiming a number.
    function pending() external view returns (uint256 coinHeld, uint256 reserveHeld) {
        return (coin.balanceOf(address(this)), reserve.balanceOf(address(this)));
    }
}
