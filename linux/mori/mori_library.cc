#include "chrome/browser/ui/views/mori/mori_library.h"

#include <algorithm>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "base/functional/bind.h"
#include "base/i18n/case_conversion.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/strings/string_number_conversions.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/cancelable_task_tracker.h"
#include "base/task/sequenced_task_runner.h"
#include "base/time/time.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/bookmarks/bookmark_model_factory.h"
#include "chrome/browser/favicon/favicon_service_factory.h"
#include "chrome/browser/history/history_service_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/sessions/tab_restore_service_factory.h"
#include "chrome/browser/ui/browser_tabstrip.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_window_events.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "chrome/browser/ui/views/mori/mori_toast.h"
#include "components/bookmarks/browser/bookmark_model.h"
#include "components/bookmarks/browser/bookmark_node.h"
#include "components/bookmarks/browser/bookmark_utils.h"
#include "components/favicon/core/favicon_service.h"
#include "components/favicon_base/favicon_types.h"
#include "components/history/core/browser/history_service.h"
#include "components/history/core/browser/history_types.h"
#include "components/keyed_service/core/service_access_type.h"
#include "components/sessions/core/serialized_navigation_entry.h"
#include "components/sessions/core/tab_restore_service.h"
#include "components/sessions/core/tab_restore_types.h"
#include "content/public/browser/web_contents.h"
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
#include "ui/gfx/image/image_skia.h"
#include "ui/gfx/paint_vector_icon.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/background.h"
#include "ui/views/border.h"
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/label.h"
#include "ui/views/controls/scroll_view.h"
#include "ui/views/controls/textfield/textfield.h"
#include "ui/views/controls/textfield/textfield_controller.h"
#include "ui/views/event_monitor.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/layout/box_layout_view.h"
#include "ui/views/view_class_properties.h"
#include "ui/views/widget/widget.h"

namespace mori {
namespace {

constexpr int kWidth = 380;
constexpr int kMaxHeight = 460;
constexpr auto kShadowInsets = gfx::Insets::TLBR(24, 40, 56, 40);
constexpr int kPadding = 8;
constexpr int kRowHeight = 30;
constexpr int kMaxRows = 200;
// Mori 3's primary (the selected segment) and destructive colours.
constexpr SkColor kPrimary = SkColorSetRGB(0x29, 0x8E, 0xCD);
constexpr SkColor kDestructive = SkColorSetRGB(0xE5, 0x48, 0x4D);
// A click on the library button while the library is open closes it, and
// the same click must not open it again.
constexpr base::TimeDelta kReopenGuard = base::Milliseconds(300);

enum class Section { kHistory, kBookmarks, kArchive };

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

SkColor Ghost(const views::View* view, SkAlpha alpha) {
  return SkColorSetA(IsDark(view) ? SK_ColorWHITE : SK_ColorBLACK, alpha);
}

gfx::FontList Font(int size) {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(size - base.GetFontSize());
}

// Mori 3's relative_time: "just now", "5m ago", "3h ago", "2d ago".
std::u16string RelativeTime(base::Time time) {
  const base::TimeDelta age = base::Time::Now() - time;
  if (time.is_null()) {
    return u"";
  }
  if (age < base::Minutes(1)) {
    return u"just now";
  }
  auto ago = [](int64_t n, std::u16string_view unit) {
    return base::NumberToString16(n) + std::u16string(unit) + u" ago";
  };
  if (age < base::Hours(1)) {
    return ago(age.InMinutes(), u"m");
  }
  if (age < base::Days(1)) {
    return ago(age.InHours(), u"h");
  }
  if (age < base::Days(30)) {
    return ago(age.InDays(), u"d");
  }
  return ago(age.InDays() / 30, u"mo");
}

struct Entry {
  std::u16string title;
  GURL url;
  std::u16string meta;
  base::RepeatingClosure remove;
};

// History / Bookmarks / Archive.
class Segment : public views::Button {
  METADATA_HEADER(Segment, views::Button)

 public:
  Segment(PressedCallback callback, std::u16string label)
      : views::Button(std::move(callback)), label_(std::move(label)) {
    SetPreferredSize(gfx::Size(0, 26));
    GetViewAccessibility().SetName(label_);
  }

  void SetSelected(bool selected) {
    selected_ = selected;
    SchedulePaint();
  }

  void PaintButtonContents(gfx::Canvas* canvas) override {
    const gfx::RectF bounds(GetLocalBounds());
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    if (selected_) {
      flags.setColor(kPrimary);
      canvas->DrawRoundRect(bounds, 4.4f, flags);
    } else if (GetState() == STATE_HOVERED || GetState() == STATE_PRESSED) {
      flags.setColor(Ghost(this, 0x14));
      canvas->DrawRoundRect(bounds, 4.4f, flags);
    }
    canvas->DrawStringRectWithFlags(
        label_, Font(12), selected_ ? SK_ColorWHITE : Foreground(this),
        GetLocalBounds(), gfx::Canvas::TEXT_ALIGN_CENTER);
  }

  void StateChanged(ButtonState old_state) override { SchedulePaint(); }

 private:
  std::u16string label_;
  bool selected_ = false;
};

BEGIN_METADATA(Segment)
END_METADATA

// A 15px favicon, or Mori 3's grey placeholder square.
class Favicon : public views::View {
  METADATA_HEADER(Favicon, views::View)

 public:
  Favicon() { SetPreferredSize(gfx::Size(15, 15)); }

  void SetImage(const gfx::ImageSkia& image) {
    image_ = image;
    SchedulePaint();
  }

  void OnPaint(gfx::Canvas* canvas) override {
    const gfx::Rect bounds = GetLocalBounds();
    if (image_.isNull()) {
      cc::PaintFlags flags;
      flags.setAntiAlias(true);
      flags.setColor(Ghost(this, 0x30));
      canvas->DrawRoundRect(gfx::RectF(bounds), 3, flags);
      return;
    }
    canvas->DrawImageInt(image_, 0, 0, image_.width(), image_.height(),
                         bounds.x(), bounds.y(), bounds.width(),
                         bounds.height(), /*filter=*/true);
  }

 private:
  gfx::ImageSkia image_;
};

BEGIN_METADATA(Favicon)
END_METADATA

// The "x" that removes a row's entry, shown while the row is hovered.
class RemoveButton : public views::Button {
  METADATA_HEADER(RemoveButton, views::Button)

 public:
  explicit RemoveButton(PressedCallback callback)
      : views::Button(std::move(callback)) {
    SetPreferredSize(gfx::Size(16, 16));
    GetViewAccessibility().SetName(u"Remove");
  }

  void PaintButtonContents(gfx::Canvas* canvas) override {
    const gfx::ImageSkia icon = gfx::CreateVectorIcon(
        LucideIcon("x"), 12,
        GetState() == STATE_HOVERED ? Foreground(this) : Muted(this));
    canvas->DrawImageInt(icon, (width() - icon.width()) / 2,
                         (height() - icon.height()) / 2);
  }

  void StateChanged(ButtonState old_state) override { SchedulePaint(); }
};

BEGIN_METADATA(RemoveButton)
END_METADATA

class Row : public views::Button {
  METADATA_HEADER(Row, views::Button)

 public:
  Row(PressedCallback open, const Entry& entry)
      : views::Button(std::move(open)) {
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal, gfx::Insets::VH(0, 8), 8));
    layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    favicon_ = AddChildView(std::make_unique<Favicon>());
    title_ = AddChildView(std::make_unique<views::Label>(
        entry.title.empty() ? base::UTF8ToUTF16(entry.url.spec())
                            : entry.title));
    title_->SetFontList(Font(13));
    title_->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    title_->SetElideBehavior(gfx::ELIDE_TAIL);
    layout->SetFlexForView(title_, 1);
    meta_ = AddChildView(std::make_unique<views::Label>(entry.meta));
    meta_->SetFontList(Font(11));
    meta_->SetVisible(!entry.meta.empty());
    if (entry.remove) {
      remove_ = AddChildView(std::make_unique<RemoveButton>(
          base::BindRepeating([](base::RepeatingClosure run) { run.Run(); },
                              entry.remove)));
      remove_->SetVisible(false);
    }
    SetPreferredSize(gfx::Size(0, kRowHeight));
    GetViewAccessibility().SetName(std::u16string(title_->GetText()));
  }

  void SetFavicon(const gfx::ImageSkia& image) { favicon_->SetImage(image); }

  void OnThemeChanged() override {
    views::Button::OnThemeChanged();
    title_->SetEnabledColor(Foreground(this));
    meta_->SetEnabledColor(Muted(this));
  }

  void StateChanged(ButtonState old_state) override {
    const bool hovered =
        GetState() == STATE_HOVERED || GetState() == STATE_PRESSED;
    if (remove_) {
      remove_->SetVisible(hovered || remove_->IsMouseHovered());
      meta_->SetVisible(!remove_->GetVisible() && !meta_->GetText().empty());
      InvalidateLayout();
    }
    SchedulePaint();
  }

  void PaintButtonContents(gfx::Canvas* canvas) override {
    if (GetState() == STATE_HOVERED || GetState() == STATE_PRESSED) {
      cc::PaintFlags flags;
      flags.setAntiAlias(true);
      flags.setColor(Ghost(this, 0x12));
      canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), 6.4f, flags);
    }
  }

 private:
  raw_ptr<Favicon> favicon_ = nullptr;
  raw_ptr<views::Label> title_ = nullptr;
  raw_ptr<views::Label> meta_ = nullptr;
  raw_ptr<RemoveButton> remove_ = nullptr;
};

BEGIN_METADATA(Row)
END_METADATA

class SearchField : public views::Textfield {
  METADATA_HEADER(SearchField, views::Textfield)

 public:
  SearchField() {
    SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(5, 10)));
    SetFontList(Font(13));
    SetPlaceholderText(u"Search");
    GetViewAccessibility().SetName(u"Search the library");
  }
  void OnThemeChanged() override {
    views::Textfield::OnThemeChanged();
    SetBackground(views::CreateRoundedRectBackground(Ghost(this, 0x10), 6));
    SetTextColorId(kColorOmniboxText);
  }
};

BEGIN_METADATA(SearchField)
END_METADATA

// "Clear browsing history".
class TextLink : public views::Button {
  METADATA_HEADER(TextLink, views::Button)

 public:
  TextLink(PressedCallback callback, std::u16string text)
      : views::Button(std::move(callback)), text_(std::move(text)) {
    SetPreferredSize(gfx::Size(0, 24));
    GetViewAccessibility().SetName(text_);
  }
  void PaintButtonContents(gfx::Canvas* canvas) override {
    gfx::Rect bounds = GetLocalBounds();
    bounds.Inset(gfx::Insets::VH(0, 8));
    canvas->DrawStringRectWithFlags(text_, Font(12), kDestructive, bounds,
                                    gfx::Canvas::TEXT_ALIGN_LEFT);
  }

 private:
  std::u16string text_;
};

BEGIN_METADATA(TextLink)
END_METADATA

class LibraryView : public views::View, public views::TextfieldController {
  METADATA_HEADER(LibraryView, views::View)

 public:
  LibraryView(base::RepeatingClosure close, base::RepeatingClosure resize)
      : close_(std::move(close)), resize_(std::move(resize)) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical,
        kShadowInsets + gfx::Insets(kPadding), kPadding));
    auto* segments = AddChildView(std::make_unique<views::BoxLayoutView>());
    segments->SetBetweenChildSpacing(4);
    auto add_segment = [&](Section section, std::u16string label) {
      auto* segment = segments->AddChildView(std::make_unique<Segment>(
          base::BindRepeating(&LibraryView::Show, base::Unretained(this),
                              section),
          std::move(label)));
      segments->SetFlexForView(segment, 1);
      segments_.push_back(segment);
    };
    add_segment(Section::kHistory, u"History");
    add_segment(Section::kBookmarks, u"Bookmarks");
    add_segment(Section::kArchive, u"Archive");

    search_ = AddChildView(std::make_unique<SearchField>());
    search_->set_controller(this);

    scroll_ = AddChildView(std::make_unique<views::ScrollView>());
    scroll_->SetBackgroundColor(std::nullopt);
    scroll_->SetHorizontalScrollBarMode(
        views::ScrollView::ScrollBarMode::kDisabled);
    scroll_->SetVerticalScrollBarMode(
        views::ScrollView::ScrollBarMode::kHiddenButEnabled);
    list_ = scroll_->SetContents(std::make_unique<views::View>());
    list_->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical, gfx::Insets(), 1));

    clear_ = AddChildView(std::make_unique<TextLink>(
        base::BindRepeating(&LibraryView::ClearHistory,
                            base::Unretained(this)),
        u"Clear browsing history"));
  }

  void Open(BrowserWindowInterface* browser) {
    browser_ = browser;
    search_->SetText(u"");
    Show(Section::kHistory);
  }

  void FocusSearch() { search_->RequestFocus(); }

  int HeightForWidth(int width) {
    // Room for the rows, up to the library's height.
    const int fixed = kShadowInsets.height() + 2 * kPadding + 26 + kPadding +
                      search_->GetPreferredSize().height() + kPadding +
                      (clear_->GetVisible() ? 24 + kPadding : 0);
    const int max_list = kMaxHeight - (fixed - kShadowInsets.height());
    scroll_->ClipHeightTo(0, max_list);
    return GetHeightForWidth(width);
  }

  // views::TextfieldController:
  void ContentsChanged(views::Textfield* sender,
                       const std::u16string& contents) override {
    Reload();
  }

  bool HandleKeyEvent(views::Textfield* sender,
                      const ui::KeyEvent& event) override {
    if (event.type() != ui::EventType::kKeyPressed) {
      return false;
    }
    if (event.key_code() == ui::VKEY_ESCAPE) {
      close_.Run();
      return true;
    }
    if (event.key_code() == ui::VKEY_RETURN && !entries_.empty()) {
      OpenUrl(entries_.front().url);
      return true;
    }
    return false;
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    gfx::Rect popover = GetLocalBounds();
    popover.Inset(kShadowInsets);
    PaintPopover(this, canvas, popover);
  }

 private:
  Profile* profile() { return browser_->GetProfile(); }

  void Show(Section section) {
    section_ = section;
    for (size_t i = 0; i < segments_.size(); ++i) {
      segments_[i]->SetSelected(static_cast<int>(i) ==
                                static_cast<int>(section));
    }
    clear_->SetVisible(section == Section::kHistory);
    Reload();
  }

  void Reload() {
    if (!browser_) {
      return;
    }
    tracker_.TryCancelAll();
    const std::u16string query(search_->GetText());
    switch (section_) {
      case Section::kHistory: {
        history::HistoryService* history =
            HistoryServiceFactory::GetForProfile(
                profile(), ServiceAccessType::EXPLICIT_ACCESS);
        if (!history) {
          Populate({});
          return;
        }
        history::QueryOptions options;
        options.max_count = kMaxRows;
        options.duplicate_policy = history::QueryOptions::REMOVE_ALL_DUPLICATES;
        history->QueryHistory(
            query, options,
            base::BindOnce(&LibraryView::OnHistory,
                           weak_factory_.GetWeakPtr()),
            &tracker_);
        return;
      }
      case Section::kBookmarks:
        Populate(Bookmarks(query));
        return;
      case Section::kArchive:
        Populate(Archive(query));
        return;
    }
  }

  void OnHistory(history::QueryResults results) {
    std::vector<Entry> entries;
    for (const history::URLResult& result : results) {
      const GURL url = result.url();
      entries.push_back(
          {result.title(), url, RelativeTime(result.visit_time()),
           base::BindRepeating(&LibraryView::RemoveHistory,
                               weak_factory_.GetWeakPtr(), url)});
    }
    Populate(std::move(entries));
  }

  static bool Matches(const std::u16string& query,
                      const std::u16string& title,
                      const GURL& url) {
    if (query.empty()) {
      return true;
    }
    const std::u16string q = base::i18n::ToLower(query);
    return base::i18n::ToLower(title).find(q) != std::u16string::npos ||
           base::i18n::ToLower(base::UTF8ToUTF16(url.spec())).find(q) !=
               std::u16string::npos;
  }

  std::vector<Entry> Bookmarks(const std::u16string& query) {
    std::vector<Entry> entries;
    bookmarks::BookmarkModel* model =
        BookmarkModelFactory::GetForBrowserContext(profile());
    if (!model || !model->loaded()) {
      return entries;
    }
    std::vector<const bookmarks::BookmarkNode*> nodes;
    std::vector<const bookmarks::BookmarkNode*> stack = {model->root_node()};
    while (!stack.empty()) {
      const bookmarks::BookmarkNode* node = stack.back();
      stack.pop_back();
      if (node->is_url()) {
        if (Matches(query, node->GetTitle(), node->url())) {
          nodes.push_back(node);
        }
        continue;
      }
      for (const auto& child : node->children()) {
        stack.push_back(child.get());
      }
    }
    std::ranges::sort(nodes, [](const bookmarks::BookmarkNode* a,
                                const bookmarks::BookmarkNode* b) {
      return a->date_added() > b->date_added();
    });
    for (const bookmarks::BookmarkNode* node : nodes) {
      if (entries.size() == kMaxRows) {
        break;
      }
      entries.push_back(
          {node->GetTitle(), node->url(), u"",
           base::BindRepeating(&LibraryView::RemoveBookmark,
                               weak_factory_.GetWeakPtr(), node->url())});
    }
    return entries;
  }

  // Arc's archive: the tabs closed lately (by hand, or by auto-archive).
  std::vector<Entry> Archive(const std::u16string& query) {
    std::vector<Entry> entries;
    sessions::TabRestoreService* service =
        TabRestoreServiceFactory::GetForProfile(profile());
    if (!service) {
      return entries;
    }
    for (const auto& entry : service->entries()) {
      if (entry->type != sessions::tab_restore::TAB) {
        continue;
      }
      const auto& tab = static_cast<const sessions::tab_restore::Tab&>(*entry);
      if (tab.navigations.empty()) {
        continue;
      }
      const sessions::SerializedNavigationEntry& nav =
          tab.navigations[tab.normalized_navigation_index()];
      if (!nav.virtual_url().SchemeIsHTTPOrHTTPS() ||
          !Matches(query, nav.title(), nav.virtual_url())) {
        continue;
      }
      entries.push_back(
          {nav.title(), nav.virtual_url(), RelativeTime(tab.timestamp),
           base::BindRepeating(&LibraryView::RemoveArchived,
                               weak_factory_.GetWeakPtr(), entry->id)});
    }
    return entries;
  }

  void Populate(std::vector<Entry> entries) {
    entries_ = std::move(entries);
    list_->RemoveAllChildViews();
    rows_.clear();
    favicon_tracker_.TryCancelAll();
    if (entries_.empty()) {
      auto* empty = list_->AddChildView(
          std::make_unique<views::Label>(u"Nothing here yet"));
      empty->SetFontList(Font(13));
      empty->SetBorder(views::CreateEmptyBorder(gfx::Insets(16)));
      empty->SetHorizontalAlignment(gfx::ALIGN_LEFT);
      empty->SetEnabledColor(Muted(this));
    }
    favicon::FaviconService* favicons = FaviconServiceFactory::GetForProfile(
        profile(), ServiceAccessType::EXPLICIT_ACCESS);
    for (const Entry& entry : entries_) {
      Row* row = list_->AddChildView(std::make_unique<Row>(
          base::BindRepeating(&LibraryView::OpenUrl, base::Unretained(this),
                              entry.url),
          entry));
      rows_.push_back(row);
      if (favicons) {
        favicons->GetFaviconImageForPageURL(
            entry.url,
            base::BindOnce(&LibraryView::OnFavicon, weak_factory_.GetWeakPtr(),
                           rows_.size() - 1),
            &favicon_tracker_);
      }
    }
    scroll_->ScrollToOffset(gfx::PointF());
    resize_.Run();
  }

  void OnFavicon(size_t index, const favicon_base::FaviconImageResult& result) {
    if (index < rows_.size() && !result.image.IsEmpty()) {
      rows_[index]->SetFavicon(*result.image.ToImageSkia());
    }
  }

  void OpenUrl(const GURL& url) {
    BrowserWindowInterface* browser = browser_;
    close_.Run();
    if (browser && url.is_valid()) {
      chrome::AddTabAt(browser, url, IndexForNewTab(browser),
                       /*foreground=*/true, GetCurrentSpace(browser));
    }
  }

  void RemoveHistory(const GURL& url) {
    if (history::HistoryService* history =
            HistoryServiceFactory::GetForProfile(
                profile(), ServiceAccessType::EXPLICIT_ACCESS)) {
      history->DeleteURLs({url});
    }
    PostReload();
  }

  void RemoveBookmark(const GURL& url) {
    if (bookmarks::BookmarkModel* model =
            BookmarkModelFactory::GetForBrowserContext(profile())) {
      bookmarks::RemoveAllBookmarks(model, url, FROM_HERE);
    }
    PostReload();
  }

  void RemoveArchived(SessionID id) {
    if (sessions::TabRestoreService* service =
            TabRestoreServiceFactory::GetForProfile(profile())) {
      service->RemoveEntryById(id);
    }
    PostReload();
  }

  void ClearHistory() {
    history::HistoryService* history = HistoryServiceFactory::GetForProfile(
        profile(), ServiceAccessType::EXPLICIT_ACCESS);
    if (!history) {
      return;
    }
    history->ExpireHistoryBetween(
        {}, std::nullopt, base::Time(), base::Time::Max(),
        /*user_initiated=*/true,
        base::BindOnce(&LibraryView::Reload, weak_factory_.GetWeakPtr()),
        &tracker_);
    ShowToast(browser_, u"Cleared browsing history");
  }

  // The row asking is still on the stack.
  void PostReload() {
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&LibraryView::Reload, weak_factory_.GetWeakPtr()));
  }

  base::RepeatingClosure close_;
  base::RepeatingClosure resize_;
  raw_ptr<BrowserWindowInterface> browser_ = nullptr;
  Section section_ = Section::kHistory;
  std::vector<raw_ptr<Segment>> segments_;
  raw_ptr<SearchField> search_ = nullptr;
  raw_ptr<views::ScrollView> scroll_ = nullptr;
  raw_ptr<views::View> list_ = nullptr;
  raw_ptr<TextLink> clear_ = nullptr;
  std::vector<Entry> entries_;
  std::vector<raw_ptr<Row>> rows_;
  base::CancelableTaskTracker tracker_;
  base::CancelableTaskTracker favicon_tracker_;
  base::WeakPtrFactory<LibraryView> weak_factory_{this};
};

BEGIN_METADATA(LibraryView)
END_METADATA

class Library : public ui::EventObserver {
 public:
  explicit Library(BrowserView* browser_view) : browser_view_(browser_view) {
    overlay_ = std::make_unique<Overlay>(
        browser_view,
        std::make_unique<LibraryView>(
            base::BindRepeating(&Library::Close, base::Unretained(this)),
            base::BindRepeating(&Library::Resize, base::Unretained(this))),
        "MoriLibrary");
  }

  void Toggle() {
    if (overlay_->IsVisible()) {
      Close();
      return;
    }
    if (base::TimeTicks::Now() - closed_at_ < kReopenGuard) {
      return;
    }
    open_ = true;
    View()->Open(browser_view_->browser());
    Resize();
    View()->FocusSearch();
    monitor_ = MonitorWindow(
        this, browser_view_->GetWidget(),
        {ui::EventType::kKeyPressed, ui::EventType::kMousePressed});
  }

  void Close() {
    if (!open_) {
      return;
    }
    open_ = false;
    monitor_.reset();
    overlay_->Hide();
    closed_at_ = base::TimeTicks::Now();
    if (content::WebContents* contents = browser_view_->browser()
                                             ->GetTabStripModel()
                                             ->GetActiveWebContents()) {
      contents->Focus();
    }
  }

  // ui::EventObserver:
  void OnEvent(const ui::Event& event) override {
    if (event.IsKeyEvent()) {
      if (event.AsKeyEvent()->key_code() == ui::VKEY_ESCAPE) {
        PostClose();
      }
      return;
    }
    // A press outside the library closes it.
    if (!IsEventInChildWidget(event, View()->GetWidget())) {
      PostClose();
    }
  }

 private:
  LibraryView* View() {
    return static_cast<LibraryView*>(overlay_->contents());
  }

  void PostClose() {
    base::SequencedTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&Library::Close, weak_factory_.GetWeakPtr()));
  }

  // Beside the sidebar, near its top (Mori 3: 44px down, 4px out).
  void Resize() {
    if (!open_ || !browser_view_->browser()) {
      return;
    }
    const gfx::Rect area = browser_view_->GetLocalBounds();
    const int width = kWidth + kShadowInsets.width();
    const int height = std::min(View()->HeightForWidth(width),
                                area.height() + kShadowInsets.height());
    views::View* contents = browser_view_->contents_container();
    const int left = contents ? views::View::ConvertRectToTarget(
                                    contents, browser_view_,
                                    contents->GetLocalBounds())
                                    .x()
                              : area.x();
    overlay_->Show(gfx::Rect(left + 4 - kShadowInsets.left(),
                             area.y() + 44 - kShadowInsets.top(), width,
                             height));
  }

  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<Overlay> overlay_;
  std::unique_ptr<views::EventMonitor> monitor_;
  base::TimeTicks closed_at_;
  bool open_ = false;
  base::WeakPtrFactory<Library> weak_factory_{this};
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::Library*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(Library, kLibraryKey)
}  // namespace

void ToggleLibrary(BrowserWindowInterface* browser) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget()) {
    return;
  }
  Library* library = browser_view->GetProperty(kLibraryKey);
  if (!library) {
    library = browser_view->SetProperty(kLibraryKey,
                                        std::make_unique<Library>(browser_view));
  }
  library->Toggle();
}

}  // namespace mori
