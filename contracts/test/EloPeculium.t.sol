// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloPeculium.sol";
import "../src/EloPeculiumFactory.sol";
import "../src/IERC20.sol";

contract MockToken is IERC20 {
    mapping(address => uint256) public bal;

    function mint(address to, uint256 amount) external {
        bal[to] += amount;
    }

    function balanceOf(address who) external view returns (uint256) {
        return bal[who];
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(bal[msg.sender] >= amount, "insufficient");
        bal[msg.sender] -= amount;
        bal[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(bal[from] >= amount, "insufficient");
        bal[from] -= amount;
        bal[to] += amount;
        return true;
    }
}

contract EloPeculiumTest is Test {
    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant ADMISSION_TYPEHASH = keccak256(
        "SignerAdmission(address vault,uint256 chainId,bytes32 policyHash,bytes32 verifierSetHash,address candidateSigner,uint64 signerEpoch,bytes32 attestationHash,bytes32 challenge,uint64 issuedAt,uint64 expiresAt,bool emergency)"
    );

    MockToken tok;
    EloPeculiumFactory factory;
    EloPeculium vault;

    uint256 private signerKey = 0xA11CE;
    address private signer;
    address private beneficiary = address(0xBEEF);
    address private venue = address(0xCAFE);
    address private rando = address(0xBAD);
    address private owner = address(0x0E9E);
    uint256[5] private verifierKeys;
    address[5] private verifierAddresses;

    uint256 private constant PER_TX = 100 ether;
    uint256 private constant WINDOW = 250 ether;
    uint64 private constant WINDOW_SECONDS = 1 days;
    uint64 private constant LIVENESS_DELAY = 2 days;
    uint64 private constant ADMISSION_TTL = 30 minutes;

    function setUp() public {
        tok = new MockToken();
        factory = new EloPeculiumFactory();
        signer = vm.addr(signerKey);
        verifierKeys = [uint256(0x101), uint256(0x202), uint256(0x303), uint256(0x404), uint256(0x505)];
        for (uint256 i; i < verifierKeys.length; ++i) {
            verifierAddresses[i] = vm.addr(verifierKeys[i]);
        }
        // The protocol requires canonical signer ordering, so sort addresses
        // while retaining the private-key correspondence used for signatures.
        for (uint256 i; i < verifierAddresses.length; ++i) {
            for (uint256 j = i + 1; j < verifierAddresses.length; ++j) {
                if (verifierAddresses[j] < verifierAddresses[i]) {
                    (verifierAddresses[i], verifierAddresses[j]) = (verifierAddresses[j], verifierAddresses[i]);
                    (verifierKeys[i], verifierKeys[j]) = (verifierKeys[j], verifierKeys[i]);
                }
            }
        }
    }

    function test_factoryDeploysAtomicallySealedWithQuorumAdmittedSigner() public {
        _deploy(signer, signerKey, 3);
        assertTrue(vault.sealedPolicy());
        assertEq(vault.factory(), address(factory));
        assertEq(vault.signer(), signer);
        assertEq(vault.signerEpoch(), 1);
        assertEq(vault.beneficiary(), beneficiary);
        assertEq(vault.verifierThreshold(), 3);
        assertTrue(vault.allowed(venue));
        assertFalse(vault.allowed(rando));
        assertEq(vault.balance(), 1000 ether);
    }

    function test_precomputedAddressIsTheAdmittedVault() public {
        EloPeculium.Init memory init = _init();
        address predicted = factory.predict(init);
        _deploy(signer, signerKey, 3);
        assertEq(address(vault), predicted);
        assertEq(vault.policyHash(), factory.policyHashFor(init));
    }

    function test_minorityVerifierSignaturesCannotDeployVault() public {
        EloPeculium.Init memory init = _init();
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures) =
            _initialBundle(init, signer, signerKey, false, 2);
        vm.expectRevert(EloPeculium.InvalidSignature.selector);
        factory.deploy(init, admission, proof, signatures);
    }

    function test_wrongPolicyOrCandidateProofCannotDeployVault() public {
        EloPeculium.Init memory init = _init();
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures) =
            _initialBundle(init, signer, signerKey, false, 3);
        admission.policyHash = bytes32(uint256(123));
        vm.expectRevert(EloPeculium.InvalidAdmission.selector);
        factory.deploy(init, admission, proof, signatures);

        (admission, proof, signatures) = _initialBundle(init, signer, signerKey, false, 3);
        proof = _sign(signerKey + 1, _candidateProofDigest(admission.challenge));
        vm.expectRevert(EloPeculium.InvalidSignature.selector);
        factory.deploy(init, admission, proof, signatures);
    }

    function test_onlyAdmittedSignerCanSpendAndCapsStillHold() public {
        _deploy(signer, signerKey, 3);
        vm.prank(signer);
        vault.spend(venue, 100 ether);
        assertEq(tok.balanceOf(venue), 100 ether);
        vm.prank(signer);
        vm.expectRevert(EloPeculium.NotAllowed.selector);
        vault.spend(rando, 1 ether);
        vm.prank(signer);
        vm.expectRevert(EloPeculium.OverPerTxCap.selector);
        vault.spend(venue, PER_TX + 1);
        vm.prank(owner);
        vm.expectRevert(EloPeculium.NotSigner.selector);
        vault.spend(venue, 1 ether);
    }

    function test_windowCapAccumulatesAndResets() public {
        _deploy(signer, signerKey, 3);
        vm.startPrank(signer);
        vault.spend(venue, 100 ether);
        vault.spend(venue, 100 ether);
        vm.expectRevert(EloPeculium.OverWindowCap.selector);
        vault.spend(venue, 100 ether);
        vm.warp(block.timestamp + WINDOW_SECONDS);
        vault.spend(venue, 100 ether);
        vm.stopPrank();
        assertEq(vault.windowSpent(), 100 ether);
    }

    function test_noOwnerRecoveryOrUnrestrictedSettlementSurface() public {
        _deploy(signer, signerKey, 3);
        // The public ABI has no owner/configurator/recovery/settle method; an
        // arbitrary address cannot obtain the only remaining powers.
        vm.prank(owner);
        vm.expectRevert(EloPeculium.NotSigner.selector);
        vault.heartbeat();
        assertEq(tok.balanceOf(beneficiary), 0);
        assertEq(vault.balance(), 1000 ether);
    }

    function test_normalRotationNeedsOldSignerCandidateProofAndQuorum() public {
        _deploy(signer, signerKey, 3);
        uint256 nextKey = 0xB0B;
        address nextSigner = vm.addr(nextKey);
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures) =
            _rotationBundle(nextSigner, nextKey, false, 3);
        bytes memory oldSignerApproval = _sign(signerKey, vault.admissionDigest(admission));
        vault.admitSigner(admission, proof, oldSignerApproval, signatures);
        assertEq(vault.signer(), nextSigner);
        assertEq(vault.signerEpoch(), 2);
        vm.prank(signer);
        vm.expectRevert(EloPeculium.NotSigner.selector);
        vault.spend(venue, 1 ether);
        vm.prank(nextSigner);
        vault.spend(venue, 1 ether);
    }

    function test_normalRotationRejectsMissingOldSignerApprovalAndReplay() public {
        _deploy(signer, signerKey, 3);
        uint256 nextKey = 0xB0B;
        address nextSigner = vm.addr(nextKey);
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures) =
            _rotationBundle(nextSigner, nextKey, false, 3);
        vm.expectRevert(EloPeculium.InvalidSignature.selector);
        vault.admitSigner(admission, proof, bytes(""), signatures);

        bytes memory oldSignerApproval = _sign(signerKey, vault.admissionDigest(admission));
        vault.admitSigner(admission, proof, oldSignerApproval, signatures);
        vm.expectRevert(EloPeculium.InvalidAdmission.selector);
        vault.admitSigner(admission, proof, oldSignerApproval, signatures);
    }

    function test_ownerAloneCannotAdmitAnEOA() public {
        _deploy(signer, signerKey, 3);
        uint256 attackerKey = 0xD00D;
        address attacker = vm.addr(attackerKey);
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory noVerifierSignatures) =
            _rotationBundle(attacker, attackerKey, false, 0);
        vm.prank(owner);
        vm.expectRevert(EloPeculium.InvalidSignature.selector);
        vault.admitSigner(admission, proof, bytes(""), noVerifierSignatures);
        assertEq(vault.signer(), signer);
    }

    function test_emergencyRotationFailsClosedUntilHeartbeatDelayThenNeedsQuorum() public {
        _deploy(signer, signerKey, 3);
        uint256 nextKey = 0xB0B;
        address nextSigner = vm.addr(nextKey);
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures) =
            _rotationBundle(nextSigner, nextKey, true, 3);
        vm.expectRevert(EloPeculium.EmergencyTooEarly.selector);
        vault.admitSigner(admission, proof, bytes(""), signatures);
        vm.warp(block.timestamp + LIVENESS_DELAY);
        // Rebuild because the certificate's short TTL must remain fresh.
        (admission, proof, signatures) = _rotationBundle(nextSigner, nextKey, true, 3);
        vault.admitSigner(admission, proof, bytes(""), signatures);
        assertEq(vault.signer(), nextSigner);
    }

    function test_wrongVaultAndDuplicateVerifierSignaturesReject() public {
        _deploy(signer, signerKey, 3);
        uint256 nextKey = 0xB0B;
        address nextSigner = vm.addr(nextKey);
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures) =
            _rotationBundle(nextSigner, nextKey, false, 3);
        admission.vault = rando;
        vm.expectRevert(EloPeculium.InvalidAdmission.selector);
        vault.admitSigner(admission, proof, bytes(""), signatures);

        (admission, proof, signatures) = _rotationBundle(nextSigner, nextKey, false, 3);
        signatures[1] = signatures[0];
        vm.expectRevert(EloPeculium.InvalidSignature.selector);
        vault.admitSigner(admission, proof, bytes(""), signatures);
    }

    function test_expiredCertificateRejects() public {
        _deploy(signer, signerKey, 3);
        uint256 nextKey = 0xB0B;
        address nextSigner = vm.addr(nextKey);
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures) =
            _rotationBundle(nextSigner, nextKey, false, 3);
        bytes memory oldSignerApproval = _sign(signerKey, vault.admissionDigest(admission));
        vm.warp(block.timestamp + ADMISSION_TTL + 1);
        vm.expectRevert(EloPeculium.InvalidAdmission.selector);
        vault.admitSigner(admission, proof, oldSignerApproval, signatures);
    }

    function _deploy(address initialSigner, uint256 initialSignerKey, uint256 signatureCount) internal {
        EloPeculium.Init memory init = _init();
        (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures) =
            _initialBundle(init, initialSigner, initialSignerKey, false, signatureCount);
        vault = factory.deploy(init, admission, proof, signatures);
        tok.mint(address(vault), 1000 ether);
    }

    function _init() internal view returns (EloPeculium.Init memory init) {
        address[] memory destinations = new address[](1);
        destinations[0] = venue;
        address[] memory verifiers = new address[](5);
        for (uint256 i; i < verifiers.length; ++i) {
            verifiers[i] = verifierAddresses[i];
        }
        init = EloPeculium.Init({
            instanceSalt: keccak256("peculium-test-instance"),
            token: IERC20(address(tok)),
            beneficiary: beneficiary,
            perTxCap: PER_TX,
            windowCap: WINDOW,
            windowSeconds: WINDOW_SECONDS,
            livenessDelay: LIVENESS_DELAY,
            maxAdmissionTtl: ADMISSION_TTL,
            runtimeMeasurementRoot: keccak256("measured-runtime-v1"),
            rewardSourceRoot: keccak256("reward-sources-v1"),
            earningsRulesVersion: 1,
            allowedDestinations: destinations,
            verifiers: verifiers,
            threshold: 3
        });
    }

    function _initialBundle(
        EloPeculium.Init memory init,
        address candidate,
        uint256 candidateKey,
        bool emergency,
        uint256 signatureCount
    )
        internal
        view
        returns (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures)
    {
        address predicted = factory.predict(init);
        bytes32 policy = factory.policyHashFor(init);
        bytes32 setHash = factory.verifierSetHash(init);
        bytes32 nonce = keccak256(abi.encode("SCRY_PECULIUM_NONCE_V1", predicted, policy, uint64(1)));
        bytes32 challenge = _challenge(predicted, policy, setHash, 1, candidate, nonce);
        admission = EloPeculium.SignerAdmission({
            vault: predicted,
            chainId: block.chainid,
            policyHash: policy,
            verifierSetHash: setHash,
            candidateSigner: candidate,
            signerEpoch: 1,
            attestationHash: keccak256("attestation-epoch-1"),
            challenge: challenge,
            issuedAt: uint64(block.timestamp),
            expiresAt: uint64(block.timestamp + ADMISSION_TTL),
            emergency: emergency
        });
        proof = _sign(candidateKey, _candidateProofDigest(challenge));
        signatures = _verifierSignatures(_admissionDigest(predicted, admission), signatureCount);
    }

    function _rotationBundle(address candidate, uint256 candidateKey, bool emergency, uint256 signatureCount)
        internal
        view
        returns (EloPeculium.SignerAdmission memory admission, bytes memory proof, bytes[] memory signatures)
    {
        uint64 epoch = vault.signerEpoch() + 1;
        bytes32 challenge = vault.expectedChallenge(epoch, candidate);
        admission = EloPeculium.SignerAdmission({
            vault: address(vault),
            chainId: block.chainid,
            policyHash: vault.policyHash(),
            verifierSetHash: vault.verifierSetHash(),
            candidateSigner: candidate,
            signerEpoch: epoch,
            attestationHash: keccak256(abi.encode("attestation", epoch, candidate)),
            challenge: challenge,
            issuedAt: uint64(block.timestamp),
            expiresAt: uint64(block.timestamp + ADMISSION_TTL),
            emergency: emergency
        });
        proof = _sign(candidateKey, vault.candidateProofDigest(challenge));
        signatures = _verifierSignatures(vault.admissionDigest(admission), signatureCount);
    }

    function _challenge(
        address vaultAddress,
        bytes32 policy,
        bytes32 setHash,
        uint64 epoch,
        address candidate,
        bytes32 nonce
    ) internal view returns (bytes32) {
        return keccak256(
            abi.encode(
                "SCRY_PECULIUM_ADMISSION_V1", block.chainid, vaultAddress, policy, setHash, epoch, candidate, nonce
            )
        );
    }

    function _admissionDigest(address vaultAddress, EloPeculium.SignerAdmission memory admission)
        internal
        view
        returns (bytes32)
    {
        bytes32 domain = keccak256(
            abi.encode(DOMAIN_TYPEHASH, keccak256("EloPeculiumAdmission"), keccak256("1"), block.chainid, vaultAddress)
        );
        bytes32 structHash = keccak256(
            abi.encode(
                ADMISSION_TYPEHASH,
                admission.vault,
                admission.chainId,
                admission.policyHash,
                admission.verifierSetHash,
                admission.candidateSigner,
                admission.signerEpoch,
                admission.attestationHash,
                admission.challenge,
                admission.issuedAt,
                admission.expiresAt,
                admission.emergency
            )
        );
        return keccak256(abi.encodePacked("\x19\x01", domain, structHash));
    }

    function _verifierSignatures(bytes32 digest, uint256 count) internal view returns (bytes[] memory signatures) {
        signatures = new bytes[](count);
        for (uint256 i; i < count; ++i) {
            signatures[i] = _sign(verifierKeys[i], digest);
        }
    }

    function _candidateProofDigest(bytes32 challenge) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", challenge));
    }

    function _sign(uint256 privateKey, bytes32 digest) internal pure returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, digest);
        return abi.encodePacked(r, s, v);
    }
}
