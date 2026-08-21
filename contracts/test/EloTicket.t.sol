// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloTicket.sol";

contract TicketGoodReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0x150b7a02;
    }
}

contract EloTicketTest is Test {
    EloTicket tick;
    address proceeds = address(0xCA5);
    address house = address(0x4055E); // where the gacha tranche lands
    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    address eidolon = address(0xE1D0);

    // launch-shaped tranches, scaled down so caps are cheap to exhaust:
    // trials 3 · snapshot 3 · gacha 3, the rest of the thousand open.
    uint256 constant TRIALS = 3;
    uint256 constant SNAPSHOT = 3;
    uint256 constant GACHA = 3;
    uint256 constant WINDOW = 2 hours;
    uint256 constant SNAP_PRICE = 0.01 ether; // "$25", posted in wei at arm time
    uint256 constant OPEN_PRICE = 0.04 ether; // "$100"

    function setUp() public {
        tick = new EloTicket(TRIALS, SNAPSHOT, GACHA, WINDOW, proceeds, 250, "https://elopros.com/api");
        tick.setEidolon(eidolon);
        vm.deal(alice, 10 ether);
        vm.deal(bob, 10 ether);
    }

    // ── merkle helpers (sorted-pair keccak, the eidolon_passes.py shape) ───────
    function _leaf(address a, uint256 al) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, al));
    }

    function _pair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return a <= b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
    }

    function _soloProof() internal pure returns (bytes32[] memory) {
        return new bytes32[](0);
    }

    /// Arm the snapshot for `who` with allowance `al`, opening one second from
    /// now at SNAP_PRICE, and warp into the window.
    function _openSnapshotFor(address who, uint256 al) internal {
        tick.armSnapshot(_leaf(who, al), SNAP_PRICE, block.timestamp + 1);
        vm.warp(block.timestamp + 1);
    }

    // ── constructor guards ─────────────────────────────────────────────────────
    function test_constructorRefusesOverSupplyCaps() public {
        vm.expectRevert(bytes("caps exceed supply"));
        new EloTicket(500, 400, 101, WINDOW, proceeds, 0, "x");
    }

    function test_constructorRefusesZeroProceedsAndWindow() public {
        vm.expectRevert(bytes("zero proceeds"));
        new EloTicket(1, 1, 1, WINDOW, address(0), 0, "x");
        vm.expectRevert(bytes("zero window"));
        new EloTicket(1, 1, 1, 0, proceeds, 0, "x");
    }

    function test_constructorRefusesFatRoyalty() public {
        vm.expectRevert(bytes("royalty > 10%"));
        new EloTicket(1, 1, 1, WINDOW, proceeds, 1001, "x");
    }

    // ── door 1: trials ─────────────────────────────────────────────────────────
    function test_trialsClosedUntilRootPosted() public {
        vm.prank(alice);
        vm.expectRevert(bytes("trials door closed"));
        tick.claimTrials(1, _soloProof());
    }

    function test_trialsClaimIsFreeAndTranched() public {
        tick.setTrialsRoot(_leaf(alice, 2));
        vm.prank(alice);
        uint256 id = tick.claimTrials(2, _soloProof());
        assertEq(tick.ownerOf(id), alice);
        assertEq(tick.trancheOf(id), 1);
        assertEq(tick.trialsMinted(), 1);
        assertEq(address(tick).balance, 0); // earned, not paid
    }

    function test_trialsAllowanceCapsPerWallet() public {
        tick.setTrialsRoot(_leaf(alice, 2));
        vm.startPrank(alice);
        tick.claimTrials(2, _soloProof());
        tick.claimTrials(2, _soloProof());
        vm.expectRevert(bytes("trials passes spent"));
        tick.claimTrials(2, _soloProof());
        vm.stopPrank();
    }

    function test_trialsRejectsForgedAllowanceAndStrangers() public {
        bytes32 la = _leaf(alice, 1);
        bytes32 lb = _leaf(bob, 1);
        tick.setTrialsRoot(_pair(la, lb));
        bytes32[] memory aProof = new bytes32[](1);
        aProof[0] = lb;
        vm.prank(alice);
        vm.expectRevert(bytes("no trials pass"));
        tick.claimTrials(5, aProof); // proof is for allowance 1, not 5
        vm.prank(address(0xC0FFEE));
        vm.expectRevert(bytes("no trials pass"));
        tick.claimTrials(1, _soloProof()); // not on the board at all
    }

    function test_trialsTrancheCapHolds() public {
        tick.setTrialsRoot(_leaf(alice, 10)); // allowance above the tranche cap
        vm.startPrank(alice);
        for (uint256 i = 0; i < TRIALS; i++) {
            tick.claimTrials(10, _soloProof());
        }
        vm.expectRevert(bytes("trials tranche spent"));
        tick.claimTrials(10, _soloProof());
        vm.stopPrank();
        assertEq(tick.trialsMinted(), TRIALS);
    }

    // ── door 2: snapshot ───────────────────────────────────────────────────────
    function test_snapshotClosedUntilArmedAndOpen() public {
        vm.prank(alice);
        vm.expectRevert(bytes("snapshot not open"));
        tick.claimSnapshot{value: SNAP_PRICE}(1, _soloProof());
        tick.armSnapshot(_leaf(alice, 1), SNAP_PRICE, block.timestamp + 100);
        vm.prank(alice);
        vm.expectRevert(bytes("snapshot not open")); // armed, not yet open
        tick.claimSnapshot{value: SNAP_PRICE}(1, _soloProof());
    }

    function test_armSnapshotRefusesPastOpeningAndZeroTerms() public {
        vm.expectRevert(bytes("opens in the past"));
        tick.armSnapshot(_leaf(alice, 1), SNAP_PRICE, block.timestamp);
        vm.expectRevert(bytes("zero terms"));
        tick.armSnapshot(bytes32(0), SNAP_PRICE, block.timestamp + 1);
        vm.expectRevert(bytes("zero terms"));
        tick.armSnapshot(_leaf(alice, 1), 0, block.timestamp + 1);
    }

    function test_snapshotBuyInsideTheWindow() public {
        _openSnapshotFor(alice, 1);
        vm.prank(alice);
        uint256 id = tick.claimSnapshot{value: SNAP_PRICE}(1, _soloProof());
        assertEq(tick.ownerOf(id), alice);
        assertEq(tick.trancheOf(id), 2);
        assertEq(address(tick).balance, SNAP_PRICE); // held until sweep, no owner path
    }

    function test_snapshotDemandsExactPayment() public {
        _openSnapshotFor(alice, 1);
        vm.startPrank(alice);
        vm.expectRevert(bytes("wrong payment"));
        tick.claimSnapshot{value: SNAP_PRICE - 1}(1, _soloProof());
        vm.expectRevert(bytes("wrong payment"));
        tick.claimSnapshot{value: SNAP_PRICE + 1}(1, _soloProof()); // no overpay-and-refund path
        vm.stopPrank();
    }

    function test_snapshotWindowCloses() public {
        _openSnapshotFor(alice, 1);
        vm.warp(block.timestamp + WINDOW); // one second past the last valid moment
        vm.prank(alice);
        vm.expectRevert(bytes("snapshot window closed"));
        tick.claimSnapshot{value: SNAP_PRICE}(1, _soloProof());
    }

    function test_snapshotRejectsStrangers() public {
        _openSnapshotFor(alice, 1);
        vm.prank(bob);
        vm.expectRevert(bytes("not in the snapshot"));
        tick.claimSnapshot{value: SNAP_PRICE}(1, _soloProof());
    }

    function test_snapshotAllowanceAndTrancheCaps() public {
        _openSnapshotFor(alice, 10);
        vm.startPrank(alice);
        for (uint256 i = 0; i < SNAPSHOT; i++) {
            tick.claimSnapshot{value: SNAP_PRICE}(10, _soloProof());
        }
        vm.expectRevert(bytes("snapshot tranche spent"));
        tick.claimSnapshot{value: SNAP_PRICE}(10, _soloProof());
        vm.stopPrank();
        // and a per-wallet allowance smaller than the tranche binds first.
        // (Absolute times: the compiler may legally cache a pre-warp
        // block.timestamp read within this frame, so don't re-read it.)
        EloTicket t2 = new EloTicket(0, 3, 0, WINDOW, proceeds, 0, "x");
        t2.armSnapshot(_leaf(bob, 1), SNAP_PRICE, 5000);
        vm.warp(5000);
        vm.startPrank(bob);
        t2.claimSnapshot{value: SNAP_PRICE}(1, _soloProof());
        vm.expectRevert(bytes("snapshot passes spent"));
        t2.claimSnapshot{value: SNAP_PRICE}(1, _soloProof());
        vm.stopPrank();
    }

    /// The terms freeze the moment the window opens: no mid-sale repricing.
    function test_snapshotTermsFreezeOnceOpen() public {
        tick.armSnapshot(_leaf(alice, 1), SNAP_PRICE, block.timestamp + 10);
        tick.armSnapshot(_leaf(alice, 2), SNAP_PRICE * 2, block.timestamp + 20); // re-arm before open is fine
        vm.warp(block.timestamp + 20);
        vm.expectRevert(bytes("snapshot already open"));
        tick.armSnapshot(_leaf(alice, 3), SNAP_PRICE, block.timestamp + 100);
    }

    // ── door 3: gacha tranche ──────────────────────────────────────────────────
    function test_gachaTrancheMintsToTheHouseUpToCap() public {
        tick.mintGacha(house, GACHA);
        assertEq(tick.balanceOf(house), GACHA);
        assertEq(tick.trancheOf(1), 3);
        vm.expectRevert(bytes("gacha tranche spent"));
        tick.mintGacha(house, 1);
    }

    function test_gachaMintIsOwnerOnly() public {
        vm.prank(alice);
        vm.expectRevert(bytes("not owner"));
        tick.mintGacha(alice, 1);
    }

    // ── door 4: public sale ────────────────────────────────────────────────────
    function test_publicClosedByDefaultAndByZeroPrice() public {
        vm.prank(alice);
        vm.expectRevert(bytes("public door closed"));
        tick.buyPublic{value: OPEN_PRICE}();
        tick.setPublicSale(OPEN_PRICE, 5);
        tick.setPublicSale(0, 5); // closing again is one knob
        vm.prank(alice);
        vm.expectRevert(bytes("public door closed"));
        tick.buyPublic{value: OPEN_PRICE}();
    }

    function test_publicBuyAndPostedCap() public {
        tick.setPublicSale(OPEN_PRICE, 2);
        vm.startPrank(alice);
        uint256 id = tick.buyPublic{value: OPEN_PRICE}();
        assertEq(tick.trancheOf(id), 4);
        tick.buyPublic{value: OPEN_PRICE}();
        vm.expectRevert(bytes("public tranche spent"));
        tick.buyPublic{value: OPEN_PRICE}();
        vm.stopPrank();
        assertEq(address(tick).balance, OPEN_PRICE * 2);
    }

    function test_publicDemandsExactPayment() public {
        tick.setPublicSale(OPEN_PRICE, 5);
        vm.prank(alice);
        vm.expectRevert(bytes("wrong payment"));
        tick.buyPublic{value: OPEN_PRICE - 1}();
    }

    /// A fat-fingered public cap can never oversell: the global thousand
    /// binds inside _mint no matter what the posted cap says.
    function test_globalThousandBindsOverAnyPublicCap() public {
        EloTicket t2 = new EloTicket(0, 0, 0, WINDOW, proceeds, 0, "x");
        t2.setPublicSale(0.001 ether, 2000); // cap posted above the thousand
        vm.deal(alice, 2 ether);
        vm.startPrank(alice);
        for (uint256 i = 0; i < 1000; i++) {
            t2.buyPublic{value: 0.001 ether}();
        }
        vm.expectRevert(bytes("the thousand tickets exist"));
        t2.buyPublic{value: 0.001 ether}();
        vm.stopPrank();
        assertEq(t2.totalMinted(), 1000);
    }

    // ── the burn ───────────────────────────────────────────────────────────────
    function test_onlyTheEidolonBurns() public {
        tick.setTrialsRoot(_leaf(alice, 1));
        vm.prank(alice);
        uint256 id = tick.claimTrials(1, _soloProof());
        vm.prank(alice); // not even the holder can burn directly
        vm.expectRevert(bytes("only the eidolon burns"));
        tick.burnFrom(alice, id);
        vm.prank(eidolon);
        vm.expectRevert(bytes("not the holder")); // and the eidolon only burns the true holder's
        tick.burnFrom(bob, id);
        vm.prank(eidolon);
        tick.burnFrom(alice, id);
        assertEq(tick.totalBurned(), 1);
        assertEq(tick.totalSupply(), 0);
        assertEq(tick.balanceOf(alice), 0);
        vm.expectRevert(bytes("no such ticket"));
        tick.ownerOf(id);
    }

    function test_setEidolonIsOneShot() public {
        vm.expectRevert(bytes("eidolon already set"));
        tick.setEidolon(address(0xBEEF));
        EloTicket t2 = new EloTicket(1, 1, 1, WINDOW, proceeds, 0, "x");
        vm.prank(alice);
        vm.expectRevert(bytes("not owner"));
        t2.setEidolon(address(0xBEEF));
        vm.expectRevert(bytes("zero eidolon"));
        t2.setEidolon(address(0));
    }

    // ── proceeds ───────────────────────────────────────────────────────────────
    function test_sweepPushesToTheImmutableProceeds() public {
        _openSnapshotFor(alice, 1);
        vm.prank(alice);
        tick.claimSnapshot{value: SNAP_PRICE}(1, _soloProof());
        uint256 before = proceeds.balance;
        vm.prank(bob); // anyone may sweep; nobody can redirect
        tick.sweep();
        assertEq(proceeds.balance - before, SNAP_PRICE);
        assertEq(address(tick).balance, 0);
    }

    function test_royaltyGoesToProceeds() public view {
        (address to, uint256 amt) = tick.royaltyInfo(1, 1 ether);
        assertEq(to, proceeds);
        assertEq(amt, 0.025 ether); // 2.5%
    }

    // ── knobs freeze on renounce ───────────────────────────────────────────────
    function test_renounceFreezesEveryKnob() public {
        tick.setTrialsRoot(_leaf(alice, 1));
        tick.transferOwnership(address(0)); // the posted door out
        vm.expectRevert(bytes("not owner"));
        tick.setTrialsRoot(bytes32(uint256(1)));
        vm.expectRevert(bytes("not owner"));
        tick.setPublicSale(1, 1);
        vm.expectRevert(bytes("not owner"));
        tick.mintGacha(house, 1);
        // the last posted trials root keeps working — frozen, not revoked
        vm.prank(alice);
        uint256 id = tick.claimTrials(1, _soloProof());
        assertEq(tick.ownerOf(id), alice);
    }

    // ── ERC-721 surface ────────────────────────────────────────────────────────
    function test_transferAndOperatorFlow() public {
        tick.setTrialsRoot(_leaf(alice, 1));
        vm.prank(alice);
        uint256 id = tick.claimTrials(1, _soloProof());
        vm.prank(bob);
        vm.expectRevert(bytes("not authorized"));
        tick.transferFrom(alice, bob, id);
        vm.prank(alice);
        tick.approve(bob, id);
        vm.prank(bob);
        tick.transferFrom(alice, bob, id);
        assertEq(tick.ownerOf(id), bob);
        assertEq(tick.getApproved(id), address(0));
    }

    function test_safeTransferChecksReceivers() public {
        tick.setTrialsRoot(_leaf(alice, 1));
        vm.prank(alice);
        uint256 id = tick.claimTrials(1, _soloProof());
        TicketGoodReceiver good = new TicketGoodReceiver();
        vm.prank(alice);
        tick.safeTransferFrom(alice, address(good), id);
        assertEq(tick.ownerOf(id), address(good));
    }

    function test_metadataAndInterfaces() public {
        tick.setTrialsRoot(_leaf(alice, 1));
        vm.prank(alice);
        uint256 id = tick.claimTrials(1, _soloProof());
        assertEq(tick.tokenURI(id), "https://elopros.com/api/ticket/1/metadata");
        assertEq(tick.contractURI(), "https://elopros.com/api/ticket/collection");
        assertTrue(tick.supportsInterface(0x01ffc9a7));
        assertTrue(tick.supportsInterface(0x80ac58cd));
        assertTrue(tick.supportsInterface(0x5b5e139f));
        assertTrue(tick.supportsInterface(0x2a55205a));
        assertFalse(tick.supportsInterface(0xdeadbeef));
        vm.expectRevert(bytes("no such ticket"));
        tick.tokenURI(999);
    }
}
