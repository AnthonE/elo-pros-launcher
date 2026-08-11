// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

// The town's burn address, at FILE level so a deploy script or a test can name
// it BEFORE a splitter exists to ask (`ScryFeeSplitter.BURN` is not reachable
// through the contract type in Solidity). The contract re-exposes it as `BURN`
// for on-chain readers.
address constant SCRY_BURN = 0x000000000000000000000000000000000000dEaD;

/// @title ScryFeeSplitter — every SERVICE fee flows through one posted split
/// @notice The services send their SCRY fees (the job-board cut, stele-edition
///         mints, familiar labor) to THIS address. Anyone may call distribute()
///         at any time; the current balance is pushed out by the posted
///         basis-point split — typically bank (ScryBank, the xSCRY flywheel) /
///         prize escrow / ops. The split is public state and every change is an
///         event, so the fee math is auditable end to end, which is the
///         SCRY-ECONOMY.md requirement.
///
///         ⚠ SUPERSEDED, 2026-07-25 — do not restore. This header used to read:
///         "only SCRY SERVICE revenue flows here. The GAMES pay in game tokens
///         (OBOL/MYRRH) — a game rake or buy-in is an OBOL ledger sink, never
///         SCRY, so it never reaches this splitter or the Bank." The operator
///         sentence "scry can enter" (2026-07-25, FEES.md §1) REVERSED that:
///         SCRY is now taken on the game surfaces as fees, entries and bonds,
///         so a game rake CAN reach this splitter. The rule was fixed here
///         before broadcast on purpose — left alone, an on-chain fee policy
///         would have contradicted the actual fee policy, welded where it
///         cannot be edited. (TOKENOMICS.md had already listed "game fees"
///         under SCRY since 07-20, so the two disagreed for five days.)
///
///         THE WALL THAT DID NOT MOVE, and it is law: this splitter never
///         touches MEASUREMENT revenue. Meter fees (USDG/USDC) fund research
///         infrastructure and never enter the token economy — a furnace fed by
///         read revenue would make SCRY's supply a function of how many
///         readings get sold, giving every holder a stake in the meter selling
///         more. Score-blind, forever (CLAUDE.md; SCRY-ECONOMY.md).
///
///         THE BURN (the SCRY half of the furnace, FEES.md §3.1). A burn is
///         just another recipient: put BURN in the posted split and SCRY fees
///         end at 0xdEaD by default, with no new contract, no new key and no
///         new audit surface. burnBps() and totalBurned make the advertised
///         claim readable straight off the ABI, and Burned() gives /tokens/flux
///         an attributable reason — a burn nobody can attribute is invisible to
///         the gauge that governs the whole elastic-supply decision.
///
///         ⚠ Say this whenever the burn is advertised: rescue() below is an
///         operator override that BYPASSES the split. So the burn is the
///         DEFAULT path, not an unbreakable one. The honest sentence is "the
///         split is the only path funds take WITHOUT the operator key."
///         Overstating it is the one thing that would make the burn worth less
///         than not having one (FEES.md §3.1; HELD-KEYS.md).
///
///         The operator owns the split percentages/recipients AND holds an
///         escape hatch: rescue() pulls the balance (or any stray token) out
///         directly, bypassing the split, on the operator's instruction. The
///         normal path stays distribute() along the posted split, with rounding
///         dust to the last recipient so nothing sticks to the contract.
contract ScryFeeSplitter is ReentrancyGuard {
    /// @notice The town's burn address. Value sent here is provably
    ///         unrecoverable by anyone, including the operator.
    ///         Burn by TRANSFER, never by assuming a burn() function: SCRY is a
    ///         fair-launched fixed-supply token this repo did not write and does
    ///         not control, so its interface is not ours to assume. 0xdEaD is
    ///         already the town's convention (ScryGardener's slash ladder).
    ///         Corollary worth stating plainly wherever this is advertised:
    ///         totalSupply() does NOT fall. The tokens become unspendable, which
    ///         is the same economics and a different sentence.
    address public constant BURN = SCRY_BURN;

    IERC20 public immutable scry;
    address public operator;

    /// Cumulative SCRY sent to BURN by distribute(). Never decreases.
    uint256 public totalBurned;

    struct Cut {
        address to;
        uint16 bps; // out of 10_000
    }

    Cut[] public cuts;

    event SplitSet(address[] to, uint16[] bps);
    event Distributed(uint256 amount);
    /// Emitted alongside Distributed when the posted split routes to BURN, so
    /// the furnace is its own line in /tokens/flux rather than an anonymous
    /// transfer an indexer has to recognize by address.
    event Burned(uint256 amount, uint256 cumulative);
    event OperatorTransferred(address indexed from, address indexed to);
    event Rescued(address indexed token, address indexed to, uint256 amount);
    event NativeReceived(address indexed from, uint256 amount);
    event NativeRescued(address indexed to, uint256 amount);

    modifier onlyOperator() {
        require(msg.sender == operator, "not operator");
        _;
    }

    constructor(IERC20 _scry, address[] memory to, uint16[] memory bps) {
        scry = _scry;
        operator = msg.sender;
        _setSplit(to, bps);
    }

    /// Push the whole current balance out along the posted split. Anyone can
    /// call, and no caller can redirect a wei of it. It is NOT the only path
    /// funds can take — `rescue()` below is the operator's posted override,
    /// and saying otherwise here contradicted this file's own header (audit
    /// 2026-07-25). The honest sentence: the split is the only path funds take
    /// WITHOUT the operator key.
    function distribute() external nonReentrant returns (uint256 amount) {
        amount = scry.balanceOf(address(this));
        require(amount > 0, "nothing to distribute");
        uint256 sent;
        uint256 burned;
        uint256 n = cuts.length;
        for (uint256 i = 0; i < n; i++) {
            uint256 share = i == n - 1
                ? amount - sent  // last recipient takes the rounding dust
                : (amount * cuts[i].bps) / 10_000;
            sent += share;
            // A BURN cut is transferred like any other, so the burn cannot
            // silently diverge from the posted split; it is only ACCOUNTED
            // separately. Summed, not assigned — the split may legitimately
            // carry more than one BURN line, and += is the difference between
            // a counter and a bug nobody would see until the total was wrong.
            if (cuts[i].to == BURN) burned += share;
            SafeERC20.safeTransfer(address(scry), cuts[i].to, share, "transfer failed");
        }
        if (burned > 0) {
            totalBurned += burned;
            emit Burned(burned, totalBurned);
        }
        emit Distributed(amount);
    }

    /// @notice The advertised burn rate, in basis points, read off the posted
    ///         split itself rather than stored beside it — so it cannot drift
    ///         from what distribute() actually does. 0 means the split carries
    ///         no burn line, which is a perfectly valid configuration and must
    ///         read as an honest zero rather than an error.
    /// @dev    Sums every BURN cut in case the split posts more than one.
    ///         Remember rescue() when quoting this number: it is the default
    ///         path, not an unbreakable one.
    function burnBps() external view returns (uint16 bps) {
        uint256 n = cuts.length;
        for (uint256 i = 0; i < n; i++) {
            if (cuts[i].to == BURN) bps += cuts[i].bps;
        }
    }

    /// Repost the split. Distributes any held balance FIRST, under the old
    /// posted split — fees collected under posted rules pay out under those
    /// rules; the new split only governs fees that arrive after it.
    function setSplit(address[] calldata to, uint16[] calldata bps) external onlyOperator {
        if (scry.balanceOf(address(this)) > 0) this.distribute();
        _setSplit(to, bps);
    }

    function transferOperator(address to) external onlyOperator {
        require(to != address(0), "zero");
        emit OperatorTransferred(operator, to);
        operator = to;
    }

    /// @notice Operator escape hatch: pull `amount` of any token out to `to`,
    ///         including the SCRY balance (this BYPASSES the split). The normal
    ///         path is distribute(); this is the direct-extraction override.
    ///         amount == 0 pulls the full current balance of `token`.
    function rescue(IERC20 token, address to, uint256 amount) external onlyOperator nonReentrant returns (uint256) {
        require(to != address(0), "zero to");
        if (amount == 0) amount = token.balanceOf(address(this));
        require(amount > 0, "nothing to rescue");
        SafeERC20.safeTransfer(address(token), to, amount, "transfer failed");
        emit Rescued(address(token), to, amount);
        return amount;
    }

    /// @notice Accept native value so an ERC-2981 royalty push cannot revert a
    ///         sale. This contract is `ScryEidolon`'s royalty receiver and that
    ///         address is IMMUTABLE from its constructor (ScryEidolon.sol:64),
    ///         so a marketplace paying royalties in native currency makes a
    ///         zero-calldata call here. With no `receive()` that call reverted:
    ///         either the sale failed outright or the marketplace dropped the
    ///         royalty. The repo already knew this and guarded exactly one path
    ///         — DeployGacha.s.sol probes `sink.call{value: 0}("")` and refuses
    ///         to broadcast, with GACHA.md naming this contract as the reason
    ///         and prescribing this fix. DeployScryEidolon has no such probe.
    ///
    /// @dev    NATIVE VALUE IS NOT COVERED BY THE POSTED SPLIT, and that is
    ///         deliberate rather than an oversight: the split is denominated in
    ///         SCRY, and pushing native value to four recipients would let one
    ///         reverting fallback brick distribution for every other outlet.
    ///         Native arrives only from marketplace royalties and leaves only
    ///         by `rescueNative`, an operator act posted in HELD-KEYS.md
    ///         beside `rescue()`. Do not quote `burnBps()` over native value —
    ///         it does not apply.
    receive() external payable {
        emit NativeReceived(msg.sender, msg.value);
    }

    /// @notice The operator's posted path out for native royalties. Mirrors
    ///         `rescue()`, which is ERC20-only and cannot reach native value.
    ///         amount == 0 pulls the full balance.
    function rescueNative(address payable to, uint256 amount) external onlyOperator nonReentrant returns (uint256) {
        require(to != address(0), "zero to");
        if (amount == 0) amount = address(this).balance;
        require(amount > 0, "nothing to rescue");
        (bool ok,) = to.call{value: amount}("");
        require(ok, "native transfer failed");
        emit NativeRescued(to, amount);
        return amount;
    }

    function splitLength() external view returns (uint256) {
        return cuts.length;
    }

    function _setSplit(address[] memory to, uint16[] memory bps) internal {
        require(to.length == bps.length && to.length > 0, "bad split");
        delete cuts;
        uint256 total;
        for (uint256 i = 0; i < to.length; i++) {
            require(to[i] != address(0), "zero recipient");
            total += bps[i];
            cuts.push(Cut(to[i], bps[i]));
        }
        require(total == 10_000, "bps must sum to 10000");
        // memory→calldata mismatch is fine for the event's abi encoding
        address[] memory a = to;
        uint16[] memory b = bps;
        emit SplitSet(a, b);
    }
}
