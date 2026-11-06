#include "chrome/browser/ui/views/mori/mori_session.h"

#include <algorithm>
#include <memory>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "chrome/browser/sessions/tab_restore_service_factory.h"
#include "components/sessions/core/session_types.h"
#include "components/sessions/core/tab_restore_service.h"
#include "components/sessions/core/tab_restore_service_observer.h"
#include "components/sessions/core/tab_restore_types.h"

namespace mori {
namespace {

// A window's pages, whatever their order. The session and the list record a
// window at the same moment, so a window in both holds the same pages. A
// mismatch only leaves an entry in the list.
using WindowPages = std::vector<std::string>;

template <typename Tabs>
WindowPages PagesOf(const Tabs& tabs) {
  WindowPages pages;
  for (const auto& tab : tabs) {
    if (!tab->navigations.empty()) {
      pages.push_back(
          tab->navigations[tab->normalized_navigation_index()].virtual_url().spec());
    }
  }
  std::ranges::sort(pages);
  return pages;
}

void RemoveEntries(sessions::TabRestoreService* service,
                   const std::set<WindowPages>& restored) {
  std::vector<SessionID> ids;
  for (const auto& entry : service->entries()) {
    if (entry->type != sessions::tab_restore::Type::WINDOW) {
      continue;
    }
    const auto& window =
        static_cast<const sessions::tab_restore::Window&>(*entry);
    if (restored.contains(PagesOf(window.tabs))) {
      ids.push_back(entry->id);
    }
  }
  for (SessionID id : ids) {
    service->RemoveEntryById(id);
  }
}

// The list loads from disk in the background. Deletes itself.
class WaitForLoad : public sessions::TabRestoreServiceObserver {
 public:
  WaitForLoad(sessions::TabRestoreService* service,
              std::set<WindowPages> restored)
      : restored_(std::move(restored)) {
    service->AddObserver(this);
    service->LoadTabsFromLastSession();
  }

  // TabRestoreServiceObserver:
  void TabRestoreServiceLoaded(sessions::TabRestoreService* service) override {
    service->RemoveObserver(this);
    RemoveEntries(service, restored_);
    delete this;
  }
  void TabRestoreServiceDestroyed(
      sessions::TabRestoreService* service) override {
    service->RemoveObserver(this);
    delete this;
  }

 private:
  ~WaitForLoad() override = default;

  const std::set<WindowPages> restored_;
};

}  // namespace

void ForgetRestoredWindowEntries(
    Profile* profile,
    const std::vector<std::unique_ptr<sessions::SessionWindow>>& windows) {
  sessions::TabRestoreService* service =
      TabRestoreServiceFactory::GetForProfile(profile);
  if (!service) {
    return;
  }
  std::set<WindowPages> restored;
  for (const auto& window : windows) {
    if (window->type == sessions::SessionWindow::TYPE_NORMAL &&
        !window->tabs.empty()) {
      restored.insert(PagesOf(window->tabs));
    }
  }
  if (restored.empty()) {
    return;
  }
  if (service->IsLoaded()) {
    RemoveEntries(service, restored);
  } else {
    new WaitForLoad(service, std::move(restored));
  }
}

}  // namespace mori
