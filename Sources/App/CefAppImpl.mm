#include "CefAppImpl.h"

#import <Foundation/Foundation.h>

#include "include/cef_parser.h"
#include "include/cef_scheme.h"
#include "include/cef_stream.h"
#include "include/wrapper/cef_stream_resource_handler.h"
#include "include/wrapper/cef_helpers.h"

#include "../Shared/MoriSchemes.h"
#import "ExtensionRuntimeBridge.h"

#include <algorithm>
#include <cstring>
#include <string>
#include <utility>

namespace {

NSString* const kMoriExtensionsCatalogKey = @"mori.extensions";
NSString* const kMoriExtensionBackgroundPath = @"/__mori_background__.html";
NSString* const kMoriExtensionCatalogEnvironmentKey =
    @"MORI_EXTENSION_CATALOG_JSON";

NSString* NSStringFromCef(const CefString& value) {
  std::string utf8 = value.ToString();
  return [NSString stringWithUTF8String:utf8.c_str()] ?: @"";
}

NSString* EnabledExtensionRootForID(NSString* extensionID) {
  if (extensionID.length == 0) {
    return nil;
  }

  NSData* data = nil;
  NSString* environmentCatalog =
      NSProcessInfo.processInfo.environment[kMoriExtensionCatalogEnvironmentKey];
  if (environmentCatalog.length > 0) {
    data = [environmentCatalog dataUsingEncoding:NSUTF8StringEncoding];
  } else {
    data = [NSUserDefaults.standardUserDefaults
        dataForKey:kMoriExtensionsCatalogKey];
  }
  if (![data isKindOfClass:NSData.class]) return nil;

  NSError* error = nil;
  id decoded = [NSJSONSerialization JSONObjectWithData:data options:0 error:&error];
  if (![decoded isKindOfClass:NSArray.class]) {
    return nil;
  }

  NSFileManager* fm = NSFileManager.defaultManager;
  for (id item in (NSArray*)decoded) {
    if (![item isKindOfClass:NSDictionary.class]) {
      continue;
    }
    NSDictionary* ext = (NSDictionary*)item;
    NSString* identifier = [ext[@"id"] isKindOfClass:NSString.class]
        ? ext[@"id"]
        : nil;
    NSString* path = [ext[@"path"] isKindOfClass:NSString.class]
        ? ext[@"path"]
        : nil;
    NSNumber* enabled = [ext[@"enabled"] isKindOfClass:NSNumber.class]
        ? ext[@"enabled"]
        : nil;
    if (!identifier || !path || !enabled.boolValue) {
      continue;
    }
    if ([identifier caseInsensitiveCompare:extensionID] != NSOrderedSame) {
      continue;
    }

    NSString* standardized = path.stringByStandardizingPath;
    BOOL isDirectory = NO;
    if ([fm fileExistsAtPath:standardized isDirectory:&isDirectory] &&
        isDirectory) {
      return standardized;
    }
  }
  return nil;
}

NSString* SafeExtensionFilePath(NSString* root, NSString* requestPath) {
  if (root.length == 0) {
    return nil;
  }

  NSString* relative = requestPath ?: @"";
  while ([relative hasPrefix:@"/"]) {
    relative = [relative substringFromIndex:1];
  }
  relative = relative.stringByRemovingPercentEncoding ?: relative;
  if (relative.length == 0) {
    return nil;
  }

  NSString* rootResolved =
      root.stringByStandardizingPath.stringByResolvingSymlinksInPath;
  NSString* candidate = [[rootResolved stringByAppendingPathComponent:relative]
      stringByStandardizingPath].stringByResolvingSymlinksInPath;
  NSString* requiredPrefix = [rootResolved stringByAppendingString:@"/"];
  if (![candidate isEqualToString:rootResolved] &&
      ![candidate hasPrefix:requiredPrefix]) {
    return nil;
  }

  BOOL isDirectory = NO;
  if (![NSFileManager.defaultManager fileExistsAtPath:candidate
                                          isDirectory:&isDirectory] ||
      isDirectory) {
    return nil;
  }
  return candidate;
}

CefString MimeTypeForPath(NSString* filePath) {
  NSString* ext = filePath.pathExtension.lowercaseString;
  if (ext.length == 0) {
    return CefString("application/octet-stream");
  }
  CefString mime = CefGetMimeType(CefString(ext.UTF8String));
  if (mime.empty()) {
    return CefString("application/octet-stream");
  }
  return mime;
}

CefRefPtr<CefResourceHandler> StaticResponse(int status_code,
                                             const char* status_text,
                                             const char* mime_type,
                                             const char* body) {
  CefResponse::HeaderMap headers;
  headers.insert(std::make_pair(CefString("Cache-Control"),
                                CefString("no-store")));
  auto stream =
      CefStreamReader::CreateForData(const_cast<char*>(body), strlen(body));
  return new CefStreamResourceHandler(status_code, CefString(status_text),
                                      CefString(mime_type), headers, stream);
}

class StringReadHandler : public CefReadHandler {
 public:
  explicit StringReadHandler(std::string body) : body_(std::move(body)) {}

  size_t Read(void* ptr, size_t size, size_t n) override {
    if (size == 0 || n == 0 || offset_ >= body_.size()) return 0;
    size_t requested = size * n;
    size_t available = body_.size() - offset_;
    size_t bytes = std::min(requested, available);
    memcpy(ptr, body_.data() + offset_, bytes);
    offset_ += bytes;
    return bytes / size;
  }

  int Seek(int64_t offset, int whence) override {
    int64_t base = 0;
    if (whence == SEEK_CUR) {
      base = static_cast<int64_t>(offset_);
    } else if (whence == SEEK_END) {
      base = static_cast<int64_t>(body_.size());
    } else if (whence != SEEK_SET) {
      return -1;
    }
    int64_t next = base + offset;
    if (next < 0 || next > static_cast<int64_t>(body_.size())) return -1;
    offset_ = static_cast<size_t>(next);
    return 0;
  }

  int64_t Tell() override { return static_cast<int64_t>(offset_); }
  int Eof() override { return offset_ >= body_.size(); }
  bool MayBlock() override { return false; }

 private:
  std::string body_;
  size_t offset_ = 0;

  IMPLEMENT_REFCOUNTING(StringReadHandler);
};

NSDictionary* ManifestForRoot(NSString* root) {
  if (root.length == 0) return nil;
  NSString* path = [root stringByAppendingPathComponent:@"manifest.json"];
  NSData* data = [NSData dataWithContentsOfFile:path];
  if (!data) return nil;
  id json = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
  return [json isKindOfClass:NSDictionary.class] ? (NSDictionary*)json : nil;
}

NSString* NormalizedExtensionResourcePath(NSString* path) {
  NSString* out = path ?: @"";
  while ([out hasPrefix:@"/"]) {
    out = [out substringFromIndex:1];
  }
  NSString* decoded = [out stringByRemovingPercentEncoding];
  return decoded.length ? decoded : out;
}

bool WildcardMatches(NSString* pattern, NSString* value) {
  if (![pattern isKindOfClass:NSString.class] ||
      ![value isKindOfClass:NSString.class]) {
    return false;
  }
  NSMutableString* regex = [NSMutableString stringWithString:@"^"];
  NSCharacterSet* special =
      [NSCharacterSet characterSetWithCharactersInString:@"\\.[]{}()+-^$?|"];
  for (NSUInteger i = 0; i < pattern.length; i++) {
    unichar ch = [pattern characterAtIndex:i];
    if (ch == '*') {
      [regex appendString:@".*"];
    } else {
      if ([special characterIsMember:ch]) [regex appendString:@"\\"];
      [regex appendFormat:@"%C", ch];
    }
  }
  [regex appendString:@"$"];
  return [value rangeOfString:regex
                      options:NSRegularExpressionSearch | NSCaseInsensitiveSearch]
             .location != NSNotFound;
}

NSString* SecurityOriginForURL(NSURL* url) {
  if (!url.scheme || url.scheme.length == 0) return @"";
  NSString* scheme = url.scheme.lowercaseString;
  if ([scheme isEqualToString:@"file"]) return @"file://";
  if (!url.host || url.host.length == 0) return scheme;
  NSNumber* port = url.port;
  if (port) {
    return [NSString stringWithFormat:@"%@://%@:%@", scheme,
                                      url.host.lowercaseString, port];
  }
  return [NSString stringWithFormat:@"%@://%@", scheme,
                                    url.host.lowercaseString];
}

bool WebAccessibleMatchAllowsOrigin(NSString* pattern, NSURL* initiator) {
  if (![pattern isKindOfClass:NSString.class] || !initiator) return false;
  if ([pattern isEqualToString:@"<all_urls>"]) return true;
  NSString* origin = SecurityOriginForURL(initiator);
  NSString* absolute = initiator.absoluteString ?: origin;
  return WildcardMatches(pattern, origin) || WildcardMatches(pattern, absolute);
}

bool WebAccessibleResourcesAllow(NSDictionary* manifest,
                                 NSString* resourcePath,
                                 NSURL* initiator) {
  NSArray* entries =
      [manifest[@"web_accessible_resources"] isKindOfClass:NSArray.class]
          ? manifest[@"web_accessible_resources"]
          : nil;
  if (entries.count == 0) return false;

  for (id entry in entries) {
    if ([entry isKindOfClass:NSString.class]) {
      if (WildcardMatches((NSString*)entry, resourcePath)) return true;
      continue;
    }
    if (![entry isKindOfClass:NSDictionary.class]) continue;
    NSDictionary* dict = (NSDictionary*)entry;
    NSArray* resources = [dict[@"resources"] isKindOfClass:NSArray.class]
        ? dict[@"resources"]
        : @[];
    bool resourceAllowed = false;
    for (id raw in resources) {
      if ([raw isKindOfClass:NSString.class] &&
          WildcardMatches((NSString*)raw, resourcePath)) {
        resourceAllowed = true;
        break;
      }
    }
    if (!resourceAllowed) continue;

    NSArray* matches = [dict[@"matches"] isKindOfClass:NSArray.class]
        ? dict[@"matches"]
        : @[];
    if (matches.count == 0) continue;
    for (id raw in matches) {
      if ([raw isKindOfClass:NSString.class] &&
          WebAccessibleMatchAllowsOrigin((NSString*)raw, initiator)) {
        return true;
      }
    }
  }
  return false;
}

bool IsExtensionInternalRequest(NSString* extensionID,
                                CefRefPtr<CefFrame> frame,
                                CefRefPtr<CefRequest> request) {
  NSString* frameURL = frame ? NSStringFromCef(frame->GetURL()) : @"";
  NSURL* frameNSURL = [NSURL URLWithString:frameURL];
  if ([frameNSURL.scheme isEqualToString:@(mori::kExtensionScheme)] &&
      (!frameNSURL.host ||
       [frameNSURL.host.lowercaseString isEqualToString:extensionID.lowercaseString])) {
    return true;
  }

  NSString* referrer = request ? NSStringFromCef(request->GetReferrerURL()) : @"";
  NSURL* referrerURL = [NSURL URLWithString:referrer];
  if ([referrerURL.scheme isEqualToString:@(mori::kExtensionScheme)] &&
      (!referrerURL.host ||
       [referrerURL.host.lowercaseString isEqualToString:extensionID.lowercaseString])) {
    return true;
  }

  if (frameURL.length == 0 && referrer.length == 0) {
    return true;
  }

  return false;
}

// A real web origin we can match web_accessible_resources `matches` against.
// about:blank / data: / the extension scheme / schemeless URLs are not usable
// as an initiator: a freshly-created subframe (e.g. Proton Pass's autofill
// dropdown iframe) reports about:blank while it loads, which matches no
// https://*/* pattern and would 403 the resource.
bool IsMeaningfulInitiator(NSURL* url) {
  NSString* scheme = url.scheme.lowercaseString;
  if (scheme.length == 0) return false;
  if ([scheme isEqualToString:@"about"] || [scheme isEqualToString:@"data"] ||
      [scheme isEqualToString:@"blob"] ||
      [scheme isEqualToString:@(mori::kExtensionScheme)]) {
    return false;
  }
  return url.host.length > 0 || [scheme isEqualToString:@"file"];
}

NSURL* InitiatorURLForRequest(CefRefPtr<CefFrame> frame,
                              CefRefPtr<CefRequest> request) {
  NSURL* frameNSURL =
      [NSURL URLWithString:(frame ? NSStringFromCef(frame->GetURL()) : @"")];
  if (IsMeaningfulInitiator(frameNSURL)) return frameNSURL;

  // about:blank subframes inherit their embedder's origin, so the embedding
  // (parent) frame is the real initiator of a resource the page injected.
  for (CefRefPtr<CefFrame> parent = frame ? frame->GetParent() : nullptr;
       parent; parent = parent->GetParent()) {
    NSURL* parentURL = [NSURL URLWithString:NSStringFromCef(parent->GetURL())];
    if (IsMeaningfulInitiator(parentURL)) return parentURL;
  }

  NSURL* referrer =
      [NSURL URLWithString:(request ? NSStringFromCef(request->GetReferrerURL())
                                    : @"")];
  if (IsMeaningfulInitiator(referrer)) return referrer;
  return frameNSURL ?: referrer;
}

NSString* HTMLEscapedAttribute(NSString* raw) {
  NSString* out = raw ?: @"";
  out = [out stringByReplacingOccurrencesOfString:@"&" withString:@"&amp;"];
  out = [out stringByReplacingOccurrencesOfString:@"\"" withString:@"&quot;"];
  out = [out stringByReplacingOccurrencesOfString:@"<" withString:@"&lt;"];
  out = [out stringByReplacingOccurrencesOfString:@">" withString:@"&gt;"];
  return out;
}

NSString* JSSingleQuotedString(NSString* raw) {
  NSString* out = raw ?: @"";
  out = [out stringByReplacingOccurrencesOfString:@"\\" withString:@"\\\\"];
  out = [out stringByReplacingOccurrencesOfString:@"'" withString:@"\\'"];
  out = [out stringByReplacingOccurrencesOfString:@"\n" withString:@"\\n"];
  out = [out stringByReplacingOccurrencesOfString:@"\r" withString:@"\\r"];
  out = [out stringByReplacingOccurrencesOfString:@"\u2028" withString:@"\\u2028"];
  out = [out stringByReplacingOccurrencesOfString:@"\u2029" withString:@"\\u2029"];
  return out;
}

NSString* HTMLScriptEscapedJavaScript(NSString* raw) {
  NSString* out = raw ?: @"";
  // Keep generated JavaScript from accidentally terminating the wrapper <script>
  // if extension-provided manifest text contains an HTML script close marker.
  out = [out stringByReplacingOccurrencesOfString:@"</script"
                                       withString:@"<\\/script"
                                          options:NSCaseInsensitiveSearch
                                            range:NSMakeRange(0, out.length)];
  return out;
}

NSString* MoriHostBrowserVersion() {
  NSString* version = [NSBundle.mainBundle
      objectForInfoDictionaryKey:@"CFBundleShortVersionString"];
  return version.length ? version : @"0.1.0";
}

NSString* ExtensionEarlyRuntimeBootstrap(NSString* extensionID) {
  return [NSString stringWithFormat:
      @"<script>"
       "document.documentElement.dataset.moriExtensionPage='true';"
       "try{"
       "Object.defineProperty(Navigator.prototype,'onLine',{configurable:true,get:function(){return true;}});"
       "try{Object.defineProperty(navigator,'onLine',{configurable:true,get:function(){return true;}});}catch(_e){}"
       "setTimeout(function(){try{window.dispatchEvent(new Event('online'));}catch(_e){}},0);"
       "}catch(e){}"
       "window.__moriNativeConsoleInfo=window.__moriNativeConsoleInfo||"
       "(function(){try{return Function.prototype.bind.call(console.info,console);}catch(e){return function(message){try{console.info(message);}catch(_){}};}})();"
       "globalThis.chrome=globalThis.chrome||{};"
       "chrome.runtime=chrome.runtime||{};"
       "chrome.runtime.id=chrome.runtime.id||'%@';"
       "var __moriHostBrowser={name:'Mori',vendor:'Mori',version:'%@',buildID:''};"
       "function __moriEarlyEvent(){"
       "var listeners=[];"
       "return {addListener:function(fn){if(typeof fn==='function'&&listeners.indexOf(fn)<0)listeners.push(fn);},"
       "removeListener:function(fn){var i=listeners.indexOf(fn);if(i>=0)listeners.splice(i,1);},"
       "hasListener:function(fn){return listeners.indexOf(fn)>=0;},"
       "hasListeners:function(){return listeners.length>0;},"
       "_listeners:listeners,"
       "_fire:function(){var args=arguments;listeners.slice().forEach(function(fn){try{fn.apply(null,args);}catch(e){console.error(e);}});}};"
       "}"
       "chrome.runtime.onMessage=chrome.runtime.onMessage||__moriEarlyEvent();"
       "chrome.runtime.onMessageExternal=chrome.runtime.onMessageExternal||__moriEarlyEvent();"
       "chrome.runtime.onConnect=chrome.runtime.onConnect||__moriEarlyEvent();"
       "chrome.runtime.onInstalled=chrome.runtime.onInstalled||__moriEarlyEvent();"
       "chrome.runtime.onStartup=chrome.runtime.onStartup||__moriEarlyEvent();"
       "chrome.runtime.onUpdateAvailable=chrome.runtime.onUpdateAvailable||__moriEarlyEvent();"
	       "try{"
	       "globalThis.browser=globalThis.browser||globalThis.chrome;"
	       "globalThis.browser.runtime=globalThis.browser.runtime||chrome.runtime;"
	       "globalThis.browser.runtime.id=globalThis.browser.runtime.id||chrome.runtime.id;"
	       "['onMessage','onMessageExternal','onConnect','onInstalled','onStartup','onUpdateAvailable'].forEach(function(name){"
	       "if(!globalThis.browser.runtime[name])globalThis.browser.runtime[name]=chrome.runtime[name];"
	       "});"
	       "globalThis.browser.name=globalThis.browser.name||__moriHostBrowser.name;"
	       "globalThis.browser.version=globalThis.browser.version||__moriHostBrowser.version;"
	       "}catch(e){}"
	       "</script>",
      JSSingleQuotedString(extensionID),
      JSSingleQuotedString(MoriHostBrowserVersion())];
}

// The markup injected into the <head> of every extension page. The early
// bootstrap defines chrome.runtime.id + event stubs synchronously so the page's
// webextension-polyfill doesn't throw; the full runtime that follows fills in
// every chrome.* method, so the page (e.g. a popup's React app) has the whole
// API the moment its own scripts run — no OnLoadStart race.
NSString* ExtensionRuntimeBootstrapMarkup(NSString* extensionID) {
  NSMutableString* markup =
      [ExtensionEarlyRuntimeBootstrap(extensionID) mutableCopy];
  NSString* full = MoriExtensionPageRuntimeJS(extensionID);
  if (full.length > 0) {
    [markup appendFormat:@"<script>%@</script>",
                         HTMLScriptEscapedJavaScript(full)];
  }
  return markup;
}

NSString* HTMLWithEarlyRuntimeBootstrap(NSString* html, NSString* extensionID) {
  NSString* bootstrap = ExtensionRuntimeBootstrapMarkup(extensionID);
  if (html.length == 0) {
    return bootstrap;
  }

  NSMutableString* out = [html mutableCopy];
  NSStringCompareOptions options =
      NSCaseInsensitiveSearch | NSLiteralSearch;
  NSArray<NSString*>* anchors = @[ @"<head", @"<html" ];
  for (NSString* anchor in anchors) {
    NSRange start = [out rangeOfString:anchor options:options];
    if (start.location == NSNotFound) continue;
    NSRange search = NSMakeRange(start.location, out.length - start.location);
    NSRange end = [out rangeOfString:@">" options:0 range:search];
    if (end.location == NSNotFound) continue;
    [out insertString:bootstrap atIndex:NSMaxRange(end)];
    return out;
  }

  NSRange doctype = [out rangeOfString:@"<!doctype" options:options];
  if (doctype.location != NSNotFound) {
    NSRange search = NSMakeRange(doctype.location, out.length - doctype.location);
    NSRange end = [out rangeOfString:@">" options:0 range:search];
    if (end.location != NSNotFound) {
      [out insertString:bootstrap atIndex:NSMaxRange(end)];
      return out;
    }
  }

  [out insertString:bootstrap atIndex:0];
  return out;
}

CefRefPtr<CefResourceHandler> HTMLResponse(NSString* html) {
  CefResponse::HeaderMap headers;
  headers.insert(std::make_pair(CefString("Access-Control-Allow-Origin"),
                                CefString("*")));
  headers.insert(std::make_pair(CefString("Cache-Control"),
                                CefString("no-store")));
  auto stream = CefStreamReader::CreateForHandler(
      new StringReadHandler(std::string([html UTF8String] ?: "")));
  // CefResponse::SetMimeType expects a bare type/subtype, not a Content-Type
  // header value — a "; charset=…" parameter makes Chromium fail to match
  // "text/html" and render the body as plain text. Keep it bare.
  return new CefStreamResourceHandler(200, CefString("OK"),
                                      CefString("text/html"),
                                      headers, stream);
}

CefRefPtr<CefResourceHandler> BackgroundPageResponse(NSString* root,
                                                     NSString* extensionID) {
  NSDictionary* manifest = ManifestForRoot(root);
  NSDictionary* background =
      [manifest[@"background"] isKindOfClass:NSDictionary.class]
          ? manifest[@"background"]
          : nil;
  if (!background) {
    return StaticResponse(404, "Not Found", "text/plain",
                          "Extension background is not declared.");
  }

  NSMutableArray<NSString*>* scripts = [NSMutableArray array];
  NSString* moduleType = nil;
  if ([background[@"service_worker"] isKindOfClass:NSString.class]) {
    [scripts addObject:background[@"service_worker"]];
    if ([background[@"type"] isKindOfClass:NSString.class] &&
        [background[@"type"] isEqualToString:@"module"]) {
      moduleType = @"module";
    }
  }
  if ([background[@"scripts"] isKindOfClass:NSArray.class]) {
    for (id item in (NSArray*)background[@"scripts"]) {
      if ([item isKindOfClass:NSString.class]) {
        [scripts addObject:item];
      }
    }
  }

  if (scripts.count == 0) {
    return StaticResponse(404, "Not Found", "text/plain",
                          "Extension background has no scripts.");
  }

  NSMutableString* html = [NSMutableString stringWithFormat:
      @"<!doctype html><meta charset=\"utf-8\">"
       "<title>Mori Extension Background</title>"
       "<script>document.documentElement.dataset.moriExtensionBackground='true';</script>"
       "%@",
      ExtensionRuntimeBootstrapMarkup(extensionID)];
	  for (NSString* script in scripts) {
	    NSString* relative = script;
	    while ([relative hasPrefix:@"/"]) {
	      relative = [relative substringFromIndex:1];
	    }
	    NSString* attributes = moduleType ? @" type=\"module\"" : @" defer";
	    [html appendFormat:@"<script%@ src=\"%@\"></script>",
	                       attributes, HTMLEscapedAttribute(relative)];
	  }
  [html appendFormat:@"<!-- %@ -->", HTMLEscapedAttribute(extensionID)];
  return HTMLResponse(html);
}

class MoriExtensionSchemeHandlerFactory
    : public CefSchemeHandlerFactory {
 public:
  CefRefPtr<CefResourceHandler> Create(
      CefRefPtr<CefBrowser> browser,
      CefRefPtr<CefFrame> frame,
      const CefString& scheme_name,
      CefRefPtr<CefRequest> request) override {
    @autoreleasepool {
      NSURL* url = [NSURL URLWithString:NSStringFromCef(request->GetURL())];
      if (!url || ![url.scheme isEqualToString:@(mori::kExtensionScheme)]) {
        return StaticResponse(400, "Bad Request", "text/plain",
                              "Bad extension resource URL.");
      }

      NSString* extensionID = url.host ?: @"";
      NSString* root = EnabledExtensionRootForID(extensionID);
      if ([url.path isEqualToString:kMoriExtensionBackgroundPath]) {
        return BackgroundPageResponse(root, extensionID);
      }
      NSString* resourcePath = NormalizedExtensionResourcePath(url.path);
      if (!IsExtensionInternalRequest(extensionID, frame, request)) {
        NSDictionary* manifest = ManifestForRoot(root);
        NSURL* initiator = InitiatorURLForRequest(frame, request);
        if (!WebAccessibleResourcesAllow(manifest, resourcePath, initiator)) {
          return StaticResponse(403, "Forbidden", "text/plain",
                                "Extension resource is not web accessible.");
        }
      }
      NSString* filePath = SafeExtensionFilePath(root, url.path);
      if (!filePath) {
        return StaticResponse(404, "Not Found", "text/plain",
                              "Extension resource not found.");
      }

      NSString* ext = filePath.pathExtension.lowercaseString;
      if ([ext isEqualToString:@"html"] || [ext isEqualToString:@"htm"]) {
        NSString* html = [NSString stringWithContentsOfFile:filePath
                                                   encoding:NSUTF8StringEncoding
                                                      error:nil];
        if (html) {
          return HTMLResponse(HTMLWithEarlyRuntimeBootstrap(html, extensionID));
        }
      }

      CefRefPtr<CefStreamReader> stream =
          CefStreamReader::CreateForFile(CefString(filePath.UTF8String));
      if (!stream) {
        return StaticResponse(404, "Not Found", "text/plain",
                              "Extension resource not found.");
      }

      CefResponse::HeaderMap headers;
      headers.insert(std::make_pair(CefString("Access-Control-Allow-Origin"),
                                    CefString("*")));
      headers.insert(std::make_pair(CefString("Cache-Control"),
                                    CefString("no-cache")));
      return new CefStreamResourceHandler(200, CefString("OK"),
                                          MimeTypeForPath(filePath), headers,
                                          stream);
    }
  }

 private:
  IMPLEMENT_REFCOUNTING(MoriExtensionSchemeHandlerFactory);
};

// Serves Mori's internal pages (currently just the new-tab / home page) from
// the bundled `home.html`. The page is self-contained, so every internal URL
// resolves to the same document.
class MoriInternalSchemeHandlerFactory : public CefSchemeHandlerFactory {
 public:
  CefRefPtr<CefResourceHandler> Create(
      CefRefPtr<CefBrowser> browser,
      CefRefPtr<CefFrame> frame,
      const CefString& scheme_name,
      CefRefPtr<CefRequest> request) override {
    @autoreleasepool {
      NSURL* url = [NSBundle.mainBundle URLForResource:@"home"
                                         withExtension:@"html"];
      NSString* html = url
          ? [NSString stringWithContentsOfURL:url
                                     encoding:NSUTF8StringEncoding
                                        error:nil]
          : nil;
      if (html.length == 0) {
        return StaticResponse(404, "Not Found", "text/plain",
                              "Internal page not found.");
      }
      return HTMLResponse(html);
    }
  }

 private:
  IMPLEMENT_REFCOUNTING(MoriInternalSchemeHandlerFactory);
};

}  // namespace

CefAppImpl::CefAppImpl() = default;

void CefAppImpl::OnBeforeCommandLineProcessing(
    const CefString& process_type,
    CefRefPtr<CefCommandLine> command_line) {
  // Browser-process only tweaks.
  if (process_type.empty()) {
    // Smooth scrolling and modern web features on by default.
    if (!command_line->HasSwitch("disable-smooth-scrolling")) {
      command_line->AppendSwitch("enable-smooth-scrolling");
    }
    // Allow autoplay so embedded media behaves like a normal browser.
    command_line->AppendSwitchWithValue("autoplay-policy",
                                        "no-user-gesture-required");

    // Clone-friendly local builds are ad-hoc signed, which changes Chromium's
    // Keychain ACL and can trigger Safe Storage prompts every rebuild. Default
    // to Chromium's basic store unless a distributor explicitly opts into the
    // real macOS Keychain for a stable Developer ID build.
    NSString* realKeychain =
        NSProcessInfo.processInfo.environment[@"MORI_USE_REAL_KEYCHAIN"];
    if (![realKeychain isEqualToString:@"1"] &&
        ![realKeychain.lowercaseString isEqualToString:@"true"]) {
      command_line->AppendSwitch("use-mock-keychain");
      command_line->AppendSwitchWithValue("password-store", "basic");
    }

    // Enable Chromium's native automatic Picture-in-Picture. With these,
    // setting `video.autoPictureInPicture = true` makes the engine pop the
    // video out when the tab is hidden — no user-gesture restriction (which
    // blocks a manual requestPictureInPicture() from visibilitychange). The
    // user-facing toggle gates whether our agent sets that attribute.
    command_line->AppendSwitchWithValue(
        "enable-features",
        "AutoPictureInPictureForVideoPlayback,MediaSessionEnterPictureInPicture");
    command_line->AppendSwitchWithValue("enable-blink-features",
                                        "AutoPictureInPicture");
  }
}

void CefAppImpl::OnRegisterCustomSchemes(
    CefRawPtr<CefSchemeRegistrar> registrar) {
  mori::RegisterCustomSchemes(registrar);
}

void CefAppImpl::OnContextInitialized() {
  CEF_REQUIRE_UI_THREAD();
  CefRegisterSchemeHandlerFactory(
      mori::kExtensionScheme, CefString(),
      new MoriExtensionSchemeHandlerFactory());
  CefRegisterSchemeHandlerFactory(
      mori::kInternalScheme, CefString(),
      new MoriInternalSchemeHandlerFactory());
  // The UI layer creates browsers on demand once the window is up.
}
