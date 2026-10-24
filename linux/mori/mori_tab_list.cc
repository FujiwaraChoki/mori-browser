#include "chrome/browser/ui/views/mori/mori_tab_list.h"

#include <algorithm>
#include <vector>

#include "base/callback_list.h"
#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/scoped_observation.h"
#include "base/task/sequenced_task_runner.h"
#include "base/task/single_thread_task_runner.h"
#include "base/time/time.h"
#include "base/timer/timer.h"
#include "build/build_config.h"
#include "cc/paint/paint_flags.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/app/vector_icons/vector_icons.h"
#include "chrome/browser/tab_group_sync/tab_group_sync_service_factory.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/navigator/browser_navigator.h"
#include "chrome/browser/ui/navigator/browser_navigator_params.h"
#include "chrome/browser/ui/tabs/tab_enums.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"
#include "chrome/browser/ui/tabs/vertical_tab_strip_state_controller.h"
#include "chrome/browser/ui/views/mori/mori_assistant.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_folders.h"
#include "chrome/browser/ui/views/mori/mori_little.h"
#include "chrome/browser/ui/views/mori/mori_new_tab.h"
#include "chrome/browser/ui/views/mori/mori_sidebar.h"
#include "chrome/browser/ui/views/mori/mori_themes.h"
#include "chrome/common/webui_url_constants.h"
#include "components/saved_tab_groups/public/tab_group_sync_service.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/base/models/image_model.h"
#include "ui/color/color_id.h"
#include "ui/color/color_provider.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/geometry/insets.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/button/image_button.h"
#include "ui/views/controls/button/image_button_factory.h"
#include "ui/views/controls/image_view.h"
#include "ui/views/controls/label.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/view.h"
#include "ui/views/view_class_properties.h"
#include "url/gurl.h"

namespace mori {
namespace {

constexpr int kButtonSize = 28;
// The bar's inset from the sidebar's edges.
constexpr int kBarInset = 7;

// Mori's sidebar metrics.
constexpr int kRowHeight = 36;
constexpr int kRowRadius = 8;
constexpr int kRowContentInset = 7;

constexpr base::TimeDelta kArchiveCheckInterval = base::Minutes(10);

// Mori's muted sidebar text ("Add Tab").
SkColor MutedColor(const views::View* view) {
  const SkColor frame =
      view->GetColorProvider()->GetColor(ui::kColorFrameActive);
  return color_utils::IsDark(frame) ? SkColorSetRGB(0xA0, 0xA1, 0xA2)
                                    : SkColorSetRGB(0x5A, 0x5C, 0x5E);
}

// The hairline under the favorites (Mori), across the header.
class FavoritesSeparator : public views::View {
  METADATA_HEADER(FavoritesSeparator, views::View)

 public:
  FavoritesSeparator() { SetPreferredSize(gfx::Size(0, 1)); }

  void OnPaint(gfx::Canvas* canvas) override {
    const SkColor frame =
        GetColorProvider()->GetColor(ui::kColorFrameActive);
    canvas->FillRect(GetLocalBounds(),
                     color_utils::IsDark(frame)
                         ? SkColorSetA(SK_ColorWHITE, 0x17)
                         : SkColorSetA(SK_ColorBLACK, 0x14));
  }
};

BEGIN_METADATA(FavoritesSeparator)
END_METADATA

// "+ Add Tab": opens the command bar, as Ctrl+T does.
class AddTabRow : public views::Button {
  METADATA_HEADER(AddTabRow, views::Button)

 public:
  explicit AddTabRow(PressedCallback callback)
      : views::Button(std::move(callback)) {
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal,
        gfx::Insets::TLBR(0, kRowContentInset, 0, 0), 10));
    layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    icon_ = AddChildView(std::make_unique<views::ImageView>());
    icon_->SetPreferredSize(gfx::Size(16, 16));
    label_ = AddChildView(std::make_unique<views::Label>(u"Add Tab"));
    // It sits on the translucent sidebar, where subpixel text gets colour
    // fringes and looks bold.
    label_->SetSubpixelRenderingEnabled(false);
    SetPreferredSize(gfx::Size(0, kRowHeight));
    SetAccessibleName(u"Add Tab");
  }

  void OnThemeChanged() override {
    views::Button::OnThemeChanged();
    const SkColor muted = MutedColor(this);
    icon_->SetImage(ui::ImageModel::FromVectorIcon(kAddIcon, muted, 13));
    label_->SetEnabledColor(muted);
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    if (GetState() != STATE_HOVERED && GetState() != STATE_PRESSED) {
      return;
    }
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    flags.setColor(GetColorProvider()->GetColor(
        kColorTabBackgroundInactiveHoverFrameActive));
    canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), kRowRadius, flags);
  }

  void StateChanged(ButtonState old_state) override { SchedulePaint(); }

 private:
  raw_ptr<views::ImageView> icon_ = nullptr;
  raw_ptr<views::Label> label_ = nullptr;
};

BEGIN_METADATA(AddTabRow)
END_METADATA

class MoriTabListHeader : public views::View, public TabStripModelObserver {
  METADATA_HEADER(MoriTabListHeader, views::View)

 public:
  explicit MoriTabListHeader(BrowserWindowInterface* browser)
      : browser_(browser), model_(browser->GetTabStripModel()) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical));
    separator_ = AddChildView(std::make_unique<FavoritesSeparator>());
    add_tab_ = AddChildView(std::make_unique<AddTabRow>(base::BindRepeating(
        [](BrowserWindowInterface* browser) {
          // Opens the command bar (see the IDC_NEW_TAB hook).
          chrome::ExecuteCommand(browser, IDC_NEW_TAB);
        },
        browser)));
    model_->AddObserver(this);
    Update();
  }

  ~MoriTabListHeader() override {
    if (model_) {
      model_->RemoveObserver(this);
    }
  }

  // TabStripModelObserver:
  void OnTabStripModelDestroyed(TabStripModel* model) override { model_ = nullptr; }
  void OnTabStripModelChanged(TabStripModel* tab_strip_model,
                              const TabStripModelChange& change,
                              const TabStripSelectionChange& selection) override {
    if (change.type() != TabStripModelChange::kSelectionOnly) {
      Update();
    }
  }
  void OnTabPinnedStateChanged(tabs::TabInterface* tab, int index) override {
    Update();
  }

 private:
  void Update() {
    // Separate saved folders from Add Tab and the ordinary tabs below it.
    const bool has_saved_items = model_ &&
        (model_->IndexOfFirstNonPinnedTab() > 0 || !ListFolders(browser_).empty());
    separator_->SetVisible(has_saved_items);
    add_tab_->SetProperty(views::kMarginsKey,
                          gfx::Insets::TLBR(has_saved_items ? 8 : 4, 0, 0, 0));
    InvalidateLayout();
  }

  raw_ptr<BrowserWindowInterface> browser_;
  raw_ptr<TabStripModel> model_ = nullptr;
  raw_ptr<FavoritesSeparator> separator_ = nullptr;
  raw_ptr<views::View> add_tab_ = nullptr;
};

BEGIN_METADATA(MoriTabListHeader)
END_METADATA

class SidebarBar : public views::View,
                   public tab_groups::TabGroupSyncService::Observer {
  METADATA_HEADER(SidebarBar, views::View)

 public:
  explicit SidebarBar(BrowserWindowInterface* browser) : browser_(browser) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal,
        gfx::Insets::VH(6, kBarInset), 2))
        ->set_cross_axis_alignment(
            views::BoxLayout::CrossAxisAlignment::kCenter);
    // Mori: the library (history and bookmarks) at the start.
    auto library = views::CreateVectorImageButtonWithNativeTheme(
        base::BindRepeating(
            [](BrowserWindowInterface* browser) {
              chrome::ExecuteCommand(browser, IDC_SHOW_HISTORY);
            },
            browser_.get()),
        LucideIcon("library-big"), 16);
#if BUILDFLAG(IS_MAC)
    library->SetTooltipText(u"History & Bookmarks (⌘Y)");
#else
    library->SetTooltipText(u"History & Bookmarks (Ctrl+Y)");
#endif
    library->SetPreferredSize(gfx::Size(kButtonSize, kButtonSize));
    AddChildView(std::move(library));
    // Mori: the assistant beside the library.
    auto assistant = views::CreateVectorImageButtonWithNativeTheme(
        base::BindRepeating(&ToggleAssistant, browser_.get()),
        LucideIcon("sparkles"), 16);
#if BUILDFLAG(IS_MAC)
    assistant->SetTooltipText(u"Assistant (⇧⌘K)");
#else
    assistant->SetTooltipText(u"Assistant (Ctrl+Shift+K)");
#endif
    assistant->SetPreferredSize(gfx::Size(kButtonSize, kButtonSize));
    AddChildView(std::move(assistant));

    archive_timer_.Start(FROM_HERE, kArchiveCheckInterval, this,
                         &SidebarBar::ArchiveIdleTabs);
    if (auto* state = tabs::VerticalTabStripStateController::From(browser_)) {
      collapse_subscription_ = state->RegisterOnCollapseChanged(
          base::BindRepeating(&SidebarBar::OnCollapseChanged,
                              base::Unretained(this)));
      // Chromium's icon-only collapsed sidebar isn't Mori's; a profile or
      // session that saved it opens expanded.
      if (state->IsCollapsed()) {
        state->RequestCollapse(false);
      }
    }
    // A new theme washes over the chrome.
    theme_subscription_ = AddThemeObserver(
        browser_, base::BindRepeating(&RepaintThemeWash, browser_.get()));
    // Mori kept its spaces in tab groups; they come out of them once, after
    // the saved-tab-groups service has connected the restored groups to
    // their saved copies (it CHECKs that they're still there).
    if (auto* service = tab_groups::TabGroupSyncServiceFactory::GetForProfile(
            browser_->GetProfile())) {
      sync_observation_.Observe(service);
    } else {
      OnInitialized();
    }
    UpdateVisibility();
  }

  // tab_groups::TabGroupSyncService::Observer:
  void OnInitialized() override {
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(
            [](base::WeakPtr<SidebarBar> bar) {
              if (bar) {
                WhenMoriSettingsLoaded(
                    bar->browser_->GetProfile(),
                    base::BindOnce(&SidebarBar::UngroupSpaces,
                                   bar->weak_factory_.GetWeakPtr()));
              }
            },
            weak_factory_.GetWeakPtr()));
  }
  void OnWillBeDestroyed() override { sync_observation_.Reset(); }

  void AddedToWidget() override {
    views::View::AddedToWidget();
    // Mori's new tab page, in this window (once the window is set up:
    // the sidebar joins it while the BrowserView is still being built).
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(
            [](base::WeakPtr<BrowserWindowInterface> browser) {
              if (browser) {
                WatchNewTabPages(browser.get());
              }
            },
            browser_->GetWeakPtr()));
  }

 private:
  // Collapsing (Chromium's button or shortcut) hides the sidebar the Mori
  // way instead, as Ctrl+S does.
  void OnCollapseChanged(tabs::VerticalTabStripCollapseState state) {
    auto* controller = tabs::VerticalTabStripStateController::From(browser_);
    if (controller && controller->IsCollapsed()) {
      base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
          FROM_HERE,
          base::BindOnce(
              [](base::WeakPtr<BrowserWindowInterface> browser) {
                auto* controller =
                    browser ? tabs::VerticalTabStripStateController::From(
                                  browser.get())
                            : nullptr;
                if (!controller || !controller->IsCollapsed()) {
                  return;
                }
                controller->RequestCollapse(false);
                if (!IsSidebarHiddenIn(browser.get())) {
                  ToggleSidebarHidden(browser.get());
                }
              },
              browser_->GetWeakPtr()));
    }
    UpdateVisibility();
  }

  void UpdateVisibility() {
    auto* state = tabs::VerticalTabStripStateController::From(browser_);
    SetVisible(!state || !state->IsCollapsed());
  }

  // Tabs Mori kept in spaces (tab groups) join the one list of tabs. Only
  // once: groups made later (by an extension) stay.
  void UngroupSpaces() {
    Profile* profile = browser_->GetProfile();
    if (SpacesUngrouped(profile)) {
      return;
    }
    TabStripModel* model = browser_->GetTabStripModel();
    if (model->SupportsTabGroups()) {
      std::vector<int> grouped;
      for (int i = 0; i < model->count(); ++i) {
        if (model->GetTabAtIndex(i)->GetGroup().has_value()) {
          grouped.push_back(i);
        }
      }
      if (!grouped.empty()) {
        model->RemoveFromGroup(grouped);
      }
    }
    SetSpacesUngrouped(profile);
  }

  // Arc's auto-archive (Mori's default of 24 hours): tabs nobody looked
  // at for a day close. Favorites, folder tabs, sound and the active tab stay; closed
  // tabs stay in History and Reopen Closed Tab.
  void ArchiveIdleTabs() {
    TabStripModel* model = browser_->GetTabStripModel();
    const int hours = ArchiveAfterHours(browser_->GetProfile());
    if (hours <= 0) {
      return;
    }
    const base::Time cutoff = base::Time::Now() - base::Hours(hours);
    std::vector<tabs::TabHandle> idle;
    for (int i = 0; i < model->count(); ++i) {
      tabs::TabInterface* tab = model->GetTabAtIndex(i);
      content::WebContents* contents = tab->GetContents();
      if (i == model->active_index() || tab->IsPinned() || GetTabFolder(tab) || tab->IsSplit() ||
          !contents || contents->IsCurrentlyAudible() ||
          !contents->GetLastCommittedURL().SchemeIsHTTPOrHTTPS() ||
          contents->GetLastActiveTime() > cutoff) {
        continue;
      }
      idle.push_back(tab->GetHandle());
    }
    for (const tabs::TabHandle& handle : idle) {
      tabs::TabInterface* tab = handle.Get();
      const int index = tab ? model->GetIndexOfTab(tab) : TabStripModel::kNoTab;
      if (index != TabStripModel::kNoTab) {
        model->CloseWebContentsAt(index, TabCloseTypes::CLOSE_NONE);
      }
    }
  }

  raw_ptr<BrowserWindowInterface> browser_;
  base::RepeatingTimer archive_timer_;
  base::CallbackListSubscription theme_subscription_;
  base::CallbackListSubscription collapse_subscription_;
  base::ScopedObservation<tab_groups::TabGroupSyncService,
                          tab_groups::TabGroupSyncService::Observer>
      sync_observation_{this};
  base::WeakPtrFactory<SidebarBar> weak_factory_{this};
};

BEGIN_METADATA(SidebarBar)
END_METADATA

}  // namespace

int IndexForNewTab(BrowserWindowInterface* browser) {
  return IndexAfterFolders(browser);
}

void ArchiveUnpinnedTabs(BrowserWindowInterface* browser) {
  TabStripModel* model = browser->GetTabStripModel();
  std::vector<tabs::TabHandle> old_tabs;
  for (int i = model->IndexOfFirstNonPinnedTab(); i < model->count(); ++i) {
    if (!GetTabFolder(model->GetTabAtIndex(i))) {
      old_tabs.push_back(model->GetTabAtIndex(i)->GetHandle());
    }
  }
  if (old_tabs.empty()) {
    return;
  }
  // A new tab stays on screen, as in Arc.
  NavigateParams params(browser, GURL(chrome::kChromeUINewTabURL),
                        ui::PAGE_TRANSITION_AUTO_TOPLEVEL);
  params.disposition = WindowOpenDisposition::NEW_FOREGROUND_TAB;
  params.tabstrip_index = model->IndexOfFirstNonPinnedTab();
  Navigate(&params);
  for (const tabs::TabHandle& handle : old_tabs) {
    tabs::TabInterface* tab = handle.Get();
    const int index = tab ? model->GetIndexOfTab(tab) : TabStripModel::kNoTab;
    if (index != TabStripModel::kNoTab) {
      model->CloseWebContentsAt(index, TabCloseTypes::CLOSE_USER_GESTURE);
    }
  }
}

std::unique_ptr<views::View> CreateTabListHeader(
    BrowserWindowInterface* browser) {
  return std::make_unique<MoriTabListHeader>(browser);
}

std::unique_ptr<views::View> CreateSidebarBar(BrowserWindowInterface* browser) {
  return std::make_unique<SidebarBar>(browser);
}

}  // namespace mori
