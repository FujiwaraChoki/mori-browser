// Experimental automatic confirmation of the official 1Password passkey UI.
// This deliberately exposes neither the vault nor general extension scripting.
#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_PASSKEYS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_PASSKEYS_H_

#include <optional>
#include <string>
#include <string_view>

#include "base/json/json_reader.h"
#include "base/strings/string_split.h"
#include "url/gurl.h"
#include "url/origin.h"

namespace mori::passkeys {

inline constexpr char kTool[] = "mori_use_1password_passkey";
inline constexpr char kSwitch[] = "mori-auto-passkey-accounts";
inline constexpr char kDefaultAccount[] = "@default";
inline constexpr char kExtensionId[] = "aeblfdkhhhdcdjpifhhbdiojplfjncoa";
inline constexpr char kPromptPath[] =
    "/inline/universal-sign-on/universal-sign-on.html";

// Configuration comes from the browser process command line, never the model
// or a web page. Keys must be canonical HTTPS origins, not URL prefixes.
inline std::optional<std::string> AccountFor(std::string_view configuration,
                                             std::string_view origin) {
  const GURL requested(origin);
  if (!requested.is_valid() || !requested.SchemeIs("https") ||
      url::Origin::Create(requested).Serialize() != origin) {
    return std::nullopt;
  }
  auto accounts =
      base::JSONReader::ReadDict(configuration, base::JSON_PARSE_RFC);
  if (!accounts) {
    return std::nullopt;
  }
  const std::string* account = accounts->FindString(origin);
  if (!account || account->empty() || account->size() > 320) {
    return std::nullopt;
  }
  return *account;
}

inline bool IsPrompt(const GURL& url) {
  return url.SchemeIs("chrome-extension") && url.host() == kExtensionId &&
         url.path() == kPromptPath && !url.has_username() &&
         !url.has_password();
}

// The prompt's tabId is consumed by 1Password. Never confirm a frame that
// points at another tab, or an ambiguous/encoded tabId query.
inline bool IsPromptForTab(const GURL& url, int tab_id) {
  if (!IsPrompt(url)) {
    return false;
  }
  bool found = false;
  for (auto part : base::SplitStringPiece(
           url.query(), "&", base::KEEP_WHITESPACE, base::SPLIT_WANT_ALL)) {
    const auto equals = part.find('=');
    const auto key = part.substr(0, equals);
    if (key.find('%') != std::string_view::npos ||
        key.find('+') != std::string_view::npos) {
      return false;
    }
    if (key == "tabId") {
      if (found || equals == std::string_view::npos ||
          part.substr(equals + 1) != std::to_string(tab_id)) {
        return false;
      }
      found = true;
    }
  }
  return found;
}

// Inspected against official extension 8.12.42.36. English UI only for this
// prototype. Return a status, never item contents, field values or vault data.
// Executed only in a verified extension frame. `account` is a JSON literal.
inline constexpr char kFocusPrompt[] = R"JS(
(() => {
  const account = ACCOUNT_JSON;
  const visible = el => {
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility === 'visible' &&
      s.display !== 'none' && el.checkVisibility({checkOpacity: true}) &&
      !el.closest('[inert],[aria-hidden="true"]');
  };
  const text = el => (el.textContent || '').trim();
  const buttons = [...document.querySelectorAll('[role="button"],button')]
    .filter(visible);
  if (buttons.some(el => text(el).includes('Unlock 1Password to sign in')))
    return 'needs_user_authentication';
  if (window.__moriPasskeyConfirmationRequested)
    return 'confirmation_already_requested';
  const candidates = buttons.filter(el =>
    (el.getAttribute('aria-label') || '').startsWith('Sign in with ') &&
    [...el.querySelectorAll('span')].some(node => text(node) === 'Use passkey') &&
    (account === '@default' ||
      [...el.querySelectorAll('span,p')].some(node => text(node) === account)) &&
    !el.matches(':disabled,[aria-disabled="true"]'));
  if (candidates.length !== 1) return 'no_unique_matching_passkey';
  const target = candidates[0];
  target.focus({preventScroll: true});
  if (document.activeElement !== target) return 'focus_failed';
  // Reserve once in Mori's isolated world, including across React rerenders.
  // A cancelled/stale attempt stays reserved until this prompt closes.
  window.__moriPasskeyConfirmationRequested = true;
  return 'ready';
})()
)JS";

}  // namespace mori::passkeys

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_PASSKEYS_H_
