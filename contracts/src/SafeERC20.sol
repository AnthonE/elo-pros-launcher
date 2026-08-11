// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title SafeERC20 — one tolerant ERC-20 call for every fund mover here
/// @notice §M5. Every contract in `src/` moved funds with
///         `require(token.transfer(...), "...")` against an interface that
///         declares a `bool` return. That is correct for `SCRY` and
///         `SpoilsToken`, and WRONG for the large family of tokens that return
///         nothing at all (the USDT shape): the ABI decoder finds an empty
///         returndata where it expected 32 bytes and reverts, so the transfer
///         appears to fail even when it succeeded. `ScryHarvest` in particular
///         documents itself "token-generic; nothing below assumes a particular
///         ERC-20", which was simply false.
///
///         The rule this encodes: a token call SUCCEEDED if the call itself
///         succeeded AND it either returned nothing or returned true. That is
///         the same test `ScryJobBoard._tryPull` already used — this is that
///         one function, shared, so the behaviour cannot drift per contract.
///
///         Takes `address` rather than a typed interface on purpose: several
///         of the older fun-layer contracts declare their own inline `IERC20`,
///         and a library bound to one of them could not be used by the others.
library SafeERC20 {
    /// @dev Reverts with `reason` if the transfer did not happen.
    function safeTransfer(address token, address to, uint256 amount, string memory reason) internal {
        (bool ok, bytes memory ret) = token.call(abi.encodeWithSignature("transfer(address,uint256)", to, amount));
        _check(ok, ret, reason);
    }

    /// @dev Reverts with `reason` if the pull did not happen.
    function safeTransferFrom(address token, address from, address to, uint256 amount, string memory reason) internal {
        (bool ok, bytes memory ret) =
            token.call(abi.encodeWithSignature("transferFrom(address,address,uint256)", from, to, amount));
        _check(ok, ret, reason);
    }

    /// @dev Approve a spender under the same tolerant-return rule.
    ///      Added for `ScryCistern`, which must let a swap router pull the exact
    ///      amount it is about to swap. Kept HERE rather than inline in that
    ///      contract for the reason in the header: a second definition of "did
    ///      this token call succeed" is a second thing to keep in step.
    ///
    ///      ⚠ Callers must approve the EXACT amount they are about to have
    ///      pulled, and only immediately before the call that pulls it. This
    ///      deliberately does not implement the reset-to-zero dance: that guards
    ///      a *standing* allowance being re-raced, and a standing allowance is
    ///      the thing a caller here must never leave behind. `exactInputSingle`
    ///      spends `amountIn` exactly, so the allowance is back to zero when it
    ///      returns.
    function safeApprove(address token, address spender, uint256 amount, string memory reason) internal {
        (bool ok, bytes memory ret) = token.call(abi.encodeWithSignature("approve(address,uint256)", spender, amount));
        _check(ok, ret, reason);
    }

    /// @dev A low-level call swallows the callee's revert reason unless you
    ///      hand it back, and losing "insufficient balance" for a flat
    ///      "transfer failed" makes every failure look alike. So: if the TOKEN
    ///      reverted, bubble ITS data unchanged; `reason` is only for the case
    ///      the low-level call could not produce one — a token that returned
    ///      `false`, or returndata that is neither empty nor a bool.
    function _check(bool ok, bytes memory ret, string memory reason) private pure {
        if (!ok) {
            if (ret.length > 0) {
                assembly {
                    revert(add(ret, 0x20), mload(ret))
                }
            }
            revert(reason);
        }
        require(ret.length == 0 || abi.decode(ret, (bool)), reason);
    }

    /// @dev A pull that MEASURES, for callers holding someone else's token.
    ///      `safeTransferFrom` proves the CALL succeeded. It cannot prove the
    ///      tokens ARRIVED: a fee-on-transfer or reflection token debits `amount`
    ///      from the sender, credits less than `amount` to the receiver, and
    ///      returns true. Every caller in `src/` credits the stated `amount` to
    ///      its own books *before* the pull, so against such a token the books
    ///      claim tokens the contract does not hold — and the shortfall is
    ///      always paid by whoever withdraws last.
    ///
    ///      Two of the three call sites are worse than mis-accounting.
    ///      `ScryGarden.addLiquidity` mints SEED against the stated amount, so a
    ///      short deposit dilutes every other depositor; `ScryGarden.swap`
    ///      prices `out` off the stated `amountIn`, so a short input pays a full
    ///      output — that one is a drain, not a rounding error.
    ///
    ///      This REFUSES rather than crediting the smaller delta, and the
    ///      refusal is the design. All three callers price something off the
    ///      stated amount before the pull happens; crediting what actually
    ///      landed would mean rewriting their arithmetic, not swapping one call.
    ///      A loud revert is the honest minimum, and it leaves the door open —
    ///      supporting these tokens later is a change to the caller, not here.
    ///      (FWA Token Packs, `0x727C…3d74A` mainnet, takes the same line from
    ///      the same position: `IncorrectAmountReceived()`.)
    ///
    ///      Reverts if the receiver's balance did not rise by EXACTLY `amount`.
    ///      A rise larger than `amount` is refused too — a token whose transfer
    ///      hook pays the receiver extra is not one this accounting models.
    function pullExact(address token, address from, address to, uint256 amount, string memory reason) internal {
        uint256 held = _balanceOf(token, to);
        safeTransferFrom(token, from, to, amount, reason);
        // 0.8.x checked arithmetic: a balance that FELL underflows and reverts,
        // which is the correct answer for a token that moves funds out of the
        // receiver during its own transfer.
        require(_balanceOf(token, to) - held == amount, "inexact transfer");
    }

    /// @dev `balanceOf` through the same bare-address door the rest of this
    ///      library uses. Unlike the movement calls there is no tolerant case
    ///      to encode: a token that cannot report a balance cannot be measured,
    ///      so anything but 32+ bytes of returndata is a refusal.
    function _balanceOf(address token, address who) private view returns (uint256) {
        (bool ok, bytes memory ret) = token.staticcall(abi.encodeWithSignature("balanceOf(address)", who));
        require(ok && ret.length >= 32, "balanceOf failed");
        return abi.decode(ret, (uint256));
    }

    /// @dev Non-reverting variant, for callers that must branch on failure
    ///      (a defaulted buyer, a thin pool) rather than abort.
    function tryTransferFrom(address token, address from, address to, uint256 amount) internal returns (bool) {
        (bool ok, bytes memory ret) =
            token.call(abi.encodeWithSignature("transferFrom(address,address,uint256)", from, to, amount));
        return ok && (ret.length == 0 || abi.decode(ret, (bool)));
    }
}
