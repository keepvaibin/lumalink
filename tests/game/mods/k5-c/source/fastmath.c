/* K5 conformance: C called by the script (text and bytes as pointer and length), and C calling the script's
   `export c fn`s through lm_exports.h. No C library (abi 1.1). */
#include "fastmath.h"
#include "lm_exports.h"

float k5_mix(float a, float b) {
    return a * 2.0f + b;
}

int64_t k5_len(const char* p, uint64_t n) {
    (void)p;
    return (int64_t)n;
}

int32_t k5_sum_bytes(const uint8_t* p, uint64_t n) {
    int32_t s = 0;
    for (uint64_t i = 0; i < n; i++) s += p[i];
    return s;
}

bool k5_odd(int32_t x) {
    return (x & 1) != 0;
}

int32_t k5_from_c(void) {
    return ball_count() * 100 + name_len("abc", 3) * 10 + (is_big(101) ? 1 : 0);
}
