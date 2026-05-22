#ifndef COMMON_H
#define COMMON_H

#include <stdio.h>

static int non_cf_failures = 0;

#define CHECK(expr) do { \
    int non_cf_ok = (expr) ? 1 : 0; \
    printf("  %-44s %s\n", #expr, non_cf_ok ? "PASS" : "FAIL"); \
    if (!non_cf_ok) { \
        non_cf_failures++; \
    } \
} while (0)

#define CHECK_INT(expr, expected) do { \
    long long non_cf_actual = (long long)(expr); \
    long long non_cf_expected = (long long)(expected); \
    printf("  %-44s %s\n", #expr, (non_cf_actual == non_cf_expected) ? "PASS" : "FAIL"); \
    if (non_cf_actual != non_cf_expected) { \
        non_cf_failures++; \
    } \
} while (0)

#define CHECK_PTR(expr) do { \
    void *non_cf_ptr = (void *)(expr); \
    printf("  %-44s %s\n", #expr, non_cf_ptr ? "PASS" : "FAIL"); \
    if (!non_cf_ptr) { \
        non_cf_failures++; \
    } \
} while (0)

#endif
