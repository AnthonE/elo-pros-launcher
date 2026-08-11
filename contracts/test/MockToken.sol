// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title MockToken — a mintable ERC-20 test double, TEST-ONLY
/// @notice Replaces `src/PlayToken.sol`, the free-faucet play coin (pGOLD /
///         pTEARS) retired with the playground on 2026-07-26.
///         Three suites used PlayToken purely as a funding fixture —
///         `SpoilsToken.t.sol` and `ScryGardener.t.sol` as a stand-in for SCRY,
///         and the Garden/Burrow suite as the pair — so the fixture moved here
///         and the product token is gone.
///
///         It lives in `test/` on purpose. A PUBLIC free-faucet ERC-20 sitting
///         in `src/` is deployable, and `SpoilsToken.sol` documents exactly why
///         that shape is dangerous: anyone mints, so it drains the real side of
///         any pool it faces. A test double belongs where it cannot be
///         broadcast.
///
///         `faucet()` is kept (same 1000e18, same signature) so the migrated
///         call sites read unchanged, but the one-day cooldown is dropped — no
///         test ever asserted it, and it was a product rule, not a fixture one.
contract MockToken {
    string public name;
    string public symbol;
    uint8 public constant decimals = 18;
    uint256 public constant FAUCET_AMOUNT = 1_000e18;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    constructor(string memory n, string memory s) {
        name = n;
        symbol = s;
    }

    function mint(address to, uint256 value) public {
        totalSupply += value;
        balanceOf[to] += value;
        emit Transfer(address(0), to, value);
    }

    /// Fund the caller. No cooldown — see the note above.
    function faucet() external returns (uint256) {
        mint(msg.sender, FAUCET_AMOUNT);
        return FAUCET_AMOUNT;
    }

    function transfer(address to, uint256 value) external returns (bool) {
        _move(msg.sender, to, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= value, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - value;
        _move(from, to, value);
        return true;
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        emit Approval(msg.sender, spender, value);
        return true;
    }

    function _move(address from, address to, uint256 value) internal {
        require(balanceOf[from] >= value, "balance");
        unchecked {
            balanceOf[from] -= value;
            balanceOf[to] += value;
        }
        emit Transfer(from, to, value);
    }
}
