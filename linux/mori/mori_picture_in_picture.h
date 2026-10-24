#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_PICTURE_IN_PICTURE_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_PICTURE_IN_PICTURE_H_

class BrowserWindowInterface;
class PrefRegistrySimple;
namespace content { class WebContents; }

namespace mori {

inline constexpr char kPictureInPictureEnabled[] = "mori.picture_in_picture.enabled";
inline constexpr char kPictureInPictureHover[] = "mori.picture_in_picture.hover_button";
inline constexpr char kPictureInPictureAuto[] = "mori.picture_in_picture.on_tab_switch";
inline constexpr char kPictureInPictureReturn[] = "mori.picture_in_picture.return_to_tab";

void RegisterPictureInPicturePrefs(PrefRegistrySimple* registry);
void AttachPictureInPicture(content::WebContents* contents);
bool IsPictureInPictureEnabled(content::WebContents* contents);
void TogglePictureInPicture(BrowserWindowInterface* browser);

}  // namespace mori

#endif
