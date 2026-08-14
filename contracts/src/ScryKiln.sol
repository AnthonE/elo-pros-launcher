// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "./ReentrancyGuard.sol";

interface IScryItem {
    function instanceOf(uint256 tokenId) external view returns (uint32 catalogId, uint8 mintKind, bytes32 mintRef);
    function ownerOf(uint256 tokenId) external view returns (address);
    function isApprovedForAll(address owner, address operator) external view returns (bool);
    function burn(uint256 tokenId) external;
    function mint(address to, uint32 catalogId, uint8 mintKind, bytes32 mintRef) external returns (uint256);
}

/// @title ScryKiln — a card set goes in, a crest comes out, the cards are gone
/// @notice The craft step of the card layer, and the only new contract that
///         layer needs. Everything else is `ScryItem`: a card is an instance,
///         `catalogId` says which card, `burn` is the sink.
///
///         WHAT IT DOES. `fire` verifies that the submitted tokens are **one of
///         each** card in a declared set and all owned by the caller, burns
///         them, and mints one crest to the same wallet. That is the whole
///         contract. It takes no fee, holds no custody, names no coin and has
///         no payable function — a craft with a price is a knob somebody can
///         move against a player who already spent the cards.
///
///         WHY THE ONE-OF-EACH CHECK IS THE REASON THIS EXISTS. Off chain it
///         would be a promise: a server saying "you completed the set" is a
///         server that could say it for someone who did not. On chain it is the
///         same arithmetic a stranger can rerun — read `instanceOf` for each
///         burned id in the `Fired` event and check the multiset yourself.
///
///         WHY CARDS AND CRESTS ARE TWO CONTRACTS, ENFORCED IN THE CONSTRUCTOR.
///         `ScryGacha` is DEPLOYED and its pools are keyed by **collection
///         address** — `openPool(address collection, ...)`, with no per-catalog
///         filter and no way to add one. So the address is the unit of "what
///         may be in a pool." Cards and crests sharing a contract would mean a
///         crest pool that admits penny cards; only the crest is worth enough
///         to clear `MIN_BACKING_FLOOR`, and that is the whole argument. The
///         constructor refuses to point both handles at one address rather than
///         leaving that as a deploy-time footgun.
///
///         WHAT IT NEEDS BEFORE IT WORKS. To be a **minter on the crest
///         contract** (`ScryItem.setMinter`), and to be **approved by the
///         player on the cards contract** (`setApprovalForAll`, or a per-token
///         `approve` for each card in the set). It never takes tokens into its
///         own custody — it burns from the holder and mints to the holder in
///         one call, so there is no state in which this contract owns anybody's
///         item.
///
///         A SET FREEZES THE FIRST TIME IT FIRES. Until then the owner may
///         correct its composition; after, never. A set whose contents could
///         move after someone assembled it is a set somebody could be robbed
///         of — they bought the last card of a five-card set and woke up
///         holding one fifth of a six-card set.
///
///         THE FORGE GATE IS MET (2026-08-07) — 48/48 across
///         this suite and `ScryItem.t.sol`, inside a full 684/684 run, at
///         foundry.toml's exact settings (solc 0.8.26, optimizer 200, via_ir).
///         Independently reproduced 2026-08-09 to the same 48 and the same
///         sizes: 4,433 B runtime, 20,143 B under EIP-170. Re-run it rather
///         than quoting this comment. What is still owed before a broadcast is
///         not a test: the §8a deploy ceremony, and an operator choosing the
///         parameters.
contract ScryKiln is ReentrancyGuard {
    string public constant NOTICE = "burns one of each card in a declared set and mints one crest to the same wallet; "
        "no fee, no custody, no coin; a set freezes the first time it fires";

    /// Matches `ScryItem.KIND_CRAFT`. A crest is neither a drop, nor a case,
    /// nor a purchase, and `mintKind` is stored per token forever.
    uint8 public constant KIND_CRAFT = 3;

    /// The `seen` bitmap below is one word, so a set can hold at most 256
    /// cards; the real bound is the O(n²) position scan, and Steam's sets are
    /// 5–15. 32 is generous and keeps a fire well inside any block.
    uint256 public constant MAX_SET_SIZE = 32;

    IScryItem public immutable cards;
    IScryItem public immutable crests;

    address public owner;

    struct Set {
        uint32[] cardCatalogIds; // one entry per card in the set, no duplicates
        uint32 crestCatalogId; // what a completed set mints, on the crest contract
        uint8 maxLevel; // how many times one wallet may fire this set (Steam's is 5)
        bool exists;
        bool frozen; // set on the first fire, never unset
    }

    mapping(uint32 => Set) private _sets;

    /// setId => wallet => how many levels that wallet has fired. Also the level
    /// of the NEXT crest, plus one.
    mapping(uint32 => mapping(address => uint8)) public firedBy;

    /// setId => how many crests have ever come out of it. The public supply
    /// counter for a crest catalog, readable without an indexer.
    mapping(uint32 => uint256) public firedTotal;

    event SetDefined(uint32 indexed setId, uint32 indexed crestCatalogId, uint256 size, uint8 maxLevel);
    event SetFrozen(uint32 indexed setId);
    event Fired(
        uint32 indexed setId, address indexed wallet, uint8 level, uint256 indexed crestTokenId, uint256[] burnedTokenIds
    );
    event OwnershipTransferred(address indexed from, address indexed to);

    error NotOwner();
    error ZeroAddress();
    error OneContractForBoth();
    error SetUnknown();
    error SetIsFrozen();
    error BadSetSize();
    error DuplicateCatalog();
    error BadMaxLevel();
    error WrongCardCount();
    error NotYourCard();
    error CardNotInSet();
    error DuplicateCard();
    error LevelCapReached();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    constructor(address cards_, address crests_) {
        if (cards_ == address(0) || crests_ == address(0)) revert ZeroAddress();
        // See the header: the gacha keys pools by address, so one address for
        // both would put penny cards in the crest pool.
        if (cards_ == crests_) revert OneContractForBoth();
        cards = IScryItem(cards_);
        crests = IScryItem(crests_);
        owner = msg.sender;
        emit OwnershipTransferred(address(0), msg.sender);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }

    // ── defining a set ───────────────────────────────────────────────────────

    /// Declare (or, before the first fire, correct) what a set contains.
    ///
    /// `cardCatalogIds` are rows on the CARDS contract; `crestCatalogId` is a
    /// row on the CREST contract. They are different contracts, so the two
    /// number spaces are allowed to overlap and nothing here compares them.
    function defineSet(uint32 setId, uint32[] calldata cardCatalogIds, uint32 crestCatalogId, uint8 maxLevel)
        external
        onlyOwner
    {
        Set storage s = _sets[setId];
        if (s.frozen) revert SetIsFrozen();
        uint256 n = cardCatalogIds.length;
        if (n < 2 || n > MAX_SET_SIZE) revert BadSetSize();
        if (maxLevel == 0) revert BadMaxLevel();

        // A duplicate row would make the set unsatisfiable: `fire` requires one
        // of each and rejects two tokens sharing a catalogId, so a set naming
        // the same card twice could never be completed by anyone.
        for (uint256 i = 0; i < n; i++) {
            for (uint256 j = i + 1; j < n; j++) {
                if (cardCatalogIds[i] == cardCatalogIds[j]) revert DuplicateCatalog();
            }
        }

        s.cardCatalogIds = cardCatalogIds;
        s.crestCatalogId = crestCatalogId;
        s.maxLevel = maxLevel;
        s.exists = true;

        emit SetDefined(setId, crestCatalogId, n, maxLevel);
    }

    // ── firing ───────────────────────────────────────────────────────────────

    /// Burn one of each card in `setId` and mint the crest. The caller must own
    /// every token and must have approved this contract on the cards contract.
    function fire(uint32 setId, uint256[] calldata tokenIds) external nonReentrant returns (uint256 crestTokenId) {
        Set storage s = _sets[setId];
        if (!s.exists) revert SetUnknown();

        uint32[] memory wanted = s.cardCatalogIds;
        uint256 n = wanted.length;
        if (tokenIds.length != n) revert WrongCardCount();

        uint8 level = firedBy[setId][msg.sender] + 1;
        if (level > s.maxLevel) revert LevelCapReached();

        // ── checks: every token is yours, is in the set, and is the only one
        // of its card. `seen` is a bitmap over positions in `wanted`, which is
        // also what makes passing the same tokenId twice fail.
        uint256 seen;
        for (uint256 i = 0; i < n; i++) {
            uint256 tokenId = tokenIds[i];
            if (cards.ownerOf(tokenId) != msg.sender) revert NotYourCard();
            (uint32 catalogId,,) = cards.instanceOf(tokenId);

            uint256 pos = type(uint256).max;
            for (uint256 j = 0; j < n; j++) {
                if (wanted[j] == catalogId) {
                    pos = j;
                    break;
                }
            }
            if (pos == type(uint256).max) revert CardNotInSet();

            uint256 bit = 1 << pos;
            if (seen & bit != 0) revert DuplicateCard();
            seen |= bit;
        }

        // ── effects, before any external call
        firedBy[setId][msg.sender] = level;
        firedTotal[setId] += 1;
        if (!s.frozen) {
            s.frozen = true;
            emit SetFrozen(setId);
        }

        // ── interactions
        for (uint256 i = 0; i < n; i++) {
            cards.burn(tokenIds[i]);
        }

        // The level is inside the token's identity, so `ScryItem.mintRefUsed`
        // makes a repeat unmintable even if this contract's own bookkeeping
        // were ever wrong. 25-byte preimage (4 ‖ 20 ‖ 1); it is hashed, not
        // proved against, so it shares nothing with the merkle dialect.
        bytes32 mintRef = keccak256(abi.encodePacked(setId, msg.sender, level));
        crestTokenId = crests.mint(msg.sender, s.crestCatalogId, KIND_CRAFT, mintRef);

        emit Fired(setId, msg.sender, level, crestTokenId, tokenIds);
    }

    // ── reads a surface asks for ─────────────────────────────────────────────

    function setOf(uint32 setId)
        external
        view
        returns (uint32[] memory cardCatalogIds, uint32 crestCatalogId, uint8 maxLevel, bool exists, bool frozen)
    {
        Set storage s = _sets[setId];
        return (s.cardCatalogIds, s.crestCatalogId, s.maxLevel, s.exists, s.frozen);
    }

    function setSize(uint32 setId) external view returns (uint256) {
        return _sets[setId].cardCatalogIds.length;
    }

    /// How many more times this wallet may fire this set. Zero for an unknown
    /// set, which is the same answer a wallet at its cap gets — the page should
    /// read `setOf` for the difference rather than inferring it here.
    function levelsLeft(uint32 setId, address wallet) external view returns (uint8) {
        Set storage s = _sets[setId];
        if (!s.exists) return 0;
        uint8 fired = firedBy[setId][wallet];
        return fired >= s.maxLevel ? 0 : s.maxLevel - fired;
    }

    /// Whether this contract may burn `wallet`'s cards wholesale. A per-token
    /// `approve` also works and is invisible here, so a `false` means "you may
    /// need to approve", never "you cannot fire".
    function approvedByAll(address wallet) external view returns (bool) {
        return cards.isApprovedForAll(wallet, address(this));
    }
}
