// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "./ReentrancyGuard.sol";
import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";

interface IGameTicketReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external returns (bytes4);
}

/// Minimal payment-side views of the two NFT standards. Only what a rail
/// needs to MOVE one from the buyer — never `safeTransferFrom` for ERC-721,
/// so a sink contract cannot brick the sale by not implementing a receiver.
/// `ownerOf` is the collection free door's whole question (see `claimFreeFor`)
/// and is `view`, so solc emits a STATICCALL and a hostile collection cannot
/// write anything back here.
interface IERC721Payment {
    function transferFrom(address from, address to, uint256 tokenId) external;
    function ownerOf(uint256 tokenId) external view returns (address);
}

interface IERC1155Payment {
    function safeTransferFrom(address from, address to, uint256 id, uint256 value, bytes calldata data) external;
}

/// Optional external metadata renderer, same shape `ScryDeed` already uses.
/// If the owner sets one it answers both URIs; `address(0)` restores the
/// built-in on-chain metadata.
interface IScryTicketRenderer {
    function tokenURI(uint256 tokenId) external view returns (string memory);
    function contractURI() external view returns (string memory);
}

/// @title ScryGameTicket — a copy of a game, as a token you own
/// @notice One deployment per title (`GATES.md` §4, rev. 2026-08-08: buying a
///         copy IS minting the ticket). Holding any ticket of a title is the
///         licence its official servers and the platform's depot check —
///         `entitled(wallet)` is the one question, and it is a plain
///         `balanceOf > 0`.
///
///         The price is posted in DOLLARS and paid over a RAIL — an asset the
///         owner has said this title accepts, at an amount the owner posted
///         (ETH, SCRY or USDG at deploy; anything else via `setRail`).
///
///         ⚠ THE FIGURE IS THE TITLE'S, NOT THE PLATFORM'S. There is no
///         platform price, no default, no floor and no ceiling anywhere in this
///         contract: `priceUsdCents` is born 0 and only ever holds what this
///         title's owner passed to `setPrices`, which they may repost as often
///         as they like. A dev charges what they want — a dollar, forty, or
///         nothing at all — so no surface may state a price for a title
///         without reading it back from that title's own deployment.
///
///         There is no oracle on chain: the owner posts the dollar figure and
///         the amounts together, a rail at amount 0 is closed, and the meter's
///         card renders any drift between the posted amounts and the live tape
///         so a buyer sees staleness before paying.
///
///         THE RAILS ARE DATA, NOT CODE (2026-08-09). Rails 1–3 are ETH, SCRY
///         and USDG, seeded at deploy and welded to their kind and token so
///         every surface that already reads `priceWei()` / `priceScry()` /
///         `priceUsdg()` keeps its meaning — those three getters are views over
///         the registry and their selectors did not move. Beyond them the owner
///         may post any number of custom rails in any of four kinds — NATIVE,
///         ERC-20, ERC-721, ERC-1155 — so taking a partner's coin, or an NFT
///         from a collection worth courting, is `setRail` rather than a
///         redeploy. `closeRail` shuts one without forgetting it.
///
///         Two walls the registry does not open. `setRail` refuses ids 1–5:
///         repointing the SCRY rail would leave every card captioned "half of
///         every SCRY buy burns" over a coin that does nothing of the kind, and
///         4/5 are the provenance marks a comp and a free claim leave in
///         `railOf`. And a rail can only ever change what a BUYER may pay
///         with — never what a HOLDER owns, never the constant-zero royalty,
///         never where ETH sweeps.
///
///         THE COPY AND THE COIN ARE ONE BUY (2026-08-09) — `COPIES.md` §0's
///         presale, which had a design and no instrument. `setGrant` posts what
///         a copy delivers alongside the licence; the reserve is FUNDED by an
///         ordinary transfer to this address and nothing here mints, because a
///         ticket with authority over somebody's token would make every listing
///         a trust question. An armed grant that cannot pay REVERTS the buy
///         rather than quietly delivering zero — a card promising a copy and
///         10,000 COIN over a dry reserve is the reward-the-rule-cannot-pay bug
///         with the money already taken. Comps and free claims grant nothing,
///         so no free door can walk the raise out of the reserve.
///
///         HANDOVER IS TWO STEPS, because `COPIES.md`'s shape ends with the
///         platform passing admin to the team that graduated. `transferOwnership`
///         offers, `acceptOwnership` takes, and until then the old owner still
///         owns everything — so a mistyped third-party address costs a second
///         call rather than freezing every knob forever. Renouncing to
///         `address(0)` stays one step: it only removes power, and it cancels
///         any offer standing at the time.
///
///         Where the money goes, welded at deploy — and ⚠ WELDED IS NOT THE
///         SAME AS KNOWN. Both destinations are immutable, but WHAT they are is
///         a deploy-time choice, so no surface may describe a title's money
///         path without reading the address:
///           - SCRY   → `scrySink`, straight from the buyer; this contract
///             never holds a wei of SCRY. Point it at the deployed
///             ScryFeeSplitter and the posted split applies — half of every
///             SCRY buy burns, and no burn is claimed until `distribute()` has
///             actually run. Point it at a `ScryLaunchpad` and the SCRY leg is
///             raise instead, paired into the game's pool and never
///             withdrawable. Both are legitimate; they are not the same
///             promise, and a card that says "half burns" over the second one
///             is lying.
///           - USDG   → `proceeds`, straight from the buyer. No custody here.
///           - ETH    → accumulates here; `sweep()` — callable by anyone —
///             pushes it to the immutable `proceeds`. Nobody can redirect.
///
///         Free copies exist in exactly two shapes, both on the record:
///           - `compMint` — the owner hands a copy to a named wallet, logged
///             by event. A favor is visible or it corrodes the ticket.
///           - `claimFree` — a posted merkle root over (wallet, allowance),
///             self-claimed. The free cohort is published, or it is a favor
///             (`GATES.md` §4). A closed root (0) closes the door.
///         Nothing else mints for free, which is the bot posture: every open
///         door either costs the posted price or requires membership in a
///         posted list.
///
///         Resale is the point: tickets are ordinary transferable ERC-721s,
///         and the royalty is a CONSTANT ZERO — not an owner knob. "You own
///         it" is only unarguable if no key can ever tax the exit
///         (`SENTENCES.md` 2026-08-03); reversing that costs a redeploy,
///         which is the right price for reversing a promise.
///
///         THE METADATA IS ON CHAIN, because the copy has to outlive us
///         (2026-08-10). It used to be `baseURI + "/ticket/<game>/<id>/metadata"`
///         — a url on scry's own origin — which made the ONE bearer asset on
///         the platform render its name and its image through our web server.
///         Origin dark, or that route quietly wrong, and a licence somebody
///         paid for and may resell shows a marketplace a blank name and a
///         broken image. It HAS been quietly wrong: `meter/tickets.py` served
///         `{}` from that route for a week with every suite green.
///
///         So `tokenURI` and `contractURI` now build a `data:application/json`
///         document from contract storage, exactly as `ScryDeed` already does
///         — *"no server, no IPFS — so the collection renders even if the
///         meter is offline."* `baseURI` survives with a narrower job: it is
///         the SITE base, and it appears only inside `external_url`, the one
///         field that is supposed to point at us. What a marketplace shows —
///         the name, the blurb, the image — is `setMetadata`, owner-posted,
///         and after graduation the owner is the game's own team.
///
///         A ticket is a licence, not a measurement: nothing about it reads,
///         prices, or moves any meter number. It never burns — a licence has
///         no door out. Paced by nothing: no block numbers, no windows.
contract ScryGameTicket is ReentrancyGuard {
    /// The TITLE, bare — "Gates", not "Gates on Scryward" and not "Gates copy".
    /// The collection name a marketplace shows adds the platform itself
    /// (`PLATFORM`, below), and an item is `name #id`.
    string public name;
    string public symbol;
    string public game; // the title's slug, as the depot knows it

    /// ⚠ THE WORD "COPY" IS GONE FROM EVERYTHING A BUYER READS (2026-08-12).
    /// Operator: *"the whole 'copy' thing is so incredibly beta… we managed to
    /// call it copy so much it sounds like I'm getting a fake copy."* He is
    /// right, and the word was load-bearing nowhere: what somebody buys here is
    /// the game. `copy` survives in field names and in the docs, where it means
    /// a unit of sale and no buyer ever sees it.
    ///
    /// The collection reads **"<title> on Scryward"** rather than *by* — a
    /// storefront credit is true for every listing, and most listings are not
    /// ours to take authorship of. That distinction is welded here on purpose:
    /// a title that leaves still minted on this platform, and its own team can
    /// point `renderer` anywhere it likes the day it wants a different name.
    string public constant PLATFORM = "Scryward";

    // ⚠ THIS STRING USED TO CLAIM THE BURN, AND IT COULD NOT KEEP THE PROMISE.
    // It read "paid in SCRY (half burns via the posted split)" — but `scrySink`
    // is a constructor argument checked only for being non-zero, so a title
    // pointing it at anything other than a fee splitter shipped a contract
    // whose own notice was false, welded, forever. That is the
    // reward-the-rule-cannot-pay bug living inside the contract rather than on
    // a card. What is true for EVERY deployment is that the destinations are
    // welded and readable, so the notice says that and names where to look.
    string public constant NOTICE = "the game itself, as a token - holding any ticket is what the official "
        "servers check at the door; posted dollar price over any rail the owner has opened (railInfo); where "
        "each rail's money goes is welded at deploy - read scrySink and proceeds and check what they are; free "
        "ones only by owner gift, posted merkle list, or holding a token in a collection the owner named "
        "(freeCollectionAt); resale free forever, royalty a constant 0";

    IERC20 public immutable scry;
    IERC20 public immutable usdg;
    address public immutable scrySink; // the fee splitter — SCRY buys land here, half burns by the posted split
    address public immutable proceeds; // ETH sweeps + USDG buys land here
    uint256 public immutable supplyCap; // 0 = uncapped: a game sells as many copies as people buy

    address public owner;

    // ── what a marketplace renders, held here ────────────────────────────────
    //
    // `baseURI` is the SITE base (e.g. "https://scry.moreright.xyz") and is
    // used for exactly one thing: the `external_url` that points a viewer back
    // at the game's page. It is deliberately NOT where metadata comes from —
    // see the header. An empty baseURI simply omits the link.
    string public baseURI;
    /// The blurb a marketplace shows. Owner-posted; empty falls back to NOTICE.
    string public blurb;
    /// The image a marketplace shows. Any URI the viewer can resolve —
    /// `ipfs://…`, `data:image/svg+xml;base64,…`, or an https url. Empty falls
    /// back to the built-in mark, so a copy always renders something.
    string public image;
    /// Optional external renderer. address(0) => the built-in on-chain document.
    address public renderer;

    // ── the posted price ─────────────────────────────────────────────────────
    // Cents, so 1234 is $12.34 — whatever THIS title's owner posted, with no
    // platform figure behind it. Informational: it is the posted claim, and
    // what a buyer actually pays is the rail amount.
    uint256 public priceUsdCents;

    // ── the rails — what this contract accepts, and for how much ─────────────
    //
    // WHY A REGISTRY. The first three rails were three hard-coded legs and
    // three `buyWith*` functions, so accepting a fourth asset was a redeploy.
    // A title should be able to take a partner's coin, or an NFT from a
    // collection it wants to court, without shipping new bytecode. So the
    // asset set is DATA the owner posts, and buying is one function over it.
    //
    // WHAT IS RESERVED, AND WHY IT IS NOT MERE TIDINESS. Rails 1–3 keep the
    // meaning every off-chain surface already reads — 1 is ETH, 2 is SCRY
    // (through the splitter, where half burns), 3 is USDG — and their KIND and
    // TOKEN are welded at deploy. `setRail` refuses them. Repointing rail 2 at
    // some other ERC-20 would leave the card still captioned "half of every
    // SCRY buy burns" over a coin that does no such thing: a surface lying
    // because a knob moved under it. The AMOUNT on those rails moves freely —
    // that is `setPrices`, unchanged.
    //
    // 4 and 5 are not payment rails at all. They are the provenance marks a
    // comp and a free claim leave in `railOf`, reserved here so a custom rail
    // can never collide with them and make a bought copy read as a gift.
    enum AssetKind {
        NATIVE,
        ERC20,
        ERC721,
        ERC1155
    }

    struct Rail {
        AssetKind kind;
        address token; // address(0) for NATIVE
        uint256 id; // ERC-1155 id; unused otherwise
        uint256 amount; // wei · token units · how many NFTs. 0 is NOT "free", it is shut
        address sink; // where payment lands. NATIVE ignores it — see `proceeds`
        bool open;
        string label; // what a surface calls it: "eth", "scry", a partner's name
    }

    uint256 public constant RAIL_ETH = 1;
    uint256 public constant RAIL_SCRY = 2;
    uint256 public constant RAIL_USDG = 3;
    uint256 public constant RAIL_COMP = 4; // provenance only, never a payment rail
    uint256 public constant RAIL_FREE = 5; // provenance only, never a payment rail
    uint256 public constant FIRST_CUSTOM_RAIL = 6;

    /// A rail paying in NFTs hands over this many at most. The bound is here
    /// because `amount` is owner-set and the loop is the buyer's gas.
    uint256 public constant MAX_NFT_PAYMENT = 32;

    mapping(uint256 => Rail) private _rails;
    uint256[] private _railIds; // every rail ever defined, for enumeration

    // ── the grant — a copy and the game's coin, in one buy ───────────────────
    //
    // `COPIES.md` §0: *"they also get a copy when they spend x$ AND get
    // gamecoins at that price until it graduates."* This is that, and it is the
    // half a rail cannot express — a rail says what a buyer PAYS, this says
    // what a buyer RECEIVES beyond the licence.
    //
    // The reserve is held HERE and funded by whoever is raising: an ordinary
    // transfer of the coin to this address. Nothing mints — this contract has
    // no authority over anybody's token and asking for one would make every
    // listing a trust question.
    //
    // ⚠ AN ARMED GRANT THAT CANNOT PAY REVERTS THE BUY. It does not quietly
    // deliver zero. A card reading "a copy and 10,000 COIN" over a dry reserve
    // is the *"card advertising a reward the rule cannot pay"* bug taking real
    // money, so the sale stops instead — visibly, and at the door.
    IERC20 public grantToken;
    uint256 public grantAmount; // per copy; 0 = no grant is offered
    uint256 public grantDelivered; // running total, so a raise is auditable

    address public pendingOwner; // handover is two steps — see transferOwnership

    // ── the free doors ───────────────────────────────────────────────────────
    bytes32 public freeRoot; // merkle over (wallet, allowance); 0 = closed
    mapping(address => uint256) public freeClaimed; // lifetime, never reset

    // THE COLLECTION DOOR — hold a token, claim against that token.
    //
    // WHY IT EXISTS BESIDE THE ROOT. A merkle root is a PHOTOGRAPH of a block
    // that has already happened; a collection is a LIVE set. So "free ones for
    // Scry Hive holders" through the root meant a snapshot, a proofs file the
    // origin serves, and a re-post of the root every time a seat changed hands
    // — an operator chore that never ends, under a promise a holder reads as
    // standing. `ownerOf` asks the collection the same question at claim time
    // and is done. The root stays for cohorts that are NOT a collection (SCRY
    // holders above dust, drop-one wallets, a hand-written list); nothing that
    // works today stops working.
    //
    // ⚠ THE COUNTER MOVES TO THE RIGHT OBJECT, AND THIS IS THE REAL ARGUMENT.
    // `freeClaimed` counts per WALLET, which is correct for a cohort of wallets
    // and wrong for a transferable NFT: sell the seat and the buyer is not in
    // the snapshot, so a perk sold as "the seat gets one" quietly belonged to
    // whoever held it on one particular block. Keyed by TOKEN ID, one seat is
    // one game forever, it survives every transfer, and splitting a collection
    // across ten wallets buys nothing — the same anti-farm property the merkle
    // got from pinning a block, without pinning anything.
    //
    // WHAT BOUNDS IT: `perToken` × that collection's supply, and `supplyCap` on
    // top if the title set one. Pointing this at a 10,000-supply collection on
    // an uncapped title is 10,000 free games, and that is the owner's arithmetic
    // to do before the call, not a knob this contract second-guesses.
    mapping(address => uint256) public freePerToken; // collection => copies each id may claim; 0 = shut
    /// collection => token id => how many it has taken. Lifetime, never reset,
    /// so re-posting `perToken` cannot re-open a spent id by accident — the
    /// same rule `freeClaimed` keeps for the root.
    mapping(address => mapping(uint256 => uint256)) public freeClaimedByToken;
    address[] private _freeCollections; // every collection ever named, for enumeration
    mapping(address => bool) private _knownFreeCollection;

    /// One call's worth of ids. The loop is the claimer's own gas, but an
    /// unbounded array is still a way to write a transaction nobody can mine.
    uint256 public constant MAX_FREE_CLAIM = 32;

    uint256 public nextTokenId = 1;
    mapping(uint256 => uint256) public railOf; // 1 eth · 2 scry · 3 usdg · 4 comp · 5 free · 6+ custom
    mapping(uint256 => address) private _owner;
    mapping(address => uint256) private _balance;
    mapping(uint256 => address) private _approved;
    mapping(address => mapping(address => bool)) private _operator;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);
    event ApprovalForAll(address indexed owner, address indexed operator, bool approved);
    event CopySold(uint256 indexed tokenId, address indexed to, uint256 indexed rail, uint256 paid);
    event CompMinted(uint256 indexed tokenId, address indexed to);
    event FreeClaimed(uint256 indexed tokenId, address indexed to);
    /// The collection door. `heldTokenId` is the SEAT (or whatever was held),
    /// `tokenId` is the game it just claimed — an indexer can walk either way
    /// and answer "has seat #412 taken its copy" without a snapshot.
    event FreeClaimedFor(
        uint256 indexed tokenId, address indexed collection, uint256 indexed heldTokenId, address to
    );
    event FreeCollectionPosted(address indexed collection, uint256 perToken);
    event PricesPosted(uint256 usdCents, uint256 ethWei, uint256 scryWei, uint256 usdgUnits);
    event RailPosted(
        uint256 indexed railId, uint8 kind, address indexed token, uint256 id, uint256 amount, address sink, bool open
    );
    event FreeRootUpdated(bytes32 indexed root);
    event Swept(uint256 amount);
    event OwnershipTransferred(address indexed from, address indexed to);
    event OwnershipOffered(address indexed from, address indexed to);
    event GrantPosted(address indexed token, uint256 amountPerCopy);
    event GrantDelivered(uint256 indexed tokenId, address indexed to, address indexed token, uint256 amount);
    event ContractURIUpdated(); // ERC-7572
    event MetadataUpdated(string blurb, string image);
    event RendererUpdated(address indexed renderer);
    event BatchMetadataUpdate(uint256 fromTokenId, uint256 toTokenId); // ERC-4906

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    constructor(
        string memory _name,
        string memory _symbol,
        string memory _game,
        IERC20 _scry,
        IERC20 _usdg,
        address _scrySink,
        address _proceeds,
        uint256 _supplyCap,
        string memory _baseURI,
        address _owner
    ) {
        require(address(_scry) != address(0) && address(_usdg) != address(0), "zero coin");
        require(_scrySink != address(0), "zero sink");
        require(_proceeds != address(0), "zero proceeds");
        // Both are IMMUTABLE, so `address(this)` here is unfixable rather than
        // merely wrong: SCRY buys would pile up in a contract whose only ERC-20
        // door is the repointable grant, and `sweep()` would pay ETH to itself
        // forever. Cheap to refuse at birth, impossible to undo after.
        require(_scrySink != address(this), "sink is this contract");
        require(_proceeds != address(this), "proceeds is this contract");
        name = _name;
        symbol = _symbol;
        game = _game;
        scry = _scry;
        usdg = _usdg;
        scrySink = _scrySink;
        proceeds = _proceeds;
        supplyCap = _supplyCap;
        baseURI = _baseURI;
        // THE OWNER IS A PARAMETER, and it is here for the factory. Handover
        // is deliberately two-step (`transferOwnership` offers, `acceptOwnership`
        // takes), which is right for a money-holding contract changing hands —
        // but it means a deployer that is a CONTRACT would own every ticket it
        // deployed until a human accepted, so a listing would take two
        // transactions and the factory would transiently own a live sale. A
        // birth-time owner costs one parameter and removes both. Zero keeps
        // the old behaviour, so a hand deploy still reads the same.
        owner = _owner == address(0) ? msg.sender : _owner;

        // The three rails every surface already knows, seeded shut. Their kind
        // and token are welded here and `setRail` refuses to touch them; only
        // the amount moves, which is what `setPrices` does.
        _seed(RAIL_ETH, AssetKind.NATIVE, address(0), address(0), "eth");
        _seed(RAIL_SCRY, AssetKind.ERC20, address(_scry), _scrySink, "scry");
        _seed(RAIL_USDG, AssetKind.ERC20, address(_usdg), _proceeds, "usdg");

        emit OwnershipTransferred(address(0), owner);
        emit ContractURIUpdated();
    }

    function _seed(uint256 railId, AssetKind kind, address token, address sink, string memory label) private {
        _rails[railId] =
            Rail({kind: kind, token: token, id: 0, amount: 0, sink: sink, open: false, label: label});
        _railIds.push(railId);
    }

    // ── admin: the posted knobs, nothing else ────────────────────────────────
    //
    // HANDOVER IS TWO STEPS, AND GRADUATION IS WHY. `COPIES.md`'s shape ends
    // with the platform passing admin of the contracts it set up to the team
    // that graduated — a transfer to a THIRD PARTY, typed once, against a
    // contract that is taking money. One-step, a mistyped address is not a bug
    // you fix: every knob freezes at whatever was last posted, forever, with no
    // door back. So the new owner has to prove the key works by using it.
    //
    // RENOUNCE STAYS ONE STEP, deliberately: it only ever REMOVES power, there
    // is nobody on the other end to accept, and `address(0)` cannot be mistyped
    // into a wrong-but-live key.
    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) {
            emit OwnershipTransferred(owner, address(0));
            owner = address(0);
            pendingOwner = address(0); // a renounce cancels any pending offer
            return;
        }
        pendingOwner = newOwner;
        emit OwnershipOffered(owner, newOwner);
    }

    /// The other half of a handover. Until this lands the old owner still owns
    /// the contract and every knob still answers to them, so a fumbled address
    /// costs nothing but a second call.
    function acceptOwnership() external {
        require(msg.sender == pendingOwner, "not the pending owner");
        emit OwnershipTransferred(owner, msg.sender);
        owner = msg.sender;
        pendingOwner = address(0);
    }

    /// Withdraw a standing offer.
    function cancelHandover() external onlyOwner {
        pendingOwner = address(0);
        emit OwnershipOffered(owner, address(0));
    }

    /// Post the dollar price and every reserved rail's amount together. An
    /// amount of 0 closes that rail; posting all three at 0 closes those three.
    /// Unchanged in name, signature and meaning — custom rails are `setRail`'s.
    function setPrices(uint256 usdCents, uint256 ethWei, uint256 scryWei, uint256 usdgUnits) external onlyOwner {
        priceUsdCents = usdCents;
        _post(RAIL_ETH, ethWei);
        _post(RAIL_SCRY, scryWei);
        _post(RAIL_USDG, usdgUnits);
        emit PricesPosted(usdCents, ethWei, scryWei, usdgUnits);
    }

    function _post(uint256 railId, uint256 amount) private {
        Rail storage r = _rails[railId];
        r.amount = amount;
        r.open = amount > 0;
        emit RailPosted(railId, uint8(r.kind), r.token, r.id, amount, r.sink, r.open);
    }

    /// Define or re-post a CUSTOM rail — any ERC-20, ERC-721 or ERC-1155, at
    /// any amount, landing anywhere the owner names.
    ///
    /// This is the modular half: accepting a partner's coin, or an NFT from a
    /// collection worth courting, is a call rather than a redeploy. `amount` is
    /// units for an ERC-20, a token count for an ERC-721 (any ids from the
    /// collection), and a quantity of `id` for an ERC-1155.
    ///
    /// ⚠ Nothing here validates that the asset is worth anything. A rail is the
    /// owner saying "I will take this for a copy", and that judgement is theirs
    /// — the wall this contract does keep is that no rail can change what a
    /// HOLDER owns, only what a BUYER may pay with.
    function setRail(
        uint256 railId,
        AssetKind kind,
        address token,
        uint256 id,
        uint256 amount,
        address sink,
        string calldata label
    ) external onlyOwner {
        require(railId >= FIRST_CUSTOM_RAIL, "rail reserved");
        // One native asset exists and rail 1 is it. A second would make
        // `msg.value` ambiguous and give ETH a redirectable sink, which is
        // exactly the promise `proceeds` is immutable to keep.
        require(kind != AssetKind.NATIVE, "native is rail 1");
        require(token != address(0), "zero token");
        require(sink != address(0), "zero sink");
        // ⚠ A SINK OF `this` IS A ONE-WAY FUND LOSS FOR NFT RAILS, and it is
        // the only way to lose money on this contract by misconfiguration.
        // An ERC-20 that lands here can still be walked out — `setGrant` is
        // repointable and `recoverGrant` moves whatever it names — but there
        // is no ERC-721 or ERC-1155 transfer anywhere in this contract, so a
        // token paid into a self-sinking rail can never move again. The buy
        // uses plain `transferFrom` for 721 precisely so a sink cannot brick
        // the sale, which also means no receiver hook rejects this for us.
        // `setFreeCollection` already refuses `address(this)`; so does this.
        require(sink != address(this), "sink is this contract");
        require(amount > 0, "zero amount - use closeRail");
        if (kind == AssetKind.ERC721) require(amount <= MAX_NFT_PAYMENT, "too many nfts");

        Rail storage r = _rails[railId];
        if (bytes(r.label).length == 0 && r.token == address(0)) _railIds.push(railId);
        r.kind = kind;
        r.token = token;
        r.id = id;
        r.amount = amount;
        r.sink = sink;
        r.open = true;
        r.label = label;
        emit RailPosted(railId, uint8(kind), token, id, amount, sink, true);
    }

    /// Post what a copy grants alongside the licence — the presale half.
    /// `amount` 0 closes the grant and the sale keeps running on licences
    /// alone, which is what a title that is not raising looks like.
    function setGrant(IERC20 token, uint256 amount) external onlyOwner {
        require(amount == 0 || address(token) != address(0), "zero grant token");
        grantToken = token;
        grantAmount = amount;
        emit GrantPosted(address(token), amount);
    }

    /// How many more copies the funded reserve can actually honour. A surface
    /// should read this rather than the balance: it is the number that says
    /// whether the offer on the card is still payable.
    function grantsRemaining() external view returns (uint256) {
        if (grantAmount == 0 || address(grantToken) == address(0)) return 0;
        return grantToken.balanceOf(address(this)) / grantAmount;
    }

    /// Take back unsold grant reserve. Restricted to the grant token on
    /// purpose — ETH has exactly one door out of this contract and it is
    /// `sweep()` to the immutable `proceeds`, which this must not become a way
    /// around.
    function recoverGrant(address to, uint256 amount) external onlyOwner {
        require(to != address(0), "zero to");
        require(address(grantToken) != address(0), "no grant token");
        SafeERC20.safeTransfer(address(grantToken), to, amount, "grant recover failed");
    }

    /// Shut a rail without forgetting it. Works on the reserved three as well
    /// — closing rail 2 is the same act as posting `priceScry` at 0.
    function closeRail(uint256 railId) external onlyOwner {
        Rail storage r = _rails[railId];
        require(r.open, "rail already shut");
        r.open = false;
        r.amount = 0;
        emit RailPosted(railId, uint8(r.kind), r.token, r.id, 0, r.sink, false);
    }

    // ── reading the rails ────────────────────────────────────────────────────
    //
    // The three legacy getters keep their exact selectors, so every surface
    // that reads `priceWei()` / `priceScry()` / `priceUsdg()` — the meter card,
    // the buy box, the MCP store — is unchanged by all of the above. They are
    // views over the registry now rather than their own storage, which is why
    // a rail closed by `closeRail` reads 0 here too.

    function priceWei() external view returns (uint256) {
        return _amountIfOpen(RAIL_ETH);
    }

    function priceScry() external view returns (uint256) {
        return _amountIfOpen(RAIL_SCRY);
    }

    function priceUsdg() external view returns (uint256) {
        return _amountIfOpen(RAIL_USDG);
    }

    function _amountIfOpen(uint256 railId) private view returns (uint256) {
        Rail storage r = _rails[railId];
        return r.open ? r.amount : 0;
    }

    function railCount() external view returns (uint256) {
        return _railIds.length;
    }

    function railIdAt(uint256 i) external view returns (uint256) {
        return _railIds[i];
    }

    /// One rail, flattened for a surface to render without knowing the struct.
    function railInfo(uint256 railId)
        external
        view
        returns (uint8 kind, address token, uint256 id, uint256 amount, address sink, bool open, string memory label)
    {
        Rail storage r = _rails[railId];
        return (uint8(r.kind), r.token, r.id, r.amount, r.sink, r.open, r.label);
    }

    /// Publish (or republish, or close with 0) the free-copy list. The root
    /// is over (wallet, allowance) leaves — a posted cohort anyone can check
    /// a wallet against, never a discretionary faucet.
    function setFreeRoot(bytes32 root) external onlyOwner {
        freeRoot = root;
        emit FreeRootUpdated(root);
    }

    /// The SITE base — `external_url` only, never the metadata source.
    function setBaseURI(string calldata newBase) external onlyOwner {
        baseURI = newBase;
        emit ContractURIUpdated();
        emit BatchMetadataUpdate(1, type(uint256).max);
    }

    /// What a marketplace shows. `image` is any URI a viewer can resolve —
    /// `ipfs://…`, a `data:` document, an https url — so a title can be fully
    /// self-hosted, fully on chain, or pointed at us, and that is the team's
    /// call rather than ours. Both fields are bounded: this text is rebuilt on
    /// every `tokenURI` read, and an unbounded string would make a view a
    /// buyer's wallet cannot render.
    function setMetadata(string calldata newBlurb, string calldata newImage) external onlyOwner {
        require(bytes(newBlurb).length <= 1024, "blurb <= 1024");
        require(bytes(newImage).length <= 2048, "image <= 2048");
        blurb = newBlurb;
        image = newImage;
        emit MetadataUpdated(newBlurb, newImage);
        emit ContractURIUpdated();
        emit BatchMetadataUpdate(1, type(uint256).max);
    }

    /// Swap the renderer. `address(0)` restores the built-in on-chain document,
    /// which is the posted door out of any renderer that goes dark.
    function setRenderer(address newRenderer) external onlyOwner {
        renderer = newRenderer;
        emit RendererUpdated(newRenderer);
        emit ContractURIUpdated();
        emit BatchMetadataUpdate(1, type(uint256).max);
    }

    /// Tell marketplaces to re-read what they already point at. Changes no
    /// state — it emits the two events and nothing else.
    ///
    /// WHY THIS EXISTS, AND IT IS THE HOSTED PATH'S ONE ROUGH EDGE. Under a
    /// `ScryTicketUrlRenderer` the document lives on our origin, so editing the
    /// art or the blurb is a file save: instant on every surface we run, and
    /// **silent to the chain**, because no storage moved and no event fired. A
    /// marketplace holding a cached document has no way to learn. Without this
    /// the poke is `setRenderer(address(this))` — which works, emits both
    /// events, and reads on an explorer like the renderer was replaced, on the
    /// one contract where a reader is trying to decide whether to trust us.
    /// One call that says what it does is worth 24k gas.
    ///
    /// Owner-only, though nothing here is worth stealing: an open version would
    /// let anyone make every indexer re-fetch the collection on demand, which
    /// is a free way to point our own traffic at somebody.
    function refreshMetadata() external onlyOwner {
        emit ContractURIUpdated();
        emit BatchMetadataUpdate(1, type(uint256).max);
    }

    // ── the paid doors ───────────────────────────────────────────────────────
    //
    // Three named functions and one general one, all landing in `_buy`. The
    // named three exist because their SELECTORS are what the buy box, the MCP
    // store and the launcher's 401 path already call; keeping them is what let
    // the rails become modular without a single surface changing.

    /// Buy with ETH at the posted amount, exactly — no refund path to audit.
    function buyWithETH() external payable nonReentrant returns (uint256 tokenId) {
        return _buy(RAIL_ETH, _none());
    }

    /// Buy with SCRY. The coin moves buyer → fee splitter in one hop — this
    /// contract never holds it — and the posted split does the burning. No
    /// surface may say the half has burned until `distribute()` has run.
    function buyWithSCRY() external nonReentrant returns (uint256 tokenId) {
        return _buy(RAIL_SCRY, _none());
    }

    /// Buy with USDG, buyer → proceeds in one hop.
    function buyWithUSDG() external nonReentrant returns (uint256 tokenId) {
        return _buy(RAIL_USDG, _none());
    }

    /// Buy over any open rail. `nftIds` is the buyer's side of an ERC-721 rail
    /// — which tokens of the collection they are handing over — and must be
    /// empty for every other kind.
    function buyWithRail(uint256 railId, uint256[] calldata nftIds)
        external
        payable
        nonReentrant
        returns (uint256 tokenId)
    {
        return _buy(railId, nftIds);
    }

    function _none() private pure returns (uint256[] calldata empty) {
        assembly {
            empty.offset := 0
            empty.length := 0
        }
    }

    function _buy(uint256 railId, uint256[] calldata nftIds) private returns (uint256 tokenId) {
        Rail storage r = _rails[railId];
        require(r.open && r.amount > 0, "rail closed");
        // 4 and 5 are provenance marks, not doors. Belt and braces: neither is
        // ever `open`, so this cannot trigger — it is here so that a future
        // edit which opens one fails loudly rather than selling a "comp".
        require(railId != RAIL_COMP && railId != RAIL_FREE, "not a payment rail");

        if (r.kind == AssetKind.NATIVE) {
            require(msg.value == r.amount, "wrong payment");
            // Stays here. `sweep()` pushes it to the immutable `proceeds`, and
            // that is the one destination no key can redirect.
        } else {
            require(msg.value == 0, "not a native rail");
            if (r.kind == AssetKind.ERC20) {
                SafeERC20.safeTransferFrom(r.token, msg.sender, r.sink, r.amount, "payment failed");
            } else if (r.kind == AssetKind.ERC721) {
                require(nftIds.length == r.amount, "wrong number of nfts");
                for (uint256 i = 0; i < nftIds.length; i++) {
                    // The same id twice would be caught by the second transfer
                    // reverting on ownership — except where the sink IS the
                    // buyer, which an owner can misconfigure. Refuse it here
                    // rather than depend on that.
                    for (uint256 j = i + 1; j < nftIds.length; j++) {
                        require(nftIds[i] != nftIds[j], "duplicate nft");
                    }
                    IERC721Payment(r.token).transferFrom(msg.sender, r.sink, nftIds[i]);
                }
            } else {
                IERC1155Payment(r.token).safeTransferFrom(msg.sender, r.sink, r.id, r.amount, "");
            }
        }

        tokenId = _mint(msg.sender, railId);
        emit CopySold(tokenId, msg.sender, railId, r.amount);
        _grant(tokenId, msg.sender);
    }

    /// The coin half of a paid copy. Comps and free claims do NOT grant — a
    /// gift of a licence is not a gift of somebody's raise, and letting a free
    /// door drain the reserve would let the owner mint the raise away.
    function _grant(uint256 tokenId, address to) private {
        uint256 amount = grantAmount;
        if (amount == 0) return;
        IERC20 token = grantToken;
        require(token.balanceOf(address(this)) >= amount, "grant reserve empty");
        grantDelivered += amount;
        SafeERC20.safeTransfer(address(token), to, amount, "grant transfer failed");
        emit GrantDelivered(tokenId, to, address(token), amount);
    }

    // ── the free doors ───────────────────────────────────────────────────────
    /// The owner hands a copy to a named wallet. On the record by event: a
    /// comp nobody can see is a favor, and favors corrode the ticket.
    function compMint(address to, uint256 n) external onlyOwner {
        for (uint256 i = 0; i < n; i++) {
            emit CompMinted(_mint(to, RAIL_COMP), to);
        }
    }

    /// Claim a free copy against the posted list. One per call, up to the
    /// wallet's allowance in the root; the claim count never resets, so a
    /// republished root cannot re-open a spent wallet by accident.
    function claimFree(uint256 allowance, bytes32[] calldata proof) external nonReentrant returns (uint256 tokenId) {
        require(freeRoot != bytes32(0), "free door closed");
        require(_verify(freeRoot, msg.sender, allowance, proof), "not on the free list");
        require(freeClaimed[msg.sender] < allowance, "already claimed yours");
        freeClaimed[msg.sender] += 1;
        tokenId = _mint(msg.sender, RAIL_FREE);
        emit FreeClaimed(tokenId, msg.sender);
    }

    /// Open (or re-price, or shut with 0) the collection door for one ERC-721.
    /// `perToken` is how many games EACH token id in that collection may take.
    ///
    /// ⚠ Shutting it with 0 stops future claims and forgets nothing: every id
    /// that already claimed stays spent in `freeClaimedByToken`, so reopening
    /// later hands nobody a second one.
    function setFreeCollection(address collection, uint256 perToken) external onlyOwner {
        require(collection != address(0), "zero collection");
        require(collection != address(this), "not this contract");
        if (!_knownFreeCollection[collection]) {
            _knownFreeCollection[collection] = true;
            _freeCollections.push(collection);
        }
        freePerToken[collection] = perToken;
        emit FreeCollectionPosted(collection, perToken);
    }

    /// Claim against tokens you hold RIGHT NOW in a posted collection. No
    /// proof, no snapshot, no list to serve — the collection is the list.
    ///
    /// Ids may repeat in one call where `perToken > 1`; the counter catches it
    /// either way, so a duplicate is just the same claim made twice and needs
    /// no dedup pass. `ownerOf` is a STATICCALL, and a collection that reverts
    /// or lies takes down only its own door.
    function claimFreeFor(address collection, uint256[] calldata heldIds)
        external
        nonReentrant
        returns (uint256 claimed)
    {
        uint256 perToken = freePerToken[collection];
        require(perToken > 0, "collection door closed");
        require(heldIds.length > 0 && heldIds.length <= MAX_FREE_CLAIM, "bad id count");

        for (uint256 i = 0; i < heldIds.length; i++) {
            uint256 heldId = heldIds[i];
            require(IERC721Payment(collection).ownerOf(heldId) == msg.sender, "not yours");
            require(freeClaimedByToken[collection][heldId] < perToken, "already claimed for this one");
            freeClaimedByToken[collection][heldId] += 1;
            uint256 tokenId = _mint(msg.sender, RAIL_FREE);
            emit FreeClaimedFor(tokenId, collection, heldId, msg.sender);
            claimed++;
        }
    }

    /// What a claim button needs, in one read: how many each id still has
    /// coming. Zeros everywhere means the door is shut or they are all spent,
    /// and the caller can tell which by reading `freePerToken` beside it.
    function freeRemainingFor(address collection, uint256[] calldata heldIds)
        external
        view
        returns (uint256[] memory remaining)
    {
        uint256 perToken = freePerToken[collection];
        remaining = new uint256[](heldIds.length);
        for (uint256 i = 0; i < heldIds.length; i++) {
            uint256 taken = freeClaimedByToken[collection][heldIds[i]];
            remaining[i] = perToken > taken ? perToken - taken : 0;
        }
    }

    function freeCollectionCount() external view returns (uint256) {
        return _freeCollections.length;
    }

    function freeCollectionAt(uint256 i) external view returns (address collection, uint256 perToken) {
        collection = _freeCollections[i];
        return (collection, freePerToken[collection]);
    }

    /// Push accumulated ETH to the immutable proceeds address. Anyone may
    /// call; nobody can redirect.
    function sweep() external nonReentrant {
        uint256 amount = address(this).balance;
        require(amount > 0, "nothing to sweep");
        (bool ok,) = proceeds.call{value: amount}("");
        require(ok, "sweep failed");
        emit Swept(amount);
    }

    // ── the one question the depot asks ──────────────────────────────────────
    function entitled(address wallet) external view returns (bool) {
        return _balance[wallet] > 0;
    }

    function _mint(address to, uint256 rail) internal returns (uint256 tokenId) {
        require(to != address(0), "zero to");
        require(supplyCap == 0 || nextTokenId <= supplyCap, "supply cap reached");
        tokenId = nextTokenId++;
        railOf[tokenId] = rail;
        _owner[tokenId] = to;
        _balance[to] += 1;
        emit Transfer(address(0), to, tokenId);
    }

    /// Sorted-pair keccak merkle over (wallet, allowance) — the same shape as
    /// ScryTicket and the harvest drops, so the meter's existing root pipeline
    /// serves this door unchanged. 52-byte leaf preimage vs 64-byte node
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
        return nextTokenId - 1; // no door out: a licence never burns
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
                IGameTicketReceiver(to).onERC721Received(msg.sender, from, tokenId, data)
                    == IGameTicketReceiver.onERC721Received.selector,
                "unsafe recipient"
            );
        }
    }

    // ── ERC-2981: a constant zero, not a knob ────────────────────────────────
    /// The interface answers well-formed so every marketplace reads a clean
    /// zero; the number is a constant so no key can ever tax a resale.
    function royaltyInfo(uint256, uint256) external view returns (address, uint256) {
        return (proceeds, 0);
    }

    function supportsInterface(bytes4 id) external pure returns (bool) {
        return id == 0x01ffc9a7 // ERC-165
            || id == 0x80ac58cd // ERC-721
            || id == 0x5b5e139f // ERC-721 Metadata
            || id == 0x49064906 // ERC-4906 metadata update
            || id == 0x2a55205a; // ERC-2981
    }

    // ── metadata: on chain, so the copy outlives us ──────────────────────────
    //
    // See the header for why this moved off our origin. The shape is
    // `ScryDeed`'s, which shipped the same reasoning first: build the document
    // from storage, base64 it, hand back a `data:` URI. A viewer needs no
    // server, no gateway and no cooperation from scry to render a licence its
    // holder paid for.

    function tokenURI(uint256 tokenId) external view returns (string memory) {
        ownerOf(tokenId); // reverts for a token that does not exist
        if (renderer != address(0)) return IScryTicketRenderer(renderer).tokenURI(tokenId);
        string memory json = string.concat(
            '{"name":"',
            _esc(name),
            " #",
            _u(tokenId),
            '","description":"',
            _esc(_blurb()),
            '"',
            _imageField(),
            _externalField(false),
            ',"attributes":[{"trait_type":"game","value":"',
            _esc(game),
            '"},{"trait_type":"acquired","value":"',
            _acquired(railOf[tokenId]),
            '"}]}'
        );
        return string.concat("data:application/json;base64,", _b64(bytes(json)));
    }

    /// ERC-7572 collection metadata, self-contained for the same reason.
    function contractURI() external view returns (string memory) {
        if (renderer != address(0)) return IScryTicketRenderer(renderer).contractURI();
        string memory json = string.concat(
            '{"name":"',
            _esc(name),
            " on ",
            PLATFORM,
            '","description":"',
            _esc(_blurb()),
            '"',
            _imageField(),
            _externalField(true),
            "}"
        );
        return string.concat("data:application/json;base64,", _b64(bytes(json)));
    }

    /// The posted blurb, or the contract's own notice. Never empty, because a
    /// marketplace showing nothing is how a real licence reads as a broken one.
    function _blurb() internal view returns (string memory) {
        return bytes(blurb).length == 0 ? NOTICE : blurb;
    }

    function _imageField() internal view returns (string memory) {
        if (bytes(image).length == 0) {
            return string.concat(',"image":"data:image/svg+xml;base64,', _b64(bytes(_mark())), '"');
        }
        return string.concat(',"image":"', _esc(image), '"');
    }

    /// ⚠ THE KEY DIFFERS BY DOCUMENT, AND BOTH SPELLINGS ARE CORRECT. A token's
    /// document is ERC-721 metadata, where the field is `external_url`; a
    /// collection's is ERC-7572, where it is `external_link`. This helper is
    /// shared by both, and it used to emit `external_url` unconditionally — so
    /// the collection page's link back to the game was a key no marketplace
    /// reads. It failed the quiet way: an unknown key parses fine and the page
    /// simply renders one field short, with nothing to see on chain or in a
    /// suite that only checked the token. `ScryDeed` and `ScrySeatArt` both
    /// already spell it per document; this is the one that did not.
    /// ⚠ COSTS 529 BYTES OF DEPLOYED BYTECODE AND THAT IS THE CHEAPEST SHAPE
    /// MEASURED. Before this the two documents emitted an IDENTICAL field, so
    /// the optimizer emitted one concat path and shared it; making them differ
    /// necessarily buys a second one. Passing the key as a `string` costs 541,
    /// and sharing a `_gameUrl()` helper while each caller wraps its own key
    /// costs 588. If this contract ever needs the headroom back, the saving is
    /// not here — it is in `NOTICE`.
    function _externalField(bool collection) internal view returns (string memory) {
        if (bytes(baseURI).length == 0) return "";
        return string.concat(
            collection ? ',"external_link":"' : ',"external_url":"',
            _esc(baseURI),
            "/game.html?slug=",
            _esc(game),
            '"'
        );
    }

    /// The provenance mark `railOf` already keeps, said in a word. A gift and
    /// a posted free claim are visible on the token itself — a favour is on the
    /// record or it corrodes the ticket.
    ///
    /// The words are the ones a buyer already knows: "comp" is trade jargon for
    /// a free ticket and reads to everyone else like an abbreviation nobody
    /// explained. What the field means has not moved a bit — it is still
    /// `railOf`, still derived, still unforgeable.
    function _acquired(uint256 rail) internal pure returns (string memory) {
        if (rail == RAIL_COMP) return "gift";
        if (rail == RAIL_FREE) return "claimed";
        return "bought";
    }

    /// The fallback mark. Deliberately small: it exists so nothing renders
    /// blank, not to be the art. A title posts its own with `setMetadata`.
    function _mark() internal view returns (string memory) {
        return string.concat(
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 350 350">'
            '<rect width="350" height="350" fill="#0b0f0d"/>'
            '<circle cx="175" cy="150" r="54" fill="none" stroke="#4ade80" stroke-width="4"/>'
            '<text x="175" y="252" font-family="monospace" font-size="20" fill="#4ade80" text-anchor="middle">',
            _esc(game),
            "</text></svg>"
        );
    }

    /// JSON string escaping. `blurb`, `image` and `game` are owner-set text and
    /// a bare quote in any of them would produce a document no wallet can
    /// parse — a metadata read that fails silently is exactly the class of bug
    /// this whole move is fixing. Quote, backslash and control bytes only;
    /// UTF-8 passes through untouched, which is what a JSON parser wants.
    function _esc(string memory s) internal pure returns (string memory) {
        bytes memory b = bytes(s);
        uint256 extra;
        for (uint256 i; i < b.length; ++i) {
            uint8 c = uint8(b[i]);
            if (c == 0x22 || c == 0x5c) extra += 1;
            else if (c < 0x20) extra += 5;
        }
        if (extra == 0) return s;
        bytes memory out = new bytes(b.length + extra);
        uint256 j;
        for (uint256 i; i < b.length; ++i) {
            uint8 c = uint8(b[i]);
            if (c == 0x22 || c == 0x5c) {
                out[j++] = 0x5c;
                out[j++] = b[i];
            } else if (c < 0x20) {
                out[j++] = 0x5c;
                out[j++] = "u";
                out[j++] = "0";
                out[j++] = "0";
                out[j++] = _hex(c >> 4);
                out[j++] = _hex(c & 0x0f);
            } else {
                out[j++] = b[i];
            }
        }
        return string(out);
    }

    function _hex(uint8 n) private pure returns (bytes1) {
        return bytes1(n < 10 ? 48 + n : 87 + n);
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
