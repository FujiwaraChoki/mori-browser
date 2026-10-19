#include "chrome/browser/ui/views/mori/mori_spaces.h"

#include "build/build_config.h"
#include "chrome/browser/ui/views/mori/mori_window_events.h"
#include "chrome/browser/ui/views/mori/mori_assistant.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_little.h"
#include "chrome/browser/ui/views/mori/mori_new_tab.h"
#include "chrome/browser/ui/views/mori/mori_sidebar.h"
#include "chrome/browser/ui/views/mori/mori_space_editor.h"
#include "chrome/browser/ui/views/mori/mori_space_themes.h"

#include <algorithm>
#include <map>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "base/callback_list.h"
#include "base/feature_list.h"
#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/task/sequenced_task_runner.h"
#include "base/no_destructor.h"
#include "base/scoped_observation.h"
#include "base/strings/string_number_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "base/time/time.h"
#include "base/timer/timer.h"
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
#include "components/saved_tab_groups/public/saved_tab_group.h"
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
#include "ui/events/event_observer.h"
#include "ui/views/event_monitor.h"
#include "ui/views/widget/widget.h"
#include "ui/gfx/geometry/point_conversions.h"
#include "content/public/browser/web_contents.h"
#include "chrome/browser/ui/tabs/tab_enums.h"
#include <string_view>
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
              std::string symbol,
              const std::u16string& title,
              bool current)
      : views::Button(std::move(callback)),
        symbol_(std::move(symbol)),
        current_(current) {
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
    // Mori 3 (Zen): the space's symbol, or a dot, bright when current.
    const bool dark = color_utils::IsDark(colors->GetColor(ui::kColorFrameActive));
    const SkColor color =
        current_ ? (dark ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                         : SkColorSetRGB(0x1D, 0x1F, 0x22))
                 : (dark ? SkColorSetRGB(0x7A, 0x7B, 0x7D)
                         : SkColorSetRGB(0x8A, 0x8C, 0x8E));
    if (!symbol_.empty()) {
      PaintSpaceSymbol(canvas, symbol_, GetLocalBounds(), 14, color);
      return;
    }
    flags.setColor(color);
    canvas->DrawCircle(bounds.CenterPoint(), 4, flags);
  }

 private:
  std::string symbol_;
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

constexpr base::TimeDelta kArchiveCheckInterval = base::Minutes(10);

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

  // Labels only take clicks when their text is selectable.
  bool GetCanProcessEventsWithinSubtree() const override { return true; }

  // Double-click or right-click edits the space.
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

// Mori 3: a symbol the user picked, 13px, before the space's name.
class SpaceSymbol : public views::View {
  METADATA_HEADER(SpaceSymbol, views::View)

 public:
  static constexpr int kSize = 13;

  SpaceSymbol() { SetPreferredSize(gfx::Size(kSize, kLabelHeight)); }

  void SetSymbol(std::string symbol) {
    symbol_ = std::move(symbol);
    SetVisible(!symbol_.empty());
    SchedulePaint();
  }

  void OnPaint(gfx::Canvas* canvas) override {
    PaintSpaceSymbol(canvas, symbol_, GetLocalBounds(), kSize,
                     MutedColor(this));
  }

 private:
  std::string symbol_;
};

BEGIN_METADATA(SpaceSymbol)
END_METADATA

// "↓ Clear" at the end of the divider: closes the space's tabs (Zen's
// Clear, Arc's archive of Today).
class ClearButton : public views::Button {
  METADATA_HEADER(ClearButton, views::Button)

 public:
  explicit ClearButton(PressedCallback callback)
      : views::Button(std::move(callback)) {
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal,
        gfx::Insets::TLBR(0, 8, 0, 2), 3));
    layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    icon_ = AddChildView(std::make_unique<views::ImageView>());
    label_ = AddChildView(std::make_unique<views::Label>(u"Clear"));
    label_->SetFontList(views::Label::GetDefaultFontList()
                            .DeriveWithSizeDelta(
                                11 - views::Label::GetDefaultFontList()
                                         .GetFontSize())
                            .DeriveWithWeight(gfx::Font::Weight::MEDIUM));
    SetTooltipText(u"Close every tab in this space");
    SetAccessibleName(u"Clear");
  }

  void OnThemeChanged() override {
    views::Button::OnThemeChanged();
    UpdateColors();
  }
  void StateChanged(ButtonState old_state) override { UpdateColors(); }

 private:
  // Half the muted colour at rest, the full one under the pointer.
  void UpdateColors() {
    if (!GetColorProvider()) {
      return;
    }
    const SkColor frame =
        GetColorProvider()->GetColor(ui::kColorFrameActive);
    const SkColor muted = MutedColor(this);
    const SkColor color =
        GetState() == STATE_HOVERED || GetState() == STATE_PRESSED
            ? muted
            : color_utils::AlphaBlend(muted, frame, 0.5f);
    icon_->SetImage(
        ui::ImageModel::FromVectorIcon(LucideIcon("arrow-down"), color, 10));
    label_->SetEnabledColor(color);
  }

  raw_ptr<views::ImageView> icon_ = nullptr;
  raw_ptr<views::Label> label_ = nullptr;
};

BEGIN_METADATA(ClearButton)
END_METADATA

// A 1px line in white (or black) at low opacity, as Mori 3's divider. While
// the pointer is over the sidebar and the space has tabs to clear, its end
// turns into "Clear".
class SpaceDivider : public views::View, public ui::EventObserver {
  METADATA_HEADER(SpaceDivider, views::View)

 public:
  explicit SpaceDivider(views::Button::PressedCallback clear) {
    SetPreferredSize(gfx::Size(0, kDividerHeight));
    clear_ = AddChildView(std::make_unique<ClearButton>(std::move(clear)));
    clear_->SetVisible(false);
  }

  void SetClearable(bool clearable) {
    clearable_ = clearable;
    UpdateClear();
  }

  void AddedToWidget() override {
    // An application monitor: a window monitor misses moves over the page.
    monitor_ = MonitorWindow(
        this, GetWidget(),
        {ui::EventType::kMouseMoved, ui::EventType::kMouseExited});
  }
  void RemovedFromWidget() override { monitor_.reset(); }

  // ui::EventObserver:
  using views::View::OnEvent;
  void OnEvent(const ui::Event& event) override {
    const views::View* sidebar = Sidebar();
    // Only this window's events; their root location is in its coordinates.
    if (!sidebar || !event.IsLocatedEvent() ||
        !IsEventInWindow(event, GetWidget())) {
      return;
    }
    // An exit also comes when the pointer crosses from the page onto the
    // sidebar; where it is decides.
    gfx::Point point =
        gfx::ToFlooredPoint(event.AsLocatedEvent()->root_location_f());
    views::View::ConvertPointFromWidget(sidebar, &point);
    const bool over = sidebar->GetLocalBounds().Contains(point);
    if (over != pointer_over_sidebar_) {
      pointer_over_sidebar_ = over;
      UpdateClear();
    }
  }

  void Layout(PassKey) override {
    const gfx::Size size = clear_->GetPreferredSize();
    clear_->SetBounds(width() - size.width(), 0, size.width(), height());
  }

  void OnPaint(gfx::Canvas* canvas) override {
    const ui::ColorProvider* colors = GetColorProvider();
    const SkColor frame = colors->GetColor(ui::kColorFrameActive);
    const bool dark = color_utils::IsDark(frame);
    const SkColor line = color_utils::AlphaBlend(
        dark ? SK_ColorWHITE : SK_ColorBLACK, frame, dark ? 0.09f : 0.10f);
    const int end = clear_->GetVisible() ? clear_->x() : width();
    canvas->FillRect(gfx::Rect(4, height() / 2, end - 4, 1), line);
  }

 private:
  // The vertical tab strip this divider sits in.
  const views::View* Sidebar() const {
    for (const views::View* v = parent(); v; v = v->parent()) {
      if (std::string_view(v->GetClassName()) == "VerticalTabStripRegionView") {
        return v;
      }
    }
    return nullptr;
  }

  void UpdateClear() {
    const bool show = clearable_ && pointer_over_sidebar_;
    if (show != clear_->GetVisible()) {
      clear_->SetVisible(show);
      SchedulePaint();
    }
  }

  raw_ptr<ClearButton> clear_ = nullptr;
  bool clearable_ = false;
  bool pointer_over_sidebar_ = false;
  std::unique_ptr<views::EventMonitor> monitor_;
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
    label_row_ = AddChildView(std::make_unique<views::View>());
    auto* row_layout =
        label_row_->SetLayoutManager(std::make_unique<views::BoxLayout>(
            views::BoxLayout::Orientation::kHorizontal,
            gfx::Insets::TLBR(0, kRowContentInset, 0, 0), 6));
    row_layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    symbol_ = label_row_->AddChildView(std::make_unique<SpaceSymbol>());
    label_ = label_row_->AddChildView(std::make_unique<SpaceLabel>(
        base::BindRepeating(&MoriSpaceHeader::EditSpace,
                            base::Unretained(this))));
    label_->SetBorder(nullptr);
    row_layout->SetFlexForView(label_, 1);
    divider_ = AddChildView(std::make_unique<SpaceDivider>(base::BindRepeating(
        [](BrowserWindowInterface* browser) { ClearSpace(browser); },
        browser)));
    AddChildView(std::make_unique<NewTabRow>(base::BindRepeating(
        [](BrowserWindowInterface* browser) {
          // Opens the command bar (see the IDC_NEW_TAB hook).
          chrome::ExecuteCommand(browser, IDC_NEW_TAB);
        },
        browser)));
    if (TabStripModel* model = SpacesModel(browser_)) {
      model->AddObserver(this);
    }
    theme_subscription_ = AddSpaceThemeObserver(
        browser_, base::BindRepeating(&MoriSpaceHeader::Update,
                                      base::Unretained(this)));
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
    symbol_->SetSymbol(GetSpaceTheme(browser_, *space).symbol);
    // Mori 3 puts the label's top 11px under the favorites (the tab strip
    // already adds 8px there) or 5px under the address field.
    const bool has_favorites = model->IndexOfFirstNonPinnedTab() > 0;
    label_row_->SetProperty(views::kMarginsKey,
                            gfx::Insets::TLBR(has_favorites ? 3 : 5, 0, 0, 0));
    divider_->SetProperty(views::kMarginsKey, gfx::Insets::TLBR(9, 0, 3, 0));
    // Nothing to clear in a space holding just a new tab.
    const gfx::Range tabs =
        model->group_model()->GetTabGroup(*space)->ListTabs();
    content::WebContents* first = model->GetWebContentsAt(tabs.start());
    divider_->SetClearable(
        tabs.length() > 1 ||
        (first && first->GetLastCommittedURL() !=
                      GURL(chrome::kChromeUINewTabURL) &&
         first->GetVisibleURL() != GURL(chrome::kChromeUINewTabURL)));
    SetVisible(true);
    InvalidateLayout();
  }

  // After the click that asked for it, so its release doesn't take the
  // editor's focus.
  void EditSpace() {
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(
                       [](base::WeakPtr<BrowserWindowInterface> browser) {
                         if (browser) {
                           mori::EditSpace(browser.get());
                         }
                       },
                       browser_->GetWeakPtr()));
  }

  raw_ptr<BrowserWindowInterface> browser_;
  raw_ptr<views::View> label_row_ = nullptr;
  raw_ptr<SpaceSymbol> symbol_ = nullptr;
  raw_ptr<SpaceLabel> label_ = nullptr;
  raw_ptr<SpaceDivider> divider_ = nullptr;
  base::CallbackListSubscription theme_subscription_;
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
      if (!archive_timer_.IsRunning()) {
        archive_timer_.Start(FROM_HERE, kArchiveCheckInterval, this,
                             &SpaceBar::ArchiveIdleTabs);
      }
    }
    if (auto* state = tabs::VerticalTabStripStateController::From(browser_)) {
      collapse_subscription_ = state->RegisterOnCollapseChanged(
          base::BindRepeating(&SpaceBar::OnCollapseChanged,
                              base::Unretained(this)));
      // Chromium's icon-only collapsed sidebar isn't Mori's; a profile or
      // session that saved it opens expanded.
      if (state->IsCollapsed()) {
        state->RequestCollapse(false);
      }
    }
    // New symbols show here; a new gradient washes over the chrome.
    theme_subscription_ = AddSpaceThemeObserver(
        browser_, base::BindRepeating(
                      [](SpaceBar* bar) {
                        bar->ScheduleUpdate();
                        RepaintSpaceWash(bar->browser_);
                      },
                      base::Unretained(this)));
    ScheduleUpdate();
  }

  // tab_groups::TabGroupSyncService::Observer:
  void OnInitialized() override {
    groups_ready_ = true;
    if (!archive_timer_.IsRunning()) {
      archive_timer_.Start(FROM_HERE, kArchiveCheckInterval, this,
                           &SpaceBar::ArchiveIdleTabs);
    }
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
    // The chrome takes the new space's wash.
    RepaintSpaceWash(browser_);
  }
  void OnTabStripModelChanged(TabStripModel* tab_strip_model,
                              const TabStripModelChange& change,
                              const TabStripSelectionChange& selection) override {
    if (change.type() != TabStripModelChange::kSelectionOnly) {
      ScheduleUpdate();
    }
  }

  void AddedToWidget() override {
    views::View::AddedToWidget();
    // Mori 3's new tab page, in this window (once the window is set up:
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

  // Arc's auto-archive (Mori 3's default of 24 hours): space tabs nobody
  // looked at for a day close. Favorites, sound and the tab on screen stay;
  // closed tabs stay in History and Reopen Closed Tab.
  void ArchiveIdleTabs() {
    TabStripModel* model = SpacesModel(browser_);
    if (!model || !groups_ready_) {
      return;
    }
    const int hours = ArchiveAfterHours(browser_->GetProfile());
    if (hours <= 0) {
      return;
    }
    const base::Time cutoff = base::Time::Now() - base::Hours(hours);
    std::vector<tabs::TabHandle> idle;
    for (int i = 0; i < model->count(); ++i) {
      tabs::TabInterface* tab = model->GetTabAtIndex(i);
      content::WebContents* contents = tab->GetContents();
      if (i == model->active_index() || tab->IsPinned() || !tab->GetGroup() ||
          tab->IsSplit() || !contents || contents->IsCurrentlyAudible() ||
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
#if BUILDFLAG(IS_MAC)
    library->SetTooltipText(u"History & Bookmarks (⌘Y)");
#else
    library->SetTooltipText(u"History & Bookmarks (Ctrl+Y)");
#endif
    library->SetPreferredSize(gfx::Size(kButtonSize, kButtonSize));
    AddChildView(std::move(library));
    // Mori 3: the assistant beside the library.
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
          GetSpaceTheme(browser_, id).symbol, title, current == id));
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
  base::RepeatingTimer archive_timer_;
  base::CallbackListSubscription theme_subscription_;
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

std::string SpaceKey(BrowserWindowInterface* browser,
                     const tab_groups::TabGroupId& space) {
  // A space outlives restarts as its saved tab group; its local id doesn't.
  if (tab_groups::TabGroupSyncService* sync =
          tab_groups::TabGroupSyncServiceFactory::GetForProfile(
              browser->GetProfile())) {
    if (std::optional<tab_groups::SavedTabGroup> group =
            sync->GetGroup(space)) {
      return group->saved_guid().AsLowercaseString();
    }
  }
  return space.ToString();
}

std::optional<tab_groups::TabGroupId> FindSpace(BrowserWindowInterface* browser,
                                                const std::string& key) {
  TabStripModel* model = SpacesModel(browser);
  if (!model) {
    return std::nullopt;
  }
  for (const tab_groups::TabGroupId& id :
       model->group_model()->ListTabGroups()) {
    if (SpaceKey(browser, id) == key) {
      return id;
    }
  }
  return std::nullopt;
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
  // Mori 3 asks for the new space's name, symbol and theme.
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE,
      base::BindOnce(
          [](base::WeakPtr<BrowserWindowInterface> browser) {
            if (browser) {
              EditSpace(browser.get());
            }
          },
          browser->GetWeakPtr()));
}

void ClearSpace(BrowserWindowInterface* browser) {
  TabStripModel* model = SpacesModel(browser);
  const std::optional<tab_groups::TabGroupId> space =
      model ? model->GetFocusedGroup() : std::nullopt;
  if (!space || !model->group_model()->ContainsTabGroup(*space)) {
    return;
  }
  std::vector<tabs::TabHandle> old_tabs;
  const gfx::Range range =
      model->group_model()->GetTabGroup(*space)->ListTabs();
  for (size_t i = range.start(); i < range.end(); ++i) {
    old_tabs.push_back(model->GetTabAtIndex(i)->GetHandle());
  }
  // The space keeps a new tab on screen, as Arc's empty space.
  NavigateParams params(browser, GURL(chrome::kChromeUINewTabURL),
                        ui::PAGE_TRANSITION_AUTO_TOPLEVEL);
  params.disposition = WindowOpenDisposition::NEW_FOREGROUND_TAB;
  params.tabstrip_index =
      model->group_model()->GetTabGroup(*space)->ListTabs().start();
  Navigate(&params);
  const int index =
      model->GetIndexOfWebContents(params.navigated_or_inserted_contents);
  if (index == TabStripModel::kNoTab) {
    return;
  }
  if (model->GetTabGroupForTab(index) != space) {
    model->AddToExistingGroup({index}, *space);
  }
  for (const tabs::TabHandle& handle : old_tabs) {
    tabs::TabInterface* tab = handle.Get();
    const int old_index =
        tab ? model->GetIndexOfTab(tab) : TabStripModel::kNoTab;
    if (old_index != TabStripModel::kNoTab) {
      model->CloseWebContentsAt(old_index, TabCloseTypes::CLOSE_USER_GESTURE);
    }
  }
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
