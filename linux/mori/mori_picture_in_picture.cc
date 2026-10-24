#include "chrome/browser/ui/views/mori/mori_picture_in_picture.h"

#include <set>

#include "base/functional/bind.h"
#include "base/callback_list.h"
#include "base/functional/callback_helpers.h"
#include "base/memory/weak_ptr.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "chrome/browser/picture_in_picture/picture_in_picture_window_manager.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/mori/mori_picture_in_picture_script.h"
#include "chrome/browser/ui/views/mori/mori_toast.h"
#include "chrome/common/chrome_isolated_world_ids.h"
#include "components/prefs/pref_change_registrar.h"
#include "components/prefs/pref_registry_simple.h"
#include "components/prefs/pref_service.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/media_player_id.h"
#include "content/public/browser/media_session.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/visibility.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "content/public/browser/web_contents_user_data.h"

namespace mori {
namespace {

class PictureInPictureTabHelper
    : public content::WebContentsObserver,
      public content::WebContentsUserData<PictureInPictureTabHelper> {
 public:
  explicit PictureInPictureTabHelper(content::WebContents* contents)
      : content::WebContentsObserver(contents),
        content::WebContentsUserData<PictureInPictureTabHelper>(*contents) {
    preferences_.Init(Profile::FromBrowserContext(
                          contents->GetBrowserContext())->GetPrefs());
    for (const char* name : {kPictureInPictureEnabled, kPictureInPictureHover,
                             kPictureInPictureAuto, kPictureInPictureReturn}) {
      preferences_.Add(name, base::BindRepeating(
          &PictureInPictureTabHelper::PreferencesChanged,
          base::Unretained(this)));
    }
  }

  void DOMContentLoaded(content::RenderFrameHost* frame) override {
    ObserveTab();
    ApplyToFrame(frame);
  }

  void MediaStartedPlaying(const MediaPlayerInfo& info,
                           const content::MediaPlayerId& id) override {
    if (info.has_video) playing_videos_.insert(id);
  }
  void MediaStoppedPlaying(const MediaPlayerInfo& info,
                           const content::MediaPlayerId& id,
                           MediaStoppedReason reason) override {
    playing_videos_.erase(id);
  }
  void MediaDestroyed(const content::MediaPlayerId& id) override {
    playing_videos_.erase(id);
  }
  void MediaPictureInPictureChanged(bool in_pip) override {
    if (in_pip && auto_requested_) auto_opened_ = true;
    if (!in_pip) auto_opened_ = false;
    auto_requested_ = false;
  }
  void PrimaryPageChanged(content::Page&) override {
    playing_videos_.clear();
    auto_opened_ = false;
    auto_requested_ = false;
    web_contents()->GetPrimaryMainFrame()->ForEachRenderFrameHost(
        [this](content::RenderFrameHost* frame) { ApplyToFrame(frame); });
  }
  void OnVisibilityChanged(content::Visibility visibility) override {
    ScheduleVisibilityUpdate();
  }

 private:
  friend class content::WebContentsUserData<PictureInPictureTabHelper>;

  void ObserveTab() {
    auto* tab = tabs::TabInterface::MaybeGetFromContents(web_contents());
    if (!tab || became_visible_) return;
    became_visible_ = tab->RegisterDidBecomeVisible(base::BindRepeating(
        [](PictureInPictureTabHelper* self, tabs::TabInterface*) {
          self->ScheduleVisibilityUpdate();
        }, base::Unretained(this)));
    activated_ = tab->RegisterDidActivate(base::BindRepeating(
        [](PictureInPictureTabHelper* self, tabs::TabInterface*) {
          self->ScheduleVisibilityUpdate();
        }, base::Unretained(this)));
    deactivated_ = tab->RegisterWillDeactivate(base::BindRepeating(
        [](PictureInPictureTabHelper* self, tabs::TabInterface*) {
          self->ScheduleVisibilityUpdate();
        }, base::Unretained(this)));
    became_hidden_ = tab->RegisterWillBecomeHidden(base::BindRepeating(
        [](PictureInPictureTabHelper* self, tabs::TabInterface*) {
          self->ScheduleVisibilityUpdate();
        }, base::Unretained(this)));
  }

  void ScheduleVisibilityUpdate() {
    // Visibility callbacks can run inside native window creation. Defer all
    // PiP window changes to avoid reentering Chromium's widget lifecycle.
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&PictureInPictureTabHelper::UpdateVisibility,
                                 weak_factory_.GetWeakPtr()));
  }

  PrefService* prefs() { return preferences_.prefs(); }

  void ApplyToFrame(content::RenderFrameHost* frame) {
    if (!frame->IsActive() || !frame->IsRenderFrameLive() ||
        !frame->GetLastCommittedURL().SchemeIsHTTPOrHTTPS()) return;
    const bool show = prefs()->GetBoolean(kPictureInPictureEnabled) &&
                      prefs()->GetBoolean(kPictureInPictureHover);
    const std::string script =
        std::string("window.__moriVideoPipEnabled = ") +
        (show ? "true;\n" : "false;\n") + kPictureInPictureScript;
    frame->ExecuteJavaScriptInIsolatedWorld(
        base::UTF8ToUTF16(script), base::NullCallback(),
        ISOLATED_WORLD_ID_CHROME_INTERNAL);
  }

  void PreferencesChanged() {
    web_contents()->GetPrimaryMainFrame()->ForEachRenderFrameHost(
        [this](content::RenderFrameHost* frame) { ApplyToFrame(frame); });
    if (!prefs()->GetBoolean(kPictureInPictureEnabled)) {
      auto_opened_ = false;
      auto_requested_ = false;
      CloseOwnedWindow();
    }
  }

  void CloseOwnedWindow() {
    auto* manager = PictureInPictureWindowManager::GetInstance();
    if (manager->GetWebContents() == web_contents() &&
        web_contents()->HasPictureInPictureVideo()) {
      PictureInPictureWindowManager::ExitPictureInPictureSoon();
    }
  }

  void UpdateVisibility() {
    const auto* tab = tabs::TabInterface::MaybeGetFromContents(web_contents());
    const bool visible = tab ? (tab->IsActivated() || (tab->IsSplit() && tab->IsVisible()))
        : web_contents()->GetVisibility() == content::Visibility::VISIBLE;
    if (visible) {
      auto_requested_ = false;
      if (auto_opened_ && prefs()->GetBoolean(kPictureInPictureReturn)) {
        auto_opened_ = false;
        CloseOwnedWindow();
      }
      return;
    }
    if ((!tab && web_contents()->GetVisibility() != content::Visibility::HIDDEN) ||
        !prefs()->GetBoolean(kPictureInPictureEnabled) ||
        !prefs()->GetBoolean(kPictureInPictureAuto) || playing_videos_.empty() ||
        auto_requested_ ||
        PictureInPictureWindowManager::GetInstance()->IsInPictureInPicture()) {
      return;
    }
    auto_requested_ = true;
    content::MediaSession::Get(web_contents())->EnterPictureInPicture();
  }

  base::CallbackListSubscription became_visible_;
  base::CallbackListSubscription became_hidden_;
  base::CallbackListSubscription activated_;
  base::CallbackListSubscription deactivated_;
  std::set<content::MediaPlayerId> playing_videos_;
  PrefChangeRegistrar preferences_;
  bool auto_opened_ = false;
  bool auto_requested_ = false;
  base::WeakPtrFactory<PictureInPictureTabHelper> weak_factory_{this};
  WEB_CONTENTS_USER_DATA_KEY_DECL();
};

WEB_CONTENTS_USER_DATA_KEY_IMPL(PictureInPictureTabHelper);

}  // namespace

void RegisterPictureInPicturePrefs(PrefRegistrySimple* registry) {
  registry->RegisterBooleanPref(kPictureInPictureEnabled, true);
  registry->RegisterBooleanPref(kPictureInPictureHover, true);
  registry->RegisterBooleanPref(kPictureInPictureAuto, false);
  registry->RegisterBooleanPref(kPictureInPictureReturn, true);
}

void AttachPictureInPicture(content::WebContents* contents) {
  PictureInPictureTabHelper::CreateForWebContents(contents);
}

bool IsPictureInPictureEnabled(content::WebContents* contents) {
  return Profile::FromBrowserContext(contents->GetBrowserContext())->GetPrefs()
      ->GetBoolean(kPictureInPictureEnabled);
}

void TogglePictureInPicture(BrowserWindowInterface* browser) {
  auto* tab = browser->GetTabStripModel()->GetActiveTab();
  auto* contents = tab ? tab->GetContents() : nullptr;
  if (!contents) return;
  if (!IsPictureInPictureEnabled(contents)) {
    ShowToast(browser, u"Enable Picture in Picture in Settings → Tabs.");
  } else if (contents->HasPictureInPictureVideo()) {
    PictureInPictureWindowManager::ExitPictureInPictureSoon();
  } else {
    content::MediaSession::Get(contents)->EnterPictureInPicture();
  }
}

}  // namespace mori
