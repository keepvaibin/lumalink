/* K11: a fault in a library's own code. */
#include "deep.h"

int32_t k11_crash(int32_t x) {
    int32_t* volatile nowhere = 0;
    *nowhere = x;
    return x;
}
