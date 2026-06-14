// Supporting types for the native passkey authenticator: hardware-backed key
// management, the on-disk credential index, and the minimal CBOR / base64url
// codecs WebAuthn needs. See PasskeyAuthenticator.swift for the overview.

import Foundation
import CryptoKit

// MARK: - Key management

/// ES256 (P-256) signing keys for passkeys. Keys are created in the **Secure
/// Enclave** via CryptoKit, so the private key is hardware-bound and never
/// leaves the device. Crucially, the persisted form is the enclave's own opaque,
/// device-bound *data representation* which **we** store (base64, in the
/// credential index) rather than the keychain — so no `keychain-access-groups`
/// entitlement or provisioning profile is needed; plain code signing suffices.
/// User verification (Touch ID) is enforced by the authenticator calling
/// LocalAuthentication before every create/sign. A software key is used only if
/// the Secure Enclave is somehow unavailable (never on Apple Silicon / macOS 26).
struct KeyError: Error { let message: String; init(_ m: String) { message = m } }

enum KeyManager {
    /// Create a fresh signing key. Returns the public point (split X/Y for COSE)
    /// and an opaque blob to persist; the blob is prefixed `se:`/`sw:` to record
    /// which key type produced it.
    static func createKey() throws
        -> (publicKeyXY: (x: Data, y: Data), blob: String) {
        if SecureEnclave.isAvailable {
            let key = try SecureEnclave.P256.Signing.PrivateKey()
            return (try xy(key.publicKey),
                    "se:" + key.dataRepresentation.base64EncodedString())
        }
        let key = P256.Signing.PrivateKey()
        return (try xy(key.publicKey),
                "sw:" + key.rawRepresentation.base64EncodedString())
    }

    /// ECDSA-over-SHA256 signature (DER, as WebAuthn expects) over `message`,
    /// using the key the blob refers to.
    static func sign(blob: String, message: Data) throws -> Data {
        guard blob.count > 3,
              let raw = Data(base64Encoded: String(blob.dropFirst(3))) else {
            throw KeyError("Corrupt key blob.")
        }
        if blob.hasPrefix("se:") {
            let key = try SecureEnclave.P256.Signing.PrivateKey(
                dataRepresentation: raw)
            return try key.signature(for: message).derRepresentation
        }
        let key = try P256.Signing.PrivateKey(rawRepresentation: raw)
        return try key.signature(for: message).derRepresentation
    }

    /// Split an uncompressed P-256 public point (0x04 || X(32) || Y(32)) into
    /// the 32-byte X and Y coordinates COSE requires.
    private static func xy(_ pub: P256.Signing.PublicKey) throws
        -> (x: Data, y: Data) {
        let rep = pub.x963Representation
        guard rep.count == 65, rep.first == 0x04 else {
            throw KeyError("Unexpected public key format.")
        }
        return (rep.subdata(in: 1..<33), rep.subdata(in: 33..<65))
    }
}

// MARK: - Credential index

/// Metadata for one registered passkey. The private key itself lives in the
/// keychain/Secure Enclave keyed by `keyTag`; this record maps a relying party
/// to it and remembers the user handle for assertions.
struct CredentialRecord: Codable {
    let credentialId: Data
    let rpId: String
    let userHandleB64: String
    let userName: String
    /// Opaque, device-bound key representation (base64, `se:`/`sw:` prefixed).
    let keyBlob: String
}

/// Thread-confined (callers serialize via the authenticator queue) JSON index of
/// registered credentials, stored alongside the browser profile.
final class CredentialStore {
    private let url: URL
    private var records: [CredentialRecord]

    init() {
        let base = FileManager.default.urls(for: .applicationSupportDirectory,
                                            in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let dir = base.appendingPathComponent("MoriBrowser", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir,
                                                 withIntermediateDirectories: true)
        url = dir.appendingPathComponent("passkeys.json")
        if let data = try? Data(contentsOf: url),
           let decoded = try? JSONDecoder().decode([CredentialRecord].self,
                                                    from: data) {
            records = decoded
        } else {
            records = []
        }
    }

    func credentials(forRpId rpId: String) -> [CredentialRecord] {
        records.filter { $0.rpId == rpId }
    }

    func add(_ record: CredentialRecord) {
        records.append(record)
        persist()
    }

    private func persist() {
        if let data = try? JSONEncoder().encode(records) {
            try? data.write(to: url, options: .atomic)
        }
    }
}

// MARK: - base64url

enum Base64URL {
    static func encode(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    static func decode(_ string: String) -> Data? {
        var s = string.replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        while s.count % 4 != 0 { s.append("=") }
        return Data(base64Encoded: s)
    }
}

// MARK: - Minimal CBOR writer

/// Just enough canonical CBOR to encode COSE keys and the "none"-format
/// attestation object: unsigned/negative ints, byte/text strings, arrays, maps.
enum CBOR {
    case int(Int)
    case bytes(Data)
    case text(String)
    case array([CBOR])
    case map([(CBOR, CBOR)])
}

enum CBORWriter {
    static func map(_ pairs: [(CBOR, CBOR)]) -> Data { encode(.map(pairs)) }

    static func encode(_ value: CBOR) -> Data {
        switch value {
        case .int(let n):
            return n >= 0
                ? header(major: 0, value: UInt64(n))
                : header(major: 1, value: UInt64(-1 - n))
        case .bytes(let d):
            return header(major: 2, value: UInt64(d.count)) + d
        case .text(let s):
            let d = Data(s.utf8)
            return header(major: 3, value: UInt64(d.count)) + d
        case .array(let items):
            var out = header(major: 4, value: UInt64(items.count))
            for item in items { out += encode(item) }
            return out
        case .map(let pairs):
            var out = header(major: 5, value: UInt64(pairs.count))
            for (k, v) in pairs { out += encode(k) + encode(v) }
            return out
        }
    }

    private static func header(major: UInt8, value: UInt64) -> Data {
        let mt = major << 5
        switch value {
        case 0..<24:
            return Data([mt | UInt8(value)])
        case 24..<0x100:
            return Data([mt | 24, UInt8(value)])
        case 0x100..<0x10000:
            return Data([mt | 25, UInt8(value >> 8), UInt8(value & 0xff)])
        case 0x10000..<0x1_0000_0000:
            return Data([mt | 26,
                         UInt8((value >> 24) & 0xff), UInt8((value >> 16) & 0xff),
                         UInt8((value >> 8) & 0xff), UInt8(value & 0xff)])
        default:
            var bytes = [mt | 27]
            for shift in stride(from: 56, through: 0, by: -8) {
                bytes.append(UInt8((value >> UInt64(shift)) & 0xff))
            }
            return Data(bytes)
        }
    }
}

// MARK: - Endian helper

extension UInt16 {
    /// Two big-endian bytes, used for the credential-id length field.
    var bigEndianBytes: Data { Data([UInt8(self >> 8), UInt8(self & 0xff)]) }
}
