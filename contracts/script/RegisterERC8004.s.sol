// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";

/// ERC-8004 Trustless Agents — register scry's identity in the ecosystem's
/// trust layer, so counterparties resolve the pubkey + agent card from the
/// registry instead of pinning an off-chain blob.
///
/// FINAL-SPEC INTERFACE (rewritten 2026-07-23). The draft this script first
/// targeted (`newAgent(agentDomain, agentAddress)`) never shipped: the
/// deployed registry is ERC-721 + URIStorage — you mint with
/// `register(agentURI)` and the URI must resolve to a registration-v1 file.
/// The meter serves that file at /.well-known/agent-registration.json
/// (pre-mint its `registrations` list is empty; after this broadcast set
/// SCRY_ERC8004_AGENT_ID on the meter and it fills in).
///
/// The canonical IdentityRegistry is the SAME vanity address on every
/// supported mainnet (Ethereum, Base, Arbitrum, …) — verified 2026-07-23
/// against github.com/erc-8004/erc-8004-contracts + live mainnet bytecode.
/// Default below is pinned to it; ALWAYS re-verify against the official
/// deployments list before broadcasting — never guess, never trust only
/// this comment. Ethereum mainnet gas, or pick a cheaper supported chain
/// (same address) with --rpc-url.
///
///   export PRIVATE_KEY=0x…
///   export SCRY_AGENT_URI=https://scry.moreright.xyz/.well-known/agent-registration.json
///   forge script script/RegisterERC8004.s.sol --rpc-url $ETH_RPC --broadcast
///
/// Then: publish the returned agentId (SHIP.md §4), set SCRY_ERC8004_AGENT_ID
/// on the meter, and record the tx in contracts/deployments.json.
/// ⚠ one-time; operator-gated like every broadcast; custody stays cap 0.
interface IIdentityRegistry8004 {
    function register(string calldata agentURI) external returns (uint256 agentId);
}

contract RegisterERC8004 is Script {
    /// 0x8004A169… — the 8004 team's vanity deployment, identical across
    /// supported mainnets. Env-overridable; re-verify before broadcast.
    address public constant DEFAULT_IDENTITY_REGISTRY = 0x8004A169FB4a3325136EB29fA0ceB6D2e539a432;

    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address registry = vm.envOr("ERC8004_IDENTITY_REGISTRY", DEFAULT_IDENTITY_REGISTRY);
        string memory agentURI =
            vm.envOr("SCRY_AGENT_URI", string("https://scry.moreright.xyz/.well-known/agent-registration.json"));
        require(registry.code.length > 0, "no code at registry - wrong chain or address");
        vm.startBroadcast(pk);
        uint256 id = IIdentityRegistry8004(registry).register(agentURI);
        vm.stopBroadcast();
        console2.log("ERC-8004 agentId", id);
        console2.log("now: set SCRY_ERC8004_AGENT_ID on the meter; record in deployments.json");
    }
}
