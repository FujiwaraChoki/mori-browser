// Lives in //ui/gfx's component (see patches/0008): gfx::PaintVectorIcon asks
// it first. Only Skia and //base are used here.

#include "chrome/browser/ui/views/mori/mori_icons.h"

#include <string_view>
#include <vector>

#include "base/containers/span.h"
#include "base/no_destructor.h"
#include "cc/paint/paint_flags.h"
#include "third_party/skia/include/core/SkPathBuilder.h"
#include "third_party/skia/include/core/SkRRect.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/scoped_canvas.h"
#include "ui/gfx/vector_icon_types.h"

namespace mori {
namespace {

// One drawing operation in Lucide's 24x24 space:
// M x y | L x y | C x1 y1 x2 y2 x y | Q x1 y1 x y |
// A rx ry rotation large sweep x y | Z | O cx cy r | E cx cy rx ry |
// R x y w h rx ry.
struct IconOp {
  char op;
  float v[7];
};

struct LucideIconData {
  std::string_view name;
  base::span<const IconOp> ops;
  const gfx::VectorIcon* vector_icon;
};

constexpr gfx::PathElement kPlaceholderPath[] = {gfx::CANVAS_DIMENSIONS, 24,
                                                 gfx::CLOSE};
constexpr gfx::VectorIconRep kPlaceholderReps[] = {{kPlaceholderPath}};

#include "chrome/browser/ui/views/mori/mori_lucide_icons.inc"

struct Replacement {
  // Chromium's icon (the C++ name, without namespace).
  std::string_view chromium;
  std::string_view lucide;
  bool filled = false;
  // Painted in this colour instead of Chromium's, when set.
  SkColor color = SK_ColorTRANSPARENT;
};

constexpr Replacement kReplacements[] = {
    // Toolbar.
    {"kBackArrowChromeRefreshOldIcon", "arrow-left"},
    {"kBackArrowTouchOldIcon", "arrow-left"},
    {"kArrowBackIcon", "arrow-left"},
    {"kForwardArrowChromeRefreshOldIcon", "arrow-right"},
    {"kForwardArrowTouchOldIcon", "arrow-right"},
    {"kArrowForwardIcon", "arrow-right"},
    {"kReloadChromeRefreshOldIcon", "rotate-cw"},
    {"kReloadTouchOldIcon", "rotate-cw"},
    {"kRefreshIcon", "rotate-cw"},
    {"kNavigateStopChromeRefreshOldIcon", "x"},
    {"kNavigateStopTouchOldIcon", "x"},
    {"kBrowserToolsChromeRefreshOldIcon", "ellipsis"},
    {"kBrowserToolsTouchOldIcon", "ellipsis"},
    {"kMoreVertIcon", "ellipsis"},
    {"kExtensionChromeRefreshOldIcon", "puzzle"},
    {"kChromeExtensionIcon", "puzzle"},
    {"kLeftPanelCloseFlippableIcon", "panel-left"},
    {"kDockToLeftIcon", "panel-left"},
    // Address field.
    {"kSecurePageInfoChromeRefreshOldIcon", "lock"},
    {"kPageInfoCustomIcon", "lock"},
    {"kStarChromeRefreshOldIcon", "star"},
    {"kStarIcon", "star"},
    // Mori 3: a bookmarked page's star is a yellow outline.
    {"kStarActiveChromeRefreshOldIcon", "star", false, SkColorSetRGB(0xFF, 0xB9, 0x00)},
    {"kStarFilledIcon", "star", false, SkColorSetRGB(0xFF, 0xB9, 0x00)},
    // Tabs and sidebar.
    {"kCloseTabChromeRefreshOldIcon", "x"},
    {"kTabCloseIcon", "x"},
    {"kCloseIcon", "x"},
    {"kAddIcon", "plus"},
    {"kKeepIcon", "pin"},
    {"kKeepOffIcon", "pin-off"},
    // Command bar results and commands.
    {"kSearchIcon", "search"},
    {"kSearchChromeRefreshOldIcon", "search"},
    {"kPageChromeRefreshOldIcon", "globe"},
    {"kPublicIcon", "globe"},
    {"kHistoryIcon", "clock"},
    {"kHistoryChromeRefreshOldIcon", "clock"},
    {"kBookmarkChromeRefreshOldIcon", "bookmark"},
    {"kBookmarkManagerIcon", "bookmark"},
    {"kTrendingUpChromeRefreshOldIcon", "trending-up"},
    {"kTrendingUpIcon", "trending-up"},
    {"kSubdirectoryArrowRightIcon", "corner-down-right"},
    {"kSubdirectoryArrowRightOldIcon", "corner-down-right"},
    {"kSettingsIcon", "settings"},
    {"kDownloadIcon", "download"},
    {"kIncognitoIcon", "venetian-mask"},
    {"kNewWindowIcon", "app-window"},
    {"kDeleteIcon", "trash"},
    {"kCodeIcon", "code"},
    {"kPrintIcon", "printer"},
    {"kSplitSceneIcon", "columns-2"},
    {"kContentCopyIcon", "copy"},
};

const LucideIconData* FindLucide(std::string_view name) {
  for (const LucideIconData& icon : kLucideIcons) {
    if (icon.name == name) {
      return &icon;
    }
  }
  return nullptr;
}

const Replacement* FindReplacement(const gfx::VectorIcon& icon) {
  if (!icon.name) {
    return nullptr;
  }
  std::string_view name = icon.name;
  // Mori's own icons are Lucide icons by name.
  if (name.starts_with("mori::")) {
    // One entry per icon, made on first use. Reserved up front so the
    // pointers handed out stay valid.
    static base::NoDestructor<std::vector<Replacement>> own([] {
      std::vector<Replacement> entries;
      entries.reserve(std::size(kLucideIcons));
      return entries;
    }());
    for (const Replacement& replacement : *own) {
      if (replacement.chromium == name) {
        return &replacement;
      }
    }
    if (own->size() == own->capacity()) {
      return nullptr;
    }
    own->push_back({name, name.substr(6)});
    return &own->back();
  }
  if (const size_t colon = name.rfind(':'); colon != std::string_view::npos) {
    name = name.substr(colon + 1);
  }
  for (const Replacement& replacement : kReplacements) {
    if (replacement.chromium == name) {
      return &replacement;
    }
  }
  return nullptr;
}

SkPath BuildPath(base::span<const IconOp> ops) {
  SkPathBuilder path;
  for (const IconOp& op : ops) {
    const float* v = op.v;
    switch (op.op) {
      case 'M':
        path.moveTo(v[0], v[1]);
        break;
      case 'L':
        path.lineTo(v[0], v[1]);
        break;
      case 'C':
        path.cubicTo(v[0], v[1], v[2], v[3], v[4], v[5]);
        break;
      case 'Q':
        path.quadTo(v[0], v[1], v[2], v[3]);
        break;
      case 'A':
        path.arcTo({v[0], v[1]}, v[2],
                   v[3] != 0 ? SkPathBuilder::kLarge_ArcSize
                             : SkPathBuilder::kSmall_ArcSize,
                   v[4] != 0 ? SkPathDirection::kCW : SkPathDirection::kCCW,
                   {v[5], v[6]});
        break;
      case 'Z':
        path.close();
        break;
      case 'O':
        path.addCircle(v[0], v[1], v[2]);
        break;
      case 'E':
        path.addOval(SkRect::MakeLTRB(v[0] - v[2], v[1] - v[3], v[0] + v[2],
                                      v[1] + v[3]));
        break;
      case 'R':
        path.addRRect(SkRRect::MakeRectXY(
            SkRect::MakeXYWH(v[0], v[1], v[2], v[3]), v[4], v[5]));
        break;
    }
  }
  return path.detach();
}

}  // namespace

const gfx::VectorIcon& LucideIcon(std::string_view name) {
  if (const LucideIconData* icon = FindLucide(name)) {
    return *icon->vector_icon;
  }
  return *FindLucide("circle")->vector_icon;
}

bool PaintReplacementIcon(gfx::Canvas* canvas,
                          const gfx::VectorIcon& icon,
                          int dip_size,
                          SkColor color) {
  const Replacement* replacement = FindReplacement(icon);
  const LucideIconData* lucide =
      replacement ? FindLucide(replacement->lucide) : nullptr;
  if (!lucide) {
    return false;
  }
  gfx::ScopedCanvas scoped(canvas);
  const float scale = dip_size / 24.0f;
  canvas->sk_canvas()->scale(scale, scale);
  cc::PaintFlags flags;
  flags.setAntiAlias(true);
  flags.setColor(replacement->color != SK_ColorTRANSPARENT ? replacement->color
                                                           : color);
  if (replacement->filled) {
    flags.setStyle(cc::PaintFlags::kFill_Style);
    canvas->DrawPath(BuildPath(lucide->ops), flags);
  }
  // Lucide: 2px strokes with round caps and joins.
  flags.setStyle(cc::PaintFlags::kStroke_Style);
  flags.setStrokeWidth(2.0f);
  flags.setStrokeCap(cc::PaintFlags::kRound_Cap);
  flags.setStrokeJoin(cc::PaintFlags::kRound_Join);
  canvas->DrawPath(BuildPath(lucide->ops), flags);
  return true;
}

}  // namespace mori
