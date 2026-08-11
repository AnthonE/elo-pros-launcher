// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ReentrancyGuard — the invariant, enforced rather than assumed
/// @notice §M6/§M7. There was no reentrancy guard anywhere in `src/`. That was
///         SOUND, but only conditionally: checks-effects-interactions is
///         genuinely followed in the fund holders, and `SCRY`/`SpoilsToken`
///         have no transfer callback. Both halves of that condition were
///         unstated and unenforced — and several of these contracts take their
///         token as a CONSTRUCTOR ARGUMENT, so "the token has no callback" is
///         a deployment-time assumption a future deployer cannot see.
///
///         Pairing any of them with an ERC-777, an ERC-1363, or a hook-bearing
///         ERC-20 turns a comment into a withdrawal. `ScryBank.enter` was the
///         sharpest case: it minted shares BEFORE pulling the deposit and said
///         so in a comment. The ordering is fixed; this makes the class safe
///         rather than the instance.
///
///         Deliberately minimal and dependency-free, matching the rest of
///         `src/`: no OpenZeppelin import, one storage slot, same semantics.
abstract contract ReentrancyGuard {
    uint256 private constant _NOT_ENTERED = 1;
    uint256 private constant _ENTERED = 2;
    uint256 private _status = _NOT_ENTERED;

    modifier nonReentrant() {
        require(_status != _ENTERED, "reentrant");
        _status = _ENTERED;
        _;
        _status = _NOT_ENTERED;
    }
}
