// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import {SeedSpoilsUniswapV3} from "../script/SeedSpoilsUniswapV3.s.sol";
import {INonfungiblePositionManager, IUniswapV3Factory} from "../src/interfaces/IUniswapV3.sol";
import {SpoilsToken} from "../src/SpoilsToken.sol";

/// Minimal 18-dp ERC-20 with the reads the seed flow calls (decimals/symbol/
/// approve/allowance). Stands in for SCRY.
contract MockToken {
    string public symbol;
    uint8 public constant decimals = 18;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(string memory s, address to, uint256 amt) {
        symbol = s;
        balanceOf[to] = amt;
    }

    function approve(address sp, uint256 a) external returns (bool) {
        allowance[msg.sender][sp] = a;
        return true;
    }

    function transfer(address to, uint256 a) external returns (bool) {
        balanceOf[msg.sender] -= a;
        balanceOf[to] += a;
        return true;
    }

    function transferFrom(address f, address to, uint256 a) external returns (bool) {
        allowance[f][msg.sender] -= a;
        balanceOf[f] -= a;
        balanceOf[to] += a;
        return true;
    }
}

/// Records what the script asks it to do; multicall delegatecalls into self so
/// state writes from initialize+mint persist, exactly like the real NPM.
///
/// Post-audit (TEST-AUDIT.md 2026-07-22) it also models the real NPM's ONE
/// economic rail: fills are priced at the POOL's sqrtP (full-range
/// approximation — the tick-boundary tails are ~2^-64 and irrelevant), the
/// short side caps liquidity, the long side scales down, and
/// "Price slippage check" fires when a scaled fill undercuts the caller's
/// mins. A price-inconsistent seed can no longer pass this mock green.
///
/// Models exactly ONE pool (poolSqrtP is a single slot) — tests seed one
/// token per run; use a fresh MockNPM per pool.
contract MockNPM {
    address public immutable factory;
    uint256 constant Q96 = 2 ** 96;
    /// The pool's live price minting is checked against. createAndInitialize
    /// only sets it when unset — for a pre-existing pool the passed sqrtP is
    /// IGNORED, exactly like the real (idempotent) periphery helper.
    uint160 public poolSqrtP;
    // recorded initialize (the call's arguments, as passed)
    address public initT0;
    address public initT1;
    uint24 public initFee;
    uint160 public initSqrtP;
    // recorded mint
    INonfungiblePositionManager.MintParams public lastMint;
    uint256 public mintCount;
    uint256 public initCount;
    uint256 public multicallCount;

    constructor(address f) {
        factory = f;
    }

    /// test hook: a pre-existing pool with a live price (the rerun scenario).
    function setPoolSqrtP(uint160 sp) external {
        poolSqrtP = sp;
    }

    function createAndInitializePoolIfNecessary(address t0, address t1, uint24 fee, uint160 sp)
        external
        payable
        returns (address)
    {
        initT0 = t0;
        initT1 = t1;
        initFee = fee;
        initSqrtP = sp;
        initCount++;
        if (poolSqrtP == 0) poolSqrtP = sp; // an existing pool keeps its live price
        return address(uint160(uint256(keccak256(abi.encode(t0, t1, fee)))));
    }

    /// Full-range fill at the pool's own price: need1 is the token1 that pairs
    /// ALL of amount0Desired; whichever side is short caps the position and
    /// the other side scales down — then the real NPM's only economic guard.
    function mint(INonfungiblePositionManager.MintParams calldata p)
        external
        payable
        returns (uint256, uint128, uint256, uint256)
    {
        require(poolSqrtP != 0, "pool not initialized");
        uint256 priceX96 = (uint256(poolSqrtP) * uint256(poolSqrtP)) / Q96; // token1 per token0, X96
        require(priceX96 > 0, "mock price underflow");
        uint256 used0;
        uint256 used1;
        uint256 need1 = (p.amount0Desired * priceX96) / Q96;
        if (need1 <= p.amount1Desired) {
            used0 = p.amount0Desired;
            used1 = need1;
        } else {
            used1 = p.amount1Desired;
            used0 = (p.amount1Desired * Q96) / priceX96;
        }
        require(used0 >= p.amount0Min && used1 >= p.amount1Min, "Price slippage check");
        lastMint = p;
        mintCount++;
        return (1, uint128(1e18), used0, used1);
    }

    function multicall(bytes[] calldata data) external payable returns (bytes[] memory res) {
        multicallCount++;
        res = new bytes[](data.length);
        for (uint256 i; i < data.length; i++) {
            (bool ok, bytes memory r) = address(this).delegatecall(data[i]);
            if (!ok) {
                // bubble the inner revert verbatim, like the real periphery
                // Multicall — so "Price slippage check" is what callers see.
                assembly { revert(add(r, 0x20), mload(r)) }
            }
            res[i] = r;
        }
    }
}

/// A pre-existing v3 pool, for the ONLY thing the script asks one: what price
/// do you already trade at? `sqrtPriceX96 == 0` is the real "created but never
/// initialized" state, which must NOT be read as a disagreement.
contract MockPool {
    uint160 public sqrtPriceX96;

    constructor(uint160 sp) {
        sqrtPriceX96 = sp;
    }

    function slot0() external view returns (uint160, int24, uint16, uint16, uint16, uint8, bool) {
        return (sqrtPriceX96, int24(0), uint16(0), uint16(1), uint16(1), uint8(0), true);
    }
}

contract MockFactory {
    int24 public spacing;
    address public poolOverride; // address(0) = no pool yet (the default path)

    constructor(int24 s) {
        spacing = s;
    }

    function setPool(address p) external {
        poolOverride = p;
    }

    function feeAmountTickSpacing(uint24) external view returns (int24) {
        return spacing;
    }

    function getPool(address, address, uint24) external view returns (address) {
        return poolOverride;
    }
}

contract SeedSpoilsUniswapV3Test is Test {
    SeedSpoilsUniswapV3 script;
    MockFactory factory;
    MockNPM npm;
    MockToken scry;
    SpoilsToken obol;
    SpoilsToken myrrh;

    uint256 constant PK = 0xA11CE;
    address operator;

    // floor(sqrt(price) * 2^96) for the POSTED launch ratios (POOLS.md §3.6:
    // 10 SCRY per OBOL, 50 SCRY per MYRRH), both sort orientations, equal
    // 18-dp both sides — computed with the same integer math as
    // script/seed_spoils_uniswap.py: isqrt(amount1 * 2^192 // amount0).
    uint256 constant SQRTP_10_SPOIL_T0 = 250541448375047940197816948333; // price 10 (spoil = token0)
    uint256 constant SQRTP_10_SCRY_T0 = 25054144837504795036549970000; // price 0.1 (scry = token0)
    uint256 constant SQRTP_50_SPOIL_T0 = 560227709747861392881423798214; // price 50 (spoil = token0)
    uint256 constant SQRTP_50_SCRY_T0 = 11204554194957228324742324713; // price 0.02 (scry = token0)
    uint256 constant SQRTP_ONE = 79228162514264337593543950336; // price 1.0 = 2^96
    // The THIRD pool (2026-07-25): MYRRH priced in OBOL at 5 OBOL/MYRRH, which
    // is exactly the cross of the two SCRY pools (50 / 10) so the launch
    // carries no genesis arb between its own three venues.
    uint256 constant SQRTP_5_MYRRH_T0 = 177159557114295710296101716160; // price 5   (myrrh = token0)
    uint256 constant SQRTP_5_OBOL_T0 = 35431911422859142059220343232; // price 0.2 (obol  = token0)
    uint256 constant MAX_SQRT_RATIO = 1461446703485210103287273052203988822378723970342;

    // posted-ratio seed amounts (the POOLS.md §3.6 worked examples)
    uint256 constant OBOL_BASE = 5_000_000e18; // 5,000,000 OBOL
    uint256 constant OBOL_QUOTE = 50_000_000e18; // + 50M SCRY => 10 SCRY/OBOL
    uint256 constant MYRRH_BASE = 1_000_000e18; // 1,000,000 MYRRH
    uint256 constant MYRRH_QUOTE = 50_000_000e18; // + 50M SCRY => 50 SCRY/MYRRH
    uint256 constant MO_BASE = 200_000e18; // 200,000 MYRRH
    uint256 constant MO_QUOTE = 1_000_000e18; // + 1,000,000 OBOL => 5 OBOL/MYRRH

    function setUp() public {
        operator = vm.addr(PK);
        factory = new MockFactory(200); // 1% tier spacing
        npm = new MockNPM(address(factory));
        scry = new MockToken("SCRY", operator, 1_000_000e18);
        obol = new SpoilsToken("obol", "OBOL", 10_000_000e18, operator);
        myrrh = new SpoilsToken("myrrh", "MYRRH", 2_500_000e18, operator);
        // give the operator spoils to seed with
        vm.startPrank(operator);
        obol.mint(operator, OBOL_BASE);
        myrrh.mint(operator, MYRRH_BASE);
        vm.stopPrank();

        script = new SeedSpoilsUniswapV3();
    }

    // ── no env in tests (the parallel-race fix, 2026-07-23) ─────────────────
    // vm.setEnv writes are PROCESS-GLOBAL and forge runs tests in PARALLEL
    // workers sharing one process — env round-trips race between tests and the
    // failures are nondeterministic (proven on this suite's first-ever run).
    // Tests build SeedInputs in memory and call script.runWith(...) directly;
    // exactly ONE test below (test_run_reads_env_roundtrip) covers run()'s
    // env parsing, and it is this suite's only env writer.
    function _baseInputs() internal view returns (SeedSpoilsUniswapV3.SeedInputs memory inp) {
        inp.pk = PK;
        inp.factory = address(factory);
        inp.npm = address(npm);
        inp.scry = address(scry);
        inp.fee = 10000;
        inp.slipBps = 100;
        inp.deadlineSecs = 900;
        inp.recipient = operator;
        // obol/myrrh stay zeroed: token == address(0) is the skip sentinel
    }

    function _seed(address token, uint256 sqrtP, uint256 baseRaw, uint256 quoteRaw)
        internal
        pure
        returns (SeedSpoilsUniswapV3.PoolSeed memory)
    {
        return SeedSpoilsUniswapV3.PoolSeed(token, uint160(sqrtP), baseRaw, quoteRaw);
    }

    /// The sqrtP that encodes the SAME price as (baseRaw spoil, quoteRaw scry)
    /// for whichever sort order this pair lands in — the price-CONSISTENT
    /// input the pre-audit suite never used.
    function _consistentSqrtP(address spoil, uint256 scryPerSpoil) internal view returns (uint256) {
        if (scryPerSpoil == 10) return spoil < address(scry) ? SQRTP_10_SPOIL_T0 : SQRTP_10_SCRY_T0;
        if (scryPerSpoil == 50) return spoil < address(scry) ? SQRTP_50_SPOIL_T0 : SQRTP_50_SCRY_T0;
        revert("no posted sqrtP for that ratio");
    }

    /// Same idea for the third pool, whose quote side is OBOL rather than SCRY.
    function _consistentSqrtPMyrrhObol() internal view returns (uint256) {
        return address(myrrh) < address(obol) ? SQRTP_5_MYRRH_T0 : SQRTP_5_OBOL_T0;
    }

    // ── the THIRD pool: MYRRH priced in OBOL, no SCRY anywhere in it ──────
    //    We open it because we cannot stop it existing: once OBOL and MYRRH
    //    both trade against SCRY those prices IMPLY a MYRRH/OBOL rate, and
    //    closing that triangle pays whoever opens the pool first. A v3 pool's
    //    first mint IS its price, so a stranger opening it away from the cross
    //    takes the difference out of the two protocol-owned SCRY positions
    //    this same script mints. (AUDIT-2026-07-25.md / LAUNCH-DECISIONS.md)
    function test_third_pool_prices_myrrh_in_obol_with_no_scry_leg() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        // obol.token stays ZERO (pool 1 skipped); obolToken carries the address
        // the third pool needs as its quote. That split is the whole point.
        inp.obolToken = address(obol);
        inp.myrrhObol = _seed(address(myrrh), _consistentSqrtPMyrrhObol(), MO_BASE, MO_QUOTE);

        vm.startPrank(operator);
        myrrh.mint(operator, MO_BASE);
        obol.mint(operator, MO_QUOTE);
        vm.stopPrank();

        script.runWith(inp);

        assertEq(npm.initCount(), 1, "only the third pool initialized");
        (address et0, address et1, uint256 e0, uint256 e1) = address(myrrh) < address(obol)
            ? (address(myrrh), address(obol), MO_BASE, MO_QUOTE)
            : (address(obol), address(myrrh), MO_QUOTE, MO_BASE);
        assertEq(npm.initT0(), et0, "token0 sorted");
        assertEq(npm.initT1(), et1, "token1 sorted");
        assertTrue(et0 != address(scry) && et1 != address(scry), "NO SCRY leg in the third pool");

        (address t0, address t1,,,, uint256 a0, uint256 a1,,,,) = npm.lastMint();
        assertEq(t0, et0);
        assertEq(t1, et1);
        assertEq(a0, e0, "amount0 mapped to the sort");
        assertEq(a1, e1, "amount1 mapped to the sort");
    }

    /// The opening rate must be EXACTLY the cross of the other two pools
    /// (50 SCRY/MYRRH ÷ 10 SCRY/OBOL = 5 OBOL/MYRRH). If it is not, the
    /// launch ships with a genesis arb against its own positions — which is
    /// the entire reason we open this pool rather than leaving it to someone
    /// else. The consistency tripwire in the script enforces it; this pins
    /// the NUMBER, so a future edit to the posted ratios trips here.
    function test_third_pool_opens_at_the_cross_of_the_other_two() public {
        assertEq(MYRRH_QUOTE / MYRRH_BASE * 1, 50, "50 SCRY per MYRRH, as posted");
        assertEq(OBOL_QUOTE / OBOL_BASE * 1, 10, "10 SCRY per OBOL, as posted");
        assertEq(MO_QUOTE / MO_BASE, 5, "and 5 OBOL per MYRRH is exactly 50/10");

        // An INCONSISTENT sqrtP for that pair must be refused, not shipped.
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        inp.obolToken = address(obol);
        inp.myrrhObol = _seed(address(myrrh), SQRTP_ONE, MO_BASE, MO_QUOTE); // claims 1:1
        vm.startPrank(operator);
        myrrh.mint(operator, MO_BASE);
        obol.mint(operator, MO_QUOTE);
        vm.stopPrank();
        vm.expectRevert();
        script.runWith(inp);
    }

    /// Unset = skipped. A two-pool launch must stay exactly one command.
    function test_third_pool_is_opt_in_and_skips_when_unset() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        inp.obol = _seed(address(obol), _consistentSqrtP(address(obol), 10), OBOL_BASE, OBOL_QUOTE);
        // myrrhObol left zeroed
        script.runWith(inp);
        assertEq(npm.initCount(), 1, "only the OBOL pool - the third is opt-in");
    }

    // ── happy path: PRICE-CONSISTENT inputs, every MintParams field pinned ──
    function test_happy_path_price_consistent_asserts_every_mint_param() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        uint256 sqrtP = _consistentSqrtP(address(obol), 10);
        inp.obol = _seed(address(obol), sqrtP, OBOL_BASE, OBOL_QUOTE);
        uint256 expectedDeadline = vm.getBlockTimestamp() + 900;

        script.runWith(inp);

        assertEq(npm.initCount(), 1, "one pool initialized");
        assertEq(npm.mintCount(), 1, "one mint");
        assertEq(npm.multicallCount(), 1, "one atomic multicall");

        (address et0, address et1, uint256 e0, uint256 e1) = address(obol) < address(scry)
            ? (address(obol), address(scry), OBOL_BASE, OBOL_QUOTE)
            : (address(scry), address(obol), OBOL_QUOTE, OBOL_BASE);
        assertEq(npm.initT0(), et0, "token0 sorted");
        assertEq(npm.initT1(), et1, "token1 sorted");
        assertEq(uint256(npm.initFee()), 10000, "initialize fee = 1% tier");
        assertEq(uint256(npm.initSqrtP()), sqrtP, "sqrtP passthrough");

        // EVERY MintParams field (audit: token0/token1/fee/deadline included)
        (
            address t0,
            address t1,
            uint24 fee,
            int24 tl,
            int24 tu,
            uint256 a0,
            uint256 a1,
            uint256 a0min,
            uint256 a1min,
            address rcpt,
            uint256 dl
        ) = npm.lastMint();
        assertEq(t0, et0, "mint token0 = sorted token0");
        assertEq(t1, et1, "mint token1 = sorted token1");
        assertEq(uint256(fee), 10000, "mint fee = 1% tier");
        assertEq(tl, int24(-887200), "tickLower full-range for spacing 200");
        assertEq(tu, int24(887200), "tickUpper full-range for spacing 200");
        assertEq(a0, e0, "amount0 mapped to the sort");
        assertEq(a1, e1, "amount1 mapped to the sort");
        assertEq(a0min, e0 * 9900 / 10000, "amount0Min at 1% slip");
        assertEq(a1min, e1 * 9900 / 10000, "amount1Min at 1% slip");
        assertEq(rcpt, operator, "LP NFT to operator, never a service");
        assertEq(dl, expectedDeadline, "deadline = now + DEADLINE_SECS");

        // approvals were granted to the NPM for exactly the deposited amounts
        assertEq(_allowanceOf(et0), e0, "token0 approved to npm");
        assertEq(_allowanceOf(et1), e1, "token1 approved to npm");
    }

    // ── skip semantics: the zero-address sentinel is "unset" ────────────────
    function test_zero_sentinel_skips_obol_seeds_only_myrrh() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs(); // OBOL stays the zero sentinel = skipped
        uint256 sqrtP = _consistentSqrtP(address(myrrh), 50);
        inp.myrrh = _seed(address(myrrh), sqrtP, MYRRH_BASE, MYRRH_QUOTE);

        script.runWith(inp);

        assertEq(npm.mintCount(), 1, "only the set token seeded");
        assertEq(npm.initCount(), 1, "no pool created for the sentinel token");
        (address et0, address et1) =
            address(myrrh) < address(scry) ? (address(myrrh), address(scry)) : (address(scry), address(myrrh));
        assertEq(npm.initT0(), et0, "the one pool is MYRRH/SCRY (token0)");
        assertEq(npm.initT1(), et1, "the one pool is MYRRH/SCRY (token1)");
        assertEq(uint256(npm.initFee()), 10000, "initFee = 1% tier");
        assertEq(uint256(npm.initSqrtP()), sqrtP, "sqrtP passthrough");
    }

    // ── THE audit headline: an inverted launch price must fail closed ───────
    /// sqrtP encodes price 1.0 while the amounts encode 0.01 SCRY/OBOL — the
    /// EXACT inputs the pre-audit suite passed green. On-chain that seed would
    /// initialize the pool at one price and deposit at another, gifting the
    /// difference to the first arber. The script's new tripwire refuses it
    /// before any value moves — in BOTH sort orders (implied 1.0 disagrees
    /// with 0.01 and with its inverse 100 alike).
    function test_inverted_launch_price_reverts() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        inp.obol = _seed(address(obol), SQRTP_ONE, 500_000e18, 5_000e18);
        vm.expectRevert(bytes("sqrtP inconsistent with amounts - price would be wrong on-chain"));
        script.runWith(inp);
    }

    // ── sqrtP range guards, exact messages ──────────────────────────────────
    function test_below_range_sqrtP_reverts_exact_message() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        inp.obol = _seed(address(obol), 1, 500_000e18, 5_000e18); // below MIN_SQRT_RATIO
        vm.expectRevert(bytes("sqrtP out of v3 range - recompute with the helper"));
        script.runWith(inp);
    }

    /// Real TickMath's upper bound is STRICT (sqrtP < MAX_SQRT_RATIO); the
    /// script now mirrors it, so the boundary value fails in the script with a
    /// helpful message instead of deep inside the pool.
    function test_max_sqrtP_boundary_reverts_exact_message() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        inp.obol = _seed(address(obol), MAX_SQRT_RATIO, 500_000e18, 5_000e18);
        vm.expectRevert(bytes("sqrtP out of v3 range - recompute with the helper"));
        script.runWith(inp);
    }

    /// In v3 range but beyond the tripwire's overflow-safe bound (2^128, i.e.
    /// a raw price above ~1.8e19): the script refuses to certify rather than
    /// skip the check.
    function test_sqrtP_beyond_consistency_bound_reverts_exact_message() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        inp.obol = _seed(address(obol), 2 ** 128, 500_000e18, 5_000e18);
        vm.expectRevert(bytes("sqrtP too extreme for consistency check - recompute with the helper"));
        script.runWith(inp);
    }

    // ── the rerun-after-partial-success scenario ────────────────────────────
    /// The pool already EXISTS at a divergent live price (an earlier partial
    /// run, or the market moved since). createAndInitializePoolIfNecessary is
    /// idempotent — the passed sqrtP is IGNORED — and the mint must die on the
    /// NPM's own "Price slippage check" rail, which the mock now models.
    //
    // PINNED, NOT ENDORSED — see contracts/TEST-AUDIT.md: for a pre-existing
    // pool the script's tripwire only cross-checks env-vs-env; it never reads
    // the LIVE pool price, so the NPM min-amounts rail is the ONLY thing
    // standing between a rerun and a mispriced mint. This test pins that the
    // rail catches a 100x divergence at 1% mins; the audit's fork-test
    // requirement for this flow stands regardless.
    /// The tripwire is env-vs-env; this is the LIVE half (TEST-AUDIT.md
    /// §Standing notes: "a rerun against a pre-existing pool still never reads
    /// the live price"). `createAndInitializePoolIfNecessary` is a no-op on an
    /// existing pool, so a re-run — LAUNCH.md §3b step 4's top-up, or a
    /// squatter who opened the pair first — mints at THAT pool's price, not the
    /// one just validated. Refused by name, before any approval is granted.
    function test_existing_pool_divergent_live_price_is_refused_by_name() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        uint256 sqrtP = _consistentSqrtP(address(obol), 10);
        inp.obol = _seed(address(obol), sqrtP, OBOL_BASE, OBOL_QUOTE); // inputs self-consistent: tripwire passes
        factory.setPool(address(new MockPool(uint160(SQRTP_ONE)))); // live price 1.0 — 100x off
        npm.setPoolSqrtP(uint160(SQRTP_ONE));
        vm.expectRevert(bytes("pool already trades at a different price - recompute the amounts at the LIVE price"));
        script.runWith(inp);
        assertEq(npm.mintCount(), 0, "nothing was minted");
        assertEq(obol.allowance(operator, address(npm)), 0, "and nothing was approved");
    }

    /// The backstop is still the backstop. A pool address that cannot answer
    /// `slot0()` skips the live check (loudly — the script says so), and the
    /// NPM's own amount0Min/amount1Min still refuse the bad price. This is the
    /// coverage the test above used to carry, kept rather than replaced.
    function test_a_pool_with_no_readable_slot0_still_fails_closed_on_the_mins() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        uint256 sqrtP = _consistentSqrtP(address(obol), 10);
        inp.obol = _seed(address(obol), sqrtP, OBOL_BASE, OBOL_QUOTE);
        factory.setPool(address(0xBEEF)); // exists, but answers nothing
        npm.setPoolSqrtP(uint160(SQRTP_ONE));
        vm.expectRevert(bytes("Price slippage check"));
        script.runWith(inp);
    }

    /// The top-up that is SUPPOSED to work: same pool, same price, second run.
    function test_existing_pool_at_the_posted_price_tops_up() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        uint256 sqrtP = _consistentSqrtP(address(obol), 10);
        inp.obol = _seed(address(obol), sqrtP, OBOL_BASE, OBOL_QUOTE);
        factory.setPool(address(new MockPool(uint160(sqrtP))));
        npm.setPoolSqrtP(uint160(sqrtP));
        script.runWith(inp);
        assertEq(npm.mintCount(), 1, "the top-up minted");
    }

    /// Created but never initialized (`slot0().sqrtPriceX96 == 0`) is not a
    /// disagreement — the multicall initializes it at the passed price.
    function test_existing_but_uninitialized_pool_is_not_a_price_disagreement() public {
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        uint256 sqrtP = _consistentSqrtP(address(obol), 10);
        inp.obol = _seed(address(obol), sqrtP, OBOL_BASE, OBOL_QUOTE);
        factory.setPool(address(new MockPool(0)));
        script.runWith(inp);
        assertEq(npm.mintCount(), 1, "seeded normally");
        assertEq(uint256(npm.initSqrtP()), sqrtP, "and initialized at the price we passed");
    }

    // ── both sort branches, deterministically ───────────────────────────────
    /// Deploy fresh SpoilsTokens until one lands on the wanted side of the
    /// SCRY mock address, so BOTH branches of the script's sort are exercised
    /// no matter where setUp's fixtures happen to land.
    function _deploySpoilSorting(bool below) internal returns (SpoilsToken t) {
        for (uint256 i; i < 128; i++) {
            t = new SpoilsToken("spoil", "SPL", 0, operator);
            if ((address(t) < address(scry)) == below) return t;
        }
        revert("could not land a token on that side of scry");
    }

    function test_sort_branch_spoil_is_token0() public {
        SpoilsToken below = _deploySpoilSorting(true);
        MockNPM npmB = new MockNPM(address(factory)); // fresh single-pool mock
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        inp.npm = address(npmB);
        inp.obol = _seed(address(below), SQRTP_10_SPOIL_T0, OBOL_BASE, OBOL_QUOTE);

        script.runWith(inp);

        assertEq(npmB.initT0(), address(below), "spoil sorts as token0");
        assertEq(npmB.initT1(), address(scry), "scry is token1");
        assertEq(uint256(npmB.initSqrtP()), SQRTP_10_SPOIL_T0, "sqrtP for spoil-as-token0 (price 10)");
        (,,,,, uint256 a0, uint256 a1,,,,) = npmB.lastMint();
        assertEq(a0, OBOL_BASE, "amount0 = spoil base");
        assertEq(a1, OBOL_QUOTE, "amount1 = scry quote");
        assertEq(below.allowance(operator, address(npmB)), OBOL_BASE, "token0 approval");
        assertEq(scry.allowance(operator, address(npmB)), OBOL_QUOTE, "token1 approval");
    }

    function test_sort_branch_scry_is_token0() public {
        SpoilsToken above = _deploySpoilSorting(false);
        MockNPM npmB = new MockNPM(address(factory)); // fresh single-pool mock
        SeedSpoilsUniswapV3.SeedInputs memory inp = _baseInputs();
        inp.npm = address(npmB);
        inp.obol = _seed(address(above), SQRTP_10_SCRY_T0, OBOL_BASE, OBOL_QUOTE);

        script.runWith(inp);

        assertEq(npmB.initT0(), address(scry), "scry sorts as token0");
        assertEq(npmB.initT1(), address(above), "spoil is token1");
        assertEq(uint256(npmB.initSqrtP()), SQRTP_10_SCRY_T0, "sqrtP INVERTED for scry-as-token0 (price 0.1)");
        (,,,,, uint256 a0, uint256 a1,,,,) = npmB.lastMint();
        assertEq(a0, OBOL_QUOTE, "amount0 = scry quote");
        assertEq(a1, OBOL_BASE, "amount1 = spoil base");
        assertEq(scry.allowance(operator, address(npmB)), OBOL_QUOTE, "token0 approval");
        assertEq(above.allowance(operator, address(npmB)), OBOL_BASE, "token1 approval");
    }

    function _allowanceOf(address token) internal view returns (uint256) {
        if (token == address(scry)) return scry.allowance(operator, address(npm));
        if (token == address(obol)) return obol.allowance(operator, address(npm));
        return myrrh.allowance(operator, address(npm));
    }

    // ── run()'s env parsing, in ONE place ───────────────────────────────────
    /// The suite's only env writer: everything else calls runWith directly,
    /// so nothing can race these writes. Covers the env → SeedInputs mapping
    /// (incl. the zero-address skip sentinel) end to end through run().
    function test_run_reads_env_roundtrip() public {
        vm.setEnv("PRIVATE_KEY", vm.toString(bytes32(PK)));
        vm.setEnv("V3_FACTORY", vm.toString(address(factory)));
        vm.setEnv("V3_NPM", vm.toString(address(npm)));
        vm.setEnv("SCRY_TOKEN", vm.toString(address(scry)));
        vm.setEnv("FEE", "10000");
        vm.setEnv("SLIP_BPS", "100");
        vm.setEnv("DEADLINE_SECS", "900");
        vm.setEnv("LP_RECIPIENT", vm.toString(operator));
        uint256 sqrtP = _consistentSqrtP(address(obol), 10);
        vm.setEnv("OBOL_TOKEN", vm.toString(address(obol)));
        vm.setEnv("OBOL_SQRTP", vm.toString(sqrtP));
        vm.setEnv("OBOL_BASE_RAW", vm.toString(OBOL_BASE));
        vm.setEnv("OBOL_QUOTE_RAW", vm.toString(OBOL_QUOTE));
        vm.setEnv("MYRRH_TOKEN", vm.toString(address(0))); // zero sentinel = skip

        script.run();

        assertEq(npm.initCount(), 1, "one pool initialized via env path");
        assertEq(npm.mintCount(), 1, "one mint via env path");
        assertEq(uint256(npm.initSqrtP()), sqrtP, "sqrtP parsed from env");
        (,,,,, uint256 a0, uint256 a1,,, address rcpt,) = npm.lastMint();
        (uint256 e0, uint256 e1) = address(obol) < address(scry) ? (OBOL_BASE, OBOL_QUOTE) : (OBOL_QUOTE, OBOL_BASE);
        assertEq(a0, e0, "amount0 parsed + sorted from env");
        assertEq(a1, e1, "amount1 parsed + sorted from env");
        assertEq(rcpt, operator, "LP_RECIPIENT parsed from env");
    }
}
