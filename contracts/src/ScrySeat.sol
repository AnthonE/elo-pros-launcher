// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

/// An on-chain metadata renderer — `ScrySeatArt` is the implementation. Kept as
/// an interface so the collection is not welded to one art contract before the
/// art is finished, and so the art can be swapped RIGHT UP UNTIL
/// `freezeMetadata()` and never after.
interface ISeatRenderer {
    function tokenURI(uint256 tokenId, uint8 tier, uint8 door, string calldata salt)
        external
        view
        returns (string memory);
    function contractURI() external view returns (string memory);
}

interface IERC721Receiver {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

// The town's burn address, at FILE level so a deploy script or a test can name
// it without an instance to ask. Same constant, same value, same reasoning as
// `ScryFeeSplitter`'s: burn by TRANSFER, never by assuming a `burn()` — SCRY is
// a fair-launched fixed-supply token this repo did not write and does not
// control, so its interface is not ours to assume.
address constant SEAT_BURN = 0x000000000000000000000000000000000000dEaD;

/// @title ScrySeat — the founder mint: a character that is a seat
/// @notice The Scry Hive as bytecode. A capped run of characters; mechanically each
///         one is a SEAT — it activates in tiers, it takes a weighted share of
///         what the platform hands out, and it clears its tier when it sells.
///
///         ⚠ THE SPLIT THIS CONTRACT EXISTS TO PRESERVE. The
///         seat transfers. **The record never does.** Nothing in here stores a
///         round, a clear, a badge, a rank, or any measurement — not because it
///         would be hard, but because a transferable record is what makes
///         account-selling launder everything on every other platform. The
///         record is soulbound to the wallet that earned it and lives nowhere
///         near this file. Selling a seat sells the seat; the buyer does not
///         get your record. That is the whole design and it is enforced here by
///         ABSENCE — read the storage below and note what is not in it.
///
///         WHAT A SEAT IS ENTITLED TO, and the wall it never crosses
///         (CLAUDE.md invariant 1): tier weight changes
///         HOW MANY cases/allotments a seat receives, and never the odds inside
///         one, never a meter number, never a rank, never a badge, never
///         matchmaking, never curation. The one test — *would the payer's
///         identity or the amount change any bit of any output?* — is answered
///         no by construction, because this contract computes no output. It
///         holds a tier and an election. That is all it holds.
///
///         THE FOUR DOORS, all posted, none discretionary:
///         1. SNAPSHOT — a merkle root over a PAST block (drop-one wallets,
///            SCRY holders). Already measured, already on chain, unfarmable
///            after announcement.
///         2. PLAY — a merkle root over a posted bar cleared in a listed title.
///            ⚠ the door no other mint on this chain can open, and the one that
///            waits on something being playable.
///         3. BUILD — a merkle root over work delivered through the board.
///         4. PAID — ETH or SCRY, first come, whatever is left.
///         `GATES.md` §4 governs all four: *the free cohort is posted, or it is
///         a favor.* The owner posts ROOTS for the three free doors; a root is
///         recomputable by anyone from a public ledger, which is the difference
///         between an allowlist and a favor.
///
///         ⚠ AND THERE IS A FIFTH DOOR, ADDED DELIBERATELY: `mintTreasury`.
///         Operator, 2026-08-08: *"idgaf about no team mint thats dumb lol. no
///         one cares its 2026 you either trust the crypto team or ur toast
///         anyways."* An earlier cut of this file refused a team allocation
///         outright and said so in three places; that was overruled and the
///         refusal is gone rather than quietly kept.
///         **What did NOT get given up is the bound.** `treasuryCap` is
///         immutable, it is carved out of supply at construction exactly as the
///         three reserved doors are, and `treasuryMinted` is public — so the
///         allocation is a number anyone reads off the ABI before minting, not
///         a discretionary tap. A capped, posted, on-chain-countable team mint
///         is a different object from an owner who can mint at will, and only
///         the second is what a rug screen is actually looking for.
///
///         AND THE RESERVE HAS AN EXPIRY DATE, WELDED AT DEPLOY. `reservedUntil`
///         is the instant the three free doors shut and everything they never
///         drew becomes paid float. There is no call that does it and no owner
///         who can choose the moment — it is a public immutable timestamp and
///         `block.timestamp` fires it. This is the answer to *"what if the free
///         mint does not mint out"*, and the shape matters more than the
///         feature: a team that can shift supply between doors is the thing a
///         rug screen hunts for, so the only version worth shipping is one that
///         was posted before mint #1 and that nobody can time. Set it to 0 and
///         an unclaimed reserve strands forever, which is what this contract
///         did before the window existed — see `_paidCeiling`.
///
///         PLAY AND BUILD ARE RESERVED, WHICH IS WHAT LETS THE MINT OPEN EARLY.
///         The design left one scheduling knob: wait for a playable title, or
///         open now against a reserved allocation. This contract implements the
///         second and makes the first a deploy choice — `playCap` and
///         `buildCap` are carved out of supply at construction and the paid
///         door CANNOT eat them (`_paidCeiling`). Set them to 0 and the design
///         collapses to the first option. So the scheduling question stays the
///         operator's, and either answer is a constructor argument rather than
///         an edit.
///
///         THE LADDER IS SUBLINEAR AND THE CONSTRUCTOR ENFORCES IT. The posted
///         claim — *25x the capital buys 3.33x the weight* — is a property,
///         not a paragraph, so it is checked at deploy: every tier above the
///         base must cost strictly more, weigh strictly more, and weigh LESS
///         than its cost multiple. A flat or superlinear ladder hands the whole
///         distribution to whoever arrives richest, which is the outcome that
///         makes a serious player read the platform as a farm and leave. This
///         contract cannot be deployed with one.
///
///         THE BURN IS WELDED HERE, NOT DELEGATED. Activation is paid in SCRY
///         and `activationBurnBps` (immutable) goes straight to 0xdEaD; only
///         the remainder reaches the splitter. Routing the whole fee through
///         `ScryFeeSplitter` would have been less code, but the splitter's
///         split is operator-editable and carries a `rescue()` — so the burn
///         would be an operator promise instead of a property of the
///         instrument. ⚠ Say it correctly wherever it is advertised, and this
///         is the sentence that is required: SCRY's `totalSupply()` does NOT
///         fall. The tokens become unspendable, which is the same economics and
///         a different sentence.
///
///         TIER CLEARS ON TRANSFER — the cleverest small thing StonkBrokers
///         built, and it makes the secondary market a RECURRING burn rather
///         than a one-time one. Every transfer clears, minting excepted. Their
///         docs say "true ownership transfer"; we do not try to tell a real
///         sale from a wallet hop, because that test is a heuristic and a
///         heuristic in a welded contract is a bug with a deploy date. Moving
///         your own seat between your own wallets re-tiers it. That is stated
///         on the card, and it is the honest version.
///
///         THE ELECTION (from the 2026-08-07 re-read of StonkBrokers' Clock In
///         v2). A seat elects up to `MAX_ELECTION`
///         tracks with weights of its choosing; no election on file means the
///         default track. This is deliberately the v2 shape and not v1's: they
///         shipped an owner-rotatable lineup plus a second engine (Overtime) on
///         a second revenue source, then RETIRED BOTH into one engine that
///         tallies per-holder elections. We are not copying the experiment, we
///         are copying its result — and the operator-discretion knob that
///         disappeared in their consolidation is one this repo would have had
///         to defend anyway.
///         ⚠ NOTHING DISTRIBUTES YET. This contract stores an election and
///         reads it back; it moves no goods and knows no track names. A
///         distributor reads `electionOf` and needs no migration when it ships.
///         Elections clear on transfer with the tier, for the same reason.
///
///         BIRTH GEAR IS DERIVED, NEVER STORED, AND HAS NO WITHDRAW PATH.
///         `saltCommitment` is `sha256(salt)`, immutable at deploy and before
///         mint #1; every seat's rolled cosmetic is `keccak256(salt ‖ tokenId)`,
///         published after mint-out so anyone recomputes the whole run in any
///         language and checks it against the commitment.
///         ⚠ THE TWO HASHES ARE DIFFERENT AND BOTH HALVES ARE LOAD-BEARING:
///         `sha256` SEALS the salt, `keccak256` ROLLS each seat off it
///         (`ScrySeatArt.traitsOf`, `art/seat/sprites.py`). This header said
///         sha256 for both, which is the one error that makes an honest
///         recomputation come out wrong — and recomputability is the entire
///         point of committing. This is
///         `ScryEidolon`'s sealed-salt deck, harvested exactly as designed —
///         the collection it was written for is dropped, the
///         machinery is the best thing in the repo. StonkBrokers seed a real
///         token into the NFT's wallet at mint, which is the one leg that can
///         be drained in the same block as a sale; ours is welded instead —
///         there is no balance and therefore nothing to front-run.
///
///         ERC-6551: a seat is a plain ERC-721, so any registry can bind an
///         account to it. ⚠ NOT the canonical registry — it does not clone and
///         initialize atomically on this chain. That registry is
///         its own deliverable and this contract deliberately does not name
///         one; binding is external and nothing here depends on it.
///
///         Owner powers, in full: post the three roots, arm/close the paid
///         door, set the base URI, reveal the salt, transfer ownership. No
///         mint, no burn, no transfer, no freeze, no re-roll, no tier grant, no
///         path to a holder's seat or a buyer's payment. Renouncing
///         (`transferOwnership(0)`) freezes every knob at its last posted value
///         — the posted door out (`HELD-KEYS.md`).
contract ScrySeat is ReentrancyGuard {
    string public name; // theme is an OPEN KNOB — not a constant, on purpose
    string public symbol;

    // ⚠ THIS STRING IS A CONSTANT AND THE WINDOW IS A CONSTRUCTOR ARGUMENT, so
    // it has to be true of BOTH deploys. It read "the three free doors shut at
    // reservedUntil" flatly, which is a promise a `reservedUntil = 0` deploy
    // does not keep — and a welded notice that describes the other build is the
    // worst kind of stale copy, because there is no edit that fixes it later.
    // Read the value, not the sentence: it is public and it was set before mint #1.
    string public constant NOTICE = "founder seats - four posted doors (snapshot, play, build, paid) plus a CAPPED treasury mint; "
        "where reservedUntil is set the three free doors shut on it and whatever they never drew becomes public float "
        "- a welded date, no release call, nobody's choice, and 0 means they never shut at all; "
        "tier is sublinear, paid in SCRY, part burned, and CLEARS ON TRANSFER; the record is soulbound "
        "elsewhere and never travels with the seat; gear sealed by saltCommitment before mint - "
        "standing and distribution weight, never power, odds, progression or a measurement";

    /// The town's burn address. Value sent here is provably unrecoverable by
    /// anyone, including the operator.
    address public constant BURN = SEAT_BURN;

    /// Most tracks one seat may elect at once. Their Clock In v2 allows 3.
    uint256 public constant MAX_ELECTION = 3;

    uint256 public immutable maxSupply;
    uint256 public immutable snapshotCap; // door 1 — a past block, already measured
    uint256 public immutable playCap; // door 2 — reserved; waits on a playable title
    uint256 public immutable buildCap; // door 3 — reserved; the board is live today
    uint256 public immutable treasuryCap; // the team allocation, welded at deploy
    /// When the three free doors shut and their unclaimed remainder becomes the
    /// paid float's. A unix timestamp, welded at deploy and public before mint
    /// #1; 0 means never, which is the original strand-forever behaviour.
    uint64 public immutable reservedUntil;
    IERC20 public immutable scry;
    address public immutable feeSplitter; // activation remainder + royalties
    address public immutable proceeds; // the ONLY place swept ETH can go
    uint96 public immutable royaltyBps; // ERC-2981
    uint16 public immutable activationBurnBps; // welded: the slice of every activation that ends at 0xdEaD
    bytes32 public immutable saltCommitment; // sha256(salt) — salt published after mint-out

    address public owner;
    string public baseURI;
    /// An on-chain `ScrySeatArt`, or address(0) to serve metadata off `baseURI`.
    address public renderer;
    /// One-way. Once true, neither `renderer` nor `baseURI` can ever move.
    bool public metadataFrozen;
    string public revealedSalt; // empty until the run is closed and the deck is opened
    bool public mintClosed; // one-way: every door shuts forever, and the run is what it is

    // ── the ladder — parallel arrays, welded at deploy ──────────────────────────
    // tier 1..tierCount; tier 0 is BENCHED and costs nothing. `tierCost[i]` is
    // the CUMULATIVE SCRY price of sitting at tier i+1, so an upgrade pays only
    // the difference and a seat is never charged twice for ground it holds.
    uint256[] private _tierCost;
    uint32[] private _tierWeight;

    uint256 public snapshotMinted;
    uint256 public playMinted;
    uint256 public buildMinted;
    uint256 public paidMinted;
    uint256 public treasuryMinted;
    uint256 public nextTokenId = 1;

    bytes32 public snapshotRoot; // 0 = door closed
    bytes32 public playRoot;
    bytes32 public buildRoot;
    /// door => wallet => seats taken at that door, for the wallet's LIFETIME.
    /// ⚠ KEYED BY DOOR, NOT BY ROOT, AND THAT IS THE WHOLE CORRECTNESS ARGUMENT.
    /// The play and build doors are *expected* to be re-rooted repeatedly as
    /// more wallets clear the bar or deliver work — that is what those doors
    /// are. Keyed by root, every republish would hand each wallet a fresh
    /// budget: a wallet with a cumulative allowance of 3 that had already taken
    /// 2 could take 3 more under the new root, for 5. Keyed by door, the same
    /// wallet takes exactly 1 more, and a corrected or extended root is safe to
    /// post as often as the ledger changes.
    /// **So an allowance in a leaf is a CUMULATIVE LIFETIME grant, never a
    /// per-root one**, and whoever builds a root must compute it that way.
    mapping(uint8 => mapping(address => uint256)) public claimedAtDoor;

    uint256 public paidPriceWei; // 0 = the ETH leg is shut
    uint256 public paidPriceScry; // 0 = the SCRY leg is shut

    /// Cumulative SCRY this contract has sent to BURN. Never decreases.
    /// ⚠ NOT a supply reduction — see the header. `totalSupply()` does not move.
    uint256 public totalSentToBurn;

    struct Election {
        uint16[MAX_ELECTION] trackId;
        uint16[MAX_ELECTION] weightBps; // sums to 10_000 when set; all zero = no election on file
    }

    mapping(uint256 => uint8) public tierOf; // 0 = benched. Cleared by every transfer.
    mapping(uint256 => uint8) public doorOf; // 1 snapshot · 2 play · 3 build · 4 paid · 5 treasury
    mapping(uint256 => uint64) public mintedAt;
    mapping(uint256 => Election) private _election;

    /// Running sum of every ACTIVATED seat's weight. Maintained in O(1) on
    /// activation and on the clear-on-transfer, so a distributor never has to
    /// iterate the collection to learn the denominator.
    /// ⚠ Added before broadcast, on purpose. A pull-based distributor needs a
    /// total it can read in one call; without this it must either walk every
    /// token id (unbounded gas) or keep its own shadow copy that drifts the
    /// first time a seat sells. There is no way to add it afterwards.
    uint256 public totalWeight;

    /// When each seat's tier last MOVED — set on activation and on the clear.
    /// ⚠ Its only job is to stop a tier change from reaching backwards into a
    /// distribution round that is already open. A round records the moment it
    /// opened; a seat whose tier moved after that moment sits the round out and
    /// counts from the next one. Without it, upgrading between the open and the
    /// claim mints a bigger share out of everyone else's.
    mapping(uint256 => uint64) public tierSetAt;

    mapping(uint256 => address) private _owner;
    mapping(address => uint256) private _balance;
    mapping(uint256 => address) private _approved;
    mapping(address => mapping(address => bool)) private _operator;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);
    event SeatMinted(uint256 indexed tokenId, address indexed to, uint8 indexed door);
    event Activated(uint256 indexed tokenId, uint8 indexed tier, uint256 paid, uint256 burned);
    event TierCleared(uint256 indexed tokenId, uint8 fromTier);
    event ElectionSet(uint256 indexed tokenId, uint16[MAX_ELECTION] trackId, uint16[MAX_ELECTION] weightBps);
    event DoorRootUpdated(uint8 indexed door, bytes32 indexed root);
    event PaidDoorSet(uint256 priceWei, uint256 priceScry);
    event MintClosed(uint256 finalSupply);
    event SaltRevealed(string salt);
    event Swept(uint256 amount);
    event OwnershipTransferred(address indexed from, address indexed to);
    event ContractURIUpdated(); // ERC-7572
    /// The last metadata event this collection can ever emit.
    event MetadataFrozen(address renderer, string baseURI);
    event MetadataUpdate(uint256 _tokenId); // ERC-4906
    event BatchMetadataUpdate(uint256 _fromTokenId, uint256 _toTokenId); // ERC-4906

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    /// Every undecided figure in the hive design is an argument here, so the
    /// operator's sentence is a deploy value and never a code edit.
    /// @param caps [maxSupply, snapshotCap, playCap, buildCap, treasuryCap,
    ///        reservedUntil]. The paid door takes whatever the four reserved
    ///        allocations do not — and, after `reservedUntil`, whatever the
    ///        three free doors never drew. The window rides in the caps array
    ///        rather than as a 13th parameter because this constructor is one
    ///        stack slot from `stack too deep`.
    ///        ⚠ `reservedUntil = 0` means the free reserve NEVER releases,
    ///        which is what this contract did before the window existed.
    /// @param tierCostScry CUMULATIVE SCRY per tier, ascending, 18-dec units.
    /// @param tierWeightX100 distribution weight per tier, x100 (base 100 = 1.00x),
    ///        ascending and SUBLINEAR in cost — enforced below.
    constructor(
        string memory _name,
        string memory _symbol,
        uint256[6] memory caps,
        IERC20 _scry,
        address _feeSplitter,
        address _proceeds,
        uint96 _royaltyBps,
        uint16 _activationBurnBps,
        bytes32 _saltCommitment,
        uint256[] memory tierCostScry,
        uint32[] memory tierWeightX100,
        string memory _baseURI
    ) {
        require(bytes(_name).length > 0 && bytes(_symbol).length > 0, "unnamed");
        require(caps[0] > 0, "zero supply");
        require(caps[1] + caps[2] + caps[3] + caps[4] <= caps[0], "reserved doors exceed supply");
        // A window already in the past would deploy a contract whose free doors
        // never opened at all — every reserved seat public from block one, and
        // the posted cohorts a fiction. Refuse it here rather than discover it
        // when the first holder cannot claim.
        require(caps[5] == 0 || caps[5] > block.timestamp, "reserve window already closed");
        require(caps[5] <= type(uint64).max, "window out of range");
        require(address(_scry) != address(0), "zero scry");
        require(_feeSplitter != address(0), "zero splitter");
        require(_proceeds != address(0), "zero proceeds");
        require(_royaltyBps <= 1000, "royalty > 10%");
        require(_activationBurnBps <= 10_000, "burn > 100%");
        require(_saltCommitment != bytes32(0), "zero commitment");
        require(tierCostScry.length > 0, "no ladder");
        require(tierCostScry.length == tierWeightX100.length, "ladder mismatch");
        require(tierCostScry.length <= 255, "ladder too deep"); // tierOf is uint8
        require(tierCostScry[0] > 0 && tierWeightX100[0] > 0, "zero base tier");

        // ⚠ THE SUBLINEARITY CHECK — the posted ladder, welded. Cost and weight both
        // strictly ascend, and every tier's weight multiple over the base stays
        // strictly BELOW its cost multiple over the base. Cross-multiplied, so
        // no division and no rounding slack:
        //     w[i]/w[0] < c[i]/c[0]   <=>   w[i]*c[0] < w[0]*c[i]
        // A ladder that hands proportional (or better) weight to the largest
        // wallet cannot be deployed by this constructor.
        for (uint256 i = 1; i < tierCostScry.length; i++) {
            require(tierCostScry[i] > tierCostScry[i - 1], "ladder cost not ascending");
            require(tierWeightX100[i] > tierWeightX100[i - 1], "ladder weight not ascending");
            require(
                uint256(tierWeightX100[i]) * tierCostScry[0] < uint256(tierWeightX100[0]) * tierCostScry[i],
                "ladder not sublinear"
            );
        }

        name = _name;
        symbol = _symbol;
        maxSupply = caps[0];
        snapshotCap = caps[1];
        playCap = caps[2];
        buildCap = caps[3];
        treasuryCap = caps[4];
        reservedUntil = uint64(caps[5]);
        scry = _scry;
        feeSplitter = _feeSplitter;
        proceeds = _proceeds;
        royaltyBps = _royaltyBps;
        activationBurnBps = _activationBurnBps;
        saltCommitment = _saltCommitment;
        _tierCost = tierCostScry;
        _tierWeight = tierWeightX100;
        baseURI = _baseURI;
        owner = msg.sender;
        emit OwnershipTransferred(address(0), msg.sender);
        emit ContractURIUpdated();
    }

    // ── admin: the posted knobs, and nothing else ────────────────────────────
    function transferOwnership(address newOwner) external onlyOwner {
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner; // address(0) renounces: every knob freezes where it stands
    }

    function setBaseURI(string calldata newBase) external onlyOwner {
        require(!metadataFrozen, "metadata frozen");
        baseURI = newBase;
        emit ContractURIUpdated();
        if (nextTokenId > 1) emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    /// Publish (or republish) a door's allowlist. Each root is computed off a
    /// PUBLIC ledger — the holder snapshot at a pinned past block, the play
    /// ledger, the board's delivered work — so anyone recomputes it and checks
    /// that no wallet was slipped in. bytes32(0) shuts that door.
    /// ⚠ Re-rooting a door does NOT reset what a wallet already took — the
    /// claimed counter is keyed by DOOR, not by root (see `claimedAtDoor`). So
    /// republishing is safe and is the normal operation for the play and build
    /// doors, and the allowance in a leaf must be the wallet's CUMULATIVE
    /// lifetime grant rather than a per-root one.
    function setDoorRoot(uint8 door, bytes32 root) external onlyOwner {
        require(door >= 1 && door <= 3, "no such gated door");
        if (door == 1) snapshotRoot = root;
        else if (door == 2) playRoot = root;
        else buildRoot = root;
        emit DoorRootUpdated(door, root);
    }

    /// Open, reprice, or shut the paid door. Either leg may be priced; 0 shuts
    /// that leg. The supply and reserve ceilings hold regardless, so no price
    /// here can eat a reserved door.
    function setPaidDoor(uint256 priceWei, uint256 priceScry) external onlyOwner {
        paidPriceWei = priceWei;
        paidPriceScry = priceScry;
        emit PaidDoorSet(priceWei, priceScry);
    }

    /// Shut every door, forever. A capped founder run needs an END that does
    /// not depend on selling out, and the reserved doors mean this run very
    /// likely does not sell out (see `_paidCeiling`). Without this the deck
    /// could never be opened: `revealSalt` must come after the last mint, and
    /// "the last mint" would otherwise never arrive.
    function closeMint() external onlyOwner {
        require(!mintClosed, "already closed");
        mintClosed = true;
        emit MintClosed(nextTokenId - 1);
    }

    /// Open the deck. Callable once, only after the last seat can possibly be
    /// minted, and only with the salt this contract committed to before mint
    /// #1 — the check is the honesty anchor, so a wrong or convenient salt
    /// reverts.
    /// ⚠ THE ORDER IS LOAD-BEARING. Reveal before the run ends and a minter
    /// computes `keccak256(salt ‖ nextTokenId)` before paying, which turns a
    /// sealed deck into a menu. Hence: minted out, or closed.
    function revealSalt(string calldata salt) external onlyOwner {
        require(bytes(revealedSalt).length == 0, "already revealed");
        require(mintClosed || nextTokenId > maxSupply, "mint still open");
        require(sha256(bytes(salt)) == saltCommitment, "salt does not match the commitment");
        revealedSalt = salt;
        emit SaltRevealed(salt);
        emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    // ── the four doors ───────────────────────────────────────────────────────
    /// Doors 1–3: prove a leaf in the posted root and take one seat, up to the
    /// caller's cumulative lifetime allowance at that door. Free — these
    /// cohorts were earned or measured, not sold.
    function claim(uint8 door, uint256 allowance, bytes32[] calldata proof)
        external
        nonReentrant
        returns (uint256 tokenId)
    {
        (bytes32 root, uint256 minted, uint256 cap) = _gatedDoor(door);
        require(root != bytes32(0), "door closed");
        require(_gatedOpen(), "the reserve window has closed");
        require(_verify(root, msg.sender, allowance, proof), "not in this cohort");
        require(claimedAtDoor[door][msg.sender] < allowance, "allowance spent");
        require(minted < cap, "door's reserve spent");

        claimedAtDoor[door][msg.sender] += 1;
        if (door == 1) snapshotMinted++;
        else if (door == 2) playMinted++;
        else buildMinted++;

        return _mint(msg.sender, door);
    }

    /// Door 4, ETH leg. Exact payment only — no refund path to audit. ETH rests
    /// here until anyone calls `sweep()`; the owner holds no path to redirect it.
    function buyWithEth() external payable nonReentrant returns (uint256) {
        require(paidPriceWei > 0, "eth leg shut");
        require(msg.value == paidPriceWei, "wrong payment");
        require(paidMinted < _paidCeiling(), "paid door spent");
        paidMinted++;
        return _mint(msg.sender, 4);
    }

    /// Door 4, SCRY leg. Straight to the splitter, whose posted split decides
    /// what burns — mint proceeds are the splitter's business. Activation is
    /// the leg whose burn is welded here, and deliberately the only one.
    function buyWithScry() external nonReentrant returns (uint256) {
        require(paidPriceScry > 0, "scry leg shut");
        require(paidMinted < _paidCeiling(), "paid door spent");
        paidMinted++;
        SafeERC20.safeTransferFrom(address(scry), msg.sender, feeSplitter, paidPriceScry, "scry payment failed");
        return _mint(msg.sender, 4);
    }

    /// What the paid door may take: supply less the three reserved doors and the
    /// treasury carve-out — plus, once the reserve window has passed, whatever
    /// those three doors never drew.
    ///
    /// ⚠ THIS NARROWS A DELIBERATE EARLIER DECISION, SO READ WHY. The version
    /// before this one stranded an unclaimed reserve permanently and argued the
    /// case in this very comment: *the alternative — an unclaimed reserve
    /// quietly becoming inventory — is how a posted cohort turns back into a
    /// favor.* That objection is exactly right about the word **quietly**, and
    /// it is what `reservedUntil` is built around:
    ///
    ///   - the date is IMMUTABLE and public before mint #1, so it is posted in
    ///     the same sense a cohort root is posted;
    ///   - nobody triggers it. There is no release call, no owner act, no
    ///     discretion about when — `block.timestamp` decides and that is all;
    ///   - the gated doors SHUT at the same instant (`_gatedOpen`), so a seat
    ///     cannot be claimed at a free door and sold at the paid one out of the
    ///     same allocation.
    ///
    /// A team that can move supply between doors is what a rug screen is
    /// actually looking for. A posted expiry that fires itself
    /// is the opposite of that, and it fixes the real defect in the old shape:
    /// a play door that waits on a title nobody shipped used to burn its seats
    /// forever, which is not honesty, it is just a smaller collection nobody
    /// chose.
    ///
    /// ⚠ `reservedUntil == 0` KEEPS THE OLD BEHAVIOUR EXACTLY — strand forever,
    /// no window, no release. The earlier call therefore survives as a deploy
    /// argument rather than being overruled.
    ///
    /// The treasury carve-out is NOT released. It is the team's own allocation
    /// bounded by an immutable cap, and expiring it is a different promise from
    /// the one this window makes.
    function _paidCeiling() internal view returns (uint256) {
        uint256 base = maxSupply - snapshotCap - playCap - buildCap - treasuryCap;
        if (!reserveReleased()) return base;
        return base + (snapshotCap - snapshotMinted) + (playCap - playMinted) + (buildCap - buildMinted);
    }

    /// Has the reserve window passed? ⚠ `block.timestamp`, never `block.number`
    /// — this is Arbitrum Nitro and `block.number` is the PARENT chain's, so a
    /// block-paced window would run ~120x long (`RUNBOOK.md` §0c).
    function reserveReleased() public view returns (bool) {
        return reservedUntil != 0 && block.timestamp >= reservedUntil;
    }

    /// The three free doors take claims only while the window is open. After it
    /// closes their remainder is the float's, so a claim here would double-spend
    /// a seat the paid ceiling has already counted.
    function _gatedOpen() internal view returns (bool) {
        return !reserveReleased();
    }

    function paidRemaining() external view returns (uint256) {
        uint256 ceiling = _paidCeiling();
        return ceiling > paidMinted ? ceiling - paidMinted : 0;
    }

    /// The treasury allocation. Bounded by `treasuryCap`, which is immutable and
    /// carved out of supply at construction — so this can never reach into the
    /// float or into a reserved door, and the most it can ever take is a number
    /// that was public before mint #1.
    ///
    /// ⚠ It is deliberately NOT time-limited and NOT sequenced. Adding "only
    /// before the public sale" or "only once" would be theatre: the owner sets
    /// the roots and the sale, so any ordering rule they can also arrange around
    /// is not a constraint, it is a decoration. The cap is the real bound, and
    /// stating one honest limit beats stacking three that do nothing.
    function mintTreasury(address to, uint256 n) external onlyOwner nonReentrant {
        require(to != address(0), "zero to");
        require(n > 0, "nothing to mint");
        require(treasuryMinted + n <= treasuryCap, "treasury allocation spent");
        treasuryMinted += n;
        for (uint256 i = 0; i < n; i++) {
            _mint(to, 5);
        }
    }

    // ── activation: the ladder, paid in SCRY, part welded to 0xdEaD ──────────
    /// Sit your seat at `tier` (1..tierCount). Pays only the DIFFERENCE from
    /// the tier it currently holds, so upgrading is never a second full price.
    /// `activationBurnBps` of what you pay goes straight to 0xdEaD; the rest
    /// reaches the splitter.
    ///
    /// ⚠ Only the seat's holder may activate it — activation is a decision
    /// about someone's own property, and letting a stranger pay to move it
    /// would let anyone burn a seat's upgrade path out from under a buyer who
    /// was going to re-tier it themselves.
    function activate(uint256 tokenId, uint8 tier) external nonReentrant {
        require(ownerOf(tokenId) == msg.sender, "not your seat");
        require(tier > 0 && tier <= _tierCost.length, "no such tier");
        uint8 held = tierOf[tokenId];
        require(tier > held, "already at or above that tier");

        uint256 owed = _tierCost[tier - 1] - (held == 0 ? 0 : _tierCost[held - 1]);
        tierOf[tokenId] = tier;
        totalWeight = totalWeight + _tierWeight[tier - 1] - (held == 0 ? 0 : _tierWeight[held - 1]);
        tierSetAt[tokenId] = uint64(block.timestamp);

        uint256 burned = owed * activationBurnBps / 10_000;
        if (burned > 0) {
            totalSentToBurn += burned;
            SafeERC20.safeTransferFrom(address(scry), msg.sender, BURN, burned, "burn leg failed");
        }
        uint256 toSplitter = owed - burned;
        if (toSplitter > 0) {
            SafeERC20.safeTransferFrom(address(scry), msg.sender, feeSplitter, toSplitter, "splitter leg failed");
        }
        emit Activated(tokenId, tier, owed, burned);
        // ⚠ ERC-4906, AND IT IS NOT DECORATION HERE. `ScrySeatArt` renders tier
        // as a live attribute of the token, so a seat that sits down has NEW
        // metadata — and a marketplace serves whatever it cached until an event
        // tells it otherwise. This contract declares 0x49064906 in
        // `supportsInterface`, so the notice has to exist or the declaration is
        // a claim nothing backs.
        emit MetadataUpdate(tokenId);
    }

    /// The seat's distribution weight, x100. A benched seat weighs nothing —
    /// it is not broken, it is off the roster.
    function weightOf(uint256 tokenId) external view returns (uint32) {
        uint8 t = tierOf[tokenId];
        return t == 0 ? 0 : _tierWeight[t - 1];
    }

    function tierCount() external view returns (uint256) {
        return _tierCost.length;
    }

    /// The whole posted ladder in one call, so a card renders it without
    /// guessing and an auditor reads it off the ABI.
    function ladder() external view returns (uint256[] memory costs, uint32[] memory weights) {
        return (_tierCost, _tierWeight);
    }

    // ── the election (read by whatever distributes; nothing does yet) ────────
    /// Direct this seat's share across up to `MAX_ELECTION` tracks. Weights are
    /// basis points and must sum to exactly 10,000 — all-in on one track is
    /// fine. Passing an all-zero weight set clears the election back to the
    /// default. Only the holder may set it; it clears when the seat sells.
    function setElection(
        uint256 tokenId,
        uint16[MAX_ELECTION] calldata trackId,
        uint16[MAX_ELECTION] calldata weightBps
    ) external {
        require(ownerOf(tokenId) == msg.sender, "not your seat");
        uint256 sum;
        for (uint256 i = 0; i < MAX_ELECTION; i++) {
            sum += weightBps[i];
            if (weightBps[i] == 0) continue;
            // A weighted track must be distinct — two slots on one track is a
            // reader's ambiguity, not an expression of preference.
            for (uint256 j = 0; j < i; j++) {
                require(weightBps[j] == 0 || trackId[j] != trackId[i], "duplicate track");
            }
        }
        require(sum == 0 || sum == 10_000, "weights must sum to 10000");
        _election[tokenId] = Election(trackId, weightBps);
        emit ElectionSet(tokenId, trackId, weightBps);
    }

    /// The seat's election. All-zero weights mean NO election on file, and a
    /// distributor reading that must fall back to its own posted default — this
    /// contract names no track and holds no default of its own.
    function electionOf(uint256 tokenId)
        external
        view
        returns (uint16[MAX_ELECTION] memory trackId, uint16[MAX_ELECTION] memory weightBps)
    {
        Election storage e = _election[tokenId];
        return (e.trackId, e.weightBps);
    }

    // ── ETH out: one address, welded, anyone may push ────────────────────────
    function sweep() external nonReentrant {
        uint256 amount = address(this).balance;
        require(amount > 0, "nothing to sweep");
        (bool ok,) = proceeds.call{value: amount}("");
        require(ok, "sweep failed");
        emit Swept(amount);
    }

    // ── ERC-721 core ─────────────────────────────────────────────────────────
    function totalMinted() external view returns (uint256) {
        return nextTokenId - 1;
    }

    function totalSupply() external view returns (uint256) {
        return nextTokenId - 1; // nothing burns a seat
    }

    function ownerOf(uint256 tokenId) public view returns (address o) {
        o = _owner[tokenId];
        require(o != address(0), "no such seat");
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

    /// ⚠ THE CLEAR LIVES HERE, so every transfer path shares it — a marketplace
    /// sale, a safeTransfer, an approved operator move. Selling the seat sells
    /// the seat; the tier and the election do not travel with it, and the buyer
    /// re-tiers at their own cost. That is what makes the secondary market a
    /// recurring burn instead of a one-time one.
    function transferFrom(address from, address to, uint256 tokenId) public {
        require(to != address(0), "zero to");
        address o = ownerOf(tokenId);
        require(o == from, "from != owner");
        require(msg.sender == o || _approved[tokenId] == msg.sender || _operator[o][msg.sender], "not authorized");
        _approved[tokenId] = address(0);
        _owner[tokenId] = to;
        _balance[from] -= 1;
        _balance[to] += 1;
        _clearOnTransfer(tokenId);
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

    function _clearOnTransfer(uint256 tokenId) internal {
        uint8 held = tierOf[tokenId];
        if (held != 0) {
            tierOf[tokenId] = 0;
            totalWeight -= _tierWeight[held - 1];
            tierSetAt[tokenId] = uint64(block.timestamp);
            emit TierCleared(tokenId, held);
            // The clear rewrites the token's metadata exactly as an activation
            // does, so it carries the same ERC-4906 notice (see `activate`).
            // Only when a tier actually fell: the election is not rendered, so
            // clearing one changes no served byte and owes no event.
            emit MetadataUpdate(tokenId);
        }
        Election storage e = _election[tokenId];
        for (uint256 i = 0; i < MAX_ELECTION; i++) {
            if (e.weightBps[i] != 0 || e.trackId[i] != 0) {
                delete _election[tokenId];
                uint16[MAX_ELECTION] memory zero;
                emit ElectionSet(tokenId, zero, zero);
                break;
            }
        }
    }

    function _mint(address to, uint8 door) internal returns (uint256 tokenId) {
        require(!mintClosed, "the mint is closed");
        require(nextTokenId <= maxSupply, "every seat is taken");
        tokenId = nextTokenId++;
        doorOf[tokenId] = door;
        mintedAt[tokenId] = uint64(block.timestamp); // ⚠ timestamp, never block.number (RUNBOOK §0c)
        _owner[tokenId] = to;
        _balance[to] += 1;
        emit Transfer(address(0), to, tokenId);
        emit SeatMinted(tokenId, to, door);
    }

    function _gatedDoor(uint8 door) internal view returns (bytes32 root, uint256 minted, uint256 cap) {
        if (door == 1) return (snapshotRoot, snapshotMinted, snapshotCap);
        if (door == 2) return (playRoot, playMinted, playCap);
        if (door == 3) return (buildRoot, buildMinted, buildCap);
        revert("no such gated door");
    }

    /// Sorted-pair keccak merkle over (wallet, allowance) — the same shape the
    /// meter's root pipeline already produces for the drops and the trials, so
    /// all three doors are served by machinery that exists. 52-byte leaf
    /// preimage vs 64-byte node preimage closes the second-preimage forgery
    /// without double-hashing.
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

    // ── metadata ─────────────────────────────────────────────────────────────
    /// Two sources, in order: an on-chain `ScrySeatArt` if one is set, else the
    /// meter. The renderer is the destination — `baseURI` is the bootstrap that
    /// lets the collection mint before the art is finished, and the fallback if
    /// a renderer is never set at all.
    ///
    /// ⚠ WHY THE FREEZE EXISTS. Until `freezeMetadata()` runs, `setBaseURI` can
    /// repoint this collection's art at anything, forever, from one key. That
    /// is the NFT-shaped version of the question this audience actually asks —
    /// not "can the team take my money" but "can the team change my picture" —
    /// and the honest answer has to become **no**, by arithmetic rather than by
    /// promise. Freezing is one-way and there is no path back.
    function tokenURI(uint256 tokenId) external view returns (string memory) {
        require(mintedAt[tokenId] != 0, "no such seat");
        if (renderer != address(0)) {
            return ISeatRenderer(renderer).tokenURI(
                tokenId, tierOf[tokenId], doorOf[tokenId], revealedSalt
            );
        }
        return string.concat(baseURI, "/seat/", _u(tokenId), "/metadata");
    }

    function contractURI() external view returns (string memory) {
        if (renderer != address(0)) return ISeatRenderer(renderer).contractURI();
        return string.concat(baseURI, "/seat/collection");
    }

    /// Point the collection at an on-chain renderer. Refused once frozen.
    function setRenderer(address r) external onlyOwner {
        require(!metadataFrozen, "metadata frozen");
        renderer = r;
        emit ContractURIUpdated();
        if (nextTokenId > 1) emit BatchMetadataUpdate(1, nextTokenId - 1);
    }

    /// ⚠ ONE WAY, AND IT IS THE LAST METADATA ACT. After this neither the
    /// renderer nor the base URI can move again, from any key, ever. Run it
    /// only once the salt is revealed and the art reads correctly — a frozen
    /// pointer at a wrong renderer is just as permanent as a right one.
    function freezeMetadata() external onlyOwner {
        require(!metadataFrozen, "already frozen");
        metadataFrozen = true;
        emit MetadataFrozen(renderer, baseURI);
    }

    function _u(uint256 x) internal pure returns (string memory) {
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
