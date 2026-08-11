// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryVowRegistry.sol";

contract ScryVowRegistryTest is Test {
    ScryVowRegistry reg;
    address oracle = address(0x0AC1E);
    address agent = address(0xA6E27);
    address stranger = address(0xBAD);

    string constant TEXT = "serve the operator's declared strategy and nothing else";
    bytes32 vowId = bytes32(bytes8(0x79e960b901d25c8f));

    function setUp() public {
        reg = new ScryVowRegistry(oracle);
    }

    function _take(bool sealed_) internal returns (uint256) {
        // sha256() is a precompile CALL — compute it before vm.prank, or the
        // prank is consumed by the precompile and takeVow runs as the test contract.
        bytes32 h = sha256(bytes(TEXT));
        vm.prank(agent);
        return reg.takeVow(vowId, h, 24, sealed_, sealed_ ? "" : TEXT);
    }

    // ── vow taking ──────────────────────────────────────────────────────────
    function test_takeVow_unsealed() public {
        uint256 id = _take(false);
        assertEq(reg.ownerOf(id), agent);
        assertEq(reg.balanceOf(agent), 1);
        assertEq(reg.tokenOf(vowId), id);
        (bytes32 vId, bytes32 th, uint32 cad,, bool sealed_, address who) = reg.vows(id);
        assertEq(vId, vowId);
        assertEq(th, sha256(bytes(TEXT)));
        assertEq(cad, 24);
        assertFalse(sealed_);
        assertEq(who, agent);
    }

    function test_takeVow_wrongHash_reverts() public {
        bytes32 h = sha256("different text");
        vm.prank(agent);
        vm.expectRevert(ScryVowRegistry.BadTextHash.selector);
        reg.takeVow(vowId, h, 24, false, TEXT);
    }

    function test_takeVow_sealed_hidesText() public {
        uint256 id = _take(true);
        assertEq(reg.ownerOf(id), agent);
        (,,,, bool sealed_,) = reg.vows(id);
        assertTrue(sealed_);
    }

    function test_takeVow_sealed_withText_reverts() public {
        bytes32 h = sha256(bytes(TEXT));
        vm.prank(agent);
        vm.expectRevert(ScryVowRegistry.SealedNeedsNoText.selector);
        reg.takeVow(vowId, h, 24, true, TEXT);
    }

    function test_takeVow_duplicate_reverts() public {
        _take(false);
        bytes32 h = sha256(bytes(TEXT));
        vm.prank(stranger);
        vm.expectRevert(ScryVowRegistry.VowAlreadyTaken.selector);
        reg.takeVow(vowId, h, 24, false, TEXT);
    }

    // ── soulbound (ERC-5192) ───────────────────────────────────────────────
    function test_locked_and_transfers_revert() public {
        uint256 id = _take(false);
        assertTrue(reg.locked(id));
        vm.startPrank(agent);
        vm.expectRevert(ScryVowRegistry.Soulbound.selector);
        reg.transferFrom(agent, stranger, id);
        vm.expectRevert(ScryVowRegistry.Soulbound.selector);
        reg.safeTransferFrom(agent, stranger, id);
        vm.expectRevert(ScryVowRegistry.Soulbound.selector);
        reg.safeTransferFrom(agent, stranger, id, "");
        vm.expectRevert(ScryVowRegistry.Soulbound.selector);
        reg.approve(stranger, id);
        vm.expectRevert(ScryVowRegistry.Soulbound.selector);
        reg.setApprovalForAll(stranger, true);
        vm.stopPrank();
    }

    function test_supportsInterface() public view {
        assertTrue(reg.supportsInterface(0x01ffc9a7)); // 165
        assertTrue(reg.supportsInterface(0x80ac58cd)); // 721
        assertTrue(reg.supportsInterface(0x5b5e139f)); // 721 metadata
        assertTrue(reg.supportsInterface(0x49064906)); // 4906 metadata update
        assertTrue(reg.supportsInterface(0xb45a3c0e)); // 5192
        assertFalse(reg.supportsInterface(0xdeadbeef));
    }

    // ── admin: mutable ticker + swappable renderer ──────────────────────────
    function test_owner_is_deployer() public view {
        assertEq(reg.owner(), address(this)); // test contract deployed reg in setUp
    }

    function test_setName_setSymbol_onlyOwner() public {
        reg.setName("scry oaths");
        reg.setSymbol("OATH");
        assertEq(reg.name(), "scry oaths");
        assertEq(reg.symbol(), "OATH");
        vm.prank(stranger);
        vm.expectRevert(ScryVowRegistry.NotOwner.selector);
        reg.setName("hijack");
    }

    function test_transferOwnership() public {
        reg.transferOwnership(agent);
        assertEq(reg.owner(), agent);
        vm.prank(agent);
        reg.setSymbol("VOW2");
        assertEq(reg.symbol(), "VOW2");
    }

    function test_setRenderer_delegates_and_gates() public {
        uint256 id = _take(false);
        // built-in art by default
        assertEq(bytes(reg.tokenURI(id))[0], bytes("d")[0]); // data:...
        MockRenderer r = new MockRenderer();
        vm.prank(stranger);
        vm.expectRevert(ScryVowRegistry.NotOwner.selector);
        reg.setRenderer(address(r));
        reg.setRenderer(address(r));
        assertEq(reg.renderer(), address(r));
        assertEq(reg.tokenURI(id), "ipfs://mock-token");
        assertEq(reg.contractURI(), "ipfs://mock-collection");
        // restore built-in
        reg.setRenderer(address(0));
        assertTrue(bytes(reg.tokenURI(id)).length > 100);
    }

    // ── anchoring ───────────────────────────────────────────────────────────
    function test_anchor_onlyOracle() public {
        vm.prank(stranger);
        vm.expectRevert(ScryVowRegistry.NotOracle.selector);
        reg.anchorRoot(bytes32(uint256(1)), 1, "cid");
        vm.prank(oracle);
        reg.anchorRoot(bytes32(uint256(1)), 1, "cid");
        assertEq(reg.lastRoot(), bytes32(uint256(1)));
        assertEq(reg.anchorCount(), 1);
    }

    function test_setOracle() public {
        vm.prank(stranger);
        vm.expectRevert(ScryVowRegistry.NotOracle.selector);
        reg.setOracle(stranger);
        vm.prank(oracle);
        reg.setOracle(stranger);
        assertEq(reg.oracle(), stranger);
    }

    // ── merkle verify (sorted-pair, OZ-compatible) ─────────────────────────
    function test_verifyProof() public view {
        bytes32 a = keccak256("leaf-a");
        bytes32 b = keccak256("leaf-b");
        bytes32 root = a < b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = b;
        assertTrue(reg.verifyProof(proof, root, a));
        proof[0] = a;
        assertTrue(reg.verifyProof(proof, root, b));
        assertFalse(reg.verifyProof(proof, root, keccak256("evil")));
    }

    // ── metadata renders + parses ───────────────────────────────────────────
    function test_tokenURI_shape() public {
        uint256 id = _take(false);
        string memory uri = reg.tokenURI(id);
        assertTrue(bytes(uri).length > 100);
        // data:application/json;base64, prefix
        bytes memory p = bytes("data:application/json;base64,");
        for (uint256 i = 0; i < p.length; i++) {
            assertEq(bytes(uri)[i], p[i]);
        }
        vm.expectRevert(ScryVowRegistry.NoSuchToken.selector);
        reg.tokenURI(999);
    }

    function test_contractURI() public view {
        assertTrue(bytes(reg.contractURI()).length > 50);
    }

    // ── the product lives in the log: full event payloads asserted ──────────
    function test_takeVow_emitsMintLockAndFullText() public {
        bytes32 h = sha256(bytes(TEXT));
        vm.warp(777);
        vm.expectEmit(true, true, true, true);
        emit ScryVowRegistry.Transfer(address(0), agent, 1); // the mint
        vm.expectEmit(false, false, false, true);
        emit ScryVowRegistry.Locked(1); // ERC-5192
        vm.expectEmit(true, true, true, true);
        emit ScryVowRegistry.VowTaken(1, vowId, agent, h, 24, false, TEXT); // full text
        vm.prank(agent);
        reg.takeVow(vowId, h, 24, false, TEXT);
    }

    function test_anchored_emitsFullPayload() public {
        bytes32 root = keccak256("epoch-root");
        vm.warp(5000);
        vm.expectEmit(true, false, false, true);
        emit ScryVowRegistry.Anchored(root, 3, "anchor_leaves/20260722T000000.json", 5000);
        vm.prank(oracle);
        reg.anchorRoot(root, 3, "anchor_leaves/20260722T000000.json");
    }

    // ── the actually-deployed configuration ─────────────────────────────────
    // DeployScryVowRegistry.s.sol defaults SCRY_ANCHOR_ORACLE to the deployer,
    // so live the SAME key is owner AND oracle. Prove both roles work from it.
    function test_liveConfig_deployerIsOwnerAndOracle() public {
        address deployer = address(0xDE9107);
        vm.prank(deployer);
        ScryVowRegistry live = new ScryVowRegistry(deployer);
        assertEq(live.owner(), deployer);
        assertEq(live.oracle(), deployer);
        bytes32 root = keccak256("live-root");
        vm.prank(deployer);
        live.anchorRoot(root, 1, "cid-live");
        assertEq(live.lastRoot(), root);
        assertEq(live.anchorCount(), 1);
        vm.prank(deployer);
        live.setName("scry vows live");
        assertEq(live.name(), "scry vows live");
    }

    // ── oracle rotation: the new key gains anchor rights, the old loses them ─
    function test_oracleRotation_newGainsOldLoses() public {
        address newOracle = address(0x0AC2E);
        vm.expectEmit(true, true, false, true);
        emit ScryVowRegistry.OracleTransferred(oracle, newOracle);
        vm.prank(oracle);
        reg.setOracle(newOracle);
        assertEq(reg.oracle(), newOracle);
        bytes32 root = keccak256("rotated-root");
        vm.prank(newOracle);
        reg.anchorRoot(root, 1, "cid-after-rotation");
        assertEq(reg.lastRoot(), root);
        bytes32 staleRoot = keccak256("old-oracle-root");
        vm.prank(oracle);
        vm.expectRevert(ScryVowRegistry.NotOracle.selector);
        reg.anchorRoot(staleRoot, 1, "cid-stale");
    }

    // ── repeat anchoring: root overwritten, count + time advance ────────────
    function test_repeatAnchor_overwritesRootAdvancesTime() public {
        bytes32 r1 = keccak256("epoch-1");
        bytes32 r2 = keccak256("epoch-2");
        vm.warp(10_000);
        vm.prank(oracle);
        reg.anchorRoot(r1, 3, "cid-1");
        assertEq(reg.lastAnchorTime(), 10_000);
        vm.warp(96_400);
        vm.prank(oracle);
        reg.anchorRoot(r2, 5, "cid-2");
        assertEq(reg.anchorCount(), 2);
        assertEq(reg.lastRoot(), r2); // overwritten, not appended
        assertEq(reg.lastAnchorTime(), 96_400); // advanced
    }

    // ── merkle cross-fixtures from the PRODUCTION tree builder ──────────────
    // Generated offline (2026-07-22) by meter/anchor_worker.py's own
    // leaf_for/merkle_root/merkle_proof (sorted-pair keccak256, odd node
    // carries up unhashed), run unmodified on deterministic inputs:
    //   vow_id_i     = 16-hex of i (1-based, the API's 8-byte form)
    //   chain_head_i = sha256("chain-head-{i}")
    //   leaf_i       = keccak256(vow_id ljust 32 ++ chain_head)
    // for i in 1..3 and 1..5. "MID" = sorted index n/2 (a middle leaf, full-
    // depth proof); "CARRY" = last sorted leaf (the odd carried leaf — its
    // proof is a single node because it joins only at the top).
    bytes32 constant ROOT3 = 0x1112b4d2e25f73e2000077c10172ebeab271fd3fd336826cf00c52e0dfe8deef;
    bytes32 constant MID3 = 0x672fc326e176955552b54a3a10914ec189187e0a92e2c91752d7a642f0f7ea9b;
    bytes32 constant MID3_P0 = 0x11ae6007721b6d4b795c4364f234e0ce274aa93258665c084439abccb56d42fd;
    bytes32 constant MID3_P1 = 0x721575aa70a3331b8be48434e814a89240551c95a8ac78e9febeec450035f41b;
    bytes32 constant CARRY3 = 0x721575aa70a3331b8be48434e814a89240551c95a8ac78e9febeec450035f41b;
    bytes32 constant CARRY3_P0 = 0x8457794d28f3271fff6261fa2671b939c6ea369a3d70b5904caee6072b955a5d;

    bytes32 constant ROOT5 = 0x414e31ec8889598daa59ef8e0f04d8d952d3cc5217d8b503aff7f54af8fd166b;
    bytes32 constant MID5 = 0x704e4947238c8130cf83ffa597584ef8f97622770db4711af1896e0e7b49ccf9;
    bytes32 constant MID5_P0 = 0x721575aa70a3331b8be48434e814a89240551c95a8ac78e9febeec450035f41b;
    bytes32 constant MID5_P1 = 0x8457794d28f3271fff6261fa2671b939c6ea369a3d70b5904caee6072b955a5d;
    bytes32 constant MID5_P2 = 0x79b0ed4d213e524099f7ba6e07c20d22aff2f272099d8b0e4ffeb325d3338c3a;
    bytes32 constant CARRY5 = 0x79b0ed4d213e524099f7ba6e07c20d22aff2f272099d8b0e4ffeb325d3338c3a;
    bytes32 constant CARRY5_P0 = 0x88433f3b7f050ec456bb1b0f095d0ace928bb9c510f0d349a88344dc95927887;

    function test_verifyProof_production3LeafTree() public view {
        bytes32[] memory proof = new bytes32[](2);
        proof[0] = MID3_P0;
        proof[1] = MID3_P1;
        assertTrue(reg.verifyProof(proof, ROOT3, MID3)); // middle leaf, depth 2
        bytes32[] memory carry = new bytes32[](1);
        carry[0] = CARRY3_P0;
        assertTrue(reg.verifyProof(carry, ROOT3, CARRY3)); // odd carried leaf, depth 1
        proof[0] = keccak256("tampered-sibling");
        assertFalse(reg.verifyProof(proof, ROOT3, MID3)); // tamper rejected
    }

    function test_verifyProof_production5LeafTree() public view {
        bytes32[] memory proof = new bytes32[](3);
        proof[0] = MID5_P0;
        proof[1] = MID5_P1;
        proof[2] = MID5_P2;
        assertTrue(reg.verifyProof(proof, ROOT5, MID5)); // middle leaf, depth 3
        bytes32[] memory carry = new bytes32[](1);
        carry[0] = CARRY5_P0;
        assertTrue(reg.verifyProof(carry, ROOT5, CARRY5)); // twice-carried leaf, depth 1
        proof[1] = keccak256("tampered-sibling");
        assertFalse(reg.verifyProof(proof, ROOT5, MID5)); // tamper rejected
    }

    // ── sealed vows: metadata renders; zero-hash acceptance pinned ──────────
    function test_tokenURI_sealedRenders() public {
        uint256 id = _take(true);
        string memory uri = reg.tokenURI(id);
        bytes memory u = bytes(uri);
        assertGt(u.length, 100);
        bytes memory pfx = bytes("data:application/json;base64,");
        for (uint256 i = 0; i < pfx.length; i++) {
            assertEq(u[i], pfx[i]);
        }
    }

    // A sealed vow with textHash == 0 commits to NOTHING — no preimage can
    // ever be proven against it — yet takeVow accepts it (only the unsealed
    // branch validates the hash; sha256 of any text is nonzero, so unsealed
    // vows cannot hit this).
    // PINNED, NOT ENDORSED — see contracts/TEST-AUDIT.md
    function test_takeVow_sealedZeroHash_accepted() public {
        bytes32 emptyId = bytes32(bytes8(0x00000000000000aa));
        vm.prank(agent);
        uint256 id = reg.takeVow(emptyId, bytes32(0), 24, true, "");
        (, bytes32 th_,,, bool sealed_, address who) = reg.vows(id);
        assertEq(th_, bytes32(0));
        assertTrue(sealed_);
        assertEq(who, agent);
        assertEq(reg.ownerOf(id), agent);
    }

    // ── ownership: unauthorized transfer + the renounce brick ───────────────
    function test_transferOwnership_strangerReverts() public {
        vm.prank(stranger);
        vm.expectRevert(ScryVowRegistry.NotOwner.selector);
        reg.transferOwnership(stranger);
    }

    // transferOwnership(address(0)) is the documented renounce path: after it,
    // NO key — including the renouncer — can ever touch renderer/name/symbol
    // again. The metadata surface is bricked forever, by one unguarded call.
    // PINNED, NOT ENDORSED — see contracts/TEST-AUDIT.md
    function test_renounceToZero_bricksMetadataAdmin() public {
        reg.transferOwnership(address(0)); // test contract is the owner
        assertEq(reg.owner(), address(0));
        vm.expectRevert(ScryVowRegistry.NotOwner.selector);
        reg.setName("too late");
    }
}

/// Minimal external renderer used to prove tokenURI/contractURI delegation.
contract MockRenderer is IScryRenderer {
    function tokenURI(uint256) external pure returns (string memory) {
        return "ipfs://mock-token";
    }

    function contractURI() external pure returns (string memory) {
        return "ipfs://mock-collection";
    }
}
