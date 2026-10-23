// Browser-neutral, read-only session parsing, shared with parser regressions.
#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_IMPORT_PARSER_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_IMPORT_PARSER_H_

#include <algorithm>
#include <map>
#include <optional>
#include <ranges>
#include <set>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "base/strings/string_number_conversions.h"
#include "base/strings/string_util.h"
#include "base/values.h"
#include "chrome/browser/ui/views/mori/mori_folder_tree.h"
#include "url/gurl.h"

namespace mori::importer {

struct ImportedFolder {
  // Source identity, scoped to a workspace; names are not identities.
  std::string id;
  std::string name;
  bool open = true;
  std::vector<ImportedFolder> ancestors = {};
};

struct ImportedTab {
  std::string url;
  std::string title;
  std::optional<ImportedFolder> folder;
};

// A Zen workspace, Firefox window or Arc space. Folder subtrees come first,
// followed by ordinary tabs. Favorites are imported separately.
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
inline bool Importable(const std::string& url) {
  const GURL gurl(url);
  return gurl.is_valid() && gurl.SchemeIsHTTPOrHTTPS();
}

inline const std::string* Str(const base::DictValue& dict,
                              std::string_view key) {
  return dict.FindString(key);
}

inline bool Flag(const base::DictValue& dict, std::string_view key) {
  return dict.FindBool(key).value_or(false);
}

// The page a session tab shows (`index` is 1-based into `entries`); for
// pinned tabs Zen remembers the page they were pinned at, which is where
// they reset to.
inline std::optional<ImportedTab> SessionTab(const base::DictValue& tab) {
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
    const size_t at =
        std::clamp<size_t>(index > 0 ? index - 1 : 0, 0, entries->size() - 1);
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
  return ImportedTab{*url, title, std::nullopt};
}

inline ImportedFolder ZenFolder(
    const std::string& id,
    const std::map<std::string, const base::DictValue*>& folder_data) {
  std::vector<ImportedFolder> path;
  std::set<std::string> seen;
  std::string current = id;
  while (!current.empty() && seen.insert(current).second && seen.size() <= 32) {
    auto it = folder_data.find(current);
    if (it == folder_data.end()) {
      path.push_back({current, "Folder", true});
      break;
    }
    const auto& data = *it->second;
    const std::string* name = Str(data, "name");
    path.push_back({current, name && !name->empty() ? *name : "Folder",
                    !Flag(data, "collapsed")});
    const std::string* parent = Str(data, "parentId");
    current = parent ? *parent : std::string();
  }
  if (path.empty()) {
    return {id, "Folder", true};
  }
  ImportedFolder folder = path.front();
  folder.ancestors.assign(path.rbegin(), path.rend() - 1);
  return folder;
}

inline void OrderFolderTabs(std::vector<ImportedTab>& tabs) {
  std::vector<std::vector<std::string>> paths;
  for (const auto& tab : tabs) {
    std::vector<std::string> path;
    if (tab.folder) {
      for (const auto& ancestor : tab.folder->ancestors) {
        path.push_back(ancestor.id);
      }
      path.push_back(tab.folder->id);
    }
    paths.push_back(std::move(path));
  }
  std::vector<ImportedTab> ordered;
  for (const auto& row : FolderTreeRows(paths)) {
    if (row.tab) {
      ordered.push_back(std::move(tabs[*row.tab]));
    }
  }
  tabs = std::move(ordered);
}

inline Imported ParseMozillaSession(const base::Value& json) {
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
  std::map<std::string, std::map<std::string, std::vector<ImportedTab>>>
      folders;
  std::map<std::string, const base::DictValue*> folder_data;
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
  // Zen stores folder metadata separately from the member tabs. Keep IDs,
  // including when two folders have the same display name.
  for (const base::DictValue* window : windows) {
    for (const char* key : {"folders", "groups"}) {
      if (const base::ListValue* list = window->FindList(key)) {
        for (const base::Value& value : *list) {
          if (const base::DictValue* dict = value.GetIfDict()) {
            if (const std::string* id = Str(*dict, "id")) {
              folder_data.try_emplace(*id, dict);
            }
          }
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
      if (const std::string* group = Str(*tab, "groupId")) {
        imported->folder = ZenFolder(*group, folder_data);
        folders[workspace][*group].push_back(*imported);
      }
      spaces[workspace].tabs.push_back(*imported);
    }
  }
  // A native folder needs a tab to carry its session data. As with New
  // Folder, give imported empty folders a blank tab so they survive restart.
  for (const auto& [id, data] : folder_data) {
    const std::string* owner = Str(*data, "workspaceId");
    const std::string workspace =
        owner && spaces.contains(*owner)
            ? *owner
            : (order.empty() ? std::string() : order.front());
    if (!spaces.contains(workspace)) {
      order.push_back(workspace);
      spaces[workspace];
    }
    if (!folders[workspace].contains(id)) {
      // A folder may only be described in one window, with its tabs in
      // another. Don't manufacture a second folder in the default workspace.
      const bool used = std::ranges::any_of(folders, [&](const auto& entry) {
        if (entry.second.contains(id)) {
          return true;
        }
        for (const auto& [child, tabs] : entry.second) {
          if (!tabs.empty() && tabs.front().folder &&
              std::ranges::any_of(
                  tabs.front().folder->ancestors,
                  [&](const auto& ancestor) { return ancestor.id == id; })) {
            return true;
          }
        }
        return false;
      });
      if (!used) {
        ImportedTab empty{"chrome://newtab/", "New Tab",
                          ZenFolder(id, folder_data)};
        folders[workspace][id].push_back(empty);
        spaces[workspace].tabs.push_back(std::move(empty));
      }
    }
  }
  for (const std::string& id : order) {
    ImportedSpace space = std::move(spaces[id]);
    OrderFolderTabs(space.tabs);
    out.spaces.push_back(std::move(space));
  }
  return out;
}

inline std::optional<ImportedTab> ArcTab(const base::DictValue& item) {
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
  return ImportedTab{*url, title, std::nullopt};
}

inline Imported ParseArcSidebar(const base::Value& json) {
  const base::ListValue* containers =
      json.is_dict() ? json.GetDict().FindListByDottedPath("sidebar.containers")
                     : nullptr;
  if (!containers) {
    return {};
  }
  const base::DictValue* container = nullptr;
  for (const base::Value& item : *containers) {
    if (item.is_dict() && item.GetDict().FindList("items")) {
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
      for (const std::string& child :
           ids(it->second->FindList("childrenIds"))) {
        if (auto c = items.find(child); c != items.end()) {
          out.push_back(c->second);
        }
      }
    }
    return out;
  };
  // Keep the original folder chain on each tab, including empty leaves.
  std::set<std::string> visited;
  auto collect = [&](auto& self, const base::DictValue& item,
                     std::vector<ImportedTab>& tabs,
                     std::optional<ImportedFolder> folder, int depth) -> void {
    const std::string* id = Str(item, "id");
    if (!id || depth > 32 || !visited.insert(*id).second) {
      return;
    }
    if (item.FindDictByDottedPath("data.list")) {
      const std::string* title = Str(item, "title");
      std::string name = title && !title->empty() ? *title : "Folder";
      ImportedFolder nested{*id, name, true};
      if (folder) {
        nested.ancestors = folder->ancestors;
        auto parent = *folder;
        parent.ancestors.clear();
        nested.ancestors.push_back(std::move(parent));
      }
      folder = std::move(nested);
    }
    const size_t first = tabs.size();
    for (const base::DictValue* child : children(*id)) {
      if (std::optional<ImportedTab> tab = ArcTab(*child)) {
        const std::string* child_id = Str(*child, "id");
        if (child_id && visited.insert(*child_id).second) {
          tab->folder = folder;
          tabs.push_back(*tab);
        }
      } else {
        self(self, *child, tabs, folder, depth + 1);
      }
    }
    if (folder && folder->id == *id &&
        std::none_of(tabs.begin() + first, tabs.end(),
                     [&](const ImportedTab& tab) {
                       return tab.folder &&
                              (tab.folder->id == *id ||
                               std::ranges::any_of(tab.folder->ancestors,
                                                   [&](const auto& ancestor) {
                                                     return ancestor.id == *id;
                                                   }));
                     })) {
      tabs.insert(tabs.begin() + first,
                  ImportedTab{"chrome://newtab/", "New Tab", folder});
    }
  };
  for (const std::string& top :
       ids(container->FindList("topAppsContainerIDs"))) {
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
      visited.clear();
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
            collect(collect, *it->second, imported.tabs, std::nullopt, 0);
          }
        }
      }
      OrderFolderTabs(imported.tabs);
      out.spaces.push_back(std::move(imported));
    }
  }
  return out;
}

}  // namespace mori::importer

#endif
