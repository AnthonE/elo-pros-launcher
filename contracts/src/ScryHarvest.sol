// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

/// @title ScryHarvest — merkle claims against the public Augury harvest ledger
/// @notice The meter's harvest ledger (GET /augury/ledger) accrues the GAME
///         COIN — OBOL, from the elastic, distributor-gated spoils ledger — per wallet,
///         deterministically and score-blind (base + streak,
///         SCRY-ECONOMY.md line #1). Games pay game tokens, never SCRY
///         (operator split, 2026-07-20), so deploy this claim contract with
///         the OBOL SpoilsToken address as `token`. The contract is
///         token-generic; nothing below assumes a particular ERC-20. This
///         turns the off-chain ledger into self-serve on-chain claims:
///
///         The operator funds the contract and posts a merkle root over
///         CUMULATIVE lifetime balances — leaf = keccak256(wallet ‖
///         cumulativeAmount), sorted-pair hashing up the tree (identical to
///         ScryVowRegistry.verifyProof, so the whole suite shares one merkle
///         dialect). A wallet claims cumulative − alreadyClaimed, any time,
///         across any number of roots — posting a new root supersedes the
///         old one and nobody has to claim per-epoch.
///
///         Deliberately NOT here: any notion of scores (the ledger being
///         claimed is score-blind upstream), any APY math, any clawback of
///         a wallet's already-claimed amount. The operator can post roots,
///         and sweep UNCOMMITTED surplus only — sweep never dips below what
///         the current root still owes.
///
///         READ THAT PRECISELY (§M4): the contract keeps the promise the
///         OPERATOR POSTED. `totalCommitted_` is supplied by the operator and
///         cannot be checked against the tree on-chain — a merkle root commits
///         to its leaves, never to their sum — so an operator who posts a real
///         root with a false total has stated a smaller obligation, and sweep
///         honours the stated one. What is unconditional: the total is public
///         in `RootPosted` before anyone relies on it, and for `SWEEP_DELAY`
///         after a root change the sweep floor is the HIGHEST obligation stated
///         during that window, so a stated obligation can never be retracted
///         and drained in the same breath — nor by re-posting twice.
///
///         HOW THAT PROMISE IS KEPT (and how it used to break). The
///         outstanding obligation is tracked EXPLICITLY in `owedUnderRoot`:
///         `postRoot` re-arms it to the freshly posted total, and every claim
///         draws it down. It is deliberately NOT derived as
///         `totalCommitted − totalClaimed`, which was the original shape and
///         was wrong: `totalCommitted` describes the CURRENT root while
///         `totalClaimed` is a LIFETIME figure, so re-posting a root without
///         a wallet that had already claimed under an earlier one made the
///         obligation read low — low enough for `sweep` to take money a live
///         claimant was still owed. Re-arming per root is conservative in the
///         only direction that is safe: re-including an already-paid wallet
///         merely over-states what is owed, and over-stating only ever means
///         the operator can sweep LESS.
contract ScryHarvest is ReentrancyGuard {
    IERC20 public immutable scry;
    address public operator;

    bytes32 public root; // over cumulative lifetime amounts
    uint64 public rootCount;
    uint64 public lastRootTime;
    uint256 public totalCommitted; // sum of cumulative amounts under `root`
    uint256 public totalClaimed; // lifetime paid out
    uint256 public owedUnderRoot; // still payable under `root` — the sweep floor
    /// Highest obligation carried into the OPEN SWEEP_DELAY window. It is a
    /// high-water mark, not a running balance: nothing draws it down except
    /// the window lapsing. `postRoot` re-arms the window on every call, so
    /// under a root cadence tighter than SWEEP_DELAY this ratchets to the
    /// largest obligation ever stated and stays there — the operator's release
    /// valve is one 7-day gap with no re-post, after which `carried` computes
    /// as 0 and the floor drops to whatever the current root owes.
    /// That over-hold is deliberate and is bounded by the largest drop ever
    /// posted; the alternative, drawing it down on claims, discharged one
    /// wallet's obligation with another wallet's claim (fixed 2026-07-28).
    uint256 public priorOwed;

    /// §M4. `postRoot` takes `totalCommitted_` from the operator and cannot
    /// check it against the tree — a merkle root commits to its leaves, not to
    /// their sum, so there is no on-chain arithmetic that would catch a wrong
    /// total. Post a real root with `totalCommitted_ = 0`, sweep the balance,
    /// and every claimant's `claim` reverts on an empty contract.
    ///
    /// The promise "a posted root is a kept promise" is therefore
    /// operator-conditional, and this is the part that is NOT conditional: for
    /// SWEEP_DELAY after any root, `sweep` respects the HIGHEST obligation
    /// stated during that window — carried across any number of intervening
    /// re-posts, so the guarantee cannot be walked down one root at a time
    /// (see `postRoot`). Dropping the stated obligation is still
    /// possible, still the operator's prerogative, and now (a) visible in
    /// `RootPosted` before it can be acted on and (b) not instant — claimants
    /// have a posted window to take what the old root owed them. Conservative
    /// in the only safe direction: over-stating what is owed only ever means
    /// the operator may sweep LESS.
    ///
    /// ⚠ THE WINDOW HOLDS FUNDS; IT DOES NOT PRESERVE A PROOF. There is one
    /// `root` and no history, so `claim` verifies against whatever root is
    /// current. The window defends the §M4 retraction — a REAL root re-posted
    /// with a falsified total — where the tree is unchanged, every old proof
    /// still verifies, and the balance is held for seven days so claimants can
    /// take what they are owed. It does NOT help against a root that replaces
    /// the leaf set: that closes every old proof immediately, and the held
    /// balance is then unreachable by the wallets it was held for. Republish
    /// the leaf to reopen it — roots are republishable precisely so that
    /// nobody forfeits by claiming late.
    uint64 public constant SWEEP_DELAY = 7 days;

    mapping(address => uint256) public claimed; // wallet => lifetime claimed

    event RootPosted(bytes32 indexed root, uint256 totalCommitted, string ledgerRef);
    event Claimed(address indexed wallet, uint256 amount, uint256 cumulative);
    event Swept(address indexed to, uint256 amount);
    event OperatorTransferred(address indexed from, address indexed to);

    modifier onlyOperator() {
        require(msg.sender == operator, "not operator");
        _;
    }

    constructor(IERC20 _scry) {
        scry = _scry;
        operator = msg.sender;
    }

    /// Post a new root over cumulative balances. `totalCommitted_` is the sum
    /// of every leaf's cumulative amount — published so anyone can check the
    /// contract is funded to cover it. `ledgerRef` points at the public
    /// off-chain ledger snapshot the root was built from.
    /// @dev `owedUnderRoot` is re-armed to the whole posted total, NOT reduced
    ///      by what earlier roots already paid. A wallet re-listed at an
    ///      unchanged cumulative amount can claim nothing more, so counting it
    ///      again over-states the obligation — which only ever shrinks what
    ///      `sweep` may take. Under-stating it is the failure that matters.
    function postRoot(bytes32 newRoot, uint256 totalCommitted_, string calldata ledgerRef) external onlyOperator {
        // Remember what the outgoing root still owed — see SWEEP_DELAY. Held
        // static for the window (claims draw down `owedUnderRoot`, not this),
        // which over-states the floor and so only ever sweeps less.
        //
        // CARRIED, never overwritten, while the window is open. `priorOwed =
        // owedUnderRoot` was the original shape and it voided the guarantee
        // this field exists for: the SECOND consecutive re-post read the
        // already-retracted `owedUnderRoot` back into `priorOwed`, so
        // 1500 → 0 → 0 collapsed the floor to zero in three calls in one
        // block, and `sweep` then emptied a contract whose claimants were
        // still owed. Taking the MAX of the two keeps the highest obligation
        // stated in the window alive for the whole window, whatever number of
        // roots pass through it, while an expired window still drops to
        // whatever the current root owes.
        uint256 carried = block.timestamp < uint256(lastRootTime) + SWEEP_DELAY ? priorOwed : 0;
        priorOwed = owedUnderRoot > carried ? owedUnderRoot : carried;
        root = newRoot;
        totalCommitted = totalCommitted_;
        owedUnderRoot = totalCommitted_;
        rootCount += 1;
        lastRootTime = uint64(block.timestamp);
        emit RootPosted(newRoot, totalCommitted_, ledgerRef);
    }

    /// Claim everything you're owed under the current root. Callable by
    /// anyone FOR anyone — payout always goes to `wallet`, so gasless agents
    /// can be claimed-for by their operators.
    function claim(address wallet, uint256 cumulative, bytes32[] calldata proof) external nonReentrant returns (uint256 amount) {
        bytes32 leaf = keccak256(abi.encodePacked(wallet, cumulative));
        require(_verify(proof, root, leaf), "bad proof");
        uint256 already = claimed[wallet];
        require(cumulative > already, "nothing to claim");
        amount = cumulative - already;
        claimed[wallet] = cumulative;
        totalClaimed += amount;
        // Saturating: a root posted with a total SMALLER than the sum of its
        // own leaves is an operator bookkeeping error, and a claimant must
        // never eat a revert for it.
        owedUnderRoot = amount >= owedUnderRoot ? 0 : owedUnderRoot - amount;
        // `priorOwed` IS DELIBERATELY NOT TOUCHED HERE, and this line used to
        // draw it down "because the obligation was paid". It is a DIFFERENT
        // obligation: `priorOwed` carries what the PREVIOUS root still owed,
        // and a claim under the CURRENT root discharges none of it unless the
        // claimant happened to be in both — which nothing here can know.
        //
        // The honest-error case it broke: root1 = {alice: 1500}. The ledger is
        // regenerated, a generator bug drops alice, and root2 = {bob: 1000} is
        // posted. `priorOwed` = 1500 correctly holds alice's claim alive for
        // the window. Bob claims his legitimate 1000, this line took priorOwed
        // to 500, and `sweepFloor()` — the view an operator reads to decide
        // what is safe to sweep — reported 500 with five of alice's seven days
        // unspent. `postRoot` states the invariant this restores (:114): held
        // static for the window, claims draw down `owedUnderRoot`, not this.
        //
        // Leaving it alone only ever means the operator may sweep LESS during
        // the window, which is the sole safe direction (:83-84).
        SafeERC20.safeTransfer(address(scry), wallet, amount, "transfer failed");
        emit Claimed(wallet, amount, cumulative);
    }

    /// Withdraw surplus only — the balance beyond what the current root still
    /// owes, and for SWEEP_DELAY after a root change, beyond what the PREVIOUS
    /// one owed too (§M4). A posted root can always be fully claimed, and a
    /// posted obligation cannot be retracted out from under a claimant in the
    /// same block it is lowered.
    function sweep(address to, uint256 amount) external onlyOperator nonReentrant {
        uint256 bal = scry.balanceOf(address(this));
        require(bal >= sweepFloor() + amount, "would break the posted root");
        SafeERC20.safeTransfer(address(scry), to, amount, "transfer failed");
        emit Swept(to, amount);
    }

    /// What `sweep` must leave behind right now — the current obligation, or
    /// the previous one while the delay window is open, whichever is larger.
    /// A view so anyone can check the contract before trusting a root.
    function sweepFloor() public view returns (uint256) {
        if (block.timestamp < uint256(lastRootTime) + SWEEP_DELAY && priorOwed > owedUnderRoot) {
            return priorOwed;
        }
        return owedUnderRoot;
    }

    function transferOperator(address to) external onlyOperator {
        require(to != address(0), "zero");
        emit OperatorTransferred(operator, to);
        operator = to;
    }

    /// Same sorted-pair keccak256 dialect as ScryVowRegistry.verifyProof.
    function _verify(bytes32[] calldata proof, bytes32 root_, bytes32 leaf) internal pure returns (bool) {
        bytes32 h = leaf;
        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 p = proof[i];
            h = h < p ? keccak256(abi.encodePacked(h, p)) : keccak256(abi.encodePacked(p, h));
        }
        return h == root_;
    }
}
