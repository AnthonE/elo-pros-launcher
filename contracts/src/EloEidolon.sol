// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

interface IERC721Receiver {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

/// @notice The two calls the summoning needs from the ticket contract: prove
///         the caller holds the ticket, then burn it. `MAX_SUPPLY` is read
///         once at deploy to weld tickets 1:1 to vessels.
interface IEloTicket {
    function ownerOf(uint256 tokenId) external view returns (address);
    function burnFrom(address from, uint256 tokenId) external;
    function MAX_SUPPLY() external view returns (uint256);
}

/// @title EloEidolon — the Thousand: 1000 one-of-one familiar vessels
/// @notice A fixed run of 1000 collectible vessels (eidola — the ancient
///         word for a spirit's visible double). ONE TICKET, ONE VESSEL: the
///         mint takes a EloTicket the caller holds and burns it in the act —
///         the ticket contract caps tickets at the same 1000 as the vessels,
///         so every ticket ever issued is redeemable and nothing else can
///         summon. How a wallet came by its ticket (earned in the Delver's
///         Trials, bought in the snapshot or open sale, drawn from the gacha,
///         traded on any marketplace) is the ticket contract's business; this
///         contract only checks possession and burns. A flat SCRY price to
///         the EloFeeSplitter remains as a knob when `price > 0` — the launch
///         plan prices the TICKET in ETH and deploys this at 0, so the vessel
///         is never charged twice.
///         Token ids are sequential — the chain itself is the atomic counter —
///         and every vessel's traits derive from sha256(salt | tokenId) where
///         sha256(salt) is committed IMMUTABLY here at deploy, before the first
///         mint. The commitment is the honesty anchor: once the salt is
///         published anyone recomputes the whole collection and checks it
///         against the immutable commitment.
///
///         A vessel is a collectible, not a measurement: it carries no meter
///         number, vow trajectory, or payout odds, and nothing about it — nor
///         about the ticket that summons it — is a reading of any agent. A
///         trials ticket is earned by participation (playing / labor), NEVER
///         by a score; the meter's ledger holds deeds, never measurements.
///
///         Deliberately absent (vs. the morr pipeline this ports): no
///         transfer/freeze delegates, no server fee-payer, no admin mint. The
///         owner's only powers are repointing the metadata base URI and
///         transferring ownership; nobody can mint free of a ticket, or
///         move / lock / re-roll a vessel. Renouncing ownership
///         (transferOwnership(0)) freezes the base URI forever — the posted
///         door out (HELD-KEYS.md).
contract EloEidolon is ReentrancyGuard {
    string public name = "elo eidolons - the thousand";
    string public symbol = "EIDOLON";
    string public constant NOTICE = "1000 vessels, one burned ticket each (EloTicket, same 1000 cap), a flat reserve fee to the "
        "splitter when priced, traits sealed by saltCommitment before mint - a collectible, not a measurement";

    uint256 public constant MAX_SUPPLY = 1000;

    IERC20 public immutable reserve;
    IEloTicket public immutable ticket; // the one gate: hold a ticket, burn it to summon
    address public immutable feeSplitter;
    uint256 public immutable price; // flat SCRY per vessel (18-dec units); 0 = the ticket is the whole price
    bytes32 public immutable saltCommitment; // sha256(salt) — salt published after mint-out
    uint96 public immutable royaltyBps; // ERC-2981, paid to the fee splitter
    string public baseURI; // metadata base — owner-settable (setBaseURI)
    address public owner; // admin: setBaseURI / transferOwnership only

    uint256 public nextTokenId = 1; // ids run 1..1000
    mapping(uint256 => uint64) public mintedAt;
    mapping(uint256 => address) private _owner;
    mapping(address => uint256) private _balance;
    mapping(uint256 => address) private _approved;
    mapping(address => mapping(address => bool)) private _operator;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);
    event EidolonMinted(uint256 indexed tokenId, address indexed to, uint256 indexed ticketId);
    event OwnershipTransferred(address indexed from, address indexed to);
    event ContractURIUpdated(); // ERC-7572
    event MetadataUpdate(uint256 _tokenId); // ERC-4906
    event BatchMetadataUpdate(uint256 _fromTokenId, uint256 _toTokenId); // ERC-4906

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    constructor(
        IERC20 _reserve,
        IEloTicket _ticket,
        address _feeSplitter,
        uint256 _price,
        bytes32 _saltCommitment,
        uint96 _royaltyBps,
        string memory _baseURI
    ) {
        require(address(_ticket) != address(0), "zero ticket");
        require(_ticket.MAX_SUPPLY() == MAX_SUPPLY, "ticket supply mismatch");
        require(_feeSplitter != address(0), "zero splitter");
        require(_saltCommitment != bytes32(0), "zero commitment");
        require(_royaltyBps <= 1000, "royalty > 10%");
        reserve = _reserve;
        ticket = _ticket;
        feeSplitter = _feeSplitter;
        price = _price;
        saltCommitment = _saltCommitment;
        royaltyBps = _royaltyBps;
        baseURI = _baseURI;
        owner = msg.sender;
        emit OwnershipTransferred(address(0), msg.sender);
        emit ContractURIUpdated();
    }

    // ── admin: metadata base + ownership (a regular ownable NFT) ─────────────
    function transferOwnership(address newOwner) external onlyOwner {
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner; // pass address(0) to renounce and freeze the base URI forever
    }

    /// Repoint the metadata base (e.g. a new meter host or an IPFS gateway) and
    /// signal every marketplace to re-pull token + collection metadata.
    function setBaseURI(string calldata newBase) external onlyOwner {
        baseURI = newBase;
        emit ContractURIUpdated();
        if (nextTokenId > 1) emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    /// Rename the collection. name/symbol are settable because this platform
    /// has been renamed more than once and a constant welds whichever name
    /// happened to be current on broadcast day into every wallet forever.
    /// Wallets cache both; the contract-level event asks them to re-pull.
    function setName(string calldata newName) external onlyOwner {
        name = newName;
        emit ContractURIUpdated();
        if (nextTokenId > 1) emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    function setSymbol(string calldata newSymbol) external onlyOwner {
        symbol = newSymbol;
        emit ContractURIUpdated();
        if (nextTokenId > 1) emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    /// Summon one of the thousand: hand over a ticket you hold, watch it burn.
    /// The ticket contract enforces who may burn (only this contract) and the
    /// ownership check here makes the failure mode a plain sentence rather
    /// than a pass-through revert. When `price > 0` the caller must also have
    /// approved `price` SCRY, pulled straight to the fee splitter — the launch
    /// deploy sets 0, because the ticket already carried the price in ETH.
    /// (Pull-before-state-write is safe ONLY because SCRY is a plain
    /// no-callback ERC-20 — this contract is SCRY-specific by design.)
    function mint(uint256 ticketId) external nonReentrant returns (uint256 tokenId) {
        require(nextTokenId <= MAX_SUPPLY, "the thousand are all summoned");
        require(ticket.ownerOf(ticketId) == msg.sender, "not your ticket");
        ticket.burnFrom(msg.sender, ticketId);
        if (price > 0) {
            SafeERC20.safeTransferFrom(address(reserve), msg.sender, feeSplitter, price, "elo payment failed");
        }
        tokenId = nextTokenId++;
        mintedAt[tokenId] = uint64(block.timestamp);
        _owner[tokenId] = msg.sender;
        _balance[msg.sender] += 1;
        emit Transfer(address(0), msg.sender, tokenId);
        emit EidolonMinted(tokenId, msg.sender, ticketId);
    }

    function totalMinted() external view returns (uint256) {
        return nextTokenId - 1;
    }

    // ── ERC-721 core ─────────────────────────────────────────────────────────
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

    // ── ERC-2981 royalties (to the splitter, immutable) ──────────────────────
    function royaltyInfo(uint256, uint256 salePrice) external view returns (address, uint256) {
        return (feeSplitter, salePrice * royaltyBps / 10_000);
    }

    function supportsInterface(bytes4 id) external pure returns (bool) {
        return id == 0x01ffc9a7 // ERC-165
            || id == 0x80ac58cd // ERC-721
            || id == 0x5b5e139f // ERC-721 Metadata
            || id == 0x49064906 // ERC-4906 (metadata update)
            || id == 0x2a55205a; // ERC-2981
    }

    // ── metadata: served by the meter, deterministic from the sealed salt ────
    // The meter recomputes every trait from sha256(salt | tokenId); it can
    // never invent one, because this contract committed sha256(salt) before
    // mint #1. Metadata upgrades in place as optional assets (AI image, GLB)
    // are forged — the traits underneath never move.
    function tokenURI(uint256 tokenId) external view returns (string memory) {
        require(mintedAt[tokenId] != 0, "no such token");
        return string.concat(baseURI, "/eidolon/", _u(tokenId), "/metadata");
    }

    function contractURI() external view returns (string memory) {
        return string.concat(baseURI, "/eidolon/collection");
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
