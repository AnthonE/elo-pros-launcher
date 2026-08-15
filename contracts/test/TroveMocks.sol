// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice The smallest honest ERC-721 — what a well-behaved collection looks
///         like. Nothing in its transfer path can revert for a reason of its
///         own, which is exactly `ITEM-CONTRACT.md` §2d's "boring ERC-721".
contract MockNft {
    string public name = "Mock";
    string public symbol = "MOCK";
    mapping(uint256 => address) public ownerOfMap;
    mapping(address => uint256) public balanceOf;
    mapping(uint256 => address) public getApproved;
    mapping(address => mapping(address => bool)) public isApprovedForAll;

    event Transfer(address indexed from, address indexed to, uint256 indexed tokenId);
    event Approval(address indexed owner, address indexed approved, uint256 indexed tokenId);

    function mint(address to, uint256 id) external {
        ownerOfMap[id] = to;
        balanceOf[to] += 1;
        emit Transfer(address(0), to, id);
    }

    function ownerOf(uint256 id) public view returns (address o) {
        o = ownerOfMap[id];
        require(o != address(0), "no token");
    }

    function approve(address to, uint256 id) external {
        require(msg.sender == ownerOf(id), "not owner");
        getApproved[id] = to;
        emit Approval(msg.sender, to, id);
    }

    function setApprovalForAll(address op, bool ok) external {
        isApprovedForAll[msg.sender][op] = ok;
    }

    function transferFrom(address from, address to, uint256 id) public virtual {
        require(ownerOf(id) == from, "not owner");
        require(
            msg.sender == from || getApproved[id] == msg.sender || isApprovedForAll[from][msg.sender],
            "not approved"
        );
        getApproved[id] = address(0);
        balanceOf[from] -= 1;
        balanceOf[to] += 1;
        ownerOfMap[id] = to;
        emit Transfer(from, to, id);
    }

    function supportsInterface(bytes4 iid) external pure virtual returns (bool) {
        return iid == 0x01ffc9a7 || iid == 0x80ac58cd;
    }
}

/// @notice A collection with a pause switch — the single most common way a
///         transfer acquires the ability to revert. `ScryTrove.wrap` must refuse
///         it while paused, and that refusal is the point: better to fail at the
///         wrap than to strand a position mid-draw and jam the whole pool.
contract PausableNft is MockNft {
    bool public paused;

    function setPaused(bool p) external {
        paused = p;
    }

    function transferFrom(address from, address to, uint256 id) public override {
        require(!paused, "paused");
        super.transferFrom(from, to, id);
    }
}

/// @notice A token that can never move. The thing no interface probe can see
///         and only a simulated transfer catches (`meter/gacha_fit.py`).
contract SoulboundNft is MockNft {
    function transferFrom(address, address, uint256) public pure override {
        revert("soulbound");
    }
}

/// @notice `ScryGacha.deposit`'s pull, isolated: plain `transferFrom`, then
///         assert `ownerOf == address(this)`. Same harness `ScryItem.t.sol`
///         uses, and it is the test that fails first if anything clever ever
///         enters a transfer path.
contract GachaLike {
    function pull(address collection, uint256 tokenId) external {
        MockNft(collection).transferFrom(msg.sender, address(this), tokenId);
        require(MockNft(collection).ownerOf(tokenId) == address(this), "nft transfer failed");
    }

    function push(address collection, address to, uint256 tokenId) external {
        MockNft(collection).transferFrom(address(this), to, tokenId);
    }
}

/// @notice A plain ERC-20 — what a game coin or a donated token looks like.
contract MockToken {
    string public name = "Mock Coin";
    string public symbol = "MOCK";
    uint8 public decimals = 18;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
        totalSupply += amount;
        emit Transfer(address(0), to, amount);
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        return _move(msg.sender, to, amount);
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        if (a != type(uint256).max) allowance[from][msg.sender] = a - amount;
        return _move(from, to, amount);
    }

    function _move(address from, address to, uint256 amount) internal virtual returns (bool) {
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
        return true;
    }
}

/// @notice A token that skims 1% on every transfer. `SafeERC20.safeTransferFrom`
///         proves the CALL succeeded; it cannot prove the tokens ARRIVED — so a
///         wrapper minted against the stated amount would promise more than it
///         holds, and the last holder out eats the difference. `pullExact` is
///         what refuses it — the half of the FWA Token Packs read that always
///         applied.
contract SkimToken is MockToken {
    function _move(address from, address to, uint256 amount) internal override returns (bool) {
        uint256 fee = amount / 100;
        balanceOf[from] -= amount;
        balanceOf[to] += amount - fee;
        balanceOf[address(0xdEaD)] += fee;
        emit Transfer(from, to, amount - fee);
        return true;
    }
}
