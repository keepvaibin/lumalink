#include "rc.h"

int64_t k8_rc(const char* p, uint64_t n) {
    if (n == 0) return -1;
    return (int64_t)*(const uint32_t*)(const void*)(p - 16);
}
