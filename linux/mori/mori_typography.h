#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_TYPOGRAPHY_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_TYPOGRAPHY_H_

#include "ui/gfx/font_list.h"

namespace mori {

// Shared by browser chrome and Mori panels; matches the sidebar URL field.
inline constexpr int kUiFontSize = 15;

inline gfx::FontList UiFont(
    gfx::Font::Weight weight = gfx::Font::Weight::NORMAL) {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(kUiFontSize - base.GetFontSize())
      .DeriveWithWeight(weight);
}

inline gfx::FontList UiMonospaceFont() {
  return gfx::FontList({"Menlo", "Consolas", "monospace"}, gfx::Font::NORMAL,
                       kUiFontSize, gfx::Font::Weight::NORMAL);
}

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_TYPOGRAPHY_H_
