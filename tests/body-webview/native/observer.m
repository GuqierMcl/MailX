// Test-process instrumentation only, loaded by the synthetic runner. This uses
// public AppKit/WebKit APIs and never changes CSP, sandbox or navigation policy.
#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>

static void record(WKWebView *webview, NSDictionary *value) {
    NSData *json = [NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
    if (!json) return;
    NSString *payload = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    [webview evaluateJavaScript:[NSString stringWithFormat:@"window.__MAILX_BODY_TEST_EVIDENCE?.observations.push(%@)", payload] completionHandler:nil];
}

@interface MailxBodyObserver : NSObject <WKNavigationDelegate>
@property(nonatomic, weak) WKWebView *webview;
@property(nonatomic, strong) id<WKNavigationDelegate> originalDelegate;
@end

@implementation MailxBodyObserver
- (BOOL)respondsToSelector:(SEL)selector {
    return [super respondsToSelector:selector] || [self.originalDelegate respondsToSelector:selector];
}
- (id)forwardingTargetForSelector:(SEL)selector {
    return [self.originalDelegate respondsToSelector:selector] ? self.originalDelegate : [super forwardingTargetForSelector:selector];
}
- (void)webView:(WKWebView *)webview decidePolicyForNavigationAction:(WKNavigationAction *)action decisionHandler:(void (^)(WKNavigationActionPolicy))handler {
    if (!action.targetFrame.isMainFrame && [action.request.URL.scheme isEqualToString:@"mailx-body"] && [action.request.URL.path hasPrefix:@"/document/"]) {
        WKFrameInfo *frame = action.targetFrame;
        NSString *url = action.request.URL.absoluteString;
        // Every probe must return an explicit sandbox error, never a timeout.
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 500 * NSEC_PER_MSEC), dispatch_get_main_queue(), ^{
            NSDictionary *probes = @{
                @"page-script": @"document.documentElement.dataset.mailxNativePageProbe='executed'; true",
                @"page-globals": @"({invoke:typeof window.__TAURI_INTERNALS__?.invoke,tauri:typeof window.__TAURI__})",
                @"page-parent-dom": @"parent.document.body.textContent",
                @"page-parent-state": @"parent.__MAILX_TEST_SECRET",
                @"page-parent-invoke": @"parent.__TAURI_INTERNALS__.invoke('app_get_startup_status',{})",
                @"page-ipc": @"window.__TAURI_INTERNALS__.invoke('app_get_startup_status',{})",
                @"isolated-script": @"document.documentElement.dataset.mailxNativeIsolatedProbe='executed'; true"
            };
            for (NSString *name in probes) {
                WKContentWorld *world = [name hasPrefix:@"isolated"] ? [WKContentWorld worldWithName:@"MailX Synthetic Body Test"] : WKContentWorld.pageWorld;
                [webview evaluateJavaScript:probes[name] inFrame:frame inContentWorld:world completionHandler:^(id result, NSError *error) {
                    record(webview, @{
                        @"kind": @"native-sandbox-probe", @"url": url, @"probe": name,
                        @"executionBlocked": ([error.domain isEqualToString:WKErrorDomain] && error.code == WKErrorJavaScriptExceptionOccurred && [error.userInfo[@"WKJavaScriptExceptionMessage"] isEqualToString:@"Cannot execute JavaScript in this document"]) ? @YES : @NO,
                        @"errorDomain": error.domain ?: @"", @"errorCode": @(error.code),
                        @"errorDescription": error.userInfo[@"WKJavaScriptExceptionMessage"] ?: error.localizedDescription ?: @"",
                        @"returnedValue": result ?: NSNull.null,
                        @"globalsNotObservable": ([name isEqualToString:@"page-globals"] && error != nil) ? @YES : @NO
                    });
                }];
            }
        });
    }
    // The production delegate remains the sole navigation-policy authority.
    if ([self.originalDelegate respondsToSelector:_cmd]) [self.originalDelegate webView:webview decidePolicyForNavigationAction:action decisionHandler:handler];
    else handler(WKNavigationActionPolicyAllow);
}
@end

static WKWebView *findWebView(NSView *view) {
    if ([view isKindOfClass:WKWebView.class]) return (WKWebView *)view;
    for (NSView *child in view.subviews) {
        WKWebView *found = findWebView(child);
        if (found) return found;
    }
    return nil;
}

static void nativeClick(WKWebView *webview, NSDictionary *target) {
    if (![target[@"x"] isKindOfClass:NSNumber.class] || ![target[@"y"] isKindOfClass:NSNumber.class]) return;
    CGFloat x = [target[@"x"] doubleValue], y = [target[@"y"] doubleValue];
    if (x < 0 || y < 0 || x >= webview.bounds.size.width || y >= webview.bounds.size.height) return;
    NSPoint local = NSMakePoint(x, webview.isFlipped ? y : webview.bounds.size.height - y);
    NSPoint point = [webview convertPoint:local toView:nil];
    NSWindow *window = webview.window;
    [window makeKeyAndOrderFront:nil];
    for (NSNumber *type in @[@(NSEventTypeLeftMouseDown), @(NSEventTypeLeftMouseUp)]) {
        NSEvent *event = [NSEvent mouseEventWithType:type.unsignedIntegerValue location:point modifierFlags:0 timestamp:NSProcessInfo.processInfo.systemUptime windowNumber:window.windowNumber context:nil eventNumber:0 clickCount:1 pressure:1.0];
        [NSApplication.sharedApplication sendEvent:event];
    }
    [webview evaluateJavaScript:@"window.__MAILX_NATIVE_CLICK_DELIVERED=true" completionHandler:nil];
}

__attribute__((constructor)) static void startObserver(void) {
    NSDictionary *environment = NSProcessInfo.processInfo.environment;
    NSString *root = environment[@"MAILX_BODY_WEBVIEW_ROOT"];
    NSString *nonce = environment[@"MAILX_BODY_WEBVIEW_NONCE"];
    if (!root.isAbsolutePath || nonce.length != 64 ||
        ![NSFileManager.defaultManager fileExistsAtPath:[root stringByAppendingPathComponent:@".mailx-synthetic-webview-test"]]) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        __block WKWebView *installed = nil;
        __block MailxBodyObserver *observer = nil;
        [NSTimer scheduledTimerWithTimeInterval:0.05 repeats:YES block:^(NSTimer *timer) {
            if (installed) {
                // Test commands come only from the trusted main document; body
                // frames receive no native handler or invocation bridge.
                [installed evaluateJavaScript:@"(() => {if(!window.__MAILX_BODY_TEST)return null;window.__MAILX_NATIVE_OBSERVER_READY=true;const v=window.__MAILX_NATIVE_CLICK_REQUEST;delete window.__MAILX_NATIVE_CLICK_REQUEST;return v??null})()" completionHandler:^(id result, NSError *error) {
                    if (!error && [result isKindOfClass:NSDictionary.class]) nativeClick(installed, result);
                }];
                return;
            }
            for (NSWindow *window in NSApplication.sharedApplication.windows) {
                WKWebView *webview = findWebView(window.contentView);
                if (!webview) continue;
                observer = [MailxBodyObserver new];
                observer.webview = webview;
                observer.originalDelegate = webview.navigationDelegate;
                webview.navigationDelegate = observer;
                installed = webview;
                return;
            }
        }];
    });
}
