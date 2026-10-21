#include "chrome/browser/ui/views/mori/mori_themes.h"

#include <algorithm>
#include <cmath>
#include <map>
#include <memory>
#include <utility>

#include "base/files/file_path.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/strings/string_number_conversions.h"
#include "base/strings/stringprintf.h"
#include "base/values.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_json_file.h"
#include "third_party/skia/include/core/SkBitmap.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/image/image_skia.h"
#include "ui/views/view.h"

namespace mori {
namespace {

// The opacity Mori 3 paints the wash at over the window background.
constexpr float kWashOpacity = 0.5f;

std::string ColorHex(SkColor color) {
  return base::StringPrintf("#%02X%02X%02X", SkColorGetR(color),
                            SkColorGetG(color), SkColorGetB(color));
}

std::optional<SkColor> ParseHex(const std::string& text) {
  uint32_t value = 0;
  if (text.size() != 7 || text[0] != '#' ||
      !base::HexStringToUInt(text.substr(1), &value)) {
    return std::nullopt;
  }
  return SkColorSetRGB((value >> 16) & 0xFF, (value >> 8) & 0xFF,
                       value & 0xFF);
}

// A ["#RRGGBB", "#RRGGBB"] list in `dict`'s "colors".
ThemeColors ColorsIn(const base::DictValue& dict) {
  const base::ListValue* colors = dict.FindList("colors");
  if (!colors || colors->size() != 2 || !(*colors)[0].is_string() ||
      !(*colors)[1].is_string()) {
    return std::nullopt;
  }
  std::optional<SkColor> a = ParseHex((*colors)[0].GetString());
  std::optional<SkColor> b = ParseHex((*colors)[1].GetString());
  if (!a || !b) {
    return std::nullopt;
  }
  return std::make_pair(*a, *b);
}

class ThemeStore {
 public:
  static ThemeStore* For(Profile* profile) {
    static base::NoDestructor<
        std::map<base::FilePath, std::unique_ptr<ThemeStore>>>
        stores;
    const base::FilePath path = profile->GetOriginalProfile()->GetPath();
    auto& store = (*stores)[path];
    if (!store) {
      store = std::make_unique<ThemeStore>(path);
    }
    return store.get();
  }

  explicit ThemeStore(const base::FilePath& profile_path)
      : profile_path_(profile_path),
        file_(profile_path.AppendASCII("Mori Theme.json"),
              base::BindOnce(&ThemeStore::OnLoaded, base::Unretained(this))) {}

  ThemeColors Get() const { return colors_; }

  void Set(ThemeColors colors) {
    colors_ = colors;
    Save();
    observers_.Notify();
  }

  base::CallbackListSubscription AddObserver(base::RepeatingClosure closure) {
    return observers_.Add(std::move(closure));
  }

 private:
  void OnLoaded(std::optional<base::Value> value) {
    if (value && value->is_dict()) {
      colors_ = ColorsIn(value->GetDict());
      observers_.Notify();
      return;
    }
    // A profile from when each space had its own theme ("Mori Spaces.json",
    // by space): the first space with a gradient gives Mori its theme.
    old_file_ = std::make_unique<JsonFile>(
        profile_path_.AppendASCII("Mori Spaces.json"),
        base::BindOnce(&ThemeStore::OnOldLoaded, base::Unretained(this)));
  }

  void OnOldLoaded(std::optional<base::Value> value) {
    if (value && value->is_dict()) {
      for (auto [key, item] : value->GetDict()) {
        if (const base::DictValue* dict = item.GetIfDict()) {
          if (ThemeColors colors = ColorsIn(*dict)) {
            colors_ = colors;
            Save();
            break;
          }
        }
      }
    }
    observers_.Notify();
  }

  void Save() {
    base::DictValue dict;
    if (colors_) {
      dict.Set("colors", base::ListValue()
                             .Append(ColorHex(colors_->first))
                             .Append(ColorHex(colors_->second)));
    }
    file_.Write(base::Value(std::move(dict)));
  }

  const base::FilePath profile_path_;
  ThemeColors colors_;
  base::RepeatingClosureList observers_;
  std::unique_ptr<JsonFile> old_file_;
  // Last: its load callback uses the members above.
  JsonFile file_;
};

// Mori 3's wash (theme.rs render_wash): the first colour, blooming into
// the second towards the bottom right. Small; it's stretched over the
// window.
const SkBitmap& WashBitmap(std::pair<SkColor, SkColor> colors) {
  static base::NoDestructor<std::map<std::pair<SkColor, SkColor>, SkBitmap>>
      cache;
  auto it = cache->find(colors);
  if (it != cache->end()) {
    return it->second;
  }
  constexpr int kWidth = 192;
  constexpr int kHeight = 128;
  SkBitmap bitmap;
  bitmap.allocN32Pixels(kWidth, kHeight);
  auto channel = [](SkColor c, int shift) { return ((c >> shift) & 0xFF) / 255.f; };
  const float base[3] = {channel(colors.first, 16), channel(colors.first, 8),
                         channel(colors.first, 0)};
  const float bloom[3] = {channel(colors.second, 16),
                          channel(colors.second, 8), channel(colors.second, 0)};
  for (int y = 0; y < kHeight; ++y) {
    for (int x = 0; x < kWidth; ++x) {
      const float u = static_cast<float>(x) / (kWidth - 1);
      const float v = static_cast<float>(y) / (kHeight - 1);
      const float d = std::hypot(u - 1.f, v - 1.f) / 1.1f;
      float t = std::clamp(1.f - d, 0.f, 1.f);
      t = t * t * (3.f - 2.f * t);
      auto mix = [&](int i) {
        return static_cast<U8CPU>(
            std::lround(std::clamp(base[i] + (bloom[i] - base[i]) * t, 0.f,
                                   1.f) *
                        255.f));
      };
      *bitmap.getAddr32(x, y) = SkPreMultiplyARGB(255, mix(0), mix(1), mix(2));
    }
  }
  bitmap.setImmutable();
  return cache->emplace(colors, std::move(bitmap)).first->second;
}

void RepaintTree(views::View* view) {
  view->SchedulePaint();
  for (views::View* child : view->children()) {
    RepaintTree(child);
  }
}

}  // namespace

ThemeColors GetTheme(BrowserWindowInterface* browser) {
  return ThemeStore::For(browser->GetProfile())->Get();
}

void SetTheme(BrowserWindowInterface* browser, ThemeColors colors) {
  ThemeStore::For(browser->GetProfile())->Set(colors);
}

base::CallbackListSubscription AddThemeObserver(BrowserWindowInterface* browser,
                                                base::RepeatingClosure closure) {
  return ThemeStore::For(browser->GetProfile())
      ->AddObserver(std::move(closure));
}

void PaintThemeWash(gfx::Canvas* canvas,
                    const views::View* view,
                    const BrowserView* browser_view) {
  if (!browser_view || !view) {
    return;
  }
  BrowserWindowInterface* browser =
      const_cast<BrowserView*>(browser_view)->browser();
  const ThemeColors colors = browser ? GetTheme(browser) : std::nullopt;
  if (!colors) {
    return;
  }
  // The wash spans the window, wherever `view` sits in it.
  const gfx::Rect dest = views::View::ConvertRectToTarget(
      browser_view, view, browser_view->GetLocalBounds());
  const SkBitmap& bitmap = WashBitmap(*colors);
  cc::PaintFlags flags;
  flags.setAlphaf(kWashOpacity);
  canvas->DrawImageInt(gfx::ImageSkia::CreateFrom1xBitmap(bitmap), 0, 0,
                       bitmap.width(), bitmap.height(), dest.x(), dest.y(),
                       dest.width(), dest.height(), /*filter=*/true, flags);
}

void RepaintThemeWash(BrowserWindowInterface* browser) {
  if (BrowserView* browser_view =
          BrowserView::GetBrowserViewForBrowser(browser)) {
    RepaintTree(browser_view);
  }
}

}  // namespace mori
