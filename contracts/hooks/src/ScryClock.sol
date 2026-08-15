// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {BaseHook} from "@openzeppelin/uniswap-hooks/src/base/BaseHook.sol";
import {IPoolManager} from "v4-core/src/interfaces/IPoolManager.sol";
import {IUnlockCallback} from "v4-core/src/interfaces/callback/IUnlockCallback.sol";
import {Hooks} from "v4-core/src/libraries/Hooks.sol";
import {PoolKey} from "v4-core/src/types/PoolKey.sol";
import {PoolId, PoolIdLibrary} from "v4-core/src/types/PoolId.sol";
import {BalanceDelta} from "v4-core/src/types/BalanceDelta.sol";
import {Currency, CurrencyLibrary} from "v4-core/src/types/Currency.sol";
import {SwapParams} from "v4-core/src/types/PoolOperation.sol";

/// @title ScryClock — the Longbarrow Clock, a last-buyer-wins pot on a Uniswap v4 pool
///
/// @notice **The game.** Buying the prize coin through this pool skims a posted
///         slice of what you receive into a pot and makes you the last buyer.
///         Every ticket pushes a countdown further out. When the countdown
///         runs out, the last buyer takes the pot. Then it starts again.
///
///         This is the FOMO3D format and we are not claiming otherwise
///         (it has shipped twice on v4, both are dead, neither with published
///         source). What v4 adds is that **the ticket is the
///         trade**: there is no separate "enter" button, and the pot is fed by
///         the swap itself rather than by a deposit into some other contract.
///
/// @dev    ## What this contract deliberately does NOT have
///
///         No owner. No admin role. No proxy. No setter of any kind. Every
///         parameter is `immutable` and public, fixed in the constructor and
///         readable by anyone forever. The rule: a hook is team-authored code
///         in the swap path, so the one thing it must never do is add a key. There is no function on this contract that the
///         deployer can call and you cannot.
///
///         The consequence is real and is the intended trade: **nothing here
///         can be fixed after deploy.** A wrong `POT_BPS` is wrong forever and
///         the answer is a new pool. That is the same bargain `SpoilsToken`
///         made with its immutable cap.
///
///         ## Why there is no randomness anywhere in this file
///
///         A swap is a call the trader fully controls, so
///         any random outcome resolved *inside* it can be simulated and
///         reverted when it loses — a free option we would be writing and
///         giving away. No entropy source fixes this, because every source
///         (`prevrandao`, any past `blockhash`, `arbBlockHash`, a stored seed)
///         is fixed for the whole block and therefore readable before the call.
///
///         This game is immune because it contains no randomness at all: the
///         winner is a deterministic function of who bought last before a wall
///         clock deadline, and it resolves in a LATER transaction. If you want
///         a gamble, route it to `ScryGacha`, which already does commit-reveal
///         on `arbBlockHash` correctly.
///
///         ## The clock is `block.timestamp`, and that is not a style choice
///
///         RH-Chain is Arbitrum Nitro. Solidity's `block.number` is the PARENT
///         chain's and advances once per ~12s while the real chain runs at
///         101ms. `ScryGacha` shipped on `block.number` and silently turned a
///         "100 block, ~10 second" reveal into ~20 minutes — nothing failed and
///         the tests passed. `contracts/RUNBOOK.md` §0c.
///
///         Every duration here is `block.timestamp`, which is accurate on
///         Nitro. This contract therefore needs no `ArbSys` call and no
///         `MockArbSys` etch in tests — a deliberate simplification, and the
///         reason the clock rig is absent rather than forgotten.
///
///         ## Playing is opt-in, and that is what makes the pool safe to route
///
///         The hook cannot tell who is swapping. `sender` in a v4 callback is
///         whoever called `PoolManager.swap` — **the router** — so through
///         UniversalRouter every swap in the pool has the same `sender`.
///         Crediting it would make the router the last buyer and hand it the
///         pot. `tx.origin` is the usual dodge and it breaks smart wallets.
///
///         So the beneficiary is named explicitly in `hookData`, and **a swap
///         that names nobody simply does not play**: no skim, no ticket, no
///         fee. `hookData` is forgeable — anyone may name anyone — which is
///         harmless here because the only thing you can forge is a gift.
///
///         The side effect is the good part. This pool is simultaneously an
///         ordinary MYRRH/OBOL market (untaxed, safe for any router to route
///         through) and a game venue (for anyone who opts in through our front
///         end). A pot skim on every passing trade would have made it a bad
///         road; this way it is a road AND a table.
///
///         ## Only exact-input buys are tickets
///
///         The skim comes out of the *unspecified* currency, and we take it
///         only when that currency is the prize and the trader is receiving it.
///         For an exact-output buy the unspecified side is what the trader
///         PAYS, so no skim happens — and because a ticket is only granted when
///         a contribution actually lands, such a swap does not become the last
///         buyer either. That closes the obvious dodge: you cannot route as
///         exact-output to take the crown without paying for it.
contract ScryClock is BaseHook, IUnlockCallback {
    using PoolIdLibrary for PoolKey;
    using CurrencyLibrary for Currency;

    uint256 public constant BPS = 10_000;

    // ---------------------------------------------------------------- immutable

    /// @notice The coin the pot accrues in — the one players BUY.
    /// @dev    Must be one of the pool's two currencies. Checked at initialize.
    Currency public immutable POT_CURRENCY;

    /// @notice Which swap direction counts as a buy.
    /// @dev    True when buying means `zeroForOne` (spending currency0 to get
    ///         currency1). With two house-minted coins there is no numeraire,
    ///         so this is a naming decision made at deploy, not a market fact.
    bool public immutable BUY_ZERO_FOR_ONE;

    /// @notice Slice of the prize coin received that goes to the pot, in bps.
    uint256 public immutable POT_BPS;

    /// @notice How far each ticket pushes the deadline out, in seconds.
    /// @dev    Minutes, not seconds. FOMO3D's famous ending was a bot stuffing
    ///         blocks so nobody else's transaction could land. On a
    ///         single-sequencer Nitro chain the shape differs, but the defense
    ///         is the same: make out-spamming the field cost more than the pot.
    uint64 public immutable EXTENSION;

    /// @notice Hard ceiling on how far ahead the deadline may ever sit.
    /// @dev    Without this a whale spams dust tickets and pushes the deadline
    ///         past anyone's patience for near-zero cost. `EXTENSION` caps the
    ///         sniping war; this caps total reach.
    uint64 public immutable MAX_HORIZON;

    /// @notice Slice of a settled pot that seeds the NEXT round, in bps.
    /// @dev    Zero means every round starts empty. Non-zero means the game
    ///         restarts with something on the table, which is the difference
    ///         between a game that runs once and a game that runs.
    uint256 public immutable ROLLOVER_BPS;

    // ------------------------------------------------------------------ storage

    /// @notice The one pool this hook serves, locked at the first initialize.
    /// @dev    The hook address is part of `PoolKey`, so ANYONE can open a pool
    ///         against this hook — including one full of worthless tokens, to
    ///         drive our clock. This is a one-shot lock: the first pool to
    ///         initialize wins and every later attempt reverts.
    ///
    ///         ⚠ That makes deployment a race unless deploy and initialize
    ///         happen in the SAME transaction. `DeployScryClock.s.sol` does
    ///         exactly that and it is not optional.
    PoolId public poolId;
    bool public bound;

    /// @notice The live pot, denominated in `POT_CURRENCY`.
    /// @dev    Held as an ERC-6909 claim on the PoolManager, not as real
    ///         tokens — no transfer happens until a winner claims, so the swap
    ///         path makes no external token call and cannot reenter.
    uint256 public pot;

    /// @notice Who takes the pot if the clock runs out right now.
    address public lastBuyer;

    /// @notice When the clock runs out. Zero means no round is running.
    uint64 public deadline;

    /// @notice How many rounds have settled.
    uint64 public round;

    /// @notice Winnings credited and not yet withdrawn.
    /// @dev    Pull, never push. A push to a contract that reverts on receive
    ///         would wedge settlement for everyone; `ScryGacha` uses the same
    ///         `owed` shape for the same reason.
    mapping(address => uint256) public owed;

    /// @notice Σ `owed`, so the contract can prove it is not crediting more
    ///         than it holds.
    uint256 public owedTotal;

    // ------------------------------------------------------------------- events

    event Bound(PoolId indexed id, Currency potCurrency, bool buyZeroForOne);
    event Ticket(address indexed buyer, uint256 contribution, uint256 pot, uint64 deadline, uint64 round);
    event Settled(uint64 indexed round, address indexed winner, uint256 won, uint256 rolledOver);
    event Claimed(address indexed who, uint256 amount);

    // ------------------------------------------------------------------- errors

    error AlreadyBound();
    error WrongPool();
    error PotCurrencyNotInPool();
    error BadParameter();
    error NoRound();
    error StillRunning();
    error NothingOwed();
    // `NotPoolManager` is BaseHook's, not ours — redeclaring it here is a
    // compile error, and inheriting it is what we wanted anyway: one selector
    // for "you are not the manager", whether the caller missed
    // `onlyPoolManager` or `unlockCallback`.

    // -------------------------------------------------------------- constructor

    constructor(
        IPoolManager _poolManager,
        Currency _potCurrency,
        bool _buyZeroForOne,
        uint256 _potBps,
        uint64 _extension,
        uint64 _maxHorizon,
        uint256 _rolloverBps
    ) BaseHook(_poolManager) {
        // Bounds are asserted here because they can never be corrected later.
        // A pot slice above 10% is a tax nobody will trade through; an
        // extension under a minute is the block-stuffing hole; a horizon
        // shorter than one extension would make the cap bite on every ticket.
        if (_potBps == 0 || _potBps > 1_000) revert BadParameter();
        if (_rolloverBps > BPS) revert BadParameter();
        if (_extension < 60) revert BadParameter();
        if (_maxHorizon < _extension) revert BadParameter();

        POT_CURRENCY = _potCurrency;
        BUY_ZERO_FOR_ONE = _buyZeroForOne;
        POT_BPS = _potBps;
        EXTENSION = _extension;
        MAX_HORIZON = _maxHorizon;
        ROLLOVER_BPS = _rolloverBps;
    }

    // -------------------------------------------------------------- permissions

    /// @dev v4 encodes a hook's permissions in the LOW BITS OF ITS OWN ADDRESS,
    ///      so the deploy is a CREATE2 salt grind until the address matches
    ///      this table. `BaseHook`'s constructor validates the match and
    ///      reverts otherwise, which is why a wrong salt fails loudly at deploy
    ///      rather than quietly at the first swap.
    ///
    ///      Flags needed: BEFORE_INITIALIZE (1<<13), AFTER_SWAP (1<<6),
    ///      AFTER_SWAP_RETURNS_DELTA (1<<2)  =  0x2044.
    function getHookPermissions() public pure override returns (Hooks.Permissions memory) {
        return Hooks.Permissions({
            beforeInitialize: true,
            afterInitialize: false,
            beforeAddLiquidity: false,
            afterAddLiquidity: false,
            beforeRemoveLiquidity: false,
            afterRemoveLiquidity: false,
            beforeSwap: false,
            afterSwap: true,
            beforeDonate: false,
            afterDonate: false,
            beforeSwapReturnDelta: false,
            afterSwapReturnDelta: true,
            afterAddLiquidityReturnDelta: false,
            afterRemoveLiquidityReturnDelta: false
        });
    }

    // ------------------------------------------------------------------- hooks

    /// @dev One-shot bind. See `poolId` for why this exists and why the deploy
    ///      script must initialize atomically.
    function _beforeInitialize(address, PoolKey calldata key, uint160) internal override returns (bytes4) {
        if (bound) revert AlreadyBound();

        // The prize must actually be one of the two coins in this pool, or the
        // skim can never fire and the game is dead on arrival with no error.
        if (!_eq(key.currency0, POT_CURRENCY) && !_eq(key.currency1, POT_CURRENCY)) {
            revert PotCurrencyNotInPool();
        }

        // The prize must be what a BUYER RECEIVES, or `_afterSwap` skips every
        // swap forever: the unspecified side of an exact-input buy is the
        // output, so the prize has to be the output of the buy direction.
        Currency bought = BUY_ZERO_FOR_ONE ? key.currency1 : key.currency0;
        if (!_eq(bought, POT_CURRENCY)) revert BadParameter();

        poolId = key.toId();
        bound = true;
        emit Bound(poolId, POT_CURRENCY, BUY_ZERO_FOR_ONE);
        return BaseHook.beforeInitialize.selector;
    }

    /// @dev The whole game, and the only place money moves in the swap path.
    function _afterSwap(
        address,
        PoolKey calldata key,
        SwapParams calldata params,
        BalanceDelta delta,
        bytes calldata hookData
    ) internal override returns (bytes4, int128) {
        if (PoolId.unwrap(key.toId()) != PoolId.unwrap(poolId)) revert WrongPool();

        // Selling is not a ticket.
        if (params.zeroForOne != BUY_ZERO_FOR_ONE) return (BaseHook.afterSwap.selector, 0);

        // Opt-in only. No named beneficiary → an ordinary, untaxed swap.
        address buyer = _beneficiary(hookData);
        if (buyer == address(0)) return (BaseHook.afterSwap.selector, 0);

        // The unspecified currency is the side the trader did NOT pin an exact
        // amount to. `amountSpecified < 0` is exact-input.
        //
        // ⚠ v4 sign convention: in the returned `delta`, negative is what the
        //    trader OWES the pool and positive is what the pool OWES the
        //    trader. We can only skim something the trader is receiving.
        bool exactInput = params.amountSpecified < 0;
        Currency unspecified;
        int128 unspecifiedAmount;
        if (exactInput) {
            unspecified = params.zeroForOne ? key.currency1 : key.currency0;
            unspecifiedAmount = params.zeroForOne ? delta.amount1() : delta.amount0();
        } else {
            unspecified = params.zeroForOne ? key.currency0 : key.currency1;
            unspecifiedAmount = params.zeroForOne ? delta.amount0() : delta.amount1();
        }

        if (unspecifiedAmount <= 0) return (BaseHook.afterSwap.selector, 0);
        if (!_eq(unspecified, POT_CURRENCY)) return (BaseHook.afterSwap.selector, 0);

        uint256 cut = (uint256(uint128(unspecifiedAmount)) * POT_BPS) / BPS;
        // No contribution, no ticket. This is what stops a dust swap from
        // taking the crown for free.
        if (cut == 0) return (BaseHook.afterSwap.selector, 0);

        // Hold the cut as an ERC-6909 claim on the singleton. No token
        // transfer, so nothing external is called and nothing can reenter.
        poolManager.mint(address(this), unspecified.toId(), cut);
        pot += cut;

        lastBuyer = buyer;
        uint64 extended = uint64(block.timestamp) + EXTENSION;
        uint64 ceiling = uint64(block.timestamp) + MAX_HORIZON;
        deadline = extended > ceiling ? ceiling : extended;

        emit Ticket(buyer, cut, pot, deadline, round);

        // Positive = the hook is owed this much of the unspecified currency.
        return (BaseHook.afterSwap.selector, int128(int256(cut)));
    }

    // ------------------------------------------------------------- the endgame

    /// @notice End the round and credit the pot to whoever bought last.
    /// @dev    Permissionless on purpose. A winner-only settle wedges the pot
    ///         the moment the winner loses their key or is a contract that
    ///         cannot call. Anyone may ring the bell; only the last buyer gets
    ///         paid, so there is nothing to gain by ringing it early and it
    ///         reverts if you try.
    function settle() external {
        if (lastBuyer == address(0) || deadline == 0) revert NoRound();
        if (block.timestamp <= deadline) revert StillRunning();

        uint256 won = pot;
        address winner = lastBuyer;

        uint256 rollover = (won * ROLLOVER_BPS) / BPS;
        uint256 payout = won - rollover;

        // Effects before anything else. The rollover stays as a 6909 claim
        // this contract already holds, so it needs no movement at all.
        pot = rollover;
        lastBuyer = address(0);
        deadline = 0;
        owed[winner] += payout;
        owedTotal += payout;

        uint64 r = round;
        round = r + 1;
        emit Settled(r, winner, payout, rollover);
    }

    /// @notice Withdraw winnings. Pull, never push.
    function claim() external {
        uint256 amount = owed[msg.sender];
        if (amount == 0) revert NothingOwed();

        owed[msg.sender] = 0;
        owedTotal -= amount;

        // Unwrap the 6909 claim into real tokens and send them out. The
        // credit is already zeroed, so the callback cannot be reentered into a
        // second payout.
        poolManager.unlock(abi.encode(msg.sender, amount));
        emit Claimed(msg.sender, amount);
    }

    /// @dev The PoolManager calls back here inside `claim`'s unlock.
    function unlockCallback(bytes calldata data) external override returns (bytes memory) {
        if (msg.sender != address(poolManager)) revert NotPoolManager();
        (address to, uint256 amount) = abi.decode(data, (address, uint256));
        poolManager.burn(address(this), POT_CURRENCY.toId(), amount);
        poolManager.take(POT_CURRENCY, to, amount);
        return "";
    }

    // ------------------------------------------------------------------- views

    /// @notice Seconds until the clock runs out; zero when it already has or no
    ///         round is running.
    function timeLeft() external view returns (uint64) {
        if (deadline == 0 || block.timestamp >= deadline) return 0;
        return deadline - uint64(block.timestamp);
    }

    /// @notice True when `settle()` would succeed right now.
    function settleable() external view returns (bool) {
        return lastBuyer != address(0) && deadline != 0 && block.timestamp > deadline;
    }

    // ---------------------------------------------------------------- internals

    /// @dev A 32-byte `hookData` is an abi-encoded address. Anything else —
    ///      empty, malformed, or the zero address — means "not playing", which
    ///      is a normal untaxed swap rather than an error. Reverting here would
    ///      break every router that passes its own payload through.
    function _beneficiary(bytes calldata hookData) private pure returns (address) {
        if (hookData.length != 32) return address(0);
        return abi.decode(hookData, (address));
    }

    function _eq(Currency a, Currency b) private pure returns (bool) {
        return Currency.unwrap(a) == Currency.unwrap(b);
    }
}
