// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScrySeat.sol";

contract SeatGoodReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0x150b7a02;
    }
}

contract SeatBadReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0xdeadbeef;
    }
}

/// Plain no-callback ERC-20, which is what SCRY is.
contract MockSCRY {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 a) external {
        balanceOf[to] += a;
    }

    function approve(address s, uint256 a) external returns (bool) {
        allowance[msg.sender][s] = a;
        return true;
    }

    function transferFrom(address f, address t, uint256 a) external returns (bool) {
        require(balanceOf[f] >= a, "balance");
        require(allowance[f][msg.sender] >= a, "allowance");
        allowance[f][msg.sender] -= a;
        balanceOf[f] -= a;
        balanceOf[t] += a;
        return true;
    }
}

/// Refuses ETH — proves `sweep()` surfaces a failed push instead of eating it.
contract RejectingProceeds {}

/// Stands in for `ScrySeatArt`. It ECHOES its arguments rather than drawing
/// anything, which is the only thing worth asserting on this side of the hinge:
/// that the collection hands the renderer the seat's LIVE tier, its door and the
/// revealed salt. The art itself is gated by `art/seat/test_parity.py`, which
/// runs the real renderer in an EVM — duplicating that here would prove nothing
/// and drift.
contract SeatMockRenderer {
    function tokenURI(uint256 tokenId, uint8 tier, uint8 door, string calldata salt)
        external
        pure
        returns (string memory)
    {
        return string.concat(
            "R:", vm2s(tokenId), "/", vm2s(tier), "/", vm2s(door), "/", bytes(salt).length == 0 ? "sealed" : salt
        );
    }

    function contractURI() external pure returns (string memory) {
        return "R:collection";
    }

    function vm2s(uint256 v) internal pure returns (string memory) {
        if (v == 0) return "0";
        uint256 n = v;
        uint256 len;
        while (n != 0) {
            len++;
            n /= 10;
        }
        bytes memory s = new bytes(len);
        while (v != 0) {
            s[--len] = bytes1(uint8(48 + (v % 10)));
            v /= 10;
        }
        return string(s);
    }
}

contract ScrySeatTest is Test {
    ScrySeat seat;
    MockSCRY scry;

    address owner = address(this);
    address splitter = address(0xFEE);
    address proceeds = address(0xB0B);
    address alice = address(0xA11CE);
    address bob = address(0xB0BB1E);
    address carol = address(0xCA401);

    // The run: 100 seats — 20 snapshot, 10 play, 10 build, 60 paid float.
    uint256 constant SUPPLY = 100;
    uint256 constant SNAP_CAP = 20;
    uint256 constant PLAY_CAP = 10;
    uint256 constant BUILD_CAP = 10;
    uint256 constant TREASURY_CAP = 5;
    /// The reserve window, off. Every test that predates the window uses this,
    /// so they keep asserting the ORIGINAL strand-forever behaviour rather than
    /// silently re-testing the new one — the window gets its own tests below.
    uint256 constant NO_WINDOW = 0;
    uint256 constant PAID_FLOAT = SUPPLY - SNAP_CAP - PLAY_CAP - BUILD_CAP - TREASURY_CAP;

    string constant SALT = "the-sealed-deck-salt-for-this-run";

    /// ⚠ PRECOMPUTED IN `setUp`, AND THAT IS LOAD-BEARING, NOT TIDINESS.
    /// `sha256()` is precompile 0x02 — a real CALL. Written inline in a
    /// constructor's argument list it becomes THE NEXT CALL after
    /// `vm.expectRevert()`, which consumes the expectation on a call that
    /// succeeds, and every constructor-revert test fails with
    /// "next call did not revert as expected" while the constructor is never
    /// reached at all. Four tests here did exactly that. Read the gas: ~11k is
    /// a precompile, not a deploy.
    bytes32 saltCommit;

    function _ladder() internal pure returns (uint256[] memory costs, uint32[] memory weights) {
        // StonkBrokers' shape, scaled: 25x the capital buys 3.33x the weight.
        // Cumulative cost, so an upgrade pays the difference.
        costs = new uint256[](5);
        costs[0] = 66_666e18;
        costs[1] = 166_666e18;
        costs[2] = 366_666e18;
        costs[3] = 666_666e18;
        costs[4] = 1_666_666e18;
        weights = new uint32[](5);
        weights[0] = 100;
        weights[1] = 125;
        weights[2] = 160;
        weights[3] = 200;
        weights[4] = 333;
    }

    function _deploy(uint16 burnBps) internal returns (ScrySeat s) {
        (uint256[] memory costs, uint32[] memory weights) = _ladder();
        s = new ScrySeat(
            "scry seats",
            "SEAT",
            [SUPPLY, SNAP_CAP, PLAY_CAP, BUILD_CAP, TREASURY_CAP, NO_WINDOW],
            IERC20(address(scry)),
            splitter,
            proceeds,
            500,
            burnBps,
            saltCommit,
            costs,
            weights,
            "https://scry.moreright.xyz/api"
        );
    }

    function setUp() public {
        saltCommit = sha256(bytes(SALT));
        scry = new MockSCRY();
        seat = _deploy(5000); // 50% burns, the posted activation burn
    }

    // ── merkle helpers: a two-leaf tree, sorted-pair keccak ──────────────────
    function _leaf(address a, uint256 allowance) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(a, allowance));
    }

    function _pair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return a <= b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
    }

    /// Root of {(alice, aAllow), (bob, bAllow)} plus the proof for each side.
    function _twoLeafTree(uint256 aAllow, uint256 bAllow)
        internal
        view
        returns (bytes32 root, bytes32[] memory aProof, bytes32[] memory bProof)
    {
        bytes32 la = _leaf(alice, aAllow);
        bytes32 lb = _leaf(bob, bAllow);
        root = _pair(la, lb);
        aProof = new bytes32[](1);
        aProof[0] = lb;
        bProof = new bytes32[](1);
        bProof[0] = la;
    }

    // ═══ the ladder is sublinear, and the constructor is what enforces it ═══

    function test_constructor_rejects_a_linear_ladder() public {
        uint256[] memory costs = new uint256[](2);
        costs[0] = 100e18;
        costs[1] = 200e18;
        uint32[] memory weights = new uint32[](2);
        weights[0] = 100;
        weights[1] = 200; // exactly proportional — 2x capital, 2x weight
        vm.expectRevert("ladder not sublinear");
        new ScrySeat(
            "n",
            "N",
            [SUPPLY, SNAP_CAP, PLAY_CAP, BUILD_CAP, TREASURY_CAP, NO_WINDOW],
            IERC20(address(scry)),
            splitter,
            proceeds,
            500,
            5000,
            saltCommit,
            costs,
            weights,
            "u"
        );
    }

    function test_constructor_rejects_a_superlinear_ladder() public {
        uint256[] memory costs = new uint256[](2);
        costs[0] = 100e18;
        costs[1] = 200e18;
        uint32[] memory weights = new uint32[](2);
        weights[0] = 100;
        weights[1] = 400; // 2x capital buys 4x weight — the whale ladder
        vm.expectRevert("ladder not sublinear");
        new ScrySeat(
            "n",
            "N",
            [SUPPLY, SNAP_CAP, PLAY_CAP, BUILD_CAP, TREASURY_CAP, NO_WINDOW],
            IERC20(address(scry)),
            splitter,
            proceeds,
            500,
            5000,
            saltCommit,
            costs,
            weights,
            "u"
        );
    }

    function test_constructor_rejects_a_descending_cost() public {
        uint256[] memory costs = new uint256[](2);
        costs[0] = 200e18;
        costs[1] = 100e18;
        uint32[] memory weights = new uint32[](2);
        weights[0] = 100;
        weights[1] = 110;
        vm.expectRevert("ladder cost not ascending");
        new ScrySeat(
            "n",
            "N",
            [SUPPLY, SNAP_CAP, PLAY_CAP, BUILD_CAP, TREASURY_CAP, NO_WINDOW],
            IERC20(address(scry)),
            splitter,
            proceeds,
            500,
            5000,
            saltCommit,
            costs,
            weights,
            "u"
        );
    }

    function test_the_shipped_ladder_is_sublinear_and_matches_the_doc() public view {
        (uint256[] memory costs, uint32[] memory weights) = seat.ladder();
        // The posted ladder: "25x the capital buys 3.33x the weight."
        assertEq(costs[4] / costs[0], 25, "top tier is 25x the base cost");
        assertEq(uint256(weights[4]) * 100 / weights[0], 333, "top tier is 3.33x the base weight");
        // and the base is 7.5x more capital-efficient per token than the top.
        // ⚠ 1e36, not 1e18: weight-per-SCRY is ~1.5e-18, so an 18-decimal
        // fixed point truncates both sides to 0 and the assertion passes on
        // nothing. Scale above the costs' own 18 decimals before dividing.
        uint256 baseEff = uint256(weights[0]) * 1e36 / costs[0];
        uint256 topEff = uint256(weights[4]) * 1e36 / costs[4];
        assertApproxEqRel(baseEff * 100 / topEff, 750, 0.01e18, "base is ~7.5x more efficient");
    }

    function test_constructor_rejects_reserves_over_supply() public {
        (uint256[] memory costs, uint32[] memory weights) = _ladder();
        vm.expectRevert("reserved doors exceed supply");
        new ScrySeat(
            "n",
            "N",
            [uint256(10), uint256(5), uint256(5), uint256(5), uint256(0), uint256(0)],
            IERC20(address(scry)),
            splitter,
            proceeds,
            500,
            5000,
            saltCommit,
            costs,
            weights,
            "u"
        );
    }

    // ═══ the four doors ═══

    function test_snapshot_door_admits_the_cohort_and_nobody_else() public {
        (bytes32 root, bytes32[] memory aProof,) = _twoLeafTree(2, 1);
        seat.setDoorRoot(1, root);

        vm.prank(alice);
        uint256 id = seat.claim(1, 2, aProof);
        assertEq(id, 1);
        assertEq(seat.ownerOf(1), alice);
        assertEq(seat.doorOf(1), 1);

        // carol is in no cohort
        vm.prank(carol);
        vm.expectRevert("not in this cohort");
        seat.claim(1, 2, aProof);

        // alice cannot inflate her own allowance
        bytes32[] memory p = aProof;
        vm.prank(alice);
        vm.expectRevert("not in this cohort");
        seat.claim(1, 99, p);
    }

    function test_allowance_is_spent_not_reusable() public {
        (bytes32 root, bytes32[] memory aProof,) = _twoLeafTree(2, 1);
        seat.setDoorRoot(1, root);
        vm.startPrank(alice);
        seat.claim(1, 2, aProof);
        seat.claim(1, 2, aProof);
        vm.expectRevert("allowance spent");
        seat.claim(1, 2, aProof);
        vm.stopPrank();
        assertEq(seat.balanceOf(alice), 2);
    }

    function test_a_closed_door_admits_nobody() public {
        (, bytes32[] memory aProof,) = _twoLeafTree(2, 1);
        vm.prank(alice);
        vm.expectRevert("door closed");
        seat.claim(2, 2, aProof); // play door never rooted
    }

    function test_each_door_has_its_own_reserve() public {
        // Give the play door a root with a huge allowance and drain it.
        bytes32 la = _leaf(alice, 100);
        seat.setDoorRoot(2, la); // single-leaf tree: the leaf IS the root
        bytes32[] memory empty = new bytes32[](0);
        vm.startPrank(alice);
        for (uint256 i = 0; i < PLAY_CAP; i++) {
            seat.claim(2, 100, empty);
        }
        vm.expectRevert("door's reserve spent");
        seat.claim(2, 100, empty);
        vm.stopPrank();
        assertEq(seat.playMinted(), PLAY_CAP);
    }

    function test_paid_door_cannot_eat_a_reserved_door() public {
        seat.setPaidDoor(1 ether, 0);
        assertEq(seat.paidRemaining(), PAID_FLOAT);

        vm.deal(alice, 1000 ether);
        vm.startPrank(alice);
        for (uint256 i = 0; i < PAID_FLOAT; i++) {
            seat.buyWithEth{value: 1 ether}();
        }
        vm.expectRevert("paid door spent");
        seat.buyWithEth{value: 1 ether}();
        vm.stopPrank();

        // 60 sold, 40 still reserved and unminted — the run is NOT sold out
        assertEq(seat.totalMinted(), PAID_FLOAT);
        assertEq(seat.paidRemaining(), 0);
    }

    function test_an_unclaimed_reserve_is_never_released_to_the_float() public {
        // Nothing claims the snapshot/play/build doors at all.
        seat.setPaidDoor(1 ether, 0);
        vm.deal(alice, 1000 ether);
        vm.startPrank(alice);
        for (uint256 i = 0; i < PAID_FLOAT; i++) {
            seat.buyWithEth{value: 1 ether}();
        }
        vm.stopPrank();
        // The ceiling did not grow to absorb the 40 unclaimed reserved seats.
        assertEq(seat.paidRemaining(), 0, "reserves are not inventory");
    }

    function test_paid_door_legs_open_independently() public {
        vm.deal(alice, 10 ether);
        vm.prank(alice);
        vm.expectRevert("eth leg shut");
        seat.buyWithEth{value: 1 ether}();

        vm.prank(alice);
        vm.expectRevert("scry leg shut");
        seat.buyWithScry();

        seat.setPaidDoor(0, 1000e18); // SCRY leg only
        vm.prank(alice);
        vm.expectRevert("eth leg shut");
        seat.buyWithEth{value: 1 ether}();

        scry.mint(alice, 1000e18);
        vm.startPrank(alice);
        scry.approve(address(seat), 1000e18);
        seat.buyWithScry();
        vm.stopPrank();
        assertEq(scry.balanceOf(splitter), 1000e18, "scry mint proceeds go to the splitter");
    }

    function test_eth_payment_must_be_exact() public {
        seat.setPaidDoor(1 ether, 0);
        vm.deal(alice, 10 ether);
        vm.prank(alice);
        vm.expectRevert("wrong payment");
        seat.buyWithEth{value: 0.9 ether}();
        vm.prank(alice);
        vm.expectRevert("wrong payment");
        seat.buyWithEth{value: 1.1 ether}();
    }

    /// ⚠ REPLACED 2026-08-08. This used to assert that no selector could mint to
    /// an address of the owner's choosing. The operator added a treasury mint —
    /// *"idgaf about no team mint thats dumb lol"* — so that assertion is simply
    /// false now, and a test that no longer describes the contract is worse than
    /// no test. What remains true, and what a rug screen actually reads, is that
    /// the allocation is BOUNDED by a number fixed before mint #1.
    function test_the_treasury_mint_is_capped_and_cannot_exceed_it() public {
        assertEq(seat.treasuryCap(), TREASURY_CAP);
        assertEq(seat.treasuryMinted(), 0);

        seat.mintTreasury(carol, TREASURY_CAP);
        assertEq(seat.balanceOf(carol), TREASURY_CAP);
        assertEq(seat.treasuryMinted(), TREASURY_CAP);
        assertEq(seat.doorOf(1), 5, "treasury seats carry their own door number");

        vm.expectRevert("treasury allocation spent");
        seat.mintTreasury(carol, 1);
    }

    function test_only_the_owner_mints_the_treasury() public {
        vm.prank(alice);
        vm.expectRevert("not owner");
        seat.mintTreasury(alice, 1);
    }

    /// ⭐ The bound that matters: the treasury is carved OUT of supply, so it can
    /// never reach into the paid float or into a reserved door.
    function test_the_treasury_cannot_eat_the_float_or_a_reserved_door() public {
        assertEq(seat.paidRemaining(), PAID_FLOAT, "the float is already net of the treasury carve-out");
        seat.mintTreasury(carol, TREASURY_CAP);
        assertEq(seat.paidRemaining(), PAID_FLOAT, "minting the treasury did not shrink the float");

        // and the float still sells out to exactly its own size
        seat.setPaidDoor(1 ether, 0);
        vm.deal(alice, 1000 ether);
        vm.startPrank(alice);
        for (uint256 i = 0; i < PAID_FLOAT; i++) {
            seat.buyWithEth{value: 1 ether}();
        }
        vm.expectRevert("paid door spent");
        seat.buyWithEth{value: 1 ether}();
        vm.stopPrank();
    }

    function test_treasury_mint_refuses_zero_and_zero_address() public {
        vm.expectRevert("zero to");
        seat.mintTreasury(address(0), 1);
        vm.expectRevert("nothing to mint");
        seat.mintTreasury(carol, 0);
    }

    // ═══ activation: the ladder, the welded burn ═══

    function _mintOneTo(address who) internal returns (uint256) {
        seat.setPaidDoor(1 ether, 0);
        vm.deal(who, 10 ether);
        vm.prank(who);
        return seat.buyWithEth{value: 1 ether}();
    }

    function test_activation_splits_exactly_at_the_welded_bps() public {
        uint256 id = _mintOneTo(alice);
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[0]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[0]);
        seat.activate(id, 1);
        vm.stopPrank();

        assertEq(seat.tierOf(id), 1);
        assertEq(seat.weightOf(id), 100);
        assertEq(scry.balanceOf(seat.BURN()), costs[0] / 2, "half burns");
        assertEq(scry.balanceOf(splitter), costs[0] / 2, "half to the splitter");
        assertEq(seat.totalSentToBurn(), costs[0] / 2);
        assertEq(scry.balanceOf(address(seat)), 0, "nothing sticks to the contract");
    }

    function test_upgrading_pays_only_the_difference() public {
        uint256 id = _mintOneTo(alice);
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[4]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[4]);
        seat.activate(id, 1);
        seat.activate(id, 4);
        vm.stopPrank();

        assertEq(seat.tierOf(id), 4);
        assertEq(seat.weightOf(id), 200);
        // paid costs[0] then costs[3]-costs[0]: total is exactly costs[3]
        assertEq(scry.balanceOf(alice), costs[4] - costs[3], "never charged twice for ground held");
    }

    function test_cannot_downgrade_or_re_pay_a_tier() public {
        uint256 id = _mintOneTo(alice);
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[4] * 2);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[4] * 2);
        seat.activate(id, 3);
        vm.expectRevert("already at or above that tier");
        seat.activate(id, 3);
        vm.expectRevert("already at or above that tier");
        seat.activate(id, 1);
        vm.stopPrank();
    }

    function test_only_the_holder_activates() public {
        uint256 id = _mintOneTo(alice);
        (uint256[] memory costs,) = _ladder();
        scry.mint(bob, costs[0]);
        vm.startPrank(bob);
        scry.approve(address(seat), costs[0]);
        vm.expectRevert("not your seat");
        seat.activate(id, 1);
        vm.stopPrank();
    }

    function test_no_such_tier() public {
        uint256 id = _mintOneTo(alice);
        vm.startPrank(alice);
        vm.expectRevert("no such tier");
        seat.activate(id, 0);
        vm.expectRevert("no such tier");
        seat.activate(id, 6);
        vm.stopPrank();
    }

    function test_a_zero_burn_deploy_sends_everything_to_the_splitter() public {
        ScrySeat s = _deploy(0);
        s.setPaidDoor(1 ether, 0);
        vm.deal(alice, 10 ether);
        vm.prank(alice);
        uint256 id = s.buyWithEth{value: 1 ether}();
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[0]);
        vm.startPrank(alice);
        scry.approve(address(s), costs[0]);
        s.activate(id, 1);
        vm.stopPrank();
        assertEq(s.totalSentToBurn(), 0);
        assertEq(scry.balanceOf(splitter), costs[0]);
    }

    // ═══ the clear on transfer — the recurring burn ═══

    function test_tier_clears_on_transfer_and_the_buyer_re_tiers() public {
        uint256 id = _mintOneTo(alice);
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[4]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[4]);
        seat.activate(id, 4);
        assertEq(seat.tierOf(id), 4);
        seat.transferFrom(alice, bob, id);
        vm.stopPrank();

        assertEq(seat.ownerOf(id), bob);
        assertEq(seat.tierOf(id), 0, "the tier did not travel");
        assertEq(seat.weightOf(id), 0, "a sold seat is benched");

        // bob re-tiers at his own cost — the burn happens a second time
        uint256 burnedBefore = seat.totalSentToBurn();
        scry.mint(bob, costs[4]);
        vm.startPrank(bob);
        scry.approve(address(seat), costs[4]);
        seat.activate(id, 4);
        vm.stopPrank();
        assertGt(seat.totalSentToBurn(), burnedBefore, "secondary sales burn again");
    }

    function test_the_clear_covers_every_transfer_path() public {
        (uint256[] memory costs,) = _ladder();

        // path 1: an approved operator moves it (a marketplace)
        uint256 id = _mintOneTo(alice);
        scry.mint(alice, costs[0] * 3);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[0] * 3);
        seat.activate(id, 1);
        seat.setApprovalForAll(carol, true);
        vm.stopPrank();
        vm.prank(carol);
        seat.transferFrom(alice, bob, id);
        assertEq(seat.tierOf(id), 0, "operator transfer clears");

        // path 2: safeTransferFrom into a contract
        SeatGoodReceiver rx = new SeatGoodReceiver();
        vm.startPrank(bob);
        scry.mint(bob, costs[0]);
        scry.approve(address(seat), costs[0]);
        seat.activate(id, 1);
        assertEq(seat.tierOf(id), 1);
        seat.safeTransferFrom(bob, address(rx), id);
        vm.stopPrank();
        assertEq(seat.tierOf(id), 0, "safeTransfer clears");
    }

    function test_minting_does_not_clear_anything() public {
        uint256 id = _mintOneTo(alice);
        assertEq(seat.tierOf(id), 0);
        assertEq(seat.doorOf(id), 4, "the door survives; only tier and election clear");
    }

    // ═══ the election ═══

    function test_election_is_recorded_and_read_back() public {
        uint256 id = _mintOneTo(alice);
        uint16[3] memory tracks = [uint16(7), uint16(9), uint16(0)];
        uint16[3] memory weights = [uint16(6000), uint16(4000), uint16(0)];
        vm.prank(alice);
        seat.setElection(id, tracks, weights);
        (uint16[3] memory gotT, uint16[3] memory gotW) = seat.electionOf(id);
        assertEq(gotT[0], 7);
        assertEq(gotW[0], 6000);
        assertEq(gotT[1], 9);
        assertEq(gotW[1], 4000);
    }

    function test_election_weights_must_sum_to_ten_thousand() public {
        uint256 id = _mintOneTo(alice);
        uint16[3] memory tracks = [uint16(1), uint16(2), uint16(0)];
        uint16[3] memory bad = [uint16(6000), uint16(3000), uint16(0)];
        vm.prank(alice);
        vm.expectRevert("weights must sum to 10000");
        seat.setElection(id, tracks, bad);
    }

    function test_all_in_on_one_track_is_fine() public {
        uint256 id = _mintOneTo(alice);
        uint16[3] memory tracks = [uint16(4), uint16(0), uint16(0)];
        uint16[3] memory weights = [uint16(10_000), uint16(0), uint16(0)];
        vm.prank(alice);
        seat.setElection(id, tracks, weights);
        (, uint16[3] memory gotW) = seat.electionOf(id);
        assertEq(gotW[0], 10_000);
    }

    function test_election_rejects_a_duplicate_track() public {
        uint256 id = _mintOneTo(alice);
        uint16[3] memory tracks = [uint16(5), uint16(5), uint16(0)];
        uint16[3] memory weights = [uint16(5000), uint16(5000), uint16(0)];
        vm.prank(alice);
        vm.expectRevert("duplicate track");
        seat.setElection(id, tracks, weights);
    }

    function test_no_election_on_file_reads_as_all_zero() public {
        uint256 id = _mintOneTo(alice);
        (uint16[3] memory t, uint16[3] memory w) = seat.electionOf(id);
        for (uint256 i = 0; i < 3; i++) {
            assertEq(t[i], 0);
            assertEq(w[i], 0);
        }
    }

    function test_election_clears_on_transfer_with_the_tier() public {
        uint256 id = _mintOneTo(alice);
        uint16[3] memory tracks = [uint16(7), uint16(0), uint16(0)];
        uint16[3] memory weights = [uint16(10_000), uint16(0), uint16(0)];
        vm.startPrank(alice);
        seat.setElection(id, tracks, weights);
        seat.transferFrom(alice, bob, id);
        vm.stopPrank();
        (uint16[3] memory t, uint16[3] memory w) = seat.electionOf(id);
        assertEq(t[0], 0, "the election did not travel");
        assertEq(w[0], 0);
    }

    function test_only_the_holder_elects() public {
        uint256 id = _mintOneTo(alice);
        uint16[3] memory tracks = [uint16(1), uint16(0), uint16(0)];
        uint16[3] memory weights = [uint16(10_000), uint16(0), uint16(0)];
        vm.prank(bob);
        vm.expectRevert("not your seat");
        seat.setElection(id, tracks, weights);
    }

    // ═══ the sealed deck ═══

    function test_salt_cannot_be_revealed_while_the_mint_is_open() public {
        _mintOneTo(alice);
        vm.expectRevert("mint still open");
        seat.revealSalt(SALT);
    }

    function test_a_wrong_salt_is_refused() public {
        seat.closeMint();
        vm.expectRevert("salt does not match the commitment");
        seat.revealSalt("a more convenient salt");
    }

    function test_reveal_after_close_and_only_once() public {
        _mintOneTo(alice);
        seat.closeMint();
        seat.revealSalt(SALT);
        assertEq(seat.revealedSalt(), SALT);
        assertEq(sha256(bytes(seat.revealedSalt())), seat.saltCommitment(), "recomputable by anyone");
        vm.expectRevert("already revealed");
        seat.revealSalt(SALT);
    }

    function test_closing_the_mint_shuts_every_door() public {
        (bytes32 root, bytes32[] memory aProof,) = _twoLeafTree(2, 1);
        seat.setDoorRoot(1, root);
        seat.setPaidDoor(1 ether, 1000e18);
        seat.closeMint();

        vm.deal(alice, 10 ether);
        scry.mint(alice, 1000e18);
        vm.startPrank(alice);
        vm.expectRevert("the mint is closed");
        seat.claim(1, 2, aProof);
        vm.expectRevert("the mint is closed");
        seat.buyWithEth{value: 1 ether}();
        scry.approve(address(seat), 1000e18);
        vm.expectRevert("the mint is closed");
        seat.buyWithScry();
        vm.stopPrank();

        vm.expectRevert("already closed");
        seat.closeMint();
    }

    function test_activation_and_transfer_still_work_after_close() public {
        uint256 id = _mintOneTo(alice);
        seat.closeMint();
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[0]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[0]);
        seat.activate(id, 1); // the run is closed; the seats are alive
        seat.transferFrom(alice, bob, id);
        vm.stopPrank();
        assertEq(seat.ownerOf(id), bob);
    }

    // ═══ proceeds and royalties ═══

    function test_sweep_pushes_to_the_welded_address_and_anyone_may_call() public {
        seat.setPaidDoor(1 ether, 0);
        vm.deal(alice, 10 ether);
        vm.prank(alice);
        seat.buyWithEth{value: 1 ether}();
        assertEq(address(seat).balance, 1 ether);

        vm.prank(carol); // a stranger pushes it
        seat.sweep();
        assertEq(proceeds.balance, 1 ether);
        assertEq(address(seat).balance, 0);

        vm.expectRevert("nothing to sweep");
        seat.sweep();
    }

    function test_a_failed_sweep_reverts_rather_than_silently_succeeding() public {
        (uint256[] memory costs, uint32[] memory weights) = _ladder();
        address rejecting = address(new RejectingProceeds());
        ScrySeat s = new ScrySeat(
            "n",
            "N",
            [SUPPLY, SNAP_CAP, PLAY_CAP, BUILD_CAP, TREASURY_CAP, NO_WINDOW],
            IERC20(address(scry)),
            splitter,
            rejecting,
            500,
            5000,
            saltCommit,
            costs,
            weights,
            "u"
        );
        s.setPaidDoor(1 ether, 0);
        vm.deal(alice, 10 ether);
        vm.prank(alice);
        s.buyWithEth{value: 1 ether}();
        vm.expectRevert("sweep failed");
        s.sweep();
    }

    function test_royalties_point_at_the_splitter() public view {
        (address to, uint256 amount) = seat.royaltyInfo(1, 10_000);
        assertEq(to, splitter);
        assertEq(amount, 500); // 5%
    }

    function test_interfaces() public view {
        assertTrue(seat.supportsInterface(0x01ffc9a7));
        assertTrue(seat.supportsInterface(0x80ac58cd));
        assertTrue(seat.supportsInterface(0x5b5e139f));
        assertTrue(seat.supportsInterface(0x49064906));
        assertTrue(seat.supportsInterface(0x2a55205a));
        assertFalse(seat.supportsInterface(0xdeadbeef));
    }

    // ═══ owner powers stop where the doc says they stop ═══

    function test_owner_cannot_move_or_tier_a_holders_seat() public {
        uint256 id = _mintOneTo(alice);
        vm.expectRevert("not authorized");
        seat.transferFrom(alice, owner, id); // owner is address(this)
        vm.expectRevert("not your seat");
        seat.activate(id, 1);
    }

    function test_only_owner_holds_the_knobs() public {
        vm.startPrank(alice);
        vm.expectRevert("not owner");
        seat.setDoorRoot(1, bytes32(uint256(1)));
        vm.expectRevert("not owner");
        seat.setPaidDoor(1 ether, 0);
        vm.expectRevert("not owner");
        seat.setBaseURI("x");
        vm.expectRevert("not owner");
        seat.closeMint();
        vm.expectRevert("not owner");
        seat.revealSalt(SALT);
        vm.stopPrank();
    }

    function test_renouncing_freezes_every_knob() public {
        seat.transferOwnership(address(0));
        vm.expectRevert("not owner");
        seat.setDoorRoot(1, bytes32(uint256(1)));
        vm.expectRevert("not owner");
        seat.setPaidDoor(1 ether, 0);
    }

    function test_rerooting_a_door_does_not_reset_what_a_wallet_already_took() public {
        (bytes32 root, bytes32[] memory aProof,) = _twoLeafTree(1, 1);
        seat.setDoorRoot(1, root);
        vm.prank(alice);
        seat.claim(1, 1, aProof);
        vm.prank(alice);
        vm.expectRevert("allowance spent");
        seat.claim(1, 1, aProof);

        seat.setDoorRoot(1, bytes32(0));
        seat.setDoorRoot(1, root);
        vm.prank(alice);
        vm.expectRevert("allowance spent");
        seat.claim(1, 1, aProof);
        assertEq(seat.claimedAtDoor(1, alice), 1);
    }

    /// ⚠ THE ONE THAT CAUGHT A REAL OVER-MINT. The play and build doors are
    /// re-rooted as more wallets earn — that is what those doors ARE. With the
    /// claimed counter keyed by ROOT (the first cut of this contract), a wallet
    /// whose cumulative allowance rose 1 -> 3 could take 3 MORE on top of the 1
    /// it already had, for 4. Keyed by DOOR it takes exactly 2 more, for 3.
    function test_a_new_root_grants_the_difference_and_never_a_fresh_budget() public {
        (bytes32 root1, bytes32[] memory p1,) = _twoLeafTree(1, 1);
        seat.setDoorRoot(2, root1);
        vm.prank(alice);
        seat.claim(2, 1, p1); // alice has earned 1 and taken 1

        // The play ledger grows: alice's CUMULATIVE lifetime allowance is now 3.
        bytes32 la = _leaf(alice, 3);
        bytes32 lb = _leaf(bob, 1);
        bytes32 root2 = _pair(la, lb);
        bytes32[] memory p2 = new bytes32[](1);
        p2[0] = lb;
        seat.setDoorRoot(2, root2);

        vm.startPrank(alice);
        seat.claim(2, 3, p2);
        seat.claim(2, 3, p2);
        vm.expectRevert("allowance spent"); // 3 total, not 4
        seat.claim(2, 3, p2);
        vm.stopPrank();

        assertEq(seat.balanceOf(alice), 3, "cumulative allowance is a ceiling, not a refill");
        assertEq(seat.claimedAtDoor(2, alice), 3);
    }

    // ═══ ERC-721 hygiene ═══

    function test_unsafe_recipient_is_refused() public {
        uint256 id = _mintOneTo(alice);
        SeatBadReceiver rx = new SeatBadReceiver();
        vm.prank(alice);
        vm.expectRevert("unsafe recipient");
        seat.safeTransferFrom(alice, address(rx), id);
    }

    function test_token_uri_shape() public {
        uint256 id = _mintOneTo(alice);
        assertEq(seat.tokenURI(id), "https://scry.moreright.xyz/api/seat/1/metadata");
        assertEq(seat.contractURI(), "https://scry.moreright.xyz/api/seat/collection");
        vm.expectRevert("no such seat");
        seat.tokenURI(999);
    }

    // ═══ the renderer hinge — AND IT IS THE PATH THAT SHIPS ═══
    // ⚠ Every metadata test above this line exercises `renderer == address(0)`,
    // the baseURI bootstrap. `DeploySeat.s.sol` wires `ScrySeatArt` on the same
    // broadcast that deploys the collection, so the fallback is what NOTHING in
    // production uses — and the delegation, the freeze and the ERC-4906 notices
    // that ride them had no coverage on either side of the hinge at all.

    function test_the_renderer_is_handed_the_seats_live_tier_door_and_salt() public {
        uint256 id = _mintOneTo(alice); // door 4
        seat.setRenderer(address(new SeatMockRenderer()));
        assertEq(seat.tokenURI(id), "R:1/0/4/sealed", "benched, sealed, paid door");

        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[2]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[2]);
        seat.activate(id, 3);
        vm.stopPrank();
        assertEq(seat.tokenURI(id), "R:1/3/4/sealed", "the tier the renderer sees is the LIVE one");

        seat.closeMint();
        seat.revealSalt(SALT);
        assertEq(seat.tokenURI(id), string.concat("R:1/3/4/", SALT), "and the salt arrives on reveal");
        assertEq(seat.contractURI(), "R:collection", "the collection document delegates too");
    }

    function test_setRenderer_zero_takes_the_built_in_path_back() public {
        uint256 id = _mintOneTo(alice);
        seat.setRenderer(address(new SeatMockRenderer()));
        assertEq(seat.tokenURI(id), "R:1/0/4/sealed");
        seat.setRenderer(address(0));
        assertEq(seat.tokenURI(id), "https://scry.moreright.xyz/api/seat/1/metadata");
        assertEq(seat.contractURI(), "https://scry.moreright.xyz/api/seat/collection");
    }

    /// ⚠ ONE WAY, FROM EVERY KEY, FOREVER. This is the answer to the question
    /// this audience actually asks of an NFT — *can the team change my picture* —
    /// and it has to be arithmetic rather than a promise.
    function test_freezeMetadata_is_one_way_and_shuts_both_pointers() public {
        _mintOneTo(alice);
        address r = address(new SeatMockRenderer());
        seat.setRenderer(r);
        assertFalse(seat.metadataFrozen());

        seat.freezeMetadata();
        assertTrue(seat.metadataFrozen());
        assertEq(seat.renderer(), r, "the pointer freezes where it stands");

        vm.expectRevert("metadata frozen");
        seat.setRenderer(address(0));
        vm.expectRevert("metadata frozen");
        seat.setBaseURI("ipfs://somewhere-else");
        vm.expectRevert("already frozen");
        seat.freezeMetadata();

        // and the owner's own key buys nothing back
        seat.transferOwnership(alice);
        vm.prank(alice);
        vm.expectRevert("metadata frozen");
        seat.setRenderer(address(0));
    }

    function test_only_the_owner_moves_metadata() public {
        vm.startPrank(alice);
        vm.expectRevert("not owner");
        seat.setRenderer(address(1));
        vm.expectRevert("not owner");
        seat.freezeMetadata();
        vm.stopPrank();
    }

    // ═══ ERC-4906: the collection declares it, so it has to emit it ═══
    // ⚠ `MetadataUpdate` was DECLARED AND NEVER EMITTED. With a renderer wired,
    // tier is a live attribute of the token — so an activation and a
    // clear-on-transfer both rewrite the served JSON, and a marketplace goes on
    // serving its cache until an event says otherwise. `supportsInterface`
    // returns true for 0x49064906; that has to be a fact rather than a claim.

    event MetadataUpdate(uint256 _tokenId); // mirrored locally for expectEmit

    function test_activating_emits_the_erc4906_notice() public {
        uint256 id = _mintOneTo(alice);
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[0]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[0]);
        vm.expectEmit(true, true, true, true, address(seat));
        emit MetadataUpdate(id);
        seat.activate(id, 1);
        vm.stopPrank();
    }

    function test_the_tier_clear_emits_the_notice_too() public {
        uint256 id = _mintOneTo(alice);
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[0]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[0]);
        seat.activate(id, 1);
        vm.expectEmit(true, true, true, true, address(seat));
        emit MetadataUpdate(id);
        seat.transferFrom(alice, bob, id);
        vm.stopPrank();
        assertEq(seat.tierOf(id), 0);
    }

    /// The mirror image, and the reason the emit sits INSIDE the `held != 0`
    /// branch: moving a benched seat changes no served byte, so it owes no
    /// notice. An event on every transfer would be noise a marketplace pays to
    /// re-fetch nothing.
    function test_a_transfer_that_clears_nothing_emits_no_notice() public {
        uint256 id = _mintOneTo(alice);
        vm.recordLogs();
        vm.prank(alice);
        seat.transferFrom(alice, bob, id);
        Vm.Log[] memory logs = vm.getRecordedLogs();
        bytes32 sig = keccak256("MetadataUpdate(uint256)");
        for (uint256 i = 0; i < logs.length; i++) {
            assertTrue(logs[i].topics[0] != sig, "a benched seat emitted a metadata notice");
        }
        assertEq(logs.length, 1, "one Transfer, and nothing else");
    }

    function test_balances_track_transfers() public {
        uint256 id = _mintOneTo(alice);
        assertEq(seat.balanceOf(alice), 1);
        vm.prank(alice);
        seat.transferFrom(alice, bob, id);
        assertEq(seat.balanceOf(alice), 0);
        assertEq(seat.balanceOf(bob), 1);
        assertEq(seat.totalSupply(), 1, "nothing burns a seat");
    }

    // ═══ the wall (CLAUDE.md invariant 1) ═══

    function test_the_contract_stores_no_record_and_no_measurement() public {
        // Asserted by ABI absence: a seat's entire on-chain state is a tier, a
        // door, a mint time and an election. There is no round, clear, badge,
        // rank, score or meter number to transfer, and therefore nothing a
        // bought seat can launder.
        uint256 id = _mintOneTo(alice);
        assertEq(seat.doorOf(id), 4);
        assertEq(seat.tierOf(id), 0);
        assertGt(seat.mintedAt(id), 0);
    }

    function test_tier_changes_weight_and_nothing_else_on_chain() public {
        uint256 id = _mintOneTo(alice);
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[4]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[4]);
        uint64 mintedBefore = seat.mintedAt(id);
        uint8 doorBefore = seat.doorOf(id);
        seat.activate(id, 5);
        vm.stopPrank();
        assertEq(seat.weightOf(id), 333, "weight is the ONLY thing money moved");
        assertEq(seat.mintedAt(id), mintedBefore);
        assertEq(seat.doorOf(id), doorBefore);
        assertEq(seat.tokenURI(id), "https://scry.moreright.xyz/api/seat/1/metadata", "paying changed no metadata path");
    }

    // ═══ RUNBOOK §0c — this is Arbitrum Nitro ═══

    function test_nothing_here_is_paced_in_block_number() public {
        uint256 id = _mintOneTo(alice);
        uint64 stamped = seat.mintedAt(id);
        assertApproxEqAbs(stamped, uint64(block.timestamp), 1, "mintedAt is a timestamp");
        // Roll the parent-chain block number a long way; nothing in this
        // contract reads it, so no behaviour moves.
        vm.roll(block.number + 1_000_000);
        (uint256[] memory costs,) = _ladder();
        scry.mint(alice, costs[0]);
        vm.startPrank(alice);
        scry.approve(address(seat), costs[0]);
        seat.activate(id, 1);
        vm.stopPrank();
        assertEq(seat.tierOf(id), 1);
        assertEq(seat.mintedAt(id), stamped);
    }

    // ── the reserve window ───────────────────────────────────────────────────
    //
    // The answer to "what if the free mint does not mint out." A welded date;
    // after it the three free doors shut and their remainder is public float.
    // What these pin is that nobody can TIME it and nothing can be minted twice
    // out of the same allocation.

    function _deployWithWindow(uint256 until_) internal returns (ScrySeat s) {
        (uint256[] memory costs, uint32[] memory weights) = _ladder();
        s = new ScrySeat(
            "scry seats",
            "SEAT",
            [SUPPLY, SNAP_CAP, PLAY_CAP, BUILD_CAP, TREASURY_CAP, until_],
            IERC20(address(scry)),
            splitter,
            proceeds,
            500,
            5000,
            saltCommit,
            costs,
            weights,
            "https://scry.moreright.xyz/api"
        );
    }

    function test_no_window_strands_an_unclaimed_reserve_exactly_as_before() public {
        // the original decision, kept reachable: paidRemaining never grows
        assertEq(seat.reservedUntil(), 0);
        assertFalse(seat.reserveReleased());
        assertEq(seat.paidRemaining(), PAID_FLOAT);
        vm.warp(block.timestamp + 3650 days);
        assertFalse(seat.reserveReleased());
        assertEq(seat.paidRemaining(), PAID_FLOAT);
    }

    function test_the_window_hands_an_unclaimed_reserve_to_the_float() public {
        ScrySeat s = _deployWithWindow(block.timestamp + 30 days);
        assertEq(s.paidRemaining(), PAID_FLOAT);

        // one seat leaves by the snapshot door before the window closes
        bytes32 la = _leaf(alice, 1);
        s.setDoorRoot(1, la);
        bytes32[] memory empty = new bytes32[](0);
        vm.prank(alice);
        s.claim(1, 1, empty);

        vm.warp(block.timestamp + 30 days);
        assertTrue(s.reserveReleased());
        // everything the three doors never drew, less the one that left
        assertEq(s.paidRemaining(), PAID_FLOAT + SNAP_CAP + PLAY_CAP + BUILD_CAP - 1);
    }

    function test_the_free_doors_shut_when_the_window_does() public {
        ScrySeat s = _deployWithWindow(block.timestamp + 1 days);
        bytes32 la = _leaf(alice, 5);
        s.setDoorRoot(1, la);
        bytes32[] memory empty = new bytes32[](0);
        vm.prank(alice);
        s.claim(1, 5, empty); // fine, the window is open

        vm.warp(block.timestamp + 1 days);
        vm.prank(alice);
        vm.expectRevert("the reserve window has closed");
        s.claim(1, 5, empty);
    }

    /// ⚠ THE DOUBLE-SPEND THE SHUT DOOR EXISTS TO PREVENT. If a free door kept
    /// claiming after release, its seat would come out of an allocation the paid
    /// ceiling had already counted, and the two together would exceed supply.
    function test_the_window_never_lets_supply_be_counted_twice() public {
        ScrySeat s = _deployWithWindow(block.timestamp + 1 days);
        bytes32 la = _leaf(alice, 100);
        s.setDoorRoot(1, la);
        s.setPaidDoor(1 ether, 0);
        vm.warp(block.timestamp + 1 days);

        uint256 float_ = s.paidRemaining();
        vm.deal(alice, 10_000 ether);
        vm.startPrank(alice);
        for (uint256 i = 0; i < float_; i++) {
            s.buyWithEth{value: 1 ether}();
        }
        vm.expectRevert("paid door spent");
        s.buyWithEth{value: 1 ether}();
        vm.stopPrank();

        // supply less the treasury carve-out, and not one seat more
        assertEq(s.totalSupply(), SUPPLY - TREASURY_CAP);
    }

    function test_a_window_already_in_the_past_cannot_be_deployed() public {
        vm.warp(1_000_000);
        vm.expectRevert("reserve window already closed");
        _deployWithWindow(block.timestamp - 1);
        vm.expectRevert("reserve window already closed");
        _deployWithWindow(block.timestamp);
    }

    function test_the_treasury_carve_out_is_not_released() public {
        ScrySeat s = _deployWithWindow(block.timestamp + 1 days);
        vm.warp(block.timestamp + 1 days);
        // the float grew by the three free doors and by nothing else
        assertEq(s.paidRemaining(), PAID_FLOAT + SNAP_CAP + PLAY_CAP + BUILD_CAP);
        s.mintTreasury(bob, TREASURY_CAP); // still the team's, still capped
        assertEq(s.treasuryMinted(), TREASURY_CAP);
    }

    /// The window is welded. There is no call that moves it, which is the whole
    /// difference between a posted expiry and an owner shifting supply around.
    function test_nobody_can_move_the_window() public {
        ScrySeat s = _deployWithWindow(block.timestamp + 1 days);
        uint64 posted = s.reservedUntil();
        s.transferOwnership(address(0)); // even renouncing changes nothing
        assertEq(s.reservedUntil(), posted);
    }
}
