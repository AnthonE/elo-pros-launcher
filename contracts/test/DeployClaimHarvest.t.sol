// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloHarvest.sol";
import "../src/SpoilsToken.sol";
import "../src/IERC20.sol";
import {DeployHarvest} from "../script/DeployHarvest.s.sol";
import {DeployClaim} from "../script/DeployClaim.s.sol";

/// The two claim-path deploy scripts, EXECUTED rather than merely compiled.
///
/// `AUDIT-2026-07-25.md` §0(a) measured that exactly one of twenty deploy
/// scripts was run by any test, and found F6 — a live bug — inside that blind
/// spot. `DeploySpoils` and `DeployGardener` have since been covered.
/// `DeployHarvest` and `DeployClaim` had NOT been, and they are the two that
/// stand up and fund the contracts holding the drop: the scripts nobody tested,
/// on the money path, on the launch path.
///
/// No env round-trip anywhere in this file: `vm.setEnv` is process-global and
/// races under forge's parallel workers (TEST-AUDIT.md, 2026-07-23). Both
/// scripts now expose a parameterized entry for exactly this reason.
abstract contract ClaimHarvestBase is Test {
    uint256 constant PK = 0xA11CE;
    address alice = address(0xA11CE0);
    address bob = address(0xB0B0);
    address treasury = address(0x7EA);

    function deployer() internal pure returns (address) {
        return vm.addr(PK);
    }

    /// What an UNFUNDED claim reverts with. Deliberately the token's OWN
    /// reason, not `SafeERC20`'s `"transfer failed"` fallback: §M5's fix
    /// bubbles the callee's revert unchanged precisely so an empty vault does
    /// not read the same as a broken one. Asserting the bubbled string is what
    /// keeps that property under test from this side.
    bytes constant EMPTY_VAULT = bytes("balance");

    function newCoin(string memory sym) internal returns (SpoilsToken) {
        // this contract is the minter, so a test can fund a harvest the way the
        // runbook does (`mint(harvest, total_committed)`).
        return new SpoilsToken(sym, sym, 0, address(this));
    }

    /// The shipped merkle dialect: leaf = keccak(wallet ‖ cumulative), sorted
    /// pairs up the tree — identical to `EloHarvest._verify` and
    /// `EloVowRegistry.verifyProof`.
    function leafOf(address w, uint256 cumulative) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(w, cumulative));
    }

    function pairOf(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return a < b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
    }

    /// A one-leaf tree: the root IS the leaf and the proof is empty.
    function soloProof() internal pure returns (bytes32[] memory p) {
        p = new bytes32[](0);
    }
}

/// ── DeployHarvest.s.sol ──────────────────────────────────────────────────
contract DeployHarvestScriptTest is ClaimHarvestBase {
    DeployHarvest script;
    SpoilsToken obol;

    function setUp() public {
        script = new DeployHarvest();
        obol = newCoin("OBOL");
    }

    function test_the_script_deploys_a_harvest_wired_to_the_token_and_the_deployer() public {
        EloHarvest h = script.runWith(PK, IERC20(address(obol)));

        assertGt(address(h).code.length, 0, "deployed");
        assertEq(address(h.reserve()), address(obol), "token is the one passed in");
        assertEq(h.operator(), deployer(), "operator is the broadcasting key, not the script");
        assertEq(h.root(), bytes32(0), "no root posted by the deploy itself");
        assertEq(h.rootCount(), 0);
        assertEq(h.owedUnderRoot(), 0);
    }

    /// `vm.envAddress` hands back address(0) happily, `reserve` is immutable, and
    /// the failure would only surface after a root had been posted and
    /// published. RED before the guard: this deployed a permanent brick.
    function test_the_script_refuses_a_zero_token_address() public {
        vm.expectRevert(bytes("OBOL_TOKEN: zero token address"));
        script.runWith(PK, IERC20(address(0)));
    }

    /// The natspec's step 3 rule ("fund BEFORE posting") is OPERATIONAL, not
    /// enforced — `postRoot` never reads the balance. This pins what that
    /// actually costs, so nobody upgrades the rule to a belief.
    function test_a_root_posted_before_funding_is_a_promise_that_cannot_pay_yet() public {
        EloHarvest h = script.runWith(PK, IERC20(address(obol)));
        bytes32 root = leafOf(alice, 500e18);

        vm.prank(deployer());
        h.postRoot(root, 500e18, "ledger-ref");

        // posted, published, and unpayable — the token call is what reverts
        assertEq(h.owedUnderRoot(), 500e18, "the obligation is armed");
        vm.expectRevert(EMPTY_VAULT);
        h.claim(alice, 500e18, soloProof());

        // fund it the way the runbook does, and the same claim now pays
        obol.mint(address(h), 500e18);
        uint256 paid = h.claim(alice, 500e18, soloProof());
        assertEq(paid, 500e18);
        assertEq(obol.balanceOf(alice), 500e18);
        assertEq(h.owedUnderRoot(), 0, "the obligation is discharged");
    }
}

/// ── DeployClaim.s.sol ────────────────────────────────────────────────────
contract DeployClaimScriptTest is ClaimHarvestBase {
    DeployClaim script;
    SpoilsToken obol;
    SpoilsToken myrrh;
    SpoilsToken reserve;

    function setUp() public {
        script = new DeployClaim();
        obol = newCoin("OBOL");
        myrrh = newCoin("MYRRH");
        reserve = newCoin("SCRY");
    }

    function inputs() internal view returns (DeployClaim.ClaimInputs memory inp) {
        inp.pk = PK;
        inp.ref = "founders-2026-07-25-b19231505";
        inp.obol = address(obol);
        inp.myrrh = address(myrrh);
        inp.reserve = address(reserve);
        inp.obolRoot = leafOf(alice, 1_500e18);
        inp.myrrhRoot = leafOf(alice, 25e18);
        inp.reserveRoot = leafOf(alice, 10_000e18);
        inp.obolTotal = 1_500e18;
        inp.myrrhTotal = 25e18;
        inp.reserveTotal = 10_000e18;
    }

    function test_one_harvest_per_token_each_carrying_its_own_root_and_total() public {
        (address o, address m, address s) = script.runWith(inputs());

        assertTrue(o != address(0) && m != address(0) && s != address(0), "all three deployed");
        assertTrue(o != m && m != s && o != s, "and they are three distinct contracts");

        assertEq(address(EloHarvest(o).reserve()), address(obol));
        assertEq(address(EloHarvest(m).reserve()), address(myrrh));
        assertEq(address(EloHarvest(s).reserve()), address(reserve));

        assertEq(EloHarvest(o).root(), leafOf(alice, 1_500e18), "OBOL root posted at deploy");
        assertEq(EloHarvest(o).totalCommitted(), 1_500e18);
        assertEq(EloHarvest(m).totalCommitted(), 25e18);
        assertEq(EloHarvest(s).totalCommitted(), 10_000e18);

        // the sweep floor is armed the moment the root lands, before funding
        assertEq(EloHarvest(o).sweepFloor(), 1_500e18);
        assertEq(EloHarvest(o).operator(), deployer(), "operator is the deploying key");
    }

    function test_a_token_with_no_root_is_skipped_and_the_others_still_deploy() public {
        DeployClaim.ClaimInputs memory inp = inputs();
        inp.myrrhRoot = bytes32(0);
        inp.myrrhTotal = 0;

        (address o, address m, address s) = script.runWith(inp);
        assertEq(m, address(0), "MYRRH skipped");
        assertGt(o.code.length, 0, "OBOL still deployed");
        assertGt(s.code.length, 0, "SCRY still deployed");
    }

    function test_a_root_with_no_token_address_is_refused_by_name() public {
        DeployClaim.ClaimInputs memory inp = inputs();
        inp.obol = address(0);
        vm.expectRevert(bytes("OBOL: token address unset"));
        script.runWith(inp);
    }

    /// §M4 at deploy time. `owedUnderRoot` IS the sweep floor and `postRoot`
    /// cannot check a total against the tree, so a real root with a zero total
    /// is a published promise the contract does not defend for one block — and
    /// the SWEEP_DELAY window cannot cover it, because that floor is
    /// `priorOwed`, which is 0 on a first root. RED before the guard.
    function test_a_root_posted_with_a_zero_total_is_refused_before_broadcast() public {
        DeployClaim.ClaimInputs memory inp = inputs();
        inp.myrrhTotal = 0; // root still set
        vm.expectRevert(bytes("MYRRH: root is set but the total is 0 - the sweep floor would be zero"));
        script.runWith(inp);
    }

    /// A paste error that points two drops at one token deploys two contracts
    /// over the same balance, each believing it owns the whole obligation.
    function test_two_roots_pointing_at_one_token_are_refused() public {
        DeployClaim.ClaimInputs memory inp = inputs();
        inp.myrrh = address(obol);
        vm.expectRevert(bytes("OBOL/MYRRH: both roots point at the SAME token address"));
        script.runWith(inp);
    }

    /// The §0.4 rule, as construction rather than as prose: run the script
    /// twice and no address is shared, so drop two can never land on drop
    /// one's instance by accident of re-running the documented command.
    function test_every_run_produces_its_own_contracts() public {
        (address o1, address m1, address s1) = script.runWith(inputs());
        (address o2, address m2, address s2) = script.runWith(inputs());
        assertTrue(o1 != o2 && m1 != m2 && s1 != s2, "two runs share nothing");
    }

    function test_a_funded_claim_contract_pays_the_published_proof() public {
        (address o,,) = script.runWith(inputs());
        obol.mint(o, 1_500e18); // the arming cast: mint the total INTO the claim

        uint256 paid = EloHarvest(o).claim(alice, 1_500e18, soloProof());
        assertEq(paid, 1_500e18);
        assertEq(obol.balanceOf(alice), 1_500e18);
        assertEq(EloHarvest(o).totalClaimed(), 1_500e18);

        // and it is spent — a second claim on the same cumulative pays nothing
        vm.expectRevert(bytes("nothing to claim"));
        EloHarvest(o).claim(alice, 1_500e18, soloProof());
    }

    /// A two-leaf tree, so the proof path is exercised and not just the
    /// degenerate one-leaf root.
    function test_a_real_proof_path_pays_each_leaf_once() public {
        bytes32 la = leafOf(alice, 1_500e18);
        bytes32 lb = leafOf(bob, 900e18);

        DeployClaim.ClaimInputs memory inp = inputs();
        inp.obolRoot = pairOf(la, lb);
        inp.obolTotal = 2_400e18;
        (address o,,) = script.runWith(inp);
        obol.mint(o, 2_400e18);

        bytes32[] memory pa = new bytes32[](1);
        pa[0] = lb;
        bytes32[] memory pb = new bytes32[](1);
        pb[0] = la;

        EloHarvest(o).claim(alice, 1_500e18, pa);
        EloHarvest(o).claim(bob, 900e18, pb);
        assertEq(obol.balanceOf(alice), 1_500e18);
        assertEq(obol.balanceOf(bob), 900e18);
        assertEq(EloHarvest(o).owedUnderRoot(), 0, "the whole stated obligation is discharged");

        // a wallet not in the tree cannot invent one
        bytes32[] memory junk = new bytes32[](1);
        junk[0] = la;
        vm.expectRevert(bytes("bad proof"));
        EloHarvest(o).claim(treasury, 1e18, junk);
    }
}

/// ── the drop-isolation laws, made executable ────────────────────────────
/// Both currently live as prose in `DeployClaim`'s natspec and as an off-chain
/// gate in `contracts/preflight.py`. Neither was pinned in Solidity, which is
/// where the arithmetic actually happens.
contract HarvestDropIsolationTest is ClaimHarvestBase {
    SpoilsToken obol;

    function setUp() public {
        obol = newCoin("OBOL");
    }

    function _harvest() internal returns (EloHarvest h) {
        h = new EloHarvest(IERC20(address(obol)));
        obol.mint(address(h), 100_000e18);
    }

    /// §0.4. `claimed[wallet]` is a LIFETIME total and `claim` pays
    /// `cumulative − claimed`, so per-drop absolute roots cannot share an
    /// instance: drop two zeroes anyone who was paid by drop one.
    function test_reusing_one_instance_for_two_drops_zeroes_a_claimant() public {
        EloHarvest shared = _harvest();

        shared.postRoot(leafOf(alice, 1_500e18), 1_500e18, "drop-one");
        assertEq(shared.claim(alice, 1_500e18, soloProof()), 1_500e18);

        // drop two: a per-drop ABSOLUTE amount, smaller than drop one's
        shared.postRoot(leafOf(alice, 900e18), 900e18, "drop-two");
        vm.expectRevert(bytes("nothing to claim"));
        shared.claim(alice, 900e18, soloProof());

        assertEq(obol.balanceOf(alice), 1_500e18, "drop two paid nothing at all");
    }

    /// The same two drops on their OWN instances pay both, which is what the
    /// rule buys and why it is a rule rather than a preference.
    function test_separate_instances_pay_both_drops_in_full() public {
        EloHarvest one = _harvest();
        EloHarvest two = _harvest();

        one.postRoot(leafOf(alice, 1_500e18), 1_500e18, "drop-one");
        two.postRoot(leafOf(alice, 900e18), 900e18, "drop-two");

        one.claim(alice, 1_500e18, soloProof());
        two.claim(alice, 900e18, soloProof());
        assertEq(obol.balanceOf(alice), 2_400e18);
    }

    /// §M4, the honest scope. The delay window holds the PREVIOUS obligation —
    /// so it protects a lowered root, and buys exactly nothing on a FIRST root,
    /// where `priorOwed` is still 0. This is why `DeployClaim` refuses a zero
    /// total rather than trusting the window to catch it.
    function test_M4_the_sweep_delay_does_not_cover_a_first_root() public {
        EloHarvest h = _harvest();
        h.postRoot(leafOf(alice, 1_500e18), 0, "first-root-zero-total");

        assertEq(h.priorOwed(), 0, "nothing came before it");
        assertEq(h.sweepFloor(), 0, "so the floor is zero from block one");

        h.sweep(treasury, 100_000e18); // immediate, in the same block
        assertEq(obol.balanceOf(treasury), 100_000e18);

        vm.expectRevert(EMPTY_VAULT);
        h.claim(alice, 1_500e18, soloProof());
    }

    /// And the half that DOES hold: an obligation already stated cannot be
    /// retracted and drained in the same breath.
    function test_M4_a_lowered_obligation_is_held_for_the_full_sweep_delay() public {
        EloHarvest h = _harvest();
        h.postRoot(leafOf(alice, 1_500e18), 1_500e18, "drop-one");

        // the operator re-posts, now stating it owes nothing
        h.postRoot(leafOf(bob, 1e18), 0, "retraction");
        assertEq(h.priorOwed(), 1_500e18);
        assertEq(h.sweepFloor(), 1_500e18, "the OLD obligation is the floor");

        vm.expectRevert(bytes("would break the posted root"));
        h.sweep(treasury, 100_000e18 - 1_500e18 + 1);

        // alice can still take what the retracted root promised her
        h.postRoot(leafOf(alice, 1_500e18), 1_500e18, "restored");
        assertEq(h.claim(alice, 1_500e18, soloProof()), 1_500e18);

        // Alice's claim does NOT walk the carried floor down (2026-07-28): a
        // payment under the CURRENT root discharges nothing carried from the
        // previous one, because the contract cannot know the claimant was in
        // both. So the high-water mark stands at 1500 through the re-posts
        // below, and only the window closing releases it.
        assertEq(h.priorOwed(), 1_500e18, "the claim left the high-water mark alone");

        h.postRoot(leafOf(alice, 400e18), 400e18, "a fresh obligation");
        h.postRoot(leafOf(bob, 1e18), 0, "retraction-again");
        assertEq(h.sweepFloor(), 1_500e18, "the carried high-water mark still holds");

        // The release valve is a gap, not a claim: one SWEEP_DELAY with no
        // re-post drops `carried` to 0 and the floor to the current root's own
        // obligation, which these two re-posts left at 0.
        vm.warp(block.timestamp + h.SWEEP_DELAY() + 1);
        assertEq(h.sweepFloor(), 0, "the window has closed");
        h.sweep(treasury, obol.balanceOf(address(h)));
    }

    /// THE BYPASS THAT MADE THAT GUARANTEE CONDITIONAL (audit 2026-07-27).
    /// The test above stops at ONE retraction, which is exactly where the guard
    /// held. `postRoot` used to assign `priorOwed = owedUnderRoot`
    /// unconditionally, so a SECOND consecutive re-post read the
    /// already-retracted zero back into the carried floor and the whole
    /// obligation vanished — three calls, one block, no warp needed. RED before
    /// the fix: `sweepFloor()` returned 0 on the third post and the sweep below
    /// emptied a contract alice was still owed 1,500 OBOL from.
    function test_M4_the_window_survives_repeated_retractions() public {
        EloHarvest h = _harvest();
        h.postRoot(leafOf(alice, 1_500e18), 1_500e18, "honest");

        // one retraction — the case the older test pinned
        h.postRoot(leafOf(bob, 1e18), 0, "retraction");
        assertEq(h.sweepFloor(), 1_500e18, "held after one retraction");

        // …and the walk-down. Every further re-post inside the window must
        // carry the SAME obligation forward, not the retracted one it stands on.
        for (uint256 i = 0; i < 3; i++) {
            h.postRoot(leafOf(bob, 1e18), 0, "retraction-again");
            assertEq(h.priorOwed(), 1_500e18, "the carried obligation is not walked down");
            assertEq(h.sweepFloor(), 1_500e18, "and neither is the floor");
        }

        // the drain the bypass bought, refused
        vm.expectRevert(bytes("would break the posted root"));
        h.sweep(treasury, 100_000e18 - 1_500e18 + 1);

        // and what alice was promised is still hers, under the root that
        // promised it
        h.postRoot(leafOf(alice, 1_500e18), 1_500e18, "restored");
        assertEq(h.claim(alice, 1_500e18, soloProof()), 1_500e18);
        assertEq(obol.balanceOf(alice), 1_500e18, "the retracted claim was still payable");
    }

    /// The other half of that fix, and the one a max-carry could plausibly
    /// break: the carry is a DELAY, not a lock. Repeated retractions hold the
    /// floor for the window's length and then it lapses like any other, so
    /// genuine surplus is never stranded by the guard.
    function test_M4_a_carried_obligation_still_expires() public {
        EloHarvest h = _harvest();
        h.postRoot(leafOf(alice, 1_500e18), 1_500e18, "honest");
        h.postRoot(leafOf(bob, 1e18), 0, "retraction");
        h.postRoot(leafOf(bob, 1e18), 0, "retraction-again");
        assertEq(h.sweepFloor(), 1_500e18, "carried while the window is open");

        vm.warp(block.timestamp + h.SWEEP_DELAY() + 1);
        assertEq(h.sweepFloor(), 0, "and released when it closes");
        h.sweep(treasury, obol.balanceOf(address(h)));
        assertEq(obol.balanceOf(address(h)), 0, "surplus is not stranded by the carry");
    }
}
