/* K8: the count of a text, read by the mod's own C (a text crosses as its bytes and length; its count is the
   block's first field, 16 bytes before the bytes: lumart.h's lumart_tb). */
#ifndef K8_RC_H
#define K8_RC_H
#include <stdint.h>

int64_t k8_rc(const char* p, uint64_t n);

#endif
