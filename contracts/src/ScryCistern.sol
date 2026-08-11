// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

/// The seat surface a distribution needs, and nothing else. `MAX_ELECTION` is
/// read once at deploy to weld the election's width to the one this contract
/// unrolls, so the two can never drift.
interface IScrySeat {
    function ownerOf(uint256 tokenId) external view returns (address);
    function weightOf(uint256 tokenId) external view returns (uint32);
    function totalWeight() external view returns (uint256);
    function tierSetAt(uint256 tokenId) external view returns (uint64);
    function MAX_ELECTION() external view returns (uint256);
    function electionOf(uint256 tokenId) external view returns (uint16[3] memory trackId, uint16[3] memory weightBps);
}

/// @notice The minimal Uniswap v3 router surface a claim uses. Deliberately
///         inlined so the repo carries no Uniswap dependency, and deliberately
///         NOT treated as authority: the router address is a constructor
///         argument verified against the live chain at deploy time, exactly as
///         `IUniswapV3.sol` says of the periphery it mirrors.
interface ISwapRouter {
    struct ExactInputSingleParams {
        address tokenIn;
        address tokenOut;
        uint24 fee;
        address recipient;
        uint256 deadline;
        uint256 amountIn;
        uint256 amountOutMinimum;
        uint160 sqrtPriceLimitX96;
    }

    function exactInputSingle(ExactInputSingleParams calldata params) external payable returns (uint256 amountOut);
}

/// @title ScryCistern — the drop bar: fee revenue in, a public round out
/// @notice `SCRY-HIVE.md` §3g1 as bytecode, and the honest half of StonkBrokers'
///         Clock In. Revenue fills a cistern anyone can watch. When it reaches
///         the posted line, **any wallet** opens a round and is paid a tip for
///         the gas. Every activated seat then draws its share, weighted by
///         tier, in **the coins that seat elected**.
///
///         ⚠ WHAT THIS CONTRACT NEVER DOES, AND THE WHOLE DESIGN TURNS ON IT:
///         **it mints nothing.** Not SCRY, not a game coin, not a point. It
///         holds fee revenue that was already earned and hands it out. A seat's
///         elected coins are BOUGHT at market on the pools the house holds LP
///         in — which takes nothing from the game that issued the coin, needs
///         no permission from it, and is buy pressure on it. Their booster
///         works the same way and cannot mint AAPL either; the stock is simply
///         what their fee revenue is spent on. Ours is the game coins, and
///         `SCRY-HIVE.md` §3g1 is the correction that got that right.
///
///         ⚠⚠ INVARIANT 9 IS WHAT SHAPED THIS FILE. *"Never trade ahead of our
///         own emissions — the house issues OBOL and MYRRH and knows schedules
///         and knob changes before the market."* Gates is in-house and the
///         granaries' steward is still the deploy wallet, so **the house sets
///         the emission schedule of coins this contract buys.** A promise not
///         to exploit that is worth nothing, so it is closed by construction —
///         four properties, each of which is load-bearing and none of which is
///         conduct:
///
///         1. **The trigger is a posted number, never a decision.** `open()`
///            fires on `threshold`, which is public.
///         2. **`threshold` and every route move on a TIMELOCK** (`KNOB_DELAY`,
///            3 days — the same shape StonkBrokers use to migrate a VRNG
///            conductor). Without this the operator could lower the bar to make
///            a buy land at a chosen moment, which is choosing *when* by the
///            back door and is the exact hole the invariant names.
///         3. **Anyone may crank it, and the caller keeps a tip.** If only the
///            house can fire it, the timing is discretionary again no matter
///            what the bar says.
///         4. **The slippage bound belongs to the claimer**, who passes their
///            own `minOut`. Nobody sets another wallet's price.
///
///         **And it holds no key to anyone's money.** There is no `rescue()`,
///         no pause, no upgrade path, and no admin withdrawal — the operator's
///         entire power is proposing knobs that take three days to land. Value
///         that reaches this contract leaves by `open` → `claim`, or it
///         recycles into the next round. That is deliberate and it is the
///         sentence a rug screen is actually reading (`DEGEN.md` §1b).
///
///         **THE POT IS SCRY, and that is a routing fact rather than a
///         preference.** Every game coin pairs against SCRY and only there
///         (`GATES.md` §2.1), so a SCRY pot is one hop from any electable coin
///         and the hop lands on our own pool. It also makes the default
///         honest: a seat with no election on file is paid **SCRY**, needing no
///         route, no router and no counterparty — so this contract works on the
///         day it is deployed with no pools posted at all.
///
///         **A round is a snapshot, and NOTHING reaches backwards into one.**
///         Both of the round's terms are fixed at `open()`: `totalWeight`, so a
///         seat whose `tierSetAt` is later than `openedAt` sits that round out
///         and counts from the next; and `window`, so the operator's knob
///         decides the NEXT round rather than one already running. The first
///         stops a holder upgrading a larger share out of everybody else's; the
///         second stops the house cancelling one. Neither is conduct.
///
///         **Unclaimed value is never stranded and never swept.** After the
///         round's own window anyone calls `recycle`, and the remainder simply
///         becomes part of the next round's pot. It cannot leave to an
///         operator, because nothing here can.
///
///         ⚠ **Say no rate, ever.** This distributes what the fees actually
///         collected. No APY, no yield, no "seats earn" — the posted NEVER in
///         `SENTENCES.md`, and the same thing `ScryBank` says about itself.
contract ScryCistern is ReentrancyGuard {
    string public constant NOTICE = "fee revenue in, a public round out - anyone opens the round at the posted bar and "
        "keeps a tip; activated seats draw a tier-weighted share in the coins they elected, BOUGHT at market. "
        "Mints nothing, holds no admin withdrawal, and quotes no rate";

    /// Knob changes land after this. Long enough that nobody can time a buy
    /// with one, short enough to stay operable.
    uint256 public constant KNOB_DELAY = 3 days;

    /// Ceiling on the crank tip. It pays for gas and a little status; it is
    /// never a revenue share, and a fat-fingered tip cannot eat a round.
    uint16 public constant MAX_TIP_BPS = 500;

    IERC20 public immutable scry;
    IScrySeat public immutable seat;
    ISwapRouter public immutable router; // address(0) => every claim pays SCRY
    uint256 public immutable electionWidth; // welded to the seat's MAX_ELECTION

    address public operator;
    uint16 public tipBps;
    uint32 public roundWindow; // seconds a round stays claimable; copied into each round at open()
    uint256 public threshold; // the bar

    /// SCRY owed to open rounds. `pooled()` is the balance minus this, so a
    /// plain ERC-20 transfer into this contract IS a contribution — the fee
    /// splitter, the gacha's sink or any wallet can fill the bar with a
    /// transfer and no integration.
    uint256 public outstanding;

    struct Round {
        uint256 pot;
        uint256 paid;
        uint256 totalWeight;
        uint64 openedAt;
        /// ⚠ THE WINDOW IS SNAPSHOTTED, NOT READ LIVE, and that is the same rule
        /// `totalWeight` above obeys for the same reason. `claim` and `recycle`
        /// used to test `openedAt + roundWindow` against the CURRENT storage
        /// knob, so `setRoundWindow` — which carries no timelock, because it
        /// cannot time a buy — reached backwards into every round already open:
        /// set it to 1 and every pending claim reverts "round window closed" and
        /// every open round becomes recyclable in the same block. Nothing leaves
        /// the contract that way (nothing can), but an operator holding seats
        /// would be converting other holders' unclaimed shares into a pot they
        /// draw from, on demand and with no delay. It ran the other way too: a
        /// longer window re-opened rounds that had already closed.
        /// A round is a snapshot. The knob decides the NEXT one.
        uint32 window;
        bool recycled;
    }

    Round[] private _rounds;
    mapping(uint256 => mapping(uint256 => bool)) public claimed; // roundId => tokenId

    /// trackId => the coin that track pays, and its v3 fee tier.
    /// **Track 0 is always SCRY and can never be set** — it is the default a
    /// seat falls back to, so it must not be something an operator can repoint.
    mapping(uint16 => address) public coinOf;
    mapping(uint16 => uint24) public feeOf;

    struct Pending {
        uint256 value;
        address coin;
        uint24 fee;
        uint64 eta;
    }

    mapping(bytes32 => Pending) public pending; // keccak(kind, key) => proposal

    event Filled(address indexed from, uint256 amount); // informational; a bare transfer emits none
    event Opened(uint256 indexed roundId, uint256 pot, uint256 totalWeight, address indexed by, uint256 tip);
    event Claimed(uint256 indexed roundId, uint256 indexed tokenId, address indexed to, uint256 scryShare);
    event Paid(uint256 indexed roundId, uint256 indexed tokenId, address coin, uint256 amountIn, uint256 amountOut);
    event Recycled(uint256 indexed roundId, uint256 remainder);
    event ThresholdProposed(uint256 value, uint64 eta);
    event ThresholdSet(uint256 value);
    event RouteProposed(uint16 indexed trackId, address coin, uint24 fee, uint64 eta);
    event RouteSet(uint16 indexed trackId, address coin, uint24 fee);
    event TipSet(uint16 bps);
    event RoundWindowSet(uint32 seconds_);
    event OperatorTransferred(address indexed from, address indexed to);

    modifier onlyOperator() {
        require(msg.sender == operator, "not operator");
        _;
    }

    constructor(
        IERC20 _scry,
        IScrySeat _seat,
        ISwapRouter _router,
        uint256 _threshold,
        uint16 _tipBps,
        uint32 _roundWindow
    ) {
        require(address(_scry) != address(0), "zero scry");
        require(address(_seat) != address(0), "zero seat");
        require(_threshold > 0, "zero threshold");
        require(_tipBps <= MAX_TIP_BPS, "tip too fat");
        require(_roundWindow > 0, "zero window");
        uint256 width = _seat.MAX_ELECTION();
        // This contract unrolls exactly three legs. Welding the width here
        // means a seat with a different election shape cannot be wired in at
        // all, rather than being wired in and silently dropping a leg.
        require(width == 3, "election width mismatch");
        scry = _scry;
        seat = _seat;
        router = _router;
        electionWidth = width;
        threshold = _threshold;
        tipBps = _tipBps;
        roundWindow = _roundWindow;
        operator = msg.sender;
        emit OperatorTransferred(address(0), msg.sender);
        emit ThresholdSet(_threshold);
        emit TipSet(_tipBps);
        emit RoundWindowSet(_roundWindow);
    }

    // ── the bar ──────────────────────────────────────────────────────────────
    /// SCRY sitting here that no open round is owed — the bar's live level.
    function pooled() public view returns (uint256) {
        uint256 bal = scry.balanceOf(address(this));
        return bal > outstanding ? bal - outstanding : 0;
    }

    /// How full the bar is, in basis points of `threshold`, capped at 10,000 —
    /// the one number a page draws, so it never has to do this arithmetic and
    /// get it subtly different from the contract.
    function fillBps() external view returns (uint256) {
        uint256 p = pooled();
        if (p >= threshold) return 10_000;
        return p * 10_000 / threshold;
    }

    /// Contribute, and say who did. A bare `transfer` works identically and is
    /// the path the fee splitter takes; this exists so a contributor can be
    /// attributed on chain when they want to be.
    function fill(uint256 amount) external nonReentrant {
        require(amount > 0, "nothing to fill");
        SafeERC20.safeTransferFrom(address(scry), msg.sender, address(this), amount, "fill failed");
        emit Filled(msg.sender, amount);
    }

    /// ⭐ The lever. Permissionless by construction, not by courtesy — the whole
    /// invariant-9 argument rests on the house having no say in when this
    /// fires. The caller keeps `tipBps` of the pot for the gas and the status.
    function open() external nonReentrant returns (uint256 roundId) {
        uint256 pot = pooled();
        require(pot >= threshold, "the bar is not full");
        uint256 tw = seat.totalWeight();
        require(tw > 0, "nobody is on the roster");

        uint256 tip = pot * tipBps / 10_000;
        uint256 potAfterTip = pot - tip;
        require(potAfterTip > 0, "empty round");

        roundId = _rounds.length;
        _rounds.push(
            Round({
                pot: potAfterTip,
                paid: 0,
                totalWeight: tw,
                openedAt: uint64(block.timestamp),
                window: roundWindow,
                recycled: false
            })
        );
        outstanding += potAfterTip;

        if (tip > 0) SafeERC20.safeTransfer(address(scry), msg.sender, tip, "tip failed");
        emit Opened(roundId, potAfterTip, tw, msg.sender, tip);
    }

    // ── the draw ─────────────────────────────────────────────────────────────
    /// Take this seat's share of a round, in the coins it elected.
    ///
    /// ⚠ **The holder calls this, and that is a safety property rather than a
    /// courtesy.** `minOut` is a slippage bound, and a stranger allowed to set
    /// another wallet's slippage could hand a sandwich most of its share. A
    /// seat with no election needs no bound and is paid SCRY.
    ///
    /// @param minOut per elected leg, in that leg's coin. Legs that pay SCRY
    ///        ignore it. Pass zeros only if you genuinely accept any price.
    function claim(uint256 roundId, uint256 tokenId, uint256[3] calldata minOut)
        external
        nonReentrant
        returns (uint256 scryShare)
    {
        require(roundId < _rounds.length, "no such round");
        Round storage r = _rounds[roundId];
        require(!r.recycled, "round recycled");
        require(block.timestamp < r.openedAt + r.window, "round window closed");
        require(!claimed[roundId][tokenId], "already claimed");

        address holder = seat.ownerOf(tokenId);
        require(holder == msg.sender, "not your seat");

        // A tier that moved after the round opened counts from the NEXT round.
        // The share it would have taken stays in the pot and recycles.
        require(seat.tierSetAt(tokenId) < r.openedAt, "tier moved after this round opened");

        uint256 w = seat.weightOf(tokenId);
        require(w > 0, "seat is benched");

        scryShare = r.pot * w / r.totalWeight;
        require(scryShare > 0, "share rounds to nothing");

        claimed[roundId][tokenId] = true;
        r.paid += scryShare;
        outstanding -= scryShare;
        emit Claimed(roundId, tokenId, holder, scryShare);

        _payOut(roundId, tokenId, holder, scryShare, minOut);
    }

    /// Split the share across the seat's elected coins, buying each at market.
    /// ⚠ **Fails OPEN to SCRY, never closed.** A track with no posted route, an
    /// unset router, or a track id nobody has defined pays plain SCRY instead
    /// of reverting — a holder's claim must never be blocked because the
    /// operator has not posted a pool yet.
    function _payOut(uint256 roundId, uint256 tokenId, address holder, uint256 share, uint256[3] calldata minOut)
        internal
    {
        (uint16[3] memory tracks, uint16[3] memory bps) = seat.electionOf(tokenId);
        uint256 sum;
        for (uint256 i = 0; i < 3; i++) {
            sum += bps[i];
        }
        if (sum != 10_000) {
            // No election on file (or a malformed one): the default track.
            SafeERC20.safeTransfer(address(scry), holder, share, "scry payout failed");
            emit Paid(roundId, tokenId, address(scry), share, share);
            return;
        }

        uint256 spent;
        for (uint256 i = 0; i < 3; i++) {
            if (bps[i] == 0) continue;
            // The last funded leg takes the rounding dust, so the seat is paid
            // its whole share and nothing sticks to this contract.
            uint256 amt = share * bps[i] / 10_000;
            if (i == 2 || _isLastFunded(bps, i)) amt = share - spent;
            spent += amt;
            if (amt == 0) continue;
            _leg(roundId, tokenId, holder, tracks[i], amt, minOut[i]);
        }
    }

    function _isLastFunded(uint16[3] memory bps, uint256 i) internal pure returns (bool) {
        for (uint256 j = i + 1; j < 3; j++) {
            if (bps[j] != 0) return false;
        }
        return true;
    }

    function _leg(uint256 roundId, uint256 tokenId, address holder, uint16 trackId, uint256 amt, uint256 minOut)
        internal
    {
        address coin = coinOf[trackId];
        uint24 fee = feeOf[trackId];
        if (trackId == 0 || coin == address(0) || fee == 0 || address(router) == address(0)) {
            SafeERC20.safeTransfer(address(scry), holder, amt, "scry leg failed");
            emit Paid(roundId, tokenId, address(scry), amt, amt);
            return;
        }
        SafeERC20.safeApprove(address(scry), address(router), amt, "approve failed");
        uint256 out = router.exactInputSingle(
            ISwapRouter.ExactInputSingleParams({
                tokenIn: address(scry),
                tokenOut: coin,
                fee: fee,
                recipient: holder, // straight to the seat's owner; never parked here
                deadline: block.timestamp,
                amountIn: amt,
                amountOutMinimum: minOut,
                sqrtPriceLimitX96: 0
            })
        );
        emit Paid(roundId, tokenId, coin, amt, out);
    }

    /// Return a closed round's unclaimed remainder to the bar. Permissionless,
    /// and the only place it can go is the next round — there is no path out.
    function recycle(uint256 roundId) external nonReentrant returns (uint256 remainder) {
        require(roundId < _rounds.length, "no such round");
        Round storage r = _rounds[roundId];
        require(!r.recycled, "already recycled");
        require(block.timestamp >= r.openedAt + r.window, "round still open");
        remainder = r.pot - r.paid;
        r.recycled = true;
        if (remainder > 0) outstanding -= remainder; // the SCRY stays put; pooled() picks it back up
        emit Recycled(roundId, remainder);
    }

    // ── reads a page draws ───────────────────────────────────────────────────
    function roundCount() external view returns (uint256) {
        return _rounds.length;
    }

    function rounds(uint256 roundId) external view returns (Round memory) {
        require(roundId < _rounds.length, "no such round");
        return _rounds[roundId];
    }

    /// What this seat would take from an open round right now, and why not if
    /// zero. Saves every surface from re-deriving the rules and getting one
    /// subtly wrong.
    function previewClaim(uint256 roundId, uint256 tokenId) external view returns (uint256 share, string memory why) {
        if (roundId >= _rounds.length) return (0, "no such round");
        Round storage r = _rounds[roundId];
        if (r.recycled) return (0, "round recycled");
        if (block.timestamp >= r.openedAt + r.window) return (0, "round window closed");
        if (claimed[roundId][tokenId]) return (0, "already claimed");
        if (seat.tierSetAt(tokenId) >= r.openedAt) return (0, "tier moved after this round opened");
        uint256 w = seat.weightOf(tokenId);
        if (w == 0) return (0, "seat is benched");
        return (r.pot * w / r.totalWeight, "");
    }

    // ── knobs: proposed, then landed after KNOB_DELAY ────────────────────────
    /// ⚠ THE TIMELOCK IS AN INVARIANT-9 CONTROL, NOT ADMIN HYGIENE. An operator
    /// who could lower the bar on demand would be choosing when the house buys,
    /// which is the thing the posted trigger exists to prevent.
    function proposeThreshold(uint256 value) external onlyOperator {
        require(value > 0, "zero threshold");
        uint64 eta = uint64(block.timestamp + KNOB_DELAY);
        pending[keccak256("threshold")] = Pending({value: value, coin: address(0), fee: 0, eta: eta});
        emit ThresholdProposed(value, eta);
    }

    /// Land a proposed threshold. Permissionless — a timelock nobody but the
    /// proposer can execute is a timelock the proposer still controls.
    function commitThreshold() external {
        Pending memory p = pending[keccak256("threshold")];
        require(p.eta != 0, "nothing proposed");
        require(block.timestamp >= p.eta, "still timelocked");
        threshold = p.value;
        delete pending[keccak256("threshold")];
        emit ThresholdSet(p.value);
    }

    /// Post (or repoint) what a track pays. Track 0 is SCRY and is not settable.
    function proposeRoute(uint16 trackId, address coin, uint24 fee) external onlyOperator {
        require(trackId != 0, "track 0 is SCRY");
        require(coin != address(scry), "track 0 is SCRY");
        // coin == 0 retires the track back to the SCRY default.
        require(coin == address(0) || fee > 0, "zero fee tier");
        uint64 eta = uint64(block.timestamp + KNOB_DELAY);
        pending[keccak256(abi.encodePacked("route", trackId))] =
            Pending({value: trackId, coin: coin, fee: fee, eta: eta});
        emit RouteProposed(trackId, coin, fee, eta);
    }

    function commitRoute(uint16 trackId) external {
        bytes32 k = keccak256(abi.encodePacked("route", trackId));
        Pending memory p = pending[k];
        require(p.eta != 0, "nothing proposed");
        require(block.timestamp >= p.eta, "still timelocked");
        coinOf[trackId] = p.coin;
        feeOf[trackId] = p.fee;
        delete pending[k];
        emit RouteSet(trackId, p.coin, p.fee);
    }

    /// The tip and the window move immediately: neither can time a buy. The tip
    /// is capped in bytecode, and the window is copied into each round at
    /// `open()` — so this sets the term of the NEXT round and cannot shorten or
    /// extend one that is already running (`Round.window`).
    function setTip(uint16 bps) external onlyOperator {
        require(bps <= MAX_TIP_BPS, "tip too fat");
        tipBps = bps;
        emit TipSet(bps);
    }

    function setRoundWindow(uint32 seconds_) external onlyOperator {
        require(seconds_ > 0, "zero window");
        roundWindow = seconds_;
        emit RoundWindowSet(seconds_);
    }

    /// address(0) renounces: the knobs freeze where they stand, and everything
    /// that matters here was already permissionless (`HELD-KEYS.md`).
    function transferOperator(address to) external onlyOperator {
        emit OperatorTransferred(operator, to);
        operator = to;
    }
}
