// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import {
    IUniswapV3Factory,
    INonfungiblePositionManager,
    IUniswapV3PoolSeed,
    IERC20Seed
} from "../src/interfaces/IUniswapV3.sol";

/// @title SeedSpoilsUniswapV3 — put the spoils on canonical Uniswap v3 (three pools)
/// @notice The earned, elastic-but-budgeted spoils (OBOL/MYRRH, `SpoilsToken.sol`) get a REAL market on
///         RH-Chain's canonical Uniswap — not the playground `ScryGarden` (a toy
///         AMM with a manipulable spot price; "never point real value at it").
///         This is POOLS.md §3 step 4, run under the exact Pool-1 discipline
///         (§2): v3, 1% tier, full-range, small first mint, initialize + mint in
///         ONE atomic multicall so no bot can trade the create/mint gap.
///
///         Two hard lines this script keeps in code:
///           1. The LP position NFT is minted to the OPERATOR wallet (default:
///              the broadcaster; override with LP_RECIPIENT). NEVER the meter,
///              the agency, or the facilitator gas key (POOLS.md §2.5). scry
///              holds no keys; a pool doesn't change that.
///           2. Nothing here prices a measurement. A pool gives the spoils a
///              price; it never moves a meter number, a mint, odds, or a payout
///              (POOLS.md §0/§5). Spoils emission stays participation-only.
///
///         The price is not guessed on-chain: `*_SQRTP` (the sqrtPriceX96 for the
///         address-sorted pair) and the two raw deposit amounts come from the
///         tested helper `script/seed_spoils_uniswap.py`, which computes them at
///         the LIVE cross rate at execution time (POOLS.md §2.2). Recompute right
///         before broadcasting — a stale price is a gift to the first arber.
///         The script CROSS-CHECKS the two (sqrtP vs amount ratio, within
///         SLIP_BPS) and refuses to broadcast when they disagree — an inverted
///         or mismatched launch price fails closed instead of shipping.
///
///         PREREQUISITES (do not skip; the repo says this in four places):
///           - `forge test -vv` GREEN on a real machine first.
///           - Verify V3_FACTORY / V3_NPM / SCRY / OBOL / MYRRH on 4663 with
///             `cast` per POOLS.md §4.1 — an address table is never the authority.
///           - OBOL/MYRRH already deployed (DeploySpoils.s.sol) and, ideally, the
///             minter rotated to the merkle distributor; seed from real
///             ledger-backed balances, publicly noted.
///
///   # amounts + sqrtP come from the helper, computed at the live cross rate:
///   #   python3 script/seed_spoils_uniswap.py --obol-price <SCRY_per_OBOL> ...
///   export PRIVATE_KEY=0x…                 # operator; LP NFT lands here by default
///   export SCRY_TOKEN=0xDa2a4b23459e9ca88183e990802be644AcA7C4B0
///   export OBOL_TOKEN=0x…  MYRRH_TOKEN=0x…
///   export OBOL_SQRTP=…  OBOL_BASE_RAW=…  OBOL_QUOTE_RAW=…
///   export MYRRH_SQRTP=… MYRRH_BASE_RAW=… MYRRH_QUOTE_RAW=…
///   # OPTIONAL third pool (MYRRH priced in OBOL, no SCRY). Unset = skipped,
///   # so a two-pool run is still exactly one command:
///   export MYRRH_OBOL_SQRTP=… MYRRH_OBOL_BASE_RAW=… MYRRH_OBOL_QUOTE_RAW=…
///   forge script script/SeedSpoilsUniswapV3.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast --ledger
///
/// ── WHY WE OPEN THE THIRD POOL (MYRRH/OBOL), 2026-07-25 ──────────────────
/// Not because we want a third market — because we cannot stop one existing.
/// Uniswap is permissionless: the moment OBOL and MYRRH both trade against
/// SCRY, those two prices IMPLY a MYRRH/OBOL rate, and opening the pool that
/// closes that triangle is profitable for whoever does it first. So the only
/// real choice is whether that stranger or we set the opening tick.
///
/// It matters because a v3 pool's first mint IS its price — there is no
/// discovery. A stranger who opens MYRRH/OBOL away from the implied cross
/// takes the difference on the first arb, and that difference comes out of the
/// two protocol-owned SCRY positions this same script mints. Opening it
/// ourselves at the consistent rate (50 ÷ 10 = 5 OBOL per MYRRH) is a DEFENSIVE
/// act, and it costs no SCRY: both sides are house-minted.
///
/// The house also market-makes across these venues; the posted rule and the
/// disclosure live in LAUNCH-DECISIONS.md §the house as market maker. Nothing
/// here touches a measurement — a pool prices a game coin, never a reading.
///
/// ⚠ Broadcast is a deliberate human act with real value. Start SMALL; scale
///   only after a day of sane arb between this pool and the incumbent
///   SCRY/WETH pool (POOLS.md §2.2, §4.3).
contract SeedSpoilsUniswapV3 is Script {
    // POOLS.md §1 verified-live addresses on RH-Chain (4663), 2026-07-19.
    // Overridable via env; ALWAYS re-verify on-chain before broadcasting (§4.1).
    // Public so the fork test asserts THESE literals against live chain state.
    address public constant DEFAULT_FACTORY = 0x1f7d7550B1b028f7571E69A784071F0205FD2EfA;
    address public constant DEFAULT_NPM = 0x73991a25C818Bf1f1128dEAaB1492D45638DE0D3;
    address public constant DEFAULT_SCRY = 0xDa2a4b23459e9ca88183e990802be644AcA7C4B0;

    uint24 constant DEFAULT_FEE = 10000; // 1% tier — matches the incumbent SCRY/WETH pool
    int24 constant MAX_TICK = 887272;

    /// One pool's numbers, from the helper. token == address(0) = skip.
    /// `base` is the token being priced; `quote` is whatever it is priced IN —
    /// SCRY for the two money pools, OBOL for the third (house-minted) one.
    struct PoolSeed {
        address token; // the base side (OBOL / MYRRH)
        uint160 sqrtP;
        uint256 baseRaw; // base side
        uint256 quoteRaw; // quote side
    }

    /// Everything the seed flow needs, as plain values. `run()` fills this
    /// from env for the CLI; tests build it in memory and call `runWith`
    /// directly — vm.setEnv is process-global and races under forge's
    /// parallel test execution, so tests must never round-trip through env.
    struct SeedInputs {
        uint256 pk;
        address factory;
        address npm;
        address scry;
        uint24 fee;
        uint256 slipBps;
        uint256 deadlineSecs;
        address recipient;
        PoolSeed obol;
        PoolSeed myrrh;
        /// The THIRD pool: MYRRH priced in OBOL, both sides house-minted, no
        /// SCRY at all. We open it deliberately rather than leave it to a
        /// stranger — see the header note "WHY WE OPEN THE THIRD POOL".
        PoolSeed myrrhObol;
        /// OBOL's address as the third pool's QUOTE side. Deliberately its own
        /// field: `obol.token` doubles as pool 1's skip sentinel, so reusing it
        /// here would make "seed the third pool" imply "seed the first" — and
        /// the canary discipline (seed small, read back, top up) needs each
        /// pool runnable on its own.
        address obolToken;
    }

    function run() external {
        SeedInputs memory inp;
        inp.pk = vm.envUint("PRIVATE_KEY");
        inp.factory = vm.envOr("V3_FACTORY", DEFAULT_FACTORY);
        inp.npm = vm.envOr("V3_NPM", DEFAULT_NPM);
        inp.scry = vm.envOr("SCRY_TOKEN", DEFAULT_SCRY);
        inp.fee = uint24(vm.envOr("FEE", uint256(DEFAULT_FEE)));
        inp.slipBps = vm.envOr("SLIP_BPS", uint256(100)); // 1% default
        inp.deadlineSecs = vm.envOr("DEADLINE_SECS", uint256(900));
        inp.recipient = vm.envOr("LP_RECIPIENT", vm.addr(inp.pk));
        // The TOKEN addresses are just addresses; the *_SQRTP is what says
        // "seed this pool". That split is what lets the third pool run on its
        // own (it needs OBOL's address as a quote, not OBOL's own pool).
        address obolToken = vm.envOr("OBOL_TOKEN", address(0));
        address myrrhToken = vm.envOr("MYRRH_TOKEN", address(0));
        inp.obolToken = obolToken;

        if (vm.envOr("OBOL_SQRTP", uint256(0)) != 0) {
            require(obolToken != address(0), "OBOL_SQRTP set but OBOL_TOKEN is not");
            inp.obol.token = obolToken;
            inp.obol.sqrtP = uint160(vm.envUint("OBOL_SQRTP"));
            inp.obol.baseRaw = vm.envUint("OBOL_BASE_RAW");
            inp.obol.quoteRaw = vm.envUint("OBOL_QUOTE_RAW");
        }
        if (vm.envOr("MYRRH_SQRTP", uint256(0)) != 0) {
            require(myrrhToken != address(0), "MYRRH_SQRTP set but MYRRH_TOKEN is not");
            inp.myrrh.token = myrrhToken;
            inp.myrrh.sqrtP = uint160(vm.envUint("MYRRH_SQRTP"));
            inp.myrrh.baseRaw = vm.envUint("MYRRH_BASE_RAW");
            inp.myrrh.quoteRaw = vm.envUint("MYRRH_QUOTE_RAW");
        }
        // The third pool: base = MYRRH, quote = OBOL, no SCRY leg at all.
        if (vm.envOr("MYRRH_OBOL_SQRTP", uint256(0)) != 0) {
            require(myrrhToken != address(0) && obolToken != address(0), "third pool needs BOTH token addresses");
            inp.myrrhObol.token = myrrhToken;
            inp.myrrhObol.sqrtP = uint160(vm.envUint("MYRRH_OBOL_SQRTP"));
            inp.myrrhObol.baseRaw = vm.envUint("MYRRH_OBOL_BASE_RAW");
            inp.myrrhObol.quoteRaw = vm.envUint("MYRRH_OBOL_QUOTE_RAW");
        }
        runWith(inp);
    }

    function runWith(SeedInputs memory inp) public {
        IUniswapV3Factory factory = IUniswapV3Factory(inp.factory);
        INonfungiblePositionManager npm = INonfungiblePositionManager(inp.npm);
        uint256 deadline = block.timestamp + inp.deadlineSecs;

        // Sanity that fails BEFORE any value moves.
        require(npm.factory() == address(factory), "NPM/factory mismatch - re-verify per POOLS.md 4.1");
        int24 spacing = factory.feeAmountTickSpacing(inp.fee);
        require(spacing != 0, "fee tier not enabled on this factory");
        require(inp.recipient != address(0), "zero LP recipient");
        require(inp.slipBps <= 2000, "slippage guard too loose (>20%)");
        // POOLS.md §2.5: the LP NFT is the operator's, never a service/contract.
        // A contract recipient (e.g. a multisig) is allowed but flagged loudly so
        // it is a deliberate choice, never the meter/agency/facilitator key.
        if (inp.recipient.code.length != 0) {
            console2.log(
                "WARNING: LP recipient is a CONTRACT - confirm this is your own custody (Safe), NOT the meter/agency/facilitator:"
            );
            console2.log("  recipient", inp.recipient);
        }

        (int24 tickLower, int24 tickUpper) = _fullRangeTicks(spacing);

        vm.startBroadcast(inp.pk);

        _seedOne(
            npm,
            factory,
            "OBOL",
            inp.obol,
            inp.scry,
            inp.fee,
            tickLower,
            tickUpper,
            inp.slipBps,
            inp.recipient,
            deadline
        );
        _seedOne(
            npm,
            factory,
            "MYRRH",
            inp.myrrh,
            inp.scry,
            inp.fee,
            tickLower,
            tickUpper,
            inp.slipBps,
            inp.recipient,
            deadline
        );
        // The third pool, priced in OBOL rather than SCRY. `_seedOne`'s quote
        // parameter was never SCRY-specific — it is just "the other side" —
        // so this needs no new machinery, only the OBOL address in that slot.
        _seedOne(
            npm,
            factory,
            "MYRRH/OBOL",
            inp.myrrhObol,
            inp.obolToken,
            inp.fee,
            tickLower,
            tickUpper,
            inp.slipBps,
            inp.recipient,
            deadline
        );

        vm.stopBroadcast();
    }

    /// One spoils pool: sort the pair, map amounts to token0/token1, then land
    /// create+initialize and mint in a single atomic multicall.
    function _seedOne(
        INonfungiblePositionManager npm,
        IUniswapV3Factory factory,
        string memory tag,
        PoolSeed memory seed,
        address quote, // whatever the base is priced IN: SCRY, or OBOL for the third pool
        uint24 fee,
        int24 tickLower,
        int24 tickUpper,
        uint256 slipBps,
        address recipient,
        uint256 deadline
    ) internal {
        address spoil = seed.token;
        if (spoil == address(0)) {
            console2.log(string.concat(tag, ": token address unset, skipping"));
            return;
        }
        require(quote != address(0), string.concat(tag, ": quote token unset (need the other side's address)"));
        require(spoil != quote, "base == quote");

        // Per-pool numbers from the helper (computed at the live cross rate).
        uint160 sqrtP = seed.sqrtP;
        uint256 baseRaw = seed.baseRaw; // spoil side
        uint256 quoteRaw = seed.quoteRaw; // SCRY side
        // Real TickMath accepts MIN_SQRT_RATIO <= sqrtP < MAX_SQRT_RATIO — the
        // upper bound is STRICT. Mirror it exactly so a boundary value fails
        // here, before any value moves, not deep inside the pool.
        require(
            sqrtP >= 4295128739 && sqrtP < 1461446703485210103287273052203988822378723970342,
            "sqrtP out of v3 range - recompute with the helper"
        );
        require(baseRaw > 0 && quoteRaw > 0, "zero seed amount");

        // v3 pairs are keyed on the address-sorted (token0, token1); the mint
        // amounts must follow the same order, and sqrtP is token1-per-token0.
        (address token0, address token1, uint256 amount0, uint256 amount1) =
            spoil < quote ? (spoil, quote, baseRaw, quoteRaw) : (quote, spoil, quoteRaw, baseRaw);

        // ── The launch-price tripwire (audit 2026-07-22, TEST-AUDIT.md) ─────
        // sqrtP and the two amounts EACH encode a price for the same sorted
        // pair. If they disagree (the classic failure: an inverted price, or
        // base/quote swapped), the pool initializes at one price while the
        // mint deposits at another — and the gap is a free arb for the first
        // taker. Recompute BOTH in the token1-per-token0 orientation the sort
        // above produced (amount1/amount0 — which is quoteRaw/baseRaw when the
        // spoil is token0, and baseRaw/quoteRaw when SCRY is) and refuse to
        // broadcast unless they agree within SLIP_BPS. Decimals cancel: both
        // sides are raw-vs-raw. sqrtP < 2^128 keeps sqrtP*sqrtP inside uint256
        // (any realistic price is far below it; the helper never emits above).
        require(sqrtP < 2 ** 128, "sqrtP too extreme for consistency check - recompute with the helper");
        uint256 impliedPriceX96 = (uint256(sqrtP) * uint256(sqrtP)) / 2 ** 96; // token1-raw per token0-raw, X96
        require(impliedPriceX96 > 0, "sqrtP too extreme for consistency check - recompute with the helper");
        // checked mul: an absurd amount (>= 2^160 raw) fails closed rather than
        // silently truncating; amount0 > 0 was established above.
        uint256 amountPriceX96 = (amount1 * (2 ** 96)) / amount0; // same orientation, from the amounts
        uint256 priceDiff =
            impliedPriceX96 > amountPriceX96 ? impliedPriceX96 - amountPriceX96 : amountPriceX96 - impliedPriceX96;
        require(
            priceDiff * 10_000 <= amountPriceX96 * slipBps,
            "sqrtP inconsistent with amounts - price would be wrong on-chain"
        );

        // ── The LIVE-price check (TEST-AUDIT.md §Standing notes) ────────────
        // The tripwire above is env-vs-env: it proves `sqrtP` agrees with the
        // amounts, and says nothing about the pool those numbers are about to
        // meet. On a FIRST seed that is the whole story — the first mint IS the
        // price. On a RE-RUN it is not: `createAndInitializePoolIfNecessary` is
        // a no-op against a pool that already exists, so the mint lands at
        // whatever price that pool trades at, not the one just validated. That
        // re-run is the top-up path — canary small, read back, top up — and it
        // is also what a squatter buys by opening the pair first.
        //
        // The `amount*Min` guards below already make this fail closed — but
        // they fail as an opaque revert from inside an NPM multicall. Read the
        // live price and refuse HERE, naming both numbers, so the operator is
        // told to recompute rather than left to guess.
        address existing = factory.getPool(token0, token1, fee);
        if (existing != address(0)) {
            console2.log(string.concat(tag, ": pool already exists, adding liquidity to it:"));
            console2.log("  pool", existing);
            console2.log("  seed sqrtPriceX96", uint256(sqrtP));
            // staticcall rather than a typed call: an address that cannot
            // answer `slot0()` must not turn a good run into a revert with no
            // data. The amount mins below stay the hard guard either way.
            (bool ok, bytes memory ret) = existing.staticcall(abi.encodeCall(IUniswapV3PoolSeed.slot0, ()));
            if (!ok || ret.length < 224) {
                // Loudly, never silently — a check that quietly did not run is
                // the vacuity this repo keeps having to fix.
                console2.log("  !! could not read slot0 - the LIVE-price check did NOT run for this pool;");
                console2.log("  !! amount0Min/amount1Min are the only thing left stopping a wrong price");
            } else {
                (uint160 liveSqrtP,,,,,,) = abi.decode(ret, (uint160, int24, uint16, uint16, uint16, uint8, bool));
                console2.log("  live sqrtPriceX96", uint256(liveSqrtP));
                // A zero means created-but-never-initialized: the multicall
                // below initializes it at `sqrtP`, so there is no live price to
                // disagree with and refusing would block a good first seed.
                if (liveSqrtP != 0) {
                    uint256 sqrtDiff = liveSqrtP > sqrtP ? uint256(liveSqrtP) - sqrtP : uint256(sqrtP) - liveSqrtP;
                    // Compared on sqrtP, not on price — deliberately the
                    // TIGHTER band, since a price moves about twice a sqrtP
                    // move. So this refuses slightly before the amount mins
                    // would, which is the direction that keeps the diagnosis
                    // readable instead of opaque.
                    require(
                        sqrtDiff * 10_000 <= uint256(liveSqrtP) * slipBps,
                        "pool already trades at a different price - recompute the amounts at the LIVE price"
                    );
                }
            }
        }

        // Approve exactly what we intend to deposit (no lingering infinite allowance).
        require(IERC20Seed(token0).approve(address(npm), amount0), "approve0");
        require(IERC20Seed(token1).approve(address(npm), amount1), "approve1");

        INonfungiblePositionManager.MintParams memory params = INonfungiblePositionManager.MintParams({
            token0: token0,
            token1: token1,
            fee: fee,
            tickLower: tickLower,
            tickUpper: tickUpper,
            amount0Desired: amount0,
            amount1Desired: amount1,
            amount0Min: (amount0 * (10_000 - slipBps)) / 10_000,
            amount1Min: (amount1 * (10_000 - slipBps)) / 10_000,
            recipient: recipient,
            deadline: deadline
        });

        bytes[] memory calls = new bytes[](2);
        calls[0] = abi.encodeWithSelector(
            INonfungiblePositionManager.createAndInitializePoolIfNecessary.selector, token0, token1, fee, sqrtP
        );
        calls[1] = abi.encodeWithSelector(INonfungiblePositionManager.mint.selector, params);

        npm.multicall(calls);

        console2.log(string.concat(tag, " v3 pool seeded:"));
        console2.log("  pool ", factory.getPool(token0, token1, fee));
        console2.log("  LP NFT recipient", recipient);
    }

    /// Full-range ticks rounded IN to the tier's spacing (POOLS.md §2.3: start
    /// full-range — maintenance-free, always quoting both sides).
    function _fullRangeTicks(int24 spacing) internal pure returns (int24 lower, int24 upper) {
        int24 usable = (MAX_TICK / spacing) * spacing; // e.g. 887200 for spacing 200
        return (-usable, usable);
    }
}
