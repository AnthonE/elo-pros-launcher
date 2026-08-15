// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "./ReentrancyGuard.sol";
import {SafeERC20} from "./SafeERC20.sol";

interface IERC721Like {
    function transferFrom(address from, address to, uint256 tokenId) external;
    function ownerOf(uint256 tokenId) external view returns (address);
}

interface IERC721Receiver {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

/// @title ScryTrove — many collections, one pot
/// @notice **The problem, and it is arithmetic rather than taste.** `ScryGacha`
///         keys its pools by collection — `mapping(address => Pool)`, and
///         `deposit(collection, tokenId)` writes `pools[collection]`. It is
///         deployed at `0xb63Ce4F3…` and welded, so one pool per collection is
///         not a preference anyone can revise.
///
///         That is fine for a collection with a thousand holders and a real
///         spread. **It is fatal for a game launching its first ten items**,
///         because a gacha's whole product is the SPREAD: put ten similar items
///         in a pool and a ticket costs more than the best prize. Measured —
///         *ten positions all at 2 ETH: a ticket costs 2.20 and the biggest
///         prize is 1.70.* **A uniform pool is a terrible gacha no matter how
///         valuable its contents.**
///
///         So this contract is a **wrapper**: escrow any ERC-721, mint one
///         token of THIS collection against it, and hand that to the gacha.
///         Every wrapped token shares one address, so the gacha sees one
///         collection and opens one pool — and that pool holds a house mint, a
///         dev's items, an artist's one-off and somebody's CryptoPunk-equivalent
///         side by side. **The pot stops being per-collection without the gacha
///         changing by a single byte.**
///
///         ═══ IT ALSO WRAPS ERC-20s, AND WHAT THAT DOES AND DOES NOT COST ═══
///
///         `wrapTokens(token, amount)` escrows a fungible the same way — the
///         donation shape (operator, 2026-08-11: *"maybe people will just
///         donate to it"*).
///
///         ⛔ **An earlier version of this header refused that path**, arguing
///         from FWA's Token Packs — an ERC-20 → 721 wrapper feeding their own
///         gacha, read once and written off as *"must not be copied."* The
///         argument here was that the gacha assumes the drawn
///         object's worth is **opaque**, so wrapping something with a public
///         price makes the backing and the contents drift apart by themselves.
///
///         **The operator's objection was one line and it is right:** *"NFTs
///         have a price too so i dont see the harm."* The split was written as a
///         binary and it is a spectrum — and this repo's own §1c already
///         falsified the clean version, because a StonkBroker is instantly
///         redeemable into an AMM vault for exactly 666,666 `$STONKBROKER`,
///         a **sharper** price than most ERC-20 positions carry.
///
///         **What survives, checked against the Book rather than asserted.**
///         Every number the gacha publishes — `chance_ppm`, the price, the
///         −22.727% floor-only edge, `break_even_value` — is a function of
///         BACKINGS alone. **Not one of them breaks when the drawn object's
///         worth is public.** What changes is that the buyer's private `?value=`
///         input becomes computable, so the buyer stops guessing. That transfers
///         value from depositor to buyer whenever backing sits below contents —
///         which is **exactly** the risk the gacha already carries for NFTs:
///         *back it at what it is actually worth,* or *somebody
///         will draw you and take the bid, cheaply, on purpose, and repeatedly.*
///         A fungible only makes the counterparty's arithmetic easier. Same
///         rule, not a new one.
///
///         ⚠ **So the one thing to get right is a GIFT, not a mechanic.** A
///         donated position backed BELOW its contents is a free option and a bot
///         takes it first, so the gift lands with one arbitrageur instead of the
///         players. It is worse than that: the price is the harmonic mean, which
///         is dominated by the cheapest position, so an under-backed donation
///         drags the whole pool toward dust *while being the thing everyone
///         wants.* **Back a donation at or above what it holds** and the gift
///         arrives as cheaper tickets across the pool — where it was aimed —
///         instead of as a lootable hole.
///
///         §1d is therefore **narrowed, not discarded**: what made Token Packs
///         unsafe to copy was **their own token, transfer-restricted, behind a
///         house allowlist**, not that a wrapped thing has a price. The half of
///         that finding which always applied is the balance-delta pull, and
///         `wrapTokens` uses it.
///
///         ⚠ **One asymmetry a wrapper of fungibles really does carry.** If a
///         wrapped token pauses or blacklists, `unwrap` reverts and the holder
///         waits. That is inherent — the wrapper is a claim on that token — and
///         it is disclosed rather than fixed. **The pool is never jammed by it**,
///         which is the property that matters: this contract's own
///         `transferFrom` is plain and cannot revert, so a sick wrapped token
///         strands its own holder and never a gacha position.
///
///         ═══ WHAT THIS CONTRACT DELIBERATELY IS NOT ═══
///
///         **It is not a curator.** Anyone may wrap anything. The curation is
///         where it always was — the operator opening a gacha pool, by hand,
///         with no queue (`GATES.md` §3). This contract's permissionlessness is
///         not a policy about which items deserve a floor; it is the absence of
///         a gate nobody asked for.
///
///         **It holds no key over what it escrows.** There is no admin
///         withdraw, no pause, no allowlist and no upgrade path. The only way an
///         escrowed token leaves is `unwrap`, callable by the wrapper's owner.
///         That is not generosity — it is the §2d compatibility rule pointed at
///         ourselves. A wrapper with a pause switch could strand a position
///         mid-draw and stop the whole pool selling, which is precisely what
///         `meter/gacha_fit.py` refuses other people's contracts for.
///
///         **It is not a marketplace and takes no fee.** Wrapping and
///         unwrapping are free. The house's take is the gacha's posted bps and
///         nothing else; a second toll here would be a fee on a fee.
///
///         ⚠ **`transferFrom` is plain and MUST stay plain.** `ScryGacha` moves
///         tokens with `transferFrom`, not `safeTransferFrom`, then asserts
///         `ownerOf == address(this)`. Anything in this contract's transfer path
///         that can revert — a hook, a pause, a blocklist — strands a drawn
///         position and jams the pool. There is nothing in it today and nothing
///         may be added.
contract ScryTrove is ReentrancyGuard {
    string public constant NOTICE =
        "a wrapper so one gacha pool can hold many collections and donated tokens - no fee, "
        "no admin key, no pause; unwrap is always open and returns exactly what went in";

    string public name = "Scry Trove";
    string public symbol = "TROVE";

    /// @notice What a wrapper token stands for. Immutable once written: the
    ///         whole promise of the wrapper is that it returns the EXACT token
    ///         that went in, and a mutable pointer here would be an admin door
    ///         onto somebody else's NFT wearing a struct.
    struct Held {
        address collection; // the ERC-721, or the ERC-20 when `fungible`
        uint256 tokenId; // the token id, or the AMOUNT when `fungible`
        address depositor; // who wrapped it — a record, never a permission
        // APPENDED, following `ScryGacha.Position`'s habit even though nothing
        // decodes this positionally yet: new state goes on the end, always.
        bool fungible; // false = one ERC-721. true = an amount of an ERC-20
    }

    mapping(uint256 => Held) public held;

    /// @notice collection => tokenId => the live wrapper id, or 0. Also the
    ///         guard that stops the same token being wrapped twice.
    mapping(address => mapping(uint256 => uint256)) public wrapperOf;

    uint256 public totalWrapped; // live wrappers; falls on unwrap
    uint256 public nextId = 1; // ids are never reused — a burned id stays burned

    // ── ERC-721 ──────────────────────────────────────────────────────────────
    mapping(uint256 => address) private _owner;
    mapping(address => uint256) private _balance;
    mapping(uint256 => address) private _approved;
    mapping(address => mapping(address => bool)) private _operator;

    /// @notice Where a wallet may read what a wrapper holds without an indexer.
    ///         `ScryGacha` never asks a collection for metadata (its
    ///         `IERC721Minimal` is `transferFrom` + `ownerOf`), so this is for
    ///         humans and marketplaces only and can never affect a draw.
    string public baseURI;

    /// @notice Set once at deploy and never again. A repointable base on a
    ///         contract holding other people's tokens is an admin door onto what
    ///         a buyer thinks they are drawing.
    address public immutable deployer;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);

    event Wrapped(
        uint256 indexed wrapperId,
        address indexed collection,
        uint256 indexed tokenId,
        address depositor
    );
    event Unwrapped(
        uint256 indexed wrapperId,
        address indexed collection,
        uint256 indexed tokenId,
        address to
    );
    /// @dev A SEPARATE event rather than a flag on `Wrapped`, for the reason
    ///      `ScryGacha`'s toll events are separate: an indexer that only ever
    ///      knew about the 721 path must not read an AMOUNT as a token id.
    event WrappedTokens(
        uint256 indexed wrapperId,
        address indexed token,
        uint256 amount,
        address depositor
    );

    error AlreadyWrapped();
    error NotAContract();
    error NotTheOwner();
    error NotWrapped();
    error PullFailed();
    error ReturnFailed();
    error ZeroAmount();
    error ZeroTo();

    constructor(string memory baseURI_) {
        deployer = msg.sender;
        baseURI = baseURI_;
    }

    // ── wrap / unwrap ────────────────────────────────────────────────────────

    /// @notice Escrow `collection`/`tokenId` and mint you a wrapper for it.
    ///         Approve this contract for the token first.
    ///
    /// @dev The pull is `transferFrom` + an `ownerOf` assertion, exactly the
    ///      shape `ScryGacha.deposit` uses — deliberately, because a collection
    ///      that cannot survive this pull cannot survive the gacha's either, and
    ///      it is better to find that out here than with a position stranded
    ///      mid-draw. A collection with a reverting hook simply fails to wrap.
    function wrap(address collection, uint256 tokenId)
        external
        nonReentrant
        returns (uint256 wrapperId)
    {
        require(collection.code.length > 0, NotAContract());
        require(wrapperOf[collection][tokenId] == 0, AlreadyWrapped());

        IERC721Like(collection).transferFrom(msg.sender, address(this), tokenId);
        require(IERC721Like(collection).ownerOf(tokenId) == address(this), PullFailed());

        wrapperId = nextId++;
        held[wrapperId] =
            Held({collection: collection, tokenId: tokenId, depositor: msg.sender, fungible: false});
        wrapperOf[collection][tokenId] = wrapperId;
        totalWrapped += 1;

        _mint(msg.sender, wrapperId);
        emit Wrapped(wrapperId, collection, tokenId, msg.sender);
    }

    /// @notice Escrow `amount` of an ERC-20 and mint you a wrapper holding it —
    ///         the donation shape. Approve this contract first.
    ///
    /// @dev **`wrapperOf` is deliberately NOT written here.** That mapping is
    ///      the never-wrap-the-same-token-twice guard, and it is meaningless for
    ///      a fungible: ten people may each wrap 500 OBOL and each gets their
    ///      own wrapper. Writing it would make the first pack of a token block
    ///      every later one.
    ///
    /// @dev The pull is `SafeERC20.pullExact`, which measures the balance across
    ///      the transfer and reverts unless it rose by exactly `amount`. Not
    ///      ceremony: a fee-on-transfer token would deliver less than the
    ///      wrapper claims, and then `unwrap` would promise a number this
    ///      contract does not hold — the wrapper would be short and the last
    ///      holder out would eat it. That came out of the FWA Token Packs
    ///      read, and it is the half of that finding that always applied.
    function wrapTokens(address token, uint256 amount)
        external
        nonReentrant
        returns (uint256 wrapperId)
    {
        require(token.code.length > 0, NotAContract());
        require(amount > 0, ZeroAmount());

        SafeERC20.pullExact(token, msg.sender, address(this), amount, "token pull failed");

        wrapperId = nextId++;
        held[wrapperId] =
            Held({collection: token, tokenId: amount, depositor: msg.sender, fungible: true});
        totalWrapped += 1;

        _mint(msg.sender, wrapperId);
        emit WrappedTokens(wrapperId, token, amount, msg.sender);
    }

    /// @notice Burn a wrapper you own and take its token back.
    ///
    /// @dev **Permissionless for the holder and gated on nothing else.** Whoever
    ///      holds the wrapper holds the token — including a gacha buyer who kept
    ///      it, which is the whole point: a draw hands you a wrapper and you
    ///      unwrap it into the real thing. The original depositor's address is
    ///      recorded and confers NOTHING; a wrapper that could be recalled by
    ///      the person who made it would not be a thing anyone should have paid
    ///      for.
    function unwrap(uint256 wrapperId) external nonReentrant {
        require(_owner[wrapperId] == msg.sender, NotTheOwner());
        Held memory h = held[wrapperId];
        require(h.collection != address(0), NotWrapped());

        // State first, then the outbound call. The wrapper is burned and the
        // books cleared before anything external runs.
        delete held[wrapperId];
        if (!h.fungible) delete wrapperOf[h.collection][h.tokenId];
        totalWrapped -= 1;
        _burn(wrapperId);

        if (h.fungible) {
            // ⚠ If this token pauses or blacklists, THIS call reverts and the
            // holder waits — inherent to holding a claim on it, disclosed in
            // the header, and never able to jam a gacha pool, because the
            // wrapper's own transfer path (below) cannot revert.
            SafeERC20.safeTransfer(h.collection, msg.sender, h.tokenId, "token return failed");
        } else {
            IERC721Like(h.collection).transferFrom(address(this), msg.sender, h.tokenId);
            require(IERC721Like(h.collection).ownerOf(h.tokenId) == msg.sender, ReturnFailed());
        }
        emit Unwrapped(wrapperId, h.collection, h.tokenId, msg.sender);
    }

    /// @notice What a wrapper is standing for, in one read. The seam a card,
    ///         the gacha's position list and a wallet all use, so nothing has to
    ///         decode the struct positionally.
    ///         ⚠ `tokenId` is an AMOUNT when `fungible` is true. Any surface
    ///         that renders it must branch, or it prints 500000000000000000000
    ///         where a token id belongs.
    function contentsOf(uint256 wrapperId)
        external
        view
        returns (address collection, uint256 tokenId, address depositor, bool live, bool fungible)
    {
        Held memory h = held[wrapperId];
        return (h.collection, h.tokenId, h.depositor, h.collection != address(0), h.fungible);
    }

    // ── ERC-721, hand-rolled to stay boring ──────────────────────────────────
    // `ScryEidolon` is the template. Nothing here overrides a transfer, and no
    // hook, pause, filter or royalty is present — see the header's last note.

    function ownerOf(uint256 tokenId) public view returns (address o) {
        o = _owner[tokenId];
        require(o != address(0), NotWrapped());
    }

    function balanceOf(address who) external view returns (uint256) {
        return _balance[who];
    }

    function totalSupply() external view returns (uint256) {
        return totalWrapped;
    }

    function approve(address to, uint256 tokenId) external {
        address o = ownerOf(tokenId);
        require(msg.sender == o || _operator[o][msg.sender], NotTheOwner());
        _approved[tokenId] = to;
        emit Approval(o, to, tokenId);
    }

    function getApproved(uint256 tokenId) external view returns (address) {
        return _approved[tokenId];
    }

    function setApprovalForAll(address op, bool okay) external {
        _operator[msg.sender][op] = okay;
        emit ApprovalForAll(msg.sender, op, okay);
    }

    function isApprovedForAll(address o, address op) external view returns (bool) {
        return _operator[o][op];
    }

    function transferFrom(address from, address to, uint256 tokenId) public {
        address o = ownerOf(tokenId);
        require(o == from, NotTheOwner());
        require(to != address(0), ZeroTo());
        require(
            msg.sender == o || _approved[tokenId] == msg.sender || _operator[o][msg.sender],
            NotTheOwner()
        );
        _approved[tokenId] = address(0);
        _balance[from] -= 1;
        _balance[to] += 1;
        _owner[tokenId] = to;
        emit Transfer(from, to, tokenId);
    }

    function safeTransferFrom(address from, address to, uint256 tokenId) external {
        safeTransferFrom(from, to, tokenId, "");
    }

    function safeTransferFrom(address from, address to, uint256 tokenId, bytes memory data) public {
        transferFrom(from, to, tokenId);
        if (to.code.length > 0) {
            require(
                IERC721Receiver(to).onERC721Received(msg.sender, from, tokenId, data)
                    == IERC721Receiver.onERC721Received.selector,
                ReturnFailed()
            );
        }
    }

    /// @notice ERC-165. **ERC-2981 is deliberately absent**: the gacha pays no
    ///         royalty, and a wrapper claiming one would advertise a payment
    ///         nothing here makes. The creator's own royalty lives on the
    ///         creator's own contract and is untouched by wrapping.
    function supportsInterface(bytes4 id) external pure returns (bool) {
        return id == 0x01ffc9a7 // ERC-165
            || id == 0x80ac58cd // ERC-721
            || id == 0x5b5e139f; // ERC-721Metadata
    }

    function tokenURI(uint256 tokenId) external view returns (string memory) {
        ownerOf(tokenId); // reverts on a token that does not exist
        return bytes(baseURI).length == 0 ? "" : string.concat(baseURI, _u(tokenId));
    }

    // ── internals ────────────────────────────────────────────────────────────
    function _mint(address to, uint256 tokenId) internal {
        _owner[tokenId] = to;
        _balance[to] += 1;
        emit Transfer(address(0), to, tokenId);
    }

    function _burn(uint256 tokenId) internal {
        address o = _owner[tokenId];
        _approved[tokenId] = address(0);
        _balance[o] -= 1;
        delete _owner[tokenId];
        emit Transfer(o, address(0), tokenId);
    }

    function _u(uint256 x) internal pure returns (string memory s) {
        if (x == 0) return "0";
        uint256 n = x;
        uint256 len;
        while (n != 0) {
            len++;
            n /= 10;
        }
        bytes memory b = new bytes(len);
        while (x != 0) {
            b[--len] = bytes1(uint8(48 + x % 10));
            x /= 10;
        }
        s = string(b);
    }
}
