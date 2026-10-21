#include "chrome/browser/ui/views/mori/mori_cheatsheet.h"

#include "build/build_config.h"
#include <memory>
#include <string>
#include <vector>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/strings/string_util.h"
#include "base/task/single_thread_task_runner.h"
#include "cc/paint/paint_flags.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_command_ids.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "ui/base/accelerators/accelerator.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/color/color_provider.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/font_list.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/background.h"
#include "ui/views/border.h"
#include "ui/views/controls/label.h"
#include "ui/views/controls/textfield/textfield.h"
#include "ui/views/controls/textfield/textfield_controller.h"
#include "ui/views/focus/focus_manager.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/view.h"

namespace mori {
namespace {

constexpr int kWidth = 760;
constexpr int kColumnGap = 24;
constexpr int kColumnWidth = (kWidth - 40 - kColumnGap) / 2;
constexpr auto kShadowInsets = gfx::Insets::TLBR(24, 40, 56, 40);

struct Entry {
  const char16_t* label;
  int command_id;
  // Shown instead of a single binding (e.g. a range of keys).
  const char16_t* keys = nullptr;
};

struct Group {
  const char16_t* title;
  std::vector<Entry> entries;
};

// Mori 3's groups (crates/mori/src/actions.rs, its descriptions).
std::vector<Group> Groups() {
  return {
      {u"Tabs",
       {{u"New tab", IDC_NEW_TAB},
        {u"Close tab", IDC_CLOSE_TAB},
        {u"Reopen closed tab", IDC_RESTORE_TAB},
        {u"Recent tabs", kCommandSwitchRecentTab},
        {u"Next tab", IDC_SELECT_NEXT_TAB},
        {u"Previous tab", IDC_SELECT_PREVIOUS_TAB},
        {u"Go to tab 1–8", 0,
#if BUILDFLAG(IS_MAC)
         u"⌘1–8"},
#elif BUILDFLAG(IS_WIN)
         u"Ctrl+1–8"},
#else
         u"Alt+1–8, Ctrl+1–8"},
#endif
        {u"Go to last tab", IDC_SELECT_LAST_TAB},
        {u"Add to or remove from Favorites", kCommandTogglePin},
        {u"Duplicate tab", IDC_DUPLICATE_TAB},
        {u"Copy URL", IDC_COPY_URL},
        {u"Bookmark page", IDC_BOOKMARK_THIS_TAB}}},
      {u"Getting around",
       {{u"Command bar", IDC_FOCUS_LOCATION},
        {u"Back", IDC_BACK},
        {u"Forward", IDC_FORWARD},
        {u"Reload", IDC_RELOAD},
        {u"Reload, skipping the cache", IDC_RELOAD_BYPASSING_CACHE},
        {u"Stop", IDC_STOP},
        {u"Home page", IDC_HOME},
        {u"Peek a link", 0,
#if BUILDFLAG(IS_MAC)
         u"⌥Click"},
#else
         u"Alt+Click"},
#endif
        {u"Peek link on clipboard", kCommandPeekClipboard}}},
      {u"Page",
       {{u"Find in page", IDC_FIND},
        {u"Find next", IDC_FIND_NEXT},
        {u"Find previous", IDC_FIND_PREVIOUS},
        {u"Zoom in", IDC_ZOOM_PLUS},
        {u"Zoom out", IDC_ZOOM_MINUS},
        {u"Actual size", IDC_ZOOM_NORMAL},
        {u"Reader view", IDC_SHOW_READING_MODE_SIDE_PANEL},
        {u"Print", IDC_PRINT},
        {u"Capture", IDC_SHARING_HUB_SCREENSHOT},
        {u"Boost this site", kCommandBoostSite},
        {u"Developer tools", IDC_DEV_TOOLS},
        {u"View source", IDC_VIEW_SOURCE}}},
      {u"Window",
       {{u"New window", IDC_NEW_WINDOW},
        {u"New private window", IDC_NEW_INCOGNITO_WINDOW},
        {u"Close window", IDC_CLOSE_WINDOW},
        {u"Show or hide sidebar", IDC_TOGGLE_VERTICAL_TABS},
        {u"Assistant", kCommandToggleAssistant},
        {u"Full screen", IDC_FULLSCREEN},
        {u"Quit", IDC_EXIT}}},
      {u"Places",
       {{u"Settings", IDC_OPTIONS},
        {u"History & bookmarks", IDC_SHOW_HISTORY},
        {u"Downloads", IDC_SHOW_DOWNLOADS},
        {u"Bookmark manager", IDC_SHOW_BOOKMARK_MANAGER},
        {u"Sleep background tabs", kCommandSleepBackgroundTabs},
        {u"Keyboard shortcuts", kCommandShowShortcuts}}},
  };
}

bool IsDark(const views::View* view) {
  return color_utils::IsDark(
      view->GetColorProvider()->GetColor(ui::kColorFrameActive));
}

gfx::FontList Font(int size, gfx::Font::Weight weight) {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(size - base.GetFontSize())
      .DeriveWithWeight(weight);
}

// A key cap: the shortcut in a small rounded chip.
class KeyCap : public views::Label {
  METADATA_HEADER(KeyCap, views::Label)

 public:
  explicit KeyCap(const std::u16string& text) : views::Label(text) {
    SetFontList(Font(11, gfx::Font::Weight::MEDIUM));
    SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(2, 6)));
  }

  void OnThemeChanged() override {
    views::Label::OnThemeChanged();
    const bool dark = IsDark(this);
    SetEnabledColor(dark ? SkColorSetRGB(0xC8, 0xC8, 0xC9)
                         : SkColorSetRGB(0x45, 0x47, 0x4A));
    SetBackground(views::CreateRoundedRectBackground(
        SkColorSetA(dark ? SK_ColorWHITE : SK_ColorBLACK, 0x14), 4));
  }
};

BEGIN_METADATA(KeyCap)
END_METADATA

class SearchField : public views::Textfield {
  METADATA_HEADER(SearchField, views::Textfield)

 public:
  explicit SearchField(base::RepeatingClosure on_blur)
      : on_blur_(std::move(on_blur)) {}
  void OnBlur() override {
    views::Textfield::OnBlur();
    on_blur_.Run();
  }

 private:
  base::RepeatingClosure on_blur_;
};

BEGIN_METADATA(SearchField)
END_METADATA

class CheatsheetPanel : public views::View, public views::TextfieldController {
  METADATA_HEADER(CheatsheetPanel, views::View)

 public:
  explicit CheatsheetPanel(BrowserView* browser_view)
      : browser_view_(browser_view) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical,
        kShadowInsets + gfx::Insets::VH(16, 20), 12));
    auto* title = AddChildView(std::make_unique<views::Label>(
        u"Keyboard Shortcuts"));
    title->SetFontList(Font(15, gfx::Font::Weight::SEMIBOLD));
    title->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    title_ = title;
    field_ = AddChildView(std::make_unique<SearchField>(base::BindRepeating(
        &CheatsheetPanel::OnBlurred, weak_factory_.GetWeakPtr())));
    field_->SetController(this);
    field_->SetPlaceholderText(u"Search shortcuts");
    field_->SetFontList(Font(14, gfx::Font::Weight::NORMAL));
    field_->SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(6, 10)));
    field_->GetViewAccessibility().SetName(u"Search shortcuts");
    columns_ = AddChildView(std::make_unique<views::View>());
    auto* columns_layout =
        columns_->SetLayoutManager(std::make_unique<views::BoxLayout>(
            views::BoxLayout::Orientation::kHorizontal, gfx::Insets(),
            kColumnGap));
    columns_layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kStart);
  }

  void Open() {
    field_->SetText(u"");
    Rebuild();
    field_->RequestFocus();
  }

  void set_on_close(base::RepeatingClosure on_close) {
    on_close_ = std::move(on_close);
  }
  // Called when a search changes what the panel holds, to fit it again.
  void set_on_resize(base::RepeatingClosure on_resize) {
    on_resize_ = std::move(on_resize);
  }

  // views::TextfieldController:
  void ContentsChanged(views::Textfield* sender,
                       const std::u16string& contents) override {
    Rebuild();
  }
  bool HandleKeyEvent(views::Textfield* sender,
                      const ui::KeyEvent& event) override {
    if (event.type() == ui::EventType::kKeyPressed &&
        event.key_code() == ui::VKEY_ESCAPE) {
      on_close_.Run();
      return true;
    }
    return false;
  }

  void OnThemeChanged() override {
    views::View::OnThemeChanged();
    const bool dark = IsDark(this);
    title_->SetEnabledColor(dark ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                                 : SkColorSetRGB(0x1D, 0x1F, 0x22));
    field_->SetBackground(views::CreateRoundedRectBackground(
        SkColorSetA(dark ? SK_ColorWHITE : SK_ColorBLACK, 0x10), 8));
    field_->SetTextColorId(kColorOmniboxText);
    Rebuild();
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    gfx::Rect popover = GetLocalBounds();
    popover.Inset(kShadowInsets);
    PaintPopover(this, canvas, popover);
  }

 private:
  void OnBlurred() {
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&CheatsheetPanel::CloseIfUnfocused,
                                  weak_factory_.GetWeakPtr()));
  }

  void CloseIfUnfocused() {
    views::FocusManager* focus = GetFocusManager();
    if (focus && focus->GetFocusedView() && !field_->HasFocus()) {
      on_close_.Run();
    }
  }

  std::u16string KeysFor(const Entry& entry) const {
    if (entry.keys) {
      return entry.keys;
    }
    ui::Accelerator accelerator;
    return browser_view_->GetAccelerator(entry.command_id, &accelerator)
               ? accelerator.GetShortcutText()
               : std::u16string();
  }

  // Two balanced columns of the groups that match the search.
  void Rebuild() {
    if (!GetColorProvider()) {
      return;
    }
    columns_->RemoveAllChildViews();
    const std::u16string query = base::ToLowerASCII(field_->GetText());
    const bool dark = IsDark(this);
    std::vector<std::unique_ptr<views::View>> cards;
    int total_rows = 0;
    std::vector<int> card_rows;
    for (const Group& group : Groups()) {
      auto card = std::make_unique<views::View>();
      card->SetLayoutManager(std::make_unique<views::BoxLayout>(
          views::BoxLayout::Orientation::kVertical, gfx::Insets(), 4));
      auto* heading = card->AddChildView(std::make_unique<views::Label>(
          group.title));
      heading->SetFontList(Font(12, gfx::Font::Weight::BOLD));
      heading->SetHorizontalAlignment(gfx::ALIGN_LEFT);
      heading->SetEnabledColor(dark ? SkColorSetRGB(0xA0, 0xA1, 0xA2)
                                    : SkColorSetRGB(0x5A, 0x5C, 0x5E));
      int rows = 0;
      for (const Entry& entry : group.entries) {
        const std::u16string keys = KeysFor(entry);
        if (keys.empty() ||
            (!query.empty() &&
             base::ToLowerASCII(std::u16string(entry.label)).find(query) ==
                 std::u16string::npos &&
             base::ToLowerASCII(keys).find(query) == std::u16string::npos)) {
          continue;
        }
        auto* row = card->AddChildView(std::make_unique<views::View>());
        auto* layout = row->SetLayoutManager(std::make_unique<views::BoxLayout>(
            views::BoxLayout::Orientation::kHorizontal, gfx::Insets(), 8));
        layout->set_cross_axis_alignment(
            views::BoxLayout::CrossAxisAlignment::kCenter);
        auto* label = row->AddChildView(std::make_unique<views::Label>(
            entry.label));
        label->SetFontList(Font(13, gfx::Font::Weight::NORMAL));
        label->SetHorizontalAlignment(gfx::ALIGN_LEFT);
        label->SetEnabledColor(dark ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                                    : SkColorSetRGB(0x1D, 0x1F, 0x22));
        layout->SetFlexForView(label, 1);
        row->AddChildView(std::make_unique<KeyCap>(keys));
        row->SetPreferredSize(gfx::Size(kColumnWidth, 24));
        ++rows;
      }
      if (rows == 0) {
        continue;
      }
      card_rows.push_back(rows + 2);
      total_rows += rows + 2;
      cards.push_back(std::move(card));
    }
    auto* left = columns_->AddChildView(std::make_unique<views::View>());
    auto* right = columns_->AddChildView(std::make_unique<views::View>());
    for (views::View* column : {left, right}) {
      column->SetLayoutManager(std::make_unique<views::BoxLayout>(
          views::BoxLayout::Orientation::kVertical, gfx::Insets(), 14));
    }
    int placed = 0;
    for (size_t i = 0; i < cards.size(); ++i) {
      (placed < total_rows / 2 ? left : right)->AddChildView(std::move(cards[i]));
      placed += card_rows[i];
    }
    PreferredSizeChanged();
    if (on_resize_) {
      on_resize_.Run();
    }
  }

  raw_ptr<BrowserView> browser_view_;
  raw_ptr<views::Label> title_ = nullptr;
  raw_ptr<SearchField> field_ = nullptr;
  raw_ptr<views::View> columns_ = nullptr;
  base::RepeatingClosure on_close_;
  base::RepeatingClosure on_resize_;
  base::WeakPtrFactory<CheatsheetPanel> weak_factory_{this};
};

BEGIN_METADATA(CheatsheetPanel)
END_METADATA

class Cheatsheet {
 public:
  explicit Cheatsheet(BrowserView* browser_view)
      : browser_view_(browser_view) {
    auto panel = std::make_unique<CheatsheetPanel>(browser_view);
    panel->set_on_close(base::BindRepeating(&Cheatsheet::Close,
                                            base::Unretained(this)));
    panel->set_on_resize(base::BindRepeating(&Cheatsheet::Fit,
                                             base::Unretained(this)));
    overlay_ = std::make_unique<Overlay>(browser_view, std::move(panel),
                                         "MoriCheatsheet");
  }

  void Toggle() {
    if (overlay_->IsVisible()) {
      Close();
      return;
    }
    auto* panel = static_cast<CheatsheetPanel*>(overlay_->contents());
    if (!panel) {
      return;
    }
    showing_ = true;
    Fit();
    panel->Open();
  }

  // Sized to what the panel holds, as high as the window allows.
  void Fit() {
    // The panel's first theme pass comes before `overlay_` is set.
    if (!showing_ || !overlay_) {
      return;
    }
    auto* panel = static_cast<CheatsheetPanel*>(overlay_->contents());
    if (!panel) {
      return;
    }
    const gfx::Rect area = browser_view_->GetLocalBounds();
    const int width = kWidth + kShadowInsets.width();
    const int top = std::max(16, area.height() / 8);
    const int height =
        std::min(panel->GetHeightForWidth(width),
                 area.height() - top - 16 + kShadowInsets.height());
    overlay_->Show(gfx::Rect(area.x() + (area.width() - width) / 2,
                             area.y() + top - kShadowInsets.top(), width,
                             height));
  }

  void Close() {
    showing_ = false;
    overlay_->Hide();
  }

 private:
  raw_ptr<BrowserView> browser_view_;
  bool showing_ = false;
  std::unique_ptr<Overlay> overlay_;
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::Cheatsheet*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(Cheatsheet, kCheatsheetKey)
}  // namespace

void ToggleCheatsheet(BrowserWindowInterface* browser) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget()) {
    return;
  }
  Cheatsheet* sheet = browser_view->GetProperty(kCheatsheetKey);
  if (!sheet) {
    sheet = browser_view->SetProperty(
        kCheatsheetKey, std::make_unique<Cheatsheet>(browser_view));
  }
  sheet->Toggle();
}

}  // namespace mori
