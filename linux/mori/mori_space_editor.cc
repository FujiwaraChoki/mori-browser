#include "chrome/browser/ui/views/mori/mori_space_editor.h"

#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/tabs/tab_group_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "chrome/browser/ui/views/mori/mori_space_themes.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "components/tab_groups/tab_group_visual_data.h"
#include "components/tabs/public/tab_group.h"
#include "third_party/skia/include/core/SkPoint.h"
#include "cc/paint/paint_shader.h"
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
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/button/md_text_button.h"
#include "ui/views/controls/label.h"
#include "ui/views/controls/textfield/textfield.h"
#include "ui/views/controls/textfield/textfield_controller.h"
#include "ui/views/focus/focus_manager.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/layout/box_layout_view.h"
#include "ui/views/layout/table_layout_view.h"
#include "ui/views/view_class_properties.h"

namespace mori {
namespace {

constexpr int kWidth = 360;
constexpr auto kShadowInsets = gfx::Insets::TLBR(24, 40, 56, 40);
constexpr int kCell = 34;
constexpr int kColumns = 8;

// The symbols offered, after the plain dot. Any emoji works too.
constexpr const char* kSymbols[] = {
    "layers",  "briefcase",   "house",     "book",         "code",
    "music",   "heart",       "coffee",    "gamepad-2",    "graduation-cap",
    "plane",   "shopping-bag", "leaf",     "film",         "folder",
    "rocket",  "flame",       "sun",       "moon",         "users",
    "dumbbell", "newspaper",  "wallet",    "palette",      "camera",
    "star",    "sparkles",    "globe",     "zap",
};

bool IsDark(const views::View* view) {
  return view->GetColorProvider() &&
         color_utils::IsDark(
             view->GetColorProvider()->GetColor(ui::kColorFrameActive));
}

SkColor Foreground(const views::View* view) {
  return IsDark(view) ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                      : SkColorSetRGB(0x1D, 0x1F, 0x22);
}

SkColor Muted(const views::View* view) {
  return IsDark(view) ? SkColorSetRGB(0xA0, 0xA1, 0xA2)
                      : SkColorSetRGB(0x5A, 0x5C, 0x5E);
}

gfx::FontList Font(int size, gfx::Font::Weight weight) {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(size - base.GetFontSize())
      .DeriveWithWeight(weight);
}

// A square choice: a symbol, the plain dot, a gradient or none.
class Choice : public views::Button {
  METADATA_HEADER(Choice, views::Button)

 public:
  enum class Kind { kSymbol, kDot, kGradient, kNoGradient };

  Choice(PressedCallback callback, Kind kind, std::string symbol,
         std::pair<SkColor, SkColor> colors, std::u16string name)
      : views::Button(std::move(callback)),
        kind_(kind),
        symbol_(std::move(symbol)),
        colors_(colors) {
    SetPreferredSize(gfx::Size(kCell, kCell));
    SetAccessibleName(name);
  }

  void SetSelected(bool selected) {
    selected_ = selected;
    SchedulePaint();
  }

  void PaintButtonContents(gfx::Canvas* canvas) override {
    const gfx::RectF bounds(GetLocalBounds());
    const bool dark = IsDark(this);
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    const bool hovered =
        GetState() == STATE_HOVERED || GetState() == STATE_PRESSED;
    if (selected_ || hovered) {
      flags.setColor(SkColorSetA(dark ? SK_ColorWHITE : SK_ColorBLACK,
                                 selected_ ? 0x2A : 0x14));
      canvas->DrawRoundRect(bounds, 8, flags);
    }
    const gfx::PointF center = bounds.CenterPoint();
    switch (kind_) {
      case Kind::kSymbol:
        PaintSpaceSymbol(canvas, symbol_, GetLocalBounds(), 16,
                         selected_ ? Foreground(this) : Muted(this));
        break;
      case Kind::kDot:
        flags.setColor(selected_ ? Foreground(this) : Muted(this));
        canvas->DrawCircle(center, 4, flags);
        break;
      case Kind::kGradient: {
        const SkPoint points[2] = {SkPoint::Make(center.x() - 10, center.y() - 10),
                                   SkPoint::Make(center.x() + 10, center.y() + 10)};
        const SkColor4f colors[2] = {SkColor4f::FromColor(colors_.first),
                                     SkColor4f::FromColor(colors_.second)};
        // Opaque, or the highlight's alpha would fade the gradient.
        flags.setColor(SK_ColorBLACK);
        flags.setShader(cc::PaintShader::MakeLinearGradient(
            points, colors, nullptr, 2, SkTileMode::kClamp));
        canvas->DrawCircle(center, 11, flags);
        break;
      }
      case Kind::kNoGradient: {
        flags.setStyle(cc::PaintFlags::kStroke_Style);
        flags.setStrokeWidth(1.5f);
        flags.setColor(Muted(this));
        canvas->DrawCircle(center, 10, flags);
        canvas->DrawLine(gfx::PointF(center.x() - 7, center.y() + 7),
                         gfx::PointF(center.x() + 7, center.y() - 7), flags);
        break;
      }
    }
  }

  void StateChanged(ButtonState old_state) override { SchedulePaint(); }

  Kind kind() const { return kind_; }
  const std::string& symbol() const { return symbol_; }
  std::pair<SkColor, SkColor> colors() const { return colors_; }

 private:
  Kind kind_;
  std::string symbol_;
  std::pair<SkColor, SkColor> colors_;
  bool selected_ = false;
};

BEGIN_METADATA(Choice)
END_METADATA

class EditorField : public views::Textfield {
  METADATA_HEADER(EditorField, views::Textfield)

 public:
  explicit EditorField(base::RepeatingClosure on_blur)
      : on_blur_(std::move(on_blur)) {
    SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(6, 10)));
  }
  void OnThemeChanged() override {
    views::Textfield::OnThemeChanged();
    SetBackground(views::CreateRoundedRectBackground(
        SkColorSetA(IsDark(this) ? SK_ColorWHITE : SK_ColorBLACK, 0x10), 8));
    SetTextColorId(kColorOmniboxText);
  }
  void OnBlur() override {
    views::Textfield::OnBlur();
    on_blur_.Run();
  }

 private:
  base::RepeatingClosure on_blur_;
};

BEGIN_METADATA(EditorField)
END_METADATA

class SpaceEditorView : public views::View, public views::TextfieldController {
  METADATA_HEADER(SpaceEditorView, views::View)

 public:
  explicit SpaceEditorView(base::RepeatingClosure close)
      : close_(std::move(close)) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical,
        kShadowInsets + gfx::Insets::VH(16, 18), 8));
    title_ = AddChildView(std::make_unique<views::Label>(u"Edit Space"));
    title_->SetFontList(Font(15, gfx::Font::Weight::SEMIBOLD));
    title_->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    name_ = AddChildView(std::make_unique<EditorField>(base::BindRepeating(
        &SpaceEditorView::OnFieldBlurred, weak_factory_.GetWeakPtr())));
    name_->set_controller(this);
    name_->SetFontList(Font(14, gfx::Font::Weight::NORMAL));
    name_->SetPlaceholderText(u"Space name");
    name_->GetViewAccessibility().SetName(u"Space name");

    headings_.push_back(AddHeading(u"Icon"));
    auto* grid = AddChildView(std::make_unique<views::View>());
    grid->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical, gfx::Insets(), 0));
    views::BoxLayoutView* row = nullptr;
    int column = 0;
    auto add_symbol = [&](Choice::Kind kind, std::string symbol,
                          std::u16string name) {
      if (!row || column == kColumns) {
        row = grid->AddChildView(std::make_unique<views::BoxLayoutView>());
        column = 0;
      }
      ++column;
      auto* choice = row->AddChildView(std::make_unique<Choice>(
          base::BindRepeating(&SpaceEditorView::PickSymbol,
                              base::Unretained(this), symbol),
          kind, symbol, std::pair<SkColor, SkColor>(), name));
      symbols_.push_back(choice);
    };
    add_symbol(Choice::Kind::kDot, "", u"Dot");
    for (const char* symbol : kSymbols) {
      add_symbol(Choice::Kind::kSymbol, symbol, base::UTF8ToUTF16(symbol));
    }
    emoji_ = AddChildView(std::make_unique<EditorField>(base::BindRepeating(
        &SpaceEditorView::OnFieldBlurred, weak_factory_.GetWeakPtr())));
    emoji_->set_controller(this);
    emoji_->SetFontList(Font(14, gfx::Font::Weight::NORMAL));
    emoji_->SetPlaceholderText(u"Or type an emoji");
    emoji_->GetViewAccessibility().SetName(u"Emoji");

    headings_.push_back(AddHeading(u"Theme"));
    auto* themes = AddChildView(std::make_unique<views::BoxLayoutView>());
    auto add_theme = [&](Choice::Kind kind,
                         std::pair<SkColor, SkColor> colors,
                         std::u16string name) {
      auto* choice = themes->AddChildView(std::make_unique<Choice>(
          base::BindRepeating(&SpaceEditorView::PickTheme,
                              base::Unretained(this), kind, colors),
          kind, "", colors, name));
      themes_.push_back(choice);
    };
    add_theme(Choice::Kind::kNoGradient, {}, u"No theme");
    for (const auto& colors : kSpaceGradients) {
      add_theme(Choice::Kind::kGradient, colors, u"Gradient");
    }

    auto* buttons = AddChildView(std::make_unique<views::BoxLayoutView>());
    buttons->SetBetweenChildSpacing(8);
    buttons->SetProperty(views::kMarginsKey, gfx::Insets::TLBR(8, 0, 0, 0));
    delete_ = buttons->AddChildView(std::make_unique<views::MdTextButton>(
        base::BindRepeating(&SpaceEditorView::Delete, base::Unretained(this)),
        u"Delete Space"));
    auto* spacer = buttons->AddChildView(std::make_unique<views::View>());
    buttons->SetFlexForView(spacer, 1);
    auto* done = buttons->AddChildView(std::make_unique<views::MdTextButton>(
        close_, u"Done"));
    done->SetStyle(ui::ButtonStyle::kProminent);
  }

  void Open(BrowserWindowInterface* browser, tab_groups::TabGroupId space) {
    browser_ = browser;
    space_ = space;
    TabStripModel* model = browser->GetTabStripModel();
    name_->SetText(
        model->group_model()->GetTabGroup(space)->visual_data()->title());
    name_->SelectAll(false);
    theme_ = GetSpaceTheme(browser, space);
    emoji_->SetText(IsEmojiSymbol(theme_.symbol)
                        ? base::UTF8ToUTF16(theme_.symbol)
                        : std::u16string());
    delete_->SetVisible(model->group_model()->ListTabGroups().size() > 1);
    UpdateSelection();
  }

  void FocusName() { name_->RequestFocus(); }

  // views::TextfieldController:
  void ContentsChanged(views::Textfield* sender,
                       const std::u16string& contents) override {
    if (!browser_ || !space_) {
      return;
    }
    if (sender == name_) {
      TabStripModel* model = browser_->GetTabStripModel();
      if (!model->group_model()->ContainsTabGroup(*space_)) {
        return;
      }
      const tab_groups::TabGroupVisualData* visuals =
          model->group_model()->GetTabGroup(*space_)->visual_data();
      model->ChangeTabGroupVisuals(
          *space_, tab_groups::TabGroupVisualData(contents, visuals->color()));
    } else if (sender == emoji_) {
      const std::string text =
          base::UTF16ToUTF8(base::TrimWhitespace(contents, base::TRIM_ALL));
      if (IsEmojiSymbol(text)) {
        PickSymbol(text);
      }
    }
  }

  bool HandleKeyEvent(views::Textfield* sender,
                      const ui::KeyEvent& event) override {
    if (event.type() == ui::EventType::kKeyPressed &&
        (event.key_code() == ui::VKEY_ESCAPE ||
         event.key_code() == ui::VKEY_RETURN)) {
      close_.Run();
      return true;
    }
    return false;
  }

  void OnThemeChanged() override {
    views::View::OnThemeChanged();
    title_->SetEnabledColor(Foreground(this));
    for (views::Label* heading : headings_) {
      heading->SetEnabledColor(Muted(this));
    }
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    gfx::Rect popover = GetLocalBounds();
    popover.Inset(kShadowInsets);
    PaintPopover(this, canvas, popover);
  }

 private:
  views::Label* AddHeading(const std::u16string& text) {
    auto* label = AddChildView(std::make_unique<views::Label>(text));
    label->SetFontList(Font(12, gfx::Font::Weight::BOLD));
    label->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    label->SetProperty(views::kMarginsKey, gfx::Insets::TLBR(6, 0, 0, 0));
    return label;
  }

  void PickSymbol(std::string symbol) {
    theme_.symbol = std::move(symbol);
    if (!IsEmojiSymbol(theme_.symbol)) {
      emoji_->SetText(u"");
    }
    Apply();
  }

  void PickTheme(Choice::Kind kind, std::pair<SkColor, SkColor> colors) {
    theme_.colors = kind == Choice::Kind::kGradient
                        ? std::optional<std::pair<SkColor, SkColor>>(colors)
                        : std::nullopt;
    Apply();
  }

  void Apply() {
    if (browser_ && space_ &&
        browser_->GetTabStripModel()->group_model()->ContainsTabGroup(
            *space_)) {
      SetSpaceTheme(browser_, *space_, theme_);
    }
    UpdateSelection();
  }

  void UpdateSelection() {
    for (Choice* choice : symbols_) {
      choice->SetSelected(choice->symbol() == theme_.symbol);
    }
    for (Choice* choice : themes_) {
      choice->SetSelected(choice->kind() == Choice::Kind::kGradient
                              ? theme_.colors == choice->colors()
                              : !theme_.colors);
    }
  }

  void Delete() {
    BrowserWindowInterface* browser = browser_;
    close_.Run();
    if (browser) {
      DeleteSpace(browser);
    }
  }

  void OnFieldBlurred() {
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&SpaceEditorView::CloseIfFocusLeft,
                                  weak_factory_.GetWeakPtr()));
  }

  // Closes when focus moved elsewhere in the window, not between its own
  // fields or buttons, and not when the window lost focus.
  void CloseIfFocusLeft() {
    views::FocusManager* focus = GetFocusManager();
    views::View* focused = focus ? focus->GetFocusedView() : nullptr;
    if (focused && !Contains(focused)) {
      close_.Run();
    }
  }

  base::RepeatingClosure close_;
  raw_ptr<BrowserWindowInterface> browser_ = nullptr;
  std::optional<tab_groups::TabGroupId> space_;
  SpaceTheme theme_;
  raw_ptr<views::Label> title_ = nullptr;
  raw_ptr<EditorField> name_ = nullptr;
  raw_ptr<EditorField> emoji_ = nullptr;
  raw_ptr<views::MdTextButton> delete_ = nullptr;
  std::vector<raw_ptr<views::Label>> headings_;
  std::vector<raw_ptr<Choice>> symbols_;
  std::vector<raw_ptr<Choice>> themes_;
  base::WeakPtrFactory<SpaceEditorView> weak_factory_{this};
};

BEGIN_METADATA(SpaceEditorView)
END_METADATA

class SpaceEditor {
 public:
  explicit SpaceEditor(BrowserView* browser_view)
      : browser_view_(browser_view) {
    overlay_ = std::make_unique<Overlay>(
        browser_view,
        std::make_unique<SpaceEditorView>(
            base::BindRepeating(&SpaceEditor::Close, base::Unretained(this))),
        "MoriSpaceEditor");
  }

  void Open() {
    BrowserWindowInterface* browser = browser_view_->browser();
    const std::optional<tab_groups::TabGroupId> space =
        GetCurrentSpace(browser);
    auto* view = static_cast<SpaceEditorView*>(overlay_->contents());
    if (!space || !view) {
      return;
    }
    view->Open(browser, *space);
    // Beside the sidebar, near its top, as Mori 3's popover.
    const gfx::Rect area = browser_view_->GetLocalBounds();
    const int width = kWidth + kShadowInsets.width();
    const int height = std::min(view->GetHeightForWidth(width),
                                area.height() + kShadowInsets.height());
    views::View* contents = browser_view_->contents_container();
    const int left =
        contents ? views::View::ConvertRectToTarget(
                       contents, browser_view_, contents->GetLocalBounds())
                       .x()
                 : area.x();
    overlay_->Show(gfx::Rect(left + 8 - kShadowInsets.left(),
                             area.y() + 48 - kShadowInsets.top(), width,
                             height));
    view->FocusName();
  }

  void Close() {
    overlay_->Hide();
    if (content::WebContents* contents =
            browser_view_->browser()->GetTabStripModel()
                ->GetActiveWebContents()) {
      contents->Focus();
    }
  }

 private:
  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<Overlay> overlay_;
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::SpaceEditor*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(SpaceEditor, kSpaceEditorKey)
}  // namespace

void EditSpace(BrowserWindowInterface* browser) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget()) {
    return;
  }
  SpaceEditor* editor = browser_view->GetProperty(kSpaceEditorKey);
  if (!editor) {
    editor = browser_view->SetProperty(
        kSpaceEditorKey, std::make_unique<SpaceEditor>(browser_view));
  }
  editor->Open();
}

}  // namespace mori
