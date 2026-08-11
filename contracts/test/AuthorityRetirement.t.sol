// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/SpoilsToken.sol";
import "../src/ScryGranary.sol";

/// WHO CAN STILL MINT WHEN THE LAUNCH IS OVER — the one question the launch
/// exists to answer, made executable.
///
/// The whole story lived in prose across `LAUNCH.md`, `deploy_town.sh` and two
/// deploy scripts, and it was re-derived by hand on 2026-07-28 because nothing
/// could be run to settle it. The derivation was right, and it took a careful
/// read of four files to reach a conclusion this suite reaches in 300ms:
///
///   `rotate` retires `SpoilsToken.minter`. But `gardener` and `granary` have
///   ALREADY moved that slot on their way past — DeployGardener.s.sol:175 and
///   DeployGranary.s.sol:54 both run
///   `if (token.minter() == deployer) token.setMinter(granary)`. So on a
///   depth-first launch the deployer holds NEITHER minter role, `rotate --arm`
///   reverts on `minter only`, and what the deployer holds instead is
///   `ScryGranary.steward` — whose `stewardMint` is UNCAPPED. The unlimited
///   mint did not retire. It changed doors.
///
/// That is the good ordering, not a bug: `minter()` ends up reading as a
/// rate-limited contract rather than another hot key, which is what the
/// rug-screen audience checks (DEGEN.md §1b). It just means the closing act is
/// `./deploy_town.sh steward` (transferSteward) rather than `rotate`.
///
/// Every test here is written as the *sequence a launch actually runs*, not as
/// a unit poke, because the defect this guards against is an ORDERING one and
/// a per-function test cannot see it.
contract AuthorityRetirementTest is Test {
    SpoilsToken myrrh;
    ScryGranary granary;

    address deployer; // the launch key — this contract, since it deploys
    address constant OFFLINE = address(0xC01D); // where a seat retires to
    address constant FARM = address(0xFA24); // stands in for ScryGardener
    address constant SILO = address(0x5170); // the organ that is granted LATER
    address constant ALICE = address(0xA11CE);

    uint256 constant DAILY = 6_000e18; // a posted grant, FARMING.md era-0 shape

    function setUp() public {
        deployer = address(this);
        // cap 0 == UNCAPPED, which is how `spoils` deploys both coins: elastic
        // supply is the design, and it is exactly why the mint ROLE is the
        // only thing standing between the launch and an infinite print.
        myrrh = new SpoilsToken("myrrh", "MYRRH", 0, deployer);
    }

    /// The handoff `gardener` / `granary` perform in passing.
    function _depthPhase() internal {
        granary = new ScryGranary(myrrh); // constructor: steward = msg.sender
        granary.setGrant(FARM, DAILY);
        if (myrrh.minter() == deployer) myrrh.setMinter(address(granary));
    }

    // ── 1. the ordering fact itself ────────────────────────────────────────
    function test_depth_phases_take_the_minter_so_rotate_cannot_run() public {
        assertEq(myrrh.minter(), deployer, "precondition: spoils leaves us the minter");
        _depthPhase();
        assertEq(myrrh.minter(), address(granary), "the slot moved to the granary");

        // This is `rotate`'s only act. After the depth phases it reverts — and
        // ROTATE_FORCE cannot help, because the role is provably not ours.
        vm.expectRevert("minter only");
        myrrh.setMinter(OFFLINE);
    }

    // ── 2. same power, different door ──────────────────────────────────────
    function test_the_unlimited_mint_moved_it_did_not_retire() public {
        _depthPhase();

        // The deployer is not the minter any more...
        vm.expectRevert("minter only");
        myrrh.mint(ALICE, 1);

        // ...and can still mint an arbitrary amount through the seat it kept.
        // No cap, no daily budget, no ceiling to hit: `stewardMint` is
        // `onlySteward` and nothing else (ScryGranary.sol:87).
        granary.stewardMint(ALICE, 1e30);
        granary.stewardMint(ALICE, 1e30);
        assertEq(myrrh.balanceOf(ALICE), 2e30, "steward mint is uncapped");
    }

    /// The handoff is TWO-STEP as of 2026-07-28, so "the seat moved" means the
    /// successor CLAIMED it, not that we offered it. Every test below that
    /// wants a retired seat goes through here.
    function _retireSeatTo(address next) internal {
        granary.transferSteward(next);
        vm.prank(next);
        granary.acceptSteward();
        assertEq(granary.steward(), next, "the seat moved on accept, not on propose");
    }

    // ── 3. the act that actually closes it ─────────────────────────────────
    /// Propose alone closes NOTHING, and that is the fix, not a gap: a
    /// single-step handoff to a typo welded the coin's mint authority to a
    /// granary nobody could drive — `stewardMint`, `setGrant`, `revokeGrant`
    /// and `rotateMinterAway` all gone in one transaction, and `deploy_town.sh`
    /// never calls this phase so its nonce/code typo filter never sees it.
    /// The incumbent keeps every door until the successor proves it can
    /// transact. This half of the test is what would fail if anyone made the
    /// handoff one-step again.
    function test_a_proposed_but_unclaimed_seat_still_drives_the_town() public {
        _depthPhase();
        granary.transferSteward(OFFLINE);
        assertEq(granary.pendingSteward(), OFFLINE, "proposed");
        assertEq(granary.steward(), deployer, "and NOT yet handed over");

        granary.stewardMint(ALICE, 1);
        granary.setGrant(SILO, DAILY);
        granary.revokeGrant(SILO);

        // and it is withdrawable right up until it is claimed
        granary.cancelStewardTransfer();
        assertEq(granary.pendingSteward(), address(0), "the proposal is withdrawn");
        vm.prank(OFFLINE);
        vm.expectRevert("not pending steward");
        granary.acceptSteward();
        assertEq(granary.steward(), deployer, "a cancelled proposal cannot be claimed later");
    }

    function test_the_claimed_handoff_closes_every_door_the_deployer_had() public {
        _depthPhase();
        _retireSeatTo(OFFLINE);

        vm.expectRevert("steward only");
        granary.stewardMint(ALICE, 1);
        vm.expectRevert("steward only");
        granary.setGrant(SILO, DAILY);
        vm.expectRevert("steward only");
        granary.revokeGrant(FARM);
        vm.expectRevert("steward only");
        granary.rotateMinterAway(OFFLINE);
        // and the other door was already shut by the handoff
        vm.expectRevert("minter only");
        myrrh.setMinter(OFFLINE);
        vm.expectRevert("minter only");
        myrrh.mint(ALICE, 1);
    }

    /// The retirement must not stop the town. A grant set BEFORE the seat moved
    /// keeps paying, on its posted cap, with nobody holding an unlimited key.
    function test_the_farm_keeps_paying_after_the_seat_retires() public {
        _depthPhase();
        _retireSeatTo(OFFLINE);

        vm.prank(FARM);
        uint256 minted = granary.mint(ALICE, DAILY);
        assertEq(minted, DAILY, "the granted daily mint survives the handover");

        // and it is still CLAMPED — the cap is what the granary bought us
        vm.prank(FARM);
        assertEq(granary.mint(ALICE, DAILY), 0, "same day, budget spent");
    }

    // ── 4. the trap the `steward` phase refuses on ─────────────────────────
    /// `setGrant` is `onlySteward` too, so an organ deployed but never granted
    /// becomes a permanently dry drip the moment the seat moves. `granary`
    /// prints "grant the silo AFTER it exists" and nothing enforced it; this is
    /// what that costs, and why `./deploy_town.sh steward` counts an ungranted
    /// silo as an unproven precondition and refuses.
    function test_an_organ_granted_after_the_seat_moves_is_dead_forever() public {
        _depthPhase(); // FARM is granted here; SILO deliberately is not
        _retireSeatTo(OFFLINE);

        vm.expectRevert("steward only");
        granary.setGrant(SILO, DAILY);

        vm.prank(SILO);
        vm.expectRevert("no grant");
        granary.mint(ALICE, 1);

        // Only the new holder can fix it — which is the whole point of the
        // precondition: from here it is somebody else's key or nothing.
        vm.prank(OFFLINE);
        granary.setGrant(SILO, DAILY);
        vm.prank(SILO);
        assertEq(granary.mint(ALICE, DAILY), DAILY, "the new steward can still grant");
    }

    // ── 5. the OTHER ordering, so both closing acts stay covered ───────────
    /// Depth phases AFTER the closing act: the deployer still holds `minter`,
    /// `rotate` is the right phase, and it works. Kept so a future change that
    /// breaks the rotate-first order fails here rather than on broadcast day.
    function test_depth_after_the_closing_act_leaves_rotate_working() public {
        myrrh.setMinter(OFFLINE); // `rotate --arm`
        assertEq(myrrh.minter(), OFFLINE);

        // and it is one-way for us
        vm.expectRevert("minter only");
        myrrh.setMinter(deployer);

        // the farm's later handoff is now the NEW holder's act, not ours —
        // exactly what deploy_town.sh's rotate block warns about
        ScryGranary late = new ScryGranary(myrrh);
        vm.prank(OFFLINE);
        myrrh.setMinter(address(late));
        assertEq(myrrh.minter(), address(late));
    }
}
