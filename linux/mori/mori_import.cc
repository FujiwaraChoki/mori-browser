#include "chrome/browser/ui/views/mori/mori_import.h"

#include <map>
#include <optional>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "base/base_paths.h"
#include "base/containers/span.h"
#include "base/files/file_enumerator.h"
#include "base/files/file_path.h"
#include "base/files/file_util.h"
#include "base/functional/bind.h"
#include "base/json/json_reader.h"
#include "base/json/json_writer.h"
#include "base/path_service.h"
#include "base/strings/string_number_conversions.h"
#include "base/strings/string_split.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/thread_pool.h"
#include "base/time/time.h"
#include "base/uuid.h"
#include "base/values.h"
#include "build/build_config.h"
#include "chrome/browser/ui/browser_tabrestore.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/mori/mori_favicons.h"
#include "chrome/browser/ui/views/mori/mori_folder_session.h"
#include "chrome/browser/ui/views/mori/mori_folders.h"
#include "chrome/browser/ui/views/mori/mori_import_parser.h"
#include "chrome/browser/ui/views/mori/mori_toast.h"
#include "components/sessions/core/serialized_navigation_entry.h"
#include "components/sessions/core/serialized_user_agent_override.h"
#include "url/gurl.h"

namespace mori {
namespace {

using importer::Imported;
using importer::ImportedSpace;
using importer::ImportedTab;
using importer::ParseMozillaSession;

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
               ? base::FilePath::FromUTF8Unsafe(path->second)
               : root.Append(base::FilePath::FromUTF8Unsafe(path->second));
  };
  for (const auto& [name, values] : sections) {
    if (name.starts_with("Install")) {
      if (auto it = values.find("Default"); it != values.end()) {
        return root.Append(base::FilePath::FromUTF8Unsafe(it->second));
      }
    }
  }
  std::optional<base::FilePath> first;
  for (const auto& [name, values] : sections) {
    if (!name.starts_with("Profile")) {
      continue;
    }
    if (auto it = values.find("Default");
        it != values.end() && it->second == "1") {
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
    if (!text.empty() ||
        base::ReadFileToString(home.AppendASCII(where), &text)) {
      break;
    }
  }
  std::optional<base::Value> json =
      text.empty() ? std::nullopt
                   : base::JSONReader::Read(text, base::JSON_PARSE_RFC);
  return json ? importer::ParseArcSidebar(*json) : Imported();
}

// ---- Into the window ----------------------------------------------------

// An unloaded tab, as session restore makes them: it loads when shown.
void AddUnloadedTab(BrowserWindowInterface* browser,
                    const ImportedTab& tab,
                    int index,
                    std::optional<tab_groups::TabGroupId> group,
                    bool pin,
                    const std::map<std::string, std::string>& extra_data = {}) {
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
      sessions::SerializedUserAgentOverride(), extra_data,
      /*from_session_restore=*/false, std::nullopt);
  if (contents) {
    FetchIconForUnloadedTab(contents);
    if (!extra_data.empty()) {
      PersistTabFolder(contents);
    }
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
                           : std::u16string(u"No ") + name + u" profile found");
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
    std::map<std::string, std::string> folder_ids;
    for (const ImportedTab& tab : space.tabs) {
      std::map<std::string, std::string> extra_data;
      if (tab.folder) {
        std::vector<FolderSessionNode> path;
        auto append = [&](const importer::ImportedFolder& source) {
          auto [it, fresh] = folder_ids.try_emplace(source.id);
          if (fresh) {
            it->second = base::Uuid::GenerateRandomV4().AsLowercaseString();
          }
          path.push_back({it->second, source.name, {}, source.open});
        };
        for (const auto& ancestor : tab.folder->ancestors) {
          append(ancestor);
        }
        append(*tab.folder);
        extra_data[kTabFolderKey] = EncodeFolderSession(path);
      }
      AddUnloadedTab(browser, tab, model->count(), std::nullopt,
                     /*pin=*/false, extra_data);
      ++added;
    }
  }
  ShowToast(browser, added == 0
                         ? std::u16string(u"Nothing to import from ") + name
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
