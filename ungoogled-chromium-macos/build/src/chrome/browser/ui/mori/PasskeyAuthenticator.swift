// Native WebAuthn platform authenticator for Mori.
//
// Why this exists: Mori embeds CEF as a child NSView (`SetAsChild`), which on
// macOS forces CEF's *Alloy* runtime style. Chromium's built-in passkey UI and
// the macOS platform-authenticator integration only ship with *Chrome* style,
// so `navigator.credentials.create/get` for `publicKey` does not work in this
// embedding. Instead of giving up on passkeys, we implement a real WebAuthn
// platform authenticator ourselves:
//
//   • An ES256 (P-256) key per credential, created in the Secure Enclave when
//     available (falling back to a Touch-ID-gated software key), so the private
//     key is hardware-bound and never leaves the device.
//   • User verification via LocalAuthentication (Touch ID, with passcode
//     fallback) on every registration and assertion.
//   • Standards-correct authenticatorData / attestationObject (fmt "none") /
//     COSE public key, so relying parties accept the credential like any other
//     platform passkey.
//
// The browser layer (BrowserClient.mm) injects a JS shim that overrides
// `navigator.credentials` and forwards the request here as JSON; we return the
// assertion/attestation as JSON which the shim turns back into a
// PublicKeyCredential. These are device-bound passkeys (not iCloud-synced),
// equivalent to a platform authenticator with `transports: ["internal"]`.

import Foundation
import CryptoKit
import LocalAuthentication
import Security

// MARK: - ObjC bridge

/// Entry point invoked from `BrowserClient.mm` (ObjC++) via `Mori-Swift.h`.
@objc(MoriPasskeys)
final class MoriPasskeys: NSObject {
    /// Handle one WebAuthn request. `requestJSON` is the serialized request from
    /// the JS shim; `completion` is invoked **on the main thread** with the
    /// serialized response (success or error), ready to hand back to JS.
    @objc static func handle(_ requestJSON: String,
                             completion: @escaping (String) -> Void) {
        PasskeyAuthenticator.shared.handle(requestJSON, completion: completion)
    }

    /// Whether a user-verifying platform authenticator is usable on this device.
    @objc static func isUserVerifyingPlatformAuthenticatorAvailable() -> Bool {
        var error: NSError?
        let ok = LAContext().canEvaluatePolicy(.deviceOwnerAuthentication,
                                               error: &error)
        return ok
    }
}

// MARK: - Authenticator

private final class PasskeyAuthenticator {
    static let shared = PasskeyAuthenticator()

    private let store = CredentialStore()
    // Serializes credential I/O and the (blocking, biometric) key operations.
    private let queue = DispatchQueue(label: "com.mori.browser.passkeys")

    /// Zero AAGUID — we don't masquerade as a specific certified authenticator.
    private static let aaguid = Data(count: 16)

    func handle(_ requestJSON: String, completion: @escaping (String) -> Void) {
        queue.async {
            let response = self.process(requestJSON)
            DispatchQueue.main.async { completion(response) }
        }
    }

    // MARK: Dispatch

    private func process(_ requestJSON: String) -> String {
        guard let data = requestJSON.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data),
              let req = obj as? [String: Any],
              let id = req["id"] as? String,
              let op = req["op"] as? String,
              let origin = req["origin"] as? String,
              let options = req["options"] as? [String: Any] else {
            return Self.errorJSON(id: "", name: "NotAllowedError",
                                  message: "Malformed passkey request.")
        }
        do {
            switch op {
            case "create":
                let cred = try register(options: options, origin: origin)
                return Self.successJSON(id: id, credential: cred)
            case "get":
                let cred = try assert(options: options, origin: origin)
                return Self.successJSON(id: id, credential: cred)
            default:
                return Self.errorJSON(id: id, name: "NotSupportedError",
                                      message: "Unsupported operation.")
            }
        } catch let e as WebAuthnError {
            return Self.errorJSON(id: id, name: e.name, message: e.message)
        } catch {
            return Self.errorJSON(id: id, name: "NotAllowedError",
                                  message: error.localizedDescription)
        }
    }

    // MARK: Registration (navigator.credentials.create)

    private func register(options: [String: Any],
                          origin: String) throws -> [String: Any] {
        let rp = options["rp"] as? [String: Any]
        let binding = try Self.validateBinding(origin: origin,
                                               requestedRPID: rp?["id"] as? String)
        let rpId = binding.rpId
        guard let challenge = options["challenge"] as? String else {
            throw WebAuthnError("NotAllowedError", "Missing challenge.")
        }

        // We only implement ES256 (-7). Refuse if the RP won't accept it.
        let params = options["pubKeyCredParams"] as? [[String: Any]] ?? []
        let acceptsES256 = params.isEmpty || params.contains {
            ($0["alg"] as? Int) == -7
        }
        guard acceptsES256 else {
            throw WebAuthnError("NotSupportedError",
                                "No supported credential algorithm (need ES256).")
        }

        // Honor excludeCredentials: if we already hold one of them, abort.
        let exclude = (options["excludeCredentials"] as? [[String: Any]]) ?? []
        let excludeIds = Set(exclude.compactMap { $0["id"] as? String })
        if !excludeIds.isEmpty {
            for existing in store.credentials(forRpId: rpId)
            where excludeIds.contains(Base64URL.encode(existing.credentialId)) {
                throw WebAuthnError("InvalidStateError",
                                    "A credential already exists for this site.")
            }
        }

        let user = options["user"] as? [String: Any]
        let userName = (user?["name"] as? String) ?? ""
        let userHandleB64 = (user?["id"] as? String) ?? ""

        // Verify the user (Touch ID) before minting the credential.
        try verifyUser(reason: "Create a passkey for \(rpId)")

        let credentialId = Self.randomBytes(16)
        let (publicKeyXY, keyBlob) = try KeyManager.createKey()

        let cosePublicKey = try Self.coseKey(from: publicKeyXY)
        let authData = Self.authenticatorData(
            rpId: rpId,
            flags: 0x45,  // UP | UV | AT
            attestedCredentialData: Self.aaguid
                + UInt16(credentialId.count).bigEndianBytes
                + credentialId + cosePublicKey)

        let attestationObject = CBORWriter.map([
            (.text("fmt"), .text("none")),
            (.text("attStmt"), .map([])),
            (.text("authData"), .bytes(authData)),
        ])

        let clientDataJSON = Self.clientDataJSON(type: "webauthn.create",
                                                 challenge: challenge,
                                                 origin: binding.origin)

        // Persist the credential record (sign counter stays 0, like Apple's
        // own platform passkeys, so there is nothing to update per-assertion).
        store.add(CredentialRecord(credentialId: credentialId,
                                   rpId: rpId,
                                   userHandleB64: userHandleB64,
                                   userName: userName,
                                   keyBlob: keyBlob))

        return [
            "id": Base64URL.encode(credentialId),
            "rawId": Base64URL.encode(credentialId),
            "type": "public-key",
            "authenticatorAttachment": "platform",
            "response": [
                "clientDataJSON": Base64URL.encode(clientDataJSON),
                "attestationObject": Base64URL.encode(attestationObject),
                "transports": ["internal"],
                "publicKeyAlgorithm": -7,
                "authenticatorData": Base64URL.encode(authData),
            ],
        ]
    }

    // MARK: Assertion (navigator.credentials.get)

    private func assert(options: [String: Any],
                        origin: String) throws -> [String: Any] {
        let binding = try Self.validateBinding(origin: origin,
                                               requestedRPID: options["rpId"] as? String)
        let rpId = binding.rpId
        guard let challenge = options["challenge"] as? String else {
            throw WebAuthnError("NotAllowedError", "Missing challenge.")
        }

        let allow = (options["allowCredentials"] as? [[String: Any]]) ?? []
        let allowIds = Set(allow.compactMap { $0["id"] as? String })

        // Candidate credentials: those we hold for this RP, optionally filtered
        // to the allowCredentials list. Newest first so a fresh re-registration
        // wins when several match.
        var candidates = store.credentials(forRpId: rpId)
        if !allowIds.isEmpty {
            candidates = candidates.filter {
                allowIds.contains(Base64URL.encode($0.credentialId))
            }
        }
        guard let record = candidates.last else {
            throw WebAuthnError("NotAllowedError",
                                "No passkey found for this site.")
        }

        try verifyUser(reason: "Sign in to \(rpId)")

        let authData = Self.authenticatorData(rpId: rpId,
                                              flags: 0x05,  // UP | UV
                                              attestedCredentialData: Data())
        let clientDataJSON = Self.clientDataJSON(type: "webauthn.get",
                                                 challenge: challenge,
                                                 origin: binding.origin)
        let clientDataHash = Data(SHA256.hash(data: clientDataJSON))
        let signature = try KeyManager.sign(blob: record.keyBlob,
                                            message: authData + clientDataHash)

        var response: [String: Any] = [
            "clientDataJSON": Base64URL.encode(clientDataJSON),
            "authenticatorData": Base64URL.encode(authData),
            "signature": Base64URL.encode(signature),
        ]
        response["userHandle"] = record.userHandleB64.isEmpty
            ? NSNull() : record.userHandleB64

        return [
            "id": Base64URL.encode(record.credentialId),
            "rawId": Base64URL.encode(record.credentialId),
            "type": "public-key",
            "authenticatorAttachment": "platform",
            "response": response,
        ]
    }

    // MARK: User verification

    /// Present Touch ID (with system passcode fallback) to verify the user.
    /// Throws a WebAuthn NotAllowedError if the user cancels or fails. This is
    /// how we satisfy WebAuthn "user verification" for our hardware-bound keys.
    private func verifyUser(reason: String) throws {
        let context = LAContext()
        let sem = DispatchSemaphore(value: 0)
        var ok = false
        var failure: Error?
        context.evaluatePolicy(.deviceOwnerAuthentication,
                               localizedReason: reason) { success, err in
            ok = success
            failure = err
            sem.signal()
        }
        sem.wait()
        guard ok else {
            throw WebAuthnError("NotAllowedError",
                                failure?.localizedDescription
                                    ?? "User verification was cancelled.")
        }
    }

    // MARK: WebAuthn data builders

    private static func authenticatorData(rpId: String, flags: UInt8,
                                          attestedCredentialData: Data) -> Data {
        var data = Data()
        data.append(Data(SHA256.hash(data: Data(rpId.utf8))))  // rpIdHash (32)
        data.append(flags)                                     // flags (1)
        data.append(contentsOf: [0, 0, 0, 0])                  // signCount (4)
        data.append(attestedCredentialData)
        return data
    }

    private static func clientDataJSON(type: String, challenge: String,
                                       origin: String) -> Data {
        // Built by hand with a stable key order (type, challenge, origin,
        // crossOrigin) matching what Chromium emits; values are JSON-escaped.
        let ordered = "{\"type\":\(Self.jsonString(type)),"
            + "\"challenge\":\(Self.jsonString(challenge)),"
            + "\"origin\":\(Self.jsonString(origin)),"
            + "\"crossOrigin\":false}"
        return Data(ordered.utf8)
    }

    private static func coseKey(from xy: (x: Data, y: Data)) throws -> Data {
        // COSE_Key for an EC2 ES256 public key.
        CBORWriter.map([
            (.int(1), .int(2)),     // kty: EC2
            (.int(3), .int(-7)),    // alg: ES256
            (.int(-1), .int(1)),    // crv: P-256
            (.int(-2), .bytes(xy.x)),
            (.int(-3), .bytes(xy.y)),
        ])
    }

    // MARK: Helpers

    private static func validateBinding(origin rawOrigin: String,
                                        requestedRPID: String?) throws
        -> (origin: String, rpId: String) {
        guard let components = URLComponents(string: rawOrigin),
              let scheme = components.scheme?.lowercased(),
              let rawHost = components.host,
              components.user == nil,
              components.password == nil,
              components.query == nil,
              components.fragment == nil,
              components.path.isEmpty || components.path == "/" else {
            throw WebAuthnError("SecurityError", "Invalid WebAuthn origin.")
        }

        let host = normalizeHost(rawHost)
        guard !host.isEmpty else {
            throw WebAuthnError("SecurityError", "Invalid WebAuthn origin.")
        }
        guard isTrustworthyOrigin(scheme: scheme, host: host) else {
            throw WebAuthnError("SecurityError", "Passkeys require a secure origin.")
        }

        let rpId = normalizeHost(requestedRPID ?? host)
        guard isValidRPID(rpId), rpIdMatchesOriginHost(rpId: rpId, host: host) else {
            throw WebAuthnError("SecurityError",
                                "Relying party ID does not match the origin.")
        }
        return (serializedOrigin(scheme: scheme,
                                 host: host,
                                 port: components.port),
                rpId)
    }

    private static func normalizeHost(_ host: String) -> String {
        var normalized = host.trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        while normalized.hasSuffix(".") {
            normalized.removeLast()
        }
        return normalized
    }

    private static func isTrustworthyOrigin(scheme: String, host: String) -> Bool {
        if scheme == "https" { return true }
        if scheme == "http" && isLocalHost(host) { return true }
        return false
    }

    private static func isValidRPID(_ rpId: String) -> Bool {
        guard !rpId.isEmpty,
              rpId.rangeOfCharacter(from: .whitespacesAndNewlines) == nil,
              rpId.rangeOfCharacter(from: .controlCharacters) == nil,
              !rpId.contains("/"),
              !rpId.contains(":"),
              !isIPv4Literal(rpId) else {
            return false
        }
        if rpId == "localhost" || rpId.hasSuffix(".localhost") {
            return true
        }
        let labels = rpId.split(separator: ".", omittingEmptySubsequences: false)
        guard labels.count >= 2,
              labels.allSatisfy({ !$0.isEmpty && $0.count <= 63 }) else {
            return false
        }
        let allowed = CharacterSet(charactersIn:
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-")
        return labels.allSatisfy { label in
            let text = String(label)
            guard text.rangeOfCharacter(from: allowed.inverted) == nil else {
                return false
            }
            return !(text.hasPrefix("-") || text.hasSuffix("-"))
        }
    }

    private static func rpIdMatchesOriginHost(rpId: String, host: String) -> Bool {
        host == rpId || host.hasSuffix("." + rpId)
    }

    private static func serializedOrigin(scheme: String,
                                         host: String,
                                         port: Int?) -> String {
        let defaultPort = (scheme == "https" && port == 443)
            || (scheme == "http" && port == 80)
        guard let port, !defaultPort else {
            return "\(scheme)://\(host)"
        }
        return "\(scheme)://\(host):\(port)"
    }

    private static func isLocalHost(_ host: String) -> Bool {
        host == "localhost"
            || host.hasSuffix(".localhost")
            || host == "::1"
            || (isIPv4Literal(host) && host.hasPrefix("127."))
    }

    private static func isIPv4Literal(_ host: String) -> Bool {
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return false }
        return parts.allSatisfy { part in
            guard let value = Int(part), (0...255).contains(value) else {
                return false
            }
            return String(value) == part || part == "0"
        }
    }

    private static func randomBytes(_ count: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        _ = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        return Data(bytes)
    }

    private static func jsonString(_ s: String) -> String {
        if let d = try? JSONSerialization.data(withJSONObject: [s]),
           let str = String(data: d, encoding: .utf8) {
            // Strip the surrounding [ ] from the single-element array.
            return String(str.dropFirst().dropLast())
        }
        return "\"\""
    }

    // MARK: Response serialization

    private static func successJSON(id: String,
                                    credential: [String: Any]) -> String {
        jsonString(["id": id, "ok": true, "credential": credential])
    }

    private static func errorJSON(id: String, name: String,
                                  message: String) -> String {
        jsonString(["id": id, "ok": false, "error": name, "message": message])
    }

    private static func jsonString(_ obj: [String: Any]) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: obj),
              let str = String(data: data, encoding: .utf8) else {
            return "{\"ok\":false,\"error\":\"UnknownError\"}"
        }
        return str
    }
}

private struct WebAuthnError: Error {
    let name: String
    let message: String
    init(_ name: String, _ message: String) {
        self.name = name
        self.message = message
    }
}
