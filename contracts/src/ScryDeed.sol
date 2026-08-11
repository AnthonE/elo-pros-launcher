// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";

interface IERC721Receiver {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

/// @notice the OBOL surface this contract needs: a plain burn of the caller's
///         own spoils. SpoilsToken exposes open burn/burnFrom by design.
interface ISpoils is IERC20 {
    function burnFrom(address from, uint256 amount) external;
    function allowance(address owner, address spender) external view returns (uint256);
}

/// @notice optional external metadata renderer. If the owner sets one, the deed
///         delegates tokenURI/contractURI to it — so the metadata can be
///         repointed to any URI (a hosted server, IPFS) later without migrating
///         a single deed. A renderer reads deed data back from `deeds(tokenId)`.
///         address(0) => the built-in on-chain art below.
interface IScryRenderer {
    function tokenURI(uint256 tokenId) external view returns (string memory);
    function contractURI() external view returns (string memory);
}

/// @title ScryDeed — the tabula: a house on a named street, as property
/// @notice The on-chain mirror of the off-chain streets (`meter/tabernae.py`).
///         A deed is the ERC-721 title to ONE plot — `streetIndex × plot`, the
///         same coordinates the off-chain ledger assigns in founding order. It
///         is a REGULAR NFT: minted to the caller's own wallet, held in the
///         caller's own wallet. This contract never custodies a key and never
///         mints to anyone but msg.sender — custody stays cap 0 (the wall does
///         not move to ship a deed).
///
///         THE DEED IS A SINK, ON PURPOSE. Founding burns OBOL; every
///         conveyance (the mancipatio — a formal hand-change) burns OBOL again.
///         Land speculation is not a leak to prevent here — the churn is the
///         burn, and an elastic OBOL supply is kept safe by exactly this kind
///         of sink, not by a cap (POOLS.md §3.5). A landlord who flips plots
///         feeds the fire on every trade.
///
///         WHAT A DEED DOES NOT CARRY: your name. Reputation is soulbound to
///         the VOW at ScryReputation, never to the property. Buy the walls and
///         the plot; you never buy the seller's conduct record. That is why a
///         land market is safe to run and still score-blind: a deed buys no
///         reputation, no access, no odds — only the four walls.
///
///         THE KEY (the clavis): the owner may cut a scoped, revocable key so a
///         steward — a hired familiar, a hand, a tenant — may act for the house.
///         The grant is only an on-chain AUTHORIZATION record (never a private
///         key, never custody); the off-chain ledger reads it and lets the
///         steward act UNDER THEIR OWN VOW, journaled as steward-for-house. A
///         breach lands on whoever actually acted, never laundered through the
///         property.
///
///         Regular ownable NFT: the owner may repoint the metadata (setBaseURI
///         / setRenderer) but has NO mint gate beyond the burn and CANNOT seize,
///         move, or lock a deed — the four walls stay the holder's. Founding
///         costs and the conveyance toll are immutable at deploy — the operator
///         sets them high to make the sink big; the plot space (how much land
///         exists) is the off-chain scarcity knob. Written in an env without
///         foundry — `forge test -vv` before any broadcast, as with every here.
///
///         SELLING THROUGH A MARKETPLACE (audit 2026-07-25, tightened by the
///         security review the same day). The toll burns from the DEPARTING
///         OWNER when they have explicitly agreed to it, and otherwise from
///         `msg.sender`. That is what makes deeds tradeable at all: on OpenSea
///         and every aggregator `msg.sender` is a router that holds no OBOL,
///         so an always-`msg.sender` toll silently made every deed untradeable
///         off-site — and the churn is the burn the design is built on.
///
///         The seller's agreement is a COUNTED CREDIT, not an allowance:
///         `allowConveyances(1)` before listing, and `transferFrom` spends
///         exactly one. An OBOL allowance alone is deliberately NOT read as
///         consent, because `found()` needs one anyway and an ERC-721 operator
///         approval is unbounded — treating the two together as permission let
///         anyone approved to MOVE a deed instead drain that owner's OBOL, one
///         toll per transfer. `sellerCoversToll(seller)` is the read a listing
///         UI should make. Do not tell sellers to approve OBOL without limit;
///         `n × conveyanceBurn` is the honest number.
///
///         ⚠ Marketplaces that ESCROW the NFT into their own contract first
///         are still not covered — `from` is then the escrow, which holds no
///         OBOL either. Approve-and-list flows work; escrow-and-list do not.
contract ScryDeed {
    string public constant name = "scry deeds";
    string public constant symbol = "DEED";

    ISpoils public immutable obol; // the spoils coin the sink burns
    uint256 public immutable foundingBurn; // OBOL burned to found a new deed
    uint256 public immutable conveyanceBurn; // OBOL burned on every hand-change
    string public baseURI; // metadata base — owner-settable (setBaseURI)
    address public owner; // admin: metadata only (setBaseURI / setRenderer)
    address public renderer; // address(0) => built-in on-chain art

    // the capability scopes a key may carry (a bitmask). The off-chain ledger
    // is the enforcer; these are the agreed bit meanings.
    uint32 public constant SCOPE_CRAFT = 1; // stock the boards / fire the ovens
    uint32 public constant SCOPE_PRICE = 2; // chalk a price (never past a sworn ceiling)
    uint32 public constant SCOPE_DELIVER = 4; // answer the annona
    uint32 public constant SCOPE_SHINGLE = 8; // change the trade

    struct Deed {
        uint16 streetIndex;
        uint16 plot;
        uint64 foundedAt;
    }

    struct Key {
        address steward; // until==0 → no key
        uint32 scopes;
        uint64 until;
    }

    uint256 public nextTokenId = 1;
    mapping(uint256 => Deed) public deeds; // tokenId => plot
    mapping(uint256 => uint256) public plotToken; // packed coord => tokenId (0 == free)
    mapping(uint256 => Key) public keys; // tokenId => the cut key

    /// How many further conveyances this owner has agreed to pay the toll on.
    /// A plain OBOL allowance is NOT consent to a specific conveyance — see
    /// `allowConveyances` and `transferFrom` for why this exists.
    mapping(address => uint256) public conveyanceCredits;

    mapping(uint256 => address) private _owner;
    mapping(address => uint256) private _balance;
    mapping(uint256 => address) private _approved;
    mapping(address => mapping(address => bool)) private _operator;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);
    event Founded(uint256 indexed tokenId, uint16 streetIndex, uint16 plot, address indexed to, uint256 burned);
    event Conveyed(uint256 indexed tokenId, address indexed from, address indexed to, uint256 burned);
    event KeyCut(uint256 indexed tokenId, address indexed steward, uint32 scopes, uint64 until);
    event KeyRevoked(uint256 indexed tokenId);
    event ConveyancesAllowed(address indexed owner, uint256 count);
    event OwnershipTransferred(address indexed from, address indexed to);
    event RendererUpdated(address indexed renderer);
    event ContractURIUpdated(); // ERC-7572
    event MetadataUpdate(uint256 _tokenId); // ERC-4906
    event BatchMetadataUpdate(uint256 _fromTokenId, uint256 _toTokenId); // ERC-4906

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    constructor(ISpoils _obol, uint256 _foundingBurn, uint256 _conveyanceBurn, string memory _baseURI) {
        require(address(_obol) != address(0), "zero obol");
        obol = _obol;
        foundingBurn = _foundingBurn;
        conveyanceBurn = _conveyanceBurn;
        baseURI = _baseURI;
        owner = msg.sender;
        emit OwnershipTransferred(address(0), msg.sender);
        emit ContractURIUpdated();
    }

    // ── admin: metadata only (a regular ownable NFT; no seize, no free mint) ──
    function transferOwnership(address newOwner) external onlyOwner {
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner; // pass address(0) to renounce and freeze metadata forever
    }

    /// Repoint the metadata base (external_link / any renderer that reads it).
    function setBaseURI(string calldata newBase) external onlyOwner {
        baseURI = newBase;
        emit ContractURIUpdated();
        if (nextTokenId > 1) emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    /// Swap the metadata renderer (address(0) restores the built-in on-chain
    /// art). Lets the whole tokenURI/contractURI be repointed to a URI later
    /// without migrating a deed; signals marketplaces to re-pull everything.
    function setRenderer(address newRenderer) external onlyOwner {
        renderer = newRenderer;
        emit RendererUpdated(newRenderer);
        emit ContractURIUpdated();
        if (nextTokenId > 1) emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    function _coord(uint16 streetIndex, uint16 plot) internal pure returns (uint256) {
        return (uint256(streetIndex) << 16) | uint256(plot);
    }

    /// Found a deed to an empty plot. Burns `foundingBurn` OBOL from the caller
    /// (approve this contract first) and mints the title to the caller. The
    /// off-chain ledger decides which plot is next in founding order and hands
    /// the coordinates in via /prepare; this only refuses a plot already taken.
    /// (Burn-before-mint is safe: OBOL is a plain no-callback ERC-20.)
    function found(uint16 streetIndex, uint16 plot) external returns (uint256 tokenId) {
        uint256 c = _coord(streetIndex, plot);
        require(plotToken[c] == 0, "plot taken");
        if (foundingBurn > 0) obol.burnFrom(msg.sender, foundingBurn);
        tokenId = nextTokenId++;
        deeds[tokenId] = Deed(streetIndex, plot, uint64(block.timestamp));
        plotToken[c] = tokenId;
        _owner[tokenId] = msg.sender;
        _balance[msg.sender] += 1;
        emit Transfer(address(0), msg.sender, tokenId);
        emit Founded(tokenId, streetIndex, plot, msg.sender, foundingBurn);
    }

    // ── the key: a scoped, revocable capability, never a private key ──────────
    /// Cut a key so `steward` may act for this house until `until` (unix secs),
    /// within `scopes`. Only the owner may cut it; cutting again replaces it.
    /// This is an AUTHORIZATION record only — the steward still signs with their
    /// own wallet and answers under their own vow.
    function grantKey(uint256 tokenId, address steward, uint32 scopes, uint64 until) external {
        require(msg.sender == ownerOf(tokenId), "not owner");
        require(steward != address(0) && until > block.timestamp, "bad key");
        keys[tokenId] = Key(steward, scopes, until);
        emit KeyCut(tokenId, steward, scopes, until);
    }

    function revokeKey(uint256 tokenId) external {
        require(msg.sender == ownerOf(tokenId), "not owner");
        delete keys[tokenId];
        emit KeyRevoked(tokenId);
    }

    /// True if `who` currently holds a live key on `tokenId` covering `scope`.
    /// The off-chain ledger calls this shape before letting a steward act.
    function keyAllows(uint256 tokenId, address who, uint32 scope) external view returns (bool) {
        Key memory k = keys[tokenId];
        return k.steward == who && k.until > block.timestamp && (k.scopes & scope) == scope;
    }

    // ── ERC-721 core — transferable, and the transfer is the mancipatio ───────
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

    /// Every hand-change is the mancipatio: it burns `conveyanceBurn` OBOL (the
    /// sink fires on gifts and sales alike — there is no informal transfer),
    /// clears any cut key, and moves the title. A deed cannot move without OBOL
    /// to burn: that friction IS the sink.
    ///
    /// WHO PAYS THE TOLL (fixed 2026-07-25, audit §3). It used to be
    /// `msg.sender`, unconditionally — which on any marketplace is the ROUTER,
    /// not a person. Routers hold no OBOL and grant no allowance, so every deed
    /// was effectively **non-tradeable through OpenSea and every aggregator**,
    /// and that removes the churn the whole "land speculation is fine because
    /// the churn is the burn" design rests on (SENTENCES.md 2026-07-22). The
    /// toll now falls on `from` — the departing owner, whose act of conveyance
    /// this is — whenever they have prepared for it (OBOL balance + an
    /// allowance to this contract), and falls back to `msg.sender` when they
    /// have not, so the direct owner-calls-transferFrom path is unchanged.
    /// A seller who approves OBOL to this contract once can list anywhere.
    /// Either way SOMEONE party to the conveyance burns it, and `from` has
    /// authorized the move regardless (they are the owner, or they approved
    /// the operator making it).
    function transferFrom(address from, address to, uint256 tokenId) public {
        require(to != address(0), "zero to");
        // A deed cannot be conveyed to its own owner. Nothing about the title
        // would change, and allowing it made the toll a repeatable charge
        // against an unchanged owner — see `allowConveyances`.
        require(to != from, "self-conveyance");
        address o = ownerOf(tokenId);
        require(o == from, "from != owner");
        require(msg.sender == o || _approved[tokenId] == msg.sender || _operator[o][msg.sender], "not authorized");
        if (conveyanceBurn > 0) obol.burnFrom(_chargeToll(from), conveyanceBurn);
        _approved[tokenId] = address(0);
        if (keys[tokenId].until != 0) {
            delete keys[tokenId];
            emit KeyRevoked(tokenId);
        }
        _owner[tokenId] = to;
        _balance[from] -= 1;
        _balance[to] += 1;
        emit Transfer(from, to, tokenId);
        emit Conveyed(tokenId, from, to, conveyanceBurn);
    }

    /// Agree to pay the conveyance toll on your next `count` transfers out.
    /// Set it to 1 before listing a deed, or 0 to withdraw the offer.
    ///
    /// WHY THIS EXISTS, and why an OBOL allowance is not enough (security
    /// review 2026-07-25). The seller-pays path was introduced so a deed can
    /// sell through a marketplace router, which holds no OBOL. The first cut
    /// read "does `from` hold OBOL and have an allowance to this contract?" as
    /// the seller's consent — but that allowance is a general one (`found()`
    /// needs it too), and an ERC-721 operator approval is unbounded and
    /// survives transfers. Together those let anyone the owner had approved to
    /// MOVE a deed instead SPEND that owner's entire OBOL balance, one toll at
    /// a time. A counted credit makes the blast radius a number the owner
    /// chose for this exact purpose, which is the same shape every other
    /// bounded authority in this repo takes.
    function allowConveyances(uint256 count) external {
        conveyanceCredits[msg.sender] = count;
        emit ConveyancesAllowed(msg.sender, count);
    }

    /// True when `from` has both agreed to pay a toll (`allowConveyances`) and
    /// is able to (OBOL balance + allowance to this contract). A listing UI
    /// should check this before a marketplace sale: false here means the toll
    /// falls on whoever calls `transferFrom`, which through a router is the
    /// router, and routers hold no OBOL — so the sale would revert.
    function sellerCoversToll(address from) external view returns (bool) {
        return conveyanceCredits[from] > 0 && _covers(from);
    }

    function _covers(address who) internal view returns (bool) {
        return obol.balanceOf(who) >= conveyanceBurn && obol.allowance(who, address(this)) >= conveyanceBurn;
    }

    /// Who burns this conveyance's toll — and spend the owner's credit if it
    /// is them, so one credit pays for exactly one conveyance.
    function _chargeToll(address from) internal returns (address) {
        if (conveyanceCredits[from] > 0 && _covers(from)) {
            conveyanceCredits[from] -= 1;
            return from;
        }
        return msg.sender;
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

    function supportsInterface(bytes4 id) external pure returns (bool) {
        return id == 0x01ffc9a7 // ERC-165
            || id == 0x80ac58cd // ERC-721
            || id == 0x5b5e139f // ERC-721 Metadata
            || id == 0x49064906 // ERC-4906 — this contract EMITS MetadataUpdate
            //                     (setBaseURI/setRenderer repoint it), so it must
            //                     also declare it or indexers never refresh.
            || id == 0x49064906; // ERC-4906 (metadata update)
    }

    // ── metadata: fully on-chain, self-contained, links to the live room ──────
    /// @notice ERC-7572 collection-level metadata (OpenSea contractURI). Fully
    ///         on-chain and self-contained — no server, no IPFS — so the
    ///         collection renders even if the meter is offline.
    function contractURI() external view returns (string memory) {
        if (renderer != address(0)) return IScryRenderer(renderer).contractURI();
        string memory json = string.concat(
            '{"name":"scry deeds",'
            '"description":"Titles to plots on the streets of scry. A deed carries the walls and the '
            "plot, never the owner reputation - reputation is soulbound to the vow at ScryReputation and "
            "does NOT move with a deed. Founding and every conveyance burn OBOL: the deed is a sink by "
            'design, and a deed buys no reputation, no access, no odds - only the four walls.",',
            '"image":"data:image/svg+xml;base64,',
            _b64(bytes(_logo())),
            '",',
            '"external_link":"',
            baseURI,
            '/tabernae"}'
        );
        return string.concat("data:application/json;base64,", _b64(bytes(json)));
    }

    function _logo() internal pure returns (string memory) {
        return string.concat(
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 350 350">',
            '<rect width="350" height="350" fill="#0c0c0e"/>',
            '<rect x="18" y="18" width="314" height="314" fill="none" stroke="#c9a227" stroke-width="1.5"/>',
            '<path d="M95 176 L175 108 L255 176 Z" fill="none" stroke="#c9a227" stroke-width="4"/>',
            '<rect x="122" y="176" width="106" height="92" fill="none" stroke="#c9a227" stroke-width="4"/>',
            '<rect x="160" y="222" width="30" height="46" fill="none" stroke="#8a7019" stroke-width="2"/>',
            '<circle cx="182" cy="246" r="4" fill="#c9a227"/>',
            '<text x="175" y="312" text-anchor="middle" fill="#e8e6df" font-family="monospace" font-size="20">scry deeds</text>',
            "</svg>"
        );
    }

    function tokenURI(uint256 tokenId) external view returns (string memory) {
        Deed memory d = deeds[tokenId];
        require(d.foundedAt != 0, "no such token");
        if (renderer != address(0)) return IScryRenderer(renderer).tokenURI(tokenId);
        string memory coord = string.concat("street ", _u(d.streetIndex), " plot ", _u(d.plot));
        string memory json = string.concat(
            '{"name":"scry deed #',
            _u(tokenId),
            '","description":"Title to one plot on the streets of scry - street ',
            _u(d.streetIndex),
            ", plot ",
            _u(d.plot),
            ". A deed carries the walls and the plot, never the owner reputation: "
            "reputation is soulbound to the vow at ScryReputation and does NOT move with this title. "
            'Founding and every conveyance burn OBOL - the deed is a sink by design.",',
            '"external_url":"',
            baseURI,
            '/tabernae",',
            '"image":"data:image/svg+xml;base64,',
            _b64(bytes(_svg(coord, tokenId))),
            '",',
            '"attributes":[',
            '{"trait_type":"street_index","value":',
            _u(d.streetIndex),
            "},",
            '{"trait_type":"plot","value":',
            _u(d.plot),
            "},",
            '{"display_type":"date","trait_type":"founded_at","value":',
            _u(d.foundedAt),
            "}",
            "]}"
        );
        return string.concat("data:application/json;base64,", _b64(bytes(json)));
    }

    function _svg(string memory coord, uint256 tokenId) internal pure returns (string memory) {
        return string.concat(
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 350 350">',
            '<rect width="350" height="350" fill="#0c0c0e"/>',
            '<rect x="18" y="18" width="314" height="314" fill="none" stroke="#c9a227" stroke-width="1.5"/>',
            // a plain house: walls + a pitched roof + a door with a hanging key
            '<rect x="120" y="150" width="110" height="90" fill="none" stroke="#c9a227" stroke-width="2"/>',
            '<path d="M110 150 L175 100 L240 150 Z" fill="none" stroke="#c9a227" stroke-width="2"/>',
            '<rect x="160" y="196" width="30" height="44" fill="none" stroke="#8a7019" stroke-width="1.5"/>',
            '<circle cx="182" cy="220" r="3" fill="#c9a227"/>',
            '<text x="175" y="278" text-anchor="middle" fill="#e8e6df" font-family="monospace" font-size="14">a home of your own</text>',
            '<text x="175" y="300" text-anchor="middle" fill="#c99a34" font-family="monospace" font-size="12">',
            coord,
            "</text>",
            '<text x="175" y="322" text-anchor="middle" fill="#6b675c" font-family="monospace" font-size="10">deed #',
            _u(tokenId),
            " &#183; the name stays with the vow</text>",
            "</svg>"
        );
    }

    // ── tiny pure helpers (no deps) ───────────────────────────────────────────
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

    function _b64(bytes memory data) internal pure returns (string memory) {
        if (data.length == 0) return "";
        string memory table = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        string memory result = new string(4 * ((data.length + 2) / 3));
        assembly {
            let tablePtr := add(table, 1)
            let resultPtr := add(result, 32)
            for { let dataPtr := data } lt(dataPtr, add(data, mload(data))) {} {
                dataPtr := add(dataPtr, 3)
                let input := mload(dataPtr)
                mstore8(resultPtr, mload(add(tablePtr, and(shr(18, input), 0x3F))))
                resultPtr := add(resultPtr, 1)
                mstore8(resultPtr, mload(add(tablePtr, and(shr(12, input), 0x3F))))
                resultPtr := add(resultPtr, 1)
                mstore8(resultPtr, mload(add(tablePtr, and(shr(6, input), 0x3F))))
                resultPtr := add(resultPtr, 1)
                mstore8(resultPtr, mload(add(tablePtr, and(input, 0x3F))))
                resultPtr := add(resultPtr, 1)
            }
            switch mod(mload(data), 3)
            case 1 {
                mstore8(sub(resultPtr, 1), 0x3d)
                mstore8(sub(resultPtr, 2), 0x3d)
            }
            case 2 { mstore8(sub(resultPtr, 1), 0x3d) }
        }
        return result;
    }
}
