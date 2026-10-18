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
#include "ui/views/view.h"
#include "ui/views/view_observer.h"
#include "ui/views/widget/widget.h"
#include "ui/views/widget/widget_observer.h"

namespace mori {
namespace {

constexpr size_t kMaxResults = 8;
constexpr int kWidth = 640;
constexpr int kCornerRadius = 12;
constexpr int kRowHeight = 40;
constexpr int kIconSize = 16;

bool IsTabMatch(const AutocompleteMatch& match) {
  return match.type == AutocompleteMatchType::OPEN_TAB ||
         match.has_tab_match.value_or(false);
}

// One suggestion: icon, title and a dimmed second line (URL or "Switch to
// tab").
class ResultRow : public views::Button {
  METADATA_HEADER(ResultRow, views::Button)

 public:
  explicit ResultRow(PressedCallback callback) : views::Button(std::move(callback)) {
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal, gfx::Insets::VH(0, 14),
        12));
    layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    icon_ = AddChildView(std::make_unique<views::ImageView>());
    icon_->SetImageSize(gfx::Size(kIconSize, kIconSize));
    title_ = AddChildView(std::make_unique<views::Label>());
    title_->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    title_->SetElideBehavior(gfx::ELIDE_TAIL);
    detail_ = AddChildView(std::make_unique<views::Label>());
    detail_->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    detail_->SetElideBehavior(gfx::ELIDE_MIDDLE);
    layout->SetFlexForView(title_, 2);
    layout->SetFlexForView(detail_, 1);
    SetPreferredSize(gfx::Size(kWidth, kRowHeight));
    SetFocusBehavior(FocusBehavior::NEVER);
  }

  void SetMatch(const AutocompleteMatch& match, const gfx::Image& favicon) {
    const bool is_url = !AutocompleteMatch::IsSearchType(match.type);
    std::u16string title = match.description.empty() || !is_url
                               ? match.contents
                               : match.description;
    std::u16string detail;
    if (IsTabMatch(match)) {
      detail = u"Switch to tab";
    } else if (is_url && !match.description.empty()) {
      detail = match.contents;
    }
    title_->SetText(title);
    detail_->SetText(detail);
    detail_->SetVisible(!detail.empty());
    SetAccessibleName(detail.empty() ? title : title + u", " + detail);
    if (!favicon.IsEmpty()) {
      icon_->SetImage(ui::ImageModel::FromImage(favicon));
    } else {
      icon_->SetImage(ui::ImageModel::FromVectorIcon(
          match.GetVectorIcon(/*is_bookmark=*/false),
          selected_ ? kColorOmniboxResultsTextSelected
                    : kColorOmniboxResultsIcon,
          kIconSize));
    }
  }

  void SetSelected(bool selected) {
    selected_ = selected;
    SetBackground(selected ? views::CreateRoundedRectBackground(
                                 kColorOmniboxResultsBackgroundSelected, 8)
                           : nullptr);
    title_->SetEnabledColor(selected ? kColorOmniboxResultsTextSelected
                                     : kColorOmniboxText);
    detail_->SetEnabledColor(selected ? kColorOmniboxResultsTextDimmedSelected
                                      : kColorOmniboxResultsTextDimmed);
  }

 private:
  bool selected_ = false;
  raw_ptr<views::ImageView> icon_ = nullptr;
  raw_ptr<views::Label> title_ = nullptr;
  raw_ptr<views::Label> detail_ = nullptr;
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
        views::BoxLayout::Orientation::kVertical, gfx::Insets(8), 2));
    SetBackground(views::CreateRoundedRectBackground(
        kColorOmniboxResultsBackground, kCornerRadius, 1));
    SetBorder(views::CreateRoundedRectBorder(1, kCornerRadius,
                                             ui::kColorSeparator));

    field_ = AddChildView(std::make_unique<CommandField>(base::BindRepeating(
        &CommandBar::OnFieldBlurred, weak_factory_.GetWeakPtr())));
    field_->SetController(this);
    field_->SetBackgroundEnabled(false);
    field_->SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(8, 10)));
    field_->SetPlaceholderText(u"Search or enter address");
    field_->SetFontList(field_->GetFontList().DeriveWithSizeDelta(4));
    field_->GetViewAccessibility().SetName(u"Command bar");

    results_ = AddChildView(std::make_unique<views::View>());
    results_->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical));

    controller_ = std::make_unique<AutocompleteController>(
        std::make_unique<ChromeAutocompleteProviderClient>(profile_),
        AutocompleteControllerConfig{
            .provider_types = AutocompleteClassifier::DefaultOmniboxProviders(),
            .unscoped_open_tab_suggestions = true});
    controller_observation_.Observe(controller_.get());
    browser_view_observation_.Observe(browser_view_);
  }

  ~CommandBar() override = default;

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
    if (text.empty()) {
      controller_->Stop(AutocompleteStopReason::kClobbered);
      matches_.clear();
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

  int VisibleMatches() const { return static_cast<int>(matches_.size()); }

  // The bar's results in display order. When the input looks like an
  // address, the address itself goes first, so Enter loads it rather than
  // searching for it or switching to an open tab on the same site.
  void RebuildMatches() {
    matches_.clear();
    if (field_->GetText().empty()) {
      return;
    }
    for (const AutocompleteMatch& match : controller_->result()) {
      if (match.destination_url.is_valid()) {
        matches_.push_back(match);
      }
      if (matches_.size() == kMaxResults) {
        break;
      }
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
      const AutocompleteMatch& match = matches_[i];
      gfx::Image favicon;
      if (!AutocompleteMatch::IsSearchType(match.type)) {
        favicon = favicon_cache_.GetFaviconForPageUrl(
            match.destination_url,
            base::BindOnce(&CommandBar::OnFaviconFetched,
                           weak_factory_.GetWeakPtr()));
      }
      row->SetSelected(i == selected_);
      row->SetMatch(match, favicon);
    }
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
    AutocompleteMatch match = matches_[index];
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
      // New tabs join the space on screen.
      params.group = GetCurrentSpace(browser);
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

  // Centered over the page, a fifth of the way down.
  void UpdateBounds() {
    views::View* anchor = browser_view_->contents_container();
    if (!anchor || !GetWidget()) {
      return;
    }
    gfx::Rect area = anchor->GetLocalBounds();
    const int width = std::min(kWidth, area.width() - 32);
    const int height = GetPreferredSize().height();
    gfx::Rect bounds(area.x() + (area.width() - width) / 2,
                     area.y() + area.height() / 5, width, height);
    views::View::ConvertRectToScreen(anchor, &bounds);
    GetWidget()->SetBounds(bounds);
  }

  raw_ptr<BrowserView> browser_view_;
  raw_ptr<Profile> profile_;
  CommandBarMode mode_ = CommandBarMode::kNewTab;
  FaviconCache favicon_cache_;
  std::unique_ptr<AutocompleteController> controller_;
  raw_ptr<CommandField> field_ = nullptr;
  raw_ptr<views::View> results_ = nullptr;
  std::vector<raw_ptr<ResultRow>> rows_;
  std::vector<AutocompleteMatch> matches_;
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
