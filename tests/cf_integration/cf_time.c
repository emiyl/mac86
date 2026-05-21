#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>

#define PASS "PASS"
#define FAIL "FAIL"
#define CHECK(expr) printf("  %-40s %s\n", #expr, (expr) ? PASS : FAIL)

int main(void) {
    printf("CFAbsoluteTimeGetCurrent\n");
    CFAbsoluteTime t = CFAbsoluteTimeGetCurrent();
    printf("  time = %f\n", t);
    CHECK(t > 0.0);

    printf("done\n");
    return 0;
}
