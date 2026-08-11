// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import {SeedSpoilsUniswapV3} from "../script/SeedSpoilsUniswapV3.s.sol";
import {SpoilsToken} from "../src/SpoilsToken.sol";
import {INonfungiblePositionManager, IUniswapV3Factory, IERC20Seed} from "../src/interfaces/IUniswapV3.sol";

/// slot0 + liquidity reads for the real pool the seed flow creates.
interface IUniswapV3PoolSeedFork {
    function slot0()
        external
        view
        returns (
            uint160 sqrtPriceX96,
            int24 tick,
            uint16 observationIndex,
            uint16 observationCardinality,
            uint16 observationCardinalityNext,
            uint8 feeProtocol,
            bool unlocked
        );

    function liquidity() external view returns (uint128);
}

interface IERC721Balance {
    function balanceOf(address owner) external view returns (uint256);
}

/// The SECOND fork harness TEST-AUDIT.md mandates ("the two places a fork
/// run is mandatory, not optional: ScryOrchard and the SeedSpoilsUniswapV3
/// flow"). The unit suite's MockNPM models one economic rail; this runs the
/// REAL script entry (`runWith`) against the REAL RH-Chain factory + NPM:
///
///   - the hardcoded DEFAULT_FACTORY / DEFAULT_NPM / DEFAULT_SCRY literals
///     are re-verified against live chain state (SHIP.md: "re-verify the
///     hardcoded NPM/factory on-chain first") — npm.factory() wiring, the
///     1% fee tier's spacing, and that SCRY really sits at its address;
///   - a fork-deployed SpoilsToken + real SCRY go through the script's
///     full path: sort, tripwire, approvals, createAndInitialize + mint in
///     one multicall — against real periphery bytecode, so a mis-transcribed
///     interface or bad tick rounding reverts HERE and nowhere else;
///   - the resulting REAL pool initializes at exactly the passed sqrtP, has
///     live liquidity, and the LP NFT lands with the recipient.
///
/// Runs on a FORK — nothing broadcasts, nothing is spent. Env-guarded like
/// OrchardFork: without RH_FORK_URL it logs a SKIP and passes, so the suite
/// stays green offline.
///
///   RH_FORK_URL=https://rpc.mainnet.chain.robinhood.com \
///     forge test --match-contract SeedSpoilsForkTest -vv
contract SeedSpoilsForkTest is Test {
    uint256 constant PK = 0xA11CE;
    uint24 constant FEE = 10000; // the 1% tier the script defaults to
    uint256 constant SQRTP_ONE = 79228162514264337593543950336; // 2^96 = price 1.0

    bool skipRun;
    address operator;
    SeedSpoilsUniswapV3 script;
    SpoilsToken spoil;

    function setUp() public {
        string memory url = vm.envOr("RH_FORK_URL", string(""));
        if (bytes(url).length == 0) {
            skipRun = true;
            return;
        }
        vm.createSelectFork(url);
        operator = vm.addr(PK);
        script = new SeedSpoilsUniswapV3();
        // A fresh spoils token on the fork (OBOL/MYRRH are not broadcast yet;
        // the flow under test is identical either way).
        spoil = new SpoilsToken("obol", "OBOL", 1_000_000e18, operator);
        vm.prank(operator);
        spoil.mint(operator, 1_000e18);
        // Real SCRY for the quote side: deal probes the balance slot of the
        // REAL token — if SCRY were nonstandard this line is where it fails.
        deal(script.DEFAULT_SCRY(), operator, 1_000e18);
    }

    /// SHIP.md's pre-broadcast tripwire, as a test: the posted literals match
    /// the live chain. If Uniswap redeploys or the fee tier changes, this
    /// reverts before anyone trusts the defaults.
    function test_fork_defaults_match_live_chain() public {
        if (skipRun) {
            emit log("SKIP: set RH_FORK_URL to run the seed-flow fork test");
            return;
        }
        INonfungiblePositionManager npm = INonfungiblePositionManager(script.DEFAULT_NPM());
        assertEq(npm.factory(), script.DEFAULT_FACTORY(), "NPM wired to the posted factory");
        int24 spacing = IUniswapV3Factory(script.DEFAULT_FACTORY()).feeAmountTickSpacing(FEE);
        assertEq(int256(spacing), int256(200), "1% tier enabled at spacing 200");
        assertGt(script.DEFAULT_SCRY().code.length, 0, "SCRY is a contract at the posted address");
        assertEq(
            IERC20Seed(script.DEFAULT_SCRY()).balanceOf(operator),
            1_000e18,
            "real SCRY balance dealt (standard balance slot)"
        );
    }

    /// The full flow against the real periphery: price 1.0 (sqrtP = 2^96) is
    /// sort-orientation-proof — its inverse is itself — so the tripwire's
    /// consistency check is exact whichever side the fork-deployed token
    /// lands on, and the pool must initialize at exactly 2^96.
    function test_fork_real_npm_seed_creates_pool_and_mints_lp() public {
        if (skipRun) {
            emit log("SKIP: set RH_FORK_URL to run the seed-flow fork test");
            return;
        }
        address scry = script.DEFAULT_SCRY();
        SeedSpoilsUniswapV3.SeedInputs memory inp;
        inp.pk = PK;
        inp.factory = script.DEFAULT_FACTORY();
        inp.npm = script.DEFAULT_NPM();
        inp.scry = scry;
        inp.fee = FEE;
        inp.slipBps = 100;
        inp.deadlineSecs = 900;
        inp.recipient = operator;
        inp.obol = SeedSpoilsUniswapV3.PoolSeed(address(spoil), uint160(SQRTP_ONE), 1_000e18, 1_000e18);
        // myrrh stays the zero sentinel = skipped

        uint256 nftBefore = IERC721Balance(inp.npm).balanceOf(operator);

        script.runWith(inp);

        (address t0, address t1) = address(spoil) < scry ? (address(spoil), scry) : (scry, address(spoil));
        address pool = IUniswapV3Factory(inp.factory).getPool(t0, t1, FEE);
        assertTrue(pool != address(0), "real factory created the pool");
        (uint160 sqrtP,,,,,,) = IUniswapV3PoolSeedFork(pool).slot0();
        assertEq(uint256(sqrtP), SQRTP_ONE, "pool initialized at exactly the passed sqrtP");
        assertGt(IUniswapV3PoolSeedFork(pool).liquidity(), 0, "position is live in-range liquidity");
        assertEq(
            IERC721Balance(inp.npm).balanceOf(operator),
            nftBefore + 1,
            "the LP NFT landed with the recipient, not a service"
        );
        // both sides actually deposited (full-range at price 1.0 takes both)
        assertLt(spoil.balanceOf(operator), 1_000e18, "spoil side deposited");
        assertLt(IERC20Seed(scry).balanceOf(operator), 1_000e18, "SCRY side deposited");
    }
}
