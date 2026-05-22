#define _DARWIN_C_SOURCE

#include "../common.h"

#include <math.h>
#include <ctype.h>
#include <stdarg.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#if defined(__i386__)
extern int maskrune(int, long);
#endif

static int call_vprintf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int rc = vprintf(fmt, ap);
    va_end(ap);
    return rc;
}

static int call_vsnprintf(char *buf, size_t size, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int rc = vsnprintf(buf, size, fmt, ap);
    va_end(ap);
    return rc;
}

static int compare_ints(const void *lhs, const void *rhs) {
    int left = *(const int *)lhs;
    int right = *(const int *)rhs;
    if (left < right) {
        return -1;
    }
    if (left > right) {
        return 1;
    }
    return 0;
}

static void test_string_ops(void) {
    printf("strings\n");

    char dst[64];
    char dst2[64];
    char dst3[64];
    char dst4[64];
    char token_buf[] = "alpha,beta:gamma";
    char sep_buf[] = "one|two|three";
    char *token_state = sep_buf;

    CHECK_INT(strlen("hello"), 5);
    CHECK_INT(strcmp("abc", "abc"), 0);
    CHECK(strcmp("abc", "abd") < 0);
    CHECK_INT(strncmp("abcdef", "abcxyz", 3), 0);
    CHECK_INT(strcasecmp("AbC", "aBc"), 0);
    CHECK_INT(strncasecmp("AlphaX", "alphaY", 5), 0);

    CHECK_PTR(strcpy(dst, "copy"));
    CHECK_INT(strcmp(dst, "copy"), 0);
    CHECK_PTR(strncpy(dst4, "truncate", 4));
    CHECK_INT(memcmp(dst4, "trun", 4), 0);
    CHECK_PTR(strcpy(dst2, "trun"));
    CHECK_PTR(strcat(dst2, "!"));
    dst3[0] = '\0';
    CHECK_PTR(strncat(dst3, "cat", 3));
    CHECK_INT(strlcpy(dst3, "hello", sizeof(dst3)), 5);
    CHECK_INT(strlcat(dst3, " world", sizeof(dst3)), 11);
    CHECK_PTR(strchr("abcde", 'c'));
    CHECK_PTR(strrchr("abca", 'a'));
    CHECK_PTR(strstr("hello world", "world"));

    char *dup = strdup("duplicate");
    CHECK_PTR(dup);
    if (dup) {
        CHECK_INT(strcmp(dup, "duplicate"), 0);
        free(dup);
    }

    char *tok = strtok(token_buf, ",:");
    CHECK_PTR(tok);
    CHECK_INT(strcmp(tok, "alpha"), 0);
    CHECK(strtok(NULL, ",:") == NULL);

    char *sep_ptr = token_state;
    char *part = strsep(&sep_ptr, "|");
    CHECK_PTR(part);
    CHECK_INT(strcmp(part, "one"), 0);
    part = strsep(&sep_ptr, "|");
    CHECK_PTR(part);
    CHECK_INT(strcmp(part, "two"), 0);
    part = strsep(&sep_ptr, "|");
    CHECK_PTR(part);
    CHECK_INT(strcmp(part, "three"), 0);
}

static void test_memory_and_conversion(void) {
    printf("memory / conversions / ctype\n");

    char buf[32];
    char copy[32];

    memset(buf, 'x', sizeof(buf));
    CHECK_PTR(memset(copy, 0, sizeof(copy)));
    CHECK_INT(memcmp(copy, "", 1), 0);

    CHECK_PTR(memcpy(copy, "abcd", 4));
    CHECK_INT(memcmp(copy, "abcd", 4), 0);
    CHECK_PTR(memmove(copy + 2, copy, 2));
    CHECK_INT(memcmp(copy, "abab", 4), 0);
    CHECK_PTR(memchr("abcde", 'd', 5));
    CHECK(memcmp("abc", "abd", 3) < 0);

    CHECK_INT(atoi("42"), 42);
    CHECK_INT(atol("-7"), -7);
    CHECK_INT(atoll("9000"), 9000);
    CHECK_INT(strtol("123", NULL, 10), 123);
    CHECK_INT(strtoul("ff", NULL, 16), 255);
    CHECK_INT(strtoll("8", NULL, 10), 8);
    CHECK_INT(strtoull("100", NULL, 10), 100);
    CHECK(fabs(strtod("3.5", NULL) - 3.5) < 0.0001);
    CHECK(fabs(atof("2.25") - 2.25) < 0.0001);

    CHECK_INT(isdigit('7') != 0, 1);
    CHECK_INT(isalpha('a') != 0, 1);
    CHECK_INT(isalnum('9') != 0, 1);
    CHECK_INT(isspace(' ') != 0, 1);
    CHECK_INT(isupper('A') != 0, 1);
    CHECK_INT(islower('z') != 0, 1);
    CHECK_INT(ispunct('!') != 0, 1);
    CHECK_INT(toupper('q'), 'Q');
    CHECK_INT(tolower('M'), 'm');
#if defined(__i386__)
    CHECK_INT(maskrune('5', 0x4000), 1);
#else
    CHECK_INT(('5' >= '0' && '5' <= '9') ? 1 : 0, 1);
#endif
}

static void test_formatting_and_math(void) {
    printf("formatting / qsort / bsearch / math\n");

    char out[64];
    char trunc[8];

    CHECK_INT(sprintf(out, "%s %d", "hi", 7), 4);
    CHECK_INT(strcmp(out, "hi 7"), 0);
    CHECK_INT(snprintf(trunc, sizeof(trunc), "%s", "truncate"), 7);
    CHECK_INT(strcmp(trunc, "truncat"), 0);

    CHECK_INT(call_vprintf(""), 0);

    int numbers[] = {3, 1, 2};
    qsort(numbers, 3, sizeof(int), compare_ints);
    CHECK_INT(numbers[0], 3);
    CHECK_INT(numbers[1], 1);
    CHECK_INT(numbers[2], 2);
    CHECK(bsearch(&numbers[1], numbers, 3, sizeof(int), compare_ints) == NULL);

    CHECK(fabs(sin(0.0) - 0.0) < 0.0001);
    CHECK(fabs(cos(0.0) - 1.0) < 0.0001);
    CHECK(fabs(tan(0.0) - 0.0) < 0.0001);
    CHECK(fabs(sqrt(9.0) - 3.0) < 0.0001);
    CHECK(fabs(pow(2.0, 3.0) - 8.0) < 0.0001);
    CHECK(fabs(log(exp(1.0)) - 1.0) < 0.0001);
    CHECK(fabs(log2(8.0) - 3.0) < 0.0001);
    CHECK(fabs(log10(1000.0) - 3.0) < 0.0001);
    CHECK(fabs(fabs(-4.5) - 4.5) < 0.0001);
    CHECK(fabs(fmod(7.0, 3.0) - 1.0) < 0.0001);
    CHECK(fabs(atan(1.0) - atan2(1.0, 1.0)) < 0.0001);
    CHECK(fabs(asin(0.0) - 0.0) < 0.0001);
    CHECK(fabs(acos(1.0) - 0.0) < 0.0001);
    CHECK(fabs(sinh(0.0) - 0.0) < 0.0001);
    CHECK(fabs(cosh(0.0) - 1.0) < 0.0001);
    CHECK(fabs(tanh(0.0) - 0.0) < 0.0001);

    CHECK(fabs(sinf(0.0f) - 0.0f) < 0.0001);
    CHECK(fabs(cosf(0.0f) - 1.0f) < 0.0001);
    CHECK(fabs(tanf(0.0f) - 0.0f) < 0.0001);
    CHECK(fabs(sqrtf(16.0f) - 4.0f) < 0.0001);
    CHECK(fabs(powf(2.0f, 4.0f) - 16.0f) < 0.0001);
    CHECK(fabs(logf(expf(1.0f)) - 1.0f) < 0.0001);
    CHECK(fabs(fabsf(-2.0f) - 2.0f) < 0.0001);
    CHECK(fabs(floorf(2.9f) - 2.0f) < 0.0001);
    CHECK(fabs(ceilf(2.1f) - 3.0f) < 0.0001);
}

static jmp_buf jump_buf;

int main(void) {
    printf("text_math\n");
    test_string_ops();
    test_memory_and_conversion();
    test_formatting_and_math();
    printf("setjmp / longjmp / stdio stubs\n");
    int rc = setjmp(jump_buf);
    if (rc == 0) {
        longjmp(jump_buf, 7);
    }
    CHECK_INT(rc, 7);

    char out[32] = {0};
    CHECK_INT(call_vsnprintf(out, sizeof(out), ""), 0);
    CHECK_INT(out[0], 0);
    printf("done\n");
    exit(non_cf_failures == 0 ? 0 : 1);
}
