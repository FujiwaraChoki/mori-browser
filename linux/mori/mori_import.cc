#include "chrome/browser/ui/views/mori/mori_import.h"

#include <map>
#include <optional>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "build/build_config.h"
#include "base/files/file_enumerator.h"
#include "base/base_paths.h"
#include "base/containers/span.h"
#include "base/files/file_path.h"
#include "base/files/file_util.h"
#include "base/functional/bind.h"
#include "base/json/json_reader.h"
#include "base/path_service.h"
#include "base/strings/string_number_conversions.h"
#include "base/strings/string_split.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/thread_pool.h"
#include "base/time/time.h"
#include "base/values.h"
#include "chrome/browser/ui/browser_tabrestore.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/mori/mori_favicons.h"
#include "chrome/browser/ui/views/mori/mori_toast.h"
#include "components/sessions/core/serialized_navigation_entry.h"
#include "components/sessions/core/serialized_user_agent_override.h"
#include "url/gurl.h"

namespace mori {
namespace {

struct ImportedTab {
  std::string url;
  std::string title;
};

// A Zen workspace, Firefox window or Arc space: its pinned tabs first,
// then its folders' tabs, then the rest.
struct ImportedSpace {
  std::string name;
  std::vector<ImportedTab> tabs;
};

struct Imported {
  bool found = false;
  std::vector<ImportedTab> favorites;
  std::vector<ImportedSpace> spaces;
};

// Pages worth bringing over (not about:, chrome:, file: pages...).
bool Importable(const std::string& url) {
  const GURL gurl(url);
  return gurl.is_valid() && gurl.SchemeIsHTTPOrHTTPS();
}

// ---- Firefox and Zen ----------------------------------------------------

// Mozilla's "mozLz40\0" container: magic, the decoded size (u32 LE) and
// one LZ4 block.
std::optional<std::string> DecompressMozLz4(const std::string& bytes) {
  constexpr std::string_view kMagic("mozLz40\0", 8);
  if (bytes.size() < 12 || std::string_view(bytes).substr(0, 8) != kMagic) {
    return std::nullopt;
  }
  const auto in = base::as_byte_span(bytes);
  const size_t size = static_cast<size_t>(in[8]) | (in[9] << 8) |
                      (in[10] << 16) | (static_cast<size_t>(in[11]) << 24);
  // A damaged header must not ask for gigabytes.
  if (size > (512u << 20) || size > bytes.size() * 255) {
    return std::nullopt;
  }
  std::string out;
  out.reserve(size);
  size_t i = 12;
  auto read_length = [&](size_t length) -> std::optional<size_t> {
    if (length != 15) {
      return length;
    }
    uint8_t b = 0;
    do {
      if (i >= in.size()) {
        return std::nullopt;
      }
      b = in[i++];
      length += b;
    } while (b == 255);
    return length;
  };
  while (i < in.size()) {
    const uint8_t token = in[i++];
    std::optional<size_t> literals = read_length(token >> 4);
    if (!literals || i + *literals > in.size() ||
        out.size() + *literals > size) {
      return std::nullopt;
    }
    out.append(bytes, i, *literals);
    i += *literals;
    if (i >= in.size()) {
      break;  // The last sequence has literals only.
    }
    if (i + 2 > in.size()) {
      return std::nullopt;
    }
    const size_t offset = in[i] | (in[i + 1] << 8);
    i += 2;
    std::optional<size_t> match = read_length(token & 15);
    if (!match || offset == 0 || offset > out.size() ||
        out.size() + *match + 4 > size) {
      return std::nullopt;
    }
    // Byte by byte: the match may overlap what it copies.
    const size_t from = out.size() - offset;
    for (size_t k = 0; k < *match + 4; ++k) {
      out.push_back(out[from + k]);
    }
  }
  if (out.size() != size) {
    return std::nullopt;
  }
  return out;
}

// The profile in use from a profiles.ini: the install's default, else the
// one marked Default=1, else the first.
std::optional<base::FilePath> ProfileFromIni(const base::FilePath& root) {
  std::string ini;
  if (!base::ReadFileToString(root.AppendASCII("profiles.ini"), &ini)) {
    return std::nullopt;
  }
  std::map<std::string, std::map<std::string, std::string>> sections;
  std::string section;
  for (std::string_view line : base::SplitStringPiece(
           ini, "\n", base::TRIM_WHITESPACE, base::SPLIT_WANT_NONEMPTY)) {
    if (line.starts_with('[') && line.ends_with(']')) {
      section = std::string(line.substr(1, line.size() - 2));
    } else if (size_t eq = line.find('='); eq != std::string_view::npos) {
      sections[section][std::string(line.substr(0, eq))] =
          std::string(line.substr(eq + 1));
    }
  }
  auto resolve = [&](const std::map<std::string, std::string>& profile) {
    auto path = profile.find("Path");
    if (path == profile.end()) {
      return base::FilePath();
    }
    auto relative = profile.find("IsRelative");
    return relative != profile.end() && relative->second == "0"
               ? base::FilePath(path->second)
               : root.Append(path->second);
  };
  for (const auto& [name, values] : sections) {
    if (name.starts_with("Install")) {
      if (auto it = values.find("Default"); it != values.end()) {
        return root.Append(it->second);
      }
    }
  }
  std::optional<base::FilePath> first;
  for (const auto& [name, values] : sections) {
    if (!name.starts_with("Profile")) {
      continue;
    }
    if (auto it = values.find("Default"); it != values.end() && it->second == "1") {
      return resolve(values);
    }
    if (!first) {
      first = resolve(values);
    }
  }
  return first;
}

std::optional<base::FilePath> FindMozillaProfile(ImportSource source) {
#if BUILDFLAG(IS_WIN)
  base::FilePath roaming;
  base::PathService::Get(base::DIR_ROAMING_APP_DATA, &roaming);
  const base::FilePath root =
      source == ImportSource::kZen
          ? roaming.AppendASCII("zen")
          : roaming.AppendASCII("Mozilla").AppendASCII("Firefox");
  return ProfileFromIni(root);
#else
  base::FilePath home;
  base::PathService::Get(base::DIR_HOME, &home);
#if BUILDFLAG(IS_MAC)
  const std::vector<const char*> roots =
      source == ImportSource::kZen
          ? std::vector<const char*>{"Library/Application Support/zen"}
          : std::vector<const char*>{"Library/Application Support/Firefox"};
#else
  const std::vector<const char*> roots =
      source == ImportSource::kZen
          ? std::vector<const char*>{".config/zen", ".zen",
                                     ".var/app/app.zen_browser.zen/.zen"}
          : std::vector<const char*>{
                ".config/mozilla/firefox", ".mozilla/firefox",
                ".var/app/org.mozilla.firefox/.mozilla/firefox",
                "snap/firefox/common/.mozilla/firefox"};
#endif
  for (const char* root : roots) {
    if (std::optional<base::FilePath> profile =
            ProfileFromIni(home.AppendASCII(root))) {
      return profile;
    }
  }
  return std::nullopt;
#endif
}

const std::string* Str(const base::DictValue& dict, std::string_view key) {
  return dict.FindString(key);
}

bool Flag(const base::DictValue& dict, std::string_view key) {
  return dict.FindBool(key).value_or(false);
}

// The page a session tab shows (`index` is 1-based into `entries`); for
// pinned tabs Zen remembers the page they were pinned at, which is where
// they reset to.
std::optional<ImportedTab> SessionTab(const base::DictValue& tab) {
  const base::DictValue* entry = nullptr;
  if (const base::DictValue* pinned =
          tab.FindDictByDottedPath("_zenPinnedInitialState.entry");
      pinned && Str(*pinned, "url")) {
    entry = pinned;
  }
  if (!entry) {
    const base::ListValue* entries = tab.FindList("entries");
    if (!entries || entries->empty()) {
      return std::nullopt;
    }
    const int index = tab.FindInt("index").value_or(entries->size());
    const size_t at = std::clamp<size_t>(index > 0 ? index - 1 : 0, 0,
                                         entries->size() - 1);
    entry = (*entries)[at].GetIfDict();
  }
  const std::string* url = entry ? Str(*entry, "url") : nullptr;
  if (!url || !Importable(*url)) {
    return std::nullopt;
  }
  std::string title;
  if (const std::string* label = Str(tab, "zenStaticLabel");
      label && !base::TrimWhitespaceASCII(*label, base::TRIM_ALL).empty()) {
    title = *label;
  } else if (const std::string* entry_title = Str(*entry, "title")) {
    title = *entry_title;
  }
  return ImportedTab{*url, title};
}

Imported ParseMozillaSession(const base::Value& json) {
  Imported out;
  out.found = true;
  std::vector<const base::DictValue*> windows;
  if (const base::ListValue* list =
          json.is_dict() ? json.GetDict().FindList("windows") : nullptr) {
    for (const base::Value& window : *list) {
      if (const base::DictValue* dict = window.GetIfDict()) {
        windows.push_back(dict);
      }
    }
  } else if (json.is_dict()) {
    windows.push_back(&json.GetDict());  // Zen's zen-sessions.
  }
  auto tabs_of = [](const base::DictValue& window) {
    std::vector<const base::DictValue*> tabs;
    if (const base::ListValue* list = window.FindList("tabs")) {
      for (const base::Value& tab : *list) {
        if (const base::DictValue* dict = tab.GetIfDict()) {
          tabs.push_back(dict);
        }
      }
    }
    return tabs;
  };
  bool zen = false;
  for (const base::DictValue* window : windows) {
    if (window->Find("spaces")) {
      zen = true;
    }
    for (const base::DictValue* tab : tabs_of(*window)) {
      if (Str(*tab, "zenWorkspace")) {
        zen = true;
      }
    }
  }

  if (!zen) {
    // Firefox: a space per window; pinned tabs first, then the rest.
    int number = 0;
    for (const base::DictValue* window : windows) {
      ImportedSpace space;
      space.name = "Window " + base::NumberToString(++number);
      std::vector<ImportedTab> rest;
      for (const base::DictValue* tab : tabs_of(*window)) {
        if (Flag(*tab, "hidden")) {
          continue;
        }
        if (std::optional<ImportedTab> imported = SessionTab(*tab)) {
          (Flag(*tab, "pinned") ? space.tabs : rest).push_back(*imported);
        }
      }
      space.tabs.insert(space.tabs.end(), rest.begin(), rest.end());
      out.spaces.push_back(std::move(space));
    }
    return out;
  }

  // Zen: essentials become favorites; each workspace a space.
  std::vector<std::string> order;
  std::map<std::string, ImportedSpace> spaces;
  std::map<std::string, std::vector<ImportedTab>> pinned;
  std::map<std::string, std::vector<ImportedTab>> folders;
  for (const base::DictValue* window : windows) {
    if (const base::ListValue* list = window->FindList("spaces")) {
      for (const base::Value& item : *list) {
        const base::DictValue* dict = item.GetIfDict();
        const std::string* uuid = dict ? Str(*dict, "uuid") : nullptr;
        if (uuid && !spaces.contains(*uuid)) {
          order.push_back(*uuid);
          const std::string* name = Str(*dict, "name");
          spaces[*uuid].name = name ? *name : std::string();
        }
      }
    }
  }
  std::set<std::string> favorite_urls;
  for (const base::DictValue* window : windows) {
    for (const base::DictValue* tab : tabs_of(*window)) {
      if (Flag(*tab, "zenIsEmpty") || Flag(*tab, "zenIsGlance")) {
        continue;
      }
      std::optional<ImportedTab> imported = SessionTab(*tab);
      if (!imported) {
        continue;
      }
      if (Flag(*tab, "zenEssential")) {
        if (favorite_urls.insert(imported->url).second) {
          out.favorites.push_back(*imported);
        }
        continue;
      }
      // Tabs of a workspace the file doesn't list go to the first one.
      std::string workspace;
      if (const std::string* id = Str(*tab, "zenWorkspace");
          id && spaces.contains(*id)) {
        workspace = *id;
      } else if (!order.empty()) {
        workspace = order.front();
      }
      if (!spaces.contains(workspace)) {
        order.push_back(workspace);
        spaces[workspace];
      }
      if (Str(*tab, "groupId")) {
        folders[workspace].push_back(*imported);
      } else if (Flag(*tab, "pinned")) {
        pinned[workspace].push_back(*imported);
      } else {
        spaces[workspace].tabs.push_back(*imported);
      }
    }
  }
  for (const std::string& id : order) {
    ImportedSpace space = std::move(spaces[id]);
    std::vector<ImportedTab> tabs = std::move(pinned[id]);
    tabs.insert(tabs.end(), folders[id].begin(), folders[id].end());
    tabs.insert(tabs.end(), space.tabs.begin(), space.tabs.end());
    space.tabs = std::move(tabs);
    out.spaces.push_back(std::move(space));
  }
  return out;
}

Imported ReadMozilla(ImportSource source) {
  std::optional<base::FilePath> profile = FindMozillaProfile(source);
  if (!profile) {
    return {};
  }
  for (const char* name :
       {"zen-sessions.jsonlz4", "sessionstore-backups/recovery.jsonlz4",
        "sessionstore.jsonlz4", "sessionstore-backups/previous.jsonlz4"}) {
    std::string bytes;
    if (!base::ReadFileToString(profile->AppendASCII(name), &bytes)) {
      continue;
    }
    std::optional<std::string> json = DecompressMozLz4(bytes);
    std::optional<base::Value> value =
        json ? base::JSONReader::Read(*json, base::JSON_PARSE_RFC)
             : std::nullopt;
    if (value) {
      return ParseMozillaSession(*value);
    }
  }
  return {};
}

// ---- Arc ------------------------------------------------------------------

std::optional<ImportedTab> ArcTab(const base::DictValue& item) {
  const base::DictValue* tab = item.FindDictByDottedPath("data.tab");
  const std::string* url = tab ? Str(*tab, "savedURL") : nullptr;
  if (!url || !Importable(*url)) {
    return std::nullopt;
  }
  std::string title;
  if (const std::string* own = Str(item, "title");
      own && !base::TrimWhitespaceASCII(*own, base::TRIM_ALL).empty()) {
    title = *own;
  } else if (const std::string* saved = Str(*tab, "savedTitle")) {
    title = *saved;
  }
  return ImportedTab{*url, title};
}

Imported ReadArc() {
  base::FilePath home;
  base::PathService::Get(base::DIR_HOME, &home);
  std::string text;
#if BUILDFLAG(IS_WIN)
  // Arc for Windows is a packaged app:
  // Packages\TheBrowserCompany.Arc_<id>\LocalCache\Local\Arc.
  base::FilePath local;
  if (base::PathService::Get(base::DIR_LOCAL_APP_DATA, &local)) {
    base::FileEnumerator packages(local.AppendASCII("Packages"),
                                  /*recursive=*/false,
                                  base::FileEnumerator::DIRECTORIES,
                                  FILE_PATH_LITERAL("TheBrowserCompany.Arc*"));
    for (base::FilePath package = packages.Next();
         !package.empty() && text.empty(); package = packages.Next()) {
      if (!base::ReadFileToString(package.AppendASCII("LocalCache")
                                      .AppendASCII("Local")
                                      .AppendASCII("Arc")
                                      .AppendASCII("StorableSidebar.json"),
                                  &text)) {
        text.clear();
      }
    }
  }
#endif
  // Arc's own file on macOS (and Windows, above); elsewhere, a copy of it.
  for (const char* where : {
#if BUILDFLAG(IS_MAC)
           "Library/Application Support/Arc/StorableSidebar.json",
#endif
           "StorableSidebar.json", "Downloads/StorableSidebar.json"}) {
    if (!text.empty() || base::ReadFileToString(home.AppendASCII(where), &text)) {
      break;
    }
  }
  std::optional<base::Value> json =
      text.empty() ? std::nullopt
                   : base::JSONReader::Read(text, base::JSON_PARSE_RFC);
  const base::ListValue* containers =
      json && json->is_dict()
          ? json->GetDict().FindListByDottedPath("sidebar.containers")
          : nullptr;
  if (!containers) {
    return {};
  }
  const base::DictValue* container = nullptr;
  for (const base::Value& item : *containers) {
    if (item.is_dict() && item.GetDict().Find("items")) {
      container = &item.GetDict();
    }
  }
  if (!container) {
    return {};
  }
  Imported out;
  out.found = true;
  // Arc's lists are flattened [id, object, id, object, ...].
  std::map<std::string, const base::DictValue*> items;
  for (const base::Value& item : *container->FindList("items")) {
    if (const base::DictValue* dict = item.GetIfDict()) {
      if (const std::string* id = Str(*dict, "id")) {
        items[*id] = dict;
      }
    }
  }
  auto ids = [](const base::ListValue* list) {
    std::vector<std::string> out;
    if (list) {
      for (const base::Value& v : *list) {
        if (v.is_string()) {
          out.push_back(v.GetString());
        }
      }
    }
    return out;
  };
  auto children = [&](const std::string& id) {
    std::vector<const base::DictValue*> out;
    auto it = items.find(id);
    if (it != items.end()) {
      for (const std::string& child : ids(it->second->FindList("childrenIds"))) {
        if (auto c = items.find(child); c != items.end()) {
          out.push_back(c->second);
        }
      }
    }
    return out;
  };
  // Tabs under an item, folders flattened.
  auto collect = [&](auto& self, const base::DictValue& item,
                     std::vector<ImportedTab>& tabs, int depth) -> void {
    if (depth > 32) {
      return;
    }
    const std::string* id = Str(item, "id");
    if (!id) {
      return;
    }
    for (const base::DictValue* child : children(*id)) {
      if (std::optional<ImportedTab> tab = ArcTab(*child)) {
        tabs.push_back(*tab);
      } else {
        self(self, *child, tabs, depth + 1);
      }
    }
  };
  for (const std::string& top : ids(container->FindList("topAppsContainerIDs"))) {
    for (const base::DictValue* child : children(top)) {
      if (std::optional<ImportedTab> tab = ArcTab(*child)) {
        out.favorites.push_back(*tab);
      }
    }
  }
  if (const base::ListValue* spaces = container->FindList("spaces")) {
    for (const base::Value& value : *spaces) {
      const base::DictValue* space = value.GetIfDict();
      if (!space) {
        continue;
      }
      ImportedSpace imported;
      if (const std::string* title = Str(*space, "title")) {
        imported.name = *title;
      }
      const std::vector<std::string> container_ids =
          ids(space->FindList("containerIDs"));
      auto after = [&](std::string_view label) -> std::optional<std::string> {
        for (size_t i = 0; i + 1 < container_ids.size(); ++i) {
          if (container_ids[i] == label) {
            return container_ids[i + 1];
          }
        }
        return std::nullopt;
      };
      for (std::string_view part : {"pinned", "unpinned"}) {
        if (std::optional<std::string> id = after(part)) {
          if (auto it = items.find(*id); it != items.end()) {
            collect(collect, *it->second, imported.tabs, 0);
          }
        }
      }
      out.spaces.push_back(std::move(imported));
    }
  }
  return out;
}

// ---- Into the window ----------------------------------------------------

// An unloaded tab, as session restore makes them: it loads when shown.
void AddUnloadedTab(BrowserWindowInterface* browser,
                    const ImportedTab& tab,
                    int index,
                    std::optional<tab_groups::TabGroupId> group,
                    bool pin) {
  sessions::SerializedNavigationEntry entry;
  entry.set_index(0);
  entry.set_virtual_url(GURL(tab.url));
  entry.set_title(base::UTF8ToUTF16(tab.title));
  entry.set_transition_type(ui::PAGE_TRANSITION_AUTO_BOOKMARK);
  entry.set_timestamp(base::Time::Now());
  const sessions::SerializedNavigationEntry entries[] = {entry};
  content::WebContents* contents = chrome::AddRestoredTab(
      browser, entries, index, 0, std::string(), group,
      /*select=*/false, pin, base::TimeTicks(), base::Time(), nullptr,
      sessions::SerializedUserAgentOverride(), {},
      /*from_session_restore=*/false, std::nullopt);
  if (contents) {
    FetchIconForUnloadedTab(contents);
  }
}

void Apply(base::WeakPtr<BrowserWindowInterface> weak_browser,
           ImportSource source,
           Imported imported) {
  BrowserWindowInterface* browser = weak_browser.get();
  if (!browser) {
    return;
  }
  const char16_t* name = source == ImportSource::kZen       ? u"Zen"
                         : source == ImportSource::kFirefox ? u"Firefox"
                                                            : u"Arc";
  if (!imported.found) {
#if BUILDFLAG(IS_MAC) || BUILDFLAG(IS_WIN)
    ShowToast(browser, std::u16string(u"No ") + name + u" data found");
#else
    ShowToast(browser, source == ImportSource::kArc
                           ? u"Put a copy of Arc's StorableSidebar.json in "
                             u"your home or Downloads folder"
                           : std::u16string(u"No ") + name +
                                 u" profile found");
#endif
    return;
  }
  TabStripModel* model = browser->GetTabStripModel();
  // Favorites, without doubles.
  std::set<std::string> have;
  for (int i = 0; i < model->IndexOfFirstNonPinnedTab(); ++i) {
    have.insert(model->GetWebContentsAt(i)->GetVisibleURL().spec());
  }
  for (const ImportedTab& favorite : imported.favorites) {
    if (have.insert(GURL(favorite.url).spec()).second) {
      AddUnloadedTab(browser, favorite, model->IndexOfFirstNonPinnedTab(),
                     std::nullopt, /*pin=*/true);
    }
  }
  // Each workspace's or window's tabs, one after the other, after the
  // tabs already open.
  int added = 0;
  for (const ImportedSpace& space : imported.spaces) {
    for (const ImportedTab& tab : space.tabs) {
      AddUnloadedTab(browser, tab, model->count(), std::nullopt,
                     /*pin=*/false);
      ++added;
    }
  }
  ShowToast(browser,
            added == 0 ? std::u16string(u"Nothing to import from ") + name
                       : u"Imported " + base::NumberToString16(added) +
                             (added == 1 ? u" tab" : u" tabs") + u" from " +
                             name);
}

}  // namespace

void ImportTabs(BrowserWindowInterface* browser, ImportSource source) {
  base::ThreadPool::PostTaskAndReplyWithResult(
      FROM_HERE, {base::MayBlock(), base::TaskPriority::USER_VISIBLE},
      base::BindOnce(
          [](ImportSource source) {
            return source == ImportSource::kArc ? ReadArc()
                                                : ReadMozilla(source);
          },
          source),
      base::BindOnce(&Apply, browser->GetWeakPtr(), source));
}

}  // namespace mori
