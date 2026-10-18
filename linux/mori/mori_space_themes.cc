#include "chrome/browser/ui/views/mori/mori_space_themes.h"

#include <algorithm>
#include <cmath>
#include <map>
#include <memory>
#include <utility>

#include "base/files/file_path.h"
#include "base/files/file_util.h"
#include "base/files/important_file_writer.h"
#include "base/functional/bind.h"
#include "base/json/json_reader.h"
#include "base/json/json_writer.h"
#include "base/memory/weak_ptr.h"
#include "base/no_destructor.h"
#include "base/strings/string_number_conversions.h"
#include "base/strings/stringprintf.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/thread_pool.h"
#include "base/values.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/tab_group_sync/tab_group_sync_service_factory.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_spaces.h"
#include "components/saved_tab_groups/public/saved_tab_group.h"
#include "components/saved_tab_groups/public/tab_group_sync_service.h"
#include "third_party/skia/include/core/SkBitmap.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/scoped_canvas.h"
#include "ui/gfx/font_list.h"
#include "ui/gfx/geometry/rect.h"
#include "ui/gfx/image/image_skia.h"
#include "ui/gfx/paint_vector_icon.h"
#include "ui/gfx/text_constants.h"
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

class ThemeStore {
 public:
  static ThemeStore* For(Profile* profile) {
    static base::NoDestructor<
        std::map<base::FilePath, std::unique_ptr<ThemeStore>>>
        stores;
    const base::FilePath path = profile->GetOriginalProfile()->GetPath();
    auto& store = (*stores)[path];
    if (!store) {
      store = std::make_unique<ThemeStore>(
          path.AppendASCII("Mori Spaces.json"));
    }
    return store.get();
  }

  explicit ThemeStore(base::FilePath path)
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
        base::BindOnce(&ThemeStore::OnLoaded, weak_factory_.GetWeakPtr()));
  }

  SpaceTheme Get(const std::string& key) const {
    auto it = themes_.find(key);
    return it == themes_.end() ? SpaceTheme() : it->second;
  }

  void Set(const std::string& key, const SpaceTheme& theme) {
    if (theme == SpaceTheme()) {
      themes_.erase(key);
    } else {
      themes_[key] = theme;
    }
    Save();
    observers_.Notify();
  }

  base::CallbackListSubscription AddObserver(base::RepeatingClosure closure) {
    return observers_.Add(std::move(closure));
  }

 private:
  void OnLoaded(std::string json) {
    std::optional<base::Value> value = base::JSONReader::Read(
        json, base::JSON_PARSE_CHROMIUM_EXTENSIONS);
    if (value && value->is_dict()) {
      for (auto [key, item] : value->GetDict()) {
        const base::DictValue* dict = item.GetIfDict();
        if (!dict) {
          continue;
        }
        SpaceTheme theme;
        if (const std::string* symbol = dict->FindString("symbol")) {
          theme.symbol = *symbol;
        }
        const base::ListValue* colors = dict->FindList("colors");
        if (colors && colors->size() == 2 && (*colors)[0].is_string() &&
            (*colors)[1].is_string()) {
          std::optional<SkColor> a = ParseHex((*colors)[0].GetString());
          std::optional<SkColor> b = ParseHex((*colors)[1].GetString());
          if (a && b) {
            theme.colors = std::make_pair(*a, *b);
          }
        }
        themes_.emplace(key, std::move(theme));
      }
    }
    observers_.Notify();
  }

  void Save() {
    base::DictValue dict;
    for (const auto& [key, theme] : themes_) {
      base::DictValue entry;
      entry.Set("symbol", theme.symbol);
      if (theme.colors) {
        entry.Set("colors", base::ListValue()
                                .Append(ColorHex(theme.colors->first))
                                .Append(ColorHex(theme.colors->second)));
      }
      dict.Set(key, std::move(entry));
    }
    std::string json;
    base::JSONWriter::WriteWithOptions(
        dict, base::JSONWriter::OPTIONS_PRETTY_PRINT, &json);
    file_runner_->PostTask(
        FROM_HERE, base::BindOnce(
                       [](base::FilePath path, std::string json) {
                         base::ImportantFileWriter::WriteFileAtomically(
                             path, json);
                       },
                       path_, std::move(json)));
  }

  base::FilePath path_;
  scoped_refptr<base::SequencedTaskRunner> file_runner_;
  std::map<std::string, SpaceTheme> themes_;
  base::RepeatingClosureList observers_;
  base::WeakPtrFactory<ThemeStore> weak_factory_{this};
};

// A space outlives restarts as its saved tab group; its local id doesn't.
std::string KeyFor(BrowserWindowInterface* browser,
                   const tab_groups::TabGroupId& space) {
  if (tab_groups::TabGroupSyncService* sync =
          tab_groups::TabGroupSyncServiceFactory::GetForProfile(
              browser->GetProfile())) {
    if (std::optional<tab_groups::SavedTabGroup> group =
            sync->GetGroup(space)) {
      return group->saved_guid().AsLowercaseString();
    }
  }
  return space.ToString();
}

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

SpaceTheme GetSpaceTheme(BrowserWindowInterface* browser,
                         const tab_groups::TabGroupId& space) {
  return ThemeStore::For(browser->GetProfile())
      ->Get(KeyFor(browser, space));
}

void SetSpaceTheme(BrowserWindowInterface* browser,
                   const tab_groups::TabGroupId& space,
                   const SpaceTheme& theme) {
  ThemeStore::For(browser->GetProfile())->Set(KeyFor(browser, space), theme);
}

base::CallbackListSubscription AddSpaceThemeObserver(
    BrowserWindowInterface* browser,
    base::RepeatingClosure closure) {
  return ThemeStore::For(browser->GetProfile())
      ->AddObserver(std::move(closure));
}

void PaintSpaceWash(gfx::Canvas* canvas,
                    const views::View* view,
                    const BrowserView* browser_view) {
  if (!browser_view || !view) {
    return;
  }
  BrowserWindowInterface* browser =
      const_cast<BrowserView*>(browser_view)->browser();
  const std::optional<tab_groups::TabGroupId> space =
      browser ? GetCurrentSpace(browser) : std::nullopt;
  if (!space) {
    return;
  }
  const SpaceTheme theme = GetSpaceTheme(browser, *space);
  if (!theme.colors) {
    return;
  }
  // The wash spans the window, wherever `view` sits in it.
  const gfx::Rect dest = views::View::ConvertRectToTarget(
      browser_view, view, browser_view->GetLocalBounds());
  const SkBitmap& bitmap = WashBitmap(*theme.colors);
  cc::PaintFlags flags;
  flags.setAlphaf(kWashOpacity);
  canvas->DrawImageInt(gfx::ImageSkia::CreateFrom1xBitmap(bitmap), 0, 0,
                       bitmap.width(), bitmap.height(), dest.x(), dest.y(),
                       dest.width(), dest.height(), /*filter=*/true, flags);
}

void RepaintSpaceWash(BrowserWindowInterface* browser) {
  if (BrowserView* browser_view =
          BrowserView::GetBrowserViewForBrowser(browser)) {
    RepaintTree(browser_view);
  }
}

bool IsEmojiSymbol(std::string_view symbol) {
  // Lucide names are lowercase ASCII words and dashes.
  return !symbol.empty() &&
         std::ranges::any_of(symbol, [](char c) {
           return static_cast<unsigned char>(c) >= 0x80;
         });
}

void PaintSpaceSymbol(gfx::Canvas* canvas,
                      std::string_view symbol,
                      const gfx::Rect& bounds,
                      int size,
                      SkColor color) {
  if (IsEmojiSymbol(symbol)) {
    const gfx::FontList base;
    canvas->DrawStringRectWithFlags(
        base::UTF8ToUTF16(symbol),
        base.DeriveWithSizeDelta(size - 1 - base.GetFontSize()), color, bounds,
        gfx::Canvas::TEXT_ALIGN_CENTER);
    return;
  }
  gfx::ScopedCanvas scoped(canvas);
  canvas->Translate(gfx::Vector2d(bounds.x() + (bounds.width() - size) / 2,
                                  bounds.y() + (bounds.height() - size) / 2));
  gfx::PaintVectorIcon(canvas, LucideIcon(symbol), size, color);
}

}  // namespace mori
