#include "chrome/browser/ui/views/mori/mori_folder_session.h"
#include "chrome/browser/ui/views/mori/mori_import_parser.h"

#include <iostream>

#include "base/check.h"
#include "base/json/json_reader.h"

namespace {
base::Value Json(std::string_view text) {
  auto json = base::JSONReader::Read(text, base::JSON_PARSE_RFC);
  CHECK(json);
  return std::move(*json);
}

void ZenFolders() {
  const auto imported = mori::importer::ParseMozillaSession(Json(R"({
    "spaces": [{"uuid":"s1","name":"Work"},{"uuid":"s2","name":"Home"}],
    "folders": [
      {"id":"a","name":"Research","collapsed":true},
      {"id":"b","name":"Research"},
      {"id":"c","name":"Child","parentId":"a"}
    ],
    "tabs": [
      {"entries":[{"url":"https://fav.test/"}],"zenEssential":true},
      {"entries":[{"url":"https://a.test/1"}],"zenWorkspace":"s1","groupId":"a"},
      {"entries":[{"url":"https://b.test/"}],"zenWorkspace":"s1","groupId":"b"},
      {"entries":[{"url":"https://a.test/2"}],"zenWorkspace":"s1","groupId":"a"},
      {"entries":[{"url":"https://child.test/"}],"zenWorkspace":"s1","groupId":"c"},
      {"entries":[{"url":"about:blank"}],"zenIsEmpty":true,"groupId":"a"},
      {"entries":[{"url":"https://loose.test/"}],"zenWorkspace":"s1"},
      {"entries":[{"url":"https://pin.test/"}],"zenWorkspace":"s1","pinned":true},
      {"entries":[{"url":"https://home.test/"}],"zenWorkspace":"s2","groupId":"a"}
    ]
  })"));
  CHECK_EQ(imported.favorites.size(), 1u);
  CHECK(!imported.favorites[0].folder);
  CHECK_EQ(imported.spaces.size(), 2u);
  const auto& tabs = imported.spaces[0].tabs;
  CHECK_EQ(tabs.size(), 6u);
  CHECK_EQ(tabs[0].folder->id, "a");
  CHECK_EQ(tabs[1].folder->id, "a");
  CHECK_EQ(tabs[1].url, "https://a.test/2");
  CHECK(!tabs[0].folder->open);
  CHECK_EQ(tabs[2].folder->id, "c");
  CHECK_EQ(tabs[2].folder->name, "Child");
  CHECK_EQ(tabs[2].folder->ancestors.size(), 1u);
  CHECK_EQ(tabs[2].folder->ancestors[0].id, "a");
  CHECK_EQ(tabs[2].folder->ancestors[0].name, "Research");
  CHECK_EQ(tabs[3].folder->id, "b");
  CHECK_EQ(tabs[0].folder->name, tabs[3].folder->name);
  CHECK_EQ(tabs[5].url, "https://pin.test/");
  CHECK(!tabs[4].folder);
  CHECK(!tabs[5].folder);
  CHECK_EQ(imported.spaces[1].tabs[0].folder->id, "a");
}

void ZenMissingMetadataAndCycles() {
  const auto imported = mori::importer::ParseMozillaSession(Json(R"({
    "spaces":[{"uuid":"s"}],
    "groups":[{"id":"a","name":"A","parentId":"b"},
              {"id":"b","name":"B","parentId":"a"}],
    "tabs":[
      {"entries":[{"url":"https://missing.test/"}],"groupId":"missing"},
      {"entries":[{"url":"https://cycle.test/"}],"groupId":"a"}
    ]
  })"));
  CHECK_EQ(imported.spaces[0].tabs.size(), 2u);
  CHECK_EQ(imported.spaces[0].tabs[0].folder->name, "Folder");
  CHECK_EQ(imported.spaces[0].tabs[1].folder->name, "A");
  CHECK_EQ(imported.spaces[0].tabs[1].folder->ancestors[0].name, "B");
}

void ArcFolders() {
  const auto imported = mori::importer::ParseArcSidebar(Json(R"({
    "sidebar":{"containers":[{"items":[
      "P",{"id":"P","childrenIds":["F","G"]},
      "F",{"id":"F","title":"Reading","data":{"list":{}},"childrenIds":["t1","C","t2","F"]},
      "C",{"id":"C","title":"Deep","data":{"list":{}},"childrenIds":["t3"]},
      "G",{"id":"G","title":"Reading","data":{"list":{}},"childrenIds":["t4"]},
      "t1",{"id":"t1","title":"Custom","data":{"tab":{"savedURL":"https://a.test/1"}}},
      "t2",{"id":"t2","data":{"tab":{"savedURL":"https://a.test/2"}}},
      "t3",{"id":"t3","data":{"tab":{"savedURL":"https://deep.test/"}}},
      "t4",{"id":"t4","data":{"tab":{"savedURL":"https://b.test/"}}},
      "U",{"id":"U","childrenIds":["u"]},
      "u",{"id":"u","data":{"tab":{"savedURL":"https://loose.test/"}}}
    ],"spaces":["S",{"title":"Work","containerIDs":["pinned","P","unpinned","U"]}]}]}
  })"));
  CHECK(imported.found);
  CHECK_EQ(imported.spaces.size(), 1u);
  const auto& tabs = imported.spaces[0].tabs;
  CHECK_EQ(tabs.size(), 5u);
  CHECK_EQ(tabs[0].title, "Custom");
  CHECK_EQ(tabs[0].folder->id, "F");
  CHECK_EQ(tabs[1].folder->id, "C");
  CHECK_EQ(tabs[1].folder->name, "Deep");
  CHECK_EQ(tabs[1].folder->ancestors.size(), 1u);
  CHECK_EQ(tabs[1].folder->ancestors[0].id, "F");
  CHECK_EQ(tabs[2].folder->id, "F");
  CHECK_EQ(tabs[3].folder->id, "G");
  CHECK_EQ(tabs[0].folder->name, tabs[3].folder->name);
  CHECK(!tabs[4].folder);
}

void EmptyFolders() {
  auto zen = mori::importer::ParseMozillaSession(Json(R"({
    "spaces":[{"uuid":"s1"},{"uuid":"s2"}],
    "folders":[{"id":"empty","name":"Empty","workspaceId":"s2"}],
    "tabs":[]
  })"));
  CHECK(zen.spaces[0].tabs.empty());
  CHECK_EQ(zen.spaces[1].tabs.size(), 1u);
  CHECK_EQ(zen.spaces[1].tabs[0].folder->name, "Empty");
  CHECK_EQ(zen.spaces[1].tabs[0].url, "chrome://newtab/");
  auto arc = mori::importer::ParseArcSidebar(Json(R"({
    "sidebar":{"containers":[{"items":[
      {"id":"p","childrenIds":["f"]},
      {"id":"f","title":"Empty","data":{"list":{}},"childrenIds":[]}
    ],"spaces":[{"containerIDs":["pinned","p"]}]}]}
  })"));
  CHECK_EQ(arc.spaces[0].tabs.size(), 1u);
  CHECK_EQ(arc.spaces[0].tabs[0].folder->name, "Empty");
  CHECK_EQ(arc.spaces[0].tabs[0].url, "chrome://newtab/");
}

void NestedTreeLayout() {
  // A loose tab that used to precede folders moves below the Add Tab divider.
  const auto rows = mori::FolderTreeRows({{},
                                          {"root"},
                                          {"root", "child", "grandchild"},
                                          {"root", "sibling"},
                                          {"other"},
                                          {}});
  CHECK_EQ(rows.size(), 11u);
  CHECK_EQ(rows[0].folder, "root");
  CHECK_EQ(rows[0].depth, 0u);
  CHECK_EQ(rows[1].tab.value(), 1u);
  CHECK_EQ(rows[1].depth, 1u);
  CHECK_EQ(rows[2].folder, "child");
  CHECK_EQ(rows[2].depth, 1u);
  CHECK_EQ(rows[3].folder, "grandchild");
  CHECK_EQ(rows[4].tab.value(), 2u);
  CHECK_EQ(rows[4].depth, 3u);
  CHECK_EQ(rows[5].folder, "sibling");
  CHECK_EQ(rows[7].folder, "other");
  CHECK_EQ(rows[9].tab.value(), 0u);
  CHECK_EQ(rows[9].depth, 0u);
  CHECK_EQ(rows[10].tab.value(), 5u);
  const std::vector<std::string> path{"root", "child", "grandchild"};
  auto all_open = [](const std::string&) { return true; };
  auto root_closed = [](const std::string& id) { return id != "root"; };
  auto leaf_closed = [](const std::string& id) { return id != "grandchild"; };
  CHECK(mori::FolderPathVisible(path, true, all_open));
  CHECK(!mori::FolderPathVisible(path, false, root_closed));
  CHECK(!mori::FolderPathVisible(path, true, root_closed));
  CHECK(mori::FolderPathVisible(path, false, leaf_closed));
  CHECK(!mori::FolderPathVisible(path, true, leaf_closed));
  CHECK(!mori::FolderCanContain(path, "root"));
  CHECK(!mori::FolderCanContain(path, "grandchild"));
  CHECK(mori::FolderCanContain(path, "sibling"));
  CHECK(mori::FolderCanContain({}, "root"));
  CHECK(mori::FolderTreeRows({}).empty());
  // Malformed paths terminate and retain the tab exactly once.
  const auto cycle = mori::FolderTreeRows({{"a", "b", "a"}});
  CHECK_EQ(cycle.size(), 3u);
  CHECK_EQ(cycle.back().tab.value(), 0u);
}

void ParentWithoutDirectTabs() {
  const auto imported = mori::importer::ParseMozillaSession(Json(R"({
    "spaces":[{"uuid":"s"}],
    "folders":[{"id":"p","name":"Parent"},
               {"id":"c","name":"Child","parentId":"p"}],
    "tabs":[{"groupId":"c","entries":[{"url":"https://child.test/"}]}]
  })"));
  CHECK_EQ(imported.spaces[0].tabs.size(), 1u);
  const auto& folder = *imported.spaces[0].tabs[0].folder;
  CHECK_EQ(folder.name, "Child");
  CHECK_EQ(folder.ancestors[0].id, "p");
  CHECK_EQ(folder.ancestors[0].name, "Parent");
}

void FolderSessionRoundTrip() {
  const std::vector<mori::FolderSessionNode> original = {
      {"root", "仕事", "briefcase", false},
      {"child", "Research", "book", true},
      {"leaf", "Notes", "", false}};
  const auto restored =
      mori::DecodeFolderSession(mori::EncodeFolderSession(original));
  CHECK_EQ(restored.size(), 3u);
  for (size_t i = 0; i < original.size(); ++i) {
    CHECK_EQ(restored[i].id, original[i].id);
    CHECK_EQ(restored[i].name, original[i].name);
    CHECK_EQ(restored[i].icon, original[i].icon);
    CHECK_EQ(restored[i].open, original[i].open);
  }
  // Existing single-level folders still load without ancestor metadata.
  const auto legacy = mori::DecodeFolderSession(
      R"({"id":"old","name":"Reading","open":false})");
  CHECK_EQ(legacy.size(), 1u);
  CHECK_EQ(legacy[0].name, "Reading");
  CHECK(!legacy[0].open);
  CHECK(mori::DecodeFolderSession("invalid").empty());
  CHECK(mori::DecodeFolderSession(R"({"name":"missing id"})").empty());
  const auto cycle = mori::DecodeFolderSession(R"({"id":"leaf","ancestors":[
    {"id":"root","name":"Root"},{"id":"root"},{"id":"leaf"}]})");
  CHECK_EQ(cycle.size(), 2u);
  CHECK_EQ(cycle[0].id, "root");
  CHECK_EQ(cycle[1].id, "leaf");
}

void InvalidAndFirefox() {
  CHECK(!mori::importer::ParseArcSidebar(
             Json(R"({"sidebar":{"containers":[{"items":true}]}})"))
             .found);
  CHECK(mori::importer::ParseArcSidebar(Json("null")).spaces.empty());
  CHECK(mori::importer::ParseMozillaSession(Json("null")).spaces.empty());
  const auto imported =
      mori::importer::ParseMozillaSession(Json(R"({"windows":[{"tabs":[
    {"pinned":true,"entries":[{"url":"https://pin.test/"}]},
    {"hidden":true,"entries":[{"url":"https://hidden.test/"}]},
    {"entries":[{"url":"https://open.test/"}]}
  ]}]})"));
  CHECK_EQ(imported.spaces[0].tabs.size(), 2u);
  CHECK_EQ(imported.spaces[0].tabs[0].url, "https://pin.test/");
  CHECK(!imported.spaces[0].tabs[1].folder);
}
}  // namespace

int main() {
  ZenFolders();
  ZenMissingMetadataAndCycles();
  ArcFolders();
  EmptyFolders();
  NestedTreeLayout();
  ParentWithoutDirectTabs();
  FolderSessionRoundTrip();
  InvalidAndFirefox();
  std::cout << "Mori import regressions passed\n";
}
