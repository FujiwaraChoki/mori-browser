// Permission prompt as a non-modal Mori notification.

#include "chrome/browser/ui/mori/mori_permission_prompt.h"

#import <Cocoa/Cocoa.h>

#include <string>
#include <vector>

#include "base/memory/weak_ptr.h"
#include "base/strings/sys_string_conversions.h"
#include "chrome/browser/ui/mori/mori_chrome_hooks.h"

#include "components/permissions/permission_request.h"
#include "components/permissions/permission_uma_util.h"
#include "url/gurl.h"

// Swift-exported surface of MoriPermissionBridge
// (ToastCenter.swift); declared locally to avoid importing a
// generated Swift header through Chromium's mixed build.
@interface MoriPermissionBridge : NSObject
+ (void)showPermissionPromptWithID:(NSInteger)identifier
                            origin:(NSString*)origin
                          requests:(NSArray<NSString*>*)requests
                        completion:(void (^)(NSInteger response))completion;
+ (void)dismissPermissionPromptWithID:(NSInteger)identifier;
@end

namespace mori {

namespace {

enum class MoriPermissionResponse : NSInteger {
  kAllow = 0,
  kBlock = 1,
  kDismiss = 2,
};

int NextPermissionPromptID() {
  static int next_id = 1;
  return next_id++;
}

class MoriPermissionPrompt : public permissions::PermissionPrompt {
 public:
  MoriPermissionPrompt(content::WebContents* web_contents,
                       Delegate* delegate)
      : delegate_(delegate->GetWeakPtr()), prompt_id_(NextPermissionPromptID()) {
    NSWindow* window = MoriMainWindow();
    if (!window) {
      return;
    }

    NSMutableArray<NSString*>* requests = [NSMutableArray array];
    for (const auto& request : delegate->Requests()) {
      [requests addObject:base::SysUTF16ToNSString(
                              request->GetMessageTextFragment())];
    }

    base::WeakPtr<MoriPermissionPrompt> prompt = weak_factory_.GetWeakPtr();
    [MoriPermissionBridge
        showPermissionPromptWithID:prompt_id_
                            origin:base::SysUTF8ToNSString(
                                       std::string(
                                           delegate->GetRequestingOrigin().host()))
                          requests:requests
                        completion:^(NSInteger response) {
                          if (prompt) {
                            prompt->OnPromptResponse(
                                static_cast<MoriPermissionResponse>(response));
                          }
                        }];
    shown_ = true;
  }

  ~MoriPermissionPrompt() override {
    weak_factory_.InvalidateWeakPtrs();
    if (shown_ && !responded_) {
      [MoriPermissionBridge dismissPermissionPromptWithID:prompt_id_];
    }
  }

  bool UpdateAnchor() override { return true; }
  TabSwitchingBehavior GetTabSwitchingBehavior() override {
    return TabSwitchingBehavior::kDestroyPromptButKeepRequestPending;
  }
  permissions::PermissionPromptDisposition GetPromptDisposition()
      const override {
    return permissions::PermissionPromptDisposition::ANCHORED_BUBBLE;
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
  void OnPromptResponse(MoriPermissionResponse response) {
    if (responded_) {
      return;
    }
    responded_ = true;
    if (!delegate_) {
      return;
    }

    switch (response) {
      case MoriPermissionResponse::kAllow:
        delegate_->Accept({});
        return;
      case MoriPermissionResponse::kBlock:
        delegate_->Deny({});
        return;
      case MoriPermissionResponse::kDismiss:
        delegate_->Dismiss({});
        return;
    }
  }

  base::WeakPtr<Delegate> delegate_;
  int prompt_id_;
  bool shown_ = false;
  bool responded_ = false;
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
