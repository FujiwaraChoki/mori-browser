// JavaScript agent injected at the start of every frame to make WebAuthn /
// passkeys work under CEF's Alloy embedding (which lacks Chromium's native
// passkey support — see PasskeyAuthenticator.swift).
//
//   • kMoriPasskeyAgent overrides navigator.credentials.create()/get() for
//     `publicKey` requests. Conditional-mediation get() requests wait silently
//     (no modal) since we have no inline autofill chip; the explicit "use a
//     passkey" button issues a normal get() that runs the Touch ID ceremony.
//   • kMoriPasskeyCapabilities overrides PublicKeyCredential's capability
//     probes (UVPAA, conditional mediation, getClientCapabilities) so relying
//     parties report full passkey support. It is a SEPARATE snippet because it
//     must be re-injected *after* extension content scripts (a password manager
//     such as Proton Pass wraps these probes at document_start and reports
//     partial support); see BrowserClient::OnLoadStart/OnLoadEnd.
//   • Serializes the request (BufferSources → base64url) and ships it to native
//     over the console channel (`console.debug('__MORI_WEBAUTHN__' + json)`),
//     captured in BrowserClient::OnConsoleMessage.
//   • Native runs the Touch ID ceremony and calls back into
//     `window.__moriWAResolve(json)`, which we turn into a real
//     PublicKeyCredential and use to settle the pending promise.
//
// Non-publicKey credential requests fall through to the original implementation.
#pragma once

static const char kMoriPasskeyAgent[] = R"JS(
(function(){
  if (window.__moriWAInstalled) { return; }
  window.__moriWAInstalled = true;

  var CC = window.CredentialsContainer && window.CredentialsContainer.prototype;
  var nav = window.navigator;
  if (!CC || !nav || !nav.credentials || !window.PublicKeyCredential) { return; }

  // Native channel (captured up-front so a page that later wraps console.debug
  // can't break us).
  var send = (window.console && console.debug)
      ? console.debug.bind(console) : function(){};

  var pending = Object.create(null);
  var seq = 0;
  function nextId(){
    seq++;
    return seq + '-' + Math.random().toString(36).slice(2);
  }

  // --- base64url <-> ArrayBuffer -------------------------------------------
  function toBytes(src){
    if (src == null) { return null; }
    if (src instanceof ArrayBuffer) { return new Uint8Array(src); }
    if (ArrayBuffer.isView(src)) {
      return new Uint8Array(src.buffer, src.byteOffset, src.byteLength);
    }
    return null;
  }
  function bufToB64url(src){
    var bytes = toBytes(src);
    if (!bytes) { return null; }
    var s = '';
    for (var i = 0; i < bytes.length; i++) { s += String.fromCharCode(bytes[i]); }
    return btoa(s).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'');
  }
  function b64urlToBuf(str){
    var s = String(str).replace(/-/g,'+').replace(/_/g,'/');
    while (s.length % 4) { s += '='; }
    var bin = atob(s);
    var bytes = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) { bytes[i] = bin.charCodeAt(i); }
    return bytes.buffer;
  }

  // --- option serialization -------------------------------------------------
  function mapCredList(list){
    if (!Array.isArray(list)) { return undefined; }
    return list.map(function(c){
      return {
        type: c.type || 'public-key',
        id: bufToB64url(c.id),
        transports: c.transports
      };
    });
  }
  function serializeCreate(pk){
    var out = {
      challenge: bufToB64url(pk.challenge),
      rp: pk.rp,
      user: pk.user ? {
        id: bufToB64url(pk.user.id),
        name: pk.user.name,
        displayName: pk.user.displayName
      } : undefined,
      pubKeyCredParams: pk.pubKeyCredParams,
      timeout: pk.timeout,
      excludeCredentials: mapCredList(pk.excludeCredentials),
      authenticatorSelection: pk.authenticatorSelection,
      attestation: pk.attestation
    };
    return out;
  }
  function serializeGet(pk){
    return {
      challenge: bufToB64url(pk.challenge),
      rpId: pk.rpId,
      timeout: pk.timeout,
      allowCredentials: mapCredList(pk.allowCredentials),
      userVerification: pk.userVerification
    };
  }

  // --- response reconstruction ---------------------------------------------
  function buildCredential(c){
    var resp = c.response || {};
    var response;
    if (resp.attestationObject !== undefined) {
      response = {
        clientDataJSON: b64urlToBuf(resp.clientDataJSON),
        attestationObject: b64urlToBuf(resp.attestationObject),
        getTransports: function(){ return resp.transports || ['internal']; },
        getAuthenticatorData: function(){
          return resp.authenticatorData ? b64urlToBuf(resp.authenticatorData) : null;
        },
        getPublicKey: function(){ return null; },
        getPublicKeyAlgorithm: function(){
          return resp.publicKeyAlgorithm != null ? resp.publicKeyAlgorithm : -7;
        }
      };
    } else {
      response = {
        clientDataJSON: b64urlToBuf(resp.clientDataJSON),
        authenticatorData: b64urlToBuf(resp.authenticatorData),
        signature: b64urlToBuf(resp.signature),
        userHandle: resp.userHandle ? b64urlToBuf(resp.userHandle) : null
      };
    }
    var cred = {
      id: c.id,
      rawId: b64urlToBuf(c.rawId),
      type: 'public-key',
      authenticatorAttachment: c.authenticatorAttachment || 'platform',
      response: response,
      getClientExtensionResults: function(){ return {}; }
    };
    try { Object.setPrototypeOf(cred, window.PublicKeyCredential.prototype); }
    catch (e) {}
    return cred;
  }

  // Native -> page bridge: settle the matching pending promise.
  window.__moriWAResolve = function(json){
    var msg;
    try { msg = JSON.parse(json); } catch (e) { return; }
    var entry = pending[msg.id];
    if (!entry) { return; }
    delete pending[msg.id];
    if (msg.ok) {
      try { entry.resolve(buildCredential(msg.credential)); }
      catch (e) { entry.reject(e); }
    } else {
      var err;
      try { err = new DOMException(msg.message || 'Request failed',
                                   msg.error || 'NotAllowedError'); }
      catch (e2) { err = new Error(msg.message || 'Request failed'); }
      entry.reject(err);
    }
  };

  function dispatch(op, pk, signal, mediation){
    return new Promise(function(resolve, reject){
      if (signal && signal.aborted) {
        reject(new DOMException('Aborted', 'AbortError'));
        return;
      }
      var id = nextId();
      pending[id] = { resolve: resolve, reject: reject };
      if (signal) {
        signal.addEventListener('abort', function(){
          if (pending[id]) {
            delete pending[id];
            reject(new DOMException('Aborted', 'AbortError'));
          }
        });
      }
      // Conditional mediation (passkey autofill): per the WebAuthn spec this must
      // never interrupt the user with a modal. We have no inline autofill chip, so
      // the request waits silently and only settles if the page aborts it — at
      // which point the user falls back to the explicit "Sign in with a passkey"
      // button, which issues a normal (modal) get() that we satisfy via Touch ID.
      // This matches how Safari/Chrome behave when nothing is autofilled, and lets
      // us advertise conditional mediation so relying parties (e.g. GitHub) report
      // full — not partial — passkey support.
      if (op === 'get' && mediation === 'conditional') {
        return;
      }
      var req = {
        id: id,
        op: op,
        origin: window.location.origin,
        rpId: (op === 'create' ? (pk.rp && pk.rp.id) : pk.rpId)
              || window.location.hostname,
        options: (op === 'create' ? serializeCreate(pk) : serializeGet(pk))
      };
      try { send('__MORI_WEBAUTHN__' + JSON.stringify(req)); }
      catch (e) {
        delete pending[id];
        reject(new DOMException('Passkey bridge unavailable',
                                'NotAllowedError'));
      }
    });
  }

  // --- overrides ------------------------------------------------------------
  var origCreate = CC.create;
  var origGet = CC.get;

  CC.create = function(options){
    if (options && options.publicKey) {
      return dispatch('create', options.publicKey, options.signal);
    }
    return origCreate.apply(this, arguments);
  };
  CC.get = function(options){
    if (options && options.publicKey) {
      return dispatch('get', options.publicKey, options.signal,
                      options.mediation);
    }
    return origGet.apply(this, arguments);
  };

  // Capability probes are installed separately (kMoriPasskeyCapabilities) so we
  // can re-assert them *after* extension content scripts run — see below.
})();
)JS";

// Capability probes, kept separate from the credential overrides above.
//
// We advertise a user-verifying platform authenticator AND conditional
// mediation so relying parties treat the browser as having full (not partial)
// passkey support. The crucial reason this lives in its own snippet: a
// password-manager extension (e.g. Proton Pass) injects a MAIN-world content
// script at document_start that wraps PublicKeyCredential's static methods and
// reports isUserVerifyingPlatformAuthenticatorAvailable() === false in this
// CEF embedding — which is exactly what surfaces "this browser is reporting
// partial passkey support". We inject this snippet *after* the extension
// content scripts (end of OnLoadStart, and again in OnLoadEnd) so Mori's
// honest capability values win, without disturbing the extension's own
// credential create/get handling (so its passkeys keep working).
//
// These native static methods are writable, so Object.defineProperty cleanly
// overwrites whatever the extension installed.
static const char kMoriPasskeyCapabilities[] = R"JS(
(function(){
  if (!window.PublicKeyCredential) { return; }
  function defineCap(name, fn){
    try {
      Object.defineProperty(window.PublicKeyCredential, name, {
        value: fn, writable: true, configurable: true, enumerable: true
      });
    } catch (e) {
      try { window.PublicKeyCredential[name] = fn; } catch (e2) {}
    }
  }
  defineCap('isUserVerifyingPlatformAuthenticatorAvailable',
            function(){ return Promise.resolve(true); });
  defineCap('isConditionalMediationAvailable',
            function(){ return Promise.resolve(true); });
  // Newer feature-detection API (getClientCapabilities).
  defineCap('getClientCapabilities', function(){
    return Promise.resolve({
      conditionalCreate: false,
      conditionalGet: true,
      hybridTransport: false,
      passkeyPlatformAuthenticator: true,
      userVerifyingPlatformAuthenticator: true,
      relatedOrigins: true,
      signalAllAcceptedCredentials: false,
      signalCurrentUserDetails: false,
      signalUnknownCredential: false
    });
  });
})();
)JS";
