#include "chrome/browser/ui/views/mori/mori_boosts.h"

#include <algorithm>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <utility>
#include <vector>

#include "base/callback_list.h"
#include "base/files/file_path.h"
#include "base/files/file_util.h"
#include "base/files/important_file_writer.h"
#include "base/functional/bind.h"
#include "base/json/json_reader.h"
#include "base/json/json_writer.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/no_destructor.h"
#include "base/rand_util.h"
#include "base/strings/string_number_conversions.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "base/task/thread_pool.h"
#include "base/values.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_overlay.h"
#include "chrome/common/chrome_isolated_world_ids.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "content/public/browser/web_contents_user_data.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/color/color_provider.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/font_list.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/background.h"
#include "ui/views/border.h"
#include "ui/views/controls/button/label_button.h"
#include "ui/views/controls/button/md_text_button.h"
#include "ui/views/controls/label.h"
#include "ui/views/controls/textarea/textarea.h"
#include "ui/views/controls/textfield/textfield_controller.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/layout/box_layout_view.h"
#include "ui/views/view.h"
#include "ui/views/view_class_properties.h"
#include "url/gurl.h"

namespace mori {
namespace {

struct Boost {
  // Lowercased, without "www."; covers its subdomains too.
  std::string host;
  std::string css;
  std::string js;
  std::vector<std::string> zapped;
  bool enabled = true;

  bool empty() const { return css.empty() && js.empty() && zapped.empty(); }
};

std::string NormalizedHost(const GURL& url) {
  std::string host = base::ToLowerASCII(url.host());
  if (host.starts_with("www.")) {
    host = host.substr(4);
  }
  return host;
}

bool Covers(const Boost& boost, const std::string& host) {
  return host == boost.host || host.ends_with("." + boost.host);
}

// Boosts of one profile, read once and written back on every change.
class BoostStore {
 public:
  static BoostStore* For(Profile* profile) {
    static base::NoDestructor<std::map<base::FilePath,
                                       std::unique_ptr<BoostStore>>>
        stores;
    const base::FilePath path = profile->GetOriginalProfile()->GetPath();
    auto& store = (*stores)[path];
    if (!store) {
      store = std::make_unique<BoostStore>(
          path.AppendASCII("Mori Boosts.json"));
    }
    return store.get();
  }

  explicit BoostStore(base::FilePath path)
      : path_(std::move(path)),
        file_runner_(base::ThreadPool::CreateSequencedTaskRunner(
            {base::MayBlock(), base::TaskShutdownBehavior::BLOCK_SHUTDOWN})) {
    file_runner_->PostTaskAndReplyWithResult(
        FROM_HERE,
        base::BindOnce(
            [](base::FilePath path) {
              std::string json;
              base::ReadFileToString(path, &json);
              return json;
            },
            path_),
        base::BindOnce(&BoostStore::OnLoaded, weak_factory_.GetWeakPtr()));
  }

  bool loaded() const { return loaded_; }

  std::optional<Boost> Find(const GURL& url) const {
    if (!url.SchemeIsHTTPOrHTTPS()) {
      return std::nullopt;
    }
    const std::string host = NormalizedHost(url);
    // The most specific boost wins.
    const Boost* best = nullptr;
    for (const Boost& boost : boosts_) {
      if (Covers(boost, host) &&
          (!best || boost.host.size() > best->host.size())) {
        best = &boost;
      }
    }
    return best ? std::optional<Boost>(*best) : std::nullopt;
  }

  void Put(Boost boost) {
    std::erase_if(boosts_,
                  [&](const Boost& b) { return b.host == boost.host; });
    if (!boost.empty()) {
      boosts_.push_back(std::move(boost));
    }
    Changed();
  }

  void Remove(const std::string& host) {
    std::erase_if(boosts_, [&](const Boost& b) { return b.host == host; });
    Changed();
  }

  base::CallbackListSubscription AddObserver(base::RepeatingClosure closure) {
    return observers_.Add(std::move(closure));
  }

 private:
  void OnLoaded(std::string json) {
    std::optional<base::Value> value = base::JSONReader::Read(
        json, base::JSON_PARSE_CHROMIUM_EXTENSIONS);
    if (value && value->is_list()) {
      for (const base::Value& item : value->GetList()) {
        const base::DictValue* dict = item.GetIfDict();
        const std::string* host = dict ? dict->FindString("host") : nullptr;
        if (!host || host->empty()) {
          continue;
        }
        Boost boost;
        boost.host = *host;
        if (const std::string* css = dict->FindString("css")) {
          boost.css = *css;
        }
        if (const std::string* js = dict->FindString("js")) {
          boost.js = *js;
        }
        if (const base::ListValue* zapped = dict->FindList("zapped")) {
          for (const base::Value& selector : *zapped) {
            if (selector.is_string()) {
              boost.zapped.push_back(selector.GetString());
            }
          }
        }
        boost.enabled = dict->FindBool("enabled").value_or(true);
        boosts_.push_back(std::move(boost));
      }
    }
    loaded_ = true;
    observers_.Notify();
  }

  void Changed() {
    base::ListValue list;
    for (const Boost& boost : boosts_) {
      base::ListValue zapped;
      for (const std::string& selector : boost.zapped) {
        zapped.Append(selector);
      }
      list.Append(base::DictValue()
                      .Set("host", boost.host)
                      .Set("css", boost.css)
                      .Set("js", boost.js)
                      .Set("zapped", std::move(zapped))
                      .Set("enabled", boost.enabled));
    }
    std::string json;
    base::JSONWriter::WriteWithOptions(
        list, base::JSONWriter::OPTIONS_PRETTY_PRINT, &json);
    file_runner_->PostTask(
        FROM_HERE, base::BindOnce(
                       [](base::FilePath path, std::string json) {
                         base::ImportantFileWriter::WriteFileAtomically(
                             path, json);
                       },
                       path_, std::move(json)));
    observers_.Notify();
  }

  base::FilePath path_;
  scoped_refptr<base::SequencedTaskRunner> file_runner_;
  std::vector<Boost> boosts_;
  bool loaded_ = false;
  base::RepeatingClosureList observers_;
  base::WeakPtrFactory<BoostStore> weak_factory_{this};
};

std::string JsString(const std::string& text) {
  std::string out;
  base::JSONWriter::Write(base::Value(text), &out);
  return out;
}

// Puts the boost's style (its CSS and the zapped elements, hidden) in the
// page, or takes it out, then runs its script. Boosts run in an isolated
// world, like an extension's content script: they see the page's DOM, not
// its JavaScript.
std::string ApplyScript(const std::optional<Boost>& boost) {
  std::string css;
  std::string js;
  if (boost && boost->enabled) {
    css = boost->css;
    if (!boost->zapped.empty()) {
      css += "\n" + base::JoinString(boost->zapped, ",\n") +
             " { display: none !important; }";
    }
    js = boost->js;
  }
  std::string script =
      "(() => {"
      "  let s = document.getElementById('__mori_boost');"
      "  const css = " + JsString(css) + ";"
      "  if (!css) { if (s) s.remove(); return; }"
      "  if (!s) {"
      "    s = document.createElement('style');"
      "    s.id = '__mori_boost';"
      "    (document.head || document.documentElement).appendChild(s);"
      "  }"
      "  s.textContent = css;"
      "})();";
  if (!js.empty()) {
    script += "\ntry {\n" + js + "\n} catch (e) { console.error(e); }";
  }
  return script;
}

// Zap mode: the element under the pointer is outlined, a click hides it
// and reports a selector for it, Escape ends the mode. Reports carry a
// nonce, so a page can't feed selectors in.
std::string ZapScript(const std::string& nonce) {
  return "(() => {"
         "  const nonce = " + JsString(nonce) + ";"
         "  if (window.__moriZap) return;"
         "  window.__moriZap = true;"
         "  const box = document.createElement('div');"
         "  box.style.cssText = 'position:fixed;pointer-events:none;"
         "z-index:2147483647;background:rgba(255,90,90,.22);"
         "outline:2px solid rgb(255,90,90);border-radius:3px';"
         "  document.documentElement.appendChild(box);"
         "  let target = null;"
         "  const ok = (s) => /^[A-Za-z_][\\w-]*$/.test(s);"
         "  const selector = (el) => {"
         "    const parts = [];"
         "    while (el && el.nodeType === 1 && el !== document.body &&"
         "           el !== document.documentElement && parts.length < 5) {"
         "      if (el.id && ok(el.id)) { parts.unshift('#' + el.id); break; }"
         "      let p = el.tagName.toLowerCase();"
         "      const cls = [...el.classList].filter(ok).slice(0, 2);"
         "      if (cls.length) p += '.' + cls.join('.');"
         "      const parent = el.parentElement;"
         "      if (parent) {"
         "        const same = [...parent.children]"
         "            .filter((c) => c.tagName === el.tagName);"
         "        if (same.length > 1)"
         "          p += ':nth-of-type(' + (same.indexOf(el) + 1) + ')';"
         "      }"
         "      parts.unshift(p);"
         "      el = parent;"
         "    }"
         "    return parts.join(' > ');"
         "  };"
         "  const move = (e) => {"
         "    target = e.target;"
         "    const r = target.getBoundingClientRect();"
         "    Object.assign(box.style, {left: r.left + 'px', top: r.top + 'px',"
         "        width: r.width + 'px', height: r.height + 'px'});"
         "  };"
         "  const swallow = (e) => { e.preventDefault(); e.stopPropagation(); };"
         "  const click = (e) => {"
         "    swallow(e);"
         "    if (!target) return;"
         "    const s = selector(target);"
         "    target.style.setProperty('display', 'none', 'important');"
         "    console.log('__mori_zap:' + nonce + ':' + s);"
         "  };"
         "  const done = () => {"
         "    document.removeEventListener('mousemove', move, true);"
         "    document.removeEventListener('mousedown', swallow, true);"
         "    document.removeEventListener('mouseup', swallow, true);"
         "    document.removeEventListener('click', click, true);"
         "    document.removeEventListener('keydown', key, true);"
         "    box.remove();"
         "    window.__moriZap = false;"
         "    console.log('__mori_zap_done:' + nonce);"
         "  };"
         "  const key = (e) => {"
         "    if (e.key === 'Escape' || e.key === 'Enter') { swallow(e); done(); }"
         "  };"
         "  document.addEventListener('mousemove', move, true);"
         "  document.addEventListener('mousedown', swallow, true);"
         "  document.addEventListener('mouseup', swallow, true);"
         "  document.addEventListener('click', click, true);"
         "  document.addEventListener('keydown', key, true);"
         "})();";
}

// Applies the boost of the page's site whenever a page loads, and runs zap
// mode.
class BoostTabHelper : public content::WebContentsObserver,
                       public content::WebContentsUserData<BoostTabHelper> {
 public:
  explicit BoostTabHelper(content::WebContents* contents)
      : content::WebContentsObserver(contents),
        content::WebContentsUserData<BoostTabHelper>(*contents) {
    store_ = BoostStore::For(
        Profile::FromBrowserContext(contents->GetBrowserContext()));
    subscription_ = store_->AddObserver(base::BindRepeating(
        &BoostTabHelper::Apply, base::Unretained(this)));
  }

  // Zap mode on the page; `done` runs when it ends.
  void StartZap(base::OnceClosure done) {
    zap_nonce_ = base::NumberToString(base::RandUint64());
    zap_done_ = std::move(done);
    Run(ZapScript(*zap_nonce_));
  }

  // content::WebContentsObserver:
  void DOMContentLoaded(content::RenderFrameHost* frame) override {
    if (frame->IsInPrimaryMainFrame() && store_->loaded()) {
      Apply();
    }
  }

  void PrimaryPageChanged(content::Page& page) override {
    EndZap();
  }

  void OnDidAddMessageToConsole(
      content::RenderFrameHost* frame,
      blink::mojom::ConsoleMessageLevel log_level,
      const std::u16string& message,
      int32_t line_no,
      const std::u16string& source_id,
      const std::optional<std::u16string>& untrusted_stack_trace) override {
    if (!zap_nonce_ || !frame->IsInPrimaryMainFrame()) {
      return;
    }
    const std::string text = base::UTF16ToUTF8(message);
    const std::string zapped = "__mori_zap:" + *zap_nonce_ + ":";
    if (text.starts_with(zapped)) {
      const std::string selector = text.substr(zapped.size());
      const GURL url = web_contents()->GetLastCommittedURL();
      Boost boost = store_->Find(url).value_or(Boost{NormalizedHost(url)});
      if (!selector.empty() &&
          std::ranges::find(boost.zapped, selector) == boost.zapped.end()) {
        boost.zapped.push_back(selector);
        store_->Put(std::move(boost));
      }
    } else if (text == "__mori_zap_done:" + *zap_nonce_) {
      EndZap();
    }
  }

 private:
  friend WebContentsUserData;
  WEB_CONTENTS_USER_DATA_KEY_DECL();

  void Apply() {
    const GURL url = web_contents()->GetLastCommittedURL();
    if (!url.SchemeIsHTTPOrHTTPS()) {
      return;
    }
    Run(ApplyScript(store_->Find(url)));
  }

  void Run(const std::string& script) {
    web_contents()->GetPrimaryMainFrame()->ExecuteJavaScriptInIsolatedWorld(
        base::UTF8ToUTF16(script), base::NullCallback(),
        ISOLATED_WORLD_ID_CHROME_INTERNAL);
  }

  void EndZap() {
    zap_nonce_.reset();
    if (zap_done_) {
      std::move(zap_done_).Run();
    }
  }

  raw_ptr<BoostStore> store_;
  base::CallbackListSubscription subscription_;
  std::optional<std::string> zap_nonce_;
  base::OnceClosure zap_done_;
};

WEB_CONTENTS_USER_DATA_KEY_IMPL(BoostTabHelper);

constexpr int kEditorWidth = 560;
constexpr auto kShadowInsets = gfx::Insets::TLBR(24, 40, 56, 40);

gfx::FontList Font(int size, gfx::Font::Weight weight) {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(size - base.GetFontSize())
      .DeriveWithWeight(weight);
}

gfx::FontList Monospace(int size) {
  return gfx::FontList({"monospace"}, gfx::Font::NORMAL, size,
                       gfx::Font::Weight::NORMAL);
}

// The editor: the site's CSS and JavaScript, its zapped elements, and Zap,
// Remove and Save, in Mori 3's popover.
class BoostEditorView : public views::View, public views::TextfieldController {
  METADATA_HEADER(BoostEditorView, views::View)

 public:
  struct Callbacks {
    base::RepeatingClosure save;
    base::RepeatingClosure remove;
    base::RepeatingClosure zap;
    base::RepeatingClosure close;
  };

  explicit BoostEditorView(Callbacks callbacks)
      : callbacks_(std::move(callbacks)) {
    SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical,
        kShadowInsets + gfx::Insets::VH(16, 20), 8));
    title_ = AddChildView(std::make_unique<views::Label>());
    title_->SetFontList(Font(15, gfx::Font::Weight::SEMIBOLD));
    title_->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    css_label_ = AddHeading(u"CSS");
    css_ = AddEditor(u"CSS for this site");
    js_label_ = AddHeading(u"JavaScript");
    js_ = AddEditor(u"JavaScript for this site");
    zapped_label_ = AddHeading(u"Zapped elements");
    zapped_ = AddChildView(std::make_unique<views::View>());
    zapped_->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical, gfx::Insets(), 2));

    auto* buttons = AddChildView(std::make_unique<views::BoxLayoutView>());
    buttons->SetBetweenChildSpacing(8);
    buttons->SetProperty(views::kMarginsKey, gfx::Insets::TLBR(8, 0, 0, 0));
    buttons->AddChildView(std::make_unique<views::MdTextButton>(
        callbacks_.zap, u"Zap Elements"));
    auto* spacer = buttons->AddChildView(std::make_unique<views::View>());
    buttons->SetFlexForView(spacer, 1);
    remove_ = buttons->AddChildView(std::make_unique<views::MdTextButton>(
        callbacks_.remove, u"Remove"));
    buttons->AddChildView(std::make_unique<views::MdTextButton>(
        callbacks_.close, u"Cancel"));
    auto* save = buttons->AddChildView(
        std::make_unique<views::MdTextButton>(callbacks_.save, u"Save"));
    save->SetStyle(ui::ButtonStyle::kProminent);
  }

  void Set(const Boost& boost, bool exists) {
    host_ = boost.host;
    title_->SetText(u"Boost " + base::UTF8ToUTF16(boost.host));
    css_->SetText(base::UTF8ToUTF16(boost.css));
    js_->SetText(base::UTF8ToUTF16(boost.js));
    zapped_list_ = boost.zapped;
    remove_->SetEnabled(exists);
    RebuildZapped();
  }

  Boost Get() const {
    Boost boost;
    boost.host = host_;
    boost.css = base::UTF16ToUTF8(css_->GetText());
    boost.js = base::UTF16ToUTF8(js_->GetText());
    boost.zapped = zapped_list_;
    return boost;
  }

  const std::string& host() const { return host_; }
  void FocusEditor() { css_->RequestFocus(); }

  // views::TextfieldController:
  bool HandleKeyEvent(views::Textfield* sender,
                      const ui::KeyEvent& event) override {
    if (event.type() == ui::EventType::kKeyPressed &&
        event.key_code() == ui::VKEY_ESCAPE) {
      callbacks_.close.Run();
      return true;
    }
    return false;
  }

  void OnThemeChanged() override {
    views::View::OnThemeChanged();
    const bool dark = IsDark();
    const SkColor fg = dark ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                            : SkColorSetRGB(0x1D, 0x1F, 0x22);
    const SkColor muted = dark ? SkColorSetRGB(0xA0, 0xA1, 0xA2)
                               : SkColorSetRGB(0x5A, 0x5C, 0x5E);
    title_->SetEnabledColor(fg);
    for (views::Label* label : {css_label_.get(), js_label_.get(),
                                zapped_label_.get()}) {
      label->SetEnabledColor(muted);
    }
    for (views::Textarea* editor : {css_.get(), js_.get()}) {
      editor->SetBackground(views::CreateRoundedRectBackground(
          SkColorSetA(dark ? SK_ColorWHITE : SK_ColorBLACK, 0x10), 8));
      editor->SetTextColorId(kColorOmniboxText);
    }
    RebuildZapped();
  }

  void OnPaintBackground(gfx::Canvas* canvas) override {
    gfx::Rect popover = GetLocalBounds();
    popover.Inset(kShadowInsets);
    PaintPopover(this, canvas, popover);
  }

 private:
  bool IsDark() const {
    return GetColorProvider() &&
           color_utils::IsDark(
               GetColorProvider()->GetColor(ui::kColorFrameActive));
  }

  views::Label* AddHeading(const std::u16string& text) {
    auto* label = AddChildView(std::make_unique<views::Label>(text));
    label->SetFontList(Font(12, gfx::Font::Weight::BOLD));
    label->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    label->SetProperty(views::kMarginsKey, gfx::Insets::TLBR(6, 0, 0, 0));
    return label;
  }

  views::Textarea* AddEditor(const std::u16string& name) {
    auto* editor = AddChildView(std::make_unique<views::Textarea>());
    editor->SetFontList(Monospace(12));
    editor->SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(6, 8)));
    editor->SetPreferredSize(gfx::Size(0, 104));
    editor->set_controller(this);
    editor->GetViewAccessibility().SetName(name);
    return editor;
  }

  void RebuildZapped() {
    zapped_->RemoveAllChildViews();
    const bool dark = IsDark();
    const SkColor fg = dark ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                            : SkColorSetRGB(0x1D, 0x1F, 0x22);
    const SkColor muted = dark ? SkColorSetRGB(0xA0, 0xA1, 0xA2)
                               : SkColorSetRGB(0x5A, 0x5C, 0x5E);
    if (zapped_list_.empty()) {
      auto* hint = zapped_->AddChildView(std::make_unique<views::Label>(
          u"Zap Elements, then click parts of the page to hide them."));
      hint->SetHorizontalAlignment(gfx::ALIGN_LEFT);
      hint->SetFontList(Font(12, gfx::Font::Weight::NORMAL));
      hint->SetEnabledColor(muted);
    }
    for (size_t i = 0; i < zapped_list_.size(); ++i) {
      auto* row = zapped_->AddChildView(std::make_unique<views::BoxLayoutView>());
      row->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kCenter);
      auto* label = row->AddChildView(std::make_unique<views::Label>(
          base::UTF8ToUTF16(zapped_list_[i])));
      label->SetFontList(Monospace(12));
      label->SetHorizontalAlignment(gfx::ALIGN_LEFT);
      label->SetElideBehavior(gfx::ELIDE_MIDDLE);
      label->SetEnabledColor(fg);
      row->SetFlexForView(label, 1);
      auto* show = row->AddChildView(std::make_unique<views::LabelButton>(
          base::BindRepeating(&BoostEditorView::Unzap, base::Unretained(this),
                              i),
          u"Show"));
      show->SetEnabledTextColors(muted);
    }
    PreferredSizeChanged();
  }

  void Unzap(size_t index) {
    if (index < zapped_list_.size()) {
      zapped_list_.erase(zapped_list_.begin() + index);
      RebuildZapped();
    }
  }

  Callbacks callbacks_;
  std::string host_;
  std::vector<std::string> zapped_list_;
  raw_ptr<views::Label> title_ = nullptr;
  raw_ptr<views::Label> css_label_ = nullptr;
  raw_ptr<views::Label> js_label_ = nullptr;
  raw_ptr<views::Label> zapped_label_ = nullptr;
  raw_ptr<views::Textarea> css_ = nullptr;
  raw_ptr<views::Textarea> js_ = nullptr;
  raw_ptr<views::View> zapped_ = nullptr;
  raw_ptr<views::MdTextButton> remove_ = nullptr;
};

BEGIN_METADATA(BoostEditorView)
END_METADATA

// One per window.
class BoostEditor {
 public:
  explicit BoostEditor(BrowserView* browser_view)
      : browser_view_(browser_view) {
    overlay_ = std::make_unique<Overlay>(
        browser_view,
        std::make_unique<BoostEditorView>(BoostEditorView::Callbacks{
            base::BindRepeating(&BoostEditor::Save, base::Unretained(this)),
            base::BindRepeating(&BoostEditor::Remove, base::Unretained(this)),
            base::BindRepeating(&BoostEditor::Zap, base::Unretained(this)),
            base::BindRepeating(&BoostEditor::Close,
                                base::Unretained(this))}),
        "MoriBoostEditor");
  }

  void Open() {
    content::WebContents* contents =
        browser_view_->browser()->GetTabStripModel()->GetActiveWebContents();
    auto* view = static_cast<BoostEditorView*>(overlay_->contents());
    if (!contents || !view) {
      return;
    }
    const GURL url = contents->GetLastCommittedURL();
    if (!url.SchemeIsHTTPOrHTTPS()) {
      return;
    }
    contents_ = contents->GetWeakPtr();
    BoostStore* store = BoostStore::For(
        Profile::FromBrowserContext(contents->GetBrowserContext()));
    const std::optional<Boost> boost = store->Find(url);
    view->Set(boost.value_or(Boost{NormalizedHost(url)}), boost.has_value());
    Fit();
    view->FocusEditor();
  }

 private:
  void Fit() {
    auto* view = static_cast<BoostEditorView*>(overlay_->contents());
    const gfx::Rect area = browser_view_->GetLocalBounds();
    const int width = kEditorWidth + kShadowInsets.width();
    const int top = std::max(16, area.height() / 8);
    const int height = std::min(view->GetHeightForWidth(width),
                                area.height() - top + kShadowInsets.height());
    overlay_->Show(gfx::Rect(area.x() + (area.width() - width) / 2,
                             area.y() + top - kShadowInsets.top(), width,
                             height));
  }

  BoostStore* Store() {
    return contents_ ? BoostStore::For(Profile::FromBrowserContext(
                           contents_->GetBrowserContext()))
                     : nullptr;
  }

  void Save() {
    auto* view = static_cast<BoostEditorView*>(overlay_->contents());
    if (BoostStore* store = Store(); store && view) {
      store->Put(view->Get());
    }
    Close();
  }

  void Remove() {
    auto* view = static_cast<BoostEditorView*>(overlay_->contents());
    if (BoostStore* store = Store(); store && view) {
      store->Remove(view->host());
    }
    Close();
  }

  // The editor steps aside while the user zaps; it comes back, with what
  // was zapped, when zap mode ends. What's typed so far is kept.
  void Zap() {
    auto* view = static_cast<BoostEditorView*>(overlay_->contents());
    BoostTabHelper* helper =
        contents_ ? BoostTabHelper::FromWebContents(contents_.get()) : nullptr;
    if (!view || !helper) {
      return;
    }
    if (BoostStore* store = Store()) {
      store->Put(view->Get());
    }
    overlay_->Hide();
    contents_->Focus();
    helper->StartZap(base::BindOnce(&BoostEditor::Open,
                                    weak_factory_.GetWeakPtr()));
  }

  void Close() {
    overlay_->Hide();
    if (contents_) {
      contents_->Focus();
    }
  }

  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<Overlay> overlay_;
  base::WeakPtr<content::WebContents> contents_;
  base::WeakPtrFactory<BoostEditor> weak_factory_{this};
};

}  // namespace
}  // namespace mori

DEFINE_UI_CLASS_PROPERTY_TYPE(mori::BoostEditor*)

namespace mori {
namespace {
DEFINE_OWNED_UI_CLASS_PROPERTY_KEY(BoostEditor, kBoostEditorKey)
}  // namespace

void AttachBoosts(content::WebContents* contents) {
  BoostTabHelper::CreateForWebContents(contents);
}

void EditBoost(BrowserWindowInterface* browser) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view || !browser_view->GetWidget()) {
    return;
  }
  BoostEditor* editor = browser_view->GetProperty(kBoostEditorKey);
  if (!editor) {
    editor = browser_view->SetProperty(
        kBoostEditorKey, std::make_unique<BoostEditor>(browser_view));
  }
  editor->Open();
}

}  // namespace mori
