// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./IERC20.sol";

/// @title ScryPeculium — sealed policy vault with attestation-quorum signer admission
/// @notice v2 core for KEYLESS-WALLET.md (threat model + admission control).
/// No owner, recovery key, configurator, or upgrade path exists. The factory
/// deploys then initializes+seals atomically; later signer rotation requires a
/// fresh EIP-712 certificate signed by the immutable verifier quorum.
///
/// WHAT ACTUALLY BOUNDS A WITHDRAWAL (audit 2026-07-25). KEYLESS-WALLET.md and
/// CLAUDE.md describe this as "withdrawals only to the registered wallet". The
/// enforced rule is narrower and stricter than one wallet but is NOT keyed on
/// `beneficiary`: `spend()` checks the immutable `allowedDestinations`
/// allowlist. `beneficiary` is recorded, sealed into `policyHash`, and never
/// read by this contract — it is the policy's declared owner-of-record for
/// off-chain accounting, not an on-chain constraint. If the intent is literally
/// one destination, seal a single-entry allowlist; do not rely on
/// `beneficiary` to do it.
contract ScryPeculium {
    struct Init {
        bytes32 instanceSalt;
        IERC20 token;
        address beneficiary;
        uint256 perTxCap;
        uint256 windowCap;
        uint64 windowSeconds;
        uint64 livenessDelay;
        uint64 maxAdmissionTtl;
        bytes32 runtimeMeasurementRoot;
        bytes32 rewardSourceRoot;
        uint8 earningsRulesVersion;
        address[] allowedDestinations;
        address[] verifiers; // strictly ascending, immutable after init
        uint256 threshold;
    }

    struct SignerAdmission {
        address vault;
        uint256 chainId;
        bytes32 policyHash;
        bytes32 verifierSetHash;
        address candidateSigner;
        uint64 signerEpoch;
        bytes32 attestationHash;
        bytes32 challenge;
        uint64 issuedAt;
        uint64 expiresAt;
        bool emergency;
    }

    string public constant NOTICE =
        "sealed familiar working-float vault; signer admission needs attested quorum evidence";

    bytes32 public constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 public constant ADMISSION_TYPEHASH = keccak256(
        "SignerAdmission(address vault,uint256 chainId,bytes32 policyHash,bytes32 verifierSetHash,address candidateSigner,uint64 signerEpoch,bytes32 attestationHash,bytes32 challenge,uint64 issuedAt,uint64 expiresAt,bool emergency)"
    );
    bytes32 private constant NAME_HASH = keccak256("ScryPeculiumAdmission");
    bytes32 private constant VERSION_HASH = keccak256("1");
    uint256 private constant SECP256K1N_HALF = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;

    address public immutable factory;
    bool public sealedPolicy;
    IERC20 public token;
    address public beneficiary;
    address public signer;
    bytes32 public policyHash;
    bytes32 public verifierSetHash;
    bytes32 public admissionNonce;
    bytes32 public runtimeMeasurementRoot;
    bytes32 public rewardSourceRoot;
    uint8 public earningsRulesVersion;
    uint64 public signerEpoch;
    uint64 public lastSignerHeartbeat;
    uint64 public windowSeconds;
    uint64 public livenessDelay;
    uint64 public maxAdmissionTtl;
    uint256 public perTxCap;
    uint256 public windowCap;
    uint256 public windowStart;
    uint256 public windowSpent;
    uint256 public verifierThreshold;

    mapping(address => bool) public allowed;
    mapping(address => bool) public isVerifier;
    mapping(bytes32 => bool) public consumedCertificate;

    event Sealed(bytes32 indexed policyHash, address indexed signer, uint64 signerEpoch);
    event DestinationAllowed(address indexed destination);
    event VerifierConfigured(address indexed verifier);
    event Spent(address indexed to, uint256 amount, uint256 windowSpent);
    event Heartbeat(address indexed signer, uint64 at);
    event SignerAdmitted(
        address indexed oldSigner,
        address indexed newSigner,
        uint64 indexed signerEpoch,
        bytes32 certificateDigest,
        bytes32 attestationHash,
        bool emergency
    );

    error NotFactory();
    error NotSigner();
    error AlreadySealed();
    error NotSealed();
    error InvalidInit();
    error InvalidAdmission();
    error InvalidSignature();
    error NotAllowed();
    error OverPerTxCap();
    error OverWindowCap();
    error TransferFailed();
    error EmergencyTooEarly();

    modifier onlyFactory() {
        if (msg.sender != factory) revert NotFactory();
        _;
    }

    modifier onlySigner() {
        if (msg.sender != signer) revert NotSigner();
        _;
    }

    modifier onlySealed() {
        if (!sealedPolicy) revert NotSealed();
        _;
    }

    constructor(address expectedFactory) {
        if (expectedFactory == address(0)) revert InvalidInit();
        factory = expectedFactory;
    }

    /// @notice Factory-only and one-time. All mutability ends before this call
    /// returns; no external actor can operate an unsealed vault.
    function initializeAndSeal(
        Init calldata init,
        SignerAdmission calldata admission,
        bytes calldata candidateProof,
        bytes[] calldata verifierSignatures
    ) external onlyFactory {
        if (sealedPolicy) revert AlreadySealed();
        _validateInit(init);

        token = init.token;
        beneficiary = init.beneficiary;
        perTxCap = init.perTxCap;
        windowCap = init.windowCap;
        windowSeconds = init.windowSeconds;
        livenessDelay = init.livenessDelay;
        maxAdmissionTtl = init.maxAdmissionTtl;
        runtimeMeasurementRoot = init.runtimeMeasurementRoot;
        rewardSourceRoot = init.rewardSourceRoot;
        earningsRulesVersion = init.earningsRulesVersion;
        verifierThreshold = init.threshold;
        verifierSetHash = _verifierSetHash(init.verifiers, init.threshold);
        policyHash = _policyHash(init, verifierSetHash);
        for (uint256 i; i < init.allowedDestinations.length; ++i) {
            allowed[init.allowedDestinations[i]] = true;
            emit DestinationAllowed(init.allowedDestinations[i]);
        }
        for (uint256 i; i < init.verifiers.length; ++i) {
            isVerifier[init.verifiers[i]] = true;
            emit VerifierConfigured(init.verifiers[i]);
        }

        admissionNonce = keccak256(abi.encode("SCRY_PECULIUM_NONCE_V1", address(this), policyHash, uint64(1)));
        _validateAdmission(admission, candidateProof, bytes(""), verifierSignatures, true);
        _admit(admission, true);
        sealedPolicy = true;
        emit Sealed(policyHash, signer, signerEpoch);
    }

    /// @notice Anyone may relay a valid certificate. Normal rotation also needs
    /// the current signer; emergency rotation needs a missed heartbeat delay.
    function admitSigner(
        SignerAdmission calldata admission,
        bytes calldata candidateProof,
        bytes memory activeSignerApproval,
        bytes[] calldata verifierSignatures
    ) external onlySealed {
        _validateAdmission(admission, candidateProof, activeSignerApproval, verifierSignatures, false);
        _admit(admission, false);
    }

    /// @notice Liveness signal only; it cannot transfer value or change policy.
    function heartbeat() external onlySealed onlySigner {
        lastSignerHeartbeat = uint64(block.timestamp);
        emit Heartbeat(signer, lastSignerHeartbeat);
    }

    /// @notice The sole value-moving power: direct ERC-20 transfer to a pinned
    /// recipient within immutable caps. There is deliberately no settle().
    function spend(address to, uint256 amount) external onlySealed onlySigner {
        if (!allowed[to]) revert NotAllowed();
        if (amount > perTxCap) revert OverPerTxCap();
        _accrueWindow(amount);
        _safeTransfer(to, amount);
        emit Spent(to, amount, windowSpent);
    }

    function balance() external view returns (uint256) {
        return address(token) == address(0) ? 0 : token.balanceOf(address(this));
    }

    function spendableThisWindow() external view returns (uint256) {
        if (!sealedPolicy || block.timestamp >= windowStart + windowSeconds) return windowCap;
        return windowSpent >= windowCap ? 0 : windowCap - windowSpent;
    }

    function domainSeparator() public view returns (bytes32) {
        return keccak256(abi.encode(DOMAIN_TYPEHASH, NAME_HASH, VERSION_HASH, block.chainid, address(this)));
    }

    function admissionDigest(SignerAdmission calldata admission) public view returns (bytes32) {
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
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), structHash));
    }

    function candidateProofDigest(bytes32 challenge) public pure returns (bytes32) {
        return keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", challenge));
    }

    function expectedChallenge(uint64 nextEpoch, address candidate) public view returns (bytes32) {
        return keccak256(
            abi.encode(
                "SCRY_PECULIUM_ADMISSION_V1",
                block.chainid,
                address(this),
                policyHash,
                verifierSetHash,
                nextEpoch,
                candidate,
                admissionNonce
            )
        );
    }

    function _validateInit(Init calldata init) internal pure {
        if (
            address(init.token) == address(0) || init.beneficiary == address(0) || init.perTxCap == 0
                || init.windowCap == 0 || init.perTxCap > init.windowCap || init.windowSeconds == 0
                || init.livenessDelay == 0 || init.maxAdmissionTtl == 0 || init.runtimeMeasurementRoot == bytes32(0)
                || init.rewardSourceRoot == bytes32(0) || init.earningsRulesVersion == 0
                || init.allowedDestinations.length == 0 || init.verifiers.length == 0 || init.threshold == 0
                || init.threshold > init.verifiers.length
        ) revert InvalidInit();
        _validateStrictlySorted(init.allowedDestinations);
        _validateStrictlySorted(init.verifiers);
    }

    function _validateAdmission(
        SignerAdmission calldata admission,
        bytes calldata candidateProof,
        bytes memory activeSignerApproval,
        bytes[] calldata verifierSignatures,
        bool initial
    ) internal view {
        if (
            admission.vault != address(this) || admission.chainId != block.chainid || admission.policyHash != policyHash
                || admission.verifierSetHash != verifierSetHash || admission.candidateSigner == address(0)
                || admission.signerEpoch != signerEpoch + 1
                || admission.challenge != expectedChallenge(admission.signerEpoch, admission.candidateSigner)
                || admission.issuedAt > block.timestamp || admission.expiresAt < block.timestamp
                || admission.expiresAt < admission.issuedAt
                || admission.expiresAt - admission.issuedAt > maxAdmissionTtl
        ) revert InvalidAdmission();
        if (initial && admission.emergency) revert InvalidAdmission();

        bytes32 digest = admissionDigest(admission);
        if (consumedCertificate[digest]) revert InvalidAdmission();
        if (_recover(candidateProofDigest(admission.challenge), candidateProof) != admission.candidateSigner) {
            revert InvalidSignature();
        }
        address previous;
        uint256 valid;
        for (uint256 i; i < verifierSignatures.length; ++i) {
            address recovered = _recover(digest, verifierSignatures[i]);
            if (recovered <= previous || !isVerifier[recovered]) revert InvalidSignature();
            previous = recovered;
            ++valid;
        }
        if (valid < verifierThreshold) revert InvalidSignature();

        if (!initial) {
            if (admission.emergency) {
                if (block.timestamp < uint256(lastSignerHeartbeat) + livenessDelay) revert EmergencyTooEarly();
            } else if (_recover(digest, activeSignerApproval) != signer) {
                revert InvalidSignature();
            }
        }
    }

    function _admit(SignerAdmission calldata admission, bool initial) internal {
        bytes32 digest = admissionDigest(admission);
        address oldSigner = signer;
        consumedCertificate[digest] = true;
        signer = admission.candidateSigner;
        signerEpoch = admission.signerEpoch;
        lastSignerHeartbeat = uint64(block.timestamp);
        admissionNonce = keccak256(abi.encode(admission.attestationHash, digest, signerEpoch));
        emit SignerAdmitted(
            oldSigner, signer, signerEpoch, digest, admission.attestationHash, !initial && admission.emergency
        );
    }

    function _policyHash(Init calldata init, bytes32 setHash) internal view returns (bytes32) {
        return keccak256(
            abi.encode(
                address(this),
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
                setHash,
                init.threshold
            )
        );
    }

    function _verifierSetHash(address[] calldata verifiers, uint256 threshold) internal pure returns (bytes32) {
        return keccak256(abi.encode(verifiers, threshold));
    }

    function _validateStrictlySorted(address[] calldata values) internal pure {
        address previous;
        for (uint256 i; i < values.length; ++i) {
            if (values[i] == address(0) || values[i] <= previous) revert InvalidInit();
            previous = values[i];
        }
    }

    function _accrueWindow(uint256 amount) internal {
        if (block.timestamp >= windowStart + windowSeconds) {
            windowStart = block.timestamp;
            windowSpent = 0;
        }
        uint256 next = windowSpent + amount;
        if (next > windowCap) revert OverWindowCap();
        windowSpent = next;
    }

    function _safeTransfer(address to, uint256 amount) internal {
        (bool ok, bytes memory ret) = address(token).call(abi.encodeWithSelector(IERC20.transfer.selector, to, amount));
        if (!ok || (ret.length != 0 && !abi.decode(ret, (bool)))) revert TransferFailed();
    }

    function _recover(bytes32 digest, bytes memory signature) internal pure returns (address recovered) {
        if (signature.length != 65) return address(0);
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly ("memory-safe") {
            r := mload(add(signature, 32))
            s := mload(add(signature, 64))
            v := byte(0, mload(add(signature, 96)))
        }
        if (v < 27) v += 27;
        if (v != 27 && v != 28 || uint256(s) > SECP256K1N_HALF) return address(0);
        recovered = ecrecover(digest, v, r, s);
    }
}
