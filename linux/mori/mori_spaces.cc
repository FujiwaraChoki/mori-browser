#include "chrome/browser/ui/views/mori/mori_spaces.h"

#include "chrome/browser/ui/views/mori/mori_icons.h"

#include <algorithm>
#include <map>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "base/feature_list.h"
#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/no_destructor.h"
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
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/ui/browser_commands.h"
#include "ui/views/controls/image_view.h"
#include "ui/views/controls/label.h"
#include "ui/views/border.h"
#include "ui/views/view_class_properties.h"
#include "ui/gfx/color_utils.h"
#include "ui/color/color_id.h"
#include "ui/base/models/image_model.h"
#include "ui/events/event.h"
#include "ui/gfx/font_list.h"
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

// Spaces being deleted on purpose (Delete Space), which must not come back.
std::set<tab_groups::TabGroupId>& SpacesBeingDeleted() {
  static base::NoDestructor<std::set<tab_groups::TabGroupId>> deleting;
  return *deleting;
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

// Mori 3's sidebar metrics (crates/mori/src/theme.rs, ui/sidebar.rs).
constexpr int kRowHeight = 36;
constexpr int kRowRadius = 8;
constexpr int kRowContentInset = 7;
constexpr int kLabelHeight = 24;
constexpr int kDividerHeight = 20;

// Mori 3's muted sidebar text (the space label, "New Tab").
SkColor MutedColor(const views::View* view) {
  const SkColor frame = view->GetColorProvider()->GetColor(ui::kColorFrameActive);
  return color_utils::IsDark(frame) ? SkColorSetRGB(0xA0, 0xA1, 0xA2)
                                    : SkColorSetRGB(0x5A, 0x5C, 0x5E);
}

// The space's name: 13px bold in the muted sidebar colour.
class SpaceLabel : public views::Label {
  METADATA_HEADER(SpaceLabel, views::Label)

 public:
  explicit SpaceLabel(base::RepeatingClosure edit) : edit_(std::move(edit)) {
    SetHorizontalAlignment(gfx::ALIGN_LEFT);
    SetElideBehavior(gfx::ELIDE_TAIL);
    SetFontList(views::Label::GetDefaultFontList()
                    .DeriveWithSizeDelta(13 - views::Label::GetDefaultFontList()
                                                  .GetFontSize())
                    .DeriveWithWeight(gfx::Font::Weight::BOLD));
    SetBorder(views::CreateEmptyBorder(gfx::Insets::TLBR(0, kRowContentInset,
                                                         0, 0)));
    SetPreferredSize(gfx::Size(0, kLabelHeight));
  }

  void OnThemeChanged() override {
    views::Label::OnThemeChanged();
    SetEnabledColor(MutedColor(this));
  }

  // Double-click or right-click renames or recolours the space.
  bool OnMousePressed(const ui::MouseEvent& event) override {
    if (event.IsRightMouseButton() ||
        (event.IsLeftMouseButton() && event.GetClickCount() == 2)) {
      edit_.Run();
      return true;
    }
    return false;
  }

 private:
  base::RepeatingClosure edit_;
};

BEGIN_METADATA(SpaceLabel)
END_METADATA

// A 1px line in white (or black) at low opacity, as Mori 3's divider.
class SpaceDivider : public views::View {
  METADATA_HEADER(SpaceDivider, views::View)

 public:
  SpaceDivider() { SetPreferredSize(gfx::Size(0, kDividerHeight)); }

  void OnPaint(gfx::Canvas* canvas) override {
    const ui::ColorProvider* colors = GetColorProvider();
    const SkColor frame = colors->GetColor(ui::kColorFrameActive);
    const bool dark = color_utils::IsDark(frame);
    const SkColor line = color_utils::AlphaBlend(
        dark ? SK_ColorWHITE : SK_ColorBLACK, frame, dark ? 0.09f : 0.10f);
    canvas->FillRect(gfx::Rect(4, height() / 2, width() - 4, 1), line);
  }
};

BEGIN_METADATA(SpaceDivider)
END_METADATA

// "+ New Tab": opens the command bar, as Ctrl+T does.
class NewTabRow : public views::Button {
  METADATA_HEADER(NewTabRow, views::Button)

 public:
  explicit NewTabRow(PressedCallback callback)
      : views::Button(std::move(callback)) {
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal,
        gfx::Insets::TLBR(0, kRowContentInset, 0, 0), 10));
    layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    icon_ = AddChildView(std::make_unique<views::ImageView>());
    icon_->SetPreferredSize(gfx::Size(16, 16));
    label_ = AddChildView(std::make_unique<views::Label>(u"New Tab"));
    SetPreferredSize(gfx::Size(0, kRowHeight));
    SetAccessibleName(u"New Tab");
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

BEGIN_METADATA(NewTabRow)
END_METADATA

class MoriSpaceHeader : public views::View, public TabStripModelObserver {
  METADATA_HEADER(MoriSpaceHeader, views::View)

 public:
  explicit MoriSpaceHeader(BrowserWindowInterface* browser)
      : browser_(browser) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical));
    label_ = AddChildView(std::make_unique<SpaceLabel>(base::BindRepeating(
        &MoriSpaceHeader::EditSpace, base::Unretained(this))));
    divider_ = AddChildView(std::make_unique<SpaceDivider>());
    AddChildView(std::make_unique<NewTabRow>(base::BindRepeating(
        [](BrowserWindowInterface* browser) {
          // Opens the command bar (see the IDC_NEW_TAB hook).
          chrome::ExecuteCommand(browser, IDC_NEW_TAB);
        },
        browser)));
    if (TabStripModel* model = SpacesModel(browser_)) {
      model->AddObserver(this);
    }
    Update();
  }

  // TabStripModelObserver:
  void OnTabGroupChanged(const TabGroupChange& change) override { Update(); }
  void OnTabGroupFocusChanged(
      std::optional<tab_groups::TabGroupId> new_focused_group,
      std::optional<tab_groups::TabGroupId> old_focused_group) override {
    Update();
  }
  void OnTabStripModelChanged(TabStripModel* tab_strip_model,
                              const TabStripModelChange& change,
                              const TabStripSelectionChange& selection) override {
    if (change.type() != TabStripModelChange::kSelectionOnly) {
      Update();
    }
  }

 private:
  void Update() {
    TabStripModel* model = SpacesModel(browser_);
    const std::optional<tab_groups::TabGroupId> space =
        model ? model->GetFocusedGroup() : std::nullopt;
    if (!space.has_value() ||
        !model->group_model()->ContainsTabGroup(*space)) {
      SetVisible(false);
      return;
    }
    label_->SetText(
        model->group_model()->GetTabGroup(*space)->visual_data()->title());
    // Mori 3 puts the label's top 11px under the favorites (the tab strip
    // already adds 8px there) or 5px under the address field.
    const bool has_favorites = model->IndexOfFirstNonPinnedTab() > 0;
    label_->SetProperty(views::kMarginsKey,
                        gfx::Insets::TLBR(has_favorites ? 3 : 5, 0, 0, 0));
    divider_->SetProperty(views::kMarginsKey, gfx::Insets::TLBR(9, 0, 3, 0));
    SetVisible(true);
    InvalidateLayout();
  }

  void EditSpace() {
    if (TabStripModel* model = SpacesModel(browser_)) {
      if (std::optional<tab_groups::TabGroupId> space =
              model->GetFocusedGroup()) {
        model->OpenTabGroupEditor(*space);
      }
    }
  }

  raw_ptr<BrowserWindowInterface> browser_;
  raw_ptr<SpaceLabel> label_ = nullptr;
  raw_ptr<SpaceDivider> divider_ = nullptr;
};

BEGIN_METADATA(MoriSpaceHeader)
END_METADATA

class SpaceBar : public views::View,
                 public TabStripModelObserver,
                 public tab_groups::TabGroupSyncService::Observer {
  METADATA_HEADER(SpaceBar, views::View)

 public:
  explicit SpaceBar(BrowserWindowInterface* browser) : browser_(browser) {
    // Mori 3: dots centred (only with two or more spaces), "+" at the end.
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal, gfx::Insets::VH(6, 7),
        2));
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
    // Arc keeps a space whose last tab closed or moved away; a tab group
    // can't be empty, so the space comes back with a new tab.
    if (change.type == TabGroupChange::kClosed && change.GetCloseChange() &&
        change.GetCloseChange()->reason() ==
            TabGroupChange::TabGroupClosureReason::kGroupClosed) {
      const bool deleted = SpacesBeingDeleted().erase(change.group) > 0;
      TabStripModel* model = SpacesModel(browser_);
      auto known = known_.find(change.group);
      if (!deleted && model && !model->closing_all() && !model->empty() &&
          known != known_.end()) {
        KnownSpace space = known->second;
        space.was_current = last_current_ == change.group;
        to_restore_.push_back(space);
      }
      if (known != known_.end()) {
        known_.erase(known);
      }
    }
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
      RestoreEmptiedSpaces();
      KeepEveryTabInASpace();
      Remember();
    }
    Rebuild();
  }

  // What each space looked like, so an emptied one can come back as it was.
  void Remember() {
    TabStripModel* model = SpacesModel(browser_);
    if (!model) {
      return;
    }
    known_.clear();
    for (const tab_groups::TabGroupId& id : SpacesInOrder(model)) {
      const TabGroup* group = model->group_model()->GetTabGroup(id);
      known_[id] = {group->visual_data()->title(),
                    group->visual_data()->color(),
                    static_cast<int>(group->ListTabs().start()), false};
    }
    last_current_ = model->GetFocusedGroup();
  }

  void RestoreEmptiedSpaces() {
    TabStripModel* model = SpacesModel(browser_);
    if (!model || model->closing_all() || model->empty()) {
      to_restore_.clear();
      return;
    }
    std::vector<KnownSpace> spaces;
    spaces.swap(to_restore_);
    for (const KnownSpace& space : spaces) {
      NavigateParams params(browser_, GURL(chrome::kChromeUINewTabURL),
                            ui::PAGE_TRANSITION_AUTO_TOPLEVEL);
      params.disposition = WindowOpenDisposition::NEW_BACKGROUND_TAB;
      params.tabstrip_index = std::min(space.start, model->count());
      Navigate(&params);
      const int index =
          model->GetIndexOfWebContents(params.navigated_or_inserted_contents);
      if (index == TabStripModel::kNoTab) {
        continue;
      }
      // New tabs join the space on screen; this one starts its own again.
      const tab_groups::TabGroupId id = model->AddToNewGroup({index});
      NameSpace(model, id, space.title, space.color);
      if (space.was_current) {
        model->SetFocusedGroup(id);
      }
    }
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
    const std::vector<tab_groups::TabGroupId> spaces = SpacesInOrder(model);
    auto* layout = static_cast<views::BoxLayout*>(GetLayoutManager());
    // Mori 3: the library (history and bookmarks) at the start.
    auto library = views::CreateVectorImageButtonWithNativeTheme(
        base::BindRepeating(
            [](BrowserWindowInterface* browser) {
              chrome::ExecuteCommand(browser, IDC_SHOW_HISTORY);
            },
            browser_.get()),
        LucideIcon("library"), 16);
    library->SetTooltipText(u"History & Bookmarks (Ctrl+Y)");
    library->SetPreferredSize(gfx::Size(kButtonSize, kButtonSize));
    AddChildView(std::move(library));
    // Flexible spacers keep the dots centred and "+" at the end.
    auto* before = AddChildView(std::make_unique<views::View>());
    layout->SetFlexForView(before, 1);
    int number = 0;
    for (const tab_groups::TabGroupId& id : spaces) {
      ++number;
      if (spaces.size() < 2) {
        break;
      }
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
    auto* after = AddChildView(std::make_unique<views::View>());
    layout->SetFlexForView(after, 1);
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

  struct KnownSpace {
    std::u16string title;
    tab_groups::TabGroupColorId color;
    int start;
    bool was_current;
  };

  raw_ptr<BrowserWindowInterface> browser_;
  bool update_pending_ = false;
  bool groups_ready_ = false;
  std::map<tab_groups::TabGroupId, KnownSpace> known_;
  std::optional<tab_groups::TabGroupId> last_current_;
  std::vector<KnownSpace> to_restore_;
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

int IndexForNewTab(BrowserWindowInterface* browser) {
  TabStripModel* model = SpacesModel(browser);
  if (!model || !model->GetFocusedGroup().has_value()) {
    return -1;
  }
  return model->group_model()
      ->GetTabGroup(*model->GetFocusedGroup())
      ->ListTabs()
      .start();
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

void DeleteSpace(BrowserWindowInterface* browser) {
  TabStripModel* model = SpacesModel(browser);
  if (!model || SpacesInOrder(model).size() < 2 ||
      !model->GetFocusedGroup().has_value()) {
    return;
  }
  const tab_groups::TabGroupId id = *model->GetFocusedGroup();
  SwitchSpace(browser, /*forward=*/false);
  SpacesBeingDeleted().insert(id);
  model->CloseAllTabsInGroup(id);
}

void SelectSpace(BrowserWindowInterface* browser, int index) {
  TabStripModel* model = SpacesModel(browser);
  if (!model) {
    return;
  }
  const std::vector<tab_groups::TabGroupId> spaces = SpacesInOrder(model);
  if (index >= 0 && index < static_cast<int>(spaces.size())) {
    model->SetFocusedGroup(spaces[index]);
  }
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

std::unique_ptr<views::View> CreateSpaceHeader(
    BrowserWindowInterface* browser) {
  return std::make_unique<MoriSpaceHeader>(browser);
}

std::unique_ptr<views::View> CreateSpaceBar(BrowserWindowInterface* browser) {
  return std::make_unique<SpaceBar>(browser);
}

}  // namespace mori
