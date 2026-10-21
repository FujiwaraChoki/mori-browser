#include "chrome/browser/ui/views/mori/mori_menu.h"

#include <array>
#include <memory>
#include <optional>
#include <utility>
#include <string_view>
#include <vector>

#include "base/functional/callback_helpers.h"
#include "base/containers/fixed_flat_map.h"
#include "base/functional/bind.h"
#include "base/i18n/case_conversion.h"
#include "base/strings/string_split.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/sequenced_task_runner.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/bookmarks/bookmark_model_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/resource_coordinator/lifecycle_unit_state.mojom.h"
#include "chrome/browser/resource_coordinator/tab_lifecycle_unit_external.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_tabstrip.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_enums.h"
#include "chrome/browser/ui/tabs/tab_muted_utils.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_app_menu.h"
#include "chrome/browser/ui/views/mori/mori_command_ids.h"
#include "chrome/browser/ui/views/mori/mori_folders.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_peek.h"
#include "chrome/browser/ui/views/mori/mori_tab_names.h"
#include "chrome/browser/ui/views/mori/mori_boosts.h"
#include "chrome/browser/ui/views/mori/mori_toast.h"
#include "components/bookmarks/browser/bookmark_model.h"
#include "components/bookmarks/browser/bookmark_utils.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/context_menu_params.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/reload_type.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/accelerators/accelerator.h"
#include "ui/base/clipboard/scoped_clipboard_writer.h"
#include "ui/base/models/image_model.h"
#include "ui/color/color_id.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/menus/simple_menu_model.h"
#include "ui/views/controls/menu/menu_runner.h"
#include "url/gurl.h"

namespace mori {
namespace {

// IDs of the items RestylePageMenu adds (their callbacks do the work).
constexpr int kMenuItemFirst = 56980;
constexpr int kMenuItemLast = 57019;

ui::ImageModel Icon(std::string_view lucide) {
  return ui::ImageModel::FromVectorIcon(LucideIcon(lucide), ui::kColorMenuIcon,
                                        15);
}

void CopyText(BrowserWindowInterface* browser,
              const std::u16string& text,
              const std::u16string& toast) {
  ui::ScopedClipboardWriter(ui::ClipboardBuffer::kCopyPaste).WriteText(text);
  ShowToast(browser, toast);
}

// Mori 3's bookmark toggle: no dialog, a toast says what happened.
void ToggleBookmark(BrowserWindowInterface* browser,
                    const GURL& url,
                    const std::u16string& title) {
  bookmarks::BookmarkModel* bookmarks =
      BookmarkModelFactory::GetForBrowserContext(browser->GetProfile());
  if (!bookmarks || !bookmarks->loaded() || !url.is_valid()) {
    return;
  }
  if (bookmarks->IsBookmarked(url)) {
    bookmarks::RemoveAllBookmarks(bookmarks, url, FROM_HERE);
    ShowToast(browser, u"Removed bookmark");
  } else {
    bookmarks::AddIfNotBookmarked(bookmarks, url, title);
    ShowToast(browser, u"Bookmarked");
  }
}

bool IsBookmarked(BrowserWindowInterface* browser, const GURL& url) {
  bookmarks::BookmarkModel* bookmarks =
      BookmarkModelFactory::GetForBrowserContext(browser->GetProfile());
  return bookmarks && bookmarks->loaded() && bookmarks->IsBookmarked(url);
}

// A menu that owns itself while it's open: items are Chromium commands, run
// through the browser, or callbacks.
class MenuHost : public ui::SimpleMenuModel::Delegate {
 public:
  explicit MenuHost(BrowserWindowInterface* browser)
      : browser_(browser), model_(this) {}

  ui::SimpleMenuModel* model() { return &model_; }

  void Command(int command_id, std::u16string label, std::string_view icon) {
    model_.AddItemWithIcon(command_id, label, Icon(icon));
  }

  // `accelerator_of`: the command whose shortcut the row shows.
  void Item(std::u16string label,
            std::string_view icon,
            base::RepeatingClosure run,
            bool enabled = true,
            int accelerator_of = 0) {
    const int id = kFirstCallbackId + static_cast<int>(callbacks_.size());
    callbacks_.push_back({std::move(run), enabled, accelerator_of});
    model_.AddItemWithIcon(id, label, Icon(icon));
  }

  // A submenu, filled with SubItem.
  ui::SimpleMenuModel* SubMenu(std::u16string label, std::string_view icon) {
    submenus_.push_back(std::make_unique<ui::SimpleMenuModel>(this));
    const int id = kFirstCallbackId + static_cast<int>(callbacks_.size());
    callbacks_.push_back({base::DoNothing(), true, 0});
    model_.AddSubMenuWithIcon(id, label, submenus_.back().get(), Icon(icon));
    return submenus_.back().get();
  }

  void SubItem(ui::SimpleMenuModel* menu,
               std::u16string label,
               std::string_view icon,
               base::RepeatingClosure run) {
    const int id = kFirstCallbackId + static_cast<int>(callbacks_.size());
    callbacks_.push_back({std::move(run), true, 0});
    menu->AddItemWithIcon(id, label, Icon(icon));
  }

  void Separator() {
    const size_t count = model_.GetItemCount();
    if (count && model_.GetTypeAt(count - 1) != ui::MenuModel::TYPE_SEPARATOR) {
      model_.AddSeparator(ui::NORMAL_SEPARATOR);
    }
  }

  void Run(views::Widget* parent,
           const gfx::Point& point,
           ui::mojom::MenuSourceType source_type) {
    if (model_.GetItemCount() &&
        model_.GetTypeAt(model_.GetItemCount() - 1) ==
            ui::MenuModel::TYPE_SEPARATOR) {
      model_.RemoveItemAt(model_.GetItemCount() - 1);
    }
    runner_ = std::make_unique<views::MenuRunner>(
        &model_,
        views::MenuRunner::HAS_MNEMONICS | views::MenuRunner::CONTEXT_MENU,
        base::BindRepeating(&MenuHost::OnClosed, base::Unretained(this)));
    runner_->RunMenuAt(parent, nullptr, gfx::Rect(point, gfx::Size()),
                       views::MenuAnchorPosition::kTopLeft, source_type);
  }

  // ui::SimpleMenuModel::Delegate:
  bool IsCommandIdEnabled(int command_id) const override {
    if (const Callback* callback = CallbackFor(command_id)) {
      return callback->enabled;
    }
    return IsMoriCommand(command_id) ||
           chrome::IsCommandEnabled(browser_, command_id);
  }
  void ExecuteCommand(int command_id, int event_flags) override {
    if (const Callback* callback = CallbackFor(command_id)) {
      callback->run.Run();
      return;
    }
    chrome::ExecuteCommand(browser_, command_id);
  }
  bool GetAcceleratorForCommandId(int command_id,
                                  ui::Accelerator* accelerator) const override {
    if (const Callback* callback = CallbackFor(command_id)) {
      command_id = callback->accelerator_of;
    }
    BrowserView* view = BrowserView::GetBrowserViewForBrowser(browser_);
    return command_id && view && view->GetAccelerator(command_id, accelerator);
  }

 private:
  static constexpr int kFirstCallbackId = 1;

  struct Callback {
    base::RepeatingClosure run;
    bool enabled;
    int accelerator_of;
  };

  const Callback* CallbackFor(int command_id) const {
    const int index = command_id - kFirstCallbackId;
    return index >= 0 && index < static_cast<int>(callbacks_.size())
               ? &callbacks_[index]
               : nullptr;
  }

  void OnClosed() {
    // The runner is still on the stack.
    base::SequencedTaskRunner::GetCurrentDefault()->DeleteSoon(FROM_HERE,
                                                              this);
  }

  const raw_ptr<BrowserWindowInterface> browser_;
  ui::SimpleMenuModel model_;
  std::vector<Callback> callbacks_;
  std::vector<std::unique_ptr<ui::SimpleMenuModel>> submenus_;
  std::unique_ptr<views::MenuRunner> runner_;
};

// Runs `run` on `tab` if it's still open.
base::RepeatingClosure OnTab(
    tabs::TabInterface* tab,
    base::RepeatingCallback<void(tabs::TabInterface*)> run) {
  return base::BindRepeating(
      [](tabs::TabHandle handle,
         base::RepeatingCallback<void(tabs::TabInterface*)> run) {
        if (tabs::TabInterface* tab = handle.Get()) {
          run.Run(tab);
        }
      },
      tab->GetHandle(), std::move(run));
}

// The tabs that aren't favorites, in sidebar order.
std::vector<tabs::TabHandle> UnpinnedTabs(TabStripModel* model) {
  std::vector<tabs::TabHandle> out;
  for (int i = model->IndexOfFirstNonPinnedTab(); i < model->count(); ++i) {
    out.push_back(model->GetTabAtIndex(i)->GetHandle());
  }
  return out;
}

void CloseTabs(TabStripModel* model, const std::vector<tabs::TabHandle>& tabs) {
  for (const tabs::TabHandle& handle : tabs) {
    tabs::TabInterface* tab = handle.Get();
    const int index = tab ? model->GetIndexOfTab(tab) : TabStripModel::kNoTab;
    if (index != TabStripModel::kNoTab) {
      model->CloseWebContentsAt(index,
                                TabCloseTypes::CLOSE_USER_GESTURE |
                                    TabCloseTypes::CLOSE_CREATE_HISTORICAL_TAB);
    }
  }
}

// "Open in Split View": `tab` beside the one on screen.
void SplitWithActive(BrowserWindowInterface* browser, tabs::TabInterface* tab) {
  TabStripModel* model = browser->GetTabStripModel();
  const int index = model->GetIndexOfTab(tab);
  if (index != TabStripModel::kNoTab &&
      model->IsContextMenuCommandEnabled(index,
                                         TabStripModel::CommandAddToSplit)) {
    model->ExecuteContextMenuCommand(index, TabStripModel::CommandAddToSplit);
  }
}

// Mori 3's capitalisation of Chromium's sentence-case labels.
std::u16string TitleCase(const std::u16string& label) {
  static constexpr auto kSmall = std::to_array<std::u16string_view>(
      {u"a", u"an", u"and", u"as", u"at", u"by", u"for", u"from", u"in",
       u"into", u"of", u"on", u"or", u"the", u"to", u"via", u"with"});
  std::vector<std::u16string> words = base::SplitString(
      label, u" ", base::KEEP_WHITESPACE, base::SPLIT_WANT_ALL);
  for (size_t i = 0; i < words.size(); ++i) {
    std::u16string& word = words[i];
    if (word.empty() || (i > 0 && std::ranges::find(kSmall, word) !=
                                      kSmall.end())) {
      continue;
    }
    word[0] = base::i18n::ToUpper(word.substr(0, 1))[0];
  }
  return base::JoinString(words, u" ");
}

struct PageItem {
  std::u16string_view label;  // Empty: Chromium's, in Title Case.
  std::string_view icon;
};

// Mori 3's labels and icons for Chromium's page menu items.
const PageItem* PageItemFor(int command_id) {
  static constexpr auto kItems = base::MakeFixedFlatMap<int, PageItem>({
      {IDC_BACK, {u"Back", "arrow-left"}},
      {IDC_FORWARD, {u"Forward", "arrow-right"}},
      {IDC_RELOAD, {u"Reload", "rotate-cw"}},
      {IDC_SAVE_PAGE, {u"Save Page As…", "save"}},
      {IDC_PRINT, {u"Print…", "printer"}},
      {IDC_CONTENT_CONTEXT_OPEN_IN_READING_MODE,
       {u"Reader View", "book-open-text"}},
      {IDC_VIEW_SOURCE, {u"View Page Source", "file-code"}},
      {IDC_CONTENT_CONTEXT_INSPECTELEMENT, {u"Inspect", "code"}},
      {IDC_CONTENT_CONTEXT_OPENLINKNEWTAB,
       {u"Open Link in New Tab", "square-arrow-out-up-right"}},
      {IDC_CONTENT_CONTEXT_OPENLINKNEWWINDOW,
       {u"Open Link in New Window", "app-window"}},
      {IDC_CONTENT_CONTEXT_OPENLINKOFFTHERECORD,
       {u"Open Link in Private Window", "hat-glasses"}},
      {IDC_CONTENT_CONTEXT_OPENLINKSPLITVIEW,
       {u"Open Link in Split View", "columns-2"}},
      {IDC_CONTENT_CONTEXT_OPENLINKINPROFILE, {u"", "user"}},
      {IDC_CONTENT_CONTEXT_SAVELINKAS, {u"Download Linked File", "download"}},
      {IDC_CONTENT_CONTEXT_COPYLINKLOCATION, {u"Copy Link", "link"}},
      {IDC_CONTENT_CONTEXT_COPYLINKTEXT, {u"Copy Link Text", "type"}},
      {IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB,
       {u"Open Image in New Tab", "image"}},
      {IDC_CONTENT_CONTEXT_OPEN_ORIGINAL_IMAGE_NEW_TAB,
       {u"Open Original Image in New Tab", "image"}},
      {IDC_CONTENT_CONTEXT_SAVEIMAGEAS, {u"Save Image As…", "download"}},
      {IDC_CONTENT_CONTEXT_COPYIMAGE, {u"Copy Image", "copy"}},
      {IDC_CONTENT_CONTEXT_COPYIMAGELOCATION,
       {u"Copy Image Address", "link"}},
      {IDC_CONTENT_CONTEXT_SEARCHWEBFORIMAGE, {u"", "search"}},
      {IDC_CONTENT_CONTEXT_OPENAVNEWTAB,
       {u"Open Media in New Tab", "square-arrow-out-up-right"}},
      {IDC_CONTENT_CONTEXT_SAVEAVAS, {u"Save Media As…", "download"}},
      {IDC_CONTENT_CONTEXT_COPYAVLOCATION, {u"Copy Media Address", "link"}},
      {IDC_CONTENT_CONTEXT_SAVEVIDEOFRAMEAS,
       {u"Save Video Frame As…", "image"}},
      {IDC_CONTENT_CONTEXT_COPYVIDEOFRAME, {u"Copy Video Frame", "copy"}},
      {IDC_CONTENT_CONTEXT_LOOP, {u"Loop", "repeat"}},
      {IDC_CONTENT_CONTEXT_CONTROLS,
       {u"Show Controls", "sliders-horizontal"}},
      {IDC_CONTENT_CONTEXT_PICTUREINPICTURE,
       {u"Picture in Picture", "picture-in-picture-2"}},
      {IDC_CONTENT_CONTEXT_UNDO, {u"Undo", "undo-2"}},
      {IDC_CONTENT_CONTEXT_REDO, {u"Redo", "redo-2"}},
      {IDC_CONTENT_CONTEXT_CUT, {u"Cut", "scissors"}},
      {IDC_CONTENT_CONTEXT_COPY, {u"Copy", "copy"}},
      {IDC_CONTENT_CONTEXT_PASTE, {u"Paste", "clipboard-paste"}},
      {IDC_CONTENT_CONTEXT_PASTE_AND_MATCH_STYLE,
       {u"Paste as Plain Text", "clipboard-paste"}},
      {IDC_CONTENT_CONTEXT_SELECTALL, {u"Select All", "square-dashed"}},
      {IDC_CONTENT_CONTEXT_SEARCHWEBFOR, {u"", "search"}},
      {IDC_CONTENT_CONTEXT_SEARCHWEBFORNEWTAB, {u"", "search"}},
      {IDC_CONTENT_CONTEXT_GOTOURL, {u"", "square-arrow-out-up-right"}},
      {IDC_CONTENT_CONTEXT_COPYLINKTOTEXT,
       {u"Copy Link to Highlight", "link"}},
      {IDC_CONTENT_CONTEXT_TRANSLATE, {u"", "languages"}},
      {IDC_SPELLCHECK_ADD_TO_DICTIONARY, {u"Add to Dictionary", "book-plus"}},
      {IDC_CONTENT_CONTEXT_GENERATEPASSWORD, {u"", "key-round"}},
      {IDC_CONTENT_CONTEXT_GENERATE_QR_CODE,
       {u"Create QR Code", "qr-code"}},
      {IDC_CONTENT_CONTEXT_SHARING_SUBMENU, {u"", "share-2"}},
      {IDC_CONTENT_CONTEXT_RELOADFRAME, {u"Reload Frame", "rotate-cw"}},
      {IDC_CONTENT_CONTEXT_VIEWFRAMESOURCE,
       {u"View Frame Source", "file-code"}},
  });
  auto it = kItems.find(command_id);
  return it == kItems.end() ? nullptr : &it->second;
}

// Chromium's own items, whose labels Mori capitalises (not suggestions,
// extensions', pages' or profiles' items, which are names).
bool IsChromiumItem(int command_id) {
  return (command_id >= IDC_BACK && command_id < IDC_SPELLCHECK_SUGGESTION_0) ||
         (command_id >= 50000 &&
          command_id < IDC_CONTENT_CONTEXT_PROTOCOL_HANDLER_FIRST);
}

// Adds an item to the page menu that runs `run`.
class PageMenuEditor {
 public:
  explicit PageMenuEditor(ui::SimpleMenuModel* model) : model_(model) {}

  // Inserts at `index`; returns the index after it.
  size_t Insert(size_t index,
                std::u16string label,
                std::string_view icon,
                base::RepeatingClosure run) {
    if (next_id_ > kMenuItemLast) {
      return index;
    }
    model_->InsertItemAt(index, next_id_++, label);
    model_->SetIcon(index, Icon(icon));
    model_->SetExecuteCallbackAt(
        index, base::BindRepeating([](base::RepeatingClosure run,
                                      int event_flags) { run.Run(); },
                                   std::move(run)));
    return index + 1;
  }

 private:
  const raw_ptr<ui::SimpleMenuModel> model_;
  int next_id_ = kMenuItemFirst;
};

void RemoveCommand(ui::SimpleMenuModel* model, int command_id) {
  if (std::optional<size_t> index = model->GetIndexOfCommandId(command_id)) {
    model->RemoveItemAt(*index);
  }
}

// No separator first, last or twice in a row.
void TidySeparators(ui::SimpleMenuModel* model) {
  for (size_t i = 0; i < model->GetItemCount();) {
    const bool separator =
        model->GetTypeAt(i) == ui::MenuModel::TYPE_SEPARATOR;
    const bool redundant =
        separator &&
        (i == 0 || i + 1 == model->GetItemCount() ||
         model->GetTypeAt(i + 1) == ui::MenuModel::TYPE_SEPARATOR);
    if (redundant) {
      model->RemoveItemAt(i);
    } else {
      ++i;
    }
  }
}

}  // namespace

void ShowTabMenu(BrowserWindowInterface* browser,
                 tabs::TabInterface* tab,
                 const gfx::Point& point,
                 ui::mojom::MenuSourceType source_type,
                 views::Widget* parent) {
  TabStripModel* model = browser->GetTabStripModel();
  content::WebContents* contents = tab->GetContents();
  const GURL url = contents->GetLastCommittedURL();
  const bool is_web = url.SchemeIsHTTPOrHTTPS();
  const bool favorite = tab->IsPinned();
  const bool selected = model->GetActiveTab() == tab;
  auto* host = new MenuHost(browser);

  // Mori 3's tab menu (tab_entries in menus.rs), in its order.
  host->Item(u"New Tab Below", "plus",
             OnTab(tab, base::BindRepeating(
                            [](BrowserWindowInterface* b,
                               tabs::TabInterface* t) {
                              TabStripModel* m = b->GetTabStripModel();
                              chrome::AddTabAt(b, GURL(),
                                               m->GetIndexOfTab(t) + 1,
                                               /*foreground=*/true,
                                               t->GetGroup());
                              chrome::ExecuteCommand(b, IDC_FOCUS_LOCATION);
                            },
                            browser)));
  host->Separator();
  host->Item(u"Reload Tab", "rotate-cw",
             OnTab(tab, base::BindRepeating([](tabs::TabInterface* t) {
                     t->GetContents()->GetController().Reload(
                         content::ReloadType::NORMAL, true);
                   })));
  if (!contents->WasDiscarded()) {
    const bool muted = contents->IsAudioMuted();
    host->Item(muted ? u"Unmute Tab" : u"Mute Tab", "volume-x",
               OnTab(tab, base::BindRepeating(
                              [](bool mute, tabs::TabInterface* t) {
                                SetTabAudioMuted(t->GetContents(), mute,
                                                 TabMutedReason::kAudioIndicator,
                                                 std::string());
                              },
                              !muted)));
  }
  host->Item(favorite ? u"Remove from Favorites" : u"Add to Favorites", "star",
             OnTab(tab, base::BindRepeating(
                            [](BrowserWindowInterface* b,
                               tabs::TabInterface* t) {
                              TabStripModel* m = b->GetTabStripModel();
                              m->SetTabPinned(m->GetIndexOfTab(t),
                                              !t->IsPinned());
                            },
                            browser)));
  host->Item(u"Duplicate Tab", "copy",
             OnTab(tab, base::BindRepeating(
                            [](BrowserWindowInterface* b,
                               tabs::TabInterface* t) {
                              TabStripModel* m = b->GetTabStripModel();
                              m->ExecuteContextMenuCommand(
                                  m->GetIndexOfTab(t),
                                  TabStripModel::CommandDuplicate);
                            },
                            browser)));
  if (is_web) {
    const std::u16string title = contents->GetTitle();
    host->Item(IsBookmarked(browser, url) ? u"Remove Bookmark"
                                          : u"Bookmark Tab",
               "bookmark",
               base::BindRepeating(&ToggleBookmark, browser, url, title));
    host->Item(u"Copy Link", "link",
               base::BindRepeating(&CopyText, browser,
                                   base::UTF8ToUTF16(url.spec()),
                                   u"Copied link"));
  }
  host->Item(u"Rename…", "pencil",
             OnTab(tab, base::BindRepeating(&RenameTab, browser)));
  if (GetTabName(contents)) {
    host->Item(u"Reset Name", "rotate-ccw",
               OnTab(tab, base::BindRepeating([](tabs::TabInterface* t) {
                       SetTabName(t, u"");
                     })));
  }

  host->Separator();
  if (!selected && !tab->IsSplit()) {
    host->Item(u"Open in Split View", "columns-2",
               OnTab(tab, base::BindRepeating(&SplitWithActive, browser)));
  }
  if (tab->IsSplit()) {
    host->Item(u"Close Split View", "columns-2",
               OnTab(tab, base::BindRepeating(
                              [](BrowserWindowInterface* b,
                                 tabs::TabInterface* t) {
                                if (t->GetSplit()) {
                                  b->GetTabStripModel()->RemoveSplit(
                                      *t->GetSplit());
                                }
                              },
                              browser)));
  }
  // Mori 3's folder items (Zen's folders, mori_folders.h).
  if (!favorite) {
    const std::optional<FolderInfo> in_folder = GetTabFolder(tab);
    for (const FolderInfo& folder : ListFolders(browser)) {
      if (in_folder && in_folder->id == folder.id) {
        continue;
      }
      host->Item(u"Add to " + folder.name, "folder",
                 OnTab(tab, base::BindRepeating(
                                [](BrowserWindowInterface* b, std::string id,
                                   tabs::TabInterface* t) {
                                  AddTabToFolder(b, t, id);
                                },
                                browser, folder.id)));
    }
    host->Item(u"New Folder with Tab", "folder-plus",
               OnTab(tab, base::BindRepeating(&NewFolderWithTab, browser)));
    if (in_folder) {
      host->Item(u"Remove from Folder", "minus",
                 OnTab(tab, base::BindRepeating(&RemoveTabFromFolder, browser)));
    }
  }
  host->Item(u"Select All Tabs", "list-checks",
             OnTab(tab, base::BindRepeating(
                            [](BrowserWindowInterface* b,
                               tabs::TabInterface* t) {
                              TabStripModel* m = b->GetTabStripModel();
                              for (const tabs::TabHandle& handle :
                                   UnpinnedTabs(m)) {
                                m->SelectTabAt(m->GetIndexOfTab(handle.Get()));
                              }
                            },
                            browser)));
  host->Item(u"Move to New Window", "app-window",
             OnTab(tab, base::BindRepeating(
                            [](BrowserWindowInterface* b,
                               tabs::TabInterface* t) {
                              TabStripModel* m = b->GetTabStripModel();
                              m->ExecuteContextMenuCommand(
                                  m->GetIndexOfTab(t),
                                  TabStripModel::CommandMoveTabsToNewWindow);
                            },
                            browser)));
  if (selected && is_web) {
    host->Separator();
    host->Command(kCommandBoostSite, u"Boost This Site…", "zap");
    host->Item(u"Zap an Element", "eraser",
               base::BindRepeating(&ZapElement, browser));
  }

  host->Separator();
  if (!selected && !favorite && !contents->WasDiscarded() &&
      !contents->IsCurrentlyAudible()) {
    host->Item(u"Sleep Tab", "moon",
               OnTab(tab, base::BindRepeating([](tabs::TabInterface* t) {
                       if (auto* lifecycle = resource_coordinator::
                               TabLifecycleUnitExternal::FromWebContents(
                                   t->GetContents())) {
                         lifecycle->DiscardTab(
                             mojom::LifecycleUnitDiscardReason::EXTERNAL);
                       }
                     })));
  }
  std::vector<tabs::TabHandle> others = UnpinnedTabs(model);
  std::erase(others, tab->GetHandle());
  host->Item(u"Close Other Tabs", "x",
             base::BindRepeating(&CloseTabs, model, others),
             /*enabled=*/!others.empty());
  if (!favorite) {
    std::vector<tabs::TabHandle> below;
    const int index = model->GetIndexOfTab(tab);
    for (const tabs::TabHandle& other : others) {
      if (model->GetIndexOfTab(other.Get()) > index) {
        below.push_back(other);
      }
    }
    host->Item(u"Close Tabs Below", "x",
               base::BindRepeating(&CloseTabs, model, below),
               /*enabled=*/!below.empty());
  }
  host->Item(u"Close Tab", "x",
             base::BindRepeating(&CloseTabs, model,
                                 std::vector<tabs::TabHandle>{tab->GetHandle()}),
             /*enabled=*/true, selected ? IDC_CLOSE_TAB : 0);
  host->Command(IDC_RESTORE_TAB, u"Reopen Closed Tab", "rotate-ccw");
  host->Run(parent, point, source_type);
}

bool ShowSidebarMenu(BrowserWindowInterface* browser,
                     const gfx::Point& point,
                     ui::mojom::MenuSourceType source_type,
                     views::Widget* parent) {
  if (browser->GetType() != BrowserWindowInterface::TYPE_NORMAL) {
    return false;
  }
  auto* host = new MenuHost(browser);
  BuildAppMenu(host->model(), browser);
  host->Run(parent, point, source_type);
  return true;
}

void ShowFolderMenu(BrowserWindowInterface* browser,
                    const std::string& folder_id,
                    const gfx::Point& point,
                    ui::mojom::MenuSourceType source_type,
                    views::Widget* parent) {
  const std::optional<FolderInfo> folder = GetFolder(folder_id);
  if (!folder) {
    return;
  }
  auto* host = new MenuHost(browser);
  // Mori 3's folder menu (folder_entries in menus.rs), Zen's Delete Folder.
  host->Item(u"New Tab in Folder", "plus",
             base::BindRepeating(&NewTabInFolder, browser, folder_id));
  host->Separator();
  host->Item(u"Rename Folder", "pencil",
             base::BindRepeating(&RenameFolder, browser, folder_id));
  host->Item(folder->open ? u"Collapse Folder" : u"Expand Folder", "folder",
             base::BindRepeating(&ToggleFolder, browser, folder_id));
  // Mori 3's folder symbols (FOLDER_SYMBOLS in sidebar.rs) that Mori's
  // Lucide set has.
  ui::SimpleMenuModel* icons = host->SubMenu(u"Folder Icon", "palette");
  host->SubItem(icons, u"None", "folder",
                base::BindRepeating(&SetFolderIcon, browser, folder_id,
                                    std::string()));
  constexpr std::pair<const char*, const char16_t*> kIcons[] = {
      {"star", u"Star"},         {"heart", u"Heart"},
      {"briefcase", u"Briefcase"}, {"code", u"Code"},
      {"book", u"Book"},         {"music", u"Music"},
      {"palette", u"Palette"},   {"bookmark", u"Bookmark"},
      {"zap", u"Zap"},           {"archive", u"Archive"},
  };
  for (const auto& [icon, label] : kIcons) {
    host->SubItem(icons, label, icon,
                  base::BindRepeating(&SetFolderIcon, browser, folder_id,
                                      std::string(icon)));
  }
  host->Separator();
  host->Item(u"Ungroup Folder", "layers",
             base::BindRepeating(&UngroupFolder, browser, folder_id));
  host->Item(u"Delete Folder", "trash",
             base::BindRepeating(&DeleteFolder, browser, folder_id));
  host->Run(parent, point, source_type);
}

void RestylePageMenu(ui::SimpleMenuModel* model,
                     BrowserWindowInterface* browser,
                     const content::ContextMenuParams& params) {
  RemoveCommand(model, IDC_ROUTE_MEDIA);

  for (size_t i = 0; i < model->GetItemCount(); ++i) {
    if (model->GetTypeAt(i) == ui::MenuModel::TYPE_SEPARATOR ||
        model->GetTypeAt(i) == ui::MenuModel::TYPE_TITLE) {
      continue;
    }
    const int id = model->GetCommandIdAt(i);
    const PageItem* item = PageItemFor(id);
    if (item && !item->label.empty()) {
      model->SetLabel(i, std::u16string(item->label));
    } else if (IsChromiumItem(id)) {
      const std::u16string label = model->GetLabelAt(i);
      if (label.find_first_of(u"\"“”'") == std::u16string::npos) {
        model->SetLabel(i, TitleCase(label));
      }
    }
    if (item) {
      model->SetIcon(i, Icon(item->icon));
    }
  }

  if (!browser) {
    TidySeparators(model);
    return;
  }
  PageMenuEditor editor(model);
  TabStripModel* tabs = browser->GetTabStripModel();

  // Links: Peek, after "Open Link in New Tab".
  if (std::optional<size_t> index =
          model->GetIndexOfCommandId(IDC_CONTENT_CONTEXT_OPENLINKNEWTAB);
      index && params.link_url.is_valid()) {
    editor.Insert(*index + 1, u"Open Link in Peek", "eye",
                       base::BindRepeating(
                           [](BrowserWindowInterface* b, const GURL& url) {
                             ShowPeek(b, url);
                           },
                           browser, params.link_url));
  }

  // The page: bookmark and copy its address after Reload, capture after
  // Reader View (Mori 3's page group).
  if (std::optional<size_t> reload = model->GetIndexOfCommandId(IDC_RELOAD)) {
    const GURL page = params.page_url;
    size_t at = *reload + 1;
    if (at < model->GetItemCount() &&
        model->GetTypeAt(at) == ui::MenuModel::TYPE_SEPARATOR) {
      ++at;
    } else {
      model->InsertSeparatorAt(at++, ui::NORMAL_SEPARATOR);
    }
    if (page.SchemeIsHTTPOrHTTPS()) {
      content::WebContents* contents =
          tabs->GetActiveTab() ? tabs->GetActiveTab()->GetContents() : nullptr;
      const std::u16string title =
          contents ? contents->GetTitle() : base::UTF8ToUTF16(page.spec());
      at = editor.Insert(
          at, IsBookmarked(browser, page) ? u"Remove Bookmark" : u"Bookmark Page",
          "bookmark", base::BindRepeating(&ToggleBookmark, browser, page, title));
      at = editor.Insert(at, u"Copy Page Link", "link",
                         base::BindRepeating(&CopyText, browser,
                                             base::UTF8ToUTF16(page.spec()),
                                             u"Copied link"));
    }
    if (std::optional<size_t> reader = model->GetIndexOfCommandId(
            IDC_CONTENT_CONTEXT_OPEN_IN_READING_MODE)) {
      at = *reader + 1;
    }
    editor.Insert(at, u"Capture Visible Tab", "camera",
                  base::BindRepeating(
                      [](BrowserWindowInterface* b) {
                        chrome::ExecuteCommand(b, IDC_SHARING_HUB_SCREENSHOT);
                      },
                      browser));
  }
  TidySeparators(model);
}

void ToggleActiveTabBookmark(BrowserWindowInterface* browser) {
  if (tabs::TabInterface* tab = browser->GetTabStripModel()->GetActiveTab()) {
    content::WebContents* contents = tab->GetContents();
    ToggleBookmark(browser, contents->GetLastCommittedURL(),
                   contents->GetTitle());
  }
}

bool IsMoriMenuItem(int command_id) {
  return command_id >= kMenuItemFirst && command_id <= kMenuItemLast;
}

bool HidesPageMenuShortcut(int command_id) {
  return command_id == IDC_BACK || command_id == IDC_FORWARD ||
         command_id == IDC_SAVE_PAGE ||
         command_id == IDC_CONTENT_CONTEXT_OPEN_IN_READING_MODE ||
         command_id == IDC_VIEW_SOURCE;
}

}  // namespace mori
