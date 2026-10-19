#include "chrome/browser/ui/views/mori/mori_settings.h"

#include <algorithm>
#include <limits>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include "base/files/file_path.h"
#include "base/functional/bind.h"
#include "base/functional/callback_helpers.h"
#include "base/i18n/case_conversion.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/scoped_multi_source_observation.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/sequenced_task_runner.h"
#include "base/task/thread_pool.h"
#include "cc/paint/paint_flags.h"
#include "cc/paint/paint_shader.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/browser_process.h"
#include "chrome/browser/prefs/session_startup_pref.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/search_engines/template_url_service_factory.h"
#include "chrome/browser/shell_integration.h"
#include "chrome/browser/themes/theme_service.h"
#include "chrome/browser/themes/theme_service_factory.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_tabstrip.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/profiles/profile_picker.h"
#include "chrome/browser/ui/tabs/tab_group_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_assistant.h"
#include "chrome/browser/ui/views/mori/mori_boosts.h"
#include "chrome/browser/ui/views/mori/mori_codex.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_import.h"
#include "chrome/browser/ui/views/mori/mori_layout.h"
#include "chrome/browser/ui/views/mori/mori_little.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "chrome/browser/ui/views/mori/mori_routes.h"
#include "chrome/browser/ui/views/mori/mori_space_editor.h"
#include "chrome/browser/ui/views/mori/mori_space_themes.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "chrome/common/pref_names.h"
#include "components/content_settings/core/browser/cookie_settings.h"
#include "components/content_settings/core/common/pref_names.h"
#include "components/password_manager/core/common/password_manager_pref_names.h"
#include "components/performance_manager/public/user_tuning/prefs.h"
#include "components/prefs/pref_service.h"
#include "components/search_engines/template_url.h"
#include "components/search_engines/template_url_service.h"
#include "components/tab_groups/tab_group_visual_data.h"
#include "components/tabs/public/tab_group.h"
#include "components/version_info/version_info.h"
#include "content/public/browser/web_contents.h"
#include "third_party/skia/include/core/SkPoint.h"
#include "ui/aura/window.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/color/color_provider.h"
#include "ui/events/event.h"
#include "ui/events/event_observer.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/font_list.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/paint_vector_icon.h"
#include "ui/gfx/text_utils.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/border.h"
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/label.h"
#include "ui/views/controls/scroll_view.h"
#include "ui/views/event_monitor.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/layout/box_layout_view.h"
#include "ui/views/view_class_properties.h"
#include "ui/views/view_observer.h"
#include "ui/views/view_utils.h"
#include "ui/views/widget/widget.h"
#include "url/gurl.h"

namespace mori {
namespace {

// Mori 3's sizes (settings.rs).
constexpr int kHeaderHeight = 56;
constexpr int kRailWidth = 200;
constexpr int kContentMaxWidth = 620;
constexpr int kLabelWidth = 150;
constexpr int kMaxSegments = 4;
constexpr SkColor kPrimary = SkColorSetRGB(0x29, 0x8E, 0xCD);

enum class Category {
  kGeneral,
  kAppearance,
  kTabs,
  kProfiles,
  kSites,
  kPrivacy,
  kPasswords,
  kAssistant,
  kImport,
  kAbout,
};

struct CategoryInfo {
  Category category;
  const char16_t* label;
  const char* icon;
};

constexpr CategoryInfo kCategories[] = {
    {Category::kGeneral, u"General", "settings"},
    {Category::kAppearance, u"Appearance", "palette"},
    {Category::kTabs, u"Tabs", "app-window"},
    {Category::kProfiles, u"Profiles", "users"},
    {Category::kSites, u"Sites", "globe"},
    {Category::kPrivacy, u"Privacy", "shield"},
    {Category::kPasswords, u"Passwords", "key-round"},
    {Category::kAssistant, u"Assistant", "sparkles"},
    {Category::kImport, u"Import", "import"},
    {Category::kAbout, u"About Mori", "info"},
};

// Mori 3's ARCHIVE_OPTIONS (hours).
constexpr std::pair<int, const char16_t*> kArchiveOptions[] = {
    {0, u"Never"},  {12, u"12 hr"},   {24, u"1 day"},
    {72, u"3 days"}, {168, u"1 week"}, {720, u"30 days"},
};

bool IsDark(const views::View* view) {
  return view->GetColorProvider() &&
         color_utils::IsDark(
             view->GetColorProvider()->GetColor(ui::kColorFrameActive));
}

SkColor Foreground(const views::View* view, SkAlpha alpha = 0xFF) {
  return SkColorSetA(IsDark(view) ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                                  : SkColorSetRGB(0x1D, 0x1F, 0x22),
                     alpha);
}

SkColor Muted(const views::View* view) {
  return IsDark(view) ? SkColorSetRGB(0xA0, 0xA1, 0xA2)
                      : SkColorSetRGB(0x5A, 0x5C, 0x5E);
}

gfx::FontList Font(int size, gfx::Font::Weight weight = gfx::Font::Weight::NORMAL) {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(size - base.GetFontSize())
      .DeriveWithWeight(weight);
}

// A label whose colour follows the theme: foreground or muted.
class ThemedLabel : public views::Label {
  METADATA_HEADER(ThemedLabel, views::Label)

 public:
  ThemedLabel(const std::u16string& text,
              int size,
              bool muted,
              gfx::Font::Weight weight = gfx::Font::Weight::NORMAL)
      : views::Label(text), muted_(muted) {
    SetFontList(Font(size, weight));
    SetHorizontalAlignment(gfx::ALIGN_LEFT);
    SetAutoColorReadabilityEnabled(false);
  }
  void OnThemeChanged() override {
    views::Label::OnThemeChanged();
    SetEnabledColor(muted_ ? Muted(this) : Foreground(this, 0xD9));
  }

 private:
  bool muted_;
};

BEGIN_METADATA(ThemedLabel)
END_METADATA

// A clickable surface painted by `paint` (choices, toggles, buttons).
class PaintedButton : public views::Button {
  METADATA_HEADER(PaintedButton, views::Button)

 public:
  using Painter =
      base::RepeatingCallback<void(views::Button*, gfx::Canvas*, bool hovered)>;

  PaintedButton(PressedCallback callback,
                gfx::Size size,
                Painter painter,
                const std::u16string& name)
      : views::Button(std::move(callback)), painter_(std::move(painter)) {
    SetPreferredSize(size);
    GetViewAccessibility().SetName(name);
  }

  void PaintButtonContents(gfx::Canvas* canvas) override {
    painter_.Run(this, canvas,
                 GetState() == STATE_HOVERED || GetState() == STATE_PRESSED);
  }
  void StateChanged(ButtonState old_state) override { SchedulePaint(); }

 private:
  Painter painter_;
};

BEGIN_METADATA(PaintedButton)
END_METADATA

int TextWidth(const std::u16string& text, int size) {
  return gfx::GetStringWidth(text, Font(size));
}

// Lays its children out in rows, wrapping (the choice pills).
class FlowView : public views::View {
  METADATA_HEADER(FlowView, views::View)

 public:
  explicit FlowView(int gap) : gap_(gap) {}

  gfx::Size CalculatePreferredSize(
      const views::SizeBounds& available_size) const override {
    const int width = available_size.width().is_bounded()
                          ? available_size.width().value()
                          : 100000;
    return gfx::Size(std::min(width, NaturalWidth()), Arrange(width, nullptr));
  }

  void Layout(PassKey) override { Arrange(width(), this); }

 private:
  int NaturalWidth() const {
    int width = 0;
    for (const views::View* child : children()) {
      width += child->GetPreferredSize().width() + gap_;
    }
    return std::max(0, width - gap_);
  }

  // Returns the height; places the children when `self` is given.
  int Arrange(int width, FlowView* self) const {
    int x = 0;
    int y = 0;
    int row_height = 0;
    for (views::View* child : children()) {
      const gfx::Size size = child->GetPreferredSize();
      if (x > 0 && x + size.width() > width) {
        x = 0;
        y += row_height + gap_;
        row_height = 0;
      }
      if (self) {
        child->SetBoundsRect(gfx::Rect(gfx::Point(x, y), size));
      }
      x += size.width() + gap_;
      row_height = std::max(row_height, size.height());
    }
    return y + row_height;
  }

  int gap_;
};

BEGIN_METADATA(FlowView)
END_METADATA

// Centres one child at up to kContentMaxWidth, with Mori 3's padding.
class CenterColumn : public views::View {
  METADATA_HEADER(CenterColumn, views::View)

 public:
  // The column was rebuilt.
  void ContentsChanged() { PreferredSizeChanged(); }

  gfx::Size CalculatePreferredSize(
      const views::SizeBounds& available_size) const override {
    const int width = available_size.width().is_bounded()
                          ? available_size.width().value()
                          : kContentMaxWidth + 56;
    return gfx::Size(width,
                     children().empty()
                         ? 0
                         : children()[0]->GetHeightForWidth(ColumnWidth(width)) +
                               48);
  }

  void Layout(PassKey) override {
    if (children().empty()) {
      return;
    }
    const int column = ColumnWidth(width());
    children()[0]->SetBounds((width() - column) / 2, 24, column,
                             children()[0]->GetHeightForWidth(column));
  }

 private:
  static int ColumnWidth(int width) {
    return std::max(200, std::min(kContentMaxWidth, width - 56));
  }
};

BEGIN_METADATA(CenterColumn)
END_METADATA

// A settings card: Mori 3's rounded, bordered group of fields.
class Card : public views::View {
  METADATA_HEADER(Card, views::View)

 public:
  Card() {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical, gfx::Insets(16), 12));
  }
  void OnPaintBackground(gfx::Canvas* canvas) override {
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    gfx::RectF bounds(GetLocalBounds());
    flags.setColor(IsDark(this) ? SkColorSetA(SK_ColorWHITE, 0x08)
                                : SK_ColorWHITE);
    canvas->DrawRoundRect(bounds, 8, flags);
    flags.setStyle(cc::PaintFlags::kStroke_Style);
    flags.setStrokeWidth(1);
    flags.setColor(Foreground(this, 0x1A));
    bounds.Inset(0.5f);
    canvas->DrawRoundRect(bounds, 7.5f, flags);
  }
};

BEGIN_METADATA(Card)
END_METADATA

// The sheet over the page: header, rail, and the category's sections.
class SettingsSheet : public views::View {
  METADATA_HEADER(SettingsSheet, views::View)

 public:
  SettingsSheet(BrowserView* browser_view, base::RepeatingClosure close)
      : browser_view_(browser_view), close_(std::move(close)) {
    // Header: back, "Settings", Done.
    header_ = AddChildView(std::make_unique<views::BoxLayoutView>());
    header_->SetInsideBorderInsets(gfx::Insets::VH(0, 16));
    header_->SetBetweenChildSpacing(8);
    header_->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kCenter);
    header_->AddChildView(std::make_unique<PaintedButton>(
        close_, gfx::Size(28, 28),
        base::BindRepeating([](views::Button* b, gfx::Canvas* canvas,
                               bool hovered) {
          if (hovered) {
            cc::PaintFlags flags;
            flags.setAntiAlias(true);
            flags.setColor(Foreground(b, 0x14));
            canvas->DrawRoundRect(gfx::RectF(b->GetLocalBounds()), 6, flags);
          }
          const gfx::ImageSkia icon = gfx::CreateVectorIcon(
              LucideIcon("arrow-left"), 16, Foreground(b));
          canvas->DrawImageInt(icon, (b->width() - icon.width()) / 2,
                               (b->height() - icon.height()) / 2);
        }),
        u"Back"));
    auto* title = header_->AddChildView(std::make_unique<ThemedLabel>(
        u"Settings", 15, false, gfx::Font::Weight::SEMIBOLD));
    header_->SetFlexForView(title, 1);
    header_->AddChildView(TextButton(u"Done", true, close_));

    rail_ = AddChildView(std::make_unique<views::BoxLayoutView>());
    rail_->SetOrientation(views::BoxLayout::Orientation::kVertical);
    rail_->SetInsideBorderInsets(gfx::Insets(12));
    rail_->SetBetweenChildSpacing(2);
    rail_->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kStretch);

    scroll_ = AddChildView(std::make_unique<views::ScrollView>());
    scroll_->SetBackgroundColor(std::nullopt);
    scroll_->SetHorizontalScrollBarMode(
        views::ScrollView::ScrollBarMode::kDisabled);
    scroll_->SetVerticalScrollBarMode(
        views::ScrollView::ScrollBarMode::kHiddenButEnabled);
    center_ = scroll_->SetContents(std::make_unique<CenterColumn>());
    // Sizes the column to the viewport's width and its own height.
    scroll_->ClipHeightTo(0, std::numeric_limits<int>::max());
  }

  void Open() {
    Rebuild();
    scroll_->ScrollToOffset(gfx::PointF());
  }

  void Layout(PassKey) override {
    header_->SetBounds(0, 0, width(), kHeaderHeight);
    rail_->SetBounds(0, kHeaderHeight, kRailWidth, height() - kHeaderHeight);
    scroll_->SetBounds(kRailWidth + 1, kHeaderHeight + 1,
                       width() - kRailWidth - 1, height() - kHeaderHeight - 1);
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    flags.setColor(IsDark(this) ? SkColorSetRGB(0x16, 0x16, 0x17)
                                : SkColorSetRGB(0xFA, 0xFA, 0xFA));
    canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), kContentCardRadius,
                          flags);
    flags.setColor(Foreground(this, 0x1A));
    canvas->FillRect(gfx::Rect(0, kHeaderHeight, width(), 1),
                     Foreground(this, 0x1A));
    canvas->FillRect(gfx::Rect(kRailWidth, kHeaderHeight, 1,
                               height() - kHeaderHeight),
                     Foreground(this, 0x14));
  }

 private:
  BrowserWindowInterface* browser() { return browser_view_->browser(); }
  Profile* profile() { return browser()->GetProfile(); }
  PrefService* prefs() { return profile()->GetPrefs(); }

  // Changing a setting rebuilds the page after the click that did it.
  base::RepeatingClosure Then(base::RepeatingClosure change) {
    return base::BindRepeating(
        [](base::WeakPtr<SettingsSheet> sheet, base::RepeatingClosure change) {
          change.Run();
          if (sheet) {
            sheet->PostRebuild();
          }
        },
        weak_factory_.GetWeakPtr(), std::move(change));
  }

  void PostRebuild() {
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&SettingsSheet::Rebuild, weak_factory_.GetWeakPtr()));
  }

  void Rebuild() {
    BuildRail();
    center_->RemoveAllChildViews();
    auto* column = center_->AddChildView(std::make_unique<views::BoxLayoutView>());
    column->SetOrientation(views::BoxLayout::Orientation::kVertical);
    column->SetBetweenChildSpacing(22);
    column->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kStretch);
    for (const CategoryInfo& info : kCategories) {
      if (info.category == category_) {
        column->AddChildView(std::make_unique<ThemedLabel>(
            info.label, 20, false, gfx::Font::Weight::SEMIBOLD));
      }
    }
    switch (category_) {
      case Category::kGeneral:
        BuildGeneral(column);
        break;
      case Category::kAppearance:
        BuildAppearance(column);
        break;
      case Category::kTabs:
        BuildTabs(column);
        break;
      case Category::kProfiles:
        BuildProfiles(column);
        break;
      case Category::kSites:
        BuildSites(column);
        break;
      case Category::kPrivacy:
        BuildPrivacy(column);
        break;
      case Category::kPasswords:
        BuildPasswords(column);
        break;
      case Category::kAssistant:
        BuildAssistant(column);
        break;
      case Category::kImport:
        BuildImport(column);
        break;
      case Category::kAbout:
        BuildAbout(column);
        break;
    }
    // The scroll view sizes its contents from their preferred size.
    center_->ContentsChanged();
    scroll_->InvalidateLayout();
    SchedulePaint();
  }

  void BuildRail() {
    rail_->RemoveAllChildViews();
    for (const CategoryInfo& info : kCategories) {
      const bool active = info.category == category_;
      const std::u16string label = info.label;
      const char* icon = info.icon;
      rail_->AddChildView(std::make_unique<PaintedButton>(
          base::BindRepeating(&SettingsSheet::Pick, base::Unretained(this),
                              info.category),
          gfx::Size(kRailWidth - 24, 32),
          base::BindRepeating(
              [](bool active, std::u16string label, const char* icon,
                 views::Button* b, gfx::Canvas* canvas, bool hovered) {
                cc::PaintFlags flags;
                flags.setAntiAlias(true);
                if (active || hovered) {
                  flags.setColor(Foreground(
                      b, active ? (IsDark(b) ? 0x1A : 0x12) : 0x0D));
                  canvas->DrawRoundRect(gfx::RectF(b->GetLocalBounds()), 6.4f,
                                        flags);
                }
                const gfx::ImageSkia image = gfx::CreateVectorIcon(
                    LucideIcon(icon), 15,
                    active ? Foreground(b) : Muted(b));
                canvas->DrawImageInt(image, 10,
                                     (b->height() - image.height()) / 2);
                canvas->DrawStringRect(
                    label,
                    Font(13, active ? gfx::Font::Weight::MEDIUM
                                    : gfx::Font::Weight::NORMAL),
                    active ? Foreground(b) : Foreground(b, 0xBF),
                    gfx::Rect(35, 0, b->width() - 35, b->height()));
              },
              active, label, icon),
          label));
    }
  }

  void Pick(Category category) {
    category_ = category;
    PostRebuild();
    scroll_->ScrollToOffset(gfx::PointF());
  }

  // ---- Building blocks (Mori 3's section, card, field, hint, choice,
  // toggle, text_button).

  Card* Section(views::View* column, const std::u16string& title) {
    auto* section = column->AddChildView(std::make_unique<views::BoxLayoutView>());
    section->SetOrientation(views::BoxLayout::Orientation::kVertical);
    section->SetBetweenChildSpacing(8);
    section->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kStretch);
    if (!title.empty()) {
      auto* label = section->AddChildView(std::make_unique<ThemedLabel>(
          base::i18n::ToUpper(title), 11, true, gfx::Font::Weight::SEMIBOLD));
      label->SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(0, 4)));
    }
    return section->AddChildView(std::make_unique<Card>());
  }

  void Field(views::View* card,
             const std::u16string& label,
             std::unique_ptr<views::View> control) {
    auto* row = card->AddChildView(std::make_unique<views::BoxLayoutView>());
    row->SetBetweenChildSpacing(12);
    row->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kCenter);
    row->SetMinimumCrossAxisSize(30);
    auto* name = row->AddChildView(std::make_unique<ThemedLabel>(label, 13, false));
    name->SetPreferredSize(gfx::Size(kLabelWidth, 20));
    name->SetElideBehavior(gfx::ELIDE_TAIL);
    auto* holder = row->AddChildView(std::make_unique<views::BoxLayoutView>());
    holder->SetMainAxisAlignment(views::BoxLayout::MainAxisAlignment::kStart);
    holder->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kCenter);
    row->SetFlexForView(holder, 1);
    views::View* added = holder->AddChildView(std::move(control));
    if (views::IsViewClass<FlowView>(added)) {
      holder->SetFlexForView(added, 1);
    }
  }

  void Hint(views::View* card, const std::u16string& text) {
    auto* hint = card->AddChildView(std::make_unique<ThemedLabel>(text, 11, true));
    hint->SetMultiLine(true);
  }

  // A segmented control for a few options, wrapping pills for more.
  std::unique_ptr<views::View> Choice(
      std::vector<std::pair<std::u16string, base::RepeatingClosure>> options,
      int selected) {
    const bool segmented = options.size() <= kMaxSegments;
    std::unique_ptr<views::View> container;
    if (segmented) {
      auto box = std::make_unique<SegmentBox>();
      container = std::move(box);
    } else {
      container = std::make_unique<FlowView>(6);
    }
    for (size_t i = 0; i < options.size(); ++i) {
      const bool active = static_cast<int>(i) == selected;
      const std::u16string label = options[i].first;
      container->AddChildView(std::make_unique<PaintedButton>(
          Then(options[i].second),
          gfx::Size(TextWidth(label, 12) + 24, 26),
          base::BindRepeating(
              [](bool active, bool segmented, std::u16string label,
                 views::Button* b, gfx::Canvas* canvas, bool hovered) {
                cc::PaintFlags flags;
                flags.setAntiAlias(true);
                gfx::RectF bounds(b->GetLocalBounds());
                if (active) {
                  flags.setColor(IsDark(b) ? SkColorSetA(SK_ColorWHITE, 0x29)
                                           : SK_ColorWHITE);
                  canvas->DrawRoundRect(bounds, 6.4f, flags);
                } else if (hovered) {
                  flags.setColor(Foreground(b, 0x12));
                  canvas->DrawRoundRect(bounds, 6.4f, flags);
                }
                if (!segmented && !active) {
                  flags.setStyle(cc::PaintFlags::kStroke_Style);
                  flags.setStrokeWidth(1);
                  flags.setColor(Foreground(b, 0x2A));
                  bounds.Inset(0.5f);
                  canvas->DrawRoundRect(bounds, 6, flags);
                }
                canvas->DrawStringRectWithFlags(
                    label,
                    Font(12, active ? gfx::Font::Weight::MEDIUM
                                    : gfx::Font::Weight::NORMAL),
                    active ? Foreground(b) : Foreground(b, 0xBF),
                    b->GetLocalBounds(), gfx::Canvas::TEXT_ALIGN_CENTER);
              },
              active, segmented, label),
          label));
    }
    return container;
  }

  // The grey track a segmented control's options sit in.
  class SegmentBox : public views::BoxLayoutView {
   public:
    SegmentBox() {
      SetInsideBorderInsets(gfx::Insets(2));
      SetBetweenChildSpacing(2);
    }
    void OnPaintBackground(gfx::Canvas* canvas) override {
      cc::PaintFlags flags;
      flags.setAntiAlias(true);
      flags.setColor(Foreground(this, 0x0F));
      canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), 8, flags);
    }
  };

  std::unique_ptr<views::View> Toggle(bool on,
                                      base::RepeatingCallback<void(bool)> set) {
    return std::make_unique<PaintedButton>(
        Then(base::BindRepeating(set, !on)), gfx::Size(36, 20),
        base::BindRepeating(
            [](bool on, views::Button* b, gfx::Canvas* canvas, bool hovered) {
              cc::PaintFlags flags;
              flags.setAntiAlias(true);
              flags.setColor(on ? kPrimary
                                : Foreground(b, IsDark(b) ? 0x26 : 0x40));
              canvas->DrawRoundRect(gfx::RectF(b->GetLocalBounds()), 10, flags);
              flags.setColor(SK_ColorWHITE);
              canvas->DrawCircle(gfx::PointF(on ? 26 : 10, 10), 8, flags);
            },
            on),
        on ? u"On" : u"Off");
  }

  std::unique_ptr<views::View> TextButton(const std::u16string& label,
                                          bool primary,
                                          base::RepeatingClosure run) {
    return std::make_unique<PaintedButton>(
        std::move(run), gfx::Size(TextWidth(label, 12) + 28, 28),
        base::BindRepeating(
            [](bool primary, std::u16string label, views::Button* b,
               gfx::Canvas* canvas, bool hovered) {
              cc::PaintFlags flags;
              flags.setAntiAlias(true);
              flags.setColor(primary ? SkColorSetA(kPrimary, hovered ? 0xE6 : 0xFF)
                                     : Foreground(b, hovered ? 0x1F : 0x14));
              canvas->DrawRoundRect(gfx::RectF(b->GetLocalBounds()), 6, flags);
              canvas->DrawStringRectWithFlags(
                  label, Font(12, gfx::Font::Weight::MEDIUM),
                  primary ? SK_ColorWHITE : Foreground(b), b->GetLocalBounds(),
                  gfx::Canvas::TEXT_ALIGN_CENTER);
            },
            primary, label),
        label);
  }

  // Buttons in a row.
  std::unique_ptr<views::View> Buttons(
      std::vector<std::unique_ptr<views::View>> buttons) {
    auto row = std::make_unique<FlowView>(8);
    for (auto& button : buttons) {
      row->AddChildView(std::move(button));
    }
    return row;
  }

  // A row of a list (a route, a boost): text and a Remove button.
  void ListRow(views::View* card,
               const std::u16string& text,
               base::RepeatingClosure remove) {
    auto* row = card->AddChildView(std::make_unique<views::BoxLayoutView>());
    row->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kCenter);
    auto* label = row->AddChildView(std::make_unique<ThemedLabel>(text, 13, false));
    label->SetElideBehavior(gfx::ELIDE_MIDDLE);
    row->SetFlexForView(label, 1);
    row->AddChildView(TextButton(u"Remove", false, Then(std::move(remove))));
  }

  void OpenPage(const GURL& url) {
    BrowserWindowInterface* b = browser();
    close_.Run();
    chrome::AddTabAt(b, url, IndexForNewTab(b), /*foreground=*/true,
                     GetCurrentSpace(b));
  }

  void RunCommand(int command_id) {
    BrowserWindowInterface* b = browser();
    close_.Run();
    chrome::ExecuteCommand(b, command_id);
  }

  base::RepeatingClosure Open(const char* url) {
    return base::BindRepeating(&SettingsSheet::OpenPage,
                               base::Unretained(this), GURL(url));
  }

  base::RepeatingClosure Command(int command_id) {
    return base::BindRepeating(&SettingsSheet::RunCommand,
                               base::Unretained(this), command_id);
  }

  base::RepeatingCallback<void(bool)> BoolPref(const char* name) {
    return base::BindRepeating(
        [](PrefService* prefs, const char* name, bool on) {
          prefs->SetBoolean(name, on);
        },
        prefs(), name);
  }

  // ---- Categories.

  void BuildGeneral(views::View* column) {
    Card* startup = Section(column, u"Startup");
    const bool restore =
        SessionStartupPref::GetStartupPref(profile()).type ==
        SessionStartupPref::LAST;
    auto set_startup = [](Profile* p, SessionStartupPref::Type type) {
      SessionStartupPref::SetStartupPref(p, SessionStartupPref(type));
    };
    Field(startup, u"On startup",
          Choice({{u"Restore tabs",
                   base::BindRepeating(set_startup, profile(),
                                       SessionStartupPref::LAST)},
                  {u"New tab", base::BindRepeating(set_startup, profile(),
                                                   SessionStartupPref::DEFAULT)}},
                 restore ? 0 : 1));

    Card* search = Section(column, u"Search");
    TemplateURLService* engines =
        TemplateURLServiceFactory::GetForProfile(profile());
    std::u16string engine_name;
    if (engines) {
      engines->Load();
      const TemplateURL* current = engines->GetDefaultSearchProvider();
      engine_name = current ? current->short_name() : u"";
      std::vector<std::pair<std::u16string, base::RepeatingClosure>> options;
      int selected = -1;
      for (TemplateURL* engine : engines->GetTemplateURLs()) {
        if (engine->prepopulate_id() <= 0 ||
            (engine != current && !engines->CanMakeDefault(engine))) {
          continue;
        }
        if (engine == current) {
          selected = options.size();
        }
        options.emplace_back(
            engine->short_name(),
            base::BindRepeating(
                [](TemplateURLService* service, TemplateURL* engine) {
                  service->SetUserSelectedDefaultSearchProvider(engine);
                },
                engines, engine));
      }
      if (!options.empty()) {
        Field(search, u"Search engine", Choice(std::move(options), selected));
      }
    }
    Field(search, u"Search suggestions",
          Toggle(prefs()->GetBoolean(prefs::kSearchSuggestEnabled),
                 BoolPref(prefs::kSearchSuggestEnabled)));
    Hint(search, u"What you type in the command bar is sent to " +
                     (engine_name.empty() ? u"your search engine" : engine_name) +
                     u" for suggestions.");

    Card* browser_card = Section(column, u"Default browser");
    auto* status = browser_card->AddChildView(std::make_unique<views::BoxLayoutView>());
    status->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kCenter);
    status->SetBetweenChildSpacing(12);
    auto* status_label = status->AddChildView(std::make_unique<ThemedLabel>(
        default_browser_ == shell_integration::IS_DEFAULT
            ? u"Mori is your default browser."
            : default_browser_ == shell_integration::NOT_DEFAULT
                  ? u"Another browser opens links from other apps."
                  : u"Your desktop doesn't say which browser is the default.",
        13, false));
    status->SetFlexForView(status_label, 1);
    if (default_browser_ == shell_integration::NOT_DEFAULT) {
      status->AddChildView(TextButton(
          u"Make Default", true,
          base::BindRepeating(&SettingsSheet::MakeDefault,
                              weak_factory_.GetWeakPtr())));
    }
    CheckDefaultBrowser();

    Card* links = Section(column, u"Links from other apps");
    const bool little = LinksOpenInLittle(profile());
    auto set_little = [](Profile* p, bool little) {
      SetLinksOpenInLittle(p, little);
    };
    Field(links, u"Open links in",
          Choice({{u"Little Mori",
                   base::BindRepeating(set_little, profile(), true)},
                  {u"Current space",
                   base::BindRepeating(set_little, profile(), false)}},
                 little ? 0 : 1));
    Hint(links, little ? u"A link from another app opens in a small window; "
                         u"“Open in Mori” keeps it as a tab."
                       : u"A link from another app opens as a tab in the space "
                         u"you used last.");
  }

  void CheckDefaultBrowser() {
    if (default_browser_checked_) {
      return;
    }
    default_browser_checked_ = true;
    base::MakeRefCounted<shell_integration::DefaultBrowserWorker>()
        ->StartCheckIsDefault(base::BindOnce(
            &SettingsSheet::OnDefaultBrowser, weak_factory_.GetWeakPtr()));
  }

  void OnDefaultBrowser(shell_integration::DefaultWebClientState state) {
    default_browser_ = state;
    if (category_ == Category::kGeneral) {
      PostRebuild();
    }
  }

  void MakeDefault() {
    base::MakeRefCounted<shell_integration::DefaultBrowserWorker>()
        ->StartSetAsDefault(base::BindOnce(&SettingsSheet::OnDefaultBrowser,
                                           weak_factory_.GetWeakPtr()));
  }

  void BuildAppearance(views::View* column) {
    Card* window = Section(column, u"Window");
    ThemeService* themes = ThemeServiceFactory::GetForProfile(profile());
    const ThemeService::BrowserColorScheme scheme =
        themes->GetBrowserColorScheme();
    auto set_scheme = [](ThemeService* t, ThemeService::BrowserColorScheme s) {
      t->SetBrowserColorScheme(s);
    };
    Field(window, u"Appearance",
          Choice({{u"System",
                   base::BindRepeating(set_scheme, themes,
                                       ThemeService::BrowserColorScheme::kSystem)},
                  {u"Light",
                   base::BindRepeating(set_scheme, themes,
                                       ThemeService::BrowserColorScheme::kLight)},
                  {u"Dark",
                   base::BindRepeating(set_scheme, themes,
                                       ThemeService::BrowserColorScheme::kDark)}},
                 static_cast<int>(scheme)));

    const std::optional<tab_groups::TabGroupId> space =
        GetCurrentSpace(browser());
    if (!space) {
      return;
    }
    const std::u16string name = browser()
                                    ->GetTabStripModel()
                                    ->group_model()
                                    ->GetTabGroup(*space)
                                    ->visual_data()
                                    ->title();
    Card* theme = Section(column, u"Theme for “" + name + u"”");
    const SpaceTheme current = GetSpaceTheme(browser(), *space);
    auto swatches = std::make_unique<FlowView>(6);
    auto add_swatch = [&](std::optional<std::pair<SkColor, SkColor>> colors) {
      const bool selected = current.colors == colors;
      swatches->AddChildView(std::make_unique<PaintedButton>(
          Then(base::BindRepeating(
              [](BrowserWindowInterface* b, tab_groups::TabGroupId space,
                 std::optional<std::pair<SkColor, SkColor>> colors) {
                if (!b->GetTabStripModel()->group_model()->ContainsTabGroup(
                        space)) {
                  return;
                }
                SpaceTheme theme = GetSpaceTheme(b, space);
                theme.colors = colors;
                SetSpaceTheme(b, space, theme);
              },
              browser(), *space, colors)),
          gfx::Size(30, 30),
          base::BindRepeating(
              [](std::optional<std::pair<SkColor, SkColor>> colors,
                 bool selected, views::Button* b, gfx::Canvas* canvas,
                 bool hovered) {
                cc::PaintFlags flags;
                flags.setAntiAlias(true);
                const gfx::PointF center =
                    gfx::RectF(b->GetLocalBounds()).CenterPoint();
                if (selected || hovered) {
                  flags.setColor(Foreground(b, selected ? 0x2A : 0x14));
                  canvas->DrawRoundRect(gfx::RectF(b->GetLocalBounds()), 8,
                                        flags);
                }
                if (!colors) {
                  flags.setStyle(cc::PaintFlags::kStroke_Style);
                  flags.setStrokeWidth(1.5f);
                  flags.setColor(Muted(b));
                  canvas->DrawCircle(center, 9, flags);
                  canvas->DrawLine(
                      gfx::PointF(center.x() - 6, center.y() + 6),
                      gfx::PointF(center.x() + 6, center.y() - 6), flags);
                  return;
                }
                const SkPoint points[2] = {
                    SkPoint::Make(center.x() - 9, center.y() - 9),
                    SkPoint::Make(center.x() + 9, center.y() + 9)};
                const SkColor4f stops[2] = {SkColor4f::FromColor(colors->first),
                                            SkColor4f::FromColor(colors->second)};
                flags.setColor(SK_ColorBLACK);
                flags.setShader(cc::PaintShader::MakeLinearGradient(
                    points, stops, nullptr, 2, SkTileMode::kClamp));
                canvas->DrawCircle(center, 10, flags);
              },
              colors, selected),
          colors ? u"Gradient" : u"No theme"));
    };
    add_swatch(std::nullopt);
    for (const auto& colors : kSpaceGradients) {
      add_swatch(colors);
    }
    Field(theme, u"Wash", std::move(swatches));
    Field(theme, u"Icon and name",
          TextButton(u"Edit Space…", false,
                     base::BindRepeating(
                         [](base::WeakPtr<SettingsSheet> sheet,
                            BrowserWindowInterface* b) {
                           if (sheet) {
                             sheet->close_.Run();
                           }
                           EditSpace(b);
                         },
                         weak_factory_.GetWeakPtr(), browser())));
  }

  void BuildTabs(views::View* column) {
    Card* idle = Section(column, u"Idle tabs");
    namespace tuning = performance_manager::user_tuning::prefs;
    // Memory Saver is a setting of the whole browser (local state).
    PrefService* local_state = g_browser_process->local_state();
    const bool sleep =
        local_state->GetInteger(tuning::kMemorySaverModeState) ==
        static_cast<int>(tuning::MemorySaverModeState::kEnabled);
    Field(idle, u"Sleep idle tabs",
          Toggle(sleep, base::BindRepeating(
                            [](PrefService* prefs, bool on) {
                              prefs->SetInteger(
                                  tuning::kMemorySaverModeState,
                                  static_cast<int>(
                                      on ? tuning::MemorySaverModeState::kEnabled
                                         : tuning::MemorySaverModeState::
                                               kDisabled));
                            },
                            local_state)));
    Hint(idle, u"Sleeping tabs stay in the sidebar but free their memory.");
    const int hours = ArchiveAfterHours(profile());
    std::vector<std::pair<std::u16string, base::RepeatingClosure>> options;
    int selected = -1;
    for (const auto& [value, label] : kArchiveOptions) {
      if (value == hours) {
        selected = options.size();
      }
      options.emplace_back(label,
                           base::BindRepeating(&SetArchiveAfterHours,
                                               profile(), value));
    }
    Field(idle, u"Archive idle tabs", Choice(std::move(options), selected));
    Hint(idle, u"Space tabs you haven't looked at close, and stay in the "
               u"library's Archive. Favorites and playing tabs stay.");
  }

  void BuildProfiles(views::View* column) {
    Card* profiles = Section(column, u"Profiles");
    Hint(profiles, u"Each profile has its own spaces, history, passwords and "
                   u"extensions, in a window of its own.");
    profiles->AddChildView(Buttons([&] {
      std::vector<std::unique_ptr<views::View>> buttons;
      buttons.push_back(TextButton(
          u"Manage Profiles…", false,
          base::BindRepeating(
              [](base::WeakPtr<SettingsSheet> sheet) {
                if (sheet) {
                  sheet->close_.Run();
                }
                ProfilePicker::Show(ProfilePicker::Params::FromEntryPoint(
                    ProfilePicker::EntryPoint::kProfileMenuManageProfiles));
              },
              weak_factory_.GetWeakPtr())));
      return buttons;
    }()));
  }

  void BuildSites(views::View* column) {
    Card* routes = Section(column, u"Air Traffic Control");
    Hint(routes, u"Links to these sites open in their space, from anywhere.");
    TabStripModel* model = browser()->GetTabStripModel();
    const std::vector<Route> list = ListRoutes(browser());
    for (const Route& route : list) {
      std::u16string space = u"a deleted space";
      if (std::optional<tab_groups::TabGroupId> id =
              FindSpace(browser(), route.space)) {
        space = model->group_model()->GetTabGroup(*id)->visual_data()->title();
      }
      ListRow(routes, base::UTF8ToUTF16(route.host) + u"  →  " + space,
              base::BindRepeating(&RemoveRoute, browser(), route.host));
    }
    if (list.empty()) {
      Hint(routes, u"Right-click a tab and choose “Always Open in This Space”.");
    }

    Card* boosts = Section(column, u"Boosts");
    const std::vector<std::string> sites = ListBoostedSites(profile());
    for (const std::string& host : sites) {
      ListRow(boosts, base::UTF8ToUTF16(host),
              base::BindRepeating(&RemoveBoost, profile(), host));
    }
    if (sites.empty()) {
      Hint(boosts, u"Boost a site from its tab's menu or with Ctrl+Shift+B.");
    }

    Card* permissions = Section(column, u"Permissions");
    Hint(permissions, u"Camera, microphone, location, notifications and the "
                      u"rest, for each site.");
    permissions->AddChildView(Buttons([&] {
      std::vector<std::unique_ptr<views::View>> buttons;
      buttons.push_back(TextButton(u"Site Permissions…", false,
                                   Open("chrome://settings/content")));
      return buttons;
    }()));
  }

  void BuildPrivacy(views::View* column) {
    Card* tracking = Section(column, u"Tracking");
    Field(tracking, u"Send Do Not Track",
          Toggle(prefs()->GetBoolean(prefs::kEnableDoNotTrack),
                 BoolPref(prefs::kEnableDoNotTrack)));
    Hint(tracking, u"Asks sites not to track you. Pages loaded before keep the "
                   u"old setting until reloaded.");
    const bool block =
        prefs()->GetInteger(prefs::kCookieControlsMode) ==
        static_cast<int>(content_settings::CookieControlsMode::kBlockThirdParty);
    auto set_cookies = [](PrefService* p, bool block) {
      p->SetInteger(
          prefs::kCookieControlsMode,
          static_cast<int>(
              block ? content_settings::CookieControlsMode::kBlockThirdParty
                    : content_settings::CookieControlsMode::kIncognitoOnly));
    };
    Field(tracking, u"Third-party cookies",
          Choice({{u"Allow", base::BindRepeating(set_cookies, prefs(), false)},
                  {u"Block", base::BindRepeating(set_cookies, prefs(), true)}},
                 block ? 1 : 0));
    Hint(tracking, u"Sites embedded in a page can't use their cookies. Some "
                   u"sign-in buttons and comment widgets may stop working.");
    Field(tracking, u"Secure connections",
          Toggle(prefs()->GetBoolean(prefs::kHttpsOnlyModeEnabled),
                 BoolPref(prefs::kHttpsOnlyModeEnabled)));
    Hint(tracking, u"Pages load over HTTPS, and Mori asks before opening a "
                   u"site that doesn't support it.");

    Card* clear = Section(column, u"Clear browsing data");
    Hint(clear, u"History, cookies, cached files and more, for a time range.");
    clear->AddChildView(Buttons([&] {
      std::vector<std::unique_ptr<views::View>> buttons;
      buttons.push_back(TextButton(u"Clear Browsing Data…", false,
                                   Command(IDC_CLEAR_BROWSING_DATA)));
      buttons.push_back(TextButton(u"More Privacy Settings…", false,
                                   Open("chrome://settings/privacy")));
      return buttons;
    }()));
  }

  void BuildPasswords(views::View* column) {
    Card* passwords = Section(column, u"Passwords");
    Field(passwords, u"Offer to save",
          Toggle(prefs()->GetBoolean(
                     password_manager::prefs::kCredentialsEnableService),
                 BoolPref(password_manager::prefs::kCredentialsEnableService)));
    Hint(passwords, u"Mori offers to save the passwords you type, and fills "
                    u"them in next time.");
    passwords->AddChildView(Buttons([&] {
      std::vector<std::unique_ptr<views::View>> buttons;
      buttons.push_back(TextButton(u"Open Password Manager", false,
                                   Open("chrome://password-manager/passwords")));
      return buttons;
    }()));
  }

  void BuildAssistant(views::View* column) {
    Card* assistant = Section(column, u"Assistant");
    Hint(assistant,
         codex_.has_value()
             ? (codex_->empty()
                    ? u"The assistant runs on Codex, which isn't installed. "
                      u"Install it and sign in with `codex login`."
                    : u"The assistant runs on Codex (" +
                          codex_->LossyDisplayName() +
                          u"), with your Codex sign-in. It asks before it "
                          u"reads a page or changes anything.")
             : u"Looking for Codex…");
    assistant->AddChildView(Buttons([&] {
      std::vector<std::unique_ptr<views::View>> buttons;
      buttons.push_back(TextButton(
          IsAssistantOpen(browser()) ? u"Hide Assistant" : u"Show Assistant",
          false,
          base::BindRepeating(
              [](base::WeakPtr<SettingsSheet> sheet,
                 BrowserWindowInterface* b) {
                if (sheet) {
                  sheet->close_.Run();
                }
                ToggleAssistant(b);
              },
              weak_factory_.GetWeakPtr(), browser())));
      return buttons;
    }()));
    if (!codex_.has_value() && !codex_pending_) {
      codex_pending_ = true;
      base::ThreadPool::PostTaskAndReplyWithResult(
          FROM_HERE, {base::MayBlock()}, base::BindOnce(&FindCodexBinary),
          base::BindOnce(&SettingsSheet::OnCodex, weak_factory_.GetWeakPtr()));
    }
  }

  void OnCodex(std::optional<base::FilePath> path) {
    codex_ = path.value_or(base::FilePath());
    codex_pending_ = false;
    if (category_ == Category::kAssistant) {
      PostRebuild();
    }
  }

  void BuildImport(views::View* column) {
    Card* import = Section(column, u"Import from another browser");
    auto import_from = [](base::WeakPtr<SettingsSheet> sheet,
                          BrowserWindowInterface* b, ImportSource source) {
      if (sheet) {
        sheet->close_.Run();
      }
      ImportSpaces(b, source);
    };
    import->AddChildView(Buttons([&] {
      std::vector<std::unique_ptr<views::View>> buttons;
      for (const auto& [label, source] :
           {std::pair{u"Zen", ImportSource::kZen},
            std::pair{u"Firefox", ImportSource::kFirefox},
            std::pair{u"Arc", ImportSource::kArc}}) {
        buttons.push_back(TextButton(
            label, false,
            base::BindRepeating(import_from, weak_factory_.GetWeakPtr(),
                                browser(), source)));
      }
      buttons.push_back(TextButton(u"Chrome and Others…", false,
                                   Command(IDC_IMPORT_SETTINGS)));
      return buttons;
    }()));
    Hint(import, u"Open tabs, Zen workspaces and Arc spaces become new spaces; "
                 u"Zen essentials and Arc favorites become favorites. The other "
                 u"browser's files are only read.");
  }

  void BuildAbout(views::View* column) {
    Card* about = Section(column, u"");
    auto* row = about->AddChildView(std::make_unique<views::BoxLayoutView>());
    row->SetBetweenChildSpacing(12);
    row->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kCenter);
    row->AddChildView(std::make_unique<PaintedButton>(
        views::Button::PressedCallback(), gfx::Size(28, 28),
        base::BindRepeating([](views::Button* b, gfx::Canvas* canvas,
                               bool hovered) {
          canvas->DrawImageInt(
              gfx::CreateVectorIcon(LucideIcon("trees"), 28, kPrimary), 0, 0);
        }),
        u"Mori"));
    auto* text = row->AddChildView(std::make_unique<views::BoxLayoutView>());
    text->SetOrientation(views::BoxLayout::Orientation::kVertical);
    text->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kStart);
    text->AddChildView(std::make_unique<ThemedLabel>(
        u"Mori", 13, false, gfx::Font::Weight::SEMIBOLD));
    text->AddChildView(std::make_unique<ThemedLabel>(
        u"Built on Chromium " +
            base::UTF8ToUTF16(std::string(version_info::GetVersionNumber())),
        11, true));

    Card* more = Section(column, u"More settings");
    Hint(more, u"Everything Mori doesn't show here: languages, downloads, "
               u"accessibility, system and more.");
    more->AddChildView(Buttons([&] {
      std::vector<std::unique_ptr<views::View>> buttons;
      buttons.push_back(TextButton(u"All Settings…", false,
                                   Open("chrome://settings")));
      buttons.push_back(
          TextButton(u"Extensions…", false, Command(IDC_MANAGE_EXTENSIONS)));
      return buttons;
    }()));
  }

  raw_ptr<BrowserView> browser_view_;
  base::RepeatingClosure close_;
  Category category_ = Category::kGeneral;
  raw_ptr<views::BoxLayoutView> header_ = nullptr;
  raw_ptr<views::BoxLayoutView> rail_ = nullptr;
  raw_ptr<views::ScrollView> scroll_ = nullptr;
  raw_ptr<CenterColumn> center_ = nullptr;
  shell_integration::DefaultWebClientState default_browser_ =
      shell_integration::UNKNOWN_DEFAULT;
  bool default_browser_checked_ = false;
  std::optional<base::FilePath> codex_;
  bool codex_pending_ = false;
  base::WeakPtrFactory<SettingsSheet> weak_factory_{this};
};

BEGIN_METADATA(SettingsSheet)
END_METADATA

// Keeps the sheet over the page card and closes it on Escape.
class Settings : public ui::EventObserver, public views::ViewObserver {
 public:
  explicit Settings(BrowserView* browser_view) : browser_view_(browser_view) {
    overlay_ = std::make_unique<Overlay>(
        browser_view,
        std::make_unique<SettingsSheet>(
            browser_view,
            base::BindRepeating(&Settings::PostClose, base::Unretained(this))),
        "MoriSettings");
  }

  void Show() {
    if (!open_) {
      open_ = true;
      static_cast<SettingsSheet*>(overlay_->contents())->Open();
      monitor_ = views::EventMonitor::CreateApplicationMonitor(
          this, browser_view_->GetWidget()->GetNativeWindow(),
          {ui::EventType::kKeyPressed});
      if (views::View* card = browser_view_->contents_container()) {
        observation_.AddObservation(card);
      }
      observation_.AddObservation(browser_view_.get());
    }
    Place();
  }

  void Close() {
    if (!open_) {
      return;
    }
    open_ = false;
    monitor_.reset();
    observation_.RemoveAllObservations();
    overlay_->Hide();
    if (content::WebContents* contents = browser_view_->browser()
                                             ->GetTabStripModel()
                                             ->GetActiveWebContents()) {
      contents->Focus();
    }
  }

  // ui::EventObserver:
  void OnEvent(const ui::Event& event) override {
    auto* target = static_cast<aura::Window*>(event.target());
    if (event.IsKeyEvent() &&
        event.AsKeyEvent()->key_code() == ui::VKEY_ESCAPE && target &&
        target->GetRootWindow() ==
            browser_view_->GetWidget()->GetNativeWindow()->GetRootWindow()) {
      PostClose();
    }
  }

  // views::ViewObserver:
  void OnViewBoundsChanged(views::View* observed_view) override { Place(); }

 private:
  // Exactly over the page card (Mori 3 shows settings in the card).
  void Place() {
    views::View* card = browser_view_->contents_container();
    if (!open_ || !card) {
      return;
    }
    overlay_->Show(views::View::ConvertRectToTarget(card, browser_view_,
                                                    card->GetLocalBounds()));
  }

  void PostClose() {
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&Settings::Close, weak_factory_.GetWeakPtr()));
  }

  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<Overlay> overlay_;
  std::unique_ptr<views::EventMonitor> monitor_;
  base::ScopedMultiSourceObservation<views::View, views::ViewObserver>
      observation_{this};
  bool open_ = false;
  base::WeakPtrFactory<Settings> weak_factory_{this};
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::Settings*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(Settings, kSettingsKey)
}  // namespace

bool ShowSettings(BrowserWindowInterface* browser) {
  if (browser->GetType() != BrowserWindowInterface::TYPE_NORMAL) {
    return false;
  }
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget() ||
      !browser_view->contents_container()) {
    return false;
  }
  Settings* settings = browser_view->GetProperty(kSettingsKey);
  if (!settings) {
    settings = browser_view->SetProperty(
        kSettingsKey, std::make_unique<Settings>(browser_view));
  }
  settings->Show();
  return true;
}

}  // namespace mori
