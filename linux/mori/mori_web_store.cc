#include "chrome/browser/ui/views/mori/mori_web_store.h"

#include "chrome/common/chrome_isolated_world_ids.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "content/public/browser/web_contents_user_data.h"
#include "extensions/common/extension_urls.h"
#include "url/gurl.h"
#include "url/origin.h"

namespace mori {
namespace {

// In the store's buttons (in any language: "Zu Chrome hinzufügen"), the
// word Chrome becomes Mori, also as the store re-renders (it is one page
// app). Runs in an isolated world, out of the page's reach.
constexpr char16_t kScript[] = uR"js(
(() => {
  if (window.__moriWebStore) return;
  window.__moriWebStore = true;
  // The store asks to switch to Chrome in browsers not named Chrome: the
  // popup (it links to Chrome's download) and the banner (its button is
  // logged as 276336). Mori installs from the store, so both go.
  const style = document.createElement('style');
  style.textContent = `
    [role=dialog]:has(a[href*="google.com/chrome"]),
    div:has(> div > div > button[jslog^="276336"]),
    div:has(> div > div > div > button[jslog^="276336"]) {
      display: none !important;
    }`;
  document.documentElement.append(style);
  const word = /\bChrome\b/g;
  const rename = () => {
    for (const button of document.querySelectorAll('button, [role=button]')) {
      const walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT);
      for (let node; (node = walker.nextNode());) {
        if (node.data.includes('Chrome')) {
          node.data = node.data.replace(word, 'Mori');
        }
      }
      const label = button.getAttribute('aria-label');
      if (label && label.includes('Chrome')) {
        button.setAttribute('aria-label', label.replace(word, 'Mori'));
      }
    }
  };
  let queued = false;
  const queue = () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => { queued = false; rename(); });
  };
  rename();
  new MutationObserver(queue).observe(document.documentElement, {
    subtree: true, childList: true, characterData: true,
  });
})();
)js";

class WebStoreTabHelper
    : public content::WebContentsObserver,
      public content::WebContentsUserData<WebStoreTabHelper> {
 public:
  explicit WebStoreTabHelper(content::WebContents* contents)
      : content::WebContentsObserver(contents),
        content::WebContentsUserData<WebStoreTabHelper>(*contents) {}

  // content::WebContentsObserver:
  void DOMContentLoaded(content::RenderFrameHost* frame) override {
    if (!frame->IsInPrimaryMainFrame() ||
        !frame->GetLastCommittedOrigin().IsSameOriginWith(url::Origin::Create(
            extension_urls::GetNewWebstoreLaunchURL()))) {
      return;
    }
    frame->ExecuteJavaScriptInIsolatedWorld(kScript,
                                            base::NullCallback(),
                                            ISOLATED_WORLD_ID_CHROME_INTERNAL);
  }

 private:
  friend WebContentsUserData;
  WEB_CONTENTS_USER_DATA_KEY_DECL();
};

WEB_CONTENTS_USER_DATA_KEY_IMPL(WebStoreTabHelper);

}  // namespace

void AttachWebStore(content::WebContents* contents) {
  WebStoreTabHelper::CreateForWebContents(contents);
}

}  // namespace mori
