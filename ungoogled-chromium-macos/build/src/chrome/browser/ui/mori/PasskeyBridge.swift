import SwiftUI

// Wires web pages' `navigator.credentials.create/get` to Mori's native WebAuthn
// platform authenticator (`MoriPasskeys` in PasskeyAuthenticator.swift).
//
// Why a JS shim + poll instead of a direct call: Mori embeds the engine in a
// style where Chromium's own passkey UI/platform-authenticator path is inert
// (see PasskeyAuthenticator.swift's header), and there is no synchronous
// page→native channel in this build — every native↔page bridge here works by
// injecting JS that exposes `window.__moriX` globals which native then polls via
// `evaluateJavaScript` (the same pattern as the media agent and context menu).
//
// So: a main-world shim overrides `navigator.credentials`, queues each request,
// and returns a Promise. Native polls `__moriPasskeyTake()` for queued requests,
// runs `MoriPasskeys.handle` (Secure Enclave + Touch ID), and pushes the result
// back with `__moriPasskeyResolve(...)`, which settles the page's Promise. The
// agent benefits for free: once passkeys work, it can click a site's "Sign in
// with a passkey" button and the user completes the Touch ID prompt.

enum PasskeyScripts {
    /// Main-world shim. Idempotent per document (guarded by a global flag). Never
    /// throws into the page or native: WebAuthn calls reject cleanly on failure,
    /// and non-WebAuthn `navigator.credentials` calls fall through to the engine.
    static let shim = #"""
    (() => {
      if (window.__moriPasskeyInstalled) return;
      window.__moriPasskeyInstalled = true;

      const b64uEncode = (buf) => {
        const bytes = new Uint8Array(buf);
        let s = "";
        for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
        return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
      };
      const b64uDecode = (str) => {
        if (str == null) return null;
        let s = String(str).replace(/-/g, "+").replace(/_/g, "/");
        while (s.length % 4) s += "=";
        const bin = atob(s);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        return bytes.buffer;
      };
      // ArrayBuffer / TypedArray -> base64url; pass strings through unchanged.
      const toB64u = (v) => {
        if (v == null) return v;
        if (v instanceof ArrayBuffer) return b64uEncode(v);
        if (ArrayBuffer.isView(v)) {
          return b64uEncode(v.buffer.slice(v.byteOffset, v.byteOffset + v.byteLength));
        }
        return v;
      };

      const pending = new Map(); // id -> { resolve, reject }
      let outbox = [];           // request JSON strings awaiting native pickup
      let seq = 0;
      const newId = () =>
        "pk-" + (seq++) + "-" + Date.now() + "-" + Math.random().toString(36).slice(2);

      // Native pulls (and clears) the queued requests.
      window.__moriPasskeyTake = function () {
        const out = outbox;
        outbox = [];
        return out;
      };

      // Native pushes a response: { id, ok, credential?, error?, message? }.
      window.__moriPasskeyResolve = function (resp) {
        try {
          if (!resp || !resp.id) return;
          const entry = pending.get(resp.id);
          if (!entry) return;
          pending.delete(resp.id);
          if (resp.ok && resp.credential) {
            entry.resolve(buildCredential(resp.credential));
          } else {
            entry.reject(new DOMException(resp.message || "Passkey request failed.",
                                          resp.error || "NotAllowedError"));
          }
        } catch (e) { /* never throw back into native */ }
      };

      // Reconstruct a PublicKeyCredential-shaped object from native's JSON.
      const buildCredential = (c) => {
        const r = c.response || {};
        const response = {};
        if (r.clientDataJSON != null) response.clientDataJSON = b64uDecode(r.clientDataJSON);
        if (r.attestationObject != null) {
          // create() — attestation response
          response.attestationObject = b64uDecode(r.attestationObject);
          const authData = r.authenticatorData != null ? b64uDecode(r.authenticatorData) : null;
          const transports = Array.isArray(r.transports) ? r.transports.slice() : ["internal"];
          response.getAuthenticatorData = () => authData;
          response.getTransports = () => transports;
          response.getPublicKey = () => null;
          response.getPublicKeyAlgorithm = () =>
            (r.publicKeyAlgorithm != null ? r.publicKeyAlgorithm : -7);
        } else {
          // get() — assertion response
          if (r.authenticatorData != null) response.authenticatorData = b64uDecode(r.authenticatorData);
          if (r.signature != null) response.signature = b64uDecode(r.signature);
          response.userHandle = (r.userHandle != null) ? b64uDecode(r.userHandle) : null;
        }
        // Give the response the matching native prototype so RPs that do
        // `resp instanceof AuthenticatorA*Response` pass.
        try {
          const rproto = (r.attestationObject != null)
            ? (window.AuthenticatorAttestationResponse && window.AuthenticatorAttestationResponse.prototype)
            : (window.AuthenticatorAssertionResponse && window.AuthenticatorAssertionResponse.prototype);
          if (rproto) Object.setPrototypeOf(response, rproto);
        } catch (e) {}

        const cred = {
          id: c.id,
          rawId: b64uDecode(c.rawId),
          type: c.type || "public-key",
          authenticatorAttachment: c.authenticatorAttachment || "platform",
          response: response,
          getClientExtensionResults: () => ({}),
          // Standardized serialization several RP libraries call. Re-uses the
          // native base64url strings, so it matches the registration/auth JSON.
          toJSON: function () {
            const rr = c.response || {};
            const out = {
              id: c.id,
              rawId: c.rawId,
              type: c.type || "public-key",
              authenticatorAttachment: c.authenticatorAttachment || "platform",
              clientExtensionResults: {},
              response: {}
            };
            if (rr.clientDataJSON != null) out.response.clientDataJSON = rr.clientDataJSON;
            if (rr.attestationObject != null) {
              out.response.attestationObject = rr.attestationObject;
              if (rr.authenticatorData != null) out.response.authenticatorData = rr.authenticatorData;
              out.response.transports = Array.isArray(rr.transports) ? rr.transports : ["internal"];
              out.response.publicKeyAlgorithm = (rr.publicKeyAlgorithm != null) ? rr.publicKeyAlgorithm : -7;
            } else {
              if (rr.authenticatorData != null) out.response.authenticatorData = rr.authenticatorData;
              if (rr.signature != null) out.response.signature = rr.signature;
              if (rr.userHandle != null) out.response.userHandle = rr.userHandle;
            }
            return out;
          }
        };
        // Make `cred instanceof PublicKeyCredential` pass; our own data
        // properties shadow the native prototype's slot-backed getters.
        try {
          if (window.PublicKeyCredential && window.PublicKeyCredential.prototype) {
            Object.setPrototypeOf(cred, window.PublicKeyCredential.prototype);
          }
        } catch (e) {}
        return cred;
      };

      const send = (op, options, signal, timeoutMs) => new Promise((resolve, reject) => {
        const id = newId();
        let settled = false;
        let timer = null;
        const cleanup = () => {
          if (timer) { clearTimeout(timer); timer = null; }
          if (signal) { try { signal.removeEventListener("abort", onAbort); } catch (e) {} }
        };
        const finish = (fn, arg) => {
          if (settled) return;
          settled = true;
          pending.delete(id);
          cleanup();
          fn(arg);
        };
        function onAbort() {
          finish(reject, new DOMException("The operation was aborted.", "AbortError"));
        }
        // The native side settles via these wrapped handlers.
        pending.set(id, {
          resolve: (v) => finish(resolve, v),
          reject: (e) => finish(reject, e)
        });
        if (signal) {
          if (signal.aborted) { onAbort(); return; }
          try { signal.addEventListener("abort", onAbort); } catch (e) {}
        }
        // Always settle eventually so a dropped native reply can't hang the page.
        const ms = (typeof timeoutMs === "number" && timeoutMs > 0) ? Math.min(timeoutMs, 600000) : 180000;
        timer = setTimeout(() => {
          finish(reject, new DOMException("Passkey request timed out.", "NotAllowedError"));
        }, ms);
        let req;
        try {
          req = JSON.stringify({ id, op, origin: location.origin, options });
        } catch (e) {
          finish(reject, new DOMException("Could not serialize passkey request.", "NotAllowedError"));
          return;
        }
        outbox.push(req);
      });

      const createOptions = (pk) => {
        const o = {
          challenge: toB64u(pk.challenge),
          rp: pk.rp ? { id: pk.rp.id, name: pk.rp.name } : undefined,
          pubKeyCredParams: Array.isArray(pk.pubKeyCredParams)
            ? pk.pubKeyCredParams.map((p) => ({ type: p.type, alg: p.alg }))
            : []
        };
        if (pk.user) {
          o.user = { id: toB64u(pk.user.id), name: pk.user.name, displayName: pk.user.displayName };
        }
        if (Array.isArray(pk.excludeCredentials)) {
          o.excludeCredentials = pk.excludeCredentials.map((c) => ({ type: c.type, id: toB64u(c.id) }));
        }
        if (pk.authenticatorSelection) o.authenticatorSelection = pk.authenticatorSelection;
        if (pk.timeout != null) o.timeout = pk.timeout;
        return o;
      };

      const getOptions = (pk) => {
        const o = { challenge: toB64u(pk.challenge), rpId: pk.rpId };
        if (Array.isArray(pk.allowCredentials)) {
          o.allowCredentials = pk.allowCredentials.map((c) => ({ type: c.type, id: toB64u(c.id) }));
        }
        if (pk.userVerification) o.userVerification = pk.userVerification;
        if (pk.timeout != null) o.timeout = pk.timeout;
        return o;
      };

      if (!navigator.credentials) {
        try {
          Object.defineProperty(navigator, "credentials", { value: {}, configurable: true });
        } catch (e) {}
      }
      const creds = navigator.credentials || {};
      const origCreate = creds.create ? creds.create.bind(creds) : null;
      const origGet = creds.get ? creds.get.bind(creds) : null;

      const create = function (options) {
        if (!options || !options.publicKey) {
          return origCreate ? origCreate(options)
            : Promise.reject(new DOMException("Not supported.", "NotSupportedError"));
        }
        return send("create", createOptions(options.publicKey),
                    options.signal, options.publicKey.timeout);
      };
      const get = function (options) {
        if (!options || !options.publicKey) {
          return origGet ? origGet(options)
            : Promise.reject(new DOMException("Not supported.", "NotSupportedError"));
        }
        return send("get", getOptions(options.publicKey),
                    options.signal, options.publicKey.timeout);
      };
      try { creds.create = create; } catch (e) {}
      try { creds.get = get; } catch (e) {}

      // Many relying parties gate the passkey UI on these probes.
      try {
        if (!window.PublicKeyCredential) { window.PublicKeyCredential = function () {}; }
        window.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable =
          () => Promise.resolve(true);
        window.PublicKeyCredential.isConditionalMediationAvailable =
          () => Promise.resolve(false);
      } catch (e) {}
    })();
    """#

    /// Encode a Swift string as a safe JS string literal (handles quotes,
    /// backslashes, control chars). Used to hand native JSON back to the shim.
    static func jsStringLiteral(_ s: String) -> String {
        if let data = try? JSONSerialization.data(withJSONObject: [s]),
           let arr = String(data: data, encoding: .utf8) {
            // `arr` looks like `["...escaped..."]`; strip the surrounding brackets.
            return String(arr.dropFirst().dropLast())
        }
        return "\"\""
    }
}

extension BrowserTab {
    /// Inject the passkey shim into this web tab's main world. Idempotent per
    /// document; never realizes an agent tab.
    func installPasskeyShim() {
        guard kind == .web, hasRealized else { return }
        Task { @MainActor in _ = try? await evaluateJavaScript(PasskeyScripts.shim) }
    }
}

extension BrowserStore {
    /// How often to drain queued passkey requests from each live web tab. Snappy
    /// enough that the Touch ID prompt feels immediate after the user/agent
    /// triggers a ceremony, cheap enough to leave running.
    private static let passkeyPollInterval: TimeInterval = 0.5

    func startPasskeyPolling() {
        passkeyPollTimer?.invalidate()
        let timer = Timer.scheduledTimer(withTimeInterval: Self.passkeyPollInterval,
                                         repeats: true) { [weak self] _ in
            self?.pollPasskeyRequests()
        }
        timer.tolerance = 0.15
        passkeyPollTimer = timer
    }

    private func pollPasskeyRequests() {
        for tab in tabs where tab.kind == .web && tab.hasRealized && !tab.isAsleep {
            Task { @MainActor in
                guard
                    let result = try? await tab.evaluateJavaScript(
                        "window.__moriPasskeyTake ? window.__moriPasskeyTake() : []"),
                    let requests = result as? [Any], !requests.isEmpty
                else { return }
                for case let requestJSON as String in requests {
                    self.handlePasskeyRequest(requestJSON, in: tab)
                }
            }
        }
    }

    private func handlePasskeyRequest(_ requestJSON: String, in tab: BrowserTab) {
        // MoriPasskeys.handle does the Secure Enclave / Touch ID work off-main
        // and calls back on the main thread with the response JSON.
        MoriPasskeys.handle(requestJSON) { [weak tab] responseJSON in
            guard let tab else { return }
            let literal = PasskeyScripts.jsStringLiteral(responseJSON)
            let js = "window.__moriPasskeyResolve && window.__moriPasskeyResolve(JSON.parse(\(literal)))"
            Task { @MainActor in _ = try? await tab.evaluateJavaScript(js) }
        }
    }
}
