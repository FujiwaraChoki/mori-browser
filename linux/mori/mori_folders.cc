#include "chrome/browser/ui/views/mori/mori_folders.h"

#include <algorithm>
#include <cmath>
#include <numbers>
#include <set>
#include <utility>

#include "base/callback_list.h"
#include "base/functional/bind.h"
#include "base/json/json_reader.h"
#include "base/json/json_writer.h"
#include "base/memory/weak_ptr.h"
#include "base/no_destructor.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "base/time/time.h"
#include "base/uuid.h"
#include "base/values.h"
#include "cc/paint/paint_flags.h"
#include "cc/paint/paint_shader.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/sessions/session_service.h"
#include "chrome/browser/sessions/session_service_factory.h"
#include "chrome/browser/ui/browser_commands.h"
#include "chrome/browser/ui/browser_tabstrip.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/layout_constants.h"
#include "chrome/browser/ui/tabs/tab_enums.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/tabs/tab_strip_model_observer.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/frame/vertical_tab_strip_region_view.h"
#include "chrome/browser/ui/views/tabs/common/tab_strip_view.h"
#include "chrome/browser/ui/views/tabs/common/pinned_tab_container_view.h"
#include "ui/views/controls/scroll_view.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_folder_tree.h"
#include "chrome/browser/ui/views/mori/mori_folder_session.h"
#include "chrome/browser/ui/views/mori/mori_tab_list.h"
#include "chrome/browser/ui/views/mori/mori_menu.h"
#include "chrome/browser/ui/views/mori/mori_themes.h"
#include "chrome/browser/ui/views/tabs/common/split_tab_view.h"
#include "chrome/browser/ui/views/tabs/common/tab_collection_node.h"
#include "chrome/browser/ui/views/tabs/common/unpinned_tab_container_view.h"
#include "chrome/browser/ui/views/tabs/common/tab_view.h"
#include "components/sessions/content/session_tab_helper.h"
#include "components/tabs/public/tab_interface.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_user_data.h"
#include "third_party/skia/include/core/SkM44.h"
#include "third_party/skia/include/core/SkMatrix.h"
#include "third_party/skia/include/core/SkPath.h"
#include "third_party/skia/include/core/SkPathBuilder.h"
#include "third_party/skia/include/core/SkRRect.h"
#include "ui/accessibility/ax_enums.mojom.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/color/color_provider.h"
#include "ui/events/event.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/animation/animation_delegate.h"
#include "ui/gfx/animation/slide_animation.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/font.h"
#include "ui/gfx/geometry/cubic_bezier.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/paint_vector_icon.h"
#include "ui/gfx/scoped_canvas.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/background.h"
#include "ui/views/border.h"
#include "ui/views/controls/label.h"
#include "ui/views/controls/textfield/textfield.h"
#include "ui/views/controls/textfield/textfield_controller.h"
#include "ui/views/view.h"
#include "ui/views/view_utils.h"
#include "ui/views/widget/widget.h"
#include "url/gurl.h"

namespace mori {
namespace {

// ---------------------------------------------------------------------------
// Folders and their tabs.

struct FolderState {
  std::u16string name;
  std::string icon;
  bool open = true;
  std::string parent;
};

// Every folder a tab has held this run, by id. Folders are tiny and their
// ids unique, so they aren't forgotten when their last tab goes.
std::map<std::string, FolderState>& Folders() {
  static base::NoDestructor<std::map<std::string, FolderState>> folders;
  return *folders;
}

// Told when a folder's name, icon or state changes, or a tab's folder.
base::RepeatingClosureList& FolderObservers() {
  static base::NoDestructor<base::RepeatingClosureList> observers;
  return *observers;
}

// A folder made from the menu, whose header starts renaming once it shows.
std::string& FolderToRename() {
  static base::NoDestructor<std::string> id;
  return *id;
}

class TabFolder : public content::WebContentsUserData<TabFolder> {
 public:
  TabFolder(content::WebContents* contents, std::string id)
      : content::WebContentsUserData<TabFolder>(*contents),
        id_(std::move(id)) {}

  const std::string& id() const { return id_; }

 private:
  friend WebContentsUserData;
  WEB_CONTENTS_USER_DATA_KEY_DECL();

  std::string id_;
};

WEB_CONTENTS_USER_DATA_KEY_IMPL(TabFolder);

std::optional<std::string> FolderIdOf(const content::WebContents* contents) {
  const TabFolder* data = contents ? TabFolder::FromWebContents(
                                         const_cast<content::WebContents*>(
                                             contents))
                                   : nullptr;
  if (!data || !Folders().contains(data->id())) {
    return std::nullopt;
  }
  return data->id();
}

std::optional<std::string> FolderIdOf(const tabs::TabInterface* tab) {
  return tab ? FolderIdOf(tab->GetContents()) : std::nullopt;
}

std::vector<std::string> FolderPath(const std::string& id) {
  std::vector<std::string> path;
  std::set<std::string> seen;
  std::string current = id;
  while (!current.empty() && Folders().contains(current) &&
         seen.insert(current).second) {
    path.push_back(current);
    current = Folders().at(current).parent;
  }
  std::reverse(path.begin(), path.end());
  return path;
}

bool InFolder(const std::optional<std::string>& leaf, const std::string& parent) {
  if (!leaf) {
    return false;
  }
  const auto path = FolderPath(*leaf);
  return std::ranges::find(path, parent) != path.end();
}

bool FolderVisible(const std::string& id, bool include_self) {
  return FolderPathVisible(FolderPath(id), include_self,
                            [](const std::string& folder) {
                              return Folders().at(folder).open;
                            });
}

FolderInfo InfoFor(const std::string& id) {
  const FolderState& state = Folders()[id];
  return FolderInfo{id, state.name, state.icon, state.open, state.parent};
}

void NotifyFoldersChanged() {
  FolderObservers().Notify();
}

// Writes `contents`' folder into the session, as Mori's tab names are.
void SaveTabFolder(content::WebContents* contents) {
  Profile* profile =
      Profile::FromBrowserContext(contents->GetBrowserContext());
  SessionService* session = SessionServiceFactory::GetForProfile(profile);
  if (!session) {
    return;
  }
  const SessionID window_id =
      sessions::SessionTabHelper::IdForWindowContainingTab(contents);
  const SessionID tab_id = sessions::SessionTabHelper::IdForTab(contents);
  if (window_id.is_valid() && tab_id.is_valid()) {
    session->AddTabExtraData(window_id, tab_id, kTabFolderKey,
                             GetTabFolderData(contents));
  }
}

void SetTabFolderId(content::WebContents* contents,
                    std::optional<std::string> id) {
  if (FolderIdOf(contents) == id) {
    return;
  }
  if (id.has_value()) {
    contents->RemoveUserData(TabFolder::UserDataKey());
    TabFolder::CreateForWebContents(contents, *id);
  } else {
    contents->RemoveUserData(TabFolder::UserDataKey());
  }
  SaveTabFolder(contents);
}

// Rewrites the session data of the folder's tabs in `browser` after a
// change to the folder itself.
void SaveFolderTabs(BrowserWindowInterface* browser, const std::string& id) {
  TabStripModel* model = browser->GetTabStripModel();
  for (int i = 0; i < model->count(); ++i) {
    content::WebContents* contents = model->GetWebContentsAt(i);
    if (InFolder(FolderIdOf(contents), id)) {
      SaveTabFolder(contents);
    }
  }
}

// The first and last strip index of the folder's tabs, if it has any.
std::optional<std::pair<int, int>> FolderRange(TabStripModel* model,
                                               const std::string& id,
                                               const tabs::TabInterface* skip) {
  std::optional<std::pair<int, int>> range;
  for (int i = 0; i < model->count(); ++i) {
    tabs::TabInterface* tab = model->GetTabAtIndex(i);
    if (tab == skip || !InFolder(FolderIdOf(tab), id)) {
      continue;
    }
    if (!range) {
      range = std::make_pair(i, i);
    }
    range->second = i;
  }
  return range;
}

// Moves the tab at `from` to just after `after`.
void MoveTabAfter(TabStripModel* model, int from, int after) {
  const int to = from < after ? after : after + 1;
  if (to != from) {
    model->MoveWebContentsAt(from, to, /*select_after_move=*/false);
  }
}

// Keep the model in the same depth-first order as the sidebar. Folder
// membership is explicit: moving a sibling never dissolves its descendants.
void NormalizeFolders(TabStripModel* model) {
  std::vector<tabs::TabHandle> tabs;
  std::vector<std::vector<std::string>> paths;
  for (int i = 0; i < model->count(); ++i) {
    auto* tab = model->GetTabAtIndex(i);
    if (tab->IsPinned()) {
      if (FolderIdOf(tab)) {
        SetTabFolderId(tab->GetContents(), std::nullopt);
      }
      continue;
    }
    tabs.push_back(tab->GetHandle());
    const auto folder = FolderIdOf(tab);
    paths.push_back(folder ? FolderPath(*folder) : std::vector<std::string>());
  }
  int index = model->IndexOfFirstNonPinnedTab();
  for (const auto& row : FolderTreeRows(paths)) {
    if (!row.tab) {
      continue;
    }
    auto* tab = tabs[*row.tab].Get();
    if (tab) {
      const int from = model->GetIndexOfTab(tab);
      if (from != index && from != TabStripModel::kNoTab) {
        model->MoveWebContentsAt(from, index, /*select_after_move=*/false);
      }
      ++index;
    }
  }
}

// The tabs a row of a group view shows: one, or two for a split pair.
std::vector<const tabs::TabInterface*> TabsOfRow(const views::View* row) {
  std::vector<const tabs::TabInterface*> tabs;
  if (const auto* tab_view = views::AsViewClass<TabView>(row)) {
    if (const tabs::TabInterface* tab = tab_view->GetTabInterface()) {
      tabs.push_back(tab);
    }
  } else if (const auto* split = views::AsViewClass<SplitTabView>(row)) {
    if (const TabCollectionNode* node = split->collection_node()) {
      for (views::View* child : node->GetDirectChildren()) {
        if (const auto* split_tab = views::AsViewClass<TabView>(child)) {
          if (const tabs::TabInterface* tab = split_tab->GetTabInterface()) {
            tabs.push_back(tab);
          }
        }
      }
    }
  }
  return tabs;
}

std::optional<std::string> FolderOfRow(const views::View* row) {
  for (const tabs::TabInterface* tab : TabsOfRow(row)) {
    if (std::optional<std::string> id = FolderIdOf(tab)) {
      return id;
    }
  }
  return std::nullopt;
}

bool RowIsActive(const views::View* row) {
  return std::ranges::any_of(TabsOfRow(row), [](const tabs::TabInterface* t) {
    return t->IsActivated();
  });
}

// ---------------------------------------------------------------------------
// Zen's folder icon (ZenFolder.mjs rawIcon, zen-folder-icon.inc.css): a back
// plate and a front plate in a 27-unit box drawn 28px wide, that tilt apart
// as the folder opens.

constexpr int kIconSize = 28;
constexpr float kIconUnits = 27.0f;
constexpr base::TimeDelta kOpenDuration = base::Milliseconds(300);

SkPath BackPlate() {
  SkPathBuilder path;
  path.moveTo(8, 5.625f);
  path.lineTo(11.9473f, 5.625f);
  path.cubicTo(12.4866f, 5.625f, 13.0105f, 5.80861f, 13.4316f, 6.14551f);
  path.lineTo(14.2881f, 6.83105f);
  path.cubicTo(14.9308f, 7.34508f, 15.7298f, 7.625f, 16.5527f, 7.625f);
  path.lineTo(20, 7.625f);
  path.cubicTo(21.3117f, 7.625f, 22.375f, 8.68832f, 22.375f, 10);
  path.lineTo(22.375f, 20);
  path.cubicTo(22.375f, 21.3117f, 21.3117f, 22.375f, 20, 22.375f);
  path.lineTo(8, 22.375f);
  path.cubicTo(6.68832f, 22.375f, 5.625f, 21.3117f, 5.625f, 20);
  path.lineTo(5.625f, 8);
  path.cubicTo(5.625f, 6.68832f, 6.68832f, 5.625f, 8, 5.625f);
  path.close();
  return path.detach();
}

SkRRect FrontPlate() {
  return SkRRect::MakeRectXY(SkRect::MakeXYWH(5.625f, 9.625f, 16.75f, 12.75f),
                             2.375f, 2.375f);
}

// The open state's transform at `t` (0 closed, 1 open): CSS interpolates
// each function from none, so skew, offset and scale all grow with `t`.
SkM44 PlateTransform(float t, float skew_degrees, float dx, float dy) {
  SkMatrix m = SkMatrix::Skew(
      std::tan(skew_degrees * t * std::numbers::pi_v<float> / 180.0f), 0);
  m.preTranslate(dx * t, dy * t);
  const float scale = 1.0f - 0.15f * t;
  m.preScale(scale, scale);
  return SkM44(m);
}

// The SVG's contrast(125%) filter, applied to its colours.
SkColor Contrast(SkColor c) {
  auto channel = [](U8CPU v) {
    return static_cast<U8CPU>(
        std::clamp((v / 255.0f - 0.5f) * 1.25f + 0.5f, 0.0f, 1.0f) * 255.0f +
        0.5f);
  };
  return SkColorSetARGB(SkColorGetA(c), channel(SkColorGetR(c)),
                        channel(SkColorGetG(c)), channel(SkColorGetB(c)));
}

// CSS color-mix(in srgb, a `amount`, b).
SkColor Mix(SkColor a, SkColor b, float amount) {
  return color_utils::AlphaBlend(a, b, amount);
}

struct FolderColors {
  SkColor behind;
  SkColor front;
  SkColor stroke;
  SkColor icon;
};

// zen-folders.css: the plates' colours from the theme's accent.
FolderColors ColorsFor(SkColor accent, bool dark, SkColor text) {
  FolderColors colors;
  if (dark) {
    colors.behind = Mix(accent, SkColorSetRGB(0xC1, 0xC1, 0xC1), 0.6f);
    colors.front = Mix(accent, SK_ColorBLACK, 0.6f);
    colors.stroke = Mix(accent, SkColorSetRGB(0xEB, 0xEB, 0xEB), 0.15f);
  } else {
    colors.behind = Mix(accent, SkColorSetRGB(0x80, 0x80, 0x80), 0.6f);
    colors.front = Mix(accent, SK_ColorWHITE, 0.3f);
    colors.stroke = Mix(accent, SK_ColorBLACK, 0.5f);
  }
  colors.behind = Contrast(colors.behind);
  colors.front = Contrast(colors.front);
  colors.stroke = Contrast(colors.stroke);
  colors.icon = SkColorSetA(Mix(colors.stroke, text, 0.5f), 0xE6);
  return colors;
}

// The plate's faint top-lit sheen (a white-to-black gradient at 10%) and
// its 1.5-unit outline.
void PaintSheenAndOutline(gfx::Canvas* canvas,
                          const SkPath& plate,
                          float top,
                          SkColor stroke) {
  const SkPoint points[2] = {SkPoint::Make(14, top),
                             SkPoint::Make(14, 22.375f)};
  const SkColor4f colors[2] = {SkColors::kWhite, SkColors::kBlack};
  cc::PaintFlags sheen;
  sheen.setAntiAlias(true);
  sheen.setShader(cc::PaintShader::MakeLinearGradient(
      points, colors, nullptr, 2, SkTileMode::kClamp));
  sheen.setAlphaf(0.1f);
  canvas->DrawPath(plate, sheen);
  cc::PaintFlags outline;
  outline.setAntiAlias(true);
  outline.setStyle(cc::PaintFlags::kStroke_Style);
  outline.setStrokeWidth(1.5f);
  outline.setColor(stroke);
  canvas->DrawPath(plate, outline);
}

// `open` runs 0 (closed) to 1 (open). `dots` replaces the folder's icon
// with Zen's three dots: a closed folder holding the active tab.
void PaintFolderIcon(gfx::Canvas* canvas,
                     const gfx::Point& origin,
                     float open,
                     bool dots,
                     const std::string& icon,
                     const FolderColors& colors) {
  gfx::ScopedCanvas scoped(canvas);
  canvas->Translate(origin.OffsetFromOrigin());
  canvas->sk_canvas()->scale(kIconSize / kIconUnits, kIconSize / kIconUnits);

  cc::PaintFlags fill;
  fill.setAntiAlias(true);
  {
    gfx::ScopedCanvas back(canvas);
    canvas->sk_canvas()->concat(PlateTransform(open, 16, -4, 2));
    const SkPath plate = BackPlate();
    fill.setColor(colors.behind);
    canvas->DrawPath(plate, fill);
    PaintSheenAndOutline(canvas, plate, 5.625f, colors.stroke);
  }
  gfx::ScopedCanvas front(canvas);
  canvas->sk_canvas()->concat(PlateTransform(open, -16, 8, 2));
  const SkPath plate = SkPath::RRect(FrontPlate());
  fill.setColor(colors.front);
  canvas->DrawPath(plate, fill);
  PaintSheenAndOutline(canvas, plate, 9.625f, colors.stroke);
  if (dots) {
    fill.setColor(colors.stroke);
    for (float x : {10.0f, 14.0f, 18.0f}) {
      canvas->sk_canvas()->drawOval(SkRect::MakeXYWH(x - 1.25f, 14.75f, 2.5f, 2.5f),
                                    fill);
    }
  } else if (!icon.empty()) {
    canvas->sk_canvas()->translate(8.5f, 10.5f);
    gfx::PaintVectorIcon(canvas, LucideIcon(icon), 11, colors.icon);
  }
}

// ---------------------------------------------------------------------------
// The folder header row.

// Zen's label sits 32px in (the icon's 28px and a gap); the icon is centred
// where the tabs' favicons are.
constexpr int kLabelX = 32;
constexpr int kIconCenterX = 17;
constexpr float kRowRadius = 8.0f;

class FolderHeader;

class FolderNameLabel : public views::Label {
  METADATA_HEADER(FolderNameLabel, views::Label)

 public:
  bool IsOverText(const gfx::Point& point) const {
    gfx::Rect text = GetTextBounds();
    text.Intersect(GetLocalBounds());
    return text.Contains(point);
  }
};

BEGIN_METADATA(FolderNameLabel)
END_METADATA

class RenameField : public views::Textfield, public views::TextfieldController {
  METADATA_HEADER(RenameField, views::Textfield)

 public:
  explicit RenameField(base::RepeatingCallback<void(bool keep)> done)
      : done_(std::move(done)) {
    set_controller(this);
    SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(0, 6)));
    GetViewAccessibility().SetName(u"Folder name");
  }

  void OnThemeChanged() override {
    views::Textfield::OnThemeChanged();
    const bool dark = color_utils::IsDark(
        GetColorProvider()->GetColor(ui::kColorFrameActive));
    // Mori's field colour, as the tab name field.
    SetBackground(views::CreateRoundedRectBackground(
        dark ? SkColorSetRGB(0x2F, 0x2F, 0x2F)
             : SkColorSetRGB(0xFF, 0xFF, 0xFF),
        6));
    SetTextColorId(kColorOmniboxText);
  }

  void OnBlur() override {
    views::Textfield::OnBlur();
    // A click elsewhere keeps the name.
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&RenameField::Finish, weak_factory_.GetWeakPtr(), true));
  }

  // views::TextfieldController:
  bool HandleKeyEvent(views::Textfield* sender,
                      const ui::KeyEvent& event) override {
    if (event.type() != ui::EventType::kKeyPressed) {
      return false;
    }
    if (event.key_code() == ui::VKEY_RETURN) {
      Finish(true);
      return true;
    }
    if (event.key_code() == ui::VKEY_ESCAPE) {
      Finish(false);
      return true;
    }
    return false;
  }

 private:
  void Finish(bool keep) {
    if (finished_) {
      return;
    }
    finished_ = true;
    done_.Run(keep);
  }

  base::RepeatingCallback<void(bool keep)> done_;
  bool finished_ = false;
  base::WeakPtrFactory<RenameField> weak_factory_{this};
};

BEGIN_METADATA(RenameField)
END_METADATA

class FolderHeader : public views::View, public gfx::AnimationDelegate {
  METADATA_HEADER(FolderHeader, views::View)

 public:
  FolderHeader(BrowserWindowInterface* browser, std::string id)
      : browser_(browser), id_(std::move(id)) {
    label_ = AddChildView(std::make_unique<FolderNameLabel>());
    label_->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    label_->SetElideBehavior(gfx::ELIDE_TAIL);
    label_->SetFontList(views::Label::GetDefaultFontList().DeriveWithWeight(
        gfx::Font::Weight::SEMIBOLD));
    label_->SetAutoColorReadabilityEnabled(false);
    label_->SetCanProcessEventsWithinSubtree(false);
    open_animation_.SetSlideDuration(kOpenDuration);
    open_animation_.SetTweenType(gfx::Tween::LINEAR);
    GetViewAccessibility().SetRole(ax::mojom::Role::kButton);
  }

  const std::string& id() const { return id_; }

  base::WeakPtr<FolderHeader> GetWeakPtr() {
    return weak_factory_.GetWeakPtr();
  }

  void SetState(const FolderInfo& info, bool active, SkColor accent) {
    label_->SetText(info.name);
    GetViewAccessibility().SetName(info.name);
    if (info.open) {
      GetViewAccessibility().SetIsExpanded();
    } else {
      GetViewAccessibility().SetIsCollapsed();
    }
    if (!state_known_) {
      // The first state shows as it is; later changes animate.
      state_known_ = true;
      open_animation_.Reset(info.open ? 1 : 0);
    } else if (info.open) {
      open_animation_.Show();
    } else {
      open_animation_.Hide();
    }
    icon_ = info.icon;
    active_ = active;
    accent_ = accent;
    SchedulePaint();
  }

  void BeginRename() {
    text_click_count_ = 0;
    if (rename_) {
      return;
    }
    label_->SetVisible(false);
    rename_ = AddChildView(std::make_unique<RenameField>(base::BindRepeating(
        &FolderHeader::EndRename, weak_factory_.GetWeakPtr())));
    rename_->SetText(label_->GetText());
    rename_->SetFontList(label_->font_list());
    rename_->SelectAll(false);
    DeprecatedLayoutImmediately();
    rename_->RequestFocus();
  }

  // views::View:
  gfx::Size CalculatePreferredSize(
      const views::SizeBounds& available_size) const override {
    return gfx::Size(0, GetLayoutConstant(LayoutConstant::kVerticalTabHeight));
  }

  void Layout(PassKey) override {
    const gfx::Rect text(kLabelX, 0, std::max(0, width() - kLabelX - 6),
                         height());
    label_->SetBoundsRect(text);
    if (rename_) {
      const int field_height = 26;
      rename_->SetBounds(kLabelX - 6, (height() - field_height) / 2,
                         std::max(0, width() - kLabelX), field_height);
    }
  }

  void OnThemeChanged() override {
    views::View::OnThemeChanged();
    label_->SetEnabledColor(
        GetColorProvider()->GetColor(kColorTabForegroundInactiveFrameActive));
    SchedulePaint();
  }

  void OnPaint(gfx::Canvas* canvas) override {
    const ui::ColorProvider* provider = GetColorProvider();
    if (hovered_ && !rename_) {
      cc::PaintFlags flags;
      flags.setAntiAlias(true);
      flags.setColor(
          provider->GetColor(kColorTabBackgroundInactiveHoverFrameActive));
      canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), kRowRadius, flags);
    }
    const bool dark =
        color_utils::IsDark(provider->GetColor(ui::kColorFrameActive));
    const FolderColors colors =
        ColorsFor(accent_, dark, label_->GetEnabledColor());
    const float open = static_cast<float>(
        gfx::CubicBezier(0.42, 0, 0, 1)
            .Solve(open_animation_.GetCurrentValue()));
    PaintFolderIcon(canvas,
                    gfx::Point(kIconCenterX - kIconSize / 2,
                               (height() - kIconSize) / 2),
                    open, active_, icon_, colors);
  }

  void OnMouseEntered(const ui::MouseEvent& event) override {
    hovered_ = true;
    SchedulePaint();
  }

  void OnMouseExited(const ui::MouseEvent& event) override {
    text_click_count_ = 0;
    hovered_ = false;
    SchedulePaint();
  }

  bool OnMousePressed(const ui::MouseEvent& event) override {
    if (event.IsRightMouseButton()) {
      text_click_count_ = 0;
      gfx::Point point = event.location();
      views::View::ConvertPointToScreen(this, &point);
      ShowFolderMenu(browser_, id_, point, ui::mojom::MenuSourceType::kMouse,
                     GetWidget());
      return true;
    }
    if (!event.IsOnlyLeftMouseButton()) {
      text_click_count_ = 0;
      return false;
    }
    gfx::Point point = event.location();
    views::View::ConvertPointToTarget(this, label_, &point);
    if (label_->IsOverText(point)) {
      // All three presses must hit the visible name within half a second.
      // Native repeat detection also rejects clicks that move too far apart.
      if (text_click_count_ == 0 || event.GetClickCount() == 1 ||
          event.time_stamp() - first_text_click_ > base::Milliseconds(500)) {
        first_text_click_ = event.time_stamp();
        text_click_count_ = 0;
      }
      ++text_click_count_;
    } else {
      text_click_count_ = 0;
    }
    if (text_click_count_ == 3) {
      // The first two presses toggled the folder back to its original state.
      BeginRename();
    } else {
      ToggleFolder(browser_, id_);
    }
    return true;
  }

  bool OnKeyPressed(const ui::KeyEvent& event) override {
    text_click_count_ = 0;
    if (event.key_code() == ui::VKEY_RETURN ||
        event.key_code() == ui::VKEY_SPACE) {
      ToggleFolder(browser_, id_);
      return true;
    }
    return false;
  }

  // gfx::AnimationDelegate:
  void AnimationProgressed(const gfx::Animation* animation) override {
    SchedulePaint();
  }

 private:
  void EndRename(bool keep) {
    if (!rename_) {
      return;
    }
    const std::u16string name(rename_->GetText());
    // The field is removed after the event that ended it.
    base::SingleThreadTaskRunner::GetCurrentDefault()->DeleteSoon(
        FROM_HERE, RemoveChildViewT(rename_.get()));
    rename_ = nullptr;
    label_->SetVisible(true);
    if (keep && !name.empty() && Folders().contains(id_)) {
      Folders()[id_].name = name;
      SaveFolderTabs(browser_, id_);
      NotifyFoldersChanged();
    }
    InvalidateLayout();
  }

  const raw_ptr<BrowserWindowInterface> browser_;
  const std::string id_;
  raw_ptr<FolderNameLabel> label_ = nullptr;
  raw_ptr<RenameField> rename_ = nullptr;
  int text_click_count_ = 0;
  base::TimeTicks first_text_click_;
  gfx::SlideAnimation open_animation_{this};
  bool state_known_ = false;
  std::string icon_;
  bool active_ = false;
  bool hovered_ = false;
  SkColor accent_ = SK_ColorGRAY;
  base::WeakPtrFactory<FolderHeader> weak_factory_{this};
};

BEGIN_METADATA(FolderHeader)
END_METADATA

std::vector<FolderHeader*> HeadersOf(const views::View* container) {
  std::vector<FolderHeader*> headers;
  for (views::View* child : container->children()) {
    if (auto* header = views::AsViewClass<FolderHeader>(child)) {
      headers.push_back(header);
    }
  }
  return headers;
}

FolderHeader* FindHeader(const views::View* container,
                         const std::string& id) {
  for (FolderHeader* header : HeadersOf(container)) {
    if (header->id() == id) {
      return header;
    }
  }
  return nullptr;
}

// The theme's accent: its gradient's first colour, else Mori's blue.
SkColor AccentFor(BrowserWindowInterface* browser, const views::View* view) {
  if (const ThemeColors colors = GetTheme(browser)) {
    return colors->first;
  }
  return view->GetColorProvider()->GetColor(ui::kColorAccent);
}

// ---------------------------------------------------------------------------
// The invisible child of the vertical tab list (UnpinnedTabContainerView)
// that keeps its folder headers in step with its tabs.

class FolderHeaders : public views::View, public TabStripModelObserver {
  METADATA_HEADER(FolderHeaders, views::View)

 public:
  FolderHeaders() { SetVisible(false); }

  ~FolderHeaders() override { StopObserving(); }

  void AddedToWidget() override {
    BrowserView* browser_view =
        BrowserView::GetBrowserViewForNativeWindow(
            GetWidget()->GetNativeWindow());
    if (!browser_view) {
      return;
    }
    browser_ = browser_view->browser();
    model_ = browser_->GetTabStripModel();
    model_->AddObserver(this);
    folders_subscription_ = FolderObservers().Add(base::BindRepeating(
        &FolderHeaders::ScheduleSync, base::Unretained(this)));
    ScheduleSync();
  }

  void RemovedFromWidget() override { StopObserving(); }

  // TabStripModelObserver:
  void OnTabStripModelDestroyed(TabStripModel* model) override {
    model_ = nullptr;
    browser_ = nullptr;
  }
  void OnTabStripModelChanged(
      TabStripModel* tab_strip_model,
      const TabStripModelChange& change,
      const TabStripSelectionChange& selection) override {
    if (change.type() == TabStripModelChange::kReplaced) {
      // Discarding swaps WebContents, while folder membership belongs to the
      // persistent sidebar entry. Transfer it before normalization sees the tab.
      const auto* replacement = change.GetReplace();
      if (const auto id = FolderIdOf(replacement->old_contents)) {
        SetTabFolderId(replacement->new_contents, id);
      }
    }
    if (!reordering_) {
      ScheduleSync();
    }
  }
  void OnTabGroupChanged(const TabGroupChange& change) override {
    ScheduleSync();
  }

 private:
  void StopObserving() {
    if (model_) {
      model_->RemoveObserver(this);
    }
    model_ = nullptr;
    browser_ = nullptr;
    folders_subscription_ = {};
  }

  // Many changes come at once (a restore, a drag); sync after them.
  void ScheduleSync() {
    if (sync_pending_) {
      return;
    }
    sync_pending_ = true;
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE,
        base::BindOnce(&FolderHeaders::Sync, weak_factory_.GetWeakPtr()));
  }

  void Sync() {
    sync_pending_ = false;
    auto* container = views::AsViewClass<UnpinnedTabContainerView>(parent());
    if (!model_ || !container || !container->collection_node()) {
      return;
    }
    // Wait for Chromium to finish a drag before changing model order.
    if (container->IsHandlingDrag()) {
      sync_pending_ = true;
      base::SingleThreadTaskRunner::GetCurrentDefault()->PostDelayedTask(
          FROM_HERE, base::BindOnce(&FolderHeaders::Sync, weak_factory_.GetWeakPtr()),
          base::Milliseconds(100));
      return;
    }
    reordering_ = true;
    NormalizeFolders(model_);
    reordering_ = false;
    bool has_divider = false;
    for (views::View* child : container->children()) {
      has_divider |= child->GetClassName() == std::string_view("MoriTabListHeader");
    }
    if (!has_divider) {
      container->AddChildView(CreateTabListHeader(browser_));
    }

    // Each folder among the list's rows, whether it holds the active tab.
    std::map<std::string, bool> folders;
    for (views::View* row : container->collection_node()->GetDirectChildren()) {
      if (std::optional<std::string> id = FolderOfRow(row)) {
        for (const auto& ancestor : FolderPath(*id)) {
          folders[ancestor] = folders[ancestor] || RowIsActive(row);
        }
      }
    }

    for (FolderHeader* header : HeadersOf(container)) {
      if (!folders.contains(header->id())) {
        container->RemoveChildViewT(header);
      }
    }
    const SkColor accent = AccentFor(browser_, this);
    for (const auto& [id, has_active] : folders) {
      FolderHeader* header = FindHeader(container, id);
      if (!header) {
        header = container->AddChildView(
            std::make_unique<FolderHeader>(browser_, id));
      }
      const FolderInfo info = InfoFor(id);
      header->SetState(info, has_active && !info.open, accent);
      if (FolderToRename() == id) {
        FolderToRename().clear();
        // After the layout that places it.
        base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
            FROM_HERE, base::BindOnce(&FolderHeader::BeginRename,
                                      header->GetWeakPtr()));
      }
    }
    container->InvalidateLayout();
  }

  raw_ptr<BrowserWindowInterface> browser_ = nullptr;
  raw_ptr<TabStripModel> model_ = nullptr;
  base::CallbackListSubscription folders_subscription_;
  bool sync_pending_ = false;
  bool reordering_ = false;
  base::WeakPtrFactory<FolderHeaders> weak_factory_{this};
};

BEGIN_METADATA(FolderHeaders)
END_METADATA

std::string MakeFolder(std::u16string name) {
  const std::string id = base::Uuid::GenerateRandomV4().AsLowercaseString();
  Folders()[id] = FolderState{std::move(name), std::string(), true, {}};
  return id;
}

// Empty leaves still need a tab to carry their session data. Keep a
// parent visible when its last child is moved or deleted.
void EnsureEmptyFolder(BrowserWindowInterface* browser, const std::string& id) {
  auto* model = browser->GetTabStripModel();
  if (id.empty() || !Folders().contains(id) || FolderRange(model, id, nullptr)) {
    return;
  }
  if (auto* contents = chrome::AddAndReturnTabAt(
          browser, GURL(), model->count(), /*foreground=*/false)) {
    SetTabFolderId(contents, id);
  }
}

}  // namespace

std::vector<FolderRow> GetFolderRows(const views::View* container,
                                     const TabCollectionNode* node) {
  std::vector<FolderRow> rows;
  if (!node) {
    return rows;
  }
  const auto children = node->GetDirectChildren();
  std::vector<std::vector<std::string>> paths;
  for (auto* child : children) {
    const auto id = FolderOfRow(child);
    paths.push_back(id ? FolderPath(*id) : std::vector<std::string>());
  }
  views::View* divider = nullptr;
  for (views::View* child : container->children()) {
    if (child->GetClassName() == std::string_view("MoriTabListHeader")) {
      divider = child;
    }
  }
  auto add_divider = [&] {
    if (divider) {
      rows.push_back({divider, false, 0});
      divider = nullptr;
    }
  };
  for (const auto& entry : FolderTreeRows(paths)) {
    if (!entry.tab) {
      if (auto* header = FindHeader(container, entry.folder)) {
        rows.push_back({header, !FolderVisible(entry.folder, false),
                        static_cast<int>(entry.depth) * kFolderIndent});
      }
    } else {
      auto* child = children[*entry.tab];
      if (paths[*entry.tab].empty()) {
        add_divider();
      }
      const auto id = FolderOfRow(child);
      rows.push_back({child, id && !FolderVisible(*id, true),
                      static_cast<int>(entry.depth) * kFolderIndent});
    }
  }
  add_divider();
  return rows;
}

std::vector<views::View*> GetUnplacedFolderHeaders(
    const views::View* container,
    const std::vector<FolderRow>& rows) {
  std::vector<views::View*> unplaced;
  for (FolderHeader* header : HeadersOf(container)) {
    if (std::ranges::none_of(rows, [header](const FolderRow& row) {
          return row.view == header;
        })) {
      unplaced.push_back(header);
    }
  }
  for (views::View* child : container->children()) {
    if (child->GetClassName() == std::string_view("MoriTabListHeader") &&
        std::ranges::none_of(rows, [child](const FolderRow& row) {
          return row.view == child;
        })) {
      unplaced.push_back(child);
    }
  }
  return unplaced;
}

std::unique_ptr<views::View> CreateFolderHeaders() {
  return std::make_unique<FolderHeaders>();
}

std::string GetTabFolderData(const content::WebContents* contents) {
  const std::optional<std::string> id = FolderIdOf(contents);
  if (!id) {
    return std::string();
  }
  std::vector<FolderSessionNode> path;
  for (const auto& folder : FolderPath(*id)) {
    const auto& state = Folders().at(folder);
    path.push_back({folder, base::UTF16ToUTF8(state.name), state.icon, state.open});
  }
  return EncodeFolderSession(path);
}

void PersistTabFolder(content::WebContents* contents) {
  SaveTabFolder(contents);
}

void RestoreTabFolder(content::WebContents* contents,
                      const std::map<std::string, std::string>& extra_data) {
  auto it = extra_data.find(kTabFolderKey);
  if (it == extra_data.end() || it->second.empty()) {
    return;
  }
  const auto path = DecodeFolderSession(it->second);
  if (path.empty()) {
    return;
  }
  std::string parent;
  for (const auto& node : path) {
    if (!Folders().contains(node.id)) {
      Folders()[node.id] = FolderState{base::UTF8ToUTF16(node.name),
                                      node.icon, node.open, parent};
    }
    parent = node.id;
  }
  TabFolder::CreateForWebContents(contents, path.back().id);
}

std::optional<FolderInfo> GetTabFolder(const tabs::TabInterface* tab) {
  if (std::optional<std::string> id = FolderIdOf(tab)) {
    return InfoFor(*id);
  }
  return std::nullopt;
}

std::optional<FolderInfo> GetFolder(const std::string& folder_id) {
  if (!Folders().contains(folder_id)) {
    return std::nullopt;
  }
  return InfoFor(folder_id);
}

std::vector<FolderInfo> ListFolders(BrowserWindowInterface* browser) {
  std::vector<FolderInfo> folders;
  std::set<std::string> seen;
  TabStripModel* model = browser->GetTabStripModel();
  for (int i = 0; i < model->count(); ++i) {
    tabs::TabInterface* other = model->GetTabAtIndex(i);
    if (other->IsPinned()) {
      continue;
    }
    if (std::optional<std::string> id = FolderIdOf(other);
        id) {
      for (const auto& ancestor : FolderPath(*id)) {
        if (seen.insert(ancestor).second) {
          folders.push_back(InfoFor(ancestor));
        }
      }
    }
  }
  return folders;
}

void NewFolder(BrowserWindowInterface* browser) {
  content::WebContents* contents = chrome::AddAndReturnTabAt(
      browser, GURL(), browser->GetTabStripModel()->IndexOfFirstNonPinnedTab(),
      /*foreground=*/true);
  if (!contents) {
    return;
  }
  NewFolderWithTab(browser, browser->GetTabStripModel()->GetActiveTab());
}

void NewSubfolder(BrowserWindowInterface* browser, const std::string& parent) {
  if (!Folders().contains(parent)) {
    return;
  }
  auto* contents = chrome::AddAndReturnTabAt(
      browser, GURL(), IndexAfterFolders(browser), /*foreground=*/true);
  if (!contents) {
    return;
  }
  const auto id = MakeFolder(u"New Folder");
  Folders()[id].parent = parent;
  for (const auto& ancestor : FolderPath(parent)) {
    Folders()[ancestor].open = true;
  }
  FolderToRename() = id;
  SetTabFolderId(contents, id);
  SaveFolderTabs(browser, parent);
  NotifyFoldersChanged();
}

void MoveFolder(BrowserWindowInterface* browser, const std::string& folder,
                const std::string& parent) {
  if (!Folders().contains(folder) ||
      (!parent.empty() && (!Folders().contains(parent) ||
                          !FolderCanContain(FolderPath(parent), folder)))) {
    return;
  }
  const std::string previous = Folders()[folder].parent;
  Folders()[folder].parent = parent;
  EnsureEmptyFolder(browser, previous);
  SaveFolderTabs(browser, folder);
  NotifyFoldersChanged();
}

int IndexAfterFolders(BrowserWindowInterface* browser) {
  auto* model = browser->GetTabStripModel();
  int index = model->IndexOfFirstNonPinnedTab();
  for (int i = index; i < model->count(); ++i) {
    if (FolderIdOf(model->GetTabAtIndex(i))) {
      index = i + 1;
    }
  }
  return index;
}

void NewFolderWithTab(BrowserWindowInterface* browser,
                      tabs::TabInterface* tab) {
  TabStripModel* model = browser->GetTabStripModel();
  if (FolderIdOf(tab)) {
    RemoveTabFromFolder(browser, tab);
  }
  const int index = model->GetIndexOfTab(tab);
  if (index == TabStripModel::kNoTab) {
    return;
  }
  if (model->IsTabPinned(index)) {
    model->SetTabPinned(index, false);
  }
  const std::string id = MakeFolder(u"New Folder");
  FolderToRename() = id;
  SetTabFolderId(tab->GetContents(), id);
  NotifyFoldersChanged();
}

void AddTabToFolder(BrowserWindowInterface* browser,
                    tabs::TabInterface* tab,
                    const std::string& folder_id) {
  TabStripModel* model = browser->GetTabStripModel();
  const std::optional<std::pair<int, int>> range =
      FolderRange(model, folder_id, tab);
  if (!range || FolderIdOf(tab) == folder_id) {
    return;
  }
  if (FolderIdOf(tab)) {
    RemoveTabFromFolder(browser, tab);
  }
  SetTabFolderId(tab->GetContents(), folder_id);
  MoveTabAfter(model, model->GetIndexOfTab(tab),
               FolderRange(model, folder_id, tab)->second);
  NotifyFoldersChanged();
}

void RemoveTabFromFolder(BrowserWindowInterface* browser,
                         tabs::TabInterface* tab) {
  const std::optional<std::string> id = FolderIdOf(tab);
  if (!id) {
    return;
  }
  TabStripModel* model = browser->GetTabStripModel();
  SetTabFolderId(tab->GetContents(), std::nullopt);
  if (const std::optional<std::pair<int, int>> range =
          FolderRange(model, *id, tab)) {
    MoveTabAfter(model, model->GetIndexOfTab(tab), range->second);
  }
  EnsureEmptyFolder(browser, *id);
  NotifyFoldersChanged();
}

void QueueSidebarTabDrop(BrowserWindowInterface* browser,
                         std::vector<tabs::TabHandle> dragged,
                         const gfx::Point& point) {
  auto* view = BrowserView::GetBrowserViewForBrowser(browser);
  auto* region = view ? views::AsViewClass<VerticalTabStripRegionView>(
                            view->tab_strip_view()) : nullptr;
  if (!region || !region->IsDrawn() ||
      !region->GetBoundsInScreen().Contains(point) || dragged.empty()) {
    return;
  }
    // Use the region's tab-strip accessor rather than its child hierarchy.
    auto* strip = views::AsViewClass<TabStripView>(region->GetTabStripView());
    if (!strip || !strip->IsDrawn()) {
    return;
  }
  const auto is_dragged = [&](const tabs::TabInterface* tab) {
    return std::ranges::find(dragged, tab->GetHandle()) != dragged.end();
  };
  auto pinned_bounds = strip->pinned_tabs_scroll_view()->GetBoundsInScreen();
  pinned_bounds.set_x(region->GetBoundsInScreen().x());
  pinned_bounds.set_width(region->width());
  const bool favorite = pinned_bounds.Contains(point);
  std::optional<std::string> folder;
  std::optional<tabs::TabHandle> anchor;
  bool after_anchor = false;
  bool header_hit = false;
  auto* container = strip->GetUnpinnedTabsContainer();
  if (favorite) {
    for (views::View* row : strip->GetPinnedTabsContainer()->children()) {
      const auto tabs = TabsOfRow(row);
      if (tabs.empty() || is_dragged(tabs.front()) || !row->IsDrawn()) {
        continue;
      }
      if (row->GetBoundsInScreen().Contains(point)) {
        anchor = tabs.front()->GetHandle();
        after_anchor = point.x() >= row->GetBoundsInScreen().CenterPoint().x();
        break;
      }
    }
  } else if (container && container->collection_node()) {
    const auto viewport = strip->unpinned_tabs_scroll_view()->GetBoundsInScreen();
    if (viewport.Contains(point)) {
      // Headers remain targets even when every tab in the folder is collapsed.
      for (auto* header : HeadersOf(container)) {
        if (header->IsDrawn() && header->GetBoundsInScreen().Contains(point)) {
          folder = header->id();
          header_hit = true;
          break;
        }
      }
      if (!header_hit) {
        // Gaps and dragged-row placeholders belong to the preceding visible
        // folder section, until the Add Tab divider starts ordinary tabs.
        int previous_y = viewport.y() - 1;
        for (const auto& row : GetFolderRows(container, container->collection_node())) {
          if (row.hidden || !row.view->IsDrawn()) {
            continue;
          }
          const auto tabs = TabsOfRow(row.view);
          if (!tabs.empty() && is_dragged(tabs.front())) {
            continue;
          }
          const int y = row.view->GetBoundsInScreen().y();
          if (y > point.y() || y < previous_y) {
            continue;
          }
          if (auto* header = views::AsViewClass<FolderHeader>(row.view)) {
            folder = header->id();
          } else if (!tabs.empty()) {
            folder = FolderOfRow(row.view);
          } else if (std::string_view(row.view->GetClassName()) == "MoriTabListHeader") {
            folder.reset();
          } else {
            continue;
          }
          previous_y = y;
        }
        for (const auto& row : GetFolderRows(container, container->collection_node())) {
          const auto tabs = TabsOfRow(row.view);
          if (row.hidden || tabs.empty() || is_dragged(tabs.front()) ||
              !row.view->IsDrawn()) {
            continue;
          }
          auto bounds = row.view->GetBoundsInScreen();
          bounds.set_x(viewport.x());
          bounds.set_width(viewport.width());
          if (bounds.Contains(point)) {
            folder = FolderOfRow(row.view);
            anchor = tabs.front()->GetHandle();
            after_anchor = point.y() >= bounds.CenterPoint().y();
            break;
          }
        }
      }
    }
  }

  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE, base::BindOnce(
          [](base::WeakPtr<BrowserWindowInterface> browser,
             std::vector<tabs::TabHandle> dragged, bool favorite,
             std::optional<std::string> folder,
             std::optional<tabs::TabHandle> anchor, bool after_anchor,
             bool header_hit) {
            if (!browser || (folder && !Folders().contains(*folder))) {
              return;
            }
            auto* model = browser->GetTabStripModel();
            std::set<std::string> previous_folders;
            bool same_destination = true;
            for (const auto& handle : dragged) {
              auto* tab = handle.Get();
              if (!tab || model->GetIndexOfTab(tab) == TabStripModel::kNoTab) {
                continue;
              }
              same_destination &= tab->IsPinned() == favorite &&
                  FolderIdOf(tab) == (favorite ? std::nullopt : folder);
              if (const auto old = FolderIdOf(tab)) {
                previous_folders.insert(*old);
              }
              // Pinning moves a tab between native collections. Do this only
              // after the native drag controller has released its view pointers.
              SetTabFolderId(tab->GetContents(), favorite ? std::nullopt : folder);
              model->SetTabPinned(model->GetIndexOfTab(tab), favorite);
            }
            int insertion = favorite ? model->IndexOfFirstNonPinnedTab()
                                     : model->count();
            if (folder) {
              if (const auto range = FolderRange(model, *folder, nullptr)) {
                insertion = range->second + 1;
              }
            }
            if (anchor && anchor->Get() &&
                model->GetIndexOfTab(anchor->Get()) != TabStripModel::kNoTab) {
              insertion = model->GetIndexOfTab(anchor->Get()) + (after_anchor ? 1 : 0);
            }
            for (const auto& handle : dragged) {
              if (same_destination && !anchor && !header_hit) {
                break;  // Keep native reordering within the same section.
              }
              auto* tab = handle.Get();
              if (!tab) {
                continue;
              }
              const int from = model->GetIndexOfTab(tab);
              if (from == TabStripModel::kNoTab) {
                continue;
              }
              const int to = std::clamp(insertion - (from < insertion ? 1 : 0),
                                         0, model->count() - 1);
              model->MoveWebContentsAt(from, to, false);
              insertion = model->GetIndexOfTab(tab) + 1;
            }
            for (const auto& old : previous_folders) {
              EnsureEmptyFolder(browser.get(), old);
            }
            NotifyFoldersChanged();
          }, browser->GetWeakPtr(), std::move(dragged), favorite, folder,
          anchor, after_anchor, header_hit));
}

void NewTabInFolder(BrowserWindowInterface* browser,
                    const std::string& folder_id) {
  TabStripModel* model = browser->GetTabStripModel();
  const std::optional<std::pair<int, int>> range =
      FolderRange(model, folder_id, nullptr);
  if (!range) {
    return;
  }
  const std::optional<tab_groups::TabGroupId> group =
      model->GetTabAtIndex(range->second)->GetGroup();
  content::WebContents* contents = chrome::AddAndReturnTabAt(
      browser, GURL(), range->second + 1, /*foreground=*/true, group);
  if (!contents) {
    return;
  }
  Folders()[folder_id].open = true;
  SetTabFolderId(contents, folder_id);
  SaveFolderTabs(browser, folder_id);
  NotifyFoldersChanged();
  chrome::ExecuteCommand(browser, IDC_FOCUS_LOCATION);
}

void RenameFolder(BrowserWindowInterface* browser,
                  const std::string& folder_id) {
  FolderToRename() = folder_id;
  NotifyFoldersChanged();
}

void ToggleFolder(BrowserWindowInterface* browser,
                  const std::string& folder_id) {
  if (!Folders().contains(folder_id)) {
    return;
  }
  Folders()[folder_id].open = !Folders()[folder_id].open;
  SaveFolderTabs(browser, folder_id);
  NotifyFoldersChanged();
}

void SetFolderIcon(BrowserWindowInterface* browser,
                   const std::string& folder_id,
                   const std::string& icon) {
  if (!Folders().contains(folder_id)) {
    return;
  }
  Folders()[folder_id].icon = icon;
  SaveFolderTabs(browser, folder_id);
  NotifyFoldersChanged();
}

void UngroupFolder(BrowserWindowInterface* browser,
                   const std::string& folder_id) {
  TabStripModel* model = browser->GetTabStripModel();
  for (int i = 0; i < model->count(); ++i) {
    content::WebContents* contents = model->GetWebContentsAt(i);
    if (FolderIdOf(contents) == folder_id) {
      const auto& parent = Folders()[folder_id].parent;
      SetTabFolderId(contents, parent.empty() ? std::nullopt : std::make_optional(parent));
    }
  }
  for (auto& [id, state] : Folders()) {
    if (state.parent == folder_id) {
      state.parent = Folders()[folder_id].parent;
      SaveFolderTabs(browser, id);
    }
  }
  NotifyFoldersChanged();
}

void DeleteFolder(BrowserWindowInterface* browser,
                  const std::string& folder_id) {
  const std::string parent = Folders().contains(folder_id)
                                ? Folders()[folder_id].parent : std::string();
  TabStripModel* model = browser->GetTabStripModel();
  for (int i = model->count() - 1; i >= 0; --i) {
    if (InFolder(FolderIdOf(model->GetWebContentsAt(i)), folder_id)) {
      model->CloseWebContentsAt(i, TabCloseTypes::CLOSE_USER_GESTURE);
    }
  }
  EnsureEmptyFolder(browser, parent);
  NotifyFoldersChanged();
}

}  // namespace mori
