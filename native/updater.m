#import <AppKit/AppKit.h>
#import <Sparkle/Sparkle.h>

// The controller must survive for the application lifetime. Rust calls these
// entry points on GPUI's main thread after AppKit has started.
static SPUStandardUpdaterController *controller;
static NSString *lastError;

const char *excavator_updater_error(void) {
    return lastError.UTF8String;
}

int excavator_updater_init(void) {
    @autoreleasepool {
        if (![NSThread isMainThread]) {
            lastError = @"The updater must be initialized on the main thread.";
            return 0;
        }
        if (controller != nil) return 1;
        NSBundle *bundle = [NSBundle mainBundle];
        NSString *feed = [bundle objectForInfoDictionaryKey:@"SUFeedURL"];
        NSString *key = [bundle objectForInfoDictionaryKey:@"SUPublicEDKey"];
        NSURL *url = [feed isKindOfClass:NSString.class] ? [NSURL URLWithString:feed] : nil;
        if (![url.scheme.lowercaseString isEqualToString:@"https"] || url.host.length == 0 ||
            url.user.length != 0 || url.password.length != 0 ||
            ![key isKindOfClass:NSString.class] || key.length == 0) {
            lastError = @"This build needs an HTTPS update feed and an update signing key.";
            return 0;
        }
        controller = [[SPUStandardUpdaterController alloc]
            initWithStartingUpdater:NO updaterDelegate:nil userDriverDelegate:nil];
        NSError *error = nil;
        if (![controller.updater startUpdater:&error]) {
            lastError = error.localizedDescription ?: @"The updater could not start.";
            controller = nil;
            return 0;
        }
        lastError = nil;
        return 1;
    }
}

int excavator_updater_check(void) {
    if (!excavator_updater_init()) return 0;
    [controller checkForUpdates:nil];
    return 1;
}
