// Permission prompt as a native sheet on the Mori window.

#include "chrome/browser/ui/mori/mori_permission_prompt.h"

#import <Cocoa/Cocoa.h>

#include "base/memory/weak_ptr.h"
#include "base/strings/sys_string_conversions.h"
#include "chrome/browser/ui/mori/mori_chrome_hooks.h"
#include <string>

#include "components/permissions/permission_request.h"
#include "components/permissions/permission_uma_util.h"
#include "url/gurl.h"

namespace mori {

namespace {

class MoriPermissionPrompt : public permissions::PermissionPrompt {
 public:
  MoriPermissionPrompt(content::WebContents* web_contents,
                       Delegate* delegate)
      : delegate_(delegate->GetWeakPtr()) {
    NSWindow* window = MoriMainWindow();
    if (!window) {
      return;
    }

    alert_ = [[NSAlert alloc] init];
    alert_.messageText = base::SysUTF8ToNSString(
        std::string(delegate->GetRequestingOrigin().host()) + " wants to:");
    NSMutableString* body = [NSMutableString string];
    for (const auto& request : delegate->Requests()) {
      [body appendFormat:@"• %@\n",
                         base::SysUTF16ToNSString(
                             request->GetMessageTextFragment())];
    }
    alert_.informativeText = body;
    [alert_ addButtonWithTitle:@"Allow"];
    [alert_ addButtonWithTitle:@"Block"];
    [alert_ addButtonWithTitle:@"Not Now"];

    sheet_parent_ = window;
    base::WeakPtr<MoriPermissionPrompt> prompt = weak_factory_.GetWeakPtr();
    [alert_ beginSheetModalForWindow:window
                   completionHandler:^(NSModalResponse response) {
                     if (prompt) {
                       prompt->OnSheetEnded(response);
                     }
                   }];
    shown_ = true;
  }

  ~MoriPermissionPrompt() override {
    weak_factory_.InvalidateWeakPtrs();
    if (sheet_parent_ && alert_.window.sheetParent == sheet_parent_) {
      [sheet_parent_ endSheet:alert_.window returnCode:NSModalResponseStop];
    }
  }

  bool UpdateAnchor() override { return true; }
  TabSwitchingBehavior GetTabSwitchingBehavior() override {
    return TabSwitchingBehavior::kDestroyPromptButKeepRequestPending;
  }
  permissions::PermissionPromptDisposition GetPromptDisposition()
      const override {
    return permissions::PermissionPromptDisposition::CUSTOM_MODAL_DIALOG;
  }
  bool IsAskPrompt() const override { return true; }
  std::optional<gfx::Rect> GetViewBoundsInScreen() const override {
    return std::nullopt;
  }
  bool ShouldFinalizeRequestAfterDecided() const override { return true; }
  std::vector<permissions::ElementAnchoredBubbleVariant> GetPromptVariants()
      const override {
    return {};
  }
  std::optional<permissions::feature_params::PermissionElementPromptPosition>
  GetPromptPosition() const override {
    return std::nullopt;
  }

  bool shown() const { return shown_; }

 private:
  void OnSheetEnded(NSModalResponse response) {
    sheet_parent_ = nil;
    alert_ = nil;

    if (!delegate_) {
      return;
    }

    if (response == NSAlertFirstButtonReturn) {
      delegate_->Accept({});
    } else if (response == NSAlertSecondButtonReturn) {
      delegate_->Deny({});
    } else {
      delegate_->Dismiss({});
    }
  }

  base::WeakPtr<Delegate> delegate_;
  NSAlert* alert_ = nil;
  NSWindow* sheet_parent_ = nil;
  bool shown_ = false;
  base::WeakPtrFactory<MoriPermissionPrompt> weak_factory_{this};
};

}  // namespace

std::unique_ptr<permissions::PermissionPrompt> CreateMoriPermissionPrompt(
    content::WebContents* web_contents,
    permissions::PermissionPrompt::Delegate* delegate) {
  auto prompt =
      std::make_unique<MoriPermissionPrompt>(web_contents, delegate);
  if (!prompt->shown()) {
    return nullptr;
  }
  return prompt;
}

}  // namespace mori
