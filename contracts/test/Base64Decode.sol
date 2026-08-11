// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Base64 decoder, for tests only.
///
/// It exists because the ticket's metadata moved on chain: `tokenURI` now
/// returns a `data:application/json;base64,…` document, and a suite that only
/// checked the prefix would pass while the JSON inside was malformed. That is
/// the exact failure mode this whole change is fixing — a metadata read that is
/// wrong and raises nothing — so the tests decode and read the document rather
/// than trusting its wrapper. **Assert the field the user sees** (`CLAUDE.md`).
library Base64Decode {
    function decode(string memory data) internal pure returns (bytes memory) {
        bytes memory b = bytes(data);
        if (b.length == 0) return "";
        require(b.length % 4 == 0, "base64: bad length");

        uint256 pad;
        if (b[b.length - 1] == "=") pad++;
        if (b[b.length - 2] == "=") pad++;

        bytes memory out = new bytes((b.length / 4) * 3 - pad);
        uint256 j;
        for (uint256 i; i < b.length; i += 4) {
            uint256 chunk = (uint256(_val(b[i])) << 18) | (uint256(_val(b[i + 1])) << 12)
                | (uint256(_val(b[i + 2])) << 6) | uint256(_val(b[i + 3]));
            if (j < out.length) out[j++] = bytes1(uint8(chunk >> 16));
            if (j < out.length) out[j++] = bytes1(uint8((chunk >> 8) & 0xff));
            if (j < out.length) out[j++] = bytes1(uint8(chunk & 0xff));
        }
        return out;
    }

    function _val(bytes1 c) private pure returns (uint8) {
        uint8 x = uint8(c);
        if (x >= 65 && x <= 90) return x - 65; // A-Z
        if (x >= 97 && x <= 122) return x - 71; // a-z
        if (x >= 48 && x <= 57) return x + 4; // 0-9
        if (x == 43) return 62; // +
        if (x == 47) return 63; // /
        if (x == 61) return 0; // = padding
        revert("base64: bad char");
    }
}
