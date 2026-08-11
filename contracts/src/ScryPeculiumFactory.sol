// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./ScryPeculium.sol";

/// @notice Atomic CREATE2 deployer for ScryPeculium. The uninitialized vault
/// accepts exactly one caller (this factory), and initialization+seal occur in
/// the same transaction as deployment.
contract ScryPeculiumFactory {
    event PeculiumDeployed(address indexed vault, bytes32 indexed deploymentSalt, bytes32 policyHash);

    function prePolicyHash(ScryPeculium.Init calldata init) public pure returns (bytes32) {
        return keccak256(
            abi.encode(
                init.token,
                init.beneficiary,
                init.perTxCap,
                init.windowCap,
                init.windowSeconds,
                init.livenessDelay,
                init.maxAdmissionTtl,
                init.runtimeMeasurementRoot,
                init.rewardSourceRoot,
                init.earningsRulesVersion,
                init.allowedDestinations,
                init.verifiers,
                init.threshold
            )
        );
    }

    function deploymentSalt(ScryPeculium.Init calldata init) public pure returns (bytes32) {
        return keccak256(abi.encode(init.instanceSalt, prePolicyHash(init)));
    }

    function predict(ScryPeculium.Init calldata init) public view returns (address) {
        bytes32 salt = deploymentSalt(init);
        bytes32 codeHash = keccak256(abi.encodePacked(type(ScryPeculium).creationCode, abi.encode(address(this))));
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), address(this), salt, codeHash)))));
    }

    function verifierSetHash(ScryPeculium.Init calldata init) public pure returns (bytes32) {
        return keccak256(abi.encode(init.verifiers, init.threshold));
    }

    function policyHashFor(ScryPeculium.Init calldata init) public view returns (bytes32) {
        return keccak256(
            abi.encode(
                predict(init),
                block.chainid,
                init.instanceSalt,
                address(init.token),
                init.beneficiary,
                init.perTxCap,
                init.windowCap,
                init.windowSeconds,
                init.livenessDelay,
                init.maxAdmissionTtl,
                keccak256(abi.encode(init.allowedDestinations)),
                init.runtimeMeasurementRoot,
                init.rewardSourceRoot,
                init.earningsRulesVersion,
                verifierSetHash(init),
                init.threshold
            )
        );
    }

    function deploy(
        ScryPeculium.Init calldata init,
        ScryPeculium.SignerAdmission calldata admission,
        bytes calldata candidateProof,
        bytes[] calldata verifierSignatures
    ) external returns (ScryPeculium vault) {
        bytes32 salt = deploymentSalt(init);
        vault = new ScryPeculium{salt: salt}(address(this));
        vault.initializeAndSeal(init, admission, candidateProof, verifierSignatures);
        emit PeculiumDeployed(address(vault), salt, vault.policyHash());
    }
}
