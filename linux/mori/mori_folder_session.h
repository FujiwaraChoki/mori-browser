// Session format for one tab's folder chain. The leaf retains the original
// single-folder keys so older sessions remain readable.
#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDER_SESSION_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDER_SESSION_H_

#include <set>
#include <string>
#include <vector>

#include "base/json/json_reader.h"
#include "base/json/json_writer.h"
#include "base/values.h"

namespace mori {
struct FolderSessionNode {
  std::string id;
  std::string name;
  std::string icon;
  bool open = true;
};

inline std::string EncodeFolderSession(
    const std::vector<FolderSessionNode>& path) {
  if (path.empty()) {
    return {};
  }
  base::ListValue ancestors;
  base::DictValue leaf;
  std::string parent;
  for (size_t i = 0; i < path.size(); ++i) {
    const auto& node = path[i];
    base::DictValue data;
    data.Set("id", node.id);
    data.Set("name", node.name);
    data.Set("icon", node.icon);
    data.Set("open", node.open);
    data.Set("parent", parent);
    parent = node.id;
    if (i + 1 == path.size()) {
      leaf = std::move(data);
    } else {
      ancestors.Append(std::move(data));
    }
  }
  leaf.Set("ancestors", std::move(ancestors));
  return base::WriteJson(leaf).value_or(std::string());
}

inline std::vector<FolderSessionNode> DecodeFolderSession(
    const std::string& text) {
  const auto leaf = base::JSONReader::ReadDict(text, base::JSON_PARSE_RFC);
  const auto* id = leaf ? leaf->FindString("id") : nullptr;
  if (!id || id->empty()) {
    return {};
  }
  std::vector<FolderSessionNode> path;
  std::set<std::string> seen;
  auto append = [&](const base::DictValue& data) {
    const auto* folder = data.FindString("id");
    if (!folder || folder->empty() || !seen.insert(*folder).second) {
      return false;
    }
    const auto* name = data.FindString("name");
    const auto* icon = data.FindString("icon");
    path.push_back({*folder, name ? *name : std::string(),
                    icon ? *icon : std::string(),
                    data.FindBool("open").value_or(true)});
    return true;
  };
  if (const auto* ancestors = leaf->FindList("ancestors")) {
    for (const auto& value : *ancestors) {
      const auto* data = value.GetIfDict();
      const auto* ancestor = data ? data->FindString("id") : nullptr;
      if (!ancestor || *ancestor == *id || !append(*data)) {
        break;
      }
    }
  }
  append(*leaf);
  return path;
}
}  // namespace mori

#endif
