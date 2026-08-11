// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title MockArbSys — ArbOS's clock, at its real address, for tests
/// @notice `ScryGacha` reads the L2 height and L2 block hashes from the ArbOS
///         precompile at `0x64`, because on Arbitrum Nitro **Solidity's
///         `block.number` is the parent chain's** — measured on 4663 at
///         25,620,709 against an L2 height of 20,319,770, advancing 0.08
///         blocks/s versus the L2's 9.9. Using the opcodes turned a "100 block,
///         ~10 second" reveal into ~20 minutes.
///
///         **Foundry does not implement the ArbOS precompiles.** On a fork,
///         `0x64` carries one byte of non-executable code, so a call there
///         returns nothing and the contract's clock is dead. Every suite that
///         touches a draw — unit or fork — has to etch this in.
///
///         Two behaviours are modelled deliberately, because the contract
///         depends on both:
///
///         1. the L2 height moves independently of `block.number`, which is the
///            entire point;
///         2. `arbBlockHash` **reverts** outside its window rather than
///            returning zero (the live node reverts `InvalidBlockNumber`), and
///            that revert is what `settle` and `expire` split on.
///
///         ⚠ `vm.etch` copies runtime code and NOTHING else — no constructor
///         runs and no storage travels with it — so `window` must be set
///         explicitly after etching. Left implicit it is 0, every reveal reads
///         as expired, and the expiry test passes for the wrong reason. That
///         happened; hence this warning.
contract MockArbSys {
    uint256 public l2;
    uint256 public window;
    mapping(uint256 => bytes32) public pinned;

    function setL2Block(uint256 n) external {
        l2 = n;
    }

    function bump(uint256 n) external {
        l2 += n;
    }

    function setWindow(uint256 w) external {
        window = w;
    }

    function setHash(uint256 n, bytes32 h) external {
        pinned[n] = h;
    }

    function arbBlockNumber() external view returns (uint256) {
        return l2;
    }

    function arbBlockHash(uint256 n) external view returns (bytes32) {
        require(n < l2 && l2 - n <= window, "InvalidBlockNumber");
        bytes32 h = pinned[n];
        // An unpinned block still answers, so a test that does not care about
        // WHICH position is drawn does not have to pin anything.
        return h == bytes32(0) ? keccak256(abi.encode("l2 block", n)) : h;
    }
}
