#ifndef CHROME_BROWSER_UI_MORI_MORI_PERMISSION_PROMPT_H_
#define CHROME_BROWSER_UI_MORI_MORI_PERMISSION_PROMPT_H_

#include <memory>

#include "components/permissions/permission_prompt.h"

namespace content {
class WebContents;
}

namespace mori {

// A native sheet on the Mori window standing in for Chrome's views
// permission bubble (which needs the toolbar we replaced).
std::unique_ptr<permissions::PermissionPrompt> CreateMoriPermissionPrompt(
    content::WebContents* web_contents,
    permissions::PermissionPrompt::Delegate* delegate);

}  // namespace mori

#endif  // CHROME_BROWSER_UI_MORI_MORI_PERMISSION_PROMPT_H_
