// The sidebar's folder tree, independent of Chromium views and tab indices.
#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDER_TREE_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_FOLDER_TREE_H_

#include <algorithm>
#include <map>
#include <optional>
#include <set>
#include <string>
#include <vector>

namespace mori {

// A header ignores its own collapsed state; member tabs include it.
template <typename IsOpen>
inline bool FolderPathVisible(const std::vector<std::string>& path,
                              bool include_leaf,
                              IsOpen is_open) {
  const size_t count = path.size() - (!include_leaf && !path.empty() ? 1 : 0);
  for (size_t i = 0; i < count; ++i) {
    if (!is_open(path[i])) {
      return false;
    }
  }
  return true;
}

inline bool FolderCanContain(const std::vector<std::string>& parent_path,
                             const std::string& moved) {
  return std::find(parent_path.begin(), parent_path.end(), moved) ==
         parent_path.end();
}

struct FolderTreeRow {
  std::string folder;
  std::optional<size_t> tab;
  size_t depth = 0;
};

// Paths contain folder IDs, from root to the tab's immediate parent.
// Sibling order follows first appearance. Direct tabs and child folders keep
// their relative order; ordinary tabs always follow all root folders.
inline std::vector<FolderTreeRow> FolderTreeRows(
    const std::vector<std::vector<std::string>>& paths) {
  std::map<std::string, std::vector<FolderTreeRow>> children;
  std::set<std::string> inserted;
  std::vector<FolderTreeRow> loose;
  for (size_t i = 0; i < paths.size(); ++i) {
    std::string parent;
    std::set<std::string> seen;
    for (const auto& id : paths[i]) {
      if (id.empty() || !seen.insert(id).second) {
        break;
      }
      if (inserted.insert(id).second) {
        children[parent].push_back({id, std::nullopt, 0});
      }
      parent = id;
    }
    if (parent.empty()) {
      loose.push_back({{}, i, 0});
    } else {
      children[parent].push_back({{}, i, 0});
    }
  }
  std::vector<FolderTreeRow> rows;
  std::set<std::string> visited;
  auto append = [&](auto& self, const std::string& parent,
                    size_t depth) -> void {
    if (!visited.insert(parent).second) {
      return;
    }
    for (auto row : children[parent]) {
      row.depth = depth;
      rows.push_back(row);
      if (!row.tab) {
        self(self, row.folder, depth + 1);
      }
    }
  };
  append(append, {}, 0);
  rows.insert(rows.end(), loose.begin(), loose.end());
  return rows;
}

}  // namespace mori

#endif
