// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloSeat.sol";

/// Plain no-callback ERC-20, which is what SCRY is.
contract RootMockSCRY {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function approve(address s, uint256 a) external returns (bool) {
        allowance[msg.sender][s] = a;
        return true;
    }

    function transferFrom(address f, address t, uint256 a) external returns (bool) {
        require(balanceOf[f] >= a, "balance");
        allowance[f][msg.sender] -= a;
        balanceOf[f] -= a;
        balanceOf[t] += a;
        return true;
    }
}

/// @title The cross-language check — does the root PYTHON built open the door
///        SOLIDITY guards?
/// @notice `EloSeat.t.sol` already covers the doors, but every tree in it is
///         built in Solidity by the test itself. That proves the contract is
///         self-consistent and proves nothing about the thing that actually
///         ships: `meter/seat_roots.py` computes the root the operator posts,
///         and if its encoding and this contract's ever drift, every proof it
///         serves fails on chain — one wallet at a time, at mint, in public.
///
///         So the vectors below are PINNED OUTPUT FROM THE PYTHON BUILDER,
///         pasted verbatim, never recomputed here. Recomputing them in Solidity
///         would test this file against itself and reproduce exactly the gap it
///         exists to close. Regenerate with:
///
///           python3 -c "import sys;sys.path.insert(0,'meter');\
///             from seat_roots import build_root;import json;\
///             print(json.dumps(build_root({'per_wallet':1,'wallets':[\
///               {'wallet':w,'allowance':1} for w in WALLETS]},door=1),indent=1))"
///
///         If this test goes red, do NOT edit the vectors to match. One of the
///         two implementations moved, and `meter/seat_roots.py`'s own selftest
///         pins the same leaf on the Python side — read both before touching
///         either.
contract EloSeatRootTest is Test {
    EloSeat seat;
    RootMockSCRY reserve;

    // The four cohort wallets. The builder sorts lowercase-ascending, which is
    // the order the pinned proofs below index into; the EIP-55 casing here is
    // only what a Solidity address literal requires.
    address constant W1 = 0x00000000000000000000000000000000000000A1;
    address constant W2 = 0x00000000000000000000000000000000000000b2;
    address constant W3 = 0x00000000000000000000000000000000000000C3;
    address constant W4 = 0x00000000000000000000000000000000000000D4;

    // ── pinned: meter/seat_roots.py build_root(door=1), allowance 1 each ──────
    bytes32 constant PY_ROOT = 0xfc5aea8f6253fde213214826de03537fa9dd2182d1f7d5baaa705b97dd4951ed;

    function _pyProof(uint8 i) internal pure returns (bytes32[] memory p) {
        p = new bytes32[](2);
        if (i == 1) {
            p[0] = 0xdccbb842812100f47f450aa3af879fc4962acf8644304e922be85dddae6efb76;
            p[1] = 0x50aa0dec779452013ef77eb4a02832f3d94d7c7375c78acb68409db8cb1c346f;
        } else if (i == 2) {
            p[0] = 0xd7d4e4b823e955a9c09e7ccb2e990a82dec59385d2eb23beecdc01071bae1352;
            p[1] = 0x50aa0dec779452013ef77eb4a02832f3d94d7c7375c78acb68409db8cb1c346f;
        } else if (i == 3) {
            p[0] = 0x42f50381948776ec16bff25205e25d50e47ed5ff93fd7bdc01a8b9205ed9a760;
            p[1] = 0x744fb05849b8025777b3125b5cc426f52c0102df6a8ef1fcc8445131a4b5bf29;
        } else {
            p[0] = 0x67d177169b0591f5371648df247cd78aadd044be78931790ee68eddab4f509b7;
            p[1] = 0x744fb05849b8025777b3125b5cc426f52c0102df6a8ef1fcc8445131a4b5bf29;
        }
    }

    function setUp() public {
        reserve = new RootMockSCRY();
        uint256[] memory costs = new uint256[](2);
        costs[0] = 66_666e18;
        costs[1] = 166_666e18;
        uint32[] memory weights = new uint32[](2);
        weights[0] = 100;
        weights[1] = 125;

        seat = new EloSeat(
            "reserve seats",
            "SEAT",
            [uint256(100), uint256(20), uint256(10), uint256(10), uint256(5), uint256(0)],
            IERC20(address(reserve)),
            address(0xFEE),
            address(0xB0B),
            500,
            5000,
            sha256(bytes("cross-language-root-test-salt")),
            costs,
            weights,
            "https://elopros.com/api"
        );
        seat.setDoorRoot(1, PY_ROOT);
    }

    /// ⭐ The one that matters: four wallets, four Python proofs, four seats.
    function test_python_built_root_opens_the_snapshot_door() public {
        address[4] memory ws = [W1, W2, W3, W4];
        for (uint8 i = 0; i < 4; i++) {
            vm.prank(ws[i]);
            uint256 id = seat.claim(1, 1, _pyProof(i + 1));
            assertEq(seat.ownerOf(id), ws[i], "seat did not land on the claimant");
        }
        assertEq(seat.snapshotMinted(), 4, "door count wrong");
        assertEq(seat.totalMinted(), 4, "supply count wrong");
    }

    /// The allowance is bound INTO the leaf, so a cohort member cannot inflate
    /// their own — the door is access, never a distribution.
    function test_a_python_proof_does_not_verify_at_a_larger_allowance() public {
        vm.prank(W1);
        vm.expectRevert("not in this cohort");
        seat.claim(1, 2, _pyProof(1));
    }

    /// One wallet cannot spend another's proof, which is what makes the list a
    /// list rather than a password.
    function test_one_cohort_member_cannot_spend_anothers_proof() public {
        vm.prank(W2);
        vm.expectRevert("not in this cohort");
        seat.claim(1, 1, _pyProof(1));
    }

    /// An address that was never in the tree gets nothing, however it asks.
    function test_an_outsider_cannot_walk_through() public {
        vm.prank(address(0xDEAD));
        vm.expectRevert("not in this cohort");
        seat.claim(1, 1, _pyProof(1));
    }

    /// Allowance is cumulative-lifetime, and the Python builder writes 1 — so a
    /// cohort wallet takes exactly one seat, forever, across any number of calls.
    function test_the_allowance_is_lifetime_not_per_call() public {
        vm.startPrank(W1);
        seat.claim(1, 1, _pyProof(1));
        vm.expectRevert("allowance spent");
        seat.claim(1, 1, _pyProof(1));
        vm.stopPrank();
    }

    /// The pinned leaf vector, asserted on THIS side of the language boundary.
    /// `meter/seat_roots.py`'s selftest asserts the identical bytes; if the two
    /// ever disagree, one of these two files is the bug and neither can hide it.
    function test_the_pinned_leaf_vector_matches_the_python_builder() public pure {
        assertEq(
            keccak256(abi.encodePacked(address(0x000000000000000000000000000000000000dEaD), uint256(1))),
            0x2408a6e0b95a9051f841a87788bdf639c26214774e8c8df1db1330bc7c6a562e,
            "leaf encoding drifted from meter/seat_roots.py"
        );
    }
}
