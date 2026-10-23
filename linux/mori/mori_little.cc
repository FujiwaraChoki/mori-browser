#include "chrome/browser/ui/views/mori/mori_little.h"

#include <algorithm>

#include <map>
#include <memory>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "base/auto_reset.h"
#include "base/functional/bind.h"
#include "base/memory/weak_ptr.h"
#include "base/no_destructor.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/sequenced_task_runner.h"
#include "base/values.h"
#include "build/build_config.h"
#include "cc/paint/paint_flags.h"
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
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_command_bar.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_json_file.h"
#include "chrome/browser/ui/views/mori/mori_tab_list.h"
#include "chrome/browser/ui/views/toolbar/toolbar_view.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/accelerators/accelerator.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/base/window_open_disposition.h"
#include "ui/color/color_id.h"
#include "ui/color/color_provider.h"
#include "ui/display/screen.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/font_list.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/paint_vector_icon.h"
#include "ui/menus/simple_menu_model.h"
#include "ui/native_theme/native_theme.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/background.h"
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/button/label_button.h"
#include "ui/views/controls/menu/menu_runner.h"
#include "ui/views/layout/layout_manager_base.h"
#include "ui/views/layout/proposed_layout.h"
#include "ui/views/widget/widget.h"
#include "url/gurl.h"

namespace mori {
namespace {

// A disposable reading window, centred on the display where the user works.
constexpr int kWidth = 900;
constexpr int kHeight = 680;
constexpr int kHeaderHeight = kLittleHeaderHeight;
DEFINE_UI_CLASS_PROPERTY_KEY(bool, kLittleWindow, false)

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
      settings =
          std::make_unique<Settings>(path.AppendASCII("Mori Settings.json"));
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
void OpenInMori(BrowserWindowInterface* little,
                BrowserWindowInterface* target = nullptr) {
  TabStripModel* from = little->GetTabStripModel();
  const int index = from->active_index();
  if (index == TabStripModel::kNoTab) {
    return;
  }
  if (!target || target->IsDeleteScheduled()) {
    target = ProfileBrowserCollection::GetForProfile(little->GetProfile())
                 ->FindTabbedBrowser();
  }
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

// Only this header participates in the popup toolbar's layout. Chromium's
// location bar remains alive for security state and command routing.
class LittleToolbarLayout : public views::LayoutManagerBase {
 public:
  explicit LittleToolbarLayout(views::View* header) : header_(header) {}
  gfx::Size GetMinimumSize(const views::View*) const override {
    return gfx::Size(560, kHeaderHeight);
  }

 protected:
  views::ProposedLayout CalculateProposedLayout(
      const views::SizeBounds& bounds) const override {
    const int width =
        bounds.width().is_bounded() ? bounds.width().value() : kWidth;
    views::ProposedLayout layout;
    layout.host_size = gfx::Size(width, kHeaderHeight);
    for (views::View* child : host_view()->children()) {
      layout.child_layouts.push_back({child, child == header_,
                                      gfx::Rect(layout.host_size),
                                      views::SizeBounds(layout.host_size)});
    }
    return layout;
  }

 private:
  raw_ptr<views::View> header_;
};

// These controls deliberately use a neutral palette, independent of the
// profile's accent color. Popup chrome must not turn into a selection-blue bar.
bool LittleDark(const views::View* view) {
  return view->GetNativeTheme()->preferred_color_scheme() ==
         ui::NativeTheme::PreferredColorScheme::kDark;
}
SkColor LittleForeground(const views::View* view) {
  return LittleDark(view) ? SkColorSetRGB(0xD7, 0xD7, 0xDA)
                          : SkColorSetRGB(0x56, 0x56, 0x59);
}

class LittleIconButton : public views::Button {
  METADATA_HEADER(LittleIconButton, views::Button)
 public:
  LittleIconButton(PressedCallback callback,
                   std::string_view icon,
                   const std::u16string& name)
      : views::Button(std::move(callback)), icon_(icon) {
    SetFocusBehavior(FocusBehavior::ALWAYS);
    SetTooltipText(name);
    SetAccessibleName(name);
  }
  void PaintButtonContents(gfx::Canvas* canvas) override {
    const bool hover =
        GetState() == STATE_HOVERED || GetState() == STATE_PRESSED;
    if (hover) {
      cc::PaintFlags fill;
      fill.setAntiAlias(true);
      fill.setColor(
          SkColorSetA(LittleDark(this) ? SK_ColorWHITE : SK_ColorBLACK, 16));
      canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), 7, fill);
    }
    const SkColor color =
        SkColorSetA(LittleForeground(this), GetEnabled() ? 255 : 75);
    const int x = (width() - 16) / 2;
    const int y = (height() - 16) / 2;
    canvas->Save();
    canvas->Translate(gfx::Vector2d(x, y));
    if (icon_ == "chevron-down") {
      cc::PaintFlags stroke;
      stroke.setAntiAlias(true);
      stroke.setColor(color);
      stroke.setStrokeWidth(1.5f);
      stroke.setStrokeCap(cc::PaintFlags::kRound_Cap);
      canvas->DrawLine(gfx::PointF(4, 6), gfx::PointF(8, 10), stroke);
      canvas->DrawLine(gfx::PointF(8, 10), gfx::PointF(12, 6), stroke);
    } else {
      gfx::PaintVectorIcon(canvas, LucideIcon(icon_), 16, color);
    }
    canvas->Restore();
  }
  void StateChanged(ButtonState) override { SchedulePaint(); }

 private:
  std::string_view icon_;
};
BEGIN_METADATA(LittleIconButton)
END_METADATA

class LittleHeader : public views::View,
                     public TabStripModelObserver,
                     public ui::SimpleMenuModel::Delegate {
  METADATA_HEADER(LittleHeader, views::View)
 public:
  explicit LittleHeader(BrowserWindowInterface* browser)
      : browser_(browser), menu_(this) {
    back_ = Navigation("arrow-left", u"Back", IDC_BACK);
    forward_ = Navigation("arrow-right", u"Forward", IDC_FORWARD);
    reload_ = Navigation("rotate-cw", u"Reload", IDC_RELOAD);
    address_ = AddChildView(std::make_unique<views::LabelButton>(
        base::BindRepeating(
            [](BrowserWindowInterface* b) {
              ShowCommandBar(b, CommandBarMode::kCurrentTab);
            },
            browser),
        u"Search or enter URL"));
    address_->SetHorizontalAlignment(gfx::ALIGN_CENTER);
    address_->SetTooltipText(u"Search or enter URL");
    address_->SetFocusBehavior(FocusBehavior::ALWAYS);
    open_ = AddChildView(std::make_unique<views::LabelButton>(
        base::BindRepeating(&LittleHeader::Promote, base::Unretained(this)),
        u"Open in Mori"));
    open_->SetHorizontalAlignment(gfx::ALIGN_CENTER);
    open_->SetFocusBehavior(FocusBehavior::ALWAYS);
#if BUILDFLAG(IS_MAC)
    open_->SetTooltipText(u"Move this page to Mori (⌘O)");
    constexpr int modifier = ui::EF_COMMAND_DOWN;
#else
    open_->SetTooltipText(u"Move this page to Mori (Ctrl+O)");
    constexpr int modifier = ui::EF_CONTROL_DOWN;
#endif
    choose_ = AddChildView(std::make_unique<LittleIconButton>(
        base::BindRepeating(&LittleHeader::ChooseWindow,
                            base::Unretained(this)),
        "chevron-down", u"Choose a Mori window"));
    AddAccelerator(ui::Accelerator(ui::VKEY_O, modifier));
    AddAccelerator(ui::Accelerator(ui::VKEY_O, modifier | ui::EF_ALT_DOWN));
    browser_->GetTabStripModel()->AddObserver(this);
    Update();
  }

  void Layout(PassKey) override {
    int left = 12;
#if BUILDFLAG(IS_MAC)
    // Native traffic lights share this row with the navigation controls.
    if (!GetWidget() || !GetWidget()->IsFullscreen())
      left = 86;
#endif
    for (views::View* button : {back_.get(), forward_.get(), reload_.get()}) {
      button->SetBounds(left, 6, 30, 32);
      left += 30;
    }
    const int right = width() - 12;
    const int open_width = 116;
    choose_->SetBounds(right - 28, 6, 28, 32);
    open_->SetBounds(right - 28 - open_width, 6, open_width, 32);
    const int start = left + 12;
    const int end = open_->x() - 12;
    const int address_width = std::max(0, std::min(420, end - start));
    const int centered = (width() - address_width) / 2;
    address_->SetBounds(
        std::clamp(centered, start, std::max(start, end - address_width)), 6,
        address_width, 32);
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    const bool dark = LittleDark(this);
    canvas->DrawColor(dark ? SkColorSetRGB(0x25, 0x25, 0x27)
                           : SkColorSetRGB(0xF3, 0xF3, 0xF3));
    cc::PaintFlags fill;
    fill.setAntiAlias(true);
    fill.setColor(dark ? SkColorSetRGB(0x35, 0x35, 0x38) : SK_ColorWHITE);
    const gfx::RectF action(open_->x(), 6, open_->width() + choose_->width(),
                            32);
    canvas->DrawRoundRect(action, 8, fill);
    fill.setStyle(cc::PaintFlags::kStroke_Style);
    fill.setStrokeWidth(1);
    fill.setColor(SkColorSetA(dark ? SK_ColorWHITE : SK_ColorBLACK, 20));
    canvas->DrawRoundRect(action, 8, fill);
  }

  void OnThemeChanged() override {
    views::View::OnThemeChanged();
    address_->SetEnabledTextColors(LittleForeground(this));
    open_->SetEnabledTextColors(LittleForeground(this));
  }

  bool AcceleratorPressed(const ui::Accelerator& accelerator) override {
    if (accelerator.IsAltDown())
      ChooseWindow();
    else
      Promote();
    return true;
  }

  void OnTabStripModelChanged(TabStripModel*,
                              const TabStripModelChange&,
                              const TabStripSelectionChange&) override {
    Update();
  }
  void OnTabChangedAt(tabs::TabInterface*, TabChangeType) override { Update(); }

  bool IsCommandIdEnabled(int id) const override { return true; }
  void ExecuteCommand(int id, int) override {
    base::WeakPtr<BrowserWindowInterface> target;
    if (id > 0 && id <= static_cast<int>(targets_.size()))
      target = targets_[id - 1];
    // Detaching the last tab destroys this header; wait until menu dispatch
    // ends.
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(
            [](base::WeakPtr<BrowserWindowInterface> little,
               base::WeakPtr<BrowserWindowInterface> target, bool new_window) {
              if (!little)
                return;
              BrowserWindowInterface* destination = target.get();
              if (new_window) {
                destination = CreateBrowserWindow(
                    BrowserWindowCreateParams(little->GetProfile(), true));
              }
              OpenInMori(little.get(), destination);
            },
            browser_->GetWeakPtr(), target, id == 0));
  }

 private:
  LittleIconButton* Navigation(const char* icon,
                               std::u16string name,
                               int command) {
    return AddChildView(std::make_unique<LittleIconButton>(
        base::BindRepeating(
            [](BrowserWindowInterface* b, int command) {
              chrome::ExecuteCommand(b, command);
            },
            browser_, command),
        icon, name));
  }
  void Update() {
    auto* contents = browser_->GetTabStripModel()->GetActiveWebContents();
    const GURL url = contents ? contents->GetVisibleURL() : GURL();
    address_->SetText(url.has_host() ? base::UTF8ToUTF16(url.host())
                                     : u"Search or enter URL");
    address_->SetTooltipText(url.is_empty() ? u"Search or enter URL"
                                            : base::UTF8ToUTF16(url.spec()));
    address_->GetViewAccessibility().SetName(u"Address and search");
    back_->SetEnabled(contents && contents->GetController().CanGoBack());
    forward_->SetEnabled(contents && contents->GetController().CanGoForward());
    open_->SetEnabled(contents != nullptr);
  }
  void Promote() { OpenInMori(browser_); }
  void ChooseWindow() {
    if (runner_ && runner_->IsRunning())
      return;
    runner_.reset();
    menu_.Clear();
    targets_.clear();
    ProfileBrowserCollection::GetForProfile(browser_->GetProfile())
        ->ForEach(
            [&](BrowserWindowInterface* b) {
              if (b->GetType() != BrowserWindowInterface::TYPE_NORMAL ||
                  b->IsDeleteScheduled())
                return true;
              targets_.push_back(b->GetWeakPtr());
              auto* contents = b->GetTabStripModel()->GetActiveWebContents();
              std::u16string title = contents ? contents->GetTitle() : u"Mori";
              menu_.AddItem(static_cast<int>(targets_.size()),
                            title.empty() ? u"Mori" : title);
              return true;
            },
            BrowserCollection::Order::kActivation);
    if (!targets_.empty())
      menu_.AddSeparator(ui::NORMAL_SEPARATOR);
    menu_.AddItem(0, u"New Mori Window");
    runner_ = std::make_unique<views::MenuRunner>(
        &menu_, views::MenuRunner::HAS_MNEMONICS);
    runner_->RunMenuAt(GetWidget(), nullptr, choose_->GetBoundsInScreen(),
                       views::MenuAnchorPosition::kTopRight,
                       ui::mojom::MenuSourceType::kNone);
  }
  raw_ptr<BrowserWindowInterface> browser_;
  raw_ptr<LittleIconButton> back_;
  raw_ptr<LittleIconButton> forward_;
  raw_ptr<LittleIconButton> reload_;
  raw_ptr<views::LabelButton> address_;
  raw_ptr<views::LabelButton> open_;
  raw_ptr<LittleIconButton> choose_;
  std::vector<base::WeakPtr<BrowserWindowInterface>> targets_;
  ui::SimpleMenuModel menu_;
  std::unique_ptr<views::MenuRunner> runner_;
};
BEGIN_METADATA(LittleHeader)
END_METADATA

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
  BrowserWindowCreateParams params(BrowserWindowInterface::TYPE_POPUP, *profile,
                                   /*from_user_gesture=*/true);
  // Centred on the screen.
  const gfx::Rect work_area =
      display::Screen::Get()
          ->GetDisplayNearestPoint(
              display::Screen::Get()->GetCursorScreenPoint())
          .work_area();
  const int width = std::min(kWidth, work_area.width());
  const int height = std::min(kHeight, work_area.height());
  params.initial_bounds = gfx::Rect(
      work_area.x() + (work_area.width() - width) / 2,
      work_area.y() + (work_area.height() - height) / 2, width, height);
  params.initial_show_state = ui::mojom::WindowShowState::kNormal;
  BrowserWindowInterface* little;
  {
    base::AutoReset<bool> building(&g_building_little, true);
    little = CreateBrowserWindow(std::move(params));
  }
  NavigateParams nav(little, url, ui::PAGE_TRANSITION_AUTO_TOPLEVEL);
  nav.disposition = WindowOpenDisposition::NEW_FOREGROUND_TAB;
  Navigate(&nav);
  little->GetWindow()->Show();
}

bool IsLittleWindow(BrowserWindowInterface* browser) {
  if (!browser || browser->GetType() != BrowserWindowInterface::TYPE_POPUP)
    return false;
  if (g_building_little)
    return true;
  auto* view = BrowserView::GetBrowserViewForBrowser(browser);
  return view && view->GetProperty(kLittleWindow);
}

bool IsLittleHeaderCaption(BrowserView* browser_view, const gfx::Point& point) {
  if (!IsLittleWindow(browser_view->browser()) || !browser_view->toolbar())
    return false;
  auto* toolbar = browser_view->toolbar();
  for (views::View* child : toolbar->children()) {
    if (std::string_view(child->GetClassName()) != "LittleHeader")
      continue;
    gfx::Point local(point);
    views::View::ConvertPointToTarget(browser_view->parent(), child, &local);
    return child->HitTestPoint(local) &&
           child->GetEventHandlerForPoint(local) == child;
  }
  return false;
}

bool OpenLittleInMori(BrowserWindowInterface* browser) {
  if (!IsLittleWindow(browser))
    return false;
  OpenInMori(browser);
  return true;
}

void MaybeAddLittleHeader(ToolbarView* toolbar, BrowserView* browser_view) {
  if (!g_building_little)
    return;
  browser_view->SetProperty(kLittleWindow, true);
  toolbar->SetBackground(nullptr);
  auto* header = toolbar->AddChildView(
      std::make_unique<LittleHeader>(browser_view->browser()));
  toolbar->SetLayoutManager(std::make_unique<LittleToolbarLayout>(header));
}

}  // namespace mori
