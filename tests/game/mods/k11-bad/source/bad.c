/* K11: a fault in a callback's own code. */
#include "bad.h"

int32_t k11_bad(int32_t x) {
    int32_t* volatile nowhere = 0;
    *nowhere = x;
    return x;
}
