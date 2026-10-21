#include "chrome/browser/ui/views/mori/mori_tab_switcher.h"

#include <algorithm>
#include <memory>
#include <vector>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/timer/timer.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/thumbnails/thumbnail_image.h"
#include "chrome/browser/ui/thumbnails/thumbnail_tab_helper.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "components/favicon/content/content_favicon_driver.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/base/models/image_model.h"
#include "ui/color/color_provider.h"
#include "ui/events/event.h"
#include "ui/events/event_observer.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/views/border.h"
#include "ui/views/controls/image_view.h"
#include "ui/views/controls/label.h"
#include "ui/views/event_monitor.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/view.h"
#include "ui/views/widget/widget.h"

namespace mori {
namespace {

// Mori 3's switcher (crates/mori/src/ui/tab_switcher.rs).
constexpr base::TimeDelta kRevealDelay = base::Milliseconds(110);
constexpr int kCardWidth = 168;
constexpr int kThumbHeight = 104;
constexpr int kCardGap = 8;
constexpr int kMaxColumns = 6;
constexpr int kPanelPadding = 12;
constexpr auto kShadowInsets = gfx::Insets::TLBR(24, 40, 56, 40);

// One recent tab: its page (thumbnail or favicon) above favicon and title.
class Card : public views::View {
  METADATA_HEADER(Card, views::View)

 public:
  explicit Card(tabs::TabInterface* tab) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical, gfx::Insets(6), 6));
    preview_ = AddChildView(std::make_unique<views::ImageView>());
    preview_->SetPreferredSize(gfx::Size(kCardWidth - 12, kThumbHeight));
    auto* row = AddChildView(std::make_unique<views::View>());
    auto* row_layout = row->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kHorizontal, gfx::Insets::VH(0, 2), 6));
    row_layout->set_cross_axis_alignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    auto* icon = row->AddChildView(std::make_unique<views::ImageView>());
    icon->SetImageSize(gfx::Size(14, 14));
    title_ = row->AddChildView(std::make_unique<views::Label>());
    title_->SetElideBehavior(gfx::ELIDE_TAIL);
    title_->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    row_layout->SetFlexForView(title_, 1);
    content::WebContents* contents = tab->GetContents();
    title_->SetText(contents ? contents->GetTitle() : std::u16string());
    if (contents) {
      if (auto* favicon =
              favicon::ContentFaviconDriver::FromWebContents(contents)) {
        icon->SetImage(ui::ImageModel::FromImage(favicon->GetFavicon()));
        favicon_ = favicon->GetFavicon();
      }
    }
    // The page's last frame, when Chromium has one.
    if (auto* helper = ThumbnailTabHelper::From(tab)) {
      if (scoped_refptr<ThumbnailImage> thumbnail = helper->thumbnail()) {
        subscription_ = thumbnail->Subscribe();
        subscription_->SetUncompressedImageCallback(base::BindRepeating(
            &Card::OnThumbnail, base::Unretained(this)));
        thumbnail->RequestThumbnailImage();
      }
    }
    SetPreferredSize(gfx::Size(kCardWidth, kThumbHeight + 40));
  }

  void SetHighlighted(bool highlighted) {
    highlighted_ = highlighted;
    SchedulePaint();
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    const SkColor frame = GetColorProvider()->GetColor(ui::kColorFrameActive);
    const bool dark = color_utils::IsDark(frame);
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    if (highlighted_) {
      flags.setColor(SkColorSetA(dark ? SK_ColorWHITE : SK_ColorBLACK,
                                 dark ? 0x26 : 0x14));
      canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), 10, flags);
    }
    // The preview's frame: Mori 3's card colour behind the page.
    flags.setColor(dark ? SkColorSetRGB(0x2B, 0x2B, 0x2B)
                        : SkColorSetRGB(0xF2, 0xF2, 0xF2));
    canvas->DrawRoundRect(gfx::RectF(preview_->bounds()), 6, flags);
  }

  void OnThemeChanged() override {
    views::View::OnThemeChanged();
    const SkColor frame = GetColorProvider()->GetColor(ui::kColorFrameActive);
    title_->SetEnabledColor(color_utils::IsDark(frame)
                                ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                                : SkColorSetRGB(0x1D, 0x1F, 0x22));
    if (!has_thumbnail_ && !favicon_.IsEmpty()) {
      // No frame yet: the favicon, large, in the middle.
      preview_->SetImage(ui::ImageModel::FromImage(favicon_));
      preview_->SetImageSize(gfx::Size(32, 32));
    }
  }

 private:
  void OnThumbnail(gfx::ImageSkia image) {
    has_thumbnail_ = true;
    preview_->SetImage(ui::ImageModel::FromImageSkia(image));
    preview_->SetImageSize(gfx::Size(kCardWidth - 12, kThumbHeight));
  }

  bool highlighted_ = false;
  bool has_thumbnail_ = false;
  gfx::Image favicon_;
  raw_ptr<views::ImageView> preview_ = nullptr;
  raw_ptr<views::Label> title_ = nullptr;
  std::unique_ptr<ThumbnailImage::Subscription> subscription_;
};

BEGIN_METADATA(Card)
END_METADATA

class SwitcherPanel : public views::View {
  METADATA_HEADER(SwitcherPanel, views::View)

 public:
  SwitcherPanel() {
    SetBorder(views::CreateEmptyBorder(kShadowInsets + gfx::Insets(kPanelPadding)));
  }

  void SetTabs(const std::vector<tabs::TabInterface*>& tabs) {
    RemoveAllChildViews();
    cards_.clear();
    for (tabs::TabInterface* tab : tabs) {
      cards_.push_back(AddChildView(std::make_unique<Card>(tab)));
    }
  }

  void SetHighlighted(size_t index) {
    for (size_t i = 0; i < cards_.size(); ++i) {
      cards_[i]->SetHighlighted(i == index);
    }
  }

  int Columns() const {
    return std::max<int>(1, std::min<int>(kMaxColumns, cards_.size()));
  }

  gfx::Size CalculatePreferredSize(
      const views::SizeBounds& available_size) const override {
    const int columns = Columns();
    const int rows = (cards_.size() + columns - 1) / columns;
    return gfx::Size(
        columns * kCardWidth + (columns - 1) * kCardGap + GetInsets().width(),
        rows * (kThumbHeight + 40) + (rows - 1) * kCardGap +
            GetInsets().height());
  }

  void Layout(PassKey) override {
    const int columns = Columns();
    for (size_t i = 0; i < cards_.size(); ++i) {
      const int column = i % columns;
      const int row = i / columns;
      cards_[i]->SetBounds(
          GetInsets().left() + column * (kCardWidth + kCardGap),
          GetInsets().top() + row * (kThumbHeight + 40 + kCardGap), kCardWidth,
          kThumbHeight + 40);
    }
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    gfx::Rect popover = GetLocalBounds();
    popover.Inset(kShadowInsets);
    PaintPopover(this, canvas, popover);
  }

 private:
  std::vector<raw_ptr<Card>> cards_;
};

BEGIN_METADATA(SwitcherPanel)
END_METADATA

class TabSwitcher : public ui::EventObserver {
 public:
  explicit TabSwitcher(BrowserView* browser_view)
      : browser_view_(browser_view) {}

  void Step(bool forward) {
    if (!active_) {
      Start(forward);
      return;
    }
    if (order_.empty()) {
      return;
    }
    index_ = (index_ + (forward ? 1 : order_.size() - 1)) % order_.size();
    if (overlay_ && overlay_->IsVisible()) {
      Panel()->SetHighlighted(index_);
    }
  }

  // ui::EventObserver:
  void OnEvent(const ui::Event& event) override {
    if (!active_) {
      return;
    }
    // A pointer event without Ctrl means Ctrl went up unseen (a modifier
    // change alone sends no key event on some Wayland keyboards).
    if (event.IsMouseEvent()) {
      if (event.type() == ui::EventType::kMousePressed) {
        Finish(/*commit=*/false);
      } else if (!(event.flags() & ui::EF_CONTROL_DOWN)) {
        Finish(/*commit=*/true);
      }
      return;
    }
    if (!event.IsKeyEvent()) {
      return;
    }
    const ui::KeyEvent& key = *event.AsKeyEvent();
    if (key.type() == ui::EventType::kKeyPressed &&
        key.key_code() == ui::VKEY_ESCAPE) {
      Finish(/*commit=*/false);
    } else if ((key.type() == ui::EventType::kKeyReleased &&
                key.key_code() == ui::VKEY_CONTROL) ||
               (key.key_code() != ui::VKEY_CONTROL && !key.IsControlDown())) {
      Finish(/*commit=*/true);
    }
  }

 private:
  // Recent tabs, most recent first.
  void Start(bool forward) {
    TabStripModel* model = browser_view_->browser()->GetTabStripModel();
    std::vector<tabs::TabInterface*> tabs;
    for (int i = 0; i < model->count(); ++i) {
      tabs.push_back(model->GetTabAtIndex(i));
    }
    std::ranges::stable_sort(tabs, [model](tabs::TabInterface* a,
                                           tabs::TabInterface* b) {
      if (a == model->GetActiveTab() || b == model->GetActiveTab()) {
        return a == model->GetActiveTab();
      }
      return a->GetContents()->GetLastActiveTimeTicks() >
             b->GetContents()->GetLastActiveTimeTicks();
    });
    if (tabs.size() < 2) {
      return;
    }
    order_.clear();
    for (tabs::TabInterface* tab : tabs) {
      order_.push_back(tab->GetHandle());
    }
    index_ = forward ? 1 : order_.size() - 1;
    active_ = true;
    monitor_ = views::EventMonitor::CreateApplicationMonitor(
        this, browser_view_->GetWidget()->GetNativeWindow(),
        {ui::EventType::kKeyPressed, ui::EventType::kKeyReleased,
         ui::EventType::kMouseMoved, ui::EventType::kMousePressed});
    reveal_.Start(FROM_HERE, kRevealDelay,
                  base::BindOnce(&TabSwitcher::Reveal, base::Unretained(this),
                                 tabs));
  }

  void Reveal(std::vector<tabs::TabInterface*> tabs) {
    if (!active_) {
      return;
    }
    if (!overlay_) {
      overlay_ = std::make_unique<Overlay>(
          browser_view_, std::make_unique<SwitcherPanel>(), "MoriTabSwitcher");
    }
    // The tabs may have changed in the 110ms; drop closed ones.
    std::vector<tabs::TabInterface*> alive;
    for (tabs::TabHandle handle : order_) {
      if (tabs::TabInterface* tab = handle.Get()) {
        alive.push_back(tab);
      }
    }
    Panel()->SetTabs(alive);
    Panel()->SetHighlighted(index_);
    const gfx::Size size = Panel()->GetPreferredSize();
    const gfx::Rect area = browser_view_->GetLocalBounds();
    overlay_->Show(gfx::Rect(area.x() + (area.width() - size.width()) / 2,
                             area.y() + (area.height() - size.height()) / 2,
                             size.width(), size.height()));
  }

  void Finish(bool commit) {
    active_ = false;
    reveal_.Stop();
    monitor_.reset();
    if (overlay_) {
      overlay_->Hide();
    }
    if (!commit || index_ >= order_.size()) {
      return;
    }
    if (tabs::TabInterface* tab = order_[index_].Get()) {
      TabStripModel* model = browser_view_->browser()->GetTabStripModel();
      const int index = model->GetIndexOfTab(tab);
      if (index != TabStripModel::kNoTab) {
        model->ActivateTabAt(index, TabStripUserGestureDetails(
                                        TabStripUserGestureDetails::
                                            GestureType::kKeyboard));
      }
    }
  }

  SwitcherPanel* Panel() {
    return static_cast<SwitcherPanel*>(overlay_->contents());
  }

  raw_ptr<BrowserView> browser_view_;
  bool active_ = false;
  std::vector<tabs::TabHandle> order_;
  size_t index_ = 0;
  std::unique_ptr<views::EventMonitor> monitor_;
  base::OneShotTimer reveal_;
  std::unique_ptr<Overlay> overlay_;
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::TabSwitcher*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(TabSwitcher, kTabSwitcherKey)
}  // namespace

void StepTabSwitcher(BrowserWindowInterface* browser, bool forward) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget()) {
    return;
  }
  TabSwitcher* switcher = browser_view->GetProperty(kTabSwitcherKey);
  if (!switcher) {
    switcher = browser_view->SetProperty(
        kTabSwitcherKey, std::make_unique<TabSwitcher>(browser_view));
  }
  switcher->Step(forward);
}

}  // namespace mori
