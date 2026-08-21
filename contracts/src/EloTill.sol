// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

/// @title EloTill — signed withdrawals against the off-chain play ledger
/// @notice The game claim: a player wins in the barrow at 3am and is paid at
///         3am. The meter signs a DRAFT — "this wallet may have withdrawn N in
///         total" — anyone submits it, this contract verifies, caps and pays.
///         The design and its tradeoffs are in this header; the coins it pays
///         and the numbers the operator must size are `docs/money/TOKENOMICS.md`.
///
///         THIS IS NOT THE AIRDROP AND DOES NOT REPLACE `EloHarvest`. Merkle
///         is the right tool for a snapshot distribution to a fixed list (the
///         drops); it is the wrong tool for "I won, pay me", because nothing
///         can be claimed until an operator posts a root. This is the wrong
///         tool for the drops for the mirror reason: it puts a KEY where a
///         published tree would do the same job with less trust. Both ship —
///         the seam is TWO instruments, and neither replaces the other.
///
///         THE HONEST TRADEOFF, stated in the contract that embodies it: a
///         signing key can authorize any amount; a published merkle root
///         cannot. What bounds this one is `dailyCap` — posted on-chain,
///         raise-delayed and lower-immediate — so a stolen signer takes one
///         day's cap and stops. It does not buy back the difference in kind.
///
///         Token-generic, one instance per coin, exactly like `EloHarvest`:
///         OBOL and MYRRH get their own float and their own cap rather than
///         sharing one number.
///
///         DELIBERATELY NOT HERE: deposits (money comes in through the
///         tessera, which holds nothing), per-player limits or cooldowns
///         (meter-side, so they change without a redeploy), scores, APY,
///         clawback of a paid amount, and — pending the operator's call — a
///         pause. A pause is a real answer to a leaked key and also a
///         censorship lever aimed at the one function players depend on; the
///         cap already bounds a leak, and that call is the operator's and
///         still open.
contract EloTill is ReentrancyGuard {
    // ── what the meter signs ────────────────────────────────────────────────
    /// A draft is an order to pay, and `cumulative` is a LIFETIME RUNNING
    /// TOTAL, never "this withdrawal's amount". That choice is the design:
    ///
    ///   · a draft that is lost, or that the player never submits, is
    ///     harmless — the next one supersedes it;
    ///   · a draft submitted twice pays 0 the second time, by arithmetic
    ///     rather than by a nonce map;
    ///   · two drafts outstanding at once need no ordering — the larger wins
    ///     and the other pays nothing;
    ///   · and the meter has to remember NOTHING. `withdrawn` below is the
    ///     record, the meter reads it with one `eth_call`.
    ///
    /// That last one decides it. The meter is one box; a per-wallet nonce map
    /// is state that must survive a restore, and state that must survive a
    /// restore is exactly where double-payments come from. It is also the
    /// dialect already in the tree — `EloHarvest.claimed[wallet]` is the same
    /// mapping doing the same job.
    struct Draft {
        address wallet; // who gets paid — NEVER the submitter
        uint256 cumulative; // lifetime total this wallet may have withdrawn
        uint64 expiry; // unix seconds
        uint64 epoch; // the signer generation this was written under
    }

    bytes32 public constant DRAFT_TYPEHASH =
        keccak256("Draft(address wallet,uint256 cumulative,uint64 expiry,uint64 epoch)");
    bytes32 public constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant NAME_HASH = keccak256("EloTill");
    bytes32 private constant VERSION_HASH = keccak256("1");
    /// Half the curve order. `ecrecover` accepts both s and n−s for one
    /// signature, so without this a second valid encoding of the same draft
    /// exists — the check `EloPeculium._recover` already carries.
    uint256 private constant SECP256K1N_HALF = 0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0;

    // ── state ───────────────────────────────────────────────────────────────
    IERC20 public immutable token;
    address public operator;
    address public pendingOperator;
    /// The secp256k1 key that writes drafts. NOT the meter's Ed25519
    /// attestation key, and not only because the curves differ: a stolen
    /// attestation key must stay *silence*, never theft (`HELD-KEYS.md`
    /// row 1). Two keys, two blast radii.
    address public signer;
    /// Bumping this voids every draft already in the wild in one transaction —
    /// the only move available if the signing key leaks after drafts are out.
    uint64 public signerEpoch;

    mapping(address => uint256) public withdrawn; // the high-water mark
    mapping(uint256 => uint256) public paidOnDay; // day index => paid that day
    uint256 public totalWithdrawn;

    uint256 public dailyCap;
    uint256 public pendingCap;
    uint64 public pendingCapAt;
    uint64 public sweepAt;

    /// One notice period, for the two acts that can take money away from
    /// players: raising the cap and sweeping the float. Seven days, matching
    /// `EloHarvest.SWEEP_DELAY`, so the suite has one duration rather than
    /// three. Anything shorter and the announcement is decoration.
    uint64 public constant NOTICE = 7 days;

    event Withdrawn(address indexed wallet, uint256 paid, uint256 cumulative, uint256 owedRemaining);
    event SignerSet(address indexed signer, uint64 epoch);
    event CapLowered(uint256 cap);
    event CapRaiseAnnounced(uint256 cap, uint64 effectiveAt);
    event CapRaised(uint256 cap);
    event SweepAnnounced(uint64 effectiveAt);
    event SweepCancelled();
    event Swept(address indexed to, uint256 amount);
    event OperatorTransferStarted(address indexed from, address indexed to);
    event OperatorTransferred(address indexed from, address indexed to);

    modifier onlyOperator() {
        require(msg.sender == operator, "not operator");
        _;
    }

    constructor(IERC20 _token, address _signer, uint256 _dailyCap) {
        require(address(_token) != address(0) && _signer != address(0), "zero");
        token = _token;
        signer = _signer;
        signerEpoch = 1;
        dailyCap = _dailyCap;
        operator = msg.sender;
        emit SignerSet(_signer, 1);
    }

    // ── the one call players make ───────────────────────────────────────────
    /// Present a draft; the wallet named in it is paid.
    ///
    /// CALLABLE BY ANYONE, FOR ANYONE — `paid` always goes to `d.wallet` and
    /// never to `msg.sender`. The property `EloHarvest.claim` already has, for
    /// the same reason: hosted familiars ship `wallet: null` with faucet cap
    /// "0" and vow-signature players hold no ETH (invariants 6–7), so a till
    /// that paid `msg.sender` would let gasless agents win and never collect —
    /// moving the lockout from the door to the cashier.
    ///
    /// PARTIAL FILL IS DELIBERATE. If the day's cap cannot cover everything
    /// owed, this pays what the cap allows and advances the high-water mark by
    /// exactly that much, so the rest is still owed and collectable later. The
    /// alternative — revert until the operator raises the cap — puts an
    /// operator act back in the path between winning and being paid, which is
    /// the whole thing this contract exists to remove.
    ///
    /// An EMPTY FLOAT is not treated the same way, and the difference is the
    /// point: the cap is a POSTED POLICY, so filling partially against it is a
    /// stated rule. A float too thin to pay is an OPERATIONAL FAILURE, and
    /// paying less because the drawer is empty would hide it. That case
    /// reverts, loudly, with the token's own reason.
    function withdraw(Draft calldata d, bytes calldata sig) external nonReentrant returns (uint256 paid) {
        require(block.timestamp <= d.expiry, "draft expired");
        require(d.epoch == signerEpoch, "stale signer epoch");
        address rec = _recover(_digest(d), sig);
        require(rec != address(0) && rec == signer, "bad signature");

        uint256 already = withdrawn[d.wallet];
        require(d.cumulative > already, "nothing to withdraw");
        uint256 owed = d.cumulative - already;

        // The day index is `block.timestamp`-based ON PURPOSE. `block.number`
        // on chain 4663 is the PARENT chain's counter (~one per 12s), so a cap
        // paced on it would be a "daily" cap that reset about every twelve
        // days. `EloGacha` already shipped a ~20-minute reveal that was meant
        // to be ~10 seconds for exactly this reason — `RUNBOOK.md` §0c.
        uint256 day = block.timestamp / 1 days;
        uint256 room = dailyCap > paidOnDay[day] ? dailyCap - paidOnDay[day] : 0;
        require(room > 0, "daily cap reached, try tomorrow");

        paid = owed < room ? owed : room;
        // Effects before interaction, and the high-water mark advances by what
        // was ACTUALLY paid — never to `d.cumulative` — or a capped fill would
        // silently forfeit the difference forever.
        withdrawn[d.wallet] = already + paid;
        paidOnDay[day] += paid;
        totalWithdrawn += paid;

        SafeERC20.safeTransfer(address(token), d.wallet, paid, "transfer failed");
        emit Withdrawn(d.wallet, paid, d.cumulative, d.cumulative - withdrawn[d.wallet]);
    }

    // ── views a page or a wallet can read before signing anything ───────────
    function remainingToday() external view returns (uint256) {
        uint256 spent = paidOnDay[block.timestamp / 1 days];
        return dailyCap > spent ? dailyCap - spent : 0;
    }

    function domainSeparator() public view returns (bytes32) {
        return keccak256(abi.encode(DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
    }

    /// The exact digest the meter must sign. Published as a view so the signing
    /// side can be checked against the verifying side rather than trusted to
    /// agree with it.
    function draftDigest(Draft calldata d) external view returns (bytes32) {
        return _digest(d);
    }

    // ── operator ────────────────────────────────────────────────────────────
    /// Rotate the signing key. Bumping the epoch is not optional and not
    /// separable: every draft written under the old key names the old epoch
    /// and stops verifying here the moment this returns. That is what makes a
    /// leak survivable at all.
    function setSigner(address newSigner) external onlyOperator {
        require(newSigner != address(0), "zero");
        signer = newSigner;
        signerEpoch += 1;
        emit SignerSet(newSigner, signerEpoch);
    }

    /// Lowering takes effect NOW; raising takes NOTICE. An operator who can
    /// raise the cap in the same transaction that drains it has a cap in name
    /// only — and lowering is a safety move, so it must never wait.
    ///
    /// A lower also CANCELS a pending raise. Without that, lower-then-apply
    /// walks the cap straight back up through a raise that was announced
    /// before anyone had reason to watch.
    function setDailyCap(uint256 newCap) external onlyOperator {
        if (newCap <= dailyCap) {
            dailyCap = newCap;
            pendingCap = 0;
            pendingCapAt = 0;
            emit CapLowered(newCap);
            return;
        }
        pendingCap = newCap;
        pendingCapAt = uint64(block.timestamp) + NOTICE;
        emit CapRaiseAnnounced(newCap, pendingCapAt);
    }

    function applyCapRaise() external onlyOperator {
        require(pendingCapAt != 0 && block.timestamp >= pendingCapAt, "not yet");
        dailyCap = pendingCap;
        pendingCap = 0;
        pendingCapAt = 0;
        emit CapRaised(dailyCap);
    }

    /// The float's own door. `EloHarvest.sweep` can bound itself with
    /// arithmetic because a posted root states what is owed; here what is owed
    /// lives in the ledger and no on-chain number can check it. So the bound is
    /// TIME: announce, wait NOTICE, sweep — a player who sees the announcement
    /// has a week to withdraw ahead of it.
    function announceSweep() external onlyOperator {
        sweepAt = uint64(block.timestamp) + NOTICE;
        emit SweepAnnounced(sweepAt);
    }

    function cancelSweep() external onlyOperator {
        sweepAt = 0;
        emit SweepCancelled();
    }

    /// Consumes the announcement. One notice licenses ONE sweep — otherwise a
    /// single announcement, made once and long forgotten, licenses draining the
    /// float in any amount forever.
    function sweep(address to, uint256 amount) external onlyOperator nonReentrant {
        require(sweepAt != 0 && block.timestamp >= sweepAt, "sweep not announced, or too soon");
        require(to != address(0), "zero");
        sweepAt = 0;
        SafeERC20.safeTransfer(address(token), to, amount, "transfer failed");
        emit Swept(to, amount);
    }

    /// Two-step, because a one-step handoff to an uncontrolled address welds
    /// the role to nobody — the failure `EloGranary`'s steward handoff was
    /// changed to avoid on 2026-07-28.
    function transferOperator(address to) external onlyOperator {
        require(to != address(0), "zero");
        pendingOperator = to;
        emit OperatorTransferStarted(operator, to);
    }

    function acceptOperator() external {
        require(msg.sender == pendingOperator, "not pending operator");
        emit OperatorTransferred(operator, pendingOperator);
        operator = pendingOperator;
        pendingOperator = address(0);
    }

    // ── internals ───────────────────────────────────────────────────────────
    function _digest(Draft calldata d) internal view returns (bytes32) {
        bytes32 structHash = keccak256(abi.encode(DRAFT_TYPEHASH, d.wallet, d.cumulative, d.expiry, d.epoch));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), structHash));
    }

    function _recover(bytes32 digest, bytes calldata signature) internal pure returns (address) {
        if (signature.length != 65) return address(0);
        bytes32 r = bytes32(signature[0:32]);
        bytes32 s = bytes32(signature[32:64]);
        uint8 v = uint8(signature[64]);
        if (v < 27) v += 27;
        if ((v != 27 && v != 28) || uint256(s) > SECP256K1N_HALF) return address(0);
        return ecrecover(digest, v, r, s);
    }
}
