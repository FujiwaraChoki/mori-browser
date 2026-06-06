// CefClient implementation for a single browser (tab).
//
// All navigation/display state is forwarded to a BrowserClientDelegate, which
// the ObjC++ view layer implements to drive the SwiftUI chrome. CEF invokes
// these handlers on the CEF UI thread which, with CefRunMessageLoop() on the
// main thread, is the AppKit main thread.
#pragma once

#include <atomic>
#include <string>
#include <vector>

#include "include/cef_client.h"
#include "include/cef_image.h"

// Pure-virtual sink the hosting view implements.
class BrowserClientDelegate {
 public:
  virtual ~BrowserClientDelegate() = default;
  virtual void OnAfterCreated(CefRefPtr<CefBrowser> browser) = 0;
  virtual void OnBeforeClose(CefRefPtr<CefBrowser> browser) = 0;
  virtual void OnTitleChange(const std::string& title) = 0;
  virtual void OnAddressChange(const std::string& url) = 0;
  virtual void OnLoadingStateChange(bool isLoading,
                                    bool canGoBack,
                                    bool canGoForward) = 0;
  virtual void OnFaviconURLChange(const std::vector<std::string>& icon_urls) = 0;
  // The site's favicon, downloaded and decoded by Chromium (handles ICO/SVG/
  // data-URI/PNG alike) and re-encoded as PNG bytes. `png_bytes` is null / `len`
  // 0 when the download failed, signalling the view to fall back.
  virtual void OnFaviconImage(const std::string& image_url,
                              const unsigned char* png_bytes,
                              size_t len) = 0;
  virtual void OnBeforeBrowse(const std::string& url,
                              bool is_redirect,
                              bool user_gesture) = 0;
  virtual void OnLoadStart(const std::string& url) = 0;
  virtual void OnLoadEnd(const std::string& url, int http_status_code) = 0;
  virtual void OnLoadError(int errorCode,
                           const std::string& errorText,
                           const std::string& failedUrl) = 0;
  // A popup / target=_blank URL that should be routed into Mori chrome
  // instead of a CEF-created top-level native window.
  virtual bool OnOpenURLFromTab(const std::string& target_url) = 0;
  // Find-in-page progress: total matches and the 1-based active match index.
  virtual void OnFindResult(int count, int activeMatchOrdinal) = 0;
};

class BrowserClient : public CefClient,
                      public CefLifeSpanHandler,
                      public CefLoadHandler,
                      public CefDisplayHandler,
                      public CefContextMenuHandler,
                      public CefDownloadHandler,
                      public CefJSDialogHandler,
                      public CefFindHandler,
                      public CefKeyboardHandler,
                      public CefRequestHandler,
                      public CefResourceRequestHandler {
 public:
  explicit BrowserClient(BrowserClientDelegate* delegate);

  // Detach when the hosting view goes away to avoid dangling callbacks.
  void DetachDelegate();
  void SetExtensionTabID(int tab_id);

  // CefClient
  CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override { return this; }
  CefRefPtr<CefLoadHandler> GetLoadHandler() override { return this; }
  CefRefPtr<CefDisplayHandler> GetDisplayHandler() override { return this; }
  CefRefPtr<CefContextMenuHandler> GetContextMenuHandler() override {
    return this;
  }
  CefRefPtr<CefDownloadHandler> GetDownloadHandler() override { return this; }
  CefRefPtr<CefJSDialogHandler> GetJSDialogHandler() override { return this; }
  CefRefPtr<CefFindHandler> GetFindHandler() override { return this; }
  CefRefPtr<CefKeyboardHandler> GetKeyboardHandler() override { return this; }
  CefRefPtr<CefRequestHandler> GetRequestHandler() override { return this; }

  // CefKeyboardHandler — intercept Mori's browser/app shortcuts before the
  // focused web page sees them, so they fire on the first press regardless of
  // where keyboard focus sits. Routed to the same dispatcher as the native
  // chrome (MoriRoot.handleShortcutEvent:).
  bool OnPreKeyEvent(CefRefPtr<CefBrowser> browser,
                     const CefKeyEvent& event,
                     CefEventHandle os_event,
                     bool* is_keyboard_shortcut) override;
  bool OnKeyEvent(CefRefPtr<CefBrowser> browser,
                  const CefKeyEvent& event,
                  CefEventHandle os_event) override;

  // CefRequestHandler
  bool OnBeforeBrowse(CefRefPtr<CefBrowser> browser,
                      CefRefPtr<CefFrame> frame,
                      CefRefPtr<CefRequest> request,
                      bool user_gesture,
                      bool is_redirect) override;
  bool OnOpenURLFromTab(CefRefPtr<CefBrowser> browser,
                        CefRefPtr<CefFrame> frame,
                        const CefString& target_url,
                        WindowOpenDisposition target_disposition,
                        bool user_gesture) override;

  // CefLifeSpanHandler
  bool OnBeforePopup(CefRefPtr<CefBrowser> browser,
                     CefRefPtr<CefFrame> frame,
                     int popup_id,
                     const CefString& target_url,
                     const CefString& target_frame_name,
                     CefLifeSpanHandler::WindowOpenDisposition target_disposition,
                     bool user_gesture,
                     const CefPopupFeatures& popupFeatures,
                     CefWindowInfo& windowInfo,
                     CefRefPtr<CefClient>& client,
                     CefBrowserSettings& settings,
                     CefRefPtr<CefDictionaryValue>& extra_info,
                     bool* no_javascript_access) override;
  void OnAfterCreated(CefRefPtr<CefBrowser> browser) override;
  void OnBeforeClose(CefRefPtr<CefBrowser> browser) override;

  // CefLoadHandler
  void OnLoadingStateChange(CefRefPtr<CefBrowser> browser,
                            bool isLoading,
                            bool canGoBack,
                            bool canGoForward) override;
  void OnLoadError(CefRefPtr<CefBrowser> browser,
                   CefRefPtr<CefFrame> frame,
                   ErrorCode errorCode,
                   const CefString& errorText,
                   const CefString& failedUrl) override;
  // Inject the WebAuthn/passkey shim as early as possible — before page scripts
  // can capture the original navigator.credentials methods.
  void OnLoadStart(CefRefPtr<CefBrowser> browser,
                   CefRefPtr<CefFrame> frame,
                   TransitionType transition_type) override;
  // Inject the media/PiP agent into each frame once it finishes loading.
  void OnLoadEnd(CefRefPtr<CefBrowser> browser,
                 CefRefPtr<CefFrame> frame,
                 int httpStatusCode) override;

  // CefDisplayHandler
  void OnTitleChange(CefRefPtr<CefBrowser> browser,
                     const CefString& title) override;
  void OnAddressChange(CefRefPtr<CefBrowser> browser,
                       CefRefPtr<CefFrame> frame,
                       const CefString& url) override;
  void OnFaviconURLChange(CefRefPtr<CefBrowser> browser,
                          const std::vector<CefString>& icon_urls) override;
  // Invoked by the favicon download callback (CEF UI thread) with the decoded
  // image; forwards PNG bytes to the delegate, or an empty result on failure.
  void DeliverFaviconImage(const CefString& image_url, CefRefPtr<CefImage> image);
  // Captures the media agent's `__MORI_MEDIA__` console channel.
  bool OnConsoleMessage(CefRefPtr<CefBrowser> browser,
                        cef_log_severity_t level,
                        const CefString& message,
                        const CefString& source,
                        int line) override;

  // CefDownloadHandler — auto-save to ~/Downloads and broadcast progress.
  bool OnBeforeDownload(CefRefPtr<CefBrowser> browser,
                        CefRefPtr<CefDownloadItem> download_item,
                        const CefString& suggested_name,
                        CefRefPtr<CefBeforeDownloadCallback> callback) override;
  void OnDownloadUpdated(CefRefPtr<CefBrowser> browser,
                         CefRefPtr<CefDownloadItem> download_item,
                         CefRefPtr<CefDownloadItemCallback> callback) override;

  // CefContextMenuHandler — lets Mori surface extension-owned menu items in
  // the browser's native right-click menu.
  void OnBeforeContextMenu(CefRefPtr<CefBrowser> browser,
                           CefRefPtr<CefFrame> frame,
                           CefRefPtr<CefContextMenuParams> params,
                           CefRefPtr<CefMenuModel> model) override;
  bool OnContextMenuCommand(CefRefPtr<CefBrowser> browser,
                            CefRefPtr<CefFrame> frame,
                            CefRefPtr<CefContextMenuParams> params,
                            int command_id,
                            EventFlags event_flags) override;

  // CefJSDialogHandler — bridge alert()/confirm()/prompt() to native NSAlert.
  bool OnJSDialog(CefRefPtr<CefBrowser> browser,
                  const CefString& origin_url,
                  JSDialogType dialog_type,
                  const CefString& message_text,
                  const CefString& default_prompt_text,
                  CefRefPtr<CefJSDialogCallback> callback,
                  bool& suppress_message) override;
  bool OnBeforeUnloadDialog(CefRefPtr<CefBrowser> browser,
                            const CefString& message_text,
                            bool is_reload,
                            CefRefPtr<CefJSDialogCallback> callback) override;

  // CefFindHandler
  void OnFindResult(CefRefPtr<CefBrowser> browser,
                    int identifier,
                    int count,
                    const CefRect& selectionRect,
                    int activeMatchOrdinal,
                    bool finalUpdate) override;

  // CefRequestHandler / CefResourceRequestHandler
  CefRefPtr<CefResourceRequestHandler> GetResourceRequestHandler(
      CefRefPtr<CefBrowser> browser,
      CefRefPtr<CefFrame> frame,
      CefRefPtr<CefRequest> request,
      bool is_navigation,
      bool is_download,
      const CefString& request_initiator,
      bool& disable_default_handling) override;
  CefRefPtr<CefResourceHandler> GetResourceHandler(
      CefRefPtr<CefBrowser> browser,
      CefRefPtr<CefFrame> frame,
      CefRefPtr<CefRequest> request) override;
  bool GetAuthCredentials(CefRefPtr<CefBrowser> browser,
                          const CefString& origin_url,
                          bool isProxy,
                          const CefString& host,
                          int port,
                          const CefString& realm,
                          const CefString& scheme,
                          CefRefPtr<CefAuthCallback> callback) override;
  ReturnValue OnBeforeResourceLoad(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefRefPtr<CefRequest> request,
                                   CefRefPtr<CefCallback> callback) override;
  bool OnResourceResponse(CefRefPtr<CefBrowser> browser,
                          CefRefPtr<CefFrame> frame,
                          CefRefPtr<CefRequest> request,
                          CefRefPtr<CefResponse> response) override;
  void OnResourceLoadComplete(CefRefPtr<CefBrowser> browser,
                              CefRefPtr<CefFrame> frame,
                              CefRefPtr<CefRequest> request,
                              CefRefPtr<CefResponse> response,
                              URLRequestStatus status,
                              int64_t received_content_length) override;

 private:
  BrowserClientDelegate* delegate_;  // not owned; cleared via DetachDelegate.
  std::atomic<int> extension_tab_id_{-1};

  IMPLEMENT_REFCOUNTING(BrowserClient);
};

// Global auto-Picture-in-Picture preference, read when injecting the media
// agent into newly loaded frames. Set from the Swift settings layer.
void MoriSetAutoPiPEnabled(bool enabled);
bool MoriAutoPiPEnabled();

// Global built-in ad blocker preference, read by the resource request handler.
void MoriSetAdBlockerEnabled(bool enabled);
bool MoriAdBlockerEnabled();

// Cancel an active Chromium-owned download by id. Returns false when the
// callback is no longer live (already finished, canceled, or unknown).
bool MoriCancelDownload(uint32_t download_id);
