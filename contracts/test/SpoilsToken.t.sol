// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "./MockToken.sol";
import "../src/ScryGarden.sol";
import {DeploySpoils} from "../script/DeploySpoils.s.sol";

/// SpoilsToken: the capped/earned/burnable play token — and the proof that
/// it (unlike the retired free-faucet play token) can safely face a valued token in
/// the Garden. `forge test -vv` before any broadcast.
contract SpoilsTokenTest is Test {
    SpoilsToken obol;
    MockToken scry; // stands in for SCRY here — any ERC20 with a faucet for test funding

    address distributor = address(0xD157);
    address alice = address(0xA11CE);
    address bob = address(0xB0B);

    uint256 constant CAP = 1_000e18;

    function setUp() public {
        obol = new SpoilsToken("obol", "OBOL", CAP, distributor);
        scry = new MockToken("stand-in scry", "tSCRY");
    }

    function test_only_distributor_mints() public {
        vm.expectRevert("minter only");
        obol.mint(alice, 1e18);
        vm.prank(distributor);
        obol.mint(alice, 10e18);
        assertEq(obol.balanceOf(alice), 10e18);
        assertEq(obol.totalMinted(), 10e18);
        assertEq(obol.totalSupply(), 10e18);
    }

    function test_cap_is_forever_and_burn_does_not_reopen_it() public {
        vm.startPrank(distributor);
        obol.mint(alice, CAP);
        vm.expectRevert("cap exceeded");
        obol.mint(alice, 1);
        vm.stopPrank();
        // alice burns half — circulating drops, mint room stays closed
        vm.prank(alice);
        obol.burn(CAP / 2);
        assertEq(obol.totalSupply(), CAP / 2);
        assertEq(obol.totalBurned(), CAP / 2);
        vm.prank(distributor);
        vm.expectRevert("cap exceeded");
        obol.mint(alice, 1);
    }

    /// Elastic default (cap 0): mint tracks the game economy with no ceiling;
    /// the distributor gate — not a cap — is what keeps it off the leak path.
    function test_uncapped_mints_freely() public {
        SpoilsToken elastic = new SpoilsToken("obol", "OBOL", 0, distributor);
        assertEq(elastic.cap(), 0);
        vm.startPrank(distributor);
        elastic.mint(alice, 1_000_000_000e18); // far beyond any old season cap
        elastic.mint(alice, 1_000_000_000e18); // and again — no revert
        vm.stopPrank();
        assertEq(elastic.totalMinted(), 2_000_000_000e18);
        // still distributor-only — not a public faucet
        vm.expectRevert("minter only");
        elastic.mint(bob, 1);
    }

    function test_burn_and_burnFrom() public {
        vm.prank(distributor);
        obol.mint(alice, 100e18);
        vm.prank(alice);
        obol.burn(30e18);
        assertEq(obol.balanceOf(alice), 70e18);
        vm.prank(alice);
        obol.approve(bob, 20e18);
        vm.prank(bob);
        obol.burnFrom(alice, 20e18);
        assertEq(obol.balanceOf(alice), 50e18);
        assertEq(obol.totalBurned(), 50e18);
        vm.prank(bob);
        vm.expectRevert("allowance");
        obol.burnFrom(alice, 1);
    }

    function test_transfer_to_zero_refused_use_burn() public {
        vm.prank(distributor);
        obol.mint(alice, 1e18);
        vm.prank(alice);
        vm.expectRevert("zero to - use burn");
        obol.transfer(address(0), 1e18);
    }

    function test_minter_rotation() public {
        vm.prank(distributor);
        obol.setMinter(bob);
        vm.prank(distributor);
        vm.expectRevert("minter only");
        obol.mint(alice, 1);
        vm.prank(bob);
        obol.mint(alice, 1e18);
        assertEq(obol.balanceOf(alice), 1e18);
    }

    // ── audit additions (TEST-AUDIT.md 2026-07-22) ──────────────────────────

    /// A distributor typo (mint to the zero address) would irrecoverably
    /// inflate supply — balanceOf[address(0)] is unspendable and unburnable,
    /// yet counts in totalSupply forever. Now refused, mirroring the existing
    /// transfer-to-zero refusal.
    function test_mint_to_zero_reverts() public {
        vm.prank(distributor);
        vm.expectRevert("zero to");
        obol.mint(address(0), 1e18);
        assertEq(obol.totalMinted(), 0, "nothing minted");
        assertEq(obol.totalSupply(), 0, "supply untouched");
    }

    function test_setMinter_non_minter_reverts() public {
        vm.prank(alice);
        vm.expectRevert("minter only");
        obol.setMinter(alice);
    }

    function test_setMinter_to_zero_reverts() public {
        vm.prank(distributor);
        vm.expectRevert("zero minter");
        obol.setMinter(address(0));
        assertEq(obol.minter(), distributor, "minter unchanged after refused rotation");
    }

    /// The standard ERC-20 infinite-allowance carve-out: type(uint256).max is
    /// never decremented — by burnFrom or transferFrom — so a distributor/pool
    /// granted max once keeps working without re-approval.
    function test_infinite_allowance_not_decremented() public {
        uint256 maxAllowance = type(uint256).max;
        vm.prank(distributor);
        obol.mint(alice, 100e18);
        vm.prank(alice);
        obol.approve(bob, maxAllowance);
        vm.prank(bob);
        obol.burnFrom(alice, 10e18);
        assertEq(obol.allowance(alice, bob), maxAllowance, "burnFrom must not touch an infinite allowance");
        vm.prank(bob);
        obol.transferFrom(alice, bob, 10e18);
        assertEq(obol.allowance(alice, bob), maxAllowance, "transferFrom must not touch an infinite allowance");
        assertEq(obol.balanceOf(alice), 80e18);
        assertEq(obol.balanceOf(bob), 10e18);
        assertEq(obol.totalBurned(), 10e18);
    }

    /// The point of the cap: a Garden pair of SpoilsToken vs a valued token
    /// is a market, not a leak — there is no faucet to drain the real side.
    function test_garden_pair_vs_valued_token_is_sound() public {
        ScryGarden garden = new ScryGarden(IERC20(address(obol)), IERC20(address(scry)));
        // fund alice: earned spoils + faucet stand-in scry
        vm.prank(distributor);
        obol.mint(alice, 500e18);
        vm.prank(alice);
        scry.faucet(); // 1000e18
        vm.startPrank(alice);
        obol.approve(address(garden), type(uint256).max);
        scry.approve(address(garden), type(uint256).max);
        garden.addLiquidity(400e18, 400e18, 0, type(uint256).max);
        vm.stopPrank();
        // bob CANNOT mint free obol to drain the scry side — only the
        // distributor mints, within the cap. His only path in is trade.
        vm.startPrank(bob);
        vm.expectRevert("minter only");
        obol.mint(bob, 1_000_000e18);
        vm.stopPrank();
        // a real swap still works for someone holding earned spoils
        vm.prank(distributor);
        obol.mint(bob, 10e18);
        vm.startPrank(bob);
        obol.approve(address(garden), type(uint256).max);
        uint256 out = garden.swap(10e18, true, 0, type(uint256).max);
        vm.stopPrank();
        assertGt(out, 0);
        assertEq(scry.balanceOf(bob), out);
    }

    // -- fuzz: supply conservation + cap enforcement -------------------------
    /// The token's two numeric laws, pinned above only at hand-picked amounts,
    /// held over a FUZZED mint/burn sequence: after every single op (a)
    /// circulating supply is exactly minted - burned, (b) cumulative mint never
    /// crosses the cap (a burn does not reopen it), and (c) with one holder the
    /// balance equals the whole supply. Even amounts mint (capped), odd amounts
    /// burn; oversized amounts are modded into range so the sequence exercises
    /// the cap boundary from both sides.
    function testFuzz_supply_conservation_and_cap(uint256[] calldata amounts) public {
        uint256 n = amounts.length > 24 ? 24 : amounts.length; // bound gas, not coverage
        for (uint256 i = 0; i < n; i++) {
            if (i % 2 == 0) {
                uint256 m = amounts[i] % (CAP + 1); // a single mint never exceeds the cap
                // read the view BEFORE pranking - a staticcall between prank and
                // mint would consume the prank (the ScryDeed cheatcode-misfire
                // class in TEST-AUDIT.md), sending the mint as the wrong caller.
                bool willFit = obol.totalMinted() + m <= CAP;
                if (willFit) {
                    vm.prank(distributor);
                    obol.mint(alice, m);
                } else {
                    vm.prank(distributor);
                    vm.expectRevert("cap exceeded"); // the ceiling is forever
                    obol.mint(alice, m);
                }
            } else {
                uint256 bal = obol.balanceOf(alice);
                uint256 b = bal == 0 ? 0 : amounts[i] % (bal + 1);
                if (b > 0) {
                    vm.prank(alice);
                    obol.burn(b);
                }
            }
            assertEq(obol.totalSupply(), obol.totalMinted() - obol.totalBurned(), "supply != minted - burned");
            assertLe(obol.totalMinted(), CAP, "cumulative mint crossed the cap");
            assertEq(obol.balanceOf(alice), obol.totalSupply(), "sole holder balance != supply");
        }
    }
}

/// DeploySpoils.s.sol EXECUTED, not merely compiled. The 2026-07-25 audit's
/// first finding was that 19 of 20 deploy scripts are never run by any test,
/// and the Garden's opening seed now lives in exactly that blind spot: a
/// ScryGarden's first add SETS ITS PRICE, so "deployed but not yet seeded" is
/// a window in which anyone can open the pool at any ratio for dust.
contract DeploySpoilsSeedTest is Test {
    DeploySpoils script;
    uint256 constant PK = 0xA11CE;

    function setUp() public {
        script = new DeploySpoils();
    }

    /// No env round-trip: vm.setEnv is process-global and forge's parallel
    /// workers share one process, so env writes race between tests.
    function _inputs() internal pure returns (DeploySpoils.SpoilsInputs memory inp) {
        inp.pk = PK;
        inp.pairGarden = true;
        inp.seedMyrrh = 20_000e18;
        inp.seedObol = 100_000e18;
    }

    function test_theGardenIsNeverLeftPubliclyEmpty() public {
        script.runWith(_inputs());
        // the script logs the addresses; re-derive them the way the runbook
        // does, from the broadcast the deployer just made
        address deployer = vm.addr(PK);
        // nonce 0,1 = the two tokens; 2 = the garden
        address obol = vm.computeCreateAddress(deployer, 0);
        address myrrh = vm.computeCreateAddress(deployer, 1);
        address garden = vm.computeCreateAddress(deployer, 2);

        assertGt(garden.code.length, 0, "the Garden was deployed");
        ScryGarden g = ScryGarden(garden);
        (uint112 r0, uint112 r1) = g.getReserves();
        assertGt(uint256(r0), 0, "and it is NOT empty - reserve0");
        assertGt(uint256(r1), 0, "and it is NOT empty - reserve1");

        // it opened at the posted 5 OBOL per MYRRH (token0 = MYRRH)
        assertEq(address(g.token0()), myrrh, "token0 is MYRRH");
        assertEq(address(g.token1()), obol, "token1 is OBOL");
        assertEq(uint256(r1) / uint256(r0), 5, "opened at 5 OBOL per MYRRH");

        // the SEED shares went to the deployer, and MINIMUM_LIQUIDITY is locked
        assertGt(g.balanceOf(deployer), 0, "LP shares to the deployer");
        assertEq(g.balanceOf(garden), g.MINIMUM_LIQUIDITY(), "the dust is locked forever");

        // nothing was minted beyond the seed - this is not a pre-mine of float
        assertEq(SpoilsToken(myrrh).totalSupply(), uint256(r0), "MYRRH minted == MYRRH pooled");
        assertEq(SpoilsToken(obol).totalSupply(), uint256(r1), "OBOL minted == OBOL pooled");
        assertEq(SpoilsToken(myrrh).balanceOf(deployer), 0, "no loose MYRRH left over");
        assertEq(SpoilsToken(obol).balanceOf(deployer), 0, "no loose OBOL left over");
    }

    string constant OFF_RATIO =
        "GARDEN_SEED_*: the Garden must open at exactly the posted 5 OBOL per MYRRH";

    function test_anOffRatioSeedIsRefusedBeforeAnythingDeploys() public {
        DeploySpoils.SpoilsInputs memory inp = _inputs();
        inp.seedObol = inp.seedMyrrh; // 1:1, not the posted 5:1
        vm.expectRevert(bytes(OFF_RATIO));
        script.runWith(inp);
    }

    /// The guard used to be `seedObol / seedMyrrh == 5`, which is integer
    /// division and therefore accepted the whole half-open interval [5.0, 6.0).
    /// On wei-scale inputs that is a ratio wrong by up to 20%, welded: the
    /// Garden's opening add SETS ITS PRICE, and the seeding branch is
    /// unreachable a second time because MINIMUM_LIQUIDITY is never burned, so
    /// totalSupply never returns to 0.
    ///
    /// This is the case the tightening was FOR, and without it the test above
    /// passes against either version of the guard — 1:1 fails both. Delete this
    /// and the fix stops being defended by anything.
    function test_aRatioTheOldIntegerDivisionWouldHaveWavedThrough() public {
        DeploySpoils.SpoilsInputs memory inp = _inputs();
        inp.seedObol = inp.seedMyrrh * 5 + (inp.seedMyrrh / 2);   // 5.5 : 1
        assertEq(inp.seedObol / inp.seedMyrrh, 5, "the old guard would have read this as 5");
        vm.expectRevert(bytes(OFF_RATIO));
        script.runWith(inp);
    }

    /// …and off by ONE WEI, the smallest thing integer division cannot see.
    function test_aSingleWeiOffTheRatioIsStillRefused() public {
        DeploySpoils.SpoilsInputs memory inp = _inputs();
        inp.seedObol = inp.seedMyrrh * 5 + 1;
        vm.expectRevert(bytes(OFF_RATIO));
        script.runWith(inp);
    }

    /// Opting out is still allowed, and it says so loudly rather than
    /// silently leaving a price-setting hole open.
    function test_seedZeroStillDeploysTheGardenUnseeded() public {
        DeploySpoils.SpoilsInputs memory inp = _inputs();
        inp.seedMyrrh = 0;
        inp.seedObol = 0;
        script.runWith(inp);
        address garden = vm.computeCreateAddress(vm.addr(PK), 2);
        assertGt(garden.code.length, 0, "deployed");
        (uint112 r0, uint112 r1) = ScryGarden(garden).getReserves();
        assertEq(uint256(r0), 0);
        assertEq(uint256(r1), 0);
    }
}
