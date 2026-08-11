// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScrySteleEdition.sol";

/// ERC-721 receivers for the safeTransfer surface (editions are the
/// TRANSFERABLE collectible — contract-recipient safety is load-bearing).
contract SteleGoodReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0x150b7a02; // IERC721Receiver.onERC721Received.selector
    }
}

contract SteleBadReceiver {
    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return 0xdeadbeef;
    }
}

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

/// Stands in for the live ScryVowRegistry. The print shop asks it one thing:
/// does this vow exist? (`tokenOf` is 0 for a vowId nobody ever swore.)
contract MockVowRegistry {
    mapping(bytes32 => uint256) public tokenOf;

    function swear(bytes32 vowId, uint256 tokenId) external {
        tokenOf[vowId] = tokenId;
    }
}

contract MockRenderer {
    function tokenURI(uint256) external pure returns (string memory) {
        return "R:token";
    }

    function contractURI() external pure returns (string memory) {
        return "R:contract";
    }
}

contract ScrySteleEditionTest is Test {
    MockSCRY scry;
    MockVowRegistry registry;
    ScrySteleEdition ed;
    address splitter = address(0xFEE);
    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    uint256 price = 5e18;
    bytes32 vowId = bytes32(bytes8(hex"a1b2c3d4e5f60718"));
    bytes32 unswornVowId = bytes32(bytes8(hex"0000000000000bad"));

    function setUp() public {
        scry = new MockSCRY();
        registry = new MockVowRegistry();
        registry.swear(vowId, 42); // the vow these tests print really was taken
        ed = new ScrySteleEdition(
            IERC20(address(scry)),
            splitter,
            price,
            "https://scry.moreright.xyz/api",
            IScryVowRegistryView(address(registry))
        );
    }

    // ── a print of a vow nobody swore is a forgery (audit 2026-07-25) ──────
    //    WAS: mintEdition(bytes32) validated nothing, so anyone could mint an
    //    on-chain "print of a vow" for a vow that never existed and list it as
    //    genuine. The register is now asked on every mint.
    function test_cannotPrintAVowThatWasNeverSworn() public {
        scry.mint(alice, price);
        vm.prank(alice);
        scry.approve(address(ed), price);
        vm.prank(alice);
        vm.expectRevert(bytes("no such vow"));
        ed.mintEdition(unswornVowId);
        assertEq(scry.balanceOf(alice), price, "and not a wei was taken for it");
        assertEq(scry.balanceOf(splitter), 0);

        // once it IS sworn, the same call goes through — the gate is existence
        registry.swear(unswornVowId, 43);
        vm.prank(alice);
        ed.mintEdition(unswornVowId);
        assertEq(ed.ownerOf(1), alice);
    }

    // The gate is existence, NEVER merit: any real vow may be printed by
    // anyone at the flat price, whatever its conduct record says. A print you
    // could only buy for a good vow would be a score money can move.
    function test_theGateIsExistenceNotMerit() public {
        bytes32 anotherVow = bytes32(bytes8(hex"00000000000000ff"));
        registry.swear(anotherVow, 7);
        scry.mint(bob, price);
        vm.prank(bob);
        scry.approve(address(ed), price);
        vm.prank(bob); // bob swore neither vow and prints someone else's freely
        uint256 tokenId = ed.mintEdition(anotherVow);
        assertEq(ed.ownerOf(tokenId), bob);
    }

    function test_constructorRejectsAZeroRegistry() public {
        vm.expectRevert(bytes("zero registry"));
        new ScrySteleEdition(IERC20(address(scry)), splitter, price, "b", IScryVowRegistryView(address(0)));
    }

    function test_mintPullsScryToSplitter() public {
        scry.mint(alice, price);
        vm.prank(alice);
        scry.approve(address(ed), price);
        vm.prank(alice);
        uint256 tokenId = ed.mintEdition(vowId);
        assertEq(tokenId, 1);
        assertEq(ed.ownerOf(1), alice);
        assertEq(scry.balanceOf(splitter), price); // SCRY went straight to the splitter
        assertEq(scry.balanceOf(alice), 0);
        (bytes32 v, uint64 edition,) = ed.editions(1);
        assertEq(v, vowId);
        assertEq(edition, 1);
    }

    function test_mintRevertsWithoutApproval() public {
        scry.mint(alice, price);
        vm.prank(alice);
        // alice is funded, so MockSCRY's balance check passes and its
        // allowance check is the require that fires — exact mock string.
        vm.expectRevert(bytes("allowance"));
        ed.mintEdition(vowId);
    }

    function test_editionsIncrementPerVow() public {
        scry.mint(alice, price * 3);
        vm.prank(alice);
        scry.approve(address(ed), price * 3);
        vm.startPrank(alice);
        ed.mintEdition(vowId);
        ed.mintEdition(vowId);
        bytes32 other = bytes32(bytes8(hex"1111111111111111"));
        vm.stopPrank();
        registry.swear(other, 43); // a second real vow in the register
        vm.startPrank(alice);
        ed.mintEdition(other);
        vm.stopPrank();
        assertEq(ed.editionCount(vowId), 2);
        assertEq(ed.editionCount(other), 1);
        (, uint64 e2,) = ed.editions(2);
        assertEq(e2, 2); // second print of vowId is edition #2
        assertEq(scry.balanceOf(splitter), price * 3); // multi-mint conservation: all 3 payments landed
    }

    function test_editionIsTransferable() public {
        scry.mint(alice, price);
        vm.prank(alice);
        scry.approve(address(ed), price);
        vm.prank(alice);
        ed.mintEdition(vowId);
        // the whole point: unlike the soulbound vow, a print can move
        vm.prank(alice);
        ed.transferFrom(alice, bob, 1);
        assertEq(ed.ownerOf(1), bob);
        assertEq(ed.balanceOf(bob), 1);
        assertEq(ed.balanceOf(alice), 0);
    }

    function test_tokenURIIsOnChainJson() public {
        scry.mint(alice, price);
        vm.prank(alice);
        scry.approve(address(ed), price);
        vm.prank(alice);
        ed.mintEdition(vowId);
        string memory uri = ed.tokenURI(1);
        assertEq(_prefix(uri, 29), "data:application/json;base64,");
    }

    function test_contractURIIsOnChainJson() public view {
        string memory uri = ed.contractURI(); // ERC-7572, no mint needed
        assertEq(_prefix(uri, 29), "data:application/json;base64,");
        assertGt(bytes(uri).length, 29); // carries a real payload
    }

    function test_ownerAdminAndRendererDelegation() public {
        assertEq(ed.owner(), address(this)); // deployer is owner
        scry.mint(alice, price);
        vm.prank(alice);
        scry.approve(address(ed), price);
        vm.prank(alice);
        ed.mintEdition(vowId);
        MockRenderer r = new MockRenderer();
        ed.setRenderer(address(r)); // repoint metadata
        assertEq(ed.tokenURI(1), "R:token");
        assertEq(ed.contractURI(), "R:contract");
        ed.setRenderer(address(0)); // restore built-in art
        assertEq(_prefix(ed.tokenURI(1), 29), "data:application/json;base64,");
    }

    function test_onlyOwnerAdmin() public {
        vm.prank(alice);
        vm.expectRevert(bytes("not owner"));
        ed.setBaseURI("x");
    }

    function test_supportsErc721() public view {
        assertTrue(ed.supportsInterface(0x80ac58cd)); // ERC-721
        assertTrue(ed.supportsInterface(0x5b5e139f)); // ERC-721 Metadata
    }

    function _prefix(string memory s, uint256 n) internal pure returns (string memory) {
        bytes memory b = bytes(s);
        bytes memory out = new bytes(n);
        for (uint256 i = 0; i < n; i++) {
            out[i] = b[i];
        }
        return string(out);
    }

    // ── the on-chain metadata is ACTUALLY on-chain JSON, decoded and
    //    field-checked — not the 29-char prefix TEST-AUDIT called theater. ──

    function _b64val(bytes1 c) internal pure returns (uint256) {
        uint8 x = uint8(c);
        if (x >= 65 && x <= 90) return x - 65; // A-Z
        if (x >= 97 && x <= 122) return x - 71; // a-z
        if (x >= 48 && x <= 57) return x + 4; // 0-9
        if (x == 43) return 62; // +
        if (x == 47) return 63; // /
        if (x == 61) return 0; // '=' padding
        revert("bad b64 char");
    }

    /// standard base64 decoder (with '=' padding) — enough to prove the meter
    /// output round-trips to real bytes, so a serialization bug can't hide.
    function _b64dec(string memory s) internal pure returns (bytes memory) {
        bytes memory d = bytes(s);
        require(d.length % 4 == 0 && d.length > 0, "b64 len");
        uint256 pad;
        if (d[d.length - 1] == "=") pad++;
        if (d[d.length - 2] == "=") pad++;
        bytes memory out = new bytes(d.length / 4 * 3 - pad);
        uint256 oi;
        for (uint256 i = 0; i < d.length; i += 4) {
            uint256 n = (_b64val(d[i]) << 18) | (_b64val(d[i + 1]) << 12) | (_b64val(d[i + 2]) << 6) | _b64val(d[i + 3]);
            if (oi < out.length) out[oi++] = bytes1(uint8(n >> 16));
            if (oi < out.length) out[oi++] = bytes1(uint8(n >> 8));
            if (oi < out.length) out[oi++] = bytes1(uint8(n));
        }
        return out;
    }

    /// tokenURI decodes to VALID on-chain JSON carrying the right identity —
    /// closes the "malformed JSON ships green" hole (prefix-only was theater).
    function test_tokenURI_decodes_to_valid_json_with_expected_fields() public {
        vm.warp(1_700_000_000); // fix mintedAt so it is a known answer
        scry.mint(alice, price);
        vm.prank(alice);
        scry.approve(address(ed), price);
        vm.prank(alice);
        uint256 id = ed.mintEdition(vowId);

        string memory uri = ed.tokenURI(id);
        bytes memory u = bytes(uri);
        bytes memory pre = bytes("data:application/json;base64,");
        for (uint256 i = 0; i < pre.length; i++) {
            assertEq(u[i], pre[i]);
        }
        bytes memory b64 = new bytes(u.length - pre.length);
        for (uint256 i = 0; i < b64.length; i++) {
            b64[i] = u[pre.length + i];
        }

        bytes memory jsonBytes = _b64dec(string(b64)); // DECODE, not prefix-check
        assertEq(jsonBytes[0], bytes1("{"));
        assertEq(jsonBytes[jsonBytes.length - 1], bytes1("}"));
        string memory json = string(jsonBytes);

        vm.parseJson(json); // reverts on malformed JSON
        assertEq(vm.parseJsonString(json, ".name"), "scry stele edition a1b2c3d4e5f60718 #1");
        assertEq(vm.parseJsonUint(json, ".attributes[1].value"), 1); // edition
        assertEq(vm.parseJsonUint(json, ".attributes[2].value"), 1_700_000_000); // minted_at
        // the image is a self-contained on-chain SVG data URI (not a URL)
        string memory image = vm.parseJsonString(json, ".image");
        assertEq(_prefix(image, 26), "data:image/svg+xml;base64,");
    }

    /// safeTransferFrom into a CONTRACT: accepted by a good receiver, refused
    /// by a bad one. Editions are meant to trade, so this surface is real.
    function test_safeTransferFrom_receiver_accept_and_reject() public {
        scry.mint(alice, price * 2);
        vm.prank(alice);
        scry.approve(address(ed), price * 2);
        vm.prank(alice);
        uint256 id1 = ed.mintEdition(vowId);
        vm.prank(alice);
        uint256 id2 = ed.mintEdition(vowId);

        SteleGoodReceiver good = new SteleGoodReceiver();
        vm.prank(alice);
        ed.safeTransferFrom(alice, address(good), id1);
        assertEq(ed.ownerOf(id1), address(good));

        SteleBadReceiver bad = new SteleBadReceiver();
        vm.prank(alice);
        vm.expectRevert(bytes("unsafe recipient"));
        ed.safeTransferFrom(alice, address(bad), id2);
        assertEq(ed.ownerOf(id2), alice);
    }
}
