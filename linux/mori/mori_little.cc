#include "chrome/browser/ui/views/mori/mori_little.h"

#include <algorithm>

#include <map>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "base/auto_reset.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/values.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/browser_window/public/create_browser_window.h"
#include "chrome/browser/ui/browser_window/public/profile_browser_collection.h"
#include "chrome/browser/ui/navigator/browser_navigator.h"
#include "chrome/browser/ui/navigator/browser_navigator_params.h"
#include "chrome/browser/ui/tabs/tab_enums.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_json_file.h"
#include "chrome/browser/ui/views/mori/mori_tab_list.h"
#include "chrome/browser/ui/views/toolbar/toolbar_button.h"
#include "chrome/browser/ui/views/toolbar/toolbar_view.h"
#include "components/tabs/public/tab_interface.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/base/window_open_disposition.h"
#include "ui/display/screen.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/controls/button/md_text_button.h"
#include "ui/views/layout/flex_layout_types.h"
#include "ui/views/view_class_properties.h"
#include "ui/views/widget/widget.h"
#include "url/gurl.h"

namespace mori {
namespace {

// Mori 3's Little Mori size (windows.rs).
constexpr int kWidth = 900;
constexpr int kHeight = 640;

// Set while OpenLittle builds its window, so the toolbar knows.
bool g_building_little = false;

// Mori's settings that Chromium has no place for, in "Mori Settings.json".
class Settings {
 public:
  static Settings* For(Profile* profile) {
    static base::NoDestructor<
        std::map<base::FilePath, std::unique_ptr<Settings>>>
        all;
    const base::FilePath path = profile->GetOriginalProfile()->GetPath();
    auto& settings = (*all)[path];
    if (!settings) {
      settings = std::make_unique<Settings>(
          path.AppendASCII("Mori Settings.json"));
    }
    return settings.get();
  }

  explicit Settings(base::FilePath path)
      : file_(std::move(path),
              base::BindOnce(&Settings::OnLoaded, base::Unretained(this))) {}

  bool links_in_little() const { return links_in_little_; }
  void set_links_in_little(bool little) {
    links_in_little_ = little;
    Save();
  }

  int archive_after_hours() const { return archive_after_hours_; }
  void set_archive_after_hours(int hours) {
    archive_after_hours_ = hours;
    Save();
  }

  bool spaces_ungrouped() const { return spaces_ungrouped_; }
  void set_spaces_ungrouped() {
    spaces_ungrouped_ = true;
    Save();
  }

  void WhenLoaded(base::OnceClosure closure) {
    if (loaded_) {
      std::move(closure).Run();
    } else {
      when_loaded_.push_back(std::move(closure));
    }
  }

 private:
  void OnLoaded(std::optional<base::Value> value) {
    if (value && value->is_dict()) {
      spaces_ungrouped_ =
          value->GetDict().FindBool("spaces_ungrouped").value_or(false);
      if (const std::string* links =
              value->GetDict().FindString("external_links")) {
        links_in_little_ = *links != "current_space";
      }
      if (std::optional<int> hours =
              value->GetDict().FindInt("archive_after_hours")) {
        archive_after_hours_ = std::max(0, *hours);
      }
    }
    loaded_ = true;
    std::vector<base::OnceClosure> closures;
    closures.swap(when_loaded_);
    for (base::OnceClosure& closure : closures) {
      std::move(closure).Run();
    }
  }

  void Save() {
    file_.Write(base::Value(
        base::DictValue()
            .Set("external_links",
                 links_in_little_ ? "little_window" : "current_space")
            .Set("archive_after_hours", archive_after_hours_)
            .Set("spaces_ungrouped", spaces_ungrouped_)));
  }

  // Mori 3's defaults.
  bool links_in_little_ = true;
  int archive_after_hours_ = 24;
  // Mori's spaces were tab groups; their tabs came out of them.
  bool spaces_ungrouped_ = false;
  bool loaded_ = false;
  std::vector<base::OnceClosure> when_loaded_;
  // Last: its load callback uses the members above.
  JsonFile file_;
};

// The page of the Little Mori window `little` moves to the top of the tabs
// of the last normal window (a new one if there is none).
void OpenInMori(BrowserWindowInterface* little) {
  TabStripModel* from = little->GetTabStripModel();
  const int index = from->active_index();
  if (index == TabStripModel::kNoTab) {
    return;
  }
  BrowserWindowInterface* target =
      ProfileBrowserCollection::GetForProfile(little->GetProfile())
          ->FindTabbedBrowser();
  if (!target) {
    target = CreateBrowserWindow(
        BrowserWindowCreateParams(little->GetProfile(), true));
  }
  TabStripModel* to = target->GetTabStripModel();
  // Detaching the only tab closes the little window.
  std::unique_ptr<tabs::TabModel> tab = from->DetachTabAtForInsertion(index);
  to->InsertDetachedTabAt(IndexForNewTab(target), std::move(tab),
                          AddTabTypes::ADD_ACTIVE);
  target->GetWindow()->Show();
  target->GetWindow()->Activate();
}

}  // namespace

int ArchiveAfterHours(Profile* profile) {
  return Settings::For(profile)->archive_after_hours();
}

void SetArchiveAfterHours(Profile* profile, int hours) {
  Settings::For(profile)->set_archive_after_hours(hours);
}

void WhenMoriSettingsLoaded(Profile* profile, base::OnceClosure closure) {
  Settings::For(profile)->WhenLoaded(std::move(closure));
}

bool SpacesUngrouped(Profile* profile) {
  return Settings::For(profile)->spaces_ungrouped();
}

void SetSpacesUngrouped(Profile* profile) {
  Settings::For(profile)->set_spaces_ungrouped();
}

bool LinksOpenInLittle(Profile* profile) {
  return Settings::For(profile)->links_in_little();
}

void SetLinksOpenInLittle(Profile* profile, bool little) {
  Settings::For(profile)->set_links_in_little(little);
}

bool ShouldOpenInLittle(BrowserWindowInterface* browser, const GURL& url) {
  return browser && url.SchemeIsHTTPOrHTTPS() &&
         !browser->GetProfile()->IsOffTheRecord() &&
         LinksOpenInLittle(browser->GetProfile());
}

void OpenLittle(Profile* profile, const GURL& url) {
  base::AutoReset<bool> building(&g_building_little, true);
  BrowserWindowCreateParams params(BrowserWindowInterface::TYPE_POPUP, *profile,
                                   /*from_user_gesture=*/true);
  // Centred on the screen.
  const gfx::Rect work_area =
      display::Screen::Get()->GetPrimaryDisplay().work_area();
  params.initial_bounds = gfx::Rect(
      work_area.x() + (work_area.width() - kWidth) / 2,
      work_area.y() + (work_area.height() - kHeight) / 2, kWidth, kHeight);
  params.initial_show_state = ui::mojom::WindowShowState::kNormal;
  BrowserWindowInterface* little = CreateBrowserWindow(std::move(params));
  NavigateParams nav(little, url, ui::PAGE_TRANSITION_AUTO_TOPLEVEL);
  nav.disposition = WindowOpenDisposition::NEW_FOREGROUND_TAB;
  Navigate(&nav);
  little->GetWindow()->Show();
}

void MaybeAddLittleHeader(ToolbarView* toolbar, BrowserView* browser_view) {
  if (!g_building_little) {
    return;
  }
  BrowserWindowInterface* browser = browser_view->browser();
  // Back, before the address.
  auto back = std::make_unique<ToolbarButton>(base::BindRepeating(
      [](BrowserWindowInterface* b) { chrome::ExecuteCommand(b, IDC_BACK); },
      browser));
  back->SetVectorIcon(LucideIcon("arrow-left"));
  back->GetViewAccessibility().SetName(u"Back");
  back->SetProperty(views::kFlexBehaviorKey, views::FlexSpecification());
  toolbar->AddChildViewAt(std::move(back), 0);
  // "Open in Mori" and close, after it.
  auto open = std::make_unique<views::MdTextButton>(
      base::BindRepeating(&OpenInMori, browser), u"Open in Mori");
  open->SetStyle(ui::ButtonStyle::kTonal);
  open->SetProperty(views::kMarginsKey, gfx::Insets::VH(0, 6));
  open->SetProperty(views::kFlexBehaviorKey, views::FlexSpecification());
  toolbar->AddChildView(std::move(open));
  auto close = std::make_unique<ToolbarButton>(base::BindRepeating(
      [](BrowserWindowInterface* b) {
        chrome::ExecuteCommand(b, IDC_CLOSE_WINDOW);
      },
      browser));
  close->SetVectorIcon(LucideIcon("x"));
  close->GetViewAccessibility().SetName(u"Close");
  close->SetProperty(views::kFlexBehaviorKey, views::FlexSpecification());
  toolbar->AddChildView(std::move(close));
}

}  // namespace mori
