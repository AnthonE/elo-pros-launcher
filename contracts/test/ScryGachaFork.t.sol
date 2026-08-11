// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import {ScryGacha} from "../src/ScryGacha.sol";
import {MockArbSys} from "./MockArbSys.sol";

/// The real collection's surface, as the pool actually touches it.
interface IRealNft {
    function ownerOf(uint256 tokenId) external view returns (address);
    function setApprovalForAll(address operator, bool approved) external;
    function supportsInterface(bytes4 id) external view returns (bool);
    function balanceOf(address owner) external view returns (uint256);
}

/// @title ScryGachaFork — the pool against a real RH-Chain collection
/// @notice The unit suite proves the arithmetic against a mock ERC-721 that
///         behaves. This runs the same flows against **StonkBrokers' real
///         deployed bytecode and real holders**, on a fork of chain 4663, so a
///         collection that reverts, filters operators, or lies about a transfer
///         fails HERE rather than in front of an audience.
///
///         Why this collection: measured 2026-07-26, it produced the single
///         largest NFT sale in the shelf sample (3.33 ETH), and it is
///         Anvil/Clutch-affiliated — the crew `GACHA.md` §1b names as the actual
///         clock on this window. 4,444 supply, 528 holders.
///
///         **Its floor is nowhere near the global one, and the pool is opened
///         accordingly.** `MIN_BACKING_FLOOR` is 0.01 ETH because that is ~p80
///         of *all* RH-Chain NFT sales; StonkBrokers' own median is ~$3,900. A
///         0.01 ETH position in a pool of 1 ETH positions would take ~99% of the
///         draws and drag the price to nothing — exactly what the harmonic mean
///         does, and exactly why `minBacking` is per-pool and may only go up.
///         This test opens it at **0.25 ETH** to make that concrete.
///
///         **What a fork test CANNOT prove, stated so nobody reads more into a
///         green run than is there: the clock.** 4663 is Arbitrum Nitro, the
///         contract reads its height and block hashes from the ArbOS precompile
///         at `0x64`, and **foundry does not implement ArbOS precompiles** — on a
///         fork that address holds one byte of non-executable code. So
///         `MockArbSys` is etched in here exactly as in the unit suite, and this
///         harness proves the COLLECTION interaction, never the clock.
///
///         The clock was settled separately and keylessly against the live node,
///         with `eth_call` + a state override and `eth_getBlockByHash`:
///
///           - `ArbSys.arbBlockNumber()` is the L2 height (20,319,770 when
///             Solidity's `block.number` read **25,620,709** — the parent
///             chain's, advancing 0.08 blocks/s against the L2's 9.9);
///           - `arbBlockHash(n)` returns the **exact** L2 block hash — verified
///             at head-1, -2 and -100 — and **reverts** outside its window,
///             measured at the 253-254 boundary against a documented 256 (the
///             gap is drift between reading a head and the call landing);
///           - plain `eth_blockNumber` is tight — 12 reads spanned 6 blocks,
///             monotonic — so a keeper watching for a reveal block reads a fresh
///             head and the ~26s settle deadline is real.
///
///         An earlier version of this contract used `block.number`/`blockhash`
///         and would have shipped a ~20 minute reveal advertised as ten seconds.
///
///         Runs on a FORK — nothing broadcasts, nothing is spent. Env-guarded
///         like `OrchardFork` and `SeedSpoilsFork`: without `RH_FORK_URL` it logs
///         a SKIP and passes, so the suite stays green offline. **The gas tell
///         from `scry-contracts-toolchain` applies — an offline skip-pass costs
///         ~4-6k gas, a real run costs millions. Check it before believing a
///         green line.**
///
///           RH_FORK_URL=https://rpc.mainnet.chain.robinhood.com \
///             forge test --match-contract ScryGachaForkTest -vv
contract ScryGachaForkTest is Test {
    /// StonkBrokers. Measured standard ERC-721: `supportsInterface(0x80ac58cd)`
    /// = 1, `transferFrom`/`setApprovalForAll`/`ownerOf` present, **no operator
    /// filter registry in the bytecode and no `paused()`** — the two things that
    /// would have blocked a pool contract outright.
    address constant COLLECTION = 0x539CdD042c2f3d93EbC5BE7DfFf0c79F3B4fAbF0;

    bytes4 constant ERC721_ID = 0x80ac58cd;
    uint256 constant MIN_BACKING = 0.25 ether; // this collection's floor, not the global one
    uint256 constant DELAY = 100; // ~10s at 101ms blocks
    uint256 constant SURCHARGE_BPS = 1_000;
    uint256 constant BID_BPS = 8_500;

    bool skipRun;
    ScryGacha g;
    MockArbSys arb;
    address constant ARBSYS = 0x0000000000000000000000000000000000000064;
    address sink = address(0x5EEE);
    address buyer = address(0xB1DDE7);

    function setUp() public {
        string memory url = vm.envOr("RH_FORK_URL", string(""));
        if (bytes(url).length == 0) {
            skipRun = true;
            return;
        }
        vm.createSelectFork(url);
        // The clock has to be etched even on a fork: foundry does not implement
        // the ArbOS precompiles, so 0x64 holds one unusable byte. `vm.etch`
        // copies code only — no constructor, no storage — so both values are set
        // explicitly (window left implicit is 0, and every reveal then reads as
        // expired while the expiry test passes for the wrong reason).
        vm.etch(ARBSYS, address(new MockArbSys()).code);
        arb = MockArbSys(ARBSYS);
        arb.setL2Block(20_000_000);
        arb.setWindow(256);

        g = new ScryGacha(DELAY, sink);
        g.openPool(COLLECTION, MIN_BACKING, SURCHARGE_BPS, BID_BPS);
        vm.deal(buyer, 100 ether);
    }

    // ---------------------------------------------------------------- helpers

    /// Two distinct real holders and one token each, discovered by walking the
    /// live collection. Never hardcoded: a token can be sold or burned between
    /// runs, and a fork test pinned to one owner rots the day it trades.
    function _twoRealHolders()
        internal
        view
        returns (address ownerA, uint256 idA, address ownerB, uint256 idB)
    {
        for (uint256 id = 1; id <= 60; id++) {
            address o;
            try IRealNft(COLLECTION).ownerOf(id) returns (address found) {
                o = found;
            } catch {
                continue;
            }
            if (o == address(0) || o.code.length != 0) continue; // EOAs only
            if (ownerA == address(0)) {
                (ownerA, idA) = (o, id);
            } else if (o != ownerA) {
                return (ownerA, idA, o, id);
            }
        }
        revert("fork: could not find two EOA holders in the first 60 ids");
    }

    function _deposit(address owner, uint256 id, uint256 backing) internal returns (uint256 posId) {
        vm.deal(owner, backing + 1 ether);
        vm.startPrank(owner);
        IRealNft(COLLECTION).setApprovalForAll(address(g), true);
        posId = g.deposit{value: backing}(COLLECTION, id);
        vm.stopPrank();
    }

    /// Advance the etched L2 clock past the reveal. The hash is the mock's own —
    /// which is the honest shape here: this harness tests the collection, and the
    /// chain's real `arbBlockHash` was measured against the live node instead
    /// (see the header).
    function _reveal(uint256 drawId) internal returns (bytes32 bh) {
        (,,,,, uint64 revealBlock,,,) = g.draws(drawId);
        arb.setL2Block(uint256(revealBlock) + 1);
        bh = arb.arbBlockHash(uint256(revealBlock));
    }

    function _assertSolvent() internal view {
        assertGe(address(g).balance, g.accountedEth(), "insolvent against a real collection");
    }

    // ------------------------------------------------------------------ tests

    /// The compatibility gate, on live bytecode. Everything below depends on it,
    /// and it is the check to re-run before curating ANY new collection.
    function test_fork_theRealCollectionIsCompatible() public {
        if (skipRun) {
            emit log("SKIP: set RH_FORK_URL to run the gacha fork test");
            return;
        }
        assertGt(COLLECTION.code.length, 0, "collection has no code on 4663");
        assertTrue(IRealNft(COLLECTION).supportsInterface(ERC721_ID), "not an ERC-721");

        (address ownerA, uint256 idA,,) = _twoRealHolders();
        assertGt(IRealNft(COLLECTION).balanceOf(ownerA), 0, "discovered holder holds nothing");

        uint256 posId = _deposit(ownerA, idA, MIN_BACKING);
        // The pull worked against real bytecode: escrowed, tracked, and weighed.
        assertEq(IRealNft(COLLECTION).ownerOf(idA), address(g), "real token not escrowed");
        assertEq(g.positionOfToken(COLLECTION, idA), posId, "escrow record missing");
        assertEq(g.treeRoot(COLLECTION), 1e36 / MIN_BACKING, "weight not in the tree");
        assertEq(g.fee(COLLECTION), MIN_BACKING * 11_000 / 10_000, "price off a single real position");
        _assertSolvent();
    }

    /// The whole loop against real bytecode and real holders: two positions in,
    /// a paid draw, a settlement, and a real token in the buyer's wallet.
    function test_fork_aRealDrawEndToEnd() public {
        if (skipRun) {
            emit log("SKIP: set RH_FORK_URL to run the gacha fork test");
            return;
        }
        (address ownerA, uint256 idA, address ownerB, uint256 idB) = _twoRealHolders();
        uint256 posA = _deposit(ownerA, idA, MIN_BACKING);
        uint256 posB = _deposit(ownerB, idB, 1 ether);

        uint256 fee = g.fee(COLLECTION);
        assertGt(fee, 0, "no price on a two-position real pool");
        vm.prank(buyer);
        uint256 drawId = g.request{value: fee}(COLLECTION, bytes32("fork"), 0, 2, 0);

        vm.expectRevert(ScryGacha.TooEarly.selector);
        g.settle(drawId);

        _reveal(drawId);
        uint256 drawn = g.settle(drawId); // permissionless: the test is a bystander
        assertTrue(drawn == posA || drawn == posB, "drew something that is not in the pool");

        // Whoever was drawn, the buyer takes the token, and it moves on the real
        // collection. This is the line a filtered or paused collection breaks.
        (,,, uint256 tokenId,,,,,,,,,,) = g.positions(drawn);
        vm.prank(buyer);
        g.keepNft(drawn);
        assertEq(IRealNft(COLLECTION).ownerOf(tokenId), buyer, "real token did not reach the buyer");

        // And the depositor's ETH is waiting, less the posted cut.
        assertGt(g.owed(_depositorOf(drawn)), 0, "depositor was not credited");
        _assertSolvent();
    }

    /// The other settlement branch: the buyer hands a real token back and takes
    /// the standing bid, funded only by that position's own escrow.
    function test_fork_theBidPathReturnsARealTokenToItsDepositor() public {
        if (skipRun) {
            emit log("SKIP: set RH_FORK_URL to run the gacha fork test");
            return;
        }
        (address ownerA, uint256 idA, address ownerB, uint256 idB) = _twoRealHolders();
        _deposit(ownerA, idA, MIN_BACKING);
        _deposit(ownerB, idB, MIN_BACKING);

        uint256 fee = g.fee(COLLECTION);
        vm.prank(buyer);
        uint256 drawId = g.request{value: fee}(COLLECTION, bytes32("bid"), 0, 2, 0);
        _reveal(drawId);
        uint256 drawn = g.settle(drawId);

        (, address dep,, uint256 tokenId, uint256 backing,,,,,,,,,) = g.positions(drawn);
        vm.prank(buyer);
        g.takeBid(drawn);

        assertEq(g.owed(buyer), backing * BID_BPS / 10_000, "bid payout wrong");
        assertEq(IRealNft(COLLECTION).ownerOf(tokenId), dep, "real token did not go home");
        _assertSolvent();
    }

    /// §4.3 asked whether a pull costs a third of the object it moves. Here is
    /// the answer in real gas against real bytecode, rather than an estimate.
    function test_fork_whatOnePullActuallyCosts() public {
        if (skipRun) {
            emit log("SKIP: set RH_FORK_URL to run the gacha fork test");
            return;
        }
        (address ownerA, uint256 idA, address ownerB, uint256 idB) = _twoRealHolders();
        uint256 gDeposit = gasleft();
        _deposit(ownerA, idA, MIN_BACKING);
        gDeposit -= gasleft();
        _deposit(ownerB, idB, 1 ether);

        uint256 fee = g.fee(COLLECTION);
        uint256 gRequest = gasleft();
        vm.prank(buyer);
        uint256 drawId = g.request{value: fee}(COLLECTION, bytes32("gas"), 0, 0, 0);
        gRequest -= gasleft();

        _reveal(drawId);
        uint256 gSettle = gasleft();
        uint256 drawn = g.settle(drawId);
        gSettle -= gasleft();

        uint256 gKeep = gasleft();
        vm.prank(buyer);
        g.keepNft(drawn);
        gKeep -= gasleft();

        emit log_named_uint("gas deposit ", gDeposit);
        emit log_named_uint("gas request ", gRequest);
        emit log_named_uint("gas settle  ", gSettle);
        emit log_named_uint("gas keepNft ", gKeep);
        uint256 pull = gRequest + gSettle + gKeep;
        emit log_named_uint("gas a whole pull (request+settle+keep)", pull);
        // At the 0.05 gwei base fee measured on 4663, wei = gas * 5e7.
        emit log_named_uint("wei at 0.05 gwei", pull * 5e7);
        assertLt(pull, 1_500_000, "a pull got expensive - re-read GACHA.md 4.3 before shipping");
    }

    function _depositorOf(uint256 posId) internal view returns (address dep) {
        (, dep,,,,,,,,,,,,) = g.positions(posId);
    }
}
