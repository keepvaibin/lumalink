/* K5 conformance: the mod's own C, declared for the script by `use c "fastmath.h"` and `extern c fn`. */
#ifndef K5_FASTMATH_H
#define K5_FASTMATH_H
#include <stdbool.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

float k5_mix(float a, float b);
int64_t k5_len(const char* p, uint64_t n);
int32_t k5_sum_bytes(const uint8_t* p, uint64_t n);
bool k5_odd(int32_t x);
int32_t k5_from_c(void);
int32_t k5_cpp_sum(int32_t a, int32_t b);

#ifdef __cplusplus
}
#endif
#endif
