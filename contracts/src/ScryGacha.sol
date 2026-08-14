// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "./ReentrancyGuard.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {SCRY_BURN} from "./ScryFeeSplitter.sol";

/// @notice The one balance read the toll rail needs. Movement goes through
///         `SafeERC20`, which takes a bare address, so nothing else is declared.
interface IERC20Balance {
    function balanceOf(address who) external view returns (uint256);
}

/// @notice the ERC-721 surface this contract needs. Deliberately minimal: the
///         pool never mints, never approves, and never asks a collection for
///         metadata. `transferFrom` (not `safeTransferFrom`) is used in both
///         directions — see `_pullNft` for why the missing receiver hook is the
///         point rather than an oversight.
interface IERC721Minimal {
    function transferFrom(address from, address to, uint256 tokenId) external;
    function ownerOf(uint256 tokenId) external view returns (address);
}

/// @notice The two ArbOS precompile calls this contract's clock is built on.
///         `arbBlockNumber` is the real L2 height (Solidity's `block.number` on
///         Nitro is the parent chain's) and `arbBlockHash` is the exact L2 block
///         hash for a block inside the node's window — it REVERTS outside it,
///         which is the signal `settle`/`expire` split on. See the
///         `ARBSYS` constant for the measurements behind both statements.
interface IArbSys {
    function arbBlockNumber() external view returns (uint256);
    function arbBlockHash(uint256 arbBlockNum) external view returns (bytes32);
}

/// @title ScryGacha — a floor under someone else's NFT, and a draw priced off it
/// @notice A depositor puts up an ERC-721 **and** ETH backing. The backing does
///         two jobs at once: it is the price the depositor claims the token is
///         worth, and it funds a standing bid to buy it back. A buyer pays a
///         formula fee to draw one random position, then either keeps the token
///         or hands it back for `bidBps` of that position's backing.
///
///         On Ethereum this mechanic is NFT market-making. **On RH-Chain, where
///         the median NFT sale measured 2026-07-26 was $3.24 and the p80 was
///         $19, the same mechanic is a floor-price factory** — the backing *is*
///         the price, and a chain with 1,500+ collections of 2,600+ holders each
///         has no permissionless way to post one. That inversion is the product.
///
///         **Forked from FWA** (`Fake World Assets`, TokenWorks/Rhynotic,
///         Ethereum mainnet `0xB276F62DB0ce8CA2Ca5bc522695bE604521eAc1c`, MIT).
///         Their source is fetched for reading with `contracts/fetch_fwa.py`
///         (the script is tracked, its output under `contracts/reference/` is
///         deliberately NOT — the Uniswap v4-core files that arrive inside their
///         verified sources are BUSL-1.1 and must never enter this repo). The
///         port table lives in the README that fetch writes. The arithmetic below
///         is theirs and it is right:
///
///         - **Selection weight is INVERSE to backing** (`weight = NUM /
///           backing`), so a cheap position is drawn often and a rich one
///           rarely.
///         - **The price is the pool's expected value**, `Σ(wᵢ·bᵢ) / Σwᵢ`, which
///           with inverse weights *is the harmonic mean of the backings*, plus a
///           posted surcharge. A pool of richer positions prices cheaper,
///           because a draw is far likelier to land a cheap one.
///         - **Fees split EQUALLY per active position**, and that is not a
///           rounding of convenience — it is the only fair split. Every
///           position's expected exposure per draw is `pᵢ·bᵢ = 1/Σ(1/b)`, the
///           *same number for every position*, precisely because the weight is
///           inverse. Equal shares are therefore exactly proportional to
///           expected loss. (FWA reached this by deleting an earlier √backing
///           share; their header says so.)
///         - **Backing is escrowed per position and only ever pays its own
///           standing bid.** There is no mutual pot, so the contract is always
///           solvent and an undrawn position is always fully withdrawable.
///
///         Four things are ours, and each is a measurement or an invariant:
///
///         1. **The draw is a commit-reveal, not a VRF.** Chainlink VRF 2.5 is
///            not on chain 4663. A request fixes a public
///            buyer salt and a reveal block `drawDelay` L2 blocks ahead; the seed
///            is `keccak(arbBlockHash(revealBlock), salt, drawId, collection,
///            buyer)`. At request time nobody knows that hash — not the buyer,
///            not the house. **Read the `ARBSYS` constant before touching any of
///            this**: the clock is the ArbOS precompile, because Solidity's
///            `block.number` on Nitro is the parent chain's and using it made a
///            "10 second" reveal into twenty minutes.
///         2. **One draw per pool at a time.** FWA's request sequencer, staged
///            deposit FIFO, reserved activation batches, word deadlines and
///            drift refunds all exist to make an *asynchronous oracle* safe. At
///            101ms L2 blocks a 100-block reveal is **~10 seconds**, so
///            serializing gives ~8,600 draws/day/pool against FWA's measured
///            ~1,400/day across their whole pool — and deletes every one of those
///            organs, with all their edge cases. Deposits, withdrawals and
///            re-prices are refused for those ten seconds, which is what makes
///            the drawn set immutable without a staging queue. **This argument is
///            load-bearing and it is only true on the L2 clock** — on the parent
///            chain's it would be ~72 draws/day and the serialization would be
///            indefensible.
///         3. **An expired draw forfeits the fee to the depositors — it is
///            never refunded.** This is the port's sharpest edit and skipping it
///            would have been a free option. Nobody can influence whether
///            Chainlink delivers, so FWA can safely refund an expired request.
///            But a commit-reveal draw is settled by a *transaction*, and `settle`
///            is permissionless — so a buyer who could walk away from an
///            unsettled draw would be buying an option: read the outcome at the
///            reveal block, settle the good ones, abandon the bad. Forfeiting
///            the whole fee makes settling weakly better than walking **for
///            every possible outcome** — the worst settlement leaves the buyer
///            `bidBps·minBacking - fee`, which is strictly greater than losing
///            the fee outright. The buyer's protection is that settling is
///            cheap, permissionless, and theirs to call.
///         4. **One pool per collection, never one pool for all of them.** FWA
///            runs a single global pool and leans on curation to keep dust out.
///            We take the seasons answer instead — pools are funded one at a
///            time, never as a shared pot — so a worthless collection can
///            only drag its own price, and every collection gets its own floor.
///
///         **The minimum backing is the design, not a knob.**
///         The harmonic mean is dominated by the cheapest position, so on a dust
///         chain one $3 deposit would drag a pool's price to $3 while the gas to
///         draw stays fixed — a griefing vector that costs cents. 0.01 ETH sat
///         at p82 of live sales when it was chosen and at **p80** when this
///         contract was written; it is also FWA's own default, arrived at
///         independently. A maximum backing ratio was considered and **measured
///         off**: the qualifying shelf prices at 0.0292 ETH harmonic and its top
///         is 114× that, so any cap that binds deletes the jackpot the draw
///         exists for — and a whale is not a threat, because its standing bid is
///         funded by its own backing and its draw weight is proportionally tiny.
///
///         **Score-blind, and the wall is not near this contract.** No meter
///         reading, vow, breach flag or reputation touches a price, a weight, an
///         odds, a fee or a payout here; the only inputs are backing, count, the
///         posted bps and an L2 block hash. Money moves freely across every
///         of it and none of it can reach a measurement, so no caveat belongs on
///         this organ (the wall is `CLAUDE.md` invariant 1, and the storefront
///         says it once, on `about.html`).
///
///         **The residual trust assumption, stated.** A chain's sequencer picks
///         the contents of the reveal block, so a sequencing party that also
///         knew a buyer's salt could grind that block toward a chosen outcome.
///         On 4663 the sequencer is not a participant here and a draw is worth
///         tens of dollars, so the exposure is priced accordingly — but it is
///         real, and it is why `DRAW_DELAY` is immutable rather than a knob a
///         later operator could shrink to one block. The upgrade, if it is ever
///         worth its cost, is to XOR in a house seed committed before the
///         request and revealed unconditionally after it (`ScryNotary` already
///         carries exactly that beacon, `anchor_worker.py`) — which buys
///         grind-resistance at the price of depending on a live worker for
///         every draw, and that trade is not worth making at this size.
///
///         **Custody is untouched.** This contract holds NFTs its depositors
///         chose to escrow and ETH they sent; it holds no key for anyone and
///         mints nothing. The house's only claim is a posted bps of fees, swept
///         to a posted sink by a permissionless call.
///
///         ═══ THE TOLL RAIL ═══
///
///         **The pull fee may be charged in `SCRY` instead of ETH** (decided
///         2026-07-27, `SENTENCES.md`). Two things about it are worth reading
///         before touching any of the `toll*` state:
///
///         **1. It is a second rail, not a conversion.** Nothing here swaps.
///         The buyer acquires the toll coin themselves, through a pool the
///         house LPs, at their own cost and their own slippage — which is the
///         whole point: it manufactures the swap volume the LP fee is paid on
///         without the house ever performing a market operation
///         (`TOKENOMICS.md` holds the house-side buyback on depth grounds, and
///         this design never needs one). So the fee rail runs twice over: an ETH
///         accumulator and a toll accumulator, each with its own escrow, carry,
///         top pot, house balance and per-position checkpoint. A pool that ran
///         in one era and then the other keeps both, and every position can
///         collect both.
///
///         **2. The BACKING and the STANDING BID never move off ETH, and that
///         is not a compromise.** A backing is a price claim about someone
///         else's real NFT, so it wants the most external unit on the chain; a
///         floor denominated in a coin whose price comes from a pool we seeded
///         is a weaker floor. The toll is a different object — a fee for a
///         service the house runs. **The consequence must be said out loud on
///         every surface that quotes a price: in the toll era a buyer pays
///         `SCRY` and a standing bid pays them back ETH, so "instant profit"
///         and "break-even" are cross-asset and depend on the market rate, not
///         on the posted one.** `tollRate` is a posted rate, not an oracle: it
///         drifts from market and the operator retunes it, exactly like the
///         barrow's and the agora's numbers.
///
///         **What the two knobs do, and why they are two.** `tollCoin` is the
///         rail's denomination and may only change while the rail is EMPTY
///         (`accountedToll() == 0`) — otherwise a switch would pay outstanding
///         credits in a coin nobody earned. `tollRate` arms and disarms: zero
///         means the ETH rail, and setting it to zero is instant, always legal,
///         and strands nothing, because credits already earned stay claimable in
///         `tollCoin`. That split is what makes "stop selling in `SCRY` now" a
///         different act from "change which coin the ledger is denominated in".
///
///         **An exit never depends on the token contract.** `claim` (ETH) and
///         `claimToll` are separate calls, and `withdraw`/`reprice`/`harvest`
///         push only the ETH leg. A paused, blacklisting or self-destructed toll
///         coin must never be able to hold a depositor's backing or their token
///         hostage — the same rule `_accrue`'s saturating counter keeps.
///
///         **The burn is accrued, never sent from a settlement path.** A slice
///         of every toll (`tollBurnBps`) is set aside and pushed to `0xdEaD` by
///         a permissionless `sweepBurn`. Doing the transfer inside `settle`
///         would put a third-party token call on the one path that must never
///         be brickable — an unsettled draw holds `pendingDraw` and stops the
///         pool selling. ⚠ **Never write "supply reduced":** the coins land at
///         a dead address and become unspendable, but `totalSupply()` does not
///         move. `totalBurned` is what actually left, counted when it left.
contract ScryGacha is ReentrancyGuard {
    using SafeERC20 for address;

    string public constant NOTICE =
        "randomized NFT pool - the backing is the floor; posted odds, published EV, no reading touches any of it";

    /*//////////////////////////////////////////////////////////////
                                 TYPES
    //////////////////////////////////////////////////////////////*/

    enum PositionStatus {
        None,
        Active, // in the pool, drawable, earning fees
        Drawn, // allocated to a buyer, awaiting resolution
        Withdrawn, // taken back by its depositor
        Settled // resolved after a draw
    }

    enum DrawStatus {
        None,
        Open, // fee escrowed, waiting for the reveal block
        Settled,
        Expired // nobody settled inside the reveal window; fee forfeited
    }

    struct Position {
        address collection;
        address depositor;
        address buyer; // zero until drawn
        uint256 tokenId;
        uint256 backing; // escrowed ETH; funds ONLY this position's standing bid
        uint256 weight; // NUM / backing — inverse, so cheap is drawn often
        uint256 slot; // segment-tree leaf while active; 0 otherwise
        uint256 feeDebt; // dividend checkpoint, ETH rail
        uint64 drawnAt; // timestamp of the draw; starts the choice window
        // ⚠ The resolution terms are SNAPSHOTTED here at settlement, never read
        // live from the knobs (review, 2026-07-27). `bidBps` was already frozen at
        // pool open for this reason; these were the hole in the same promise —
        // `choiceWindow`/`finalizeWindow`/`houseKeepBps` were global mutable
        // storage read at resolution time, so an owner could collapse a buyer's
        // exclusive window to zero AFTER the buyer paid for it and let the
        // depositor take the backing and hand back a worthless token. A knob may
        // change what the NEXT draw costs; it may never re-price a draw that has
        // already happened.
        uint64 chooseBy; // the buyer's exclusive window ends here
        uint64 finalizeBy; // after this, anyone may finalize the default
        uint16 keepBps; // the house's cut on an NFT outcome, as posted at the draw
        PositionStatus status;
        // APPENDED, and that is deliberate. The meter and the page decode
        // `positions(uint256)` positionally by word index; a new field in the
        // middle would silently shift every one of them onto the wrong number.
        // New state goes on the end here, always.
        uint256 tollDebt; // dividend checkpoint, toll rail (see THE TOLL RAIL)
    }

    struct Draw {
        address collection;
        address buyer;
        uint256 fee; // ETH escrowed until settled or forfeited; 0 on the toll rail
        uint256 positionId; // written at settlement
        uint256 autoBidAbove; // 0 = the buyer chooses later; else resolve atomically
        uint64 revealBlock;
        bytes32 salt; // public at request time, and that is safe (see header)
        DrawStatus status;
        uint256 toll; // toll coin escrowed, same rules; 0 on the ETH rail. APPENDED — see Position
    }

    /// @dev One fee rail. A pool carries two of them — ETH and toll — and every
    ///      routing function takes one plus a flag, rather than existing twice.
    ///      Written as a struct because the alternative (eight parallel fields
    ///      with `Toll` suffixes) is how the two halves drift apart: a fix
    ///      applied to `_distribute` and forgotten in `_distributeToll` is
    ///      invisible until a depositor is short. Same reason the pool's own
    ///      mappings live inside `Pool`.
    struct Rail {
        uint256 acc; // dividend accumulator, scaled by ACC
        uint256 carry; // truncation carry; see _distribute
        uint256 topPot; // accrued for the CURRENT top holder
        uint256 volume; // lifetime paid in on this rail
    }

    /// @dev One pool per collection. The mappings live inside the struct so a
    ///      pool is a self-contained namespace — there is no way to write
    ///      collection A's tree while holding collection B's pool, which is the
    ///      "never a shared pot" rule made structural instead of remembered.
    struct Pool {
        bool exists;
        bool open; // false: no new deposits or draws; exits still work
        uint256 minBacking; // >= MIN_BACKING_FLOOR
        uint256 surchargeBps; // the house/depositor edge over EV
        uint256 bidBps; // standing bid as a share of backing; FIXED at open
        uint256 count; // active positions
        uint256 totalWeight; // Σ wᵢ
        uint256 weightedBacking; // Σ wᵢ·bᵢ — the EV numerator
        Rail eth; // the native rail — backing, and the fee until the toll is armed
        Rail toll; // the toll rail; see THE TOLL RAIL in the header
        uint256 pendingDraw; // open draw id, or 0
        uint256 draws; // lifetime settled draws
        uint256 topPositionId; // the top-backed position, or 0
        // The bar the seat was last held at. A vacant seat is NOT free: without
        // this, a position could wait for the top to be drawn out and take the
        // size incentive for the minimum backing, which made `topPositionId`
        // stop meaning "the top-backed position" (review, 2026-07-27). Never
        // lowered, so the bar only ratchets up — and an empty seat costs nothing,
        // because `_spread` folds the slice back into the equal split.
        uint256 topBacking;
        uint256 nextUnusedSlot;
        uint256 freeSlotHead;
        mapping(uint256 => uint256) tree; // sparse segment tree over slots
        mapping(uint256 => uint256) slotToPosition;
        mapping(uint256 => uint256) nextFreeSlot;
    }

    /*//////////////////////////////////////////////////////////////
                               CONSTANTS
    //////////////////////////////////////////////////////////////*/

    uint256 public constant BPS = 10_000;

    /// @notice Numerator of the inverse weight. Shared, so it cancels in every
    ///         relative comparison and only sets absolute precision: at the
    ///         0.01 ETH floor a weight is ~1e20, so the truncation in
    ///         `NUM / backing` is ~1e-20 of the weight.
    uint256 internal constant NUM = 1e36;

    /// @notice Scale for the fee dividend accumulator. Paired with `Rail.carry`
    ///         so no remainder is ever dropped.
    uint256 internal constant ACC = 1e36;

    /// @notice 2^16 concurrent positions per collection. Slots are recycled
    ///         through a free list, so this bounds *concurrent* positions, not
    ///         lifetime ones. The depth is the gas trade, deliberately small:
    ///         every insert and removal writes one node per level (~17 SSTOREs),
    ///         and §4.3 measured that a pull already costs a third of the median
    ///         object. A collection that ever needs more gets its own contract.
    uint256 internal constant TREE_DEPTH = 16;
    uint256 internal constant CAPACITY = 1 << TREE_DEPTH;
    uint256 internal constant LEAF_BASE = CAPACITY;

    /// @notice The floor under every pool's floor. A pool may set its minimum
    ///         higher, never lower — 0.01 ETH is ~p80 of live sales and ~40x the
    ///         gas of a pull, and below it the harmonic mean is a free griefing
    ///         vector.
    uint256 public constant MIN_BACKING_FLOOR = 0.01 ether;

    /// @notice Posted bounds on the standing bid. A bid of 100% would let a
    ///         buyer round-trip a rich position for free; a bid that low is not
    ///         a bid at all.
    uint256 internal constant MIN_BID_BPS = 5_000;
    uint256 internal constant MAX_BID_BPS = 9_500;

    /// @notice Ceilings on the knobs an operator can move after depositors have
    ///         committed. The surcharge is what a buyer pays over EV — they see
    ///         it and cap it with `maxFee` — and the house cuts are hard-capped
    ///         at 5% each.
    ///
    ///         ⚠ **Be precise about what that does and does not bound**, because
    ///         an earlier version of this comment claimed it meant "no later
    ///         operator can turn the pool into a rake" and that overstated it
    ///         (review, 2026-07-27). `MAX_HOUSE_BPS` bounds the two DIRECT cuts
    ///         only. The reachable house take is larger: up to `MAX_TOP_SHARE_BPS`
    ///         of every fee can be routed to the top slot (and an owner holding
    ///         that slot collects it), and on a bid settlement `retainedToHouse`
    ///         sends the whole `BPS - bidBps` slice — as much as 50% of that
    ///         position's backing — to the house, outside these caps entirely.
    ///         Every one of those is disclosed and posted before a depositor
    ///         commits; none of them is capped at 5%.
    uint256 internal constant MAX_SURCHARGE_BPS = 5_000;
    uint256 internal constant MAX_HOUSE_BPS = 500;
    uint256 internal constant MAX_TOP_SHARE_BPS = 2_000;

    /// @notice Ceiling on the toll burn. The burn is a real tax on the
    ///         depositors' side of the split, not on the house's, so it is
    ///         posted and capped like the top-slot slice rather than like a
    ///         house cut. The toll safety inequality — `rebateBps <
    ///         burnBps + houseDrawBps` — needs this number readable as a
    ///         constant, which is why the burn lives here and not only
    ///         downstream in `ScryFeeSplitter`.
    uint256 internal constant MAX_TOLL_BURN_BPS = 2_000;

    /// @notice Denominator of `tollRate`: the rate is toll-coin wei charged per
    ///         **1e18 wei (one ETH) of the ETH-quoted fee**, so it is a posted
    ///         exchange rate and nothing else. `toll()` applies it to the whole
    ///         quoted fee — EV *times* the surcharge — so the pool's edge rides
    ///         across the change of unit instead of being quietly dropped.
    uint256 internal constant RATE_ONE = 1e18;

    /// @notice Where a burned toll lands. Imported from `ScryFeeSplitter` rather
    ///         than retyped, so the town has exactly one burn address and the two
    ///         can never drift apart.
    address public constant BURN = SCRY_BURN;

    /// @notice **The clock is `ArbSys`, not `block.number`, and that is not a
    ///         preference — it is a correction.** Chain 4663 is Arbitrum Nitro
    ///         (measured: `ArbSys`/`ArbGasInfo`/`ArbOwnerPublic`/`ArbRetryableTx`
    ///         all carry code, no OP predeploys, `arbChainID()` = 4663). On Nitro
    ///         **Solidity's `block.number` tracks the PARENT chain**: measured on
    ///         the same day it read 25,620,709 while the L2 height was 20,319,770,
    ///         and it advanced **0.08 blocks/s** — one per ~12s — against the L2's
    ///         9.9/s. An earlier version of this contract used `block.number` and
    ///         `blockhash`, which silently made a "100 block, ~10 second" reveal
    ///         into **~20 minutes**, locked the pool for that whole time, and cut
    ///         throughput from ~8,640 draws/day/pool to ~72.
    ///
    ///         `ArbSys.arbBlockNumber()` is the real L2 height and
    ///         `arbBlockHash(n)` returns the **exact** L2 block hash (verified
    ///         against `eth_getBlockByHash`: exact matches at head-1, -2, -100).
    ///         It is also the right primitive on its own terms — Arbitrum
    ///         documents the `blockhash` opcode as unfit for randomness, while
    ///         `arbBlockHash` is a real block hash a player can look up on the
    ///         explorer, which is what makes the receipt in `DrawSettled`
    ///         checkable by hand.
    IArbSys internal constant ARBSYS = IArbSys(0x0000000000000000000000000000000000000064);

    /// @notice How far back `arbBlockHash` will answer. Documented as 256 and
    ///         measured at the 253-254 boundary — the difference is the ~10
    ///         blocks/s of drift between reading a head and the call landing.
    ///         Kept only for the card and the UI: the settle/expire branches do
    ///         NOT compare against it, because `arbBlockHash` REVERTS past its
    ///         window rather than returning zero, so the two paths are split on
    ///         the call's own success and are exactly complementary at any
    ///         boundary the chain chooses. ~26 seconds at 101ms blocks.
    uint256 public constant REVEAL_WINDOW_BLOCKS = 256;

    /// @notice L2 blocks between the request and the reveal. Immutable on
    ///         purpose: it is the whole grind-resistance of the draw, and a knob
    ///         here is a knob on the fairness of the game (see the header's
    ///         residual trust note). At 101ms blocks the default 100 is ~10.1s.
    uint256 public immutable drawDelay;

    /*//////////////////////////////////////////////////////////////
                                STORAGE
    //////////////////////////////////////////////////////////////*/

    address public owner;

    /// @notice May open and close pools, but may never touch money. This is the
    ///         curation seat, and curation is the product surface: a
    ///         separate contract can own "which collections get a floor"
    ///         without owning the fees or the escrow.
    address public curator;

    /// @notice Where the house's share is swept. Anyone may push it there.
    address public feeSink;

    /// @notice House cut of a draw fee, in bps of the fee (before the split).
    uint256 public houseDrawBps = 100; // 1%

    /// @notice House cut of the backing on an NFT outcome, in bps. Charged only
    ///         when the buyer keeps the token; on a bid settlement the retained
    ///         `1 - bidBps` slice is the whole house/depositor take instead.
    uint256 public houseKeepBps = 100; // 1%

    /// @notice Where the retained slice on a bid settlement goes. True: the
    ///         house. False: split across the pool's active depositors.
    bool public retainedToHouse = true;

    /// @notice Slice of each draw fee reserved for the pool's top-backed
    ///         position, carved before the equal split. The size incentive that
    ///         equal fee sharing otherwise removes.
    uint256 public topShareBps = 500; // 5%

    /// @notice How far a challenger must clear the standing top to seize it.
    uint256 public topThresholdBps = 1_000; // 10%

    /// @notice The buyer's exclusive window to choose keep-or-bid. Shorter than
    ///         FWA's 24h because the choice IS a free option on the token's
    ///         price and this chain settles in milliseconds; a buyer who wants
    ///         no option can pre-commit `autoBidAbove` at request time.
    uint256 public choiceWindow = 1 hours;

    /// @notice After this, anyone may finalize the default (token to the buyer,
    ///         backing to the depositor) so nothing locks if both walk away.
    uint256 public finalizeWindow = 24 hours;

    /// @notice The toll rail's denomination — `SCRY` when armed, zero for the
    ///         ETH rail. Changeable ONLY while the rail is empty, because
    ///         credits already earned are denominated in whatever this says.
    address public tollCoin;

    /// @notice Toll-coin wei per `RATE_ONE` of the ETH-quoted fee. **This is the
    ///         arm/disarm knob**: zero charges the pull in ETH exactly as the
    ///         contract shipped. A posted number, retuned by the operator — the
    ///         town posts prices, it does not read oracles for them, and a
    ///         posted rate cannot be manipulated by moving a thin pool.
    uint256 public tollRate;

    /// @notice Slice of every toll set aside for the burn, in bps of the toll.
    ///         Off by default. Carved beside the house cut and before the
    ///         split, so the toll safety inequality reads off two constants.
    uint256 public tollBurnBps;

    mapping(address => Pool) internal pools;

    /// @notice Sticky. A blocked collection can never be opened again — an
    ///         operator removing a malicious collection must not be undoable.
    mapping(address => bool) public blockedCollection;

    uint256 public positionCount;
    mapping(uint256 => Position) public positions;

    /// @notice collection => tokenId => the position currently holding it.
    ///         Also the guard on `rescueStray`: a token this maps is never loose.
    mapping(address => mapping(uint256 => uint256)) public positionOfToken;

    uint256 public drawCount;
    mapping(uint256 => Draw) public draws;

    /// @notice Pull-based ETH. Nothing in a settlement path calls out to a
    ///         participant, so no counterparty can revert a draw, jam a pool, or
    ///         reenter one. Every payout lands here and is collected by `claim`.
    mapping(address => uint256) public owed;

    /// @notice The same, on the toll rail. Deliberately a SECOND map and a
    ///         second claim: an exit must never be able to fail because a
    ///         third-party token reverted (see THE TOLL RAIL in the header).
    mapping(address => uint256) public owedToll;

    /// @notice A resolved position whose token would not transfer (a paused or
    ///         non-standard collection). The ETH legs settled; the recipient
    ///         pulls the token whenever the collection recovers.
    mapping(uint256 => address) public stuckNftRecipient;

    // -- solvency counters: every wei in the contract belongs to one of these.
    // `unaccountedEth()` is the difference, and it is a donation, never a
    // liability. Kept as running sums so the invariant suite can assert
    // solvency directly instead of reasoning about it.
    uint256 public backingTotal; // escrowed backing, active + drawn
    uint256 public escrowTotal; // draw fees held pending settlement
    uint256 public owedTotal; // credited, unclaimed
    uint256 public houseOwed; // the house's swept-but-unpaid share
    uint256 public topPotTotal; // Σ pool.topPot
    uint256 public feeCreditsOutstanding; // distributed into accumulators, unrealised

    // -- the same six counters for the toll rail, so `accountedToll()` is the
    // exact twin of `accountedEth()` and the invariant suite can assert
    // `balanceOf(this) >= accountedToll()` the same way it asserts solvency in
    // ETH. There is no `backingTotal` twin on purpose: backing is ETH, always.
    uint256 internal escrowTollTotal; // tolls held pending settlement
    uint256 internal owedTollTotal; // credited, unclaimed
    uint256 internal houseTollOwed; // the house's share, unswept
    uint256 internal topPotTollTotal; // Σ pool.toll.topPot
    uint256 internal tollCreditsOutstanding; // distributed into accumulators, unrealised
    uint256 internal burnOwed; // set aside for the burn, not yet sent to BURN

    /// @notice Toll actually sent to `BURN`, counted when it left — never when
    ///         it was set aside. Only ever increases. ⚠ This is not a supply
    ///         reduction; `totalSupply()` does not move.
    uint256 public totalBurned;

    /*//////////////////////////////////////////////////////////////
                                EVENTS
    //////////////////////////////////////////////////////////////*/

    event PoolOpened(address indexed collection, uint256 minBacking, uint256 surchargeBps, uint256 bidBps);
    event PoolClosed(address indexed collection, bool open);
    event CollectionBlocked(address indexed collection);
    event PoolLimitsSet(address indexed collection, uint256 minBacking, uint256 surchargeBps);

    event Deposited(
        uint256 indexed positionId,
        address indexed collection,
        address indexed depositor,
        uint256 tokenId,
        uint256 backing,
        uint256 weight,
        uint256 slot
    );
    event Repriced(uint256 indexed positionId, uint256 oldBacking, uint256 newBacking, uint256 newWeight);
    event WithdrawnPosition(uint256 indexed positionId, address indexed depositor, uint256 backing);

    event DrawRequested(
        uint256 indexed drawId,
        address indexed collection,
        address indexed buyer,
        uint256 fee,
        uint256 revealBlock,
        uint256 count,
        uint256 harmonicMean
    );
    event DrawSettled(
        uint256 indexed drawId,
        uint256 indexed positionId,
        address indexed buyer,
        uint256 fee,
        uint256 backing,
        bytes32 seed
    );
    /// @dev The fee is distributed to the depositors exactly as a settlement
    ///      would have. Nobody is refunded — see the header, point 3.
    event DrawForfeited(uint256 indexed drawId, address indexed buyer, uint256 fee);

    event NftKept(uint256 indexed positionId, address indexed buyer, address indexed depositor, uint256 toDepositor);
    event BidTaken(
        uint256 indexed positionId, address indexed buyer, address indexed depositor, uint256 payout, uint256 retained
    );
    event Finalized(uint256 indexed positionId, address indexed by);
    event NftDeliveryFailed(uint256 indexed positionId, address indexed to, address collection, uint256 tokenId);
    event StuckNftRecovered(uint256 indexed positionId, address indexed to);
    event StrayRescued(address indexed collection, uint256 tokenId, address indexed to);

    event FeesDistributed(address indexed collection, uint256 amount, uint256 perPosition, uint256 count);
    event EarningsAccrued(address indexed depositor, uint256 indexed positionId, uint256 amount);
    event Claimed(address indexed who, uint256 amount);
    event HouseAccrued(uint256 amount);
    event HouseSwept(address indexed to, uint256 amount);

    // -- the toll rail. Separate events rather than a currency field, so an
    // indexer that only ever knew about the ETH rail cannot mistake a toll
    // number for an ETH one by reading a topic it already understood.
    event TollSet(address indexed coin, uint256 rate);
    event TollPaid(uint256 indexed drawId, address indexed buyer, address indexed coin, uint256 amount);
    event TollForfeited(uint256 indexed drawId, address indexed buyer, address indexed coin, uint256 amount);
    event TollDistributed(address indexed collection, uint256 amount, uint256 perPosition, uint256 count);
    event TollAccrued(address indexed depositor, uint256 indexed positionId, uint256 amount);
    event TollClaimed(address indexed who, uint256 amount);
    event HouseTollAccrued(uint256 amount);
    event HouseTollSwept(address indexed to, uint256 amount);
    event TollBurnAccrued(uint256 amount);
    event Burned(uint256 amount, uint256 totalBurned);
    event TopFundedToll(address indexed collection, uint256 indexed positionId, uint256 amount, uint256 pot);
    event TopSettledToll(
        address indexed collection, uint256 indexed positionId, address indexed depositor, uint256 amount
    );

    event TopSet(address indexed collection, uint256 indexed positionId, address indexed depositor);
    event TopFunded(address indexed collection, uint256 indexed positionId, uint256 amount, uint256 pot);
    event TopSettled(address indexed collection, uint256 indexed positionId, address indexed depositor, uint256 amount);

    event KnobSet(string what, uint256 value);
    event OwnerTransferred(address indexed from, address indexed to);
    event CuratorSet(address indexed curator);
    event FeeSinkSet(address indexed sink);

    /*//////////////////////////////////////////////////////////////
                                ERRORS
    //////////////////////////////////////////////////////////////*/

    /// @dev Named errors rather than strings, and the reason is bytecode: with
    ///      the toll rail in, the string form put this contract 366 bytes OVER
    ///      the EIP-170 24,576-byte runtime limit — undeployable, with a green
    ///      suite, because `forge test` never asks how big the thing is. The
    ///      names below say exactly what the strings said; only the encoding
    ///      changed. ⚠ **A wallet shows a selector, not a sentence**, so any
    ///      surface that surfaced a revert reason to a person must decode these
    ///      — `watchtower/js/gacha.js` derives them from the signatures the same
    ///      way it derives function selectors.
    error BackingTooHigh(); // "backing too high"
    error BelowFloor(); // "below the floor"
    error BelowMinBacking(); // "below min backing"
    error BidOutOfRange(); // "bid out of range"
    error BurnTooHigh(); // "burn too high"
    error BuyersWindow(); // "buyer's window"
    error ChoiceExceedsFinalize(); // "choice > finalize"
    error ChoiceWindowTooShort(); // "choice window too short"
    error CollectionIsBlocked(); // "collection blocked"
    error DelayOutOfRange(); // "delay 2..128"
    error DrawNotOpen(); // "draw not open"
    error DrawPending(); // "draw pending"
    error EthSendFailed(); // "eth send failed"
    error FeeAboveCap(); // "fee above your cap"
    error FinalizeTooLong(); // "finalize too long"
    error HouseCutTooHigh(); // "house cut too high"
    error NftTransferFailed(); // "nft transfer failed"
    error NoPool(); // "no pool"
    error NoPositions(); // "no positions"
    error NoPrice(); // "no price"
    error NotAContract(); // "not a contract"
    error NotTheBuyer(); // "not the buyer"
    error NotTheDepositor(); // "not the depositor"
    error NotTheRecipient(); // "not the recipient"
    error NothingAccrued(); // "nothing accrued"
    error NothingEarned(); // "nothing earned"
    error NothingOwed(); // "nothing owed"
    error NothingToBurn(); // "nothing to burn"
    error OwnerOnly(); // "owner only"
    error OwnerOrCurator(); // "owner or curator"
    error PoolAtCapacity(); // "pool at capacity"
    error PoolNotOpen(); // "pool closed"
    error PoolExists(); // "pool exists"
    error PositionNotActive(); // "position not active"
    error PositionNotDrawn(); // "position not drawn"
    error RateWithoutCoin(); // "rate without a coin"
    error RevealExpired(); // "reveal expired"
    error SendTheDifference(); // "send the difference"
    error SinkRefused(); // "sink refused"
    error StillSettleable(); // "still settleable"
    error SurchargeTooHigh(); // "surcharge too high"
    error ThresholdTooHigh(); // "threshold too high"
    error TokenIsEscrowed(); // "token is escrowed"
    error TollRailNotEmpty(); // "toll rail not empty"
    error TollRailTakesNoEth(); // "toll rail: send no ETH"
    error TollShortDelivered(); // "toll short-delivered"
    error TooEarly(); // "too early"
    error TooEarlyToFinalize(); // "too early to finalize"
    error TooFewPositions(); // "too few positions"
    error TopNotBeaten(); // "top not beaten"
    error TopShareTooHigh(); // "top share too high"
    error TreeSlotMismatch(); // "tree/slot mismatch"
    error Underpaid(); // "underpaid"
    error ZeroOwner(); // "zero owner"
    error ZeroSink(); // "zero sink"
    error ZeroTo(); // "zero to"

    modifier onlyOwner() {
        require(msg.sender == owner, OwnerOnly());
        _;
    }

    /*//////////////////////////////////////////////////////////////
                              CONSTRUCTOR
    //////////////////////////////////////////////////////////////*/

    /// @param drawDelay_ **L2** blocks between a request and its reveal (the
    ///        ArbOS clock — see `ARBSYS`). 100 is ~10.1s on 4663's 101ms blocks,
    ///        the same "siege window, not an atomic snipe" shape the burrow
    ///        settled on. Bounded so a deployer can neither make the draw
    ///        next-block guessable nor push the reveal past the `arbBlockHash`
    ///        window it depends on.
    /// @param feeSink_ where the house's share is swept.
    constructor(uint256 drawDelay_, address feeSink_) {
        require(drawDelay_ >= 2 && drawDelay_ <= 128, DelayOutOfRange());
        require(feeSink_ != address(0), ZeroSink());
        drawDelay = drawDelay_;
        feeSink = feeSink_;
        owner = msg.sender;
    }

    /*//////////////////////////////////////////////////////////////
                               CURATION
    //////////////////////////////////////////////////////////////*/

    /// @notice Give a collection a pool. This IS the whitelist: no pool, no
    ///         deposits. `bidBps` is fixed here and never editable — depositors
    ///         price their backing against the buyback they were promised, and a
    ///         posted commitment is not something an operator gets to revise.
    function openPool(address collection, uint256 minBacking_, uint256 surchargeBps_, uint256 bidBps_) external {
        require(msg.sender == owner || msg.sender == curator, OwnerOrCurator());
        require(!blockedCollection[collection], CollectionIsBlocked());
        require(collection.code.length > 0, NotAContract());
        Pool storage p = pools[collection];
        require(!p.exists, PoolExists());
        require(minBacking_ >= MIN_BACKING_FLOOR, BelowFloor());
        require(surchargeBps_ <= MAX_SURCHARGE_BPS, SurchargeTooHigh());
        require(bidBps_ >= MIN_BID_BPS && bidBps_ <= MAX_BID_BPS, BidOutOfRange());

        p.exists = true;
        p.open = true;
        p.minBacking = minBacking_;
        p.surchargeBps = surchargeBps_;
        p.bidBps = bidBps_;
        p.nextUnusedSlot = 1;
        emit PoolOpened(collection, minBacking_, surchargeBps_, bidBps_);
    }

    /// @notice Stop or resume NEW deposits and draws. Exits are never gated:
    ///         a closed pool still lets every depositor withdraw and every
    ///         drawn position resolve.
    function setPoolOpen(address collection, bool open_) external {
        require(msg.sender == owner || msg.sender == curator, OwnerOrCurator());
        Pool storage p = pools[collection];
        require(p.exists, NoPool());
        // Blocking is one-way, and this is where that promise is actually kept.
        // `blockCollection` is owner-only and only sets `open = false`; without
        // this line the CURATOR — a seat documented as unable to touch money —
        // could reverse an owner's emergency block by re-opening the pool. Only
        // the re-opening direction is gated, so closing always works.
        if (open_) require(!blockedCollection[collection], CollectionIsBlocked());
        p.open = open_;
        emit PoolClosed(collection, open_);
    }

    /// @notice Bar a collection forever and close its pool. One-way.
    function blockCollection(address collection) external onlyOwner {
        blockedCollection[collection] = true;
        Pool storage p = pools[collection];
        if (p.exists) p.open = false;
        emit CollectionBlocked(collection);
    }

    /// @notice Move a pool's deposit floor and surcharge. Both bind NEW
    ///         activity only: an open position keeps its backing and weight, and
    ///         a buyer's price is quoted and capped in the same transaction.
    function setPoolLimits(address collection, uint256 minBacking_, uint256 surchargeBps_) external onlyOwner {
        Pool storage p = pools[collection];
        require(p.exists, NoPool());
        require(minBacking_ >= MIN_BACKING_FLOOR, BelowFloor());
        require(surchargeBps_ <= MAX_SURCHARGE_BPS, SurchargeTooHigh());
        p.minBacking = minBacking_;
        p.surchargeBps = surchargeBps_;
        emit PoolLimitsSet(collection, minBacking_, surchargeBps_);
    }

    /*//////////////////////////////////////////////////////////////
                          DEPOSIT / REPRICE / EXIT
    //////////////////////////////////////////////////////////////*/

    /// @notice Escrow a token plus `msg.value` of backing. Approve this
    ///         contract for the token first.
    function deposit(address collection, uint256 tokenId) external payable nonReentrant returns (uint256 positionId) {
        Pool storage p = pools[collection];
        require(p.exists && p.open, PoolNotOpen());
        require(p.pendingDraw == 0, DrawPending());
        require(msg.value >= p.minBacking, BelowMinBacking());

        uint256 weight = NUM / msg.value;
        require(weight > 0, BackingTooHigh());

        _pullNft(collection, tokenId);

        positionId = ++positionCount;
        positions[positionId] = Position({
            collection: collection,
            depositor: msg.sender,
            buyer: address(0),
            tokenId: tokenId,
            backing: msg.value,
            weight: weight,
            slot: 0,
            feeDebt: 0,
            tollDebt: 0,
            drawnAt: 0,
            chooseBy: 0,
            finalizeBy: 0,
            keepBps: 0,
            status: PositionStatus.Active
        });
        positionOfToken[collection][tokenId] = positionId;
        backingTotal += msg.value;

        _activate(p, positionId, weight, msg.value);
    }

    /// @dev Join the selection pool: checkpoint the dividend accumulator, take a
    ///      slot, add the weight, and run the top-slot contest. The checkpoint
    ///      is CEILED so a position can never collect a fee distributed before
    ///      it existed — the one rounding here that must go against the joiner.
    function _activate(Pool storage p, uint256 positionId, uint256 weight, uint256 backing) internal {
        Position storage pos = positions[positionId];
        pos.feeDebt = (p.eth.acc + ACC - 1) / ACC;
        pos.tollDebt = (p.toll.acc + ACC - 1) / ACC;

        uint256 slot = _allocateSlot(p);
        pos.slot = slot;
        p.slotToPosition[slot] = positionId;

        _addWeight(p, slot, weight);
        p.totalWeight += weight;
        p.weightedBacking += weight * backing;
        p.count += 1;

        uint256 top = p.topPositionId;
        if (top == 0) {
            if (_mayTakeTop(p, backing, 0)) _setTop(pos.collection, p, positionId);
        } else if (top != positionId && _beatsTop(backing, top)) {
            _vacateTop(pos.collection, p);
            _setTop(pos.collection, p, positionId);
        }

        emit Deposited(positionId, pos.collection, pos.depositor, pos.tokenId, backing, weight, slot);
    }

    /// @notice Change the backing on one of your active positions, which
    ///         re-prices its weight. Send the shortfall to raise it; a reduction
    ///         (and any overpayment) is credited back to you.
    /// @dev Raising a floor is the depositor's honest signal and lowering it is
    ///      their retreat, but both move the price a buyer would pay, so both
    ///      wait for an in-flight draw exactly like a withdrawal does. A lower
    ///      re-price forfeits the top slot, mirroring a withdrawal.
    function reprice(uint256 positionId, uint256 newBacking) external payable nonReentrant {
        Position storage pos = _activeOwned(positionId);
        Pool storage p = pools[pos.collection];
        require(p.pendingDraw == 0, DrawPending());
        require(newBacking >= p.minBacking, BelowMinBacking());
        uint256 oldBacking = pos.backing;
        if (newBacking > oldBacking) require(p.open, PoolNotOpen());

        uint256 newWeight = NUM / newBacking;
        require(newWeight > 0, BackingTooHigh());
        require(msg.value + oldBacking >= newBacking, SendTheDifference());
        uint256 back = msg.value + oldBacking - newBacking;

        // Settle at the OLD share before anything moves, so the dividend math
        // stays exact across the re-price.
        _accrue(positionId, pos, p);

        uint256 oldWeight = pos.weight;
        if (newWeight > oldWeight) {
            _addWeight(p, pos.slot, newWeight - oldWeight);
        } else if (newWeight < oldWeight) {
            _removeWeight(p, pos.slot, oldWeight - newWeight);
        }
        p.totalWeight = p.totalWeight - oldWeight + newWeight;
        p.weightedBacking = p.weightedBacking - oldWeight * oldBacking + newWeight * newBacking;

        pos.backing = newBacking;
        pos.weight = newWeight;
        backingTotal = backingTotal + newBacking - oldBacking;

        uint256 top = p.topPositionId;
        if (top == positionId) {
            if (newBacking < oldBacking) _vacateTop(pos.collection, p);
        } else if (top != 0 && _beatsTop(newBacking, top)) {
            _vacateTop(pos.collection, p);
            _setTop(pos.collection, p, positionId);
        }

        emit Repriced(positionId, oldBacking, newBacking, newWeight);
        if (back > 0) _credit(msg.sender, back);
        _payout(msg.sender);
    }

    /// @notice Take an undrawn position back: the token, its backing, and every
    ///         fee it earned.
    function withdraw(uint256 positionId) external nonReentrant {
        Position storage pos = _activeOwned(positionId);
        Pool storage p = pools[pos.collection];
        require(p.pendingDraw == 0, DrawPending());

        uint256 backing = pos.backing;
        _accrue(positionId, pos, p);
        _removeFromPool(p, positionId, pos);
        if (p.topPositionId == positionId) _vacateTop(pos.collection, p);

        pos.status = PositionStatus.Withdrawn;
        backingTotal -= backing;
        delete positionOfToken[pos.collection][pos.tokenId];

        _credit(msg.sender, backing);
        emit WithdrawnPosition(positionId, msg.sender, backing);

        _deliverNft(positionId, pos.collection, msg.sender, pos.tokenId);
        _payout(msg.sender);
    }

    /// @notice Sweep the fees your still-active positions have earned without
    ///         touching the positions. Touches no weight and no total, so it is
    ///         allowed even mid-draw.
    /// @dev Both rails accrue; only the ETH one is pushed. The toll lands in
    ///      `owedToll` and is collected with `claimToll`, so a harvest can never
    ///      revert on a sick token — see THE TOLL RAIL in the header.
    function harvest(uint256[] calldata positionIds)
        external
        nonReentrant
        returns (uint256 total, uint256 tollTotal)
    {
        for (uint256 i = 0; i < positionIds.length; i++) {
            Position storage pos = _activeOwned(positionIds[i]);
            (uint256 eth, uint256 tol) = _accrue(positionIds[i], pos, pools[pos.collection]);
            total += eth;
            tollTotal += tol;
        }
        require(total > 0 || tollTotal > 0, NothingEarned());
        _payout(msg.sender);
    }

    /*//////////////////////////////////////////////////////////////
                                THE DRAW
    //////////////////////////////////////////////////////////////*/

    /// @notice The fee a draw costs right now: the pool's expected value times
    ///         the posted surcharge. With inverse weights the EV *is* the
    ///         harmonic mean of the backings, so a pool of richer positions
    ///         prices cheaper — you are far likelier to draw a cheap one.
    function fee(address collection) public view returns (uint256) {
        Pool storage p = pools[collection];
        if (p.totalWeight == 0) return 0;
        uint256 ev = p.weightedBacking / p.totalWeight;
        return ev * (BPS + p.surchargeBps) / BPS;
    }

    /// @notice The pool's expected value — the harmonic mean of its backings,
    ///         and the exact expected backing of whatever the next draw lands.
    function harmonicMean(address collection) public view returns (uint256) {
        Pool storage p = pools[collection];
        if (p.totalWeight == 0) return 0;
        return p.weightedBacking / p.totalWeight;
    }

    /// @notice The same pull, priced in the toll coin — `fee` carried across the
    ///         posted rate. Zero whenever the toll is disarmed, which is also the
    ///         signal that `request` wants ETH.
    function toll(address collection) public view returns (uint256) {
        if (tollCoin == address(0) || tollRate == 0) return 0;
        return fee(collection) * tollRate / RATE_ONE;
    }

    /// @notice What a pull costs and in what asset, in one read. `asset` is zero
    ///         for native ETH. **The seam every caller should use** — a front end
    ///         that branches on this cannot hardcode `msg.value` into a button,
    ///         and a front end that reads `fee` alone will quote ETH into a
    ///         wallet that is about to be asked for `SCRY`.
    function quote(address collection) external view returns (address asset, uint256 amount) {
        uint256 t = toll(collection);
        if (t != 0) return (tollCoin, t);
        return (address(0), fee(collection));
    }

    /// @notice Pay for a draw. Send at least `fee(collection)`; the excess is
    ///         credited back. The position is chosen `drawDelay` blocks from now
    ///         by `settle`, which anyone may call — including you, which is the
    ///         point: an unsettled draw forfeits its fee.
    /// @param salt Buyer entropy, public and fixed here. Publishing it is safe:
    ///        the seed also eats a block hash that does not exist yet, so
    ///        nobody — buyer or house — can aim a salt at an outcome.
    /// @param maxFee Revert if the price exceeds this. 0 disables. Protects you
    ///        from a deposit landing in front of your request. **Denominated in
    ///        whatever `quote` returns** — ETH wei on the ETH rail, toll-coin wei
    ///        once the toll is armed. Read `quote`, not `fee`, or you will cap a
    ///        `SCRY` charge with an ETH number.
    /// @param minPositions Revert if the pool holds fewer active positions than
    ///        this. 0 disables. Protects you from drawing a one-horse pool.
    /// @param autoBidAbove If non-zero, the draw resolves ATOMICALLY at
    ///        settlement: a position backed above this takes the standing bid
    ///        (you get ETH), anything at or below it hands you the token. Zero
    ///        keeps the choice, and the choice is a free option you are welcome
    ///        to hold for `choiceWindow`.
    function request(address collection, bytes32 salt, uint256 maxFee, uint256 minPositions, uint256 autoBidAbove)
        external
        payable
        nonReentrant
        returns (uint256 drawId)
    {
        Pool storage p = pools[collection];
        require(p.exists && p.open, PoolNotOpen());
        require(p.pendingDraw == 0, DrawPending());
        require(p.count > 0, NoPositions());
        require(p.count >= minPositions, TooFewPositions());

        // The price and the rail, together. `t != 0` IS the armed toll: it can
        // only be nonzero when a coin and a rate are both posted.
        uint256 t = toll(collection);
        uint256 f = t != 0 ? 0 : fee(collection);
        require(f > 0 || t > 0, NoPrice());
        require(maxFee == 0 || (t != 0 ? t : f) <= maxFee, FeeAboveCap());

        if (t != 0) {
            // Refuse ETH outright rather than crediting it back. On this rail a
            // `msg.value` is a buyer who read `fee` instead of `quote`, and the
            // kind thing is to fail their transaction, not to take their ETH and
            // hand them a claim they will never think to make.
            require(msg.value == 0, TollRailTakesNoEth());
            _pullToll(msg.sender, t);
        } else {
            require(msg.value >= f, Underpaid());
        }

        drawId = ++drawCount;
        draws[drawId] = Draw({
            collection: collection,
            buyer: msg.sender,
            fee: f,
            toll: t,
            positionId: 0,
            autoBidAbove: autoBidAbove,
            revealBlock: uint64(_l2Block() + drawDelay),
            salt: salt,
            status: DrawStatus.Open
        });
        p.pendingDraw = drawId;
        if (f != 0) escrowTotal += f;
        if (t != 0) {
            escrowTollTotal += t;
            emit TollPaid(drawId, msg.sender, tollCoin, t);
        }

        // ⚠ `fee` here is the ETH leg and ONLY the ETH leg — zero on the toll
        // rail, where `TollPaid` above carries the amount and names the coin. One
        // event field must never mean two assets: an indexer summing `fee` across
        // both eras would otherwise add 11e18 of a coin to a column of ether and
        // report an eleven-ETH draw. A zero beside a `TollPaid` in the same
        // transaction is recoverable; a plausible wrong unit is not.
        emit DrawRequested(
            drawId, collection, msg.sender, f, _l2Block() + drawDelay, p.count, harmonicMean(collection)
        );

        uint256 excess = msg.value - f;
        if (excess > 0) {
            _credit(msg.sender, excess);
            _payout(msg.sender);
        }
    }

    /// @dev Pull the toll and prove it arrived. The balance check is not
    ///      ceremony: a fee-on-transfer or rebasing toll coin would deliver less
    ///      than the escrow claims, and every downstream split would then be
    ///      paying out of somebody else's credits. Surplus is left where it
    ///      lands, where it reads as `balanceOf(this) - accountedToll()` — a
    ///      donation, never a liability, exactly as `unaccountedEth` does.
    /// @dev The ONE place the toll coin leaves this contract. Claim, house
    ///      sweep and burn all funnel through it: `SafeERC20`'s tolerant check
    ///      is several hundred bytes inlined, and a contract already at the
    ///      EIP-170 ceiling cannot afford three copies of it.
    function _sendToll(address to, uint256 amount) internal {
        tollCoin.safeTransfer(to, amount, "toll send failed");
    }

    function _pullToll(address from, uint256 amount) internal {
        address coin = tollCoin;
        uint256 before = IERC20Balance(coin).balanceOf(address(this));
        coin.safeTransferFrom(from, address(this), amount, "toll transfer failed");
        require(IERC20Balance(coin).balanceOf(address(this)) - before >= amount, TollShortDelivered());
    }

    /// @notice Resolve a draw once its reveal block is mined. Permissionless on
    ///         purpose — the buyer, a depositor, or a bystander may all call it,
    ///         and the outcome is identical whoever does.
    function settle(uint256 drawId) external nonReentrant returns (uint256 positionId) {
        Draw storage d = draws[drawId];
        require(d.status == DrawStatus.Open, DrawNotOpen());
        require(_l2Block() > d.revealBlock, TooEarly());
        (bool ok, bytes32 bh) = _revealHash(d.revealBlock);
        require(ok, RevealExpired());

        Pool storage p = pools[d.collection];
        bytes32 seed = keccak256(abi.encode(bh, d.salt, drawId, d.collection, d.buyer));
        positionId = p.slotToPosition[_select(p, uint256(seed) % p.totalWeight)];
        require(positionId != 0, TreeSlotMismatch());

        Position storage pos = positions[positionId];
        require(pos.status == PositionStatus.Active, PositionNotActive());

        // The fee is split BEFORE the drawn position leaves, so the position
        // that bore the weight for this draw shares in the draw it lost.
        if (d.fee != 0) {
            _spread(d.collection, p, d.fee, false);
            escrowTotal -= d.fee;
            p.eth.volume += d.fee;
        }
        if (d.toll != 0) {
            _spread(d.collection, p, d.toll, true);
            escrowTollTotal -= d.toll;
            p.toll.volume += d.toll;
        }
        p.draws += 1;

        _removeFromPool(p, positionId, pos);
        if (p.topPositionId == positionId) _vacateTop(d.collection, p);

        pos.buyer = d.buyer;
        pos.drawnAt = uint64(block.timestamp);
        // The terms this draw resolves under, fixed here and never re-read from
        // the knobs. Stamped BEFORE the atomic `autoBidAbove` branch below, so
        // that path reads the same snapshot every other path does.
        pos.chooseBy = uint64(block.timestamp + choiceWindow);
        pos.finalizeBy = uint64(block.timestamp + finalizeWindow);
        pos.keepBps = uint16(houseKeepBps);
        pos.status = PositionStatus.Drawn;

        d.positionId = positionId;
        d.status = DrawStatus.Settled;
        p.pendingDraw = 0;

        emit DrawSettled(drawId, positionId, d.buyer, d.fee, pos.backing, seed);

        // A pre-committed rule resolves in the same transaction, so a buyer who
        // wants no option never holds one.
        if (d.autoBidAbove != 0) {
            if (pos.backing > d.autoBidAbove) {
                _takeBid(positionId, pos, p);
            } else {
                _keepNft(positionId, pos, false);
            }
        }
    }

    /// @notice Retire a draw whose reveal block has aged out of the node's
    ///         `arbBlockHash` window. The fee is distributed to the pool's
    ///         depositors exactly as a settlement would have distributed it.
    ///         **Nobody is refunded** — see the header, point 3: a refund here
    ///         would be a free option on every draw, and the buyer had the whole
    ///         window to settle it themselves.
    /// @dev The gate is the *unavailability of the hash*, not a block-count
    ///      comparison, so `settle` and `expire` are complementary at whatever
    ///      boundary the chain actually enforces — measured at 253-254 against a
    ///      documented 256, a gap that is pure measurement drift and exactly the
    ///      kind of off-by-a-few that a hardcoded window would have gotten wrong.
    function expire(uint256 drawId) external nonReentrant {
        Draw storage d = draws[drawId];
        require(d.status == DrawStatus.Open, DrawNotOpen());
        uint256 l2 = _l2Block();
        require(l2 > d.revealBlock, TooEarly());
        (bool ok,) = _revealHash(d.revealBlock);
        require(!ok, StillSettleable());

        Pool storage p = pools[d.collection];
        if (d.fee != 0) {
            _spread(d.collection, p, d.fee, false);
            escrowTotal -= d.fee;
            p.eth.volume += d.fee;
        }
        if (d.toll != 0) {
            _spread(d.collection, p, d.toll, true);
            escrowTollTotal -= d.toll;
            p.toll.volume += d.toll;
        }
        d.status = DrawStatus.Expired;
        p.pendingDraw = 0;
        // Same rule as `DrawRequested`: `fee` is the ETH leg only. The forfeited
        // toll rides in its own event so no field ever carries two units.
        emit DrawForfeited(drawId, d.buyer, d.fee);
        if (d.toll != 0) emit TollForfeited(drawId, d.buyer, tollCoin, d.toll);
    }

    /// @dev Route one fee down one rail: the burn (toll rail only — there is no
    ///      burning ETH), the house's posted cut, the top-slot slice, then an
    ///      equal share to every active position. Called while the drawn
    ///      position is still active, so it shares in the draw that took it.
    ///
    ///      **The two flat carves come off the whole amount**, which is exactly
    ///      the shape the toll safety inequality assumes when it proves a
    ///      depositor who buys from themselves loses `(rebateBps − burnBps −
    ///      houseDrawBps)·T` at any volume.
    ///
    ///      **Nothing here calls the token.** The burn and the house cut accrue;
    ///      `sweepBurn`/`sweepHouseToll` send them. This runs inside `settle`,
    ///      and a `settle` a token can revert is a pool that one abandoned pull
    ///      shuts down for good.
    function _spread(address collection, Pool storage p, uint256 amount, bool onToll) internal {
        Rail storage r = onToll ? p.toll : p.eth;
        uint256 distributable = amount;

        if (onToll) {
            uint256 burn = amount * tollBurnBps / BPS;
            if (burn != 0) {
                burnOwed += burn;
                distributable -= burn;
                emit TollBurnAccrued(burn);
            }
        }

        uint256 house = amount * houseDrawBps / BPS;
        if (house != 0) {
            distributable -= house;
            _accrueHouse(house, onToll);
        }

        uint256 top = p.topPositionId;
        if (top != 0 && topShareBps != 0) {
            uint256 share = distributable * topShareBps / BPS;
            if (share != 0) {
                r.topPot += share;
                distributable -= share;
                if (onToll) {
                    topPotTollTotal += share;
                    emit TopFundedToll(collection, top, share, r.topPot);
                } else {
                    topPotTotal += share;
                    emit TopFunded(collection, top, share, r.topPot);
                }
            }
        }
        _distribute(collection, p, r, distributable, onToll);
    }

    /*//////////////////////////////////////////////////////////////
                             RESOLUTION
    //////////////////////////////////////////////////////////////*/

    /// @notice Buyer: keep the token. The backing returns to its depositor less
    ///         the house's posted cut.
    /// @dev Strict delivery: if the collection refuses the transfer this whole
    ///      call reverts, so the buyer can fall back to `takeBid` instead of
    ///      being committed to a token that cannot move.
    function keepNft(uint256 positionId) external nonReentrant {
        Position storage pos = _drawnByBuyer(positionId);
        _keepNft(positionId, pos, true);
    }

    /// @notice Buyer: hand the token back and take the depositor's standing bid
    ///         — `bidBps` of that position's backing, paid out of that same
    ///         backing. The retained slice goes where `retainedToHouse` says.
    function takeBid(uint256 positionId) external nonReentrant {
        Position storage pos = _drawnByBuyer(positionId);
        _takeBid(positionId, pos, pools[pos.collection]);
    }

    /// @notice Depositor, once the buyer's window lapses: keep the ETH. Same
    ///         economics as the buyer choosing `keepNft`.
    function depositorReclaimBacking(uint256 positionId) external nonReentrant {
        Position storage pos = _drawnAfterWindow(positionId, true);
        _keepNft(positionId, pos, false);
    }

    /// @notice Depositor, once the buyer's window lapses: take the token back
    ///         and pay the bid. Same economics as the buyer choosing `takeBid`.
    function depositorReclaimNft(uint256 positionId) external nonReentrant {
        Position storage pos = _drawnAfterWindow(positionId, true);
        _takeBid(positionId, pos, pools[pos.collection]);
    }

    /// @notice Anyone, once `finalizeWindow` lapses with nobody resolving: the
    ///         token goes to the buyer and the backing to the depositor, so
    ///         neither asset can be locked by a party who walked away.
    function finalize(uint256 positionId) external nonReentrant {
        Position storage pos = _drawnAfterWindow(positionId, false);
        _keepNft(positionId, pos, false);
        emit Finalized(positionId, msg.sender);
    }

    function _keepNft(uint256 positionId, Position storage pos, bool strict) internal {
        pos.status = PositionStatus.Settled;
        uint256 backing = pos.backing;
        // The cut posted when this position was DRAWN, not whatever the knob says
        // now. Raising the knob must not reach backing already committed.
        uint256 house = backing * pos.keepBps / BPS;
        _accrueHouse(house, false);
        backingTotal -= backing;
        _credit(pos.depositor, backing - house);
        delete positionOfToken[pos.collection][pos.tokenId];

        emit NftKept(positionId, pos.buyer, pos.depositor, backing - house);

        if (strict) {
            IERC721Minimal(pos.collection).transferFrom(address(this), pos.buyer, pos.tokenId);
        } else {
            _deliverNft(positionId, pos.collection, pos.buyer, pos.tokenId);
        }
    }

    function _takeBid(uint256 positionId, Position storage pos, Pool storage p) internal {
        pos.status = PositionStatus.Settled;
        uint256 backing = pos.backing;
        uint256 payout = backing * p.bidBps / BPS;
        uint256 retained = backing - payout;

        backingTotal -= backing;
        _credit(pos.buyer, payout);
        if (retained != 0) {
            if (retainedToHouse) {
                _accrueHouse(retained, false);
            } else {
                _distribute(pos.collection, p, p.eth, retained, false);
            }
        }
        delete positionOfToken[pos.collection][pos.tokenId];

        emit BidTaken(positionId, pos.buyer, pos.depositor, payout, retained);
        _deliverNft(positionId, pos.collection, pos.depositor, pos.tokenId);
    }

    /// @notice Pull a token whose delivery failed at resolution. Only the
    ///         address it was owed to may take it; every ETH leg already settled.
    function recoverStuckNft(uint256 positionId) external nonReentrant {
        require(stuckNftRecipient[positionId] == msg.sender, NotTheRecipient());
        delete stuckNftRecipient[positionId];
        Position storage pos = positions[positionId];
        // The escrow record was restored when delivery failed; it goes only when
        // the token actually leaves. Cleared BEFORE the transfer so a revert
        // restores both and the recipient keeps their claim.
        delete positionOfToken[pos.collection][pos.tokenId];
        IERC721Minimal(pos.collection).transferFrom(address(this), msg.sender, pos.tokenId);
        emit StuckNftRecovered(positionId, msg.sender);
    }

    /// @notice Send out a token this contract holds that belongs to no position
    ///         and is owed to nobody — i.e. one someone pushed in with a raw
    ///         `transferFrom`, which no path here creates. The guard is the
    ///         whole door: a tracked token and a stuck token are both refused,
    ///         so this can never touch escrow.
    /// @dev The guard is now TRUE by construction, which it was not when this
    ///      comment first claimed it. A failed delivery restores
    ///      `positionOfToken`, so a token owed to someone through
    ///      `stuckNftRecipient` reads as escrowed here and is refused.
    function rescueStray(address collection, uint256 tokenId, address to) external onlyOwner {
        require(positionOfToken[collection][tokenId] == 0, TokenIsEscrowed());
        require(to != address(0), ZeroTo());
        IERC721Minimal(collection).transferFrom(address(this), to, tokenId);
        emit StrayRescued(collection, tokenId, to);
    }

    /*//////////////////////////////////////////////////////////////
                              THE TOP SLOT
    //////////////////////////////////////////////////////////////*/

    /// @notice Take the pool's top slot for one of your active positions. Vacant
    ///         is free; otherwise your backing must clear the standing top by
    ///         `topThresholdBps`. The outgoing holder banks their pot.
    function claimTop(uint256 positionId) external nonReentrant {
        Position storage pos = _activeOwned(positionId);
        Pool storage p = pools[pos.collection];
        uint256 top = p.topPositionId;
        if (top == positionId) return; // load-bearing: re-topping would zero your own pot
        require(_mayTakeTop(p, pos.backing, top), TopNotBeaten());
        if (top != 0) _vacateTop(pos.collection, p);
        _setTop(pos.collection, p, positionId);
    }

    /// @dev May `backing` take the seat? With a holder, it must beat them by the
    ///      posted threshold. With the seat VACANT it must still beat the bar the
    ///      seat was last held at — a fresh pool's bar is 0, so the first
    ///      depositor takes it freely, but a seat emptied by a draw or a
    ///      withdrawal cannot be picked up for the minimum backing.
    function _mayTakeTop(Pool storage p, uint256 backing, uint256 top) internal view returns (bool) {
        if (top != 0) return _beatsTop(backing, top);
        return backing * BPS >= p.topBacking * (BPS + topThresholdBps);
    }

    function _beatsTop(uint256 backing, uint256 top) internal view returns (bool) {
        // Cross-multiplied so no intermediate division rounds the bar down in
        // the challenger's favour.
        return backing * BPS >= positions[top].backing * (BPS + topThresholdBps);
    }

    function _setTop(address collection, Pool storage p, uint256 positionId) internal {
        p.topPositionId = positionId;
        // The bar for whoever comes next, and it outlives this tenure on purpose.
        p.topBacking = positions[positionId].backing;
        emit TopSet(collection, positionId, positions[positionId].depositor);
    }

    /// @dev Settle the pot to the outgoing holder and clear the slot. Idempotent,
    ///      and the pot resets in the same write as the id, so a tenure's pot
    ///      can be paid at most once and can never strand.
    function _vacateTop(address collection, Pool storage p) internal {
        uint256 top = p.topPositionId;
        if (top == 0) return;
        uint256 pot = p.eth.topPot;
        uint256 potToll = p.toll.topPot;
        p.topPositionId = 0;
        p.eth.topPot = 0;
        p.toll.topPot = 0;
        address depositor = positions[top].depositor;
        if (pot != 0) {
            topPotTotal -= pot;
            _credit(depositor, pot);
            emit TopSettled(collection, top, depositor, pot);
        }
        // Both pots leave in the same write as the id, for the same reason: a
        // tenure that earned on both rails must be paid on both, once.
        if (potToll != 0) {
            topPotTollTotal -= potToll;
            _creditToll(depositor, potToll);
            emit TopSettledToll(collection, top, depositor, potToll);
        }
        emit TopSet(collection, 0, address(0));
    }

    /*//////////////////////////////////////////////////////////////
                          FEES AND ACCOUNTING
    //////////////////////////////////////////////////////////////*/

    /// @dev Equal split across active positions, with the remainder CARRIED.
    ///      The carry is not a nicety: `ScryGardener` and `ScrySilo` both learned
    ///      that an accumulator which drops `amount·ACC % count` on every
    ///      distribution loses that value forever, and a public caller can
    ///      arrange for the floor to bite repeatedly. With nothing active to
    ///      receive it the amount goes to the house rather than nowhere.
    function _distribute(address collection, Pool storage p, Rail storage r, uint256 amount, bool onToll) internal {
        if (amount == 0) return;
        if (p.count == 0) {
            // Nothing active to receive it, so it goes to the house rather than
            // nowhere — on the rail it arrived on, which is the only rail it can
            // actually be swept from.
            _accrueHouse(amount, onToll);
            return;
        }
        uint256 scaled = amount * ACC + r.carry;
        uint256 inc = scaled / p.count;
        r.acc += inc;
        r.carry = scaled - inc * p.count;
        if (onToll) {
            tollCreditsOutstanding += amount;
            emit TollDistributed(collection, amount, inc / ACC, p.count);
        } else {
            feeCreditsOutstanding += amount;
            emit FeesDistributed(collection, amount, inc / ACC, p.count);
        }
    }

    /// @dev Fees a position has earned since its checkpoint. Saturating: the
    ///      floored accumulator can sit just under a ceiled checkpoint, and the
    ///      contract must round against the earner, never against itself.
    function _due(uint256 acc, uint256 debt) internal pure returns (uint256) {
        uint256 a = acc / ACC;
        return a > debt ? a - debt : 0;
    }

    /// @dev Credit a position's earnings and advance its checkpoint.
    ///
    ///      **The checkpoint only ever moves FORWARD, and that guard is the whole
    ///      point of this function's shape** (review, 2026-07-27). `_activate`
    ///      deliberately CEILS the join checkpoint so a joiner can never be paid
    ///      for a distribution that preceded it. This used to assign the FLOOR
    ///      unconditionally, which silently undid that ceiling: a position could
    ///      call `reprice(id, itsOwnBacking)` with no value — a legal no-op — and
    ///      walk its own checkpoint back below its join point. `_pending`
    ///      saturates, so nothing was paid on that call and nothing looked wrong;
    ///      the position simply collected one extra wei of *other* depositors'
    ///      fees on every later accrual.
    function _accrue(uint256 positionId, Position storage pos, Pool storage p)
        internal
        returns (uint256 amount, uint256 tollAmount)
    {
        amount = _due(p.eth.acc, pos.feeDebt);
        uint256 settled = p.eth.acc / ACC;
        if (settled > pos.feeDebt) pos.feeDebt = settled;
        if (amount != 0) {
            // Saturating on purpose. `feeCreditsOutstanding` is a COUNTER, and the
            // ETH it describes is physically present either way — but it sits on
            // the withdraw, harvest, reprice and settle paths, so a plain `-=`
            // turns any residual accounting drift into a panic that locks a
            // depositor's backing AND their token in a pool that can no longer
            // receive a fee to top the counter back up. A wrong counter must never
            // outrank a real exit.
            feeCreditsOutstanding = feeCreditsOutstanding > amount ? feeCreditsOutstanding - amount : 0;
            _credit(pos.depositor, amount);
            emit EarningsAccrued(pos.depositor, positionId, amount);
        }

        // The toll rail, settled in the same breath and by the same rules — a
        // position that has earned on both must never be able to exit on one and
        // strand the other, which is what a separate `harvestToll` would invite.
        tollAmount = _due(p.toll.acc, pos.tollDebt);
        uint256 settledToll = p.toll.acc / ACC;
        if (settledToll > pos.tollDebt) pos.tollDebt = settledToll;
        if (tollAmount != 0) {
            tollCreditsOutstanding = tollCreditsOutstanding > tollAmount ? tollCreditsOutstanding - tollAmount : 0;
            _creditToll(pos.depositor, tollAmount);
            emit TollAccrued(pos.depositor, positionId, tollAmount);
        }
    }

    /// @dev Take a position out of the selection pool: settle its fees, drop its
    ///      weight and totals, free its slot.
    function _removeFromPool(Pool storage p, uint256 positionId, Position storage pos) internal {
        _accrue(positionId, pos, p);
        uint256 slot = pos.slot;
        _removeWeight(p, slot, pos.weight);
        p.totalWeight -= pos.weight;
        p.weightedBacking -= pos.weight * pos.backing;
        p.count -= 1;
        delete p.slotToPosition[slot];
        _releaseSlot(p, slot);
        pos.slot = 0;
    }

    function _accrueHouse(uint256 amount, bool onToll) internal {
        if (amount == 0) return;
        if (onToll) {
            houseTollOwed += amount;
            emit HouseTollAccrued(amount);
        } else {
            houseOwed += amount;
            emit HouseAccrued(amount);
        }
    }

    function _credit(address to, uint256 amount) internal {
        if (amount == 0) return;
        owed[to] += amount;
        owedTotal += amount;
    }

    function _creditToll(address to, uint256 amount) internal {
        if (amount == 0) return;
        owedToll[to] += amount;
        owedTollTotal += amount;
    }

    /// @notice Collect the ETH credited to you. **Never touches the toll coin**,
    ///         so no state of that token can stand between a depositor and their
    ///         backing.
    function claim() external nonReentrant returns (uint256 amount) {
        amount = _payout(msg.sender);
        require(amount > 0, NothingOwed());
    }

    /// @notice Collect the toll coin credited to you. A separate call from
    ///         `claim` on purpose — see THE TOLL RAIL in the header.
    function claimToll() external nonReentrant returns (uint256 amount) {
        amount = owedToll[msg.sender];
        require(amount > 0, NothingOwed());
        owedToll[msg.sender] = 0;
        owedTollTotal -= amount;
        _sendToll(msg.sender, amount);
        emit TollClaimed(msg.sender, amount);
    }

    function _payout(address to) internal returns (uint256 amount) {
        amount = owed[to];
        if (amount == 0) return 0;
        owed[to] = 0;
        owedTotal -= amount;
        (bool ok,) = to.call{value: amount}("");
        require(ok, EthSendFailed());
        emit Claimed(to, amount);
    }

    /// @notice Push the house's accrued share to the posted sink. Permissionless
    ///         — the destination is not the caller's choice.
    function sweepHouse() external nonReentrant returns (uint256 amount) {
        amount = houseOwed;
        require(amount > 0, NothingAccrued());
        houseOwed = 0;
        address to = feeSink;
        (bool ok,) = to.call{value: amount}("");
        require(ok, SinkRefused());
        emit HouseSwept(to, amount);
    }

    /// @notice Push the house's accrued toll to the same posted sink.
    ///         Permissionless, and the destination is not the caller's choice.
    /// @dev Separate from `sweepHouse` for the same reason `claimToll` is
    ///      separate from `claim`: a toll coin that will not move must not be
    ///      able to hold the ETH sweep hostage. When `feeSink` is
    ///      `ScryFeeSplitter` this is the whole handoff — the splitter takes it
    ///      from here on its own posted split, which is why the house side of
    ///      the toll rail needed zero new machinery.
    function sweepHouseToll() external nonReentrant returns (uint256 amount) {
        amount = houseTollOwed;
        require(amount > 0, NothingAccrued());
        houseTollOwed = 0;
        address to = feeSink;
        _sendToll(to, amount);
        emit HouseTollSwept(to, amount);
    }

    /// @notice Send the accrued burn slice to `BURN`. Permissionless — the
    ///         destination is a constant, so there is nothing for a caller to
    ///         choose and no reason to gate it.
    /// @dev `totalBurned` moves HERE and not in `_spread`, so the advertised
    ///      number counts coins that actually left rather than coins we intend
    ///      to send. ⚠ It is not a supply reduction — `totalSupply()` is
    ///      unchanged and the coins are merely unspendable.
    function sweepBurn() external nonReentrant returns (uint256 amount) {
        amount = burnOwed;
        require(amount > 0, NothingToBurn());
        burnOwed = 0;
        _sendToll(BURN, amount);
        totalBurned += amount;
        emit Burned(amount, totalBurned);
    }

    /*//////////////////////////////////////////////////////////////
                        SEGMENT TREE OVER SLOTS
    //////////////////////////////////////////////////////////////*/

    function _addWeight(Pool storage p, uint256 slot, uint256 amount) internal {
        uint256 node = LEAF_BASE + slot - 1;
        while (true) {
            p.tree[node] += amount;
            if (node == 1) break;
            node >>= 1;
        }
    }

    function _removeWeight(Pool storage p, uint256 slot, uint256 amount) internal {
        uint256 node = LEAF_BASE + slot - 1;
        while (true) {
            p.tree[node] -= amount;
            if (node == 1) break;
            node >>= 1;
        }
    }

    /// @dev Walk the cumulative weight down to a leaf: descend left while the
    ///      target is inside the left subtree, otherwise subtract it and go
    ///      right. `TREE_DEPTH` reads, whatever the position count.
    function _select(Pool storage p, uint256 target) internal view returns (uint256 slot) {
        uint256 node = 1;
        while (node < LEAF_BASE) {
            uint256 left = node << 1;
            uint256 leftWeight = p.tree[left];
            if (target < leftWeight) {
                node = left;
            } else {
                target -= leftWeight;
                node = left | 1;
            }
        }
        slot = node - LEAF_BASE + 1;
    }

    function _allocateSlot(Pool storage p) internal returns (uint256 slot) {
        slot = p.freeSlotHead;
        if (slot != 0) {
            p.freeSlotHead = p.nextFreeSlot[slot];
            delete p.nextFreeSlot[slot];
            return slot;
        }
        slot = p.nextUnusedSlot++;
        require(slot <= CAPACITY, PoolAtCapacity());
    }

    function _releaseSlot(Pool storage p, uint256 slot) internal {
        p.nextFreeSlot[slot] = p.freeSlotHead;
        p.freeSlotHead = slot;
    }

    /*//////////////////////////////////////////////////////////////
                                THE CLOCK
    //////////////////////////////////////////////////////////////*/

    /// @dev The L2 height. NOT `block.number`, which on Nitro is the parent
    ///      chain's and moves ~100x slower — see the `ARBSYS` constant for the
    ///      measurement and for what using the wrong one silently did.
    function _l2Block() internal view returns (uint256) {
        return ARBSYS.arbBlockNumber();
    }

    /// @dev The reveal hash, and whether it is still available. `arbBlockHash`
    ///      reverts outside the node's window instead of returning zero, so the
    ///      success of the call — not a block-count comparison — is what divides
    ///      a settleable draw from a forfeit one. Returning a flag rather than
    ///      reverting lets both branches read the same source of truth.
    function _revealHash(uint256 revealBlock) internal view returns (bool ok, bytes32 h) {
        try ARBSYS.arbBlockHash(revealBlock) returns (bytes32 got) {
            // A zero hash is treated as unavailable too: no honest block hashes
            // to zero, and a node that answers zero instead of reverting must not
            // be allowed to seed a draw with a constant.
            return (got != bytes32(0), got);
        } catch {
            return (false, bytes32(0));
        }
    }

    /*//////////////////////////////////////////////////////////////
                          GUARDS AND NFT MOVES
    //////////////////////////////////////////////////////////////*/

    function _activeOwned(uint256 positionId) internal view returns (Position storage pos) {
        pos = positions[positionId];
        require(pos.status == PositionStatus.Active, PositionNotActive());
        require(pos.depositor == msg.sender, NotTheDepositor());
    }

    function _drawnByBuyer(uint256 positionId) internal view returns (Position storage pos) {
        pos = positions[positionId];
        require(pos.status == PositionStatus.Drawn, PositionNotDrawn());
        require(pos.buyer == msg.sender, NotTheBuyer());
    }

    /// @dev `asDepositor` gates the depositor's post-window reclaims on
    ///      `choiceWindow`; the open `finalize` waits for `finalizeWindow`.
    function _drawnAfterWindow(uint256 positionId, bool asDepositor) internal view returns (Position storage pos) {
        pos = positions[positionId];
        require(pos.status == PositionStatus.Drawn, PositionNotDrawn());
        if (asDepositor) {
            require(pos.depositor == msg.sender, NotTheDepositor());
            require(block.timestamp >= uint256(pos.chooseBy), BuyersWindow());
        } else {
            require(block.timestamp >= uint256(pos.finalizeBy), TooEarlyToFinalize());
        }
    }

    /// @dev Pull the token in with `transferFrom`, then verify. Plain rather than
    ///      safe on purpose, and the omission is the feature: this contract
    ///      implements no `onERC721Received`, so a token pushed here with a safe
    ///      transfer REVERTS instead of landing as an untracked deposit with no
    ///      backing behind it. The `ownerOf` check catches a collection whose
    ///      `transferFrom` lies.
    function _pullNft(address collection, uint256 tokenId) internal {
        IERC721Minimal(collection).transferFrom(msg.sender, address(this), tokenId);
        require(IERC721Minimal(collection).ownerOf(tokenId) == address(this), NftTransferFailed());
    }

    /// @dev Best-effort delivery. A paused, reverting or non-standard collection
    ///      must never be able to trap the ETH legs of a resolution or lock a
    ///      position, so a failure records a pull claim instead of reverting.
    function _deliverNft(uint256 positionId, address collection, address to, uint256 tokenId) internal {
        try IERC721Minimal(collection).transferFrom(address(this), to, tokenId) {
            return;
        } catch {
            stuckNftRecipient[positionId] = to;
            // Put the escrow record BACK. Every resolution path clears
            // `positionOfToken` before it gets here, so a token whose delivery
            // failed used to sit in this contract with no escrow record and a live
            // claim — which is exactly the state `rescueStray` reads as "loose"
            // (review, 2026-07-27: both reviewers found it independently). The
            // pool is still holding this token on someone's behalf, so the record
            // that says so must be true until they actually take it.
            positionOfToken[collection][tokenId] = positionId;
            emit NftDeliveryFailed(positionId, to, collection, tokenId);
        }
    }

    /*//////////////////////////////////////////////////////////////
                          VIEWS FOR THE BOOK
    //////////////////////////////////////////////////////////////*/

    /// @notice Everything `/gacha/book` needs to publish the odds in one read.
    ///         The Book serves the exact draw probability of every position and
    ///         the closed-form keep-or-bid boundary, free, on the same terms as
    ///         `/barrow/book` — a game that publishes its own solution is a
    ///         teaching instrument (`SENTENCES.md` 2026-07-25).
    function poolCard(address collection)
        external
        view
        returns (
            bool exists,
            bool open,
            uint256 minBacking_,
            uint256 surchargeBps_,
            uint256 bidBps_,
            uint256 count,
            uint256 totalWeight,
            uint256 ev,
            uint256 price,
            uint256 pendingDraw,
            uint256 topPositionId,
            uint256 topPot,
            uint256 lifetimeDraws,
            uint256 lifetimeVolume
        )
    {
        Pool storage p = pools[collection];
        return (
            p.exists,
            p.open,
            p.minBacking,
            p.surchargeBps,
            p.bidBps,
            p.count,
            p.totalWeight,
            harmonicMean(collection),
            fee(collection),
            p.pendingDraw,
            p.topPositionId,
            p.eth.topPot,
            p.draws,
            p.eth.volume
        );
    }

    /// @notice The backing a challenger must beat (by `topThresholdBps`) to take
    ///         the top slot, whether or not anyone currently holds it. A vacant
    ///         seat still carries the bar it was last held at.
    function topBar(address collection) external view returns (uint256) {
        return pools[collection].topBacking;
    }

    /// @notice A position's exact draw probability, in parts per million of the
    ///         pool's weight. Derived, never stored: it is `wᵢ / Σw`.
    function drawChancePpm(uint256 positionId) external view returns (uint256) {
        Position storage pos = positions[positionId];
        if (pos.status != PositionStatus.Active) return 0;
        Pool storage p = pools[pos.collection];
        if (p.totalWeight == 0) return 0;
        return pos.weight * 1_000_000 / p.totalWeight;
    }

    /// @notice Unclaimed fees on an active position, both rails. `toll_` is
    ///         denominated in `tollCoin` and `eth` in wei — they are different
    ///         assets and are never summed.
    function pendingFees(uint256 positionId) external view returns (uint256 eth, uint256 toll_) {
        Position storage pos = positions[positionId];
        if (pos.status != PositionStatus.Active) return (0, 0);
        Pool storage p = pools[pos.collection];
        return (_due(p.eth.acc, pos.feeDebt), _due(p.toll.acc, pos.tollDebt));
    }

    /// @notice The toll rail's side of a pool, kept OUT of `poolCard` so that
    ///         tuple's shape — which the meter and the page decode positionally
    ///         — never moves under a reader that predates the toll.
    function tollCard(address collection)
        external
        view
        returns (
            address coin,
            uint256 rate,
            uint256 burnBps_,
            uint256 price,
            uint256 topPotToll_,
            uint256 lifetimeTollVolume
        )
    {
        Pool storage p = pools[collection];
        return (tollCoin, tollRate, tollBurnBps, toll(collection), p.toll.topPot, p.toll.volume);
    }

    /// @notice The position a slot holds, for anyone rebuilding the tree.
    function positionAtSlot(address collection, uint256 slot) external view returns (uint256) {
        return pools[collection].slotToPosition[slot];
    }

    /// @notice Root of a pool's segment tree. Equals `totalWeight` whenever the
    ///         tree and the running sum agree — which is the cheapest external
    ///         check that the two never drifted.
    function treeRoot(address collection) external view returns (uint256) {
        return pools[collection].tree[1];
    }

    /// @notice Every wei this contract owes someone. `unaccountedEth` is the
    ///         remainder: rounding dust and anything sent here by mistake, never
    ///         a liability. The invariant is `address(this).balance >=
    ///         accountedEth()`, and the suite asserts it after every action.
    function accountedEth() public view returns (uint256) {
        return backingTotal + escrowTotal + owedTotal + houseOwed + topPotTotal + feeCreditsOutstanding;
    }

    function unaccountedEth() external view returns (uint256) {
        uint256 acc = accountedEth();
        uint256 bal = address(this).balance;
        return bal > acc ? bal - acc : 0;
    }

    /// @notice Every unit of the toll coin this contract owes someone. The exact
    ///         twin of `accountedEth`, and the reason `setToll` can tell an empty
    ///         rail from a busy one with a single read. There is no backing term:
    ///         backing is ETH, always.
    function accountedToll() public view returns (uint256) {
        return escrowTollTotal + owedTollTotal + houseTollOwed + topPotTollTotal + tollCreditsOutstanding + burnOwed;
    }

    /// @notice The toll rail's six liabilities, itemised. One view rather than
    ///         six public counters, because each auto-getter is bytecode this
    ///         contract does not have to spare — and they are only ever read
    ///         together anyway, by the invariant suite and by `/gacha`.
    ///         `unaccountedToll` is `balanceOf(this) - accountedToll()` and is
    ///         left to the caller for the same reason.
    function tollLedger()
        external
        view
        returns (
            uint256 escrowed,
            uint256 owedOut,
            uint256 house,
            uint256 topPots,
            uint256 credits,
            uint256 burnPending
        )
    {
        return (escrowTollTotal, owedTollTotal, houseTollOwed, topPotTollTotal, tollCreditsOutstanding, burnOwed);
    }

    /*//////////////////////////////////////////////////////////////
                                KNOBS
    //////////////////////////////////////////////////////////////*/

    function setHouseBps(uint256 drawBps, uint256 keepBps) external onlyOwner {
        require(drawBps <= MAX_HOUSE_BPS && keepBps <= MAX_HOUSE_BPS, HouseCutTooHigh());
        houseDrawBps = drawBps;
        houseKeepBps = keepBps;
        emit KnobSet("houseDrawBps", drawBps);
        emit KnobSet("houseKeepBps", keepBps);
    }

    function setTopKnobs(uint256 shareBps, uint256 thresholdBps) external onlyOwner {
        require(shareBps <= MAX_TOP_SHARE_BPS, TopShareTooHigh());
        require(thresholdBps <= BPS, ThresholdTooHigh());
        topShareBps = shareBps;
        topThresholdBps = thresholdBps;
        emit KnobSet("topShareBps", shareBps);
        emit KnobSet("topThresholdBps", thresholdBps);
    }

    function setRetainedToHouse(bool toHouse) external onlyOwner {
        retainedToHouse = toHouse;
        emit KnobSet("retainedToHouse", toHouse ? 1 : 0);
    }

    /// @dev Both windows bound the same asset, so they are set together and
    ///      checked against each other — a finalize window under the choice
    ///      window would let a bystander finalize inside the buyer's own turn.
    function setWindows(uint256 choice, uint256 finalize_) external onlyOwner {
        require(choice <= finalize_, ChoiceExceedsFinalize());
        require(finalize_ <= 30 days, FinalizeTooLong());
        // A floor as well as a ceiling. At zero the depositor could resolve in the
        // block after settlement and strip the choice the buyer's fee paid for —
        // already-drawn positions are safe because their deadlines are snapshotted,
        // but a future draw would inherit a window that is not a window.
        require(choice >= 10 minutes, ChoiceWindowTooShort());
        choiceWindow = choice;
        finalizeWindow = finalize_;
        emit KnobSet("choiceWindow", choice);
        emit KnobSet("finalizeWindow", finalize_);
    }

    /// @notice Arm, retune or disarm the toll rail.
    /// @param coin The rail's denomination — `SCRY`, or zero for the ETH rail.
    /// @param rate Toll-coin wei per `RATE_ONE` of the ETH-quoted fee. **Zero is
    ///        the disarm**, and it is always legal.
    /// @dev Two separate guards, because they answer two different questions.
    ///
    ///      **Changing the COIN requires an empty rail.** `owedToll`, an escrowed
    ///      toll, an unswept house cut, a top pot and an unrealised credit are all
    ///      denominated in whatever this address says at the moment they are
    ///      paid. Letting the address move while any of them is outstanding would
    ///      pay a depositor in a coin they never earned — or, with a coin the
    ///      contract holds none of, not at all. `accountedToll()` is exactly that
    ///      question, which is why it is a running sum rather than something to
    ///      re-derive here.
    ///
    ///      **Changing the RATE is unguarded, and disarming is instant.** Setting
    ///      the rate to zero moves new pulls back to ETH in the next block and
    ///      strands nothing: the coin stays posted, so every credit already
    ///      earned is still claimable in the coin it was earned in. That split is
    ///      the whole reason there are two knobs — "stop selling in `SCRY`" must
    ///      not have to wait on the last depositor to press claim.
    ///
    ///      ⚠ A rate change **re-prices the next pull, never an open one**: a
    ///      draw's toll is fixed and escrowed at `request`, and `maxFee` caps it
    ///      in the same transaction. Same promise `pos.keepBps` keeps for a
    ///      resolution.
    function setToll(address coin, uint256 rate) external onlyOwner {
        if (coin != tollCoin) {
            require(accountedToll() == 0, TollRailNotEmpty());
            if (coin != address(0)) require(coin.code.length > 0, NotAContract());
        }
        require(coin != address(0) || rate == 0, RateWithoutCoin());
        tollCoin = coin;
        tollRate = rate;
        emit TollSet(coin, rate);
    }

    /// @notice The burn slice, in bps of every toll. Zero turns it off.
    /// @dev ⚠ The toll safety inequality lives here when the cold-gap
    ///      rebate lands: `rebateBps < burnBps + houseDrawBps` is what makes a
    ///      depositor who buys from themselves lose money at any volume, and it
    ///      is meant to be enforced in this setter rather than remembered. There
    ///      is no rebate today and so no inequality to check — **whoever adds
    ///      one adds the require here, not a comment somewhere.**
    function setTollBurnBps(uint256 bps) external onlyOwner {
        require(bps <= MAX_TOLL_BURN_BPS, BurnTooHigh());
        tollBurnBps = bps;
        emit KnobSet("tollBurnBps", bps);
    }

    function setFeeSink(address sink) external onlyOwner {
        require(sink != address(0), ZeroSink());
        feeSink = sink;
        emit FeeSinkSet(sink);
    }

    function setCurator(address who) external onlyOwner {
        curator = who; // zero revokes
        emit CuratorSet(who);
    }

    function transferOwner(address to) external onlyOwner {
        require(to != address(0), ZeroOwner());
        emit OwnerTransferred(owner, to);
        owner = to;
    }
}
