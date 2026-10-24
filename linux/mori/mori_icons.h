// Mori draws Lucide icons, as Mori does, in place of Chromium's own: back,
// forward, reload, the menu, the address field, tabs, the command bar.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_ICONS_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_ICONS_H_

#include <string_view>

#include "base/component_export.h"
#include "third_party/skia/include/core/SkColor.h"

namespace gfx {
class Canvas;
struct VectorIcon;
}  // namespace gfx

namespace mori {

// The Lucide icon `name` (e.g. "layers") as a Chromium icon, for Mori code
// that hands icons to Chromium (menus, buttons).
COMPONENT_EXPORT(GFX) const gfx::VectorIcon& LucideIcon(std::string_view name);

// Paints Mori's Lucide version of `icon` at `dip_size`. Returns false when
// Mori has no replacement and Chromium's own icon should be painted.
bool PaintReplacementIcon(gfx::Canvas* canvas,
                          const gfx::VectorIcon& icon,
                          int dip_size,
                          SkColor color);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_ICONS_H_
