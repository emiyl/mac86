#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>

#define PASS "PASS"
#define FAIL "FAIL"
#define CHECK(expr) printf("  %-40s %s\n", #expr, (expr) ? PASS : FAIL)

/* ---------- CFArrayApplyFunction callback ---------- */

static int g_apply_count;
static int g_apply_sum;

static void count_applier(const void *value, void *context) {
    int v = 0;
    CFNumberGetValue((CFNumberRef)value, kCFNumberIntType, &v);
    int *sum = (int *)context;
    *sum += v;
    g_apply_count++;
}

static CFNumberRef cfnum(int n) {
    return CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &n);
}

static int cfnum_val(CFNumberRef n) {
    int v = 0;
    CFNumberGetValue(n, kCFNumberIntType, &v);
    return v;
}

/* --------------------------------------------------- */

int main(void) {
    /* 1. CFAbsoluteTimeGetCurrent ---------------------------------------- */
    printf("CFAbsoluteTimeGetCurrent\n");
    CFAbsoluteTime t = CFAbsoluteTimeGetCurrent();
    CHECK(t > 0.0);

    /* 2. CFArrayCreateMutable / AppendValue / GetCount / GetValueAtIndex --- */
    printf("CFArrayCreateMutable / AppendValue / GetCount / GetValueAtIndex\n");
    CFMutableArrayRef arr = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    CFArrayAppendValue(arr, cfnum(10));
    CFArrayAppendValue(arr, cfnum(20));
    CFArrayAppendValue(arr, cfnum(30));
    CHECK(CFArrayGetCount(arr) == 3);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(arr, 0)) == 10);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(arr, 1)) == 20);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(arr, 2)) == 30);

    /* 3. CFArrayContainsValue -------------------------------------------- */
    printf("CFArrayContainsValue\n");
    CFRange full = CFRangeMake(0, CFArrayGetCount(arr));
    CFNumberRef n20 = cfnum(20);
    CFNumberRef n99 = cfnum(99);
    CHECK( CFArrayContainsValue(arr, full, n20));
    CHECK(!CFArrayContainsValue(arr, full, n99));
    CFRelease(n20); CFRelease(n99);

    /* 4. CFArrayInsertValueAtIndex / RemoveValueAtIndex ------------------- */
    printf("CFArrayInsertValueAtIndex / RemoveValueAtIndex\n");
    CFArrayInsertValueAtIndex(arr, 1, cfnum(15));
    CHECK(CFArrayGetCount(arr) == 4);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(arr, 1)) == 15);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(arr, 2)) == 20);
    CFArrayRemoveValueAtIndex(arr, 1);
    CHECK(CFArrayGetCount(arr) == 3);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(arr, 1)) == 20);

    /* 5. CFArrayApplyFunction -------------------------------------------- */
    printf("CFArrayApplyFunction\n");
    g_apply_count = 0;
    g_apply_sum   = 0;
    CFArrayApplyFunction(arr, CFRangeMake(0, CFArrayGetCount(arr)),
                         count_applier, &g_apply_sum);
    CHECK(g_apply_count == 3);
    CHECK(g_apply_sum   == 60); /* 10 + 20 + 30 */

    /* 6. CFArrayCreateCopy / CFArrayCreateMutableCopy -------------------- */
    printf("CFArrayCreateCopy / CFArrayCreateMutableCopy\n");
    CFArrayRef copy = CFArrayCreateCopy(kCFAllocatorDefault, arr);
    CHECK(CFArrayGetCount(copy) == 3);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(copy, 0)) == 10);

    CFMutableArrayRef mcopy = CFArrayCreateMutableCopy(kCFAllocatorDefault, 0, arr);
    CFArrayAppendValue(mcopy, cfnum(40));
    CHECK(CFArrayGetCount(mcopy) == 4);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(mcopy, 3)) == 40);
    /* original untouched */
    CHECK(CFArrayGetCount(arr) == 3);

    /* 7. CFArrayAppendArray ----------------------------------------------- */
    printf("CFArrayAppendArray\n");
    CFMutableArrayRef extra = CFArrayCreateMutable(kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    CFArrayAppendValue(extra, cfnum(100));
    CFArrayAppendValue(extra, cfnum(200));
    CFArrayAppendArray(arr, extra, CFRangeMake(0, CFArrayGetCount(extra)));
    CHECK(CFArrayGetCount(arr) == 5);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(arr, 3)) == 100);
    CHECK(cfnum_val(CFArrayGetValueAtIndex(arr, 4)) == 200);

    /* 8. CFArrayRemoveAllValues ------------------------------------------ */
    printf("CFArrayRemoveAllValues\n");
    CFArrayRemoveAllValues(arr);
    CHECK(CFArrayGetCount(arr) == 0);

    /* 9. CFArrayGetTypeID ------------------------------------------------- */
    printf("CFArrayGetTypeID\n");
    CHECK(CFArrayGetTypeID() > 0);

    printf("done\n");
    return 0;
}
