// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "./ReentrancyGuard.sol";

interface ITicketReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

/// @title EloTicket — burnable mint tickets for the Thousand
/// @notice One ticket summons one eidolon vessel and burns in the act. Exactly
///         `MAX_SUPPLY` (1000) tickets can ever exist — the same number as the
///         vessels — so every ticket that leaves this contract is always
///         redeemable; there is no oversold lottery and no worthless tail.
///
///         Tickets enter the world through four posted doors, each with its
///         own hard cap fixed at deploy:
///
///         1. TRIALS  — earned free by playing (the Delver's Trials). A merkle
///            `trialsRoot` over (wallet, allowance) leaves, computed by the
///            meter from a PUBLIC play ledger — a transparent allowlist, not a
///            discretionary mint power.
///         2. SNAPSHOT — a holder snapshot pays a fixed ETH price inside a
///            posted window (`snapshotOpensAt` .. + `SNAPSHOT_WINDOW`). Same
///            recomputable merkle shape; price and opening are armed before
///            the window and cannot move once it is set.
///         3. GACHA   — the owner mints a capped tranche to the house wallet
///            to seed EloGacha positions, each escrowed with ETH backing.
///            The gacha's standing buyback IS the ticket floor — escrowed in
///            that contract, not promised by this one.
///         4. PUBLIC  — whatever remains sells open at a posted ETH price,
///            first come, until the thousand tickets exist.
///
///         Tickets are ordinary transferable ERC-721s on purpose: secondary
///         trading (OpenSea or anywhere) is expected and welcome. ETH taken
///         here accumulates in the contract and `sweep()` — callable by
///         anyone — pushes it to the immutable `proceeds` address; the owner
///         never holds a path to redirect a buyer's payment.
///
///         A ticket is a collectible key, not a measurement: nothing about it
///         reads, prices, or moves any meter number. Owner powers are the
///         posted knobs only (roots, sale arming, base URI); renouncing
///         ownership (transferOwnership(0)) freezes every knob at its last
///         posted value — the door out (HELD-KEYS.md).
contract EloTicket is ReentrancyGuard {
    string public name = "elo mint tickets - the thousand";
    string public symbol = "TICKET";
    string public constant NOTICE = "1000 burnable mint tickets for the eidolon vessels - four posted doors in "
        "(trials earn, snapshot sale, gacha floor, open sale), one door out (the summoning burn); "
        "a key, not a measurement";

    uint256 public constant MAX_SUPPLY = 1000;

    uint256 public immutable trialsCap; // earned free from play
    uint256 public immutable snapshotCap; // snapshot allowlist, priced window
    uint256 public immutable gachaCap; // owner-minted to seed the gacha
    uint256 public immutable snapshotWindow; // seconds the snapshot sale stays open
    address public immutable proceeds; // the only place swept ETH can go
    uint96 public immutable royaltyBps; // ERC-2981, paid to `proceeds`

    address public owner; // admin: roots / sale knobs / baseURI / setEidolon-once
    address public eidolon; // the one burner — set once, then welded
    string public baseURI;

    // ── door 1: trials (earned, free) ────────────────────────────────────────
    bytes32 public trialsRoot; // merkle over (wallet, allowance); 0 = closed
    uint256 public trialsMinted;
    mapping(address => uint256) public trialsClaimed;

    // ── door 2: snapshot (allowlist, priced, windowed) ───────────────────────
    bytes32 public snapshotRoot;
    uint256 public snapshotPriceWei;
    uint256 public snapshotOpensAt; // 0 = unarmed; window = [opensAt, opensAt + snapshotWindow)
    uint256 public snapshotMinted;
    mapping(address => uint256) public snapshotClaimed;

    // ── door 3: gacha tranche (owner mints to the house to escrow-back) ──────
    uint256 public gachaMinted;

    // ── door 4: public sale (open, priced) ───────────────────────────────────
    uint256 public publicPriceWei; // 0 = closed
    uint256 public publicCap; // posted when the sale opens ("whatever is left")
    uint256 public publicMinted;

    uint256 public nextTokenId = 1; // ids run 1..1000, all doors share the counter
    uint256 public totalBurned;
    mapping(uint256 => uint8) public trancheOf; // 1 trials · 2 snapshot · 3 gacha · 4 public
    mapping(uint256 => address) private _owner;
    mapping(address => uint256) private _balance;
    mapping(uint256 => address) private _approved;
    mapping(address => mapping(address => bool)) private _operator;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);
    event TicketMinted(uint256 indexed tokenId, address indexed to, uint8 indexed tranche);
    event TicketBurned(uint256 indexed tokenId, address indexed from);
    event TrialsRootUpdated(bytes32 indexed root);
    event SnapshotArmed(bytes32 indexed root, uint256 priceWei, uint256 opensAt);
    event PublicSaleSet(uint256 priceWei, uint256 cap);
    event EidolonSet(address indexed eidolon);
    event Swept(uint256 amount);
    event OwnershipTransferred(address indexed from, address indexed to);
    event ContractURIUpdated(); // ERC-7572

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    constructor(
        uint256 _trialsCap,
        uint256 _snapshotCap,
        uint256 _gachaCap,
        uint256 _snapshotWindow,
        address _proceeds,
        uint96 _royaltyBps,
        string memory _baseURI
    ) {
        require(_proceeds != address(0), "zero proceeds");
        require(_trialsCap + _snapshotCap + _gachaCap <= MAX_SUPPLY, "caps exceed supply");
        require(_snapshotWindow > 0, "zero window");
        require(_royaltyBps <= 1000, "royalty > 10%");
        trialsCap = _trialsCap;
        snapshotCap = _snapshotCap;
        gachaCap = _gachaCap;
        snapshotWindow = _snapshotWindow;
        proceeds = _proceeds;
        royaltyBps = _royaltyBps;
        baseURI = _baseURI;
        owner = msg.sender;
        emit OwnershipTransferred(address(0), msg.sender);
        emit ContractURIUpdated();
    }

    // ── admin ────────────────────────────────────────────────────────────────
    function transferOwnership(address newOwner) external onlyOwner {
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner; // address(0) renounces: every knob freezes at its last posted value
    }

    /// Point the burner at the eidolon contract — once. The ticket deploys
    /// before the vessel contract (which takes this address as an immutable),
    /// so the wiring is a one-shot setter rather than a constructor arg; after
    /// the shot it is as welded as an immutable.
    function setEidolon(address _eidolon) external onlyOwner {
        require(eidolon == address(0), "eidolon already set");
        require(_eidolon != address(0), "zero eidolon");
        eidolon = _eidolon;
        emit EidolonSet(_eidolon);
    }

    function setBaseURI(string calldata newBase) external onlyOwner {
        baseURI = newBase;
        emit ContractURIUpdated();
    }

    /// Rename the collection. name/symbol are settable because this platform
    /// has been renamed more than once and a constant welds whichever name
    /// happened to be current on broadcast day into every wallet forever.
    /// Wallets cache both; the contract-level event asks them to re-pull.
    function setName(string calldata newName) external onlyOwner {
        name = newName;
        emit ContractURIUpdated();
    }

    function setSymbol(string calldata newSymbol) external onlyOwner {
        symbol = newSymbol;
        emit ContractURIUpdated();
    }

    /// Publish (or republish) the trials allowlist. The meter computes the
    /// root from the public play ledger, so anyone recomputes it and checks no
    /// wallet was slipped in. bytes32(0) closes the door.
    function setTrialsRoot(bytes32 root) external onlyOwner {
        trialsRoot = root;
        emit TrialsRootUpdated(root);
    }

    /// Arm the snapshot sale: allowlist root, ETH price, opening time. Only
    /// before it opens — once the window starts the terms are frozen, so the
    /// price a buyer saw posted is the price for everyone in the window.
    function armSnapshot(bytes32 root, uint256 priceWei, uint256 opensAt) external onlyOwner {
        require(snapshotOpensAt == 0 || block.timestamp < snapshotOpensAt, "snapshot already open");
        require(opensAt > block.timestamp, "opens in the past");
        require(root != bytes32(0) && priceWei > 0, "zero terms");
        snapshotRoot = root;
        snapshotPriceWei = priceWei;
        snapshotOpensAt = opensAt;
        emit SnapshotArmed(root, priceWei, opensAt);
    }

    /// Open (or reprice, or close with priceWei 0) the public sale. `cap` is
    /// the posted "whatever is left" — the global MAX_SUPPLY check still holds
    /// regardless, so a fat-fingered cap can never oversell the thousand.
    function setPublicSale(uint256 priceWei, uint256 cap) external onlyOwner {
        publicPriceWei = priceWei;
        publicCap = cap;
        emit PublicSaleSet(priceWei, cap);
    }

    /// Mint the gacha tranche to the house wallet, which escrows each ticket
    /// into EloGacha with ETH backing. The floor lives THERE, in escrow —
    /// this contract only caps how many tickets can take that door.
    function mintGacha(address to, uint256 n) external onlyOwner {
        require(gachaMinted + n <= gachaCap, "gacha tranche spent");
        gachaMinted += n;
        for (uint256 i = 0; i < n; i++) {
            _mint(to, 3);
        }
    }

    // ── the four doors ───────────────────────────────────────────────────────
    /// Door 1: claim an earned ticket, free. One per call, up to the caller's
    /// allowance in the published trials root.
    function claimTrials(uint256 allowance, bytes32[] calldata proof) external nonReentrant returns (uint256) {
        require(trialsRoot != bytes32(0), "trials door closed");
        require(_verify(trialsRoot, msg.sender, allowance, proof), "no trials pass");
        require(trialsClaimed[msg.sender] < allowance, "trials passes spent");
        require(trialsMinted < trialsCap, "trials tranche spent");
        trialsClaimed[msg.sender] += 1;
        trialsMinted += 1;
        return _mint(msg.sender, 1);
    }

    /// Door 2: buy at the snapshot price, inside the window, if the wallet is
    /// in the snapshot root. Exact payment only — no refund path to audit.
    function claimSnapshot(uint256 allowance, bytes32[] calldata proof)
        external
        payable
        nonReentrant
        returns (uint256)
    {
        require(snapshotOpensAt != 0 && block.timestamp >= snapshotOpensAt, "snapshot not open");
        require(block.timestamp < snapshotOpensAt + snapshotWindow, "snapshot window closed");
        require(_verify(snapshotRoot, msg.sender, allowance, proof), "not in the snapshot");
        require(snapshotClaimed[msg.sender] < allowance, "snapshot passes spent");
        require(snapshotMinted < snapshotCap, "snapshot tranche spent");
        require(msg.value == snapshotPriceWei, "wrong payment");
        snapshotClaimed[msg.sender] += 1;
        snapshotMinted += 1;
        return _mint(msg.sender, 2);
    }

    /// Door 4: open sale while the posted cap and the thousand both hold.
    function buyPublic() external payable nonReentrant returns (uint256) {
        require(publicPriceWei > 0, "public door closed");
        require(publicMinted < publicCap, "public tranche spent");
        require(msg.value == publicPriceWei, "wrong payment");
        publicMinted += 1;
        return _mint(msg.sender, 4);
    }

    /// The one door out: the eidolon contract burns a ticket as it summons a
    /// vessel. Nobody else — not even the owner — can burn someone's ticket.
    function burnFrom(address from, uint256 tokenId) external {
        require(msg.sender == eidolon, "only the eidolon burns");
        require(_owner[tokenId] == from, "not the holder");
        _approved[tokenId] = address(0);
        _owner[tokenId] = address(0);
        _balance[from] -= 1;
        totalBurned += 1;
        emit Transfer(from, address(0), tokenId);
        emit TicketBurned(tokenId, from);
    }

    /// Push accumulated ETH to the immutable proceeds address. Anyone may
    /// call; nobody can redirect.
    function sweep() external nonReentrant {
        uint256 amount = address(this).balance;
        (bool ok,) = proceeds.call{value: amount}("");
        require(ok, "sweep failed");
        emit Swept(amount);
    }

    function _mint(address to, uint8 tranche) internal returns (uint256 tokenId) {
        require(nextTokenId <= MAX_SUPPLY, "the thousand tickets exist");
        tokenId = nextTokenId++;
        trancheOf[tokenId] = tranche;
        _owner[tokenId] = to;
        _balance[to] += 1;
        emit Transfer(address(0), to, tokenId);
        emit TicketMinted(tokenId, to, tranche);
    }

    /// Sorted-pair keccak merkle over (wallet, allowance) — same shape as the
    /// eidolon pass ledger (eidolon_passes.py), so the meter's root pipeline
    /// serves both doors unchanged. 52-byte leaf preimage vs 64-byte node
    /// preimage closes the second-preimage forgery without double-hashing.
    function _verify(bytes32 root, address account, uint256 allowance, bytes32[] calldata proof)
        internal
        pure
        returns (bool)
    {
        bytes32 h = keccak256(abi.encodePacked(account, allowance));
        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 p = proof[i];
            h = h <= p ? keccak256(abi.encodePacked(h, p)) : keccak256(abi.encodePacked(p, h));
        }
        return h == root;
    }

    function totalMinted() external view returns (uint256) {
        return nextTokenId - 1;
    }

    function totalSupply() external view returns (uint256) {
        return nextTokenId - 1 - totalBurned;
    }

    // ── ERC-721 core ─────────────────────────────────────────────────────────
    function ownerOf(uint256 tokenId) public view returns (address o) {
        o = _owner[tokenId];
        require(o != address(0), "no such ticket");
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
                ITicketReceiver(to).onERC721Received(msg.sender, from, tokenId, data)
                    == ITicketReceiver.onERC721Received.selector,
                "unsafe recipient"
            );
        }
    }

    // ── ERC-2981 royalties (to proceeds, immutable) ──────────────────────────
    function royaltyInfo(uint256, uint256 salePrice) external view returns (address, uint256) {
        return (proceeds, salePrice * royaltyBps / 10_000);
    }

    function supportsInterface(bytes4 id) external pure returns (bool) {
        return id == 0x01ffc9a7 // ERC-165
            || id == 0x80ac58cd // ERC-721
            || id == 0x5b5e139f // ERC-721 Metadata
            || id == 0x2a55205a; // ERC-2981
    }

    // ── metadata ─────────────────────────────────────────────────────────────
    function tokenURI(uint256 tokenId) external view returns (string memory) {
        ownerOf(tokenId);
        return string.concat(baseURI, "/ticket/", _u(tokenId), "/metadata");
    }

    function contractURI() external view returns (string memory) {
        return string.concat(baseURI, "/ticket/collection");
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
