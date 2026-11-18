/* M13 conformance: the mod's own C, reading lists and writing the script's places, and calling the script's export
   with a list, bytes, a text and two places of its own. */
#ifndef M13_SIDE_H
#define M13_SIDE_H
#include "lm_exports.h"
#ifdef __cplusplus
extern "C" {
#endif

int32_t m13_total(const int32_t* xs, uint64_t n);
int32_t m13_lit(const bool* xs, uint64_t n);
float m13_spots(const lm_vec3* xs, uint64_t n);
void m13_grow(int32_t* n, bool* lamp, lm_vec3* at);
int32_t m13_run_c(void);

#ifdef __cplusplus
}
#endif
#endif
