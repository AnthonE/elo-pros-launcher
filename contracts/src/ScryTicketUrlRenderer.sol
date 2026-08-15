// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ScryTicketUrlRenderer — a ticket's metadata, served from a URL
/// @notice `ScryGameTicket` builds its own `data:application/json` document
///         from contract storage, which is the right default: the one bearer
///         asset on the platform should not need our web server to have a
///         name. This is the OTHER end of the `setRenderer` hinge the ticket
///         already posts — a title whose art and copy move often points at
///         `/ticket/<game>/<id>/metadata` instead, and edits JSON rather than
///         sending a transaction.
///
///         THE TRADE, SAID PLAINLY. On chain: no server dependency, one tx per
///         edit, and the document is only what the contract can hold. Here:
///         free instant edits and every field a marketplace supports
///         (`banner_image`, `featured_image`, `animation_url`, numeric traits)
///         — paid for with a live dependency on an origin. The escape is one
///         transaction and it is posted in the ticket, not here:
///         `setRenderer(address(0))` restores the built-in document, so an
///         origin that goes dark costs a tx, never the asset.
///
///         ONE RENDERER PER TITLE, deliberately. A shared renderer would have
///         to key off `msg.sender` to know which game it is answering for, and
///         an unregistered ticket would then revert inside `tokenURI` — a
///         wallet showing a broken token because a mapping was missed. A slug
///         fixed at construction cannot be missed.
///
///         NEITHER FUNCTION CAN REVERT. Both are pure string concatenation
///         over storage that is set in the constructor and never unset. A
///         renderer that can throw is a renderer that can brick every token in
///         the collection at once, and `tokenURI` is a view a marketplace
///         calls without a retry.
contract ScryTicketUrlRenderer {
    /// The API root, with no trailing slash — e.g.
    /// `https://scry.moreright.xyz/api`.
    ///
    /// ⚠ **IT CARRIES `/api`, AND THE SITE BASE DOES NOT.** The ticket's own
    /// `baseURI` is the SITE root (it builds `…/game.html?slug=gates`, a static
    /// page). This one addresses the meter, which nginx serves under `/api/`
    /// while the module declares its routes bare — `CLAUDE.md`'s named trap.
    /// Two bases, two different roots, one letter of difference in a field that
    /// decides whether every token in the collection has metadata at all. The
    /// bare origin was the example here until 2026-08-12 and it 404s, measured.
    string public base;

    /// The listing slug this renderer answers for, fixed at construction.
    string public game;

    address public owner;
    address public pendingOwner;

    event BaseUpdated(string base);
    event OwnershipTransferred(address indexed from, address indexed to);
    event OwnershipOffered(address indexed from, address indexed to);

    error NotOwner();
    error NotPending();
    error EmptyBase();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    constructor(address _owner, string memory _base, string memory _game) {
        if (bytes(_base).length == 0) revert EmptyBase();
        owner = _owner == address(0) ? msg.sender : _owner;
        base = _base;
        game = _game;
        emit BaseUpdated(_base);
        emit OwnershipTransferred(address(0), owner);
    }

    /// Move the origin — and the door back from a base that was typed without
    /// its `/api` segment, which is the mistake this field invites.
    ///
    /// An empty base is refused rather than stored: it would produce
    /// `/ticket/gates/1/metadata`, a relative path a marketplace resolves
    /// against ITS own host, which 404s somewhere we never see.
    ///
    /// ⚠ This contract cannot tell a marketplace anything changed — the
    /// ERC-4906 and ERC-7572 events live on the TICKET. After calling this,
    /// re-post the same renderer on the ticket (`setRenderer(address(this))`)
    /// and it emits `ContractURIUpdated` + `BatchMetadataUpdate` for you.
    function setBase(string calldata newBase) external onlyOwner {
        if (bytes(newBase).length == 0) revert EmptyBase();
        base = newBase;
        emit BaseUpdated(newBase);
    }

    function transferOwnership(address to) external onlyOwner {
        pendingOwner = to;
        emit OwnershipOffered(msg.sender, to);
    }

    function acceptOwnership() external {
        if (msg.sender != pendingOwner) revert NotPending();
        emit OwnershipTransferred(owner, msg.sender);
        owner = msg.sender;
        pendingOwner = address(0);
    }

    function cancelHandover() external onlyOwner {
        pendingOwner = address(0);
    }

    // ── what the ticket asks for ─────────────────────────────────────────────

    function tokenURI(uint256 tokenId) external view returns (string memory) {
        return string.concat(base, "/ticket/", game, "/", _u(tokenId), "/metadata");
    }

    function contractURI() external view returns (string memory) {
        return string.concat(base, "/ticket/", game, "/collection");
    }

    /// uint → decimal string. Same shape as the ticket's own `_u`, restated
    /// rather than imported so this file has no dependency to keep in step.
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
            b[--len] = bytes1(uint8(48 + (x % 10)));
            x /= 10;
        }
        s = string(b);
    }
}
