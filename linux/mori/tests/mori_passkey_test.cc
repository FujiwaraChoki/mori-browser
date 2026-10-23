#include "chrome/browser/ui/views/mori/mori_passkeys.h"

#include <iostream>

#include "base/check.h"

int main() {
  using mori::passkeys::AccountFor;
  using mori::passkeys::IsPromptForTab;
  constexpr char config[] =
      R"({"https://example.com":"alice@example.com","https://example.com:8443":"work"})";
  CHECK_EQ(AccountFor(config, "https://example.com").value(),
           "alice@example.com");
  CHECK_EQ(AccountFor(config, "https://example.com:8443").value(), "work");
  for (auto origin :
       {"http://example.com", "https://example.com/",
        "https://example.com/login", "https://example.com.evil.test",
        "https://other.example.com", "https://example.com:443",
        "https://alice@example.com", "https://example.com?x=1",
        "https://example.com#fragment", "null", ""}) {
    CHECK(!AccountFor(config, origin)) << origin;
  }
  for (auto invalid :
       {"", "[]", "not-json", "{", R"({"https://example.com":false})",
        R"({"https://example.com":""})"}) {
    CHECK(!AccountFor(invalid, "https://example.com"));
  }
  CHECK_EQ(
      AccountFor(R"({"https://example.com":"@default"})", "https://example.com")
          .value(),
      "@default");
  const std::string prompt = std::string("chrome-extension://") +
                             mori::passkeys::kExtensionId +
                             mori::passkeys::kPromptPath;
  CHECK(IsPromptForTab(GURL(prompt + "?tabId=12"), 12));
  CHECK(IsPromptForTab(
      GURL(prompt + "?theme=dark&tabId=12&fillSessionPending=true"), 12));
  for (auto query : {"", "?tabId=13", "?tabId=012", "?tabId=12&tabId=12",
                     "?%74abId=13&tabId=12", "?tab+Id=13&tabId=12",
                     "?tabId=%31%32", "?tabId", "?tabId=12evil"}) {
    CHECK(!IsPromptForTab(GURL(prompt + query), 12)) << query;
  }
  CHECK(!IsPromptForTab(
      GURL("https://example.com" + std::string(mori::passkeys::kPromptPath) +
           "?tabId=12"),
      12));
  CHECK(!IsPromptForTab(
      GURL("chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" +
           std::string(mori::passkeys::kPromptPath) + "?tabId=12"),
      12));
  CHECK(!IsPromptForTab(
      GURL(std::string("chrome-extension://") + mori::passkeys::kExtensionId +
           "/popup/index.html?tabId=12"),
      12));
  std::cout << "Passkey authorization and prompt identity tests passed.\n";
}
