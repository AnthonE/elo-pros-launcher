// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {EloGameTicketFactory} from "./EloGameTicketFactory.sol";

interface IKnownCode {
    function deployedByFactory(address) external view returns (bool);
}

/// @title EloGameDirectory — the store's shelf, as one address a stranger can read
/// @notice THE PROBLEM THIS SOLVES. The catalog is a committed file
///         (`watchtower/listings/listings.json`) plus a signed overlay
///         (`storekeeper.py`), and a title's contracts are recorded by hand in
///         `deployments.json` and named again in a `SCRY_TICKET_<SLUG>` env var
///         on the box. Every one of those is a thing our own server is the sole
///         authority on. Someone who wanted to run a mirror of the store — or
///         just to check that the row they are reading is the row the operator
///         signed — has nowhere to look but us.
///
///         That is the whole gap. It is also how the address sprawl happened:
///         SCRY had no durable on-chain record to read, so seven modules fell
///         back to `os.getenv("SCRY_TOKEN", "0xDa2a…")` with a literal for a
///         default, and 72 files in this repo now carry that literal.
///
///         So: one address, and from it a stranger walks every listed title,
///         its ticket, its coin, its pool, and the hash of the copy the store
///         is showing. No API key, no cooperation from reserve, no trust in this
///         repo being honest about its own catalog.
///
///         ⚠ IT IS ENUMERABLE STATE, NOT EVENTS, AND THAT IS FORCED. RPC log
///         history is not available here — `eth_getLogs` is refused beyond
///         ~100 blocks on all three keyed providers, about **ten seconds** at
///         101ms blocks, and no RPC method enumerates at all (`CLAUDE.md`
///         §traps). An index built from events would be readable only by
///         whoever was watching live, which is nobody. `slugs()` and
///         `page()` walk storage.
///
///         WHAT IS ON CHAIN AND WHAT IS NOT, which is the design.
///         On chain: **pointers and one hash.** The ticket, the coin, the pool,
///         who may edit the row, and `contentHash` — the keccak of the exact
///         catalog bytes. Off chain, at the `uris` this row points to: the
///         name, the blurb, the art, the tags, the record. That split is
///         `CLAUDE.md`'s own line about what may be welded — a blurb is meant
///         to be corrected in public and a typo must not cost a transaction —
///         and it keeps a row about five slots wide instead of five kilobytes.
///         A mirror fetches the bytes from anywhere, hashes them, and compares.
///
///         ⚠ NO NUMBER THAT BECOMES MONEY LIVES HERE. Not a price, not a
///         supply, not a rate, not a rank. The same wall `storekeeper.py`
///         enforces on the catalog file, for the same reason: every number that
///         becomes money is an `eth_call` against the contract that owns it.
///         Prices are on the ticket and stay there. **This contract never owns
///         a ticket, so it can never touch a price** — not by policy, by
///         reachability.
///
///         WRITE AUTHORITY IS GATED AND STAYS GATED. Curation is a hand act and
///         there is no queue, no vote, and no fee to be considered
///         (`GATES.md` §3) — a permissionless listing registry would BE the
///         queue this store is defined against. So keepers list, and that is
///         not a debt to pay off later: the gate is the product. What
///         decentralizes here is READING, which is complete from block one and
///         needs no permission from anybody.
///
///         The authority shape mirrors the desk that already exists
///         (`devdesk.py`): the house says who is listed, the game's own team
///         says what their row shows. A keeper registers a row and names its
///         steward; the steward — the studio — sets the content hash and the
///         urls. A steward may hand off to another steward; a steward may not
///         create a listing, and there is no third link. Two levels answer the
///         whole use case in constant time.
///
///         THE ANCHOR. `catalogHead` / `catalogLength` pin the head of
///         `storekeeper.py`'s edit log. That log is already append-only and
///         every entry carries an EIP-191 signature, so *authorization* is
///         already checkable by a stranger — what is not is **which edits
///         exist and in what order**, because our server is the only one who
///         says. Anchoring the head closes exactly that, and it batches: a
///         hundred edits can share one transaction, and edits made since the
///         last anchor are still provably authorized, just not provably
///         complete. That degradation is honest and is the reason a blurb fix
///         does not need gas.
///
///         Holds no money. Nothing here is payable, takes a fee, or mints.
contract EloGameDirectory {
    string public constant NOTICE = "the store's shelf as chain state - pointers and a content hash per title, "
        "readable by anyone, writable by keepers and a title's own steward. No price, no supply, no rank, no fee";

    // ── who may act ──────────────────────────────────────────────────────────
    address public owner;
    address public pendingOwner; // handover is two steps, as everywhere here

    mapping(address => bool) public isKeeper;
    address[] private _keepers; // enumerable: a mirror checks the root itself
    mapping(address => bool) private _everKeeper; // so a re-grant never double-lists

    /// Factories whose `deployedByFactory` answer this directory will record.
    /// Owner-set, because "which build counts as the known one" is exactly the
    /// kind of claim that must not come from a caller.
    mapping(address => bool) public trustedFactory;

    // ── the shelf ────────────────────────────────────────────────────────────
    struct Listing {
        address ticket; // the copy — EloGameTicket. Prices live THERE, never here
        address coin; // the title's own coin, once it has one
        address pool; // the canonical v3 pool it pairs against SCRY in
        address steward; // the game's own team: may edit this row, nothing else
        address viaFactory; // the factory that attested the ticket's code, or 0
        bytes32 contentHash; // keccak256 of the exact catalog bytes at `uris`
        uint64 listedAt;
        uint64 updatedAt;
        bool delisted; // never deleted — a shelf's history is part of the record
    }

    mapping(bytes32 => Listing) private _rows; // keccak256(slug) => row
    mapping(bytes32 => string[]) private _uris; // where the bytes may be fetched
    mapping(bytes32 => string) public slugOf; // hash => the slug, for a reader
    bytes32[] private _slugHashes;

    // ── the anchor over storekeeper's signed edit log ────────────────────────
    bytes32 public catalogHead;
    uint256 public catalogLength;

    event OwnershipOffered(address indexed from, address indexed to);
    event OwnershipTransferred(address indexed from, address indexed to);
    event KeeperSet(address indexed keeper, bool allowed);
    event TrustedFactorySet(address indexed factory, bool trusted);
    event Listed(bytes32 indexed slugHash, string slug, address indexed ticket, address indexed steward);
    event StewardSet(bytes32 indexed slugHash, address indexed steward);
    event ContentSet(bytes32 indexed slugHash, bytes32 contentHash, uint256 uriCount);
    event PointersSet(bytes32 indexed slugHash, address coin, address pool);
    event DelistedSet(bytes32 indexed slugHash, bool delisted);
    event CatalogAnchored(bytes32 indexed head, uint256 length);

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    modifier onlyKeeper() {
        require(isKeeper[msg.sender], "not keeper");
        _;
    }

    constructor(address _owner) {
        owner = _owner == address(0) ? msg.sender : _owner;
        isKeeper[owner] = true;
        _everKeeper[owner] = true;
        _keepers.push(owner);
        emit OwnershipTransferred(address(0), owner);
        emit KeeperSet(owner, true);
    }

    // ── admin ────────────────────────────────────────────────────────────────
    function setKeeper(address keeper, bool allowed) external onlyOwner {
        require(keeper != address(0), "zero keeper");
        if (isKeeper[keeper] == allowed) return;
        isKeeper[keeper] = allowed;
        // Granted, revoked, then granted again must not appear twice in the
        // enumerable list — `keepers()` is what a stranger reads to check the
        // authority root, and a duplicate there reads as two distinct holders.
        if (allowed && !_everKeeper[keeper]) {
            _everKeeper[keeper] = true;
            _keepers.push(keeper);
        }
        emit KeeperSet(keeper, allowed);
    }

    function setTrustedFactory(address factory, bool trusted) external onlyOwner {
        trustedFactory[factory] = trusted;
        emit TrustedFactorySet(factory, trusted);
    }

    /// Two steps, for the same reason the ticket's handover is: a mistyped
    /// third-party address should cost a second call, not freeze the shelf.
    /// Renouncing to address(0) stays one step — it only removes power.
    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) {
            emit OwnershipTransferred(owner, address(0));
            owner = address(0);
            pendingOwner = address(0);
            return;
        }
        pendingOwner = newOwner;
        emit OwnershipOffered(owner, newOwner);
    }

    function acceptOwnership() external {
        require(msg.sender == pendingOwner, "not the pending owner");
        emit OwnershipTransferred(owner, msg.sender);
        owner = msg.sender;
        pendingOwner = address(0);
    }

    // ── listing ──────────────────────────────────────────────────────────────

    /// Put a title on the shelf. The hand act, and the only one.
    ///
    /// `viaFactory` may be zero — a ticket deployed by hand lists exactly like
    /// one from the factory, because the factory is a signal and never a
    /// requirement. When it is not zero it must be a trusted factory that
    /// actually attests this ticket, so the row can never claim a known build
    /// it does not have.
    function register(
        string calldata slug,
        address ticket,
        address coin,
        address pool,
        address steward,
        address viaFactory,
        bytes32 contentHash,
        string[] calldata uris
    ) public onlyKeeper {
        require(bytes(slug).length > 0 && bytes(slug).length <= 64, "slug 1..64");
        require(ticket != address(0), "zero ticket");
        require(steward != address(0), "zero steward");
        bytes32 h = keccak256(bytes(slug));
        require(_rows[h].ticket == address(0), "slug taken");
        if (viaFactory != address(0)) {
            require(trustedFactory[viaFactory], "factory not trusted");
            require(IKnownCode(viaFactory).deployedByFactory(ticket), "not from that factory");
        }
        _rows[h] = Listing({
            ticket: ticket,
            coin: coin,
            pool: pool,
            steward: steward,
            viaFactory: viaFactory,
            contentHash: contentHash,
            listedAt: uint64(block.timestamp),
            updatedAt: uint64(block.timestamp),
            delisted: false
        });
        slugOf[h] = slug;
        _slugHashes.push(h);
        _setUris(h, uris);
        emit Listed(h, slug, ticket, steward);
        emit PointersSet(h, coin, pool);
        emit ContentSet(h, contentHash, uris.length);
    }

    /// Deploy a title's ticket through the factory and shelve it in one act.
    ///
    /// The owner of the new ticket is `params.owner` and it may NOT be zero
    /// here: zero means "the caller" at the factory, and the caller is this
    /// contract. A directory that owned a live sale contract could set its
    /// prices, which is the one thing this design is careful never to be able
    /// to do.
    function listNew(
        string calldata slug,
        bytes calldata creationCode,
        EloGameTicketFactory.TicketParams calldata params,
        bytes32 salt,
        address factory,
        address coin,
        address steward,
        bytes32 contentHash,
        string[] calldata uris
    ) external onlyKeeper returns (address ticket) {
        require(trustedFactory[factory], "factory not trusted");
        require(params.owner != address(0), "name the ticket's owner");
        ticket = EloGameTicketFactory(factory).deploy(creationCode, params, salt);
        register(slug, ticket, coin, address(0), steward, factory, contentHash, uris);
    }

    // ── editing a row ────────────────────────────────────────────────────────
    //
    // The house says WHO is listed; the game's own team says WHAT their row
    // shows. Same split the dev desk already runs off chain.

    function _row(string calldata slug) private view returns (bytes32 h) {
        h = keccak256(bytes(slug));
        require(_rows[h].ticket != address(0), "no such listing");
    }

    modifier onlySteward(bytes32 h) {
        require(msg.sender == _rows[h].steward || isKeeper[msg.sender], "not steward");
        _;
    }

    /// The studio's own hand: repoint the row at new bytes. A keeper may also
    /// do this, because the house has to be able to correct a shelf it curates.
    function setContent(string calldata slug, bytes32 contentHash, string[] calldata uris)
        external
        onlySteward(keccak256(bytes(slug)))
    {
        bytes32 h = _row(slug);
        _rows[h].contentHash = contentHash;
        _rows[h].updatedAt = uint64(block.timestamp);
        _setUris(h, uris);
        emit ContentSet(h, contentHash, uris.length);
    }

    function setPointers(string calldata slug, address coin, address pool)
        external
        onlySteward(keccak256(bytes(slug)))
    {
        bytes32 h = _row(slug);
        _rows[h].coin = coin;
        _rows[h].pool = pool;
        _rows[h].updatedAt = uint64(block.timestamp);
        emit PointersSet(h, coin, pool);
    }

    /// A steward may hand its own row to another steward, and a keeper may
    /// reassign one. The chain stops here: a steward cannot mint a listing and
    /// there is no third link, so "may this wallet edit gates?" is one lookup.
    function setSteward(string calldata slug, address steward) external onlySteward(keccak256(bytes(slug))) {
        require(steward != address(0), "zero steward");
        bytes32 h = _row(slug);
        _rows[h].steward = steward;
        _rows[h].updatedAt = uint64(block.timestamp);
        emit StewardSet(h, steward);
    }

    /// Take a title off the shelf without erasing that it was on it. Keeper
    /// only, and it is a flag rather than a delete: a shelf that can forget is
    /// a shelf whose history nobody can check.
    function setDelisted(string calldata slug, bool delisted) external onlyKeeper {
        bytes32 h = _row(slug);
        _rows[h].delisted = delisted;
        _rows[h].updatedAt = uint64(block.timestamp);
        emit DelistedSet(h, delisted);
    }

    function _setUris(bytes32 h, string[] calldata uris) private {
        require(uris.length <= 8, "at most 8 uris");
        delete _uris[h];
        for (uint256 i; i < uris.length; ++i) {
            require(bytes(uris[i]).length > 0 && bytes(uris[i]).length <= 512, "uri 1..512");
            _uris[h].push(uris[i]);
        }
    }

    // ── the anchor ───────────────────────────────────────────────────────────

    /// Pin the head of the signed catalog edit log. `length` may not go
    /// backwards: an append-only log whose anchor could rewind would let a
    /// replaced head look like a fresh one, which is the property the anchor
    /// exists to provide.
    function anchorCatalog(bytes32 head, uint256 length) external onlyKeeper {
        require(length >= catalogLength, "length must not rewind");
        catalogHead = head;
        catalogLength = length;
        emit CatalogAnchored(head, length);
    }

    // ── reading, which is open to everyone forever ───────────────────────────

    function count() external view returns (uint256) {
        return _slugHashes.length;
    }

    function slugHashAt(uint256 i) external view returns (bytes32) {
        return _slugHashes[i];
    }

    function listingOf(string calldata slug) external view returns (Listing memory) {
        return _rows[keccak256(bytes(slug))];
    }

    function listingOfHash(bytes32 slugHash) external view returns (Listing memory) {
        return _rows[slugHash];
    }

    function urisOf(string calldata slug) external view returns (string[] memory) {
        return _uris[keccak256(bytes(slug))];
    }

    function urisOfHash(bytes32 slugHash) external view returns (string[] memory) {
        return _uris[slugHash];
    }

    /// Bootstrap in one call: slugs and rows together, paged. This is the
    /// method a mirror actually uses, and it exists because walking an index
    /// one `eth_call` at a time is what makes people give up and trust an API.
    /// `limit` 0 means "to the end"; `start` past the end returns empty arrays.
    function page(uint256 start, uint256 limit)
        external
        view
        returns (string[] memory slugs, Listing[] memory rows)
    {
        uint256 n = _slugHashes.length;
        if (start >= n) return (new string[](0), new Listing[](0));
        uint256 end = limit == 0 ? n : start + limit;
        if (end > n) end = n;
        uint256 len = end - start;
        slugs = new string[](len);
        rows = new Listing[](len);
        for (uint256 i; i < len; ++i) {
            bytes32 h = _slugHashes[start + i];
            slugs[i] = slugOf[h];
            rows[i] = _rows[h];
        }
    }

    /// The write-authority root, readable so a stranger can check the chain of
    /// signatures ends somewhere they can see rather than in a file on our box.
    function keepers() external view returns (address[] memory live) {
        uint256 n;
        for (uint256 i; i < _keepers.length; ++i) {
            if (isKeeper[_keepers[i]]) n++;
        }
        live = new address[](n);
        uint256 j;
        for (uint256 i; i < _keepers.length; ++i) {
            if (isKeeper[_keepers[i]]) live[j++] = _keepers[i];
        }
    }
}
