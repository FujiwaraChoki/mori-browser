// Mori's command bar. It lives in a child widget of the browser window (like
// Chromium's find bar), so it floats above the page, and asks Chromium's
// autocomplete for results: what was typed, history, bookmarks, search
// suggestions and open tabs.

#include "chrome/browser/ui/views/mori/mori_command_bar.h"

#include <algorithm>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/scoped_observation.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "chrome/browser/autocomplete/chrome_autocomplete_provider_client.h"
#include "chrome/browser/autocomplete/chrome_autocomplete_scheme_classifier.h"
#include "chrome/browser/favicon/favicon_service_factory.h"
#include "chrome/browser/history/history_service_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/navigator/browser_navigator.h"
#include "chrome/browser/ui/navigator/browser_navigator_params.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_commands.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "chrome/browser/ui/views/theme_copying_widget.h"
#include "components/keyed_service/core/service_access_type.h"
#include "components/omnibox/browser/autocomplete_classifier.h"
#include "components/omnibox/browser/autocomplete_controller.h"
#include "components/omnibox/browser/autocomplete_input.h"
#include "components/omnibox/browser/autocomplete_match.h"
#include "components/omnibox/browser/autocomplete_result.h"
#include "components/omnibox/browser/favicon_cache.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/base/models/image_model.h"
#include "ui/color/color_id.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/geometry/insets.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/text_elider.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/background.h"
#include "ui/views/border.h"
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/image_view.h"
#include "ui/views/controls/label.h"
#include "ui/views/controls/textfield/textfield.h"
#include "ui/views/controls/textfield/textfield_controller.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/focus/focus_manager.h"
#include "chrome/browser/search_engines/template_url_service_factory.h"
#include "components/search_engines/template_url.h"
#include "components/search_engines/template_url_service.h"
#include "components/vector_icons/vector_icons.h"
#include "cc/paint/paint_flags.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/font_list.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/shadow_value.h"
#include "ui/gfx/skia_paint_util.h"
#include "ui/views/layout/flex_layout.h"
#include "ui/views/view_class_properties.h"
#include "ui/views/view.h"
#include "ui/views/view_observer.h"
#include "ui/views/widget/widget.h"
#include "ui/views/widget/widget_observer.h"

namespace mori {
namespace {

// Mori 3's launcher (crates/mori/src/ui/launcher.rs), itself measured from
// Zen's.
constexpr size_t kMaxResults = 10;
constexpr size_t kMaxCommands = 3;
constexpr int kPanelRadius = 10;
constexpr int kInputRowHeight = 62;
constexpr int kInputTextSize = 17;
constexpr int kRowHeight = 45;
constexpr int kRowGap = 4;
constexpr int kRowTextSize = 14;
constexpr int kIconSize = 16;
// Room around the panel for its shadow.
constexpr auto kShadowInsets = gfx::Insets::TLBR(32, 56, 88, 56);

struct BarColors {
  bool dark;
  SkColor panel;
  SkColor border;
  SkColor text;
  SkColor muted;
  SkColor highlight;
  SkColor highlight_text;
  SkColor hairline;
  SkColor chip;
  SkColor field_icon;
};

BarColors ColorsFor(const views::View* view) {
  const SkColor frame =
      view->GetColorProvider()->GetColor(ui::kColorFrameActive);
  const bool dark = color_utils::IsDark(frame);
  BarColors c;
  c.dark = dark;
  if (dark) {
    c.panel = SkColorSetRGB(0x18, 0x18, 0x18);
    c.border = SkColorSetRGB(0x4B, 0x4B, 0x4B);
    c.text = SkColorSetRGB(0xEE, 0xEE, 0xEF);
    c.muted = SkColorSetRGB(0xA0, 0xA1, 0xA2);
    c.highlight = SkColorSetRGB(0x3E, 0x4F, 0x64);
    c.highlight_text = c.text;
  } else {
    c.panel = SK_ColorWHITE;
    c.border = SkColorSetRGB(0xE2, 0xE2, 0xE2);
    c.text = SkColorSetRGB(0x1D, 0x1F, 0x22);
    c.muted = SkColorSetRGB(0x5A, 0x5C, 0x5E);
    c.highlight = SkColorSetRGB(0x71, 0xAF, 0xFB);
    c.highlight_text = SK_ColorWHITE;
  }
  c.hairline = color_utils::AlphaBlend(c.text, c.panel, dark ? 0.08f : 0.07f);
  c.chip = color_utils::AlphaBlend(c.text, c.panel, 0.08f);
  c.field_icon = color_utils::AlphaBlend(c.text, c.panel, 0.55f);
  return c;
}

gfx::FontList SizedFont(int size, gfx::Font::Weight weight) {
  const gfx::FontList& base = views::Label::GetDefaultFontList();
  return base.DeriveWithSizeDelta(size - base.GetFontSize())
      .DeriveWithWeight(weight);
}

bool IsTabMatch(const AutocompleteMatch& match) {
  return match.type == AutocompleteMatchType::OPEN_TAB ||
         match.has_tab_match.value_or(false);
}

// One result: icon, title, " — source", and "Switch to Tab" for open tabs.
class ResultRow : public views::Button {
  METADATA_HEADER(ResultRow, views::Button)

 public:
  explicit ResultRow(PressedCallback callback)
      : views::Button(std::move(callback)) {
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal, gfx::Insets::VH(0, 12),
        12));
    layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    icon_ = AddChildView(std::make_unique<views::ImageView>());
    icon_->SetImageSize(gfx::Size(kIconSize, kIconSize));

    auto* text = AddChildView(std::make_unique<views::View>());
    text->SetLayoutManager(std::make_unique<views::FlexLayout>())
        ->SetOrientation(views::LayoutOrientation::kHorizontal)
        .SetCrossAxisAlignment(views::LayoutAlignment::kCenter)
        .SetDefault(views::kMarginsKey, gfx::Insets::TLBR(0, 0, 0, 6));
    layout->SetFlexForView(text, 1);
    title_ = text->AddChildView(std::make_unique<views::Label>());
    title_->SetFontList(SizedFont(kRowTextSize, gfx::Font::Weight::SEMIBOLD));
    title_->SetElideBehavior(gfx::ELIDE_TAIL);
    title_->SetProperty(
        views::kFlexBehaviorKey,
        views::FlexSpecification(views::MinimumFlexSizeRule::kScaleToZero,
                                 views::MaximumFlexSizeRule::kPreferred));
    dash_ = text->AddChildView(std::make_unique<views::Label>(u"\u2014"));
    dash_->SetFontList(SizedFont(kRowTextSize, gfx::Font::Weight::NORMAL));
    source_ = text->AddChildView(std::make_unique<views::Label>());
    source_->SetFontList(SizedFont(kRowTextSize, gfx::Font::Weight::NORMAL));
    source_->SetElideBehavior(gfx::ELIDE_TAIL);
    source_->SetMaximumWidthSingleLine(320);

    tab_ = AddChildView(std::make_unique<views::View>());
    auto* tab_layout = tab_->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal, gfx::Insets(), 8));
    tab_layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    tab_label_ = tab_->AddChildView(std::make_unique<views::Label>(u"Switch to Tab"));
    tab_label_->SetFontList(SizedFont(12, gfx::Font::Weight::SEMIBOLD));
    tab_arrow_ = tab_->AddChildView(std::make_unique<views::ImageView>());
    tab_arrow_->SetPreferredSize(gfx::Size(20, 20));

    SetPreferredSize(gfx::Size(0, kRowHeight));
    SetFocusBehavior(FocusBehavior::NEVER);
  }

  void SetContent(const ui::ImageModel& icon,
                  const std::u16string& title,
                  const std::u16string& source,
                  bool tab) {
    icon_model_ = icon;
    title_->SetText(title);
    source_->SetText(source);
    dash_->SetVisible(!source.empty());
    source_->SetVisible(!source.empty());
    tab_->SetVisible(tab);
    SetAccessibleName(source.empty() ? title : title + u", " + source);
    ApplyColors();
  }

  void SetHighlighted(bool highlighted) {
    if (highlighted_ == highlighted) {
      return;
    }
    highlighted_ = highlighted;
    ApplyColors();
    SchedulePaint();
  }

  void OnThemeChanged() override {
    views::Button::OnThemeChanged();
    ApplyColors();
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    if (!highlighted_) {
      return;
    }
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    flags.setColor(ColorsFor(this).highlight);
    canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), 8, flags);
  }

 private:
  void ApplyColors() {
    if (!GetColorProvider()) {
      return;
    }
    const BarColors c = ColorsFor(this);
    // Light mode highlights in saturated blue with white text.
    const bool on_blue = highlighted_ && !c.dark;
    const SkColor title = on_blue ? c.highlight_text : c.text;
    const SkColor secondary =
        on_blue ? SkColorSetA(SK_ColorWHITE, 0xD9)
        : highlighted_ ? color_utils::AlphaBlend(c.text, c.highlight, 0.75f)
                       : c.muted;
    title_->SetEnabledColor(title);
    dash_->SetEnabledColor(color_utils::AlphaBlend(
        c.muted, highlighted_ ? c.highlight : c.panel, 0.7f));
    source_->SetEnabledColor(secondary);
    tab_label_->SetEnabledColor(secondary);
    tab_arrow_->SetImage(ui::ImageModel::FromVectorIcon(
        vector_icons::kArrowForwardIcon, c.muted, 12));
    tab_arrow_->SetBackground(views::CreateRoundedRectBackground(c.chip, 4));
    if (icon_model_.IsVectorIcon()) {
      icon_->SetImage(ui::ImageModel::FromVectorIcon(
          *icon_model_.GetVectorIcon().vector_icon(), title, kIconSize));
    } else {
      icon_->SetImage(icon_model_);
    }
  }

  bool highlighted_ = false;
  ui::ImageModel icon_model_;
  raw_ptr<views::ImageView> icon_ = nullptr;
  raw_ptr<views::Label> title_ = nullptr;
  raw_ptr<views::Label> dash_ = nullptr;
  raw_ptr<views::Label> source_ = nullptr;
  raw_ptr<views::View> tab_ = nullptr;
  raw_ptr<views::Label> tab_label_ = nullptr;
  raw_ptr<views::ImageView> tab_arrow_ = nullptr;
};

BEGIN_METADATA(ResultRow)
END_METADATA

// The text field closes the bar when it loses focus, e.g. on a click into
// the page.
class CommandField : public views::Textfield {
  METADATA_HEADER(CommandField, views::Textfield)

 public:
  explicit CommandField(base::RepeatingClosure on_blur)
      : on_blur_(std::move(on_blur)) {}

  void OnBlur() override {
    views::Textfield::OnBlur();
    on_blur_.Run();
  }

 private:
  base::RepeatingClosure on_blur_;
};

BEGIN_METADATA(CommandField)
END_METADATA

class CommandBar : public views::View,
                   public views::TextfieldController,
                   public views::ViewObserver,
                   public AutocompleteController::Observer {
  METADATA_HEADER(CommandBar, views::View)

 public:
  explicit CommandBar(BrowserView* browser_view)
      : browser_view_(browser_view),
        profile_(browser_view->browser()->GetProfile()),
        favicon_cache_(
            FaviconServiceFactory::GetForProfile(
                profile_, ServiceAccessType::EXPLICIT_ACCESS),
            HistoryServiceFactory::GetForProfile(
                profile_, ServiceAccessType::EXPLICIT_ACCESS)) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical));
    SetBorder(views::CreateEmptyBorder(kShadowInsets));

    // The search field: icon, 17px text.
    auto* input_row = AddChildView(std::make_unique<views::View>());
    auto* input_layout =
        input_row->SetLayoutManager(std::make_unique<views::BoxLayout>(
            views::BoxLayout::Orientation::kHorizontal,
            gfx::Insets::VH(0, 20), 14));
    input_layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    input_row->SetPreferredSize(gfx::Size(0, kInputRowHeight));
    search_icon_ = input_row->AddChildView(std::make_unique<views::ImageView>());
    search_icon_->SetImageSize(gfx::Size(kIconSize, kIconSize));
    field_ = input_row->AddChildView(std::make_unique<CommandField>(
        base::BindRepeating(&CommandBar::OnFieldBlurred,
                            weak_factory_.GetWeakPtr())));
    input_layout->SetFlexForView(field_, 1);
    field_->SetController(this);
    field_->SetBackgroundEnabled(false);
    field_->SetBorder(nullptr);
    field_->SetPlaceholderText(u"Search or enter address");
    field_->SetFontList(SizedFont(kInputTextSize, gfx::Font::Weight::NORMAL));
    field_->SetTextColorId(kColorOmniboxText);
    field_->SetPlaceholderTextColorId(kColorOmniboxResultsTextDimmed);
    field_->GetViewAccessibility().SetName(u"Command bar");

    // A hairline, then the results.
    hairline_ = AddChildView(std::make_unique<views::View>());
    hairline_->SetPreferredSize(gfx::Size(0, 1));
    results_ = AddChildView(std::make_unique<views::View>());
    results_->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical, gfx::Insets(8), kRowGap));

    controller_ = std::make_unique<AutocompleteController>(
        std::make_unique<ChromeAutocompleteProviderClient>(profile_),
        AutocompleteControllerConfig{
            .provider_types = AutocompleteClassifier::DefaultOmniboxProviders(),
            .unscoped_open_tab_suggestions = true});
    controller_observation_.Observe(controller_.get());
    browser_view_observation_.Observe(browser_view_);
  }

  ~CommandBar() override = default;

  void OnThemeChanged() override {
    views::View::OnThemeChanged();
    const BarColors c = ColorsFor(this);
    search_icon_->SetImage(ui::ImageModel::FromVectorIcon(
        vector_icons::kSearchIcon, c.field_icon, kIconSize));
    hairline_->SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(0, 10)));
    hairline_->SetBackground(views::CreateSolidBackground(c.hairline));
  }

  // The panel: #181818 (white in light mode) with a 1px edge and Mori 3's
  // two shadows, a wide soft one and a close one.
  void OnPaintBackground(gfx::Canvas* canvas) override {
    const BarColors c = ColorsFor(this);
    const gfx::RectF panel(GetContentsBounds());
    {
      cc::PaintFlags shadow;
      shadow.setAntiAlias(true);
      shadow.setColor(c.panel);
      shadow.setLooper(gfx::CreateShadowDrawLooper(
          {gfx::ShadowValue(gfx::Vector2d(0, 24), 110,
                            SkColorSetA(SK_ColorBLACK, c.dark ? 64 : 28)),
           gfx::ShadowValue(gfx::Vector2d(0, 4), 14,
                            SkColorSetA(SK_ColorBLACK, c.dark ? 77 : 26))}));
      canvas->DrawRoundRect(panel, kPanelRadius, shadow);
    }
    cc::PaintFlags fill;
    fill.setAntiAlias(true);
    fill.setColor(c.panel);
    canvas->DrawRoundRect(panel, kPanelRadius, fill);
    cc::PaintFlags edge;
    edge.setAntiAlias(true);
    edge.setStyle(cc::PaintFlags::kStroke_Style);
    edge.setStrokeWidth(1);
    edge.setColor(c.border);
    gfx::RectF edge_rect = panel;
    edge_rect.Inset(0.5f);
    canvas->DrawRoundRect(edge_rect, kPanelRadius - 0.5f, edge);
  }

  void Show(CommandBarMode mode) {
    mode_ = mode;
    std::u16string text;
    content::WebContents* contents =
        browser_view_->browser()->GetTabStripModel()->GetActiveWebContents();
    if (mode == CommandBarMode::kCurrentTab && contents) {
      text = base::UTF8ToUTF16(contents->GetVisibleURL().spec());
    }
    field_->SetText(text);
    field_->SelectAll(false);
    selected_ = 0;
    controller_->Stop(AutocompleteStopReason::kClobbered);
    matches_.clear();
    commands_.clear();
    UpdateResults();
    UpdateBounds();
    GetWidget()->Show();
    field_->RequestFocus();
  }

  void Hide() {
    if (!GetWidget()->IsVisible()) {
      return;
    }
    controller_->Stop(AutocompleteStopReason::kClobbered);
    GetWidget()->Hide();
  }

  // views::TextfieldController:
  void ContentsChanged(views::Textfield* sender,
                       const std::u16string& new_contents) override {
    selected_ = 0;
    StartAutocomplete();
  }

  bool HandleKeyEvent(views::Textfield* sender,
                      const ui::KeyEvent& key_event) override {
    if (key_event.type() != ui::EventType::kKeyPressed) {
      return false;
    }
    switch (key_event.key_code()) {
      case ui::VKEY_ESCAPE:
        Hide();
        return true;
      case ui::VKEY_RETURN:
        OpenSelected();
        return true;
      case ui::VKEY_DOWN:
        MoveSelection(1);
        return true;
      case ui::VKEY_UP:
        MoveSelection(-1);
        return true;
      default:
        return false;
    }
  }

  // AutocompleteController::Observer:
  void OnResultChanged(AutocompleteController* controller,
                       bool default_match_changed) override {
    RebuildMatches();
    selected_ = std::min(selected_, std::max<int>(0, VisibleMatches() - 1));
    UpdateResults();
    UpdateBounds();
  }

  // views::ViewObserver:
  void OnViewBoundsChanged(views::View* observed_view) override {
    if (GetWidget() && GetWidget()->IsVisible()) {
      UpdateBounds();
    }
  }

 private:
  void StartAutocomplete() {
    const std::u16string text(field_->GetText());
    if (auto* service = TemplateURLServiceFactory::GetForProfile(profile_)) {
      if (const TemplateURL* engine = service->GetDefaultSearchProvider()) {
        engine_name_ = engine->short_name();
      }
    }
    if (text.empty()) {
      controller_->Stop(AutocompleteStopReason::kClobbered);
      matches_.clear();
      commands_.clear();
      UpdateResults();
      UpdateBounds();
      return;
    }
    AutocompleteInput input(text, text.size(),
                            metrics::OmniboxEventProto::OTHER,
                            ChromeAutocompleteSchemeClassifier(profile_));
    input.set_prevent_inline_autocomplete(true);
    content::WebContents* contents =
        browser_view_->browser()->GetTabStripModel()->GetActiveWebContents();
    if (mode_ == CommandBarMode::kCurrentTab && contents) {
      input.set_current_url(contents->GetVisibleURL());
    }
    input_is_url_ = input.type() == metrics::OmniboxInputType::URL;
    controller_->Start(input);
  }

  // Commands come first, then Chromium's suggestions.
  int VisibleMatches() const {
    return static_cast<int>(commands_.size() + matches_.size());
  }

  // The bar's results in display order. When the input looks like an
  // address, the address itself goes first, so Enter loads it rather than
  // searching for it or switching to an open tab on the same site.
  void RebuildMatches() {
    matches_.clear();
    commands_.clear();
    if (field_->GetText().empty()) {
      return;
    }
    if (!input_is_url_) {
      commands_ = MatchCommands(browser_view_->browser(), field_->GetText(),
                                kMaxCommands);
    }
    for (const AutocompleteMatch& match : controller_->result()) {
      if (matches_.size() + commands_.size() == kMaxResults) {
        break;
      }
      if (match.destination_url.is_valid()) {
        matches_.push_back(match);
      }
    }
    // Otherwise Mori 3's order: the search for what was typed, the search
    // suggestions, then tabs, history and bookmarks.
    if (!input_is_url_) {
      std::ranges::stable_partition(matches_, [](const AutocompleteMatch& m) {
        return m.type == AutocompleteMatchType::SEARCH_WHAT_YOU_TYPED;
      });
      auto rest = std::ranges::find_if(matches_, [](const AutocompleteMatch& m) {
        return m.type != AutocompleteMatchType::SEARCH_WHAT_YOU_TYPED;
      });
      std::stable_partition(rest, matches_.end(),
                            [](const AutocompleteMatch& m) {
                              return AutocompleteMatch::IsSearchType(m.type);
                            });
    }
    if (input_is_url_ && !matches_.empty()) {
      auto address = std::ranges::find_if(matches_, [](const auto& m) {
        return m.type == AutocompleteMatchType::URL_WHAT_YOU_TYPED;
      });
      if (address == matches_.end()) {
        address = std::ranges::find_if(matches_, [](const auto& m) {
          return !IsTabMatch(m) && !AutocompleteMatch::IsSearchType(m.type);
        });
      }
      if (address != matches_.end()) {
        std::rotate(matches_.begin(), address, address + 1);
      }
    }
  }

  void UpdateResults() {
    const int count = VisibleMatches();
    while (static_cast<int>(rows_.size()) < count) {
      const int index = rows_.size();
      rows_.push_back(results_->AddChildView(std::make_unique<ResultRow>(
          base::BindRepeating(&CommandBar::OpenMatch, base::Unretained(this),
                              index))));
    }
    for (int i = 0; i < static_cast<int>(rows_.size()); ++i) {
      ResultRow* row = rows_[i];
      row->SetVisible(i < count);
      if (i >= count) {
        continue;
      }
      row->SetHighlighted(i == selected_);
      if (i < static_cast<int>(commands_.size())) {
        const Command& command = *commands_[i];
        row->SetContent(ui::ImageModel::FromVectorIcon(*command.icon),
                        std::u16string(command.title), u"", false);
        continue;
      }
      const AutocompleteMatch& match = matches_[i - commands_.size()];
      const bool search = AutocompleteMatch::IsSearchType(match.type);
      ui::ImageModel icon = ui::ImageModel::FromVectorIcon(
          match.GetVectorIcon(/*is_bookmark=*/false));
      if (!search) {
        const gfx::Image favicon = favicon_cache_.GetFaviconForPageUrl(
            match.destination_url,
            base::BindOnce(&CommandBar::OnFaviconFetched,
                           weak_factory_.GetWeakPtr()));
        if (!favicon.IsEmpty()) {
          icon = ui::ImageModel::FromImage(favicon);
        }
      }
      // As Mori 3: a search shows the query (and, for what was typed, the
      // engine); a page shows its title and host.
      std::u16string title;
      std::u16string source;
      if (search) {
        title = match.contents;
        if (match.type == AutocompleteMatchType::SEARCH_WHAT_YOU_TYPED &&
            !engine_name_.empty()) {
          source = u"Search with " + engine_name_;
        }
      } else {
        std::u16string host = base::UTF8ToUTF16(match.destination_url.host());
        if (host.starts_with(u"www.")) {
          host = host.substr(4);
        }
        title = match.description.empty() ? host : match.description;
        if (title != host) {
          source = host;
        }
      }
      row->SetContent(icon, title, source, IsTabMatch(match));
    }
    const bool has_rows = count > 0;
    hairline_->SetVisible(has_rows);
    results_->SetVisible(has_rows);
  }

  void OnFaviconFetched(const gfx::Image& image) { UpdateResults(); }

  void MoveSelection(int delta) {
    const int count = VisibleMatches();
    if (count == 0) {
      return;
    }
    selected_ = (selected_ + delta + count) % count;
    UpdateResults();
  }

  void OpenSelected() {
    if (VisibleMatches() == 0) {
      if (field_->GetText().empty()) {
        Hide();
      }
      return;
    }
    OpenMatch(selected_);
  }

  void OpenMatch(int index) {
    if (index >= VisibleMatches()) {
      return;
    }
    if (index < static_cast<int>(commands_.size())) {
      const Command* command = commands_[index];
      Hide();
      command->run(browser_view_->browser());
      return;
    }
    AutocompleteMatch match = matches_[index - commands_.size()];
    if (!match.destination_url.is_valid()) {
      return;
    }
    BrowserWindowInterface* browser = browser_view_->browser();
    Hide();
    NavigateParams params(browser, match.destination_url, match.transition);
    const bool switch_to_tab = IsTabMatch(match);
    if (switch_to_tab) {
      params.disposition = WindowOpenDisposition::SWITCH_TO_TAB;
    } else if (mode_ == CommandBarMode::kNewTab) {
      params.disposition = WindowOpenDisposition::NEW_FOREGROUND_TAB;
      // New tabs join the space on screen, at its top.
      params.group = GetCurrentSpace(browser);
      params.tabstrip_index = IndexForNewTab(browser);
    } else {
      params.disposition = WindowOpenDisposition::CURRENT_TAB;
    }
    params.window_action = NavigateParams::WindowAction::kShowWindow;
    Navigate(&params);
    if (switch_to_tab) {
      ShowSpaceOfActiveTab(browser);
    }
  }

  void OnFieldBlurred() {
    // Hiding during a focus change re-enters the focus manager; do it after.
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&CommandBar::HideIfUnfocused,
                                  weak_factory_.GetWeakPtr()));
  }

  // Closes the bar when focus moved elsewhere in the window, e.g. into the
  // page. When the window itself loses focus nothing is focused, and the bar
  // stays: the focus manager gives the field its focus back on return.
  void HideIfUnfocused() {
    views::FocusManager* focus_manager = GetFocusManager();
    if (focus_manager && focus_manager->GetFocusedView() &&
        !field_->HasFocus()) {
      Hide();
    }
  }

  // As Mori 3: 63% of the window's width (560 to 910px), centred, its top
  // 31.6% down the window (at most 296px).
  void UpdateBounds() {
    if (!GetWidget()) {
      return;
    }
    const gfx::Rect area = browser_view_->GetLocalBounds();
    const int width = std::min(
        std::clamp(static_cast<int>(area.width() * 0.63), 560, 910),
        area.width() - 32);
    const int top =
        std::clamp(static_cast<int>(area.height() * 0.316), 16, 296);
    gfx::Rect bounds(area.x() + (area.width() - width) / 2 -
                         kShadowInsets.left(),
                     area.y() + top - kShadowInsets.top(),
                     width + kShadowInsets.width(),
                     GetPreferredSize().height());
    views::View::ConvertRectToScreen(browser_view_, &bounds);
    GetWidget()->SetBounds(bounds);
  }

  raw_ptr<BrowserView> browser_view_;
  raw_ptr<Profile> profile_;
  CommandBarMode mode_ = CommandBarMode::kNewTab;
  FaviconCache favicon_cache_;
  std::unique_ptr<AutocompleteController> controller_;
  raw_ptr<views::ImageView> search_icon_ = nullptr;
  raw_ptr<CommandField> field_ = nullptr;
  raw_ptr<views::View> hairline_ = nullptr;
  raw_ptr<views::View> results_ = nullptr;
  std::u16string engine_name_;
  std::vector<raw_ptr<ResultRow>> rows_;
  std::vector<AutocompleteMatch> matches_;
  std::vector<const Command*> commands_;
  bool input_is_url_ = false;
  int selected_ = 0;
  base::ScopedObservation<AutocompleteController,
                          AutocompleteController::Observer>
      controller_observation_{this};
  base::ScopedObservation<views::View, views::ViewObserver>
      browser_view_observation_{this};
  base::WeakPtrFactory<CommandBar> weak_factory_{this};
};

BEGIN_METADATA(CommandBar)
END_METADATA

// Owns the command bar's widget. Stored on the BrowserView, so it goes away
// with the window.
class CommandBarHost : public views::WidgetObserver {
 public:
  explicit CommandBarHost(BrowserView* browser_view) {
    views::Widget* parent = browser_view->GetWidget();
    widget_ = std::make_unique<ThemeCopyingWidget>(parent);
    views::Widget::InitParams params(
        views::Widget::InitParams::CLIENT_OWNS_WIDGET,
        views::Widget::InitParams::TYPE_CONTROL);
    params.name = "MoriCommandBar";
    params.parent = parent->GetNativeView();
    params.opacity = views::Widget::InitParams::WindowOpacity::kTranslucent;
    // Like the find bar: the browser window stays active and its focus
    // manager routes keys to the bar. An activatable child takes activation
    // from the window, which clears focus and closes the bar at once.
    params.activatable = views::Widget::InitParams::Activatable::kNo;
    widget_->Init(std::move(params));
    bar_ = widget_->SetContentsView(std::make_unique<CommandBar>(browser_view));
    widget_observation_.Observe(widget_.get());
  }

  ~CommandBarHost() override = default;

  CommandBar* bar() { return bar_; }

  // views::WidgetObserver:
  void OnWidgetDestroying(views::Widget* widget) override {
    bar_ = nullptr;
    widget_observation_.Reset();
  }

 private:
  std::unique_ptr<views::Widget> widget_;
  raw_ptr<CommandBar> bar_ = nullptr;
  base::ScopedObservation<views::Widget, views::WidgetObserver>
      widget_observation_{this};
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::CommandBarHost*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(CommandBarHost, kCommandBarHostKey)
}  // namespace

bool ShowCommandBar(BrowserWindowInterface* browser, CommandBarMode mode) {
  if (!browser || browser->GetType() != BrowserWindowInterface::TYPE_NORMAL) {
    return false;
  }
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget()) {
    return false;
  }
  CommandBarHost* host = browser_view->GetProperty(kCommandBarHostKey);
  if (!host || !host->bar()) {
    host = browser_view->SetProperty(
        kCommandBarHostKey, std::make_unique<CommandBarHost>(browser_view));
  }
  if (!host->bar()) {
    return false;
  }
  host->bar()->Show(mode);
  return true;
}

}  // namespace mori
