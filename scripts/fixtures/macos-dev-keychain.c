// Synthetic Keychain probe: never use an actual account password here.
#include <CoreFoundation/CoreFoundation.h>
#include <Security/Security.h>
#include <stdio.h>
#include <string.h>

int main(int argc, char **argv) {
    if (argc != 3) return 1;
    if (SecKeychainSetUserInteractionAllowed(false) != errSecSuccess) return 4;

    SecKeychainRef keychain = NULL;
    if (SecKeychainOpen(argv[1], &keychain) != errSecSuccess) return 2;

    const char *service = "com.mailxapp.tests.development-signing";
    const char *account = "synthetic-probe";
    const char *expected = "synthetic-value-only";
    UInt32 length = 0;
    void *password = NULL;
    OSStatus status = SecKeychainFindGenericPassword(keychain,
        (UInt32)strlen(service), service, (UInt32)strlen(account), account,
        &length, &password, NULL);
    if (status == errSecItemNotFound && strcmp(argv[2], "create") == 0) {
        status = SecKeychainAddGenericPassword(keychain,
            (UInt32)strlen(service), service, (UInt32)strlen(account), account,
            (UInt32)strlen(expected), expected, NULL);
    } else if (status == errSecSuccess) {
        if (length != strlen(expected) || memcmp(password, expected, length) != 0) {
            status = errSecDecode;
        }
        SecKeychainItemFreeContent(NULL, password);
    }
    CFRelease(keychain);
    printf("build=%d status=%d\n", MAILX_PROBE_VERSION, (int)status);
    return status == errSecSuccess ? 0 : 3;
}
