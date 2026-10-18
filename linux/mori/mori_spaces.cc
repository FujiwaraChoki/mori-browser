#include "chrome/browser/ui/views/mori/mori_spaces.h"

#include <algorithm>
#include <string>
#include <utility>
#include <vector>

#include "base/feature_list.h"
#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/scoped_observation.h"
#include "base/strings/string_number_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "cc/paint/paint_flags.h"
#include "chrome/app/vector_icons/vector_icons.h"
#include "chrome/browser/tab_group_sync/tab_group_sync_service_factory.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/navigator/browser_navigator.h"
#include "chrome/browser/ui/navigator/browser_navigator_params.h"
#include "chrome/browser/ui/tabs/tab_group_model.h"
#include "chrome/browser/ui/tabs/tab_group_theme.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model_delegate.h"
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"
#include "chrome/browser/ui/tabs/vertical_tab_strip_state_controller.h"
#include "chrome/browser/ui/ui_features.h"
#include "chrome/common/webui_url_constants.h"
#include "components/saved_tab_groups/public/tab_group_sync_service.h"
#include "components/tab_groups/tab_group_color.h"
#include "components/tab_groups/tab_group_visual_data.h"
#include "components/tabs/public/tab_group.h"
#include "components/tabs/public/tab_interface.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/color/color_provider.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/geometry/insets.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/button/image_button.h"
#include "ui/views/controls/button/image_button_factory.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/view.h"
#include "url/gurl.h"

namespace mori {
namespace {

constexpr int kButtonSize = 28;
constexpr int kDotRadius = 5;
constexpr float kButtonCornerRadius = 8.0f;

// Colours handed to new spaces in turn.
constexpr tab_groups::TabGroupColorId kSpaceColors[] = {
    tab_groups::TabGroupColorId::kBlue,   tab_groups::TabGroupColorId::kPurple,
    tab_groups::TabGroupColorId::kPink,   tab_groups::TabGroupColorId::kGreen,
    tab_groups::TabGroupColorId::kOrange, tab_groups::TabGroupColorId::kCyan,
    tab_groups::TabGroupColorId::kYellow, tab_groups::TabGroupColorId::kRed,
    tab_groups::TabGroupColorId::kGrey,
};

TabStripModel* SpacesModel(BrowserWindowInterface* browser) {
  if (!browser ||
      !base::FeatureList::IsEnabled(features::kTabGroupsFocusing)) {
    return nullptr;
  }
  TabStripModel* model = browser->GetTabStripModel();
  return model && model->SupportsTabGroups() ? model : nullptr;
}

// The first space in a window without one.
constexpr char16_t kFirstSpaceTitle[] = u"Personal";

// Spaces in the order their tabs appear in the strip.
std::vector<tab_groups::TabGroupId> SpacesInOrder(TabStripModel* model) {
  std::vector<tab_groups::TabGroupId> spaces =
      model->group_model()->ListTabGroups();
  std::ranges::sort(spaces, {}, [model](const tab_groups::TabGroupId& id) {
    return model->group_model()->GetTabGroup(id)->ListTabs().start();
  });
  return spaces;
}

void NameSpace(TabStripModel* model,
               tab_groups::TabGroupId id,
               std::u16string title,
               tab_groups::TabGroupColorId color) {
  model->ChangeTabGroupVisuals(id,
                               tab_groups::TabGroupVisualData(title, color));
}

// One space: a dot in the space's colour, on a highlight when current.
class SpaceButton : public views::Button {
  METADATA_HEADER(SpaceButton, views::Button)

 public:
  SpaceButton(PressedCallback callback,
              tab_groups::TabGroupColorId color,
              const std::u16string& title,
              bool current)
      : views::Button(std::move(callback)), color_(color), current_(current) {
    SetTooltipText(title);
    GetViewAccessibility().SetName(title);
    SetPreferredSize(gfx::Size(kButtonSize, kButtonSize));
  }

  void PaintButtonContents(gfx::Canvas* canvas) override {
    const ui::ColorProvider* colors = GetColorProvider();
    const gfx::RectF bounds(GetLocalBounds());
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    if (current_ || GetState() == STATE_HOVERED) {
      flags.setColor(colors->GetColor(
          current_ ? kColorTabBackgroundActiveFrameActive
                   : kColorTabBackgroundInactiveHoverFrameActive));
      canvas->DrawRoundRect(bounds, kButtonCornerRadius, flags);
    }
    flags.setColor(
        colors->GetColor(GetTabGroupTabStripColorId(color_, true)));
    canvas->DrawCircle(bounds.CenterPoint(), current_ ? kDotRadius + 1
                                                      : kDotRadius,
                       flags);
  }

 private:
  tab_groups::TabGroupColorId color_;
  bool current_;
};

BEGIN_METADATA(SpaceButton)
END_METADATA

class SpaceBar : public views::View,
                 public TabStripModelObserver,
                 public tab_groups::TabGroupSyncService::Observer {
  METADATA_HEADER(SpaceBar, views::View)

 public:
  explicit SpaceBar(BrowserWindowInterface* browser) : browser_(browser) {
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal, gfx::Insets::VH(6, 10),
        2));
    layout->set_main_axis_alignment(
        views::BoxLayout::MainAxisAlignment::kCenter);
    layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    if (TabStripModel* model = SpacesModel(browser_)) {
      model->AddObserver(this);
    }
    // Groups made before the saved-tab-groups service is ready get its
    // stored copy (default name and colour) applied over them, so spaces are
    // only created and named once it is.
    if (auto* service = tab_groups::TabGroupSyncServiceFactory::GetForProfile(
            browser_->GetProfile())) {
      sync_observation_.Observe(service);
    } else {
      groups_ready_ = true;
    }
    if (auto* state = tabs::VerticalTabStripStateController::From(browser_)) {
      collapse_subscription_ = state->RegisterOnCollapseChanged(
          base::BindRepeating(&SpaceBar::OnCollapseChanged,
                              base::Unretained(this)));
    }
    ScheduleUpdate();
  }

  // tab_groups::TabGroupSyncService::Observer:
  void OnInitialized() override {
    groups_ready_ = true;
    ScheduleUpdate();
  }
  void OnWillBeDestroyed() override { sync_observation_.Reset(); }

  // TabStripModelObserver:
  void OnTabGroupChanged(const TabGroupChange& change) override {
    ScheduleUpdate();
  }
  void OnTabGroupFocusChanged(
      std::optional<tab_groups::TabGroupId> new_focused_group,
      std::optional<tab_groups::TabGroupId> old_focused_group) override {
    ScheduleUpdate();
  }
  void OnTabStripModelChanged(TabStripModel* tab_strip_model,
                              const TabStripModelChange& change,
                              const TabStripSelectionChange& selection) override {
    if (change.type() != TabStripModelChange::kSelectionOnly) {
      ScheduleUpdate();
    }
  }

 private:
  void OnCollapseChanged(tabs::VerticalTabStripCollapseState state) {
    ScheduleUpdate();
  }

  // Runs after the current change: changing the model from inside its own
  // notifications re-enters it, and rebuilding inside a button's click would
  // delete the button under it.
  void ScheduleUpdate() {
    if (update_pending_) {
      return;
    }
    update_pending_ = true;
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&SpaceBar::Update, weak_factory_.GetWeakPtr()));
  }

  void Update() {
    update_pending_ = false;
    if (groups_ready_) {
      KeepEveryTabInASpace();
    }
    Rebuild();
  }

  // Arc's rule: every tab that isn't a favorite (pinned) is in a space, and
  // one space is always on screen. Loose tabs join the space on screen, or
  // become the first space if there is none.
  void KeepEveryTabInASpace() {
    TabStripModel* model = SpacesModel(browser_);
    if (!model || model->empty() || model->closing_all() ||
        !model->delegate()->IsTabStripEditable()) {
      return;
    }
    std::vector<int> loose;
    for (int i = 0; i < model->count(); ++i) {
      tabs::TabInterface* tab = model->GetTabAtIndex(i);
      if (!tab->IsPinned() && !tab->GetGroup().has_value()) {
        loose.push_back(i);
      }
    }
    const std::vector<tab_groups::TabGroupId> spaces = SpacesInOrder(model);
    if (spaces.empty()) {
      if (loose.empty()) {
        return;
      }
      const tab_groups::TabGroupId id = model->AddToNewGroup(loose);
      NameSpace(model, id, kFirstSpaceTitle, kSpaceColors[0]);
      model->SetFocusedGroup(id);
      return;
    }
    std::optional<tab_groups::TabGroupId> target = model->GetFocusedGroup();
    if (!target.has_value()) {
      tabs::TabInterface* active = model->GetActiveTab();
      target = active && active->GetGroup().has_value() ? active->GetGroup()
                                                        : spaces.front();
    }
    if (!loose.empty()) {
      model->AddToExistingGroup(loose, *target);
    }
    if (model->GetFocusedGroup() != target) {
      model->SetFocusedGroup(target);
    }
  }

  void Rebuild() {
    RemoveAllChildViews();
    TabStripModel* model = SpacesModel(browser_);
    auto* state = tabs::VerticalTabStripStateController::From(browser_);
    if (!model || (state && state->IsCollapsed())) {
      SetVisible(false);
      return;
    }
    const std::optional<tab_groups::TabGroupId> current =
        model->GetFocusedGroup();
    int number = 0;
    for (const tab_groups::TabGroupId& id : SpacesInOrder(model)) {
      ++number;
      const tab_groups::TabGroupVisualData* visuals =
          model->group_model()->GetTabGroup(id)->visual_data();
      std::u16string title = visuals->title();
      if (title.empty()) {
        title = u"Space " + base::NumberToString16(number);
      }
      AddChildView(std::make_unique<SpaceButton>(
          base::BindRepeating(&SpaceBar::ShowSpace, base::Unretained(this),
                              id),
          visuals->color(), title, current == id));
    }
    auto add = views::CreateVectorImageButtonWithNativeTheme(
        base::BindRepeating(&mori::NewSpace, browser_.get()),
        kAddIcon, 16);
    add->SetTooltipText(u"New space");
    add->SetPreferredSize(gfx::Size(kButtonSize, kButtonSize));
    AddChildView(std::move(add));
    SetVisible(true);
    PreferredSizeChanged();
  }

  void ShowSpace(tab_groups::TabGroupId id) {
    TabStripModel* model = SpacesModel(browser_);
    if (model && model->group_model()->ContainsTabGroup(id)) {
      model->SetFocusedGroup(id);
    }
  }

  raw_ptr<BrowserWindowInterface> browser_;
  bool update_pending_ = false;
  bool groups_ready_ = false;
  base::ScopedObservation<tab_groups::TabGroupSyncService,
                          tab_groups::TabGroupSyncService::Observer>
      sync_observation_{this};
  base::CallbackListSubscription collapse_subscription_;
  base::WeakPtrFactory<SpaceBar> weak_factory_{this};
};

BEGIN_METADATA(SpaceBar)
END_METADATA

}  // namespace

bool HasSpaces(BrowserWindowInterface* browser) {
  TabStripModel* model = SpacesModel(browser);
  return model && !model->group_model()->ListTabGroups().empty();
}

std::optional<tab_groups::TabGroupId> GetCurrentSpace(
    BrowserWindowInterface* browser) {
  TabStripModel* model = SpacesModel(browser);
  return model ? model->GetFocusedGroup() : std::nullopt;
}

void ShowSpaceOfActiveTab(BrowserWindowInterface* browser) {
  TabStripModel* model = SpacesModel(browser);
  if (!model) {
    return;
  }
  tabs::TabInterface* tab = model->GetActiveTab();
  if (tab && tab->GetGroup().has_value()) {
    model->SetFocusedGroup(tab->GetGroup());
  }
}

void NewSpace(BrowserWindowInterface* browser) {
  TabStripModel* model = SpacesModel(browser);
  if (!model) {
    return;
  }
  const size_t count = model->group_model()->ListTabGroups().size();
  NavigateParams params(browser, GURL(chrome::kChromeUINewTabURL),
                        ui::PAGE_TRANSITION_AUTO_TOPLEVEL);
  params.disposition = WindowOpenDisposition::NEW_FOREGROUND_TAB;
  Navigate(&params);
  const int index =
      model->GetIndexOfWebContents(params.navigated_or_inserted_contents);
  if (index == TabStripModel::kNoTab) {
    return;
  }
  const tab_groups::TabGroupId id = model->AddToNewGroup({index});
  NameSpace(model, id, u"Space " + base::NumberToString16(count + 1),
            kSpaceColors[count % std::size(kSpaceColors)]);
  model->SetFocusedGroup(id);
}

void SwitchSpace(BrowserWindowInterface* browser, bool forward) {
  TabStripModel* model = SpacesModel(browser);
  if (!model) {
    return;
  }
  // Wraps around; unlike Chromium's rotation there is no "all tabs" stop.
  const std::vector<tab_groups::TabGroupId> spaces = SpacesInOrder(model);
  if (spaces.empty()) {
    return;
  }
  auto it = std::ranges::find(spaces, model->GetFocusedGroup());
  size_t index = it == spaces.end() ? 0 : it - spaces.begin();
  if (it != spaces.end()) {
    index = (index + (forward ? 1 : spaces.size() - 1)) % spaces.size();
  }
  model->SetFocusedGroup(spaces[index]);
}

std::unique_ptr<views::View> CreateSpaceBar(BrowserWindowInterface* browser) {
  return std::make_unique<SpaceBar>(browser);
}

}  // namespace mori
