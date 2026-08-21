// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Math - full-precision muldiv, for the one shape that keeps recurring
/// @notice `EloGardener.updatePool` and `EloSilo.updateBin` both compute a
///         pro-rata share as `(emitted * allocPoint) / totalAllocPoint`. The
///         RESULT is always bounded — `allocPoint <= totalAllocPoint`, so it can
///         never exceed `emitted` — but the INTERMEDIATE product is not, and a
///         256-bit multiply reverts long before the division would have brought
///         it back in range.
///
///         That matters more than an overflow usually would, because of where
///         it sits. Every exit from both contracts (`withdraw`, `reap`,
///         `unseal`, `breakSeal`) routes through the update, and both
///         `setAllocPoint`s mass-update BEFORE mutating — so once the product
///         is too big, the very call that would shrink `allocPoint` reverts
///         too. There is no path home and staked principal is trapped. It is
///         the same "no path home" shape `MAX_REWARD_PER_SECOND` was added to
///         remove, and the rate cap does not cover it: the cap bounds one
///         factor and the other one is unbounded.
///
///         A bound on `allocPoint` was the obvious alternative and was
///         deliberately not taken — there is no posted maximum, so any number
///         chosen here would be inventing policy for an operator knob. Doing
///         the arithmetic at full precision removes the failure without
///         constraining anything. (Added 2026-07-29.)
///
/// @dev    Remco Bloemen's 512-bit muldiv, the same implementation OpenZeppelin
///         and Uniswap ship. Kept verbatim in structure on purpose: this is
///         cryptographic-grade fiddly and a "tidied" variant is a rewrite.
library Math {
    /// @notice floor(x * y / d) computed on a 512-bit intermediate.
    /// @dev Reverts if the true quotient does not fit in 256 bits, or if d == 0.
    ///      Callers here already guarantee d != 0 (`totalAllocPoint == 0`
    ///      returns early in both contracts).
    function mulDiv(uint256 x, uint256 y, uint256 d) internal pure returns (uint256 result) {
        unchecked {
            // 512-bit product as prod1*2^256 + prod0.
            uint256 prod0;
            uint256 prod1;
            assembly {
                let mm := mulmod(x, y, not(0))
                prod0 := mul(x, y)
                prod1 := sub(sub(mm, prod0), lt(mm, prod0))
            }

            // The common case: the product fit in 256 bits after all.
            if (prod1 == 0) {
                require(d > 0, "mulDiv: zero denominator");
                return prod0 / d;
            }

            // The quotient must fit in 256 bits. This also catches d == 0.
            require(d > prod1, "mulDiv: overflow");

            // -- 512 by 256 division ------------------------------------------
            // Subtract the remainder from [prod1 prod0].
            uint256 remainder;
            assembly {
                remainder := mulmod(x, y, d)
                prod1 := sub(prod1, gt(remainder, prod0))
                prod0 := sub(prod0, remainder)
            }

            // Factor the powers of two out of d, so what is left is odd and
            // therefore invertible modulo 2^256.
            uint256 twos = d & (0 - d);
            assembly {
                d := div(d, twos)
                prod0 := div(prod0, twos)
                // the bits of prod1 that shift down into prod0
                twos := add(div(sub(0, twos), twos), 1)
            }
            prod0 |= prod1 * twos;

            // Newton-Raphson for the modular inverse of d mod 2^256: correct to
            // 4 bits by inspection, then doubling to 8, 16, 32, 64, 128, 256.
            uint256 inverse = (3 * d) ^ 2;
            inverse *= 2 - d * inverse;
            inverse *= 2 - d * inverse;
            inverse *= 2 - d * inverse;
            inverse *= 2 - d * inverse;
            inverse *= 2 - d * inverse;
            inverse *= 2 - d * inverse;

            // Exact division: multiply by the inverse. The high half is no
            // longer needed once the remainder is gone.
            result = prod0 * inverse;
            return result;
        }
    }
}
