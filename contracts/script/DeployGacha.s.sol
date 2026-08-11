// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/ScryGacha.sol";

/// Deploy the NFT gacha (`GACHA.md`, forked from FWA — see
/// `contracts/reference/fwa/README.md` for the port table).
///
///   export PRIVATE_KEY=0x...
///   export GACHA_FEE_SINK=0x...              # where the house's ETH cut is swept
///   # optional:
///   # export GACHA_DRAW_DELAY=100            # blocks to the reveal; IMMUTABLE
///   # export GACHA_CURATOR=0x...             # may open/close pools, never money
///   # export GACHA_POOL=0x539CdD042c2f3d93EbC5BE7DfFf0c79F3B4fAbF0  # StonkBrokers
///   # export GACHA_MIN_BACKING=10000000000000000   # 0.01 ETH — the measured floor
///   # export GACHA_SURCHARGE_BPS=1000        # 10% over EV
///   # export GACHA_BID_BPS=8500              # the standing bid, FIXED per pool
///   # export GACHA_TOLL_COIN=0x...           # SCRY — the toll rail's denomination
///   # export GACHA_TOLL_RATE=0               # coin wei per 1e18 of the ETH quote
///   # export GACHA_TOLL_BURN_BPS=0           # slice of every toll set aside to burn
///
///   forge script script/DeployGacha.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// **The three numbers, and where they came from.** None of them is a taste.
///
/// - `GACHA_DRAW_DELAY = 100` **L2** blocks. At 4663's 101ms blocks that is ~10s —
///   a "siege window, not an atomic snipe". It is immutable in the
///   constructor because it is the entire grind-resistance of the draw, and the
///   settle window that follows it is the 256 L2 blocks `arbBlockHash` answers for
///   (~26s). Shortening it later would be a change to the fairness of the game,
///   which is why no setter exists.
/// - `GACHA_MIN_BACKING = 0.01 ETH`. Measured twice from different windows:
///   ~p82 of live Seaport sales when `GACHA.md` §4.4 chose it, **p80** when the
///   contract was written (`python3 meter/gacha_shelf.py`). It is also FWA's own
///   default, arrived at independently. Below it the harmonic mean is a free
///   griefing vector on a dust chain (§4.2), so the contract enforces a floor of
///   0.01 ETH and this knob can only go up.
/// - `GACHA_BID_BPS = 8500`. FWA's number, and the contract fixes it per pool at
///   open: depositors price their backing against the buyback they were
///   promised, so it is not something a later operator gets to revise.
///
/// **The toll rail ships DISARMED, and that is the deploy plan, not a default**
/// (`GACHA.md` §10d/§10e). The toll asset is decided — `SCRY` — but the price is
/// a posted number and posting it is an operator act, so `GACHA_TOLL_RATE`
/// defaults to **0**, which is the ETH rail this contract was measured on.
/// Posting the coin without a rate is deliberately a no-op: arming takes two
/// posts, and the coin can be posted early because it may only ever change while
/// the rail is EMPTY. Once anything is outstanding in coin it is welded, so post
/// the right one or leave it zero and set it later.
///
/// ⚠ **`GACHA_TOLL_RATE` is toll-coin wei per 1e18 wei (one ETH) of the quoted
/// fee — an exchange rate, not a fee.** It is not read from an oracle and never
/// will be: the town posts prices and retunes them. Drift from market is
/// expected and is the operator's to correct.
///
/// **What this script does NOT do.** It opens at most one pool and it does not
/// deposit, fund, or enable anything else. There is nothing to enable: a pool
/// with no positions quotes no price and refuses draws by construction, so the
/// "loading phase" flag FWA needs has no counterpart here.
///
/// **THE FIRST COLLECTION IS DECIDED: StonkBrokers,**
/// `0x539CdD042c2f3d93EbC5BE7DfFf0c79F3B4fAbF0` (operator, 2026-07-27 —
/// `SENTENCES.md`, detail in `GACHA.md` §8a). That is the address `GACHA_POOL`
/// takes. It is not a taste: it is the collection `test/ScryGachaFork.t.sol`
/// already drives the whole loop against on a fork of 4663 — real bytecode, real
/// holders, standard ERC-721, no operator filter, no pause hook — so the first
/// pool is the one case proven end-to-end live rather than the first one tried.
/// Verified the same day: ERC-721, 529 holders, 4,444 supply.
///
/// ⚠ Whatever address goes here, `openPool` only checks that it HAS CODE. It
/// cannot tell an ERC-721 from anything else, and a pool cannot be closed to
/// nothing — `blockCollection` is one-way. Read the address back off the
/// explorer before broadcasting; a typo here is a permanent dead pool.
///
/// Curation is the product surface (`GACHA.md` §5b), so run this with
/// `GACHA_CURATOR` pointed at whatever will decide which collections get a
/// floor — the curator can open and close pools and can never touch escrow, a
/// fee, or a payout. Blocking a collection stays with the owner and is one-way.
///
/// `forge test` green before any broadcast. Always.
contract DeployGacha is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(pk);

        address sink = vm.envAddress("GACHA_FEE_SINK");
        // ⚠ ONE address serves BOTH sweeps: `sweepHouse` sends native ETH and
        // `sweepHouseToll` sends the toll coin. `ScryFeeSplitter` — the obvious
        // thing to point this at, and what GACHA.md §10d called "zero new
        // machinery" — has NO `receive()` and NO `fallback()`, so a plain ETH
        // send to it reverts. Point `feeSink` there and the SCRY sweep works
        // while the ETH sweep is refused forever (recoverable only by
        // `setFeeSink`, since `houseOwed` just keeps accruing).
        //
        // A zero-value call with empty calldata reverts against a contract with
        // neither hook, so this is a real probe and not a guess. EOAs pass.
        (bool takesEth,) = sink.call{value: 0}("");
        require(takesEth, "GACHA_FEE_SINK cannot receive ETH - sweepHouse would revert forever");
        uint256 delay = vm.envOr("GACHA_DRAW_DELAY", uint256(100));
        address curator = vm.envOr("GACHA_CURATOR", address(0));
        address pool = vm.envOr("GACHA_POOL", address(0));
        uint256 minBacking = vm.envOr("GACHA_MIN_BACKING", uint256(0.01 ether));
        uint256 surchargeBps = vm.envOr("GACHA_SURCHARGE_BPS", uint256(1_000));
        uint256 bidBps = vm.envOr("GACHA_BID_BPS", uint256(8_500));
        address tollCoin = vm.envOr("GACHA_TOLL_COIN", address(0));
        uint256 tollRate = vm.envOr("GACHA_TOLL_RATE", uint256(0));
        uint256 tollBurnBps = vm.envOr("GACHA_TOLL_BURN_BPS", uint256(0));

        console.log("deployer     ", deployer);
        console.log("fee sink     ", sink);
        console.log("draw delay   ", delay, "blocks (immutable)");

        vm.startBroadcast(pk);
        ScryGacha gacha = new ScryGacha(delay, sink);
        if (curator != address(0)) gacha.setCurator(curator);
        if (tollCoin != address(0)) gacha.setToll(tollCoin, tollRate);
        if (tollBurnBps != 0) gacha.setTollBurnBps(tollBurnBps);
        if (pool != address(0)) gacha.openPool(pool, minBacking, surchargeBps, bidBps);
        vm.stopBroadcast();

        console.log("ScryGacha    ", address(gacha));
        if (curator != address(0)) console.log("curator      ", curator);
        // Say which rail this pool is actually selling on. A deploy that posted a
        // coin and forgot the rate is on the ETH rail, and the log must not read
        // as though a toll were live.
        if (tollRate != 0) {
            console.log("toll coin    ", tollCoin);
            console.log("toll rate    ", tollRate, "coin wei per 1e18 of the ETH quote");
            console.log("toll burn bps", tollBurnBps);
            console.log("PULLS ARE CHARGED IN THE TOLL COIN. Buyers need an allowance, not msg.value.");
        } else {
            console.log("toll         DISARMED - pulls are charged in native ETH");
            if (tollCoin != address(0)) console.log("  (coin posted, rate 0: arm it with setToll(coin, rate))");
        }
        if (pool != address(0)) {
            console.log("pool         ", pool);
            console.log("min backing  ", minBacking);
            console.log("surcharge bps", surchargeBps);
            console.log("bid bps      ", bidBps);
        } else {
            console.log("no pool opened - openPool(collection, minBacking, surchargeBps, bidBps) when curated");
        }
        console.log("");
        console.log("next: the Book. GET /gacha/book?collection=<addr> publishes the odds for this pool,");
        console.log("free, on the same terms as /barrow/book. It ships with the pool, not after it.");
    }
}
