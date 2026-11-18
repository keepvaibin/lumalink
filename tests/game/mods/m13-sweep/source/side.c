/* M13 conformance: see side.h. */
#include "side.h"

int32_t m13_total(const int32_t* xs, uint64_t n) {
    int32_t s = 0;
    for (uint64_t i = 0; i < n; i++) s += xs[i];
    return s;
}

int32_t m13_lit(const bool* xs, uint64_t n) {
    int32_t s = 0;
    for (uint64_t i = 0; i < n; i++) s += xs[i] ? 1 : 0;
    return s;
}

float m13_spots(const lm_vec3* xs, uint64_t n) {
    float s = 0.0f;
    for (uint64_t i = 0; i < n; i++) s += xs[i].x + xs[i].y;
    return s;
}

void m13_grow(int32_t* n, bool* lamp, lm_vec3* at) {
    *n += 1;
    *lamp = !*lamp;
    at->x *= 10.0f;
}

/* The export with a list, bytes, two places and a text; then with a null place, which runs nothing and gives 0. */
int32_t m13_run_c(void) {
    int32_t xs[3] = {5, 6, 7};
    uint8_t b[2] = {1, 2};
    bool fs[3] = {1, 1, 0};
    int32_t out = 0;
    bool flag = 0;
    int32_t s = m13_back(xs, 3, b, 2, &out, &flag, "abc", 3);
    int32_t none = m13_back(xs, 3, b, 2, 0, &flag, "abc", 3);
    return s * 1000 + out * 10 + (flag ? 1 : 0) + m13_flags(fs, 3) * 100000 + none;
}
