// Swift-facing entry point for privacy/data operations that need to talk to the
// global CEF context (cookies, HTTP cache). Free of any CEF/C++ types so it can
// be imported through the bridging header.
#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@interface MoriPrivacy : NSObject

/// Delete every cookie in the global jar, then flush the change to disk.
+ (void)clearCookies;

/// Clear the global HTTP cache.
+ (void)clearCache;

/// Force the cookie store to write to disk. Cheap; safe to call on quit so
/// session/persistent cookies are never lost on an abrupt termination.
+ (void)flushCookies;

@end

NS_ASSUME_NONNULL_END
