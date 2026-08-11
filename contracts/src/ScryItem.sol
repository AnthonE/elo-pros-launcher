// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "./ReentrancyGuard.sol";

interface IERC721Receiver {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

/// @title ScryItem — one catalog's items, and not one trait among them
/// @notice The item contract from `docs/items/ITEM-CONTRACT.md`. One deployment per
///         catalog (`TOPOLOGY.md` §2 — marketplace indexing decides it before
///         security does), ERC-721 for everything a player collects, and
///         **owned by the creator, never by the house** (operator, 2026-08-04).
///
///         WHAT IT STORES, WHICH IS ALMOST NOTHING. A token is
///         `(catalogId, mintKind, mintRef)` — 37 bytes. No float, no pattern,
///         no StatTrak, no rarity, no tier. Every attribute is
///         `H(mintRef ‖ field_tag)` mapped into that field's declared range by
///         the published derivation spec, so **nothing about rarity is stored
///         and therefore nothing about rarity can be edited** — not by the
///         creator, not by a compromised key, not by a migration. That is
///         `SKINS.md` §1 as bytecode instead of as a paragraph, and it is why
///         this contract can ship BEFORE the derivation spec exists: it commits
///         to a `mintRef` and renders later.
///
///         `mintRefUsed` makes a dupe unmintable rather than merely detectable.
///         It is never cleared — burning an instance does not free its
///         `mintRef`, so a burned drop can never be re-minted.
///
///         MINT AUTHORITY IS DISCLOSED, NOT RESTRICTED. An earlier design made
///         "no owner mint" the load-bearing invariant. Wrong conclusion from
///         right reasoning: the claim was never *nobody can mint*, it is **you
///         can check**. So the minter set is public, `renounceMint` is a real
///         and permanent door, and the game card reads all of it off chain
///         (`GATES.md` §3.4 — the platform requires disclosure and never
///         dictates design). A creator who closes their mint gets to say the
///         supply is final AND point at the transaction, which is strictly
///         stronger than Rust's retirement — that one is a policy promise.
///
///         ⚠ WHAT `renounceMint` DOES AND DOES NOT DO. It permanently stops
///         discretionary minting: no minter may mint, no new drop root may be
///         posted, no minter may be added. It deliberately does NOT kill the
///         drop root already posted, because that would strand claimants
///         holding valid proofs. So after renouncing, supply is bounded by
///         `totalSupply()` plus whatever the standing root still owes — and the
///         root is public, so anyone can count its leaves. A creator wanting a
///         hard number renounces once the claims are done. Say it that way on
///         the card; do not print "supply final" while a root is live.
///
///         ⚠ IT IS A BORING ERC-721 ON PURPOSE, and that is a compatibility
///         requirement rather than a style. `ScryGacha` is DEPLOYED, moves
///         tokens with `transferFrom` (not `safeTransferFrom`), and holds them
///         across a draw window. A transfer hook that can revert, a pause, a
///         soulbind, an allowlist or a fee taken in `transferFrom` strands a
///         position mid-draw and blocks the whole pool, which serializes one
///         draw at a time. Nothing clever goes in the transfer path. Ever.
///
///         ROYALTY IS IMMUTABLE because a royalty the deployer can redirect
///         later is not a payment. The house's own resale cut is settled at
///         zero (`SENTENCES.md` 2026-08-03); this contract takes the number
///         from its deployer, so a house catalog deploys with 0 and a
///         third-party creator sets their own. Capped at 10% — the same
///         ceiling `ScryEidolon` carries. A creator who wants more deploys
///         something else and wears the "unknown code" badge (§1b).
contract ScryItem is ReentrancyGuard {
    string public constant NOTICE = "one catalog's items - traits are NOT stored, they derive from mintRef by the "
        "published spec; mint authority is public and renounceable; royalty immutable; deliberately a boring ERC-721";

    uint8 public constant KIND_PLAY = 0;
    uint8 public constant KIND_CASE = 1;
    uint8 public constant KIND_STORE = 2;
    /// Minted by consuming other items — `ScryKiln` firing a card set into a
    /// crest (`CARDS.md` §7), and any future trade-up (`SKINS.md` §12 row 6).
    /// Added 2026-08-04, BEFORE any deploy, and that timing is the whole
    /// argument: `mintKind` is stored per token and this contract is welded the
    /// moment it ships, so a crest minted as `KIND_CASE` would carry the wrong
    /// provenance forever with no migration available. Off chain we correct
    /// copy; on chain we do not get to.
    uint8 public constant KIND_CRAFT = 3;

    /// The whole of an item. 37 bytes, and not one of them is a trait.
    struct Instance {
        uint32 catalogId; // which skin — a row in the title's content/*.toml
        uint8 mintKind; // 0 play · 1 case · 2 store
        bytes32 mintRef; // play: keccak(shard ‖ wal ‖ tick ‖ entity) · store: keccak(catalogId ‖ edition)
    }

    string public name;
    string public symbol;

    address public immutable royaltyReceiver;
    uint96 public immutable royaltyBps;

    address public owner; // the CREATOR. admin: minters, drop root, base URI
    string public baseURI;

    uint256 public nextTokenId = 1;
    uint256 public burned;
    bool public mintClosed;
    bytes32 public dropRoot;
    uint256 public minterCount;

    mapping(uint256 => Instance) public instanceOf;
    mapping(bytes32 => bool) public mintRefUsed;
    mapping(address => bool) public isMinter;

    mapping(uint256 => address) private _owner;
    mapping(address => uint256) private _balance;
    mapping(uint256 => address) private _approved;
    mapping(address => mapping(address => bool)) private _operator;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);
    event ItemMinted(uint256 indexed tokenId, address indexed to, uint32 indexed catalogId, uint8 mintKind, bytes32 mintRef);
    event Claimed(uint256 indexed tokenId, address indexed wallet, bytes32 indexed root);
    event MinterSet(address indexed who, bool allowed);
    event MintRenounced();
    event DropRootPosted(bytes32 indexed root);
    event OwnershipTransferred(address indexed from, address indexed to);
    event ContractURIUpdated(); // ERC-7572
    event MetadataUpdate(uint256 _tokenId); // ERC-4906
    event BatchMetadataUpdate(uint256 _fromTokenId, uint256 _toTokenId); // ERC-4906

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    /// @param _royaltyReceiver where ERC-2981 points, forever. A house catalog
    ///        passes the splitter with `_royaltyBps = 0`; a third-party creator
    ///        passes their own address and their own number.
    constructor(
        string memory _name,
        string memory _symbol,
        address _royaltyReceiver,
        uint96 _royaltyBps,
        string memory _baseURI
    ) {
        require(bytes(_name).length != 0, "no name");
        require(_royaltyBps <= 1000, "royalty > 10%");
        require(_royaltyReceiver != address(0) || _royaltyBps == 0, "zero receiver");
        name = _name;
        symbol = _symbol;
        royaltyReceiver = _royaltyReceiver;
        royaltyBps = _royaltyBps;
        baseURI = _baseURI;
        owner = msg.sender;
        emit OwnershipTransferred(address(0), msg.sender);
        emit ContractURIUpdated();
    }

    // ── the creator's admin surface — all of it public, all of it on the card ─

    function transferOwnership(address newOwner) external onlyOwner {
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner; // address(0) renounces: base URI and minter set freeze
    }

    /// Add or remove a minter. Deliberately unenumerable on chain — the card
    /// reads `MinterSet` from the explorer and re-checks each address with
    /// `isMinter` through RPC. That is `CLAUDE.md`'s source division, not an
    /// omission: the explorer answers *who*, an `eth_call` answers *is it still
    /// true*. `minterCount` is the cheap state answer for "is anyone able to
    /// mint at all".
    function setMinter(address who, bool allowed) external onlyOwner {
        require(!mintClosed, "mint closed");
        require(who != address(0), "zero minter");
        if (isMinter[who] == allowed) return;
        isMinter[who] = allowed;
        if (allowed) minterCount += 1;
        else minterCount -= 1;
        emit MinterSet(who, allowed);
    }

    /// Post the drop root that `claim` verifies against. Replaces the previous
    /// one — an unclaimed leaf under a superseded root is no longer claimable,
    /// so a root is replaced only by one that carries the old leaves forward.
    function setDropRoot(bytes32 root) external onlyOwner {
        require(!mintClosed, "mint closed");
        dropRoot = root;
        emit DropRootPosted(root);
    }

    /// Close the mint forever. No minter may mint, no root may be posted, no
    /// minter may be added — and the standing root stays claimable, which is
    /// the whole of the caveat in this contract's header. There is no reopen.
    function renounceMint() external onlyOwner {
        mintClosed = true;
        emit MintRenounced();
    }

    /// Repoint the metadata base and signal marketplaces to re-pull. ⚠ This
    /// moves ART. It cannot move a trait, because no trait is stored — the
    /// token's truth is `mintRefOf` plus the published derivation, and both are
    /// recomputable by a stranger who has never heard of this host.
    function setBaseURI(string calldata newBase) external onlyOwner {
        baseURI = newBase;
        emit ContractURIUpdated();
        if (nextTokenId > 1) emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    // ── minting ──────────────────────────────────────────────────────────────

    function mint(address to, uint32 catalogId, uint8 mintKind, bytes32 mintRef)
        external
        nonReentrant
        returns (uint256 tokenId)
    {
        require(isMinter[msg.sender], "not a minter");
        require(!mintClosed, "mint closed");
        tokenId = _mint(to, catalogId, mintKind, mintRef);
    }

    /// N instances of ONE catalog to ONE wallet. This is both the store-edition
    /// shape (ten thousand copies of a livery are ten thousand tokens sharing a
    /// `catalogId`, differing only by edition — what CS and Steam already
    /// render) and the placeholder shape for wiring a surface end to end.
    function mintBatch(address to, uint32 catalogId, uint8 mintKind, bytes32[] calldata mintRefs)
        external
        nonReentrant
        returns (uint256 firstTokenId)
    {
        require(isMinter[msg.sender], "not a minter");
        require(!mintClosed, "mint closed");
        require(mintRefs.length != 0, "empty batch");
        firstTokenId = nextTokenId;
        for (uint256 i = 0; i < mintRefs.length; i++) {
            _mint(to, catalogId, mintKind, mintRefs[i]);
        }
    }

    /// Claim a drop leaf. Callable by anyone FOR anyone — the token always goes
    /// to `wallet`, so a gasless agent can be claimed for by its operator
    /// (`ScryHarvest`'s stated pattern, same merkle dialect).
    ///
    /// Lazy mint is the point: a drop is a leaf until its owner wants it on
    /// chain, so an item nobody trades never becomes a token.
    ///
    /// ⚠ NOT gated by `mintClosed`, deliberately — see the header.
    function claim(address wallet, uint32 catalogId, uint8 mintKind, bytes32 mintRef, bytes32[] calldata proof)
        external
        nonReentrant
        returns (uint256 tokenId)
    {
        bytes32 root = dropRoot;
        require(root != bytes32(0), "no drop root");
        // 57-byte preimage (20 ‖ 4 ‖ 1 ‖ 32), so a leaf can never be read as an
        // internal node (64 bytes) — the second-preimage guard the whole repo
        // relies on, stated once here because it is load-bearing.
        bytes32 leaf = keccak256(abi.encodePacked(wallet, catalogId, mintKind, mintRef));
        require(_verify(proof, root, leaf), "bad proof");
        tokenId = _mint(wallet, catalogId, mintKind, mintRef);
        emit Claimed(tokenId, wallet, root);
    }

    /// Sorted-pair hashing, identical to `ScryVowRegistry.verifyProof` — the one
    /// merkle dialect this repo speaks. Do not fork it.
    function verifyProof(bytes32[] calldata proof, bytes32 root, bytes32 leaf) public pure returns (bool) {
        return _verify(proof, root, leaf);
    }

    function _verify(bytes32[] calldata proof, bytes32 root, bytes32 leaf) internal pure returns (bool) {
        bytes32 h = leaf;
        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 p = proof[i];
            h = h < p ? keccak256(abi.encodePacked(h, p)) : keccak256(abi.encodePacked(p, h));
        }
        return h == root;
    }

    function _mint(address to, uint32 catalogId, uint8 mintKind, bytes32 mintRef) internal returns (uint256 tokenId) {
        require(to != address(0), "zero to");
        require(mintRef != bytes32(0), "zero mintRef");
        require(mintKind <= KIND_CRAFT, "bad kind");
        require(!mintRefUsed[mintRef], "mintRef used");
        mintRefUsed[mintRef] = true;
        tokenId = nextTokenId++;
        instanceOf[tokenId] = Instance(catalogId, mintKind, mintRef);
        _owner[tokenId] = to;
        _balance[to] += 1;
        emit Transfer(address(0), to, tokenId);
        emit ItemMinted(tokenId, to, catalogId, mintKind, mintRef);
    }

    /// Burn what you hold — the sink trade-ups and salvage need
    /// (`SKINS.md` §12 row 6). `instanceOf` survives so a marketplace can still
    /// say what a burned token was, and `mintRefUsed` survives so it can never
    /// come back.
    function burn(uint256 tokenId) external {
        address o = ownerOf(tokenId);
        require(msg.sender == o || _approved[tokenId] == msg.sender || _operator[o][msg.sender], "not authorized");
        _approved[tokenId] = address(0);
        _owner[tokenId] = address(0);
        _balance[o] -= 1;
        burned += 1;
        emit Transfer(o, address(0), tokenId);
    }

    // ── reads the game card and the market ask for ───────────────────────────

    function totalSupply() public view returns (uint256) {
        return nextTokenId - 1 - burned;
    }

    function totalMinted() external view returns (uint256) {
        return nextTokenId - 1;
    }

    /// The token's truth. Everything a buyer pays a premium for is a function
    /// of this and the published spec — never of anything stored here.
    function mintRefOf(uint256 tokenId) external view returns (bytes32) {
        require(_owner[tokenId] != address(0), "no such token");
        return instanceOf[tokenId].mintRef;
    }

    /// One call for the rug screen's item rows. `rootLive` is the honest half:
    /// closed with a live root means supply is bounded but not yet final.
    function mintStatus()
        external
        view
        returns (bool closed, uint256 minters, uint256 supply, bool rootLive, bytes32 root)
    {
        return (mintClosed, minterCount, totalSupply(), dropRoot != bytes32(0), dropRoot);
    }

    // ── ERC-721 core — boring on purpose (see the header) ────────────────────

    function ownerOf(uint256 tokenId) public view returns (address o) {
        o = _owner[tokenId];
        require(o != address(0), "no such token");
    }

    function balanceOf(address who) external view returns (uint256) {
        require(who != address(0), "zero");
        return _balance[who];
    }

    function approve(address to, uint256 tokenId) external {
        address o = ownerOf(tokenId);
        require(msg.sender == o || _operator[o][msg.sender], "not authorized");
        _approved[tokenId] = to;
        emit Approval(o, to, tokenId);
    }

    function getApproved(uint256 tokenId) external view returns (address) {
        ownerOf(tokenId);
        return _approved[tokenId];
    }

    function setApprovalForAll(address op, bool ok) external {
        _operator[msg.sender][op] = ok;
        emit ApprovalForAll(msg.sender, op, ok);
    }

    function isApprovedForAll(address o, address op) external view returns (bool) {
        return _operator[o][op];
    }

    function transferFrom(address from, address to, uint256 tokenId) public {
        require(to != address(0), "zero to");
        address o = ownerOf(tokenId);
        require(o == from, "from != owner");
        require(msg.sender == o || _approved[tokenId] == msg.sender || _operator[o][msg.sender], "not authorized");
        _approved[tokenId] = address(0);
        _owner[tokenId] = to;
        _balance[from] -= 1;
        _balance[to] += 1;
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
                "unsafe recipient"
            );
        }
    }

    // ── ERC-2981 — immutable, because a movable royalty is not a promise ─────

    function royaltyInfo(uint256, uint256 salePrice) external view returns (address, uint256) {
        return (royaltyReceiver, salePrice * royaltyBps / 10_000);
    }

    function supportsInterface(bytes4 id) external pure returns (bool) {
        return id == 0x01ffc9a7 // ERC-165
            || id == 0x80ac58cd // ERC-721
            || id == 0x5b5e139f // ERC-721 Metadata
            || id == 0x49064906 // ERC-4906 (metadata update)
            || id == 0x2a55205a; // ERC-2981
    }

    // ── metadata ─────────────────────────────────────────────────────────────

    function tokenURI(uint256 tokenId) external view returns (string memory) {
        require(_owner[tokenId] != address(0), "no such token");
        return string.concat(baseURI, "/item/", _u(tokenId), "/metadata");
    }

    function contractURI() external view returns (string memory) {
        return string.concat(baseURI, "/item/collection");
    }

    function _u(uint256 x) internal pure returns (string memory s) {
        if (x == 0) return "0";
        uint256 t = x;
        uint256 len;
        while (t != 0) {
            len++;
            t /= 10;
        }
        bytes memory b = new bytes(len);
        while (x != 0) {
            len--;
            b[len] = bytes1(uint8(48 + x % 10));
            x /= 10;
        }
        return string(b);
    }
}
