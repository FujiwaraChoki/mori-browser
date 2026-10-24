#include "chrome/browser/ui/views/mori/mori_tab_names.h"

#include <memory>
#include <utility>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "base/time/time.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/sessions/session_service.h"
#include "chrome/browser/sessions/session_service_factory.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/tab_ui_helper.h"
#include "chrome/browser/ui/tabs/tab_change_type.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "chrome/browser/ui/views/tabs/common/tab_view.h"
#include "chrome/browser/ui/views/tabs/tab/tab_title.h"
#include "components/sessions/content/session_tab_helper.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "content/public/browser/web_contents_user_data.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/color/color_provider.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/color_utils.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/background.h"
#include "ui/views/border.h"
#include "ui/views/controls/textfield/textfield.h"
#include "ui/views/controls/textfield/textfield_controller.h"
#include "ui/views/focus/focus_manager.h"
#include "ui/views/layout/fill_layout.h"
#include "ui/views/view_tracker.h"
#include "ui/views/view_utils.h"

namespace mori {
namespace {

class TabName : public content::WebContentsUserData<TabName> {
 public:
  TabName(content::WebContents* contents, std::u16string name)
      : content::WebContentsUserData<TabName>(*contents),
        name_(std::move(name)) {}

  const std::u16string& name() const { return name_; }
  void set_name(std::u16string name) { name_ = std::move(name); }

 private:
  friend WebContentsUserData;
  WEB_CONTENTS_USER_DATA_KEY_DECL();

  std::u16string name_;
};

WEB_CONTENTS_USER_DATA_KEY_IMPL(TabName);

void SetName(content::WebContents* contents, const std::u16string& name) {
  if (name.empty()) {
    contents->RemoveUserData(TabName::UserDataKey());
  } else if (TabName* data = TabName::FromWebContents(contents)) {
    data->set_name(name);
  } else {
    TabName::CreateForWebContents(contents, name);
  }
}

// The field over the tab's title.
class RenameField : public views::Textfield,
                    public views::TextfieldController {
  METADATA_HEADER(RenameField, views::Textfield)

 public:
  explicit RenameField(base::RepeatingCallback<void(bool keep)> done)
      : done_(std::move(done)) {
    set_controller(this);
    SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(0, 6)));
    GetViewAccessibility().SetName(u"Tab name");
  }

  void OnThemeChanged() override {
    views::Textfield::OnThemeChanged();
    const bool dark = color_utils::IsDark(
        GetColorProvider()->GetColor(ui::kColorFrameActive));
    // Mori's field colour.
    SetBackground(views::CreateRoundedRectBackground(
        dark ? SkColorSetRGB(0x2F, 0x2F, 0x2F)
             : SkColorSetRGB(0xFF, 0xFF, 0xFF),
        6));
    SetTextColorId(kColorOmniboxText);
  }

  void OnBlur() override {
    views::Textfield::OnBlur();
    // A click elsewhere keeps the name, as Mori. Leaving the window
    // (nothing focused) doesn't.
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&RenameField::FinishIfFocusMoved,
                                  weak_factory_.GetWeakPtr()));
  }

  void MarkOpened() { opened_ = base::TimeTicks::Now(); }

  // views::TextfieldController:
  bool HandleKeyEvent(views::Textfield* sender,
                      const ui::KeyEvent& event) override {
    if (event.type() != ui::EventType::kKeyPressed) {
      return false;
    }
    if (event.key_code() == ui::VKEY_RETURN) {
      done_.Run(true);
      return true;
    }
    if (event.key_code() == ui::VKEY_ESCAPE) {
      done_.Run(false);
      return true;
    }
    return false;
  }

 private:
  void FinishIfFocusMoved() {
    views::FocusManager* focus = GetFocusManager();
    if (!focus || !focus->GetFocusedView() || HasFocus() || !IsDrawn()) {
      return;
    }
    // The double-click's last release lands on the tab, which focuses the
    // page; the field takes the focus back.
    if (base::TimeTicks::Now() - opened_ < base::Milliseconds(600)) {
      RequestFocus();
      return;
    }
    done_.Run(true);
  }

  base::RepeatingCallback<void(bool keep)> done_;
  base::TimeTicks opened_;
  base::WeakPtrFactory<RenameField> weak_factory_{this};
};

BEGIN_METADATA(RenameField)
END_METADATA

// One per window: the overlay holding the field and the tab being named.
class TabRenamer {
 public:
  explicit TabRenamer(BrowserView* browser_view)
      : browser_view_(browser_view) {
    overlay_ = std::make_unique<Overlay>(
        browser_view,
        std::make_unique<RenameField>(base::BindRepeating(
            &TabRenamer::Finish, base::Unretained(this))),
        "MoriTabRename");
  }

  void Begin(TabView* tab_view) {
    tabs::TabInterface* tab =
        const_cast<tabs::TabInterface*>(tab_view->GetTabInterface());
    auto* field = static_cast<RenameField*>(overlay_->contents());
    if (!tab || !field) {
      return;
    }
    tab_ = tab->GetWeakPtr();
    content::WebContents* contents = tab->GetContents();
    std::optional<std::u16string> name = GetTabName(contents);
    field->SetText(name ? *name : contents->GetTitle());
    field->SetFontList(tab_view->title_for_testing()->font_list());
    field->SelectAll(false);
    // Over the title, from just before it to the row's end.
    views::View* title = tab_view->title_for_testing();
    gfx::Rect bounds = views::View::ConvertRectToTarget(
        title, browser_view_, title->GetLocalBounds());
    const gfx::Rect row = views::View::ConvertRectToTarget(
        tab_view, browser_view_, tab_view->GetLocalBounds());
    bounds.set_x(bounds.x() - 6);
    bounds.set_width(row.right() - 4 - bounds.x());
    bounds.set_y(row.y() + 4);
    bounds.set_height(row.height() - 8);
    overlay_->Show(bounds);
    field->MarkOpened();
    field->RequestFocus();
  }

 private:
  void Finish(bool keep) {
    auto* field = static_cast<RenameField*>(overlay_->contents());
    if (!overlay_->IsVisible() || !field) {
      return;
    }
    const std::u16string text =
        std::u16string(base::TrimWhitespace(field->GetText(), base::TRIM_ALL));
    overlay_->Hide();
    tabs::TabInterface* tab = tab_.get();
    if (keep && tab) {
      // The page's own title again: no name of its own.
      SetTabName(tab, text == tab->GetContents()->GetTitle() ? u"" : text);
    }
    if (tab && tab->IsActivated()) {
      tab->GetContents()->Focus();
    }
  }

  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<Overlay> overlay_;
  base::WeakPtr<tabs::TabInterface> tab_;
};

TabView* FindTabView(views::View* root, const tabs::TabInterface* tab) {
  if (auto* tab_view = views::AsViewClass<TabView>(root)) {
    if (tab_view->GetTabInterface() == tab && !tab_view->IsClosing()) {
      return tab_view;
    }
  }
  for (views::View* child : root->children()) {
    if (TabView* found = FindTabView(child, tab)) {
      return found;
    }
  }
  return nullptr;
}

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::TabRenamer*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(TabRenamer, kTabRenamerKey)

void StartTabRename(views::View* view);
}  // namespace

std::optional<std::u16string> GetTabName(
    const content::WebContents* contents) {
  const TabName* data =
      contents ? TabName::FromWebContents(contents) : nullptr;
  return data ? std::optional<std::u16string>(data->name()) : std::nullopt;
}

void SetTabName(tabs::TabInterface* tab, const std::u16string& name) {
  content::WebContents* contents = tab->GetContents();
  SetName(contents, name);
  // Kept in the session; an empty value reads as no name.
  Profile* profile =
      Profile::FromBrowserContext(contents->GetBrowserContext());
  if (SessionService* session =
          SessionServiceFactory::GetForProfile(profile)) {
    const SessionID window_id =
        sessions::SessionTabHelper::IdForWindowContainingTab(contents);
    const SessionID tab_id = sessions::SessionTabHelper::IdForTab(contents);
    if (window_id.is_valid() && tab_id.is_valid()) {
      session->AddTabExtraData(window_id, tab_id, kTabNameKey,
                               base::UTF16ToUTF8(name));
    }
  }
  // The sidebar's rows refresh when TabUIHelper hears of a new page title;
  // a new name is the same news.
  if (TabUIHelper* helper = TabUIHelper::From(tab)) {
    static_cast<content::WebContentsObserver*>(helper)->TitleWasSet(nullptr);
  }
  if (BrowserWindowInterface* browser = tab->GetBrowserWindowInterface()) {
    browser->GetTabStripModel()->NotifyTabChanged(tab, TabChangeType::kAll);
  }
}

void RestoreTabName(content::WebContents* contents,
                    const std::map<std::string, std::string>& extra_data) {
  auto it = extra_data.find(kTabNameKey);
  if (it != extra_data.end() && !it->second.empty()) {
    SetName(contents, base::UTF8ToUTF16(it->second));
  }
}

void BeginTabRename(views::View* view) {
  // Focus can't move while the double-click's press is being handled; the
  // field opens right after it.
  auto tracker = std::make_unique<views::ViewTracker>(view);
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE, base::BindOnce(
                     [](std::unique_ptr<views::ViewTracker> tracker) {
                       if (views::View* v = tracker->view()) {
                         StartTabRename(v);
                       }
                     },
                     std::move(tracker)));
}

namespace {
void StartTabRename(views::View* view) {
  auto* tab_view = views::AsViewClass<TabView>(view);
  if (!tab_view || !tab_view->GetWidget() || tab_view->pinned()) {
    return;
  }
  BrowserView* browser_view =
      BrowserView::GetBrowserViewForNativeWindow(
          tab_view->GetWidget()->GetNativeWindow());
  if (!browser_view) {
    return;
  }
  TabRenamer* renamer = browser_view->GetProperty(kTabRenamerKey);
  if (!renamer) {
    renamer = browser_view->SetProperty(
        kTabRenamerKey, std::make_unique<TabRenamer>(browser_view));
  }
  renamer->Begin(tab_view);
}
}  // namespace

void RenameActiveTab(BrowserWindowInterface* browser) {
  if (tabs::TabInterface* tab = browser->GetTabStripModel()->GetActiveTab()) {
    RenameTab(browser, tab);
  }
}

void RenameTab(BrowserWindowInterface* browser, tabs::TabInterface* tab) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view) {
    return;
  }
  if (TabView* tab_view = FindTabView(browser_view, tab)) {
    BeginTabRename(tab_view);
  }
}

}  // namespace mori
