// K5 conformance: C++20 in the same mod (abi 9: no exceptions, no RTTI, no standard library), called by the script.
#include "fastmath.h"

namespace {
template <typename T>
constexpr T square(T x) {
    return x * x;
}

struct Pair {
    int32_t a;
    int32_t b;
    constexpr int32_t sum() const { return square(a) + b; }
};
static_assert(Pair{5, 1}.sum() == 26);
}  // namespace

extern "C" int32_t k5_cpp_sum(int32_t a, int32_t b) {
    return Pair{a, b}.sum();
}
