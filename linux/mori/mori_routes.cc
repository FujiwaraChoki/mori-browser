#include "chrome/browser/ui/views/mori/mori_routes.h"

#include <map>
#include <memory>
#include <utility>
#include <vector>

#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/strings/string_util.h"
#include "base/values.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/mori/mori_json_file.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "content/public/browser/web_contents.h"
#include "url/gurl.h"

namespace mori {
namespace {

std::string SiteOf(const GURL& url) {
  std::string host = base::ToLowerASCII(url.host());
  if (host.starts_with("www.")) {
    host = host.substr(4);
  }
  return host;
}

class RouteStore {
 public:
  static RouteStore* For(Profile* profile) {
    static base::NoDestructor<
        std::map<base::FilePath, std::unique_ptr<RouteStore>>>
        stores;
    const base::FilePath path = profile->GetOriginalProfile()->GetPath();
    auto& store = (*stores)[path];
    if (!store) {
      store = std::make_unique<RouteStore>(
          path.AppendASCII("Mori Routes.json"));
    }
    return store.get();
  }

  explicit RouteStore(base::FilePath path)
      : file_(std::move(path),
              base::BindOnce(&RouteStore::OnLoaded, base::Unretained(this))) {}

  std::optional<Route> Find(const GURL& url) const {
    if (!url.SchemeIsHTTPOrHTTPS()) {
      return std::nullopt;
    }
    const std::string site = SiteOf(url);
    const Route* best = nullptr;
    for (const Route& route : routes_) {
      if ((site == route.host || site.ends_with("." + route.host)) &&
          (!best || route.host.size() > best->host.size())) {
        best = &route;
      }
    }
    return best ? std::optional<Route>(*best) : std::nullopt;
  }

  void Put(Route route) {
    std::erase_if(routes_,
                  [&](const Route& r) { return r.host == route.host; });
    routes_.push_back(std::move(route));
    Save();
  }

  void Remove(const std::string& host) {
    std::erase_if(routes_, [&](const Route& r) { return r.host == host; });
    Save();
  }

 private:
  void OnLoaded(std::optional<base::Value> value) {
    if (!value || !value->is_list()) {
      return;
    }
    for (const base::Value& item : value->GetList()) {
      const base::DictValue* dict = item.GetIfDict();
      const std::string* host = dict ? dict->FindString("host") : nullptr;
      const std::string* space = dict ? dict->FindString("space") : nullptr;
      if (host && space && !host->empty()) {
        routes_.push_back({*host, *space});
      }
    }
  }

  void Save() {
    base::ListValue list;
    for (const Route& route : routes_) {
      list.Append(
          base::DictValue().Set("host", route.host).Set("space", route.space));
    }
    file_.Write(base::Value(std::move(list)));
  }

  std::vector<Route> routes_;
  // Last: its load callback uses the members above.
  JsonFile file_;
};

}  // namespace

std::optional<Route> RouteFor(BrowserWindowInterface* browser,
                              const GURL& url) {
  return RouteStore::For(browser->GetProfile())->Find(url);
}

void RouteSiteToCurrentSpace(BrowserWindowInterface* browser,
                             const GURL& url) {
  const std::optional<tab_groups::TabGroupId> space = GetCurrentSpace(browser);
  if (!space || !url.SchemeIsHTTPOrHTTPS() || SiteOf(url).empty()) {
    return;
  }
  RouteStore::For(browser->GetProfile())
      ->Put({SiteOf(url), SpaceKey(browser, *space)});
}

void RemoveRoute(BrowserWindowInterface* browser, const std::string& host) {
  RouteStore::For(browser->GetProfile())->Remove(host);
}

void RouteExternalTab(BrowserWindowInterface* browser,
                      content::WebContents* contents,
                      bool first) {
  if (!browser || !contents) {
    return;
  }
  const std::optional<Route> route =
      RouteFor(browser, contents->GetVisibleURL());
  if (!route) {
    return;
  }
  const std::optional<tab_groups::TabGroupId> space =
      FindSpace(browser, route->space);
  TabStripModel* model = browser->GetTabStripModel();
  const int index = model->GetIndexOfWebContents(contents);
  if (!space || index == TabStripModel::kNoTab) {
    return;
  }
  if (model->GetTabGroupForTab(index) != space) {
    if (model->IsTabPinned(index)) {
      model->SetTabPinned(index, false);
    }
    model->AddToExistingGroup({model->GetIndexOfWebContents(contents)},
                              *space);
  }
  if (first) {
    model->SetFocusedGroup(*space);
    model->ActivateTabAt(model->GetIndexOfWebContents(contents));
  }
}

}  // namespace mori
