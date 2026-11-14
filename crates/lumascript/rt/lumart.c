/* SPDX-License-Identifier: GPL-3.0-only */
/* lumart: see lumart.h. Freestanding C17; the compiler's memcpy/memset come from luma.h's LUMA_MEMORY in the mod. */
#include "lumart.h"
#include "luma_text_tags.h"

/* ---------------------------------------------------------------- memory (docs/modding/language/compiled-form.md, section 7.1)

   Blocks of 16 to 512 bytes come from pools, one free list per size class, carved from 64 KiB pages the core gives;
   each page's first 16 bytes link it to the page before, and lumart_end gives every page back once the mod has
   ended. Larger blocks go straight to the core. Every free names the size it was asked with, so no block carries a
   header. */

#define POOL_PAGE 65536u
#define POOL_CLASSES 10

static const uint32_t pool_size[POOL_CLASSES] = {16, 32, 48, 64, 96, 128, 192, 256, 384, 512};

typedef struct pool_free {
    struct pool_free* next;
} pool_free;

static pool_free* pool_list[POOL_CLASSES];
static char* pool_page;      /* the page blocks are carved from */
static char* pool_pages;     /* the last page taken; each page's first pointer is the one before */
static uint32_t pool_used;   /* bytes of it carved */

static int pool_class(uint64_t size) {
#ifdef LUMART_HOST_NO_POOL
    /* The host checks: every block from the core, so a leak or a use after free shows. */
    (void)size;
    return -1;
#endif
    for (int i = 0; i < POOL_CLASSES; i++) {
        if (size <= pool_size[i]) return i;
    }
    return -1;
}

void* lumart_alloc(uint64_t size) {
    if (luma_rt_core == 0) return 0;
    if (size == 0) size = 1;
    const int c = pool_class(size);
    if (c < 0) return luma_rt_core->alloc(luma_rt_self, size, 16);
    pool_free* f = pool_list[c];
    if (f != 0) {
        pool_list[c] = f->next;
        return f;
    }
    const uint32_t n = pool_size[c];
    if (pool_page == 0 || pool_used + n > POOL_PAGE) {
        /* The page's rest goes to the free lists, largest class first, so nothing of it is lost. */
        if (pool_page != 0) {
            for (int k = POOL_CLASSES - 1; k >= 0; k--) {
                while (pool_used + pool_size[k] <= POOL_PAGE) {
                    pool_free* r = (pool_free*)(pool_page + pool_used);
                    r->next = pool_list[k];
                    pool_list[k] = r;
                    pool_used += pool_size[k];
                }
            }
        }
        char* page = (char*)luma_rt_core->alloc(luma_rt_self, POOL_PAGE, 16);
        if (page == 0) return 0;
        *(char**)page = pool_pages;
        pool_pages = page;
        pool_page = page;
        pool_used = 16;
    }
    void* p = pool_page + pool_used;
    pool_used += n;
    return p;
}

void lumart_free(void* p, uint64_t size) {
    if (p == 0 || luma_rt_core == 0) return;
    if (size == 0) size = 1;
    const int c = pool_class(size);
    if (c < 0) {
        luma_rt_core->free(luma_rt_self, p);
        return;
    }
    pool_free* f = (pool_free*)p;
    f->next = pool_list[c];
    pool_list[c] = f;
}

/* ---------------------------------------------------------------- numbers as text, with no C library */

/* Unsigned decimal into the end of `buf` (24 bytes); gives the first character. */
static char* digits_u64(char* end, uint64_t v) {
    *--end = 0;
    do {
        *--end = (char)('0' + v % 10u);
        v /= 10u;
    } while (v != 0);
    return end;
}

/* ---------------------------------------------------------------- script errors */

lumart_as lumart_caller;

int32_t lumart_raise(const char* file, uint32_t line, uint32_t code, uint32_t argc, const char* const* args) {
    if (luma_rt_core == 0) return 1;
    if (lumart_caller.mod != 0) {
        luma_rt_core->report(lumart_caller.mod, lumart_caller.file, lumart_caller.line, code, args, argc);
    } else {
        luma_rt_core->report(luma_rt_self, file, line, code, args, argc);
    }
    return 1;
}

int32_t lumart_wrong_call(const char* file, uint32_t line, const char* fn, const char* param, const char* value, const char* must) {
    const char* args[4] = {fn, param, value, must};
    return lumart_raise(file, line, 914, 4, args);
}

int32_t lumart_wrong_length(const char* file, uint32_t line, const char* fn, const char* param, uint64_t n, uint64_t want) {
    char a[24], b[24];
    return lumart_wrong_call(file, line, fn, param, digits_u64(a + sizeof(a), n), digits_u64(b + sizeof(b), want));
}

int32_t lumart_wrong_multiple(const char* file, uint32_t line, const char* fn, const char* param, uint64_t n, const char* of,
                              uint64_t k) {
    char a[24], b[24], must[96];
    const char* kd = digits_u64(b + sizeof(b), k);
    uint32_t at = 0;
    for (const char* p = of; *p != 0 && at < sizeof(must) - 28u; p++) must[at++] = *p;
    must[at++] = ' ';
    must[at++] = 'x';
    must[at++] = ' ';
    for (const char* p = kd; *p != 0; p++) must[at++] = *p;
    must[at] = 0;
    return lumart_wrong_call(file, line, fn, param, digits_u64(a + sizeof(a), n), must);
}

int32_t lumart_overflow(const char* file, uint32_t line, const char* type, const char* source) {
    const char* args[2] = {type, source};
    return lumart_raise(file, line, 904, 2, args);
}

int32_t lumart_div_zero(const char* file, uint32_t line, const char* source) {
    const char* args[1] = {source};
    return lumart_raise(file, line, 905, 1, args);
}

int32_t lumart_out_of_memory(const char* file, uint32_t line, uint64_t size) {
    char buf[24];
    const char* args[1] = {digits_u64(buf + sizeof(buf), size)};
    return lumart_raise(file, line, 911, 1, args);
}

/* ---------------------------------------------------------------- text being made */

void lumart_sb_init(lumart_sb* b) {
    b->p = 0;
    b->len = 0;
    b->cap = 0;
    b->ok = 1;
    b->scratch = 0;
    b->wanted = 0;
}

void lumart_sb_free(lumart_sb* b) {
    const int32_t scratch = b->scratch;
    if (!scratch) lumart_free(b->p, b->cap);
    lumart_sb_init(b);
    b->scratch = scratch;
}

/* ---------------------------------------------------------------- scratch (docs/modding/language/language.md, section 6.4, docs/modding/language/compiled-form.md, section 7.2) */

#define SCRATCH_CHUNKS 48
#define SCRATCH_FIRST 65536u
#define SCRATCH_AT 0x00FFFFFFFFFFFFFFull
static char* scratch_chunk[SCRATCH_CHUNKS];
static uint64_t scratch_size[SCRATCH_CHUNKS];
static uint32_t scratch_cur;  /* the chunk in use */
static uint64_t scratch_used; /* its bytes in use, from its start */

uint64_t lumart_scratch_mark(void) {
    return ((uint64_t)scratch_cur << 56) | scratch_used;
}

void lumart_scratch_release(uint64_t mark) {
    scratch_cur = (uint32_t)(mark >> 56);
    scratch_used = mark & SCRATCH_AT;
}

void lumart_sb_scratch(lumart_sb* b) {
    lumart_sb_init(b);
    b->scratch = 1;
}

/* Room for `need` bytes (its text and the NUL) for a builder in the scratch. One being written is the scratch's top
   (whatever went above it was given back first, uses nest), so it grows where it is; if it is not (a use above it
   ended by a script error, given back only at its function's end) or its chunk is full, it moves to the top or to
   the next chunk, its bytes copied: no one holds a pointer into a text still being made. */
static int32_t scratch_room(lumart_sb* b, uint64_t need) {
    uint32_t c = scratch_cur;
    char* chunk = scratch_chunk[c];
    if (chunk != 0 && b->p != 0 && b->p + b->cap == chunk + scratch_used && scratch_used - b->cap + need <= scratch_size[c]) {
        scratch_used += need - b->cap;
        b->cap = need;
        return 1;
    }
    uint64_t at = scratch_used;
    if (chunk == 0 || scratch_size[c] - at < need) {
        if (chunk != 0) c++;
        if (c >= SCRATCH_CHUNKS) {
            b->ok = 0;
            b->wanted = need;
            return 0;
        }
        if (scratch_size[c] < need) {
            /* A chunk holds at most SCRATCH_AT bytes: a mark keeps the place in it in 56 bits (lumart_scratch_mark). */
            uint64_t size = c == 0 ? SCRATCH_FIRST : scratch_size[c - 1] * 2;
            if (size < SCRATCH_FIRST) size = SCRATCH_FIRST;
            while (size < need && size < SCRATCH_AT) size *= 2;
            if (size > SCRATCH_AT) size = SCRATCH_AT;
            char* p = size < need ? 0 : (char*)lumart_alloc(size);
            if (p == 0) {
                b->ok = 0;
                b->wanted = size < need ? need : size;
                return 0;
            }
            lumart_free(scratch_chunk[c], scratch_size[c]);
            scratch_chunk[c] = p;
            scratch_size[c] = size;
        }
        at = 0;
    }
    /* The new place is past the builder's old bytes or in another chunk: a forward copy never overwrites them. */
    char* p = scratch_chunk[c] + at;
    for (uint64_t i = 0; i < b->len; i++) p[i] = b->p[i];
    b->p = p;
    b->cap = need;
    scratch_cur = c;
    scratch_used = at + need;
    return 1;
}

const char* lumart_sb_cstr(lumart_sb* b, const char* file, uint32_t line, const char* function, const char* param) {
    for (uint64_t i = 0; i < b->len; i++) {
        if (b->p[i] == 0) {
            const char* args[2] = {function, param};
            lumart_raise(file, line, 918, 2, args);
            return 0;
        }
    }
    return lumart_sb_c(b);
}

void lumart_scratch_free_all(void) {
    for (uint32_t c = 0; c < SCRATCH_CHUNKS; c++) {
        lumart_free(scratch_chunk[c], scratch_size[c]);
        scratch_chunk[c] = 0;
        scratch_size[c] = 0;
    }
    scratch_cur = 0;
    scratch_used = 0;
}

/* Room for `n` more bytes and the NUL. */
static int32_t sb_room(lumart_sb* b, uint64_t n) {
    if (!b->ok) return 0;
    const uint64_t need = b->len + n + 1;
    if (need <= b->cap) return 1;
    if (b->scratch) return scratch_room(b, need);
    uint64_t cap = b->cap != 0 ? b->cap : 64;
    while (cap < need) cap *= 2;
    char* p = (char*)lumart_alloc(cap);
    if (p == 0) {
        b->ok = 0;
        b->wanted = cap;
        return 0;
    }
    for (uint64_t i = 0; i < b->len; i++) p[i] = b->p[i];
    lumart_free(b->p, b->cap);
    b->p = p;
    b->cap = cap;
    return 1;
}

void lumart_sb_bytes(lumart_sb* b, const char* s, uint64_t n) {
    if (!sb_room(b, n)) return;
    for (uint64_t i = 0; i < n; i++) b->p[b->len + i] = s[i];
    b->len += n;
    b->p[b->len] = 0;
}

void lumart_sb_str(lumart_sb* b, const char* s) {
    uint64_t n = 0;
    while (s[n] != 0) n++;
    lumart_sb_bytes(b, s, n);
}

void lumart_sb_u64(lumart_sb* b, uint64_t v) {
    char buf[24];
    lumart_sb_str(b, digits_u64(buf + sizeof(buf), v));
}

void lumart_sb_i64(lumart_sb* b, int64_t v) {
    if (v < 0) {
        lumart_sb_bytes(b, "-", 1);
        lumart_sb_u64(b, 0u - (uint64_t)v);
    } else {
        lumart_sb_u64(b, (uint64_t)v);
    }
}

void lumart_sb_bool(lumart_sb* b, int v) {
    lumart_sb_str(b, v ? "true" : "false");
}

static int utf8_decode(const unsigned char* p, uint64_t n, uint32_t* cp);

void lumart_sb_json_str(lumart_sb* b, const char* s, uint64_t n) {
    static const char hex[] = "0123456789abcdef";
    lumart_sb_bytes(b, "\"", 1);
    for (uint64_t i = 0; i < n; i++) {
        const unsigned char c = (unsigned char)s[i];
        if (c >= 0x80) {
            /* JSON is UTF-8: a game's text in another encoding writes U+FFFD for each byte that starts no UTF-8. */
            uint32_t cp;
            const int k = utf8_decode((const unsigned char*)s + i, n - i, &cp);
            if (k == 0) {
                lumart_sb_str(b, "\\ufffd");
            } else {
                lumart_sb_bytes(b, s + i, (uint64_t)k);
                i += (uint64_t)k - 1;
            }
        } else if (c == '"' || c == '\\') {
            const char e[2] = {'\\', (char)c};
            lumart_sb_bytes(b, e, 2);
        } else if (c < 0x20) {
            const char e[6] = {'\\', 'u', '0', '0', hex[c >> 4], hex[c & 15]};
            lumart_sb_bytes(b, e, 6);
        } else {
            lumart_sb_bytes(b, (const char*)&c, 1);
        }
    }
    lumart_sb_bytes(b, "\"", 1);
}

int32_t lumart_sb_done(lumart_sb* b, const char* file, uint32_t line) {
    if (b->ok) return 0;
    const uint64_t wanted = b->wanted;
    lumart_sb_free(b);
    return lumart_out_of_memory(file, line, wanted);
}

/* ---------------------------------------------------------------- floats as the shortest decimal that reads back

   Burger and Dybvig's free-format algorithm (PLDI 1996) on exact big integers: v = m * 2^e; the digits generated are
   the fewest that lie inside v's rounding interval, so reading them back (round to nearest, ties to even) gives v
   again, for f32 and f64 alike. The interval's ends count when m is even (ties go to the even mantissa). */

/* 3904 bits. Printing: s reaches 2^1076 * 10^17 for f64's smallest subnormal. Reading (lumart_text_parse_f64): a
   divisor of 10^1125 (800 kept digits below 10^-324) shifted to 55 bits under the dividend, 3793 bits. */
#define BIG_LIMBS 122

typedef struct {
    uint32_t w[BIG_LIMBS]; /* little-endian */
    int n;                 /* limbs in use; 0 is zero */
} big;

static void big_set(big* a, uint64_t v) {
    a->n = 0;
    while (v != 0) {
        a->w[a->n++] = (uint32_t)v;
        v >>= 32;
    }
}

static void big_trim(big* a) {
    while (a->n > 0 && a->w[a->n - 1] == 0) a->n--;
}

static void big_mul_small(big* a, uint32_t k) {
    uint64_t carry = 0;
    for (int i = 0; i < a->n; i++) {
        const uint64_t t = (uint64_t)a->w[i] * k + carry;
        a->w[i] = (uint32_t)t;
        carry = t >> 32;
    }
    if (carry != 0 && a->n < BIG_LIMBS) a->w[a->n++] = (uint32_t)carry;
}

static void big_shl(big* a, int bits) {
    if (a->n == 0 || bits == 0) return;
    const int limbs = bits / 32, rest = bits % 32;
    int n = a->n + limbs + 1;
    if (n > BIG_LIMBS) n = BIG_LIMBS;
    for (int i = n - 1; i >= 0; i--) {
        const int from = i - limbs;
        uint32_t hi = from >= 0 && from < a->n ? a->w[from] : 0;
        uint32_t lo = from - 1 >= 0 && from - 1 < a->n ? a->w[from - 1] : 0;
        a->w[i] = rest == 0 ? hi : (hi << rest) | (lo >> (32 - rest));
    }
    a->n = n;
    big_trim(a);
}

static void big_pow10(big* a, int k) {
    while (k >= 9) {
        big_mul_small(a, 1000000000u);
        k -= 9;
    }
    static const uint32_t small[9] = {1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000};
    if (k > 0) big_mul_small(a, small[k]);
}

static int big_cmp(const big* a, const big* b) {
    if (a->n != b->n) return a->n < b->n ? -1 : 1;
    for (int i = a->n - 1; i >= 0; i--) {
        if (a->w[i] != b->w[i]) return a->w[i] < b->w[i] ? -1 : 1;
    }
    return 0;
}

static void big_add(big* out, const big* a, const big* b) {
    const int n = a->n > b->n ? a->n : b->n;
    uint64_t carry = 0;
    for (int i = 0; i < n; i++) {
        const uint64_t t = (uint64_t)(i < a->n ? a->w[i] : 0) + (i < b->n ? b->w[i] : 0) + carry;
        out->w[i] = (uint32_t)t;
        carry = t >> 32;
    }
    out->n = n;
    if (carry != 0 && n < BIG_LIMBS) out->w[out->n++] = (uint32_t)carry;
}

/* a -= b, with a >= b. */
static void big_sub(big* a, const big* b) {
    int64_t borrow = 0;
    for (int i = 0; i < a->n; i++) {
        int64_t t = (int64_t)a->w[i] - (i < b->n ? b->w[i] : 0) - borrow;
        borrow = t < 0;
        a->w[i] = (uint32_t)(t + (borrow << 32));
    }
    big_trim(a);
}

/* (r + mp) against s: high_ok ? >= : >. */
static int big_reaches(const big* r, const big* mp, const big* s, int inclusive) {
    big t;
    big_add(&t, r, mp);
    const int c = big_cmp(&t, s);
    return inclusive ? c >= 0 : c > 0;
}

static int bit_length(uint64_t m) {
    int n = 0;
    while (m != 0) {
        n++;
        m >>= 1;
    }
    return n;
}

/* The shortest digits of m * 2^e (m > 0) and the decimal exponent k: v = 0.digits * 10^k. `boundary`: m is the
   smallest mantissa of its binade and the binade is not the lowest, so the gap below is half the gap above. */
static int shortest(uint64_t m, int e, int boundary, char* digits, int* k_out) {
    big r, s, mp, mm;
    const int even = (m & 1) == 0;
    if (e >= 0) {
        big_set(&r, m);
        big_set(&mp, 1);
        big_shl(&mp, e);
        if (!boundary) {
            big_shl(&r, e + 1);
            big_set(&s, 2);
            mm = mp;
        } else {
            big_shl(&r, e + 2);
            big_set(&s, 4);
            mm = mp;
            big_shl(&mp, 1);
        }
    } else {
        big_set(&r, m);
        big_set(&s, 1);
        if (!boundary) {
            big_shl(&r, 1);
            big_shl(&s, 1 - e);
            big_set(&mp, 1);
            big_set(&mm, 1);
        } else {
            big_shl(&r, 2);
            big_shl(&s, 2 - e);
            big_set(&mp, 2);
            big_set(&mm, 1);
        }
    }
    /* k: ceil(log10 v), estimated from floor(log2 v) (at most one low; the check below raises it). */
    const int log2v = e + bit_length(m) - 1;
    double est = (double)log2v * 0.30102999566398114 - 1e-10;
    int k = (int)est;
    if ((double)k < est) k++;
    if (k >= 0) {
        big_pow10(&s, k);
    } else {
        big_pow10(&r, -k);
        big_pow10(&mp, -k);
        big_pow10(&mm, -k);
    }
    if (big_reaches(&r, &mp, &s, even)) {
        big_mul_small(&s, 10);
        k++;
    }
    int n = 0;
    for (;;) {
        big_mul_small(&r, 10);
        big_mul_small(&mp, 10);
        big_mul_small(&mm, 10);
        int d = 0;
        while (big_cmp(&r, &s) >= 0) {
            big_sub(&r, &s);
            d++;
        }
        const int c_low = big_cmp(&r, &mm);
        const int low = even ? c_low <= 0 : c_low < 0;
        const int high = big_reaches(&r, &mp, &s, even);
        if (!low && !high) {
            digits[n++] = (char)('0' + d);
            if (n >= 40) break; /* never reached: 17 digits suffice for f64 */
            continue;
        }
        if (low && high) {
            big twice = r;
            big_shl(&twice, 1);
            if (big_cmp(&twice, &s) >= 0) d++;
        } else if (high) {
            d++;
        }
        digits[n++] = (char)('0' + d);
        break;
    }
    *k_out = k;
    return n;
}

/* digits * 10^(k - n) as JSON writes a number, by ECMAScript's Number::toString rules. */
static void sb_decimal(lumart_sb* b, int negative, const char* d, int n, int k) {
    if (negative) lumart_sb_bytes(b, "-", 1);
    if (n <= k && k <= 21) {
        lumart_sb_bytes(b, d, (uint64_t)n);
        for (int i = n; i < k; i++) lumart_sb_bytes(b, "0", 1);
    } else if (0 < k && k <= 21) {
        lumart_sb_bytes(b, d, (uint64_t)k);
        lumart_sb_bytes(b, ".", 1);
        lumart_sb_bytes(b, d + k, (uint64_t)(n - k));
    } else if (-6 < k && k <= 0) {
        lumart_sb_bytes(b, "0.", 2);
        for (int i = k; i < 0; i++) lumart_sb_bytes(b, "0", 1);
        lumart_sb_bytes(b, d, (uint64_t)n);
    } else {
        lumart_sb_bytes(b, d, 1);
        if (n > 1) {
            lumart_sb_bytes(b, ".", 1);
            lumart_sb_bytes(b, d + 1, (uint64_t)(n - 1));
        }
        lumart_sb_bytes(b, k - 1 < 0 ? "e-" : "e+", 2);
        lumart_sb_u64(b, (uint64_t)(k - 1 < 0 ? 1 - k : k - 1));
    }
}

static void sb_float(lumart_sb* b, uint64_t bits, int mant_bits, int exp_bits) {
    const int negative = (int)(bits >> (mant_bits + exp_bits));
    const uint64_t frac = bits & ((1ull << mant_bits) - 1);
    const int bias = (1 << (exp_bits - 1)) - 1;
    const int raw = (int)((bits >> mant_bits) & ((1ull << exp_bits) - 1));
    if (raw == (1 << exp_bits) - 1) {
        lumart_sb_str(b, frac != 0 ? "nan" : negative ? "-inf" : "inf");
        return;
    }
    if (raw == 0 && frac == 0) {
        lumart_sb_str(b, negative ? "-0" : "0");
        return;
    }
    uint64_t m;
    int e, boundary;
    if (raw == 0) {
        m = frac;
        e = 1 - bias - mant_bits;
        boundary = 0;
    } else {
        m = frac | (1ull << mant_bits);
        e = raw - bias - mant_bits;
        boundary = frac == 0 && raw > 1;
    }
    char d[48];
    int k = 0;
    const int n = shortest(m, e, boundary, d, &k);
    sb_decimal(b, negative, d, n, k);
}

void lumart_sb_f32(lumart_sb* b, float v) {
    uint32_t bits;
    __builtin_memcpy(&bits, &v, 4);
    sb_float(b, bits, 23, 8);
}

void lumart_sb_f64(lumart_sb* b, double v) {
    uint64_t bits;
    __builtin_memcpy(&bits, &v, 8);
    sb_float(b, bits, 52, 11);
}

void lumart_sb_vec(lumart_sb* b, const float* v, int n) {
    lumart_sb_bytes(b, "[", 1);
    for (int i = 0; i < n; i++) {
        if (i > 0) lumart_sb_bytes(b, ", ", 2);
        lumart_sb_f32(b, v[i]);
    }
    lumart_sb_bytes(b, "]", 1);
}

/* ---------------------------------------------------------------- text */

static uint64_t text_block(uint64_t len) {
    return sizeof(lumart_tb) + len + 1;
}

/* A row's block: the text's head and its key, then where it was made and the message once read. */
typedef struct text_row_tail {
    lumart_text read;  /* the game's message, once read; 0 until then */
    const char* file;  /* the `text "Key"` that made the row */
    uint32_t line;
    uint32_t said;     /* 1 once the mod log said it was read as its key */
} text_row_tail;

static uint64_t row_head(uint64_t len) {
    return (text_block(len) + 7u) & ~(uint64_t)7u;
}

static uint64_t row_block(uint64_t len) {
    return row_head(len) + sizeof(text_row_tail);
}

static text_row_tail* row_tail(lumart_text t) {
    return (text_row_tail*)((char*)t + row_head(t->len));
}

void lumart_text_drop(lumart_text t) {
    if (t->reserved != 0) {
        lm_rel_lumart_text(&row_tail(t)->read);
        lumart_free(t, row_block(t->len));
        return;
    }
    lumart_free(t, text_block(t->len));
}

static lumart_text text_new(const char* p, uint64_t n) {
    lumart_text t = (lumart_text)lumart_alloc(text_block(n));
    if (t == 0) return 0;
    t->rc = 1;
    t->reserved = 0;
    t->len = n;
    for (uint64_t i = 0; i < n; i++) t->b[i] = p[i];
    t->b[n] = 0;
    return t;
}

int32_t lumart_text_make(lumart_text* out, const char* p, uint64_t n, const char* file, uint32_t line) {
    lumart_text t = 0;
    if (n != 0) {
        t = text_new(p, n);
        if (t == 0) return lumart_out_of_memory(file, line, text_block(n));
    }
    lumart_text old = *out;
    *out = t;
    lm_rel_lumart_text(&old);
    return 0;
}

int32_t lumart_text_from_sb(lumart_text* out, lumart_sb* b, const char* file, uint32_t line) {
    if (lumart_sb_done(b, file, line)) return 1;
    const int32_t r = lumart_text_make(out, b->p, b->len, file, line);
    lumart_sb_free(b);
    return r;
}

int32_t lumart_text_len(int32_t* out, lumart_text t, const char* file, uint32_t line, const char* source) {
    const uint64_t n = lumart_text_n(t);
    if (n > 0x7FFFFFFFu) return lumart_overflow(file, line, "i32", source);
    *out = (int32_t)n;
    return 0;
}

uint8_t lumart_text_eq(lumart_text a, lumart_text b) {
    if (a == b) return 1;
    const uint64_t n = lumart_text_n(a);
    if (n != lumart_text_n(b)) return 0;
    const char* p = lumart_text_p(a);
    const char* q = lumart_text_p(b);
    for (uint64_t i = 0; i < n; i++) {
        if (p[i] != q[i]) return 0;
    }
    return 1;
}

int32_t lumart_text_cmp(lumart_text a, lumart_text b) {
    const uint64_t na = lumart_text_n(a), nb = lumart_text_n(b);
    const unsigned char* p = (const unsigned char*)lumart_text_p(a);
    const unsigned char* q = (const unsigned char*)lumart_text_p(b);
    const uint64_t n = na < nb ? na : nb;
    for (uint64_t i = 0; i < n; i++) {
        if (p[i] != q[i]) return p[i] < q[i] ? -1 : 1;
    }
    return na < nb ? -1 : na > nb ? 1 : 0;
}

uint8_t lumart_text_starts(lumart_text t, lumart_text x) {
    const uint64_t n = lumart_text_n(x);
    if (n > lumart_text_n(t)) return 0;
    const char* p = lumart_text_p(t);
    const char* q = lumart_text_p(x);
    for (uint64_t i = 0; i < n; i++) {
        if (p[i] != q[i]) return 0;
    }
    return 1;
}

uint8_t lumart_text_ends(lumart_text t, lumart_text x) {
    const uint64_t n = lumart_text_n(x), m = lumart_text_n(t);
    if (n > m) return 0;
    const char* p = lumart_text_p(t) + (m - n);
    const char* q = lumart_text_p(x);
    for (uint64_t i = 0; i < n; i++) {
        if (p[i] != q[i]) return 0;
    }
    return 1;
}

int64_t lumart_text_find(lumart_text t, lumart_text x) {
    const uint64_t n = lumart_text_n(x), m = lumart_text_n(t);
    if (n > m) return -1;
    const char* p = lumart_text_p(t);
    const char* q = lumart_text_p(x);
    for (uint64_t i = 0; i + n <= m; i++) {
        uint64_t k = 0;
        while (k < n && p[i + k] == q[k]) k++;
        if (k == n) return (int64_t)i;
    }
    return -1;
}

/* Whether byte `i` of `p` (length `n`) starts a code point (or is the end). */
static int text_boundary(const char* p, uint64_t n, uint64_t i) {
    return i == n || (((unsigned char)p[i]) & 0xC0u) != 0x80u;
}

/* S0910: [from, to) is not inside n bytes. Negative offsets are shown as written. */
static int32_t range_error(int64_t from, int64_t to, uint64_t n, const char* file, uint32_t line) {
    char a[24], b[24], c[24];
    char* fa;
    char* fb;
    if (from < 0) {
        fa = digits_u64(a + sizeof(a), 0u - (uint64_t)from);
        *--fa = '-';
    } else {
        fa = digits_u64(a + sizeof(a), (uint64_t)from);
    }
    if (to < 0) {
        fb = digits_u64(b + sizeof(b), 0u - (uint64_t)to);
        *--fb = '-';
    } else {
        fb = digits_u64(b + sizeof(b), (uint64_t)to);
    }
    const char* args[3] = {fa, fb, digits_u64(c + sizeof(c), n)};
    return lumart_raise(file, line, 910, 3, args);
}

int32_t lumart_text_slice(lumart_text* out, lumart_text t, int64_t from, int64_t to, const char* file, uint32_t line) {
    const uint64_t n = lumart_text_n(t);
    const char* p = lumart_text_p(t);
    if (from < 0 || to < from || (uint64_t)to > n) return range_error(from, to, n, file, line);
    if (!text_boundary(p, n, (uint64_t)from) || !text_boundary(p, n, (uint64_t)to)) {
        char a[24];
        const int64_t off = !text_boundary(p, n, (uint64_t)from) ? from : to;
        const char* args[1] = {digits_u64(a + sizeof(a), (uint64_t)off)};
        return lumart_raise(file, line, 909, 1, args);
    }
    if (from == 0 && (uint64_t)to == n) {
        lm_cp_lumart_text(out, t);
        return 0;
    }
    return lumart_text_make(out, p + from, (uint64_t)(to - from), file, line);
}

uint32_t lumart_hash32(lumart_text t) {
    const unsigned char* p = (const unsigned char*)lumart_text_p(t);
    const uint64_t n = lumart_text_n(t);
    uint32_t h = 2166136261u;
    for (uint64_t i = 0; i < n; i++) {
        h ^= p[i];
        h *= 16777619u;
    }
    return h;
}

/* ---------------------------------------------------------------- case, numbers from text */

typedef struct {
    uint32_t lo, hi;
    int32_t delta;
    uint32_t stride;
} lumart_case;

/* Unicode 15.0.0 simple case mappings (UnicodeData.txt fields 12 and 13), generated by
   gen_case.py: 1433 lower in 182 runs, 1450 upper in 200 runs. A run maps lo..hi
   (every `stride`-th code point) to itself plus delta. */
static const lumart_case case_lower[182] = {
    {0x41, 0x5A, 32, 1}, {0xC0, 0xD6, 32, 1}, {0xD8, 0xDE, 32, 1}, {0x100, 0x12E, 1, 2},
    {0x130, 0x130, -199, 1}, {0x132, 0x136, 1, 2}, {0x139, 0x147, 1, 2}, {0x14A, 0x176, 1, 2},
    {0x178, 0x178, -121, 1}, {0x179, 0x17D, 1, 2}, {0x181, 0x181, 210, 1}, {0x182, 0x184, 1, 2},
    {0x186, 0x186, 206, 1}, {0x187, 0x187, 1, 1}, {0x189, 0x18A, 205, 1}, {0x18B, 0x18B, 1, 1},
    {0x18E, 0x18E, 79, 1}, {0x18F, 0x18F, 202, 1}, {0x190, 0x190, 203, 1}, {0x191, 0x191, 1, 1},
    {0x193, 0x193, 205, 1}, {0x194, 0x194, 207, 1}, {0x196, 0x196, 211, 1}, {0x197, 0x197, 209, 1},
    {0x198, 0x198, 1, 1}, {0x19C, 0x19C, 211, 1}, {0x19D, 0x19D, 213, 1}, {0x19F, 0x19F, 214, 1},
    {0x1A0, 0x1A4, 1, 2}, {0x1A6, 0x1A6, 218, 1}, {0x1A7, 0x1A7, 1, 1}, {0x1A9, 0x1A9, 218, 1},
    {0x1AC, 0x1AC, 1, 1}, {0x1AE, 0x1AE, 218, 1}, {0x1AF, 0x1AF, 1, 1}, {0x1B1, 0x1B2, 217, 1},
    {0x1B3, 0x1B5, 1, 2}, {0x1B7, 0x1B7, 219, 1}, {0x1B8, 0x1B8, 1, 1}, {0x1BC, 0x1BC, 1, 1},
    {0x1C4, 0x1C4, 2, 1}, {0x1C5, 0x1C5, 1, 1}, {0x1C7, 0x1C7, 2, 1}, {0x1C8, 0x1C8, 1, 1},
    {0x1CA, 0x1CA, 2, 1}, {0x1CB, 0x1DB, 1, 2}, {0x1DE, 0x1EE, 1, 2}, {0x1F1, 0x1F1, 2, 1},
    {0x1F2, 0x1F4, 1, 2}, {0x1F6, 0x1F6, -97, 1}, {0x1F7, 0x1F7, -56, 1}, {0x1F8, 0x21E, 1, 2},
    {0x220, 0x220, -130, 1}, {0x222, 0x232, 1, 2}, {0x23A, 0x23A, 10795, 1}, {0x23B, 0x23B, 1, 1},
    {0x23D, 0x23D, -163, 1}, {0x23E, 0x23E, 10792, 1}, {0x241, 0x241, 1, 1}, {0x243, 0x243, -195, 1},
    {0x244, 0x244, 69, 1}, {0x245, 0x245, 71, 1}, {0x246, 0x24E, 1, 2}, {0x370, 0x372, 1, 2},
    {0x376, 0x376, 1, 1}, {0x37F, 0x37F, 116, 1}, {0x386, 0x386, 38, 1}, {0x388, 0x38A, 37, 1},
    {0x38C, 0x38C, 64, 1}, {0x38E, 0x38F, 63, 1}, {0x391, 0x3A1, 32, 1}, {0x3A3, 0x3AB, 32, 1},
    {0x3CF, 0x3CF, 8, 1}, {0x3D8, 0x3EE, 1, 2}, {0x3F4, 0x3F4, -60, 1}, {0x3F7, 0x3F7, 1, 1},
    {0x3F9, 0x3F9, -7, 1}, {0x3FA, 0x3FA, 1, 1}, {0x3FD, 0x3FF, -130, 1}, {0x400, 0x40F, 80, 1},
    {0x410, 0x42F, 32, 1}, {0x460, 0x480, 1, 2}, {0x48A, 0x4BE, 1, 2}, {0x4C0, 0x4C0, 15, 1},
    {0x4C1, 0x4CD, 1, 2}, {0x4D0, 0x52E, 1, 2}, {0x531, 0x556, 48, 1}, {0x10A0, 0x10C5, 7264, 1},
    {0x10C7, 0x10C7, 7264, 1}, {0x10CD, 0x10CD, 7264, 1}, {0x13A0, 0x13EF, 38864, 1}, {0x13F0, 0x13F5, 8, 1},
    {0x1C90, 0x1CBA, -3008, 1}, {0x1CBD, 0x1CBF, -3008, 1}, {0x1E00, 0x1E94, 1, 2}, {0x1E9E, 0x1E9E, -7615, 1},
    {0x1EA0, 0x1EFE, 1, 2}, {0x1F08, 0x1F0F, -8, 1}, {0x1F18, 0x1F1D, -8, 1}, {0x1F28, 0x1F2F, -8, 1},
    {0x1F38, 0x1F3F, -8, 1}, {0x1F48, 0x1F4D, -8, 1}, {0x1F59, 0x1F5F, -8, 2}, {0x1F68, 0x1F6F, -8, 1},
    {0x1F88, 0x1F8F, -8, 1}, {0x1F98, 0x1F9F, -8, 1}, {0x1FA8, 0x1FAF, -8, 1}, {0x1FB8, 0x1FB9, -8, 1},
    {0x1FBA, 0x1FBB, -74, 1}, {0x1FBC, 0x1FBC, -9, 1}, {0x1FC8, 0x1FCB, -86, 1}, {0x1FCC, 0x1FCC, -9, 1},
    {0x1FD8, 0x1FD9, -8, 1}, {0x1FDA, 0x1FDB, -100, 1}, {0x1FE8, 0x1FE9, -8, 1}, {0x1FEA, 0x1FEB, -112, 1},
    {0x1FEC, 0x1FEC, -7, 1}, {0x1FF8, 0x1FF9, -128, 1}, {0x1FFA, 0x1FFB, -126, 1}, {0x1FFC, 0x1FFC, -9, 1},
    {0x2126, 0x2126, -7517, 1}, {0x212A, 0x212A, -8383, 1}, {0x212B, 0x212B, -8262, 1}, {0x2132, 0x2132, 28, 1},
    {0x2160, 0x216F, 16, 1}, {0x2183, 0x2183, 1, 1}, {0x24B6, 0x24CF, 26, 1}, {0x2C00, 0x2C2F, 48, 1},
    {0x2C60, 0x2C60, 1, 1}, {0x2C62, 0x2C62, -10743, 1}, {0x2C63, 0x2C63, -3814, 1}, {0x2C64, 0x2C64, -10727, 1},
    {0x2C67, 0x2C6B, 1, 2}, {0x2C6D, 0x2C6D, -10780, 1}, {0x2C6E, 0x2C6E, -10749, 1}, {0x2C6F, 0x2C6F, -10783, 1},
    {0x2C70, 0x2C70, -10782, 1}, {0x2C72, 0x2C72, 1, 1}, {0x2C75, 0x2C75, 1, 1}, {0x2C7E, 0x2C7F, -10815, 1},
    {0x2C80, 0x2CE2, 1, 2}, {0x2CEB, 0x2CED, 1, 2}, {0x2CF2, 0x2CF2, 1, 1}, {0xA640, 0xA66C, 1, 2},
    {0xA680, 0xA69A, 1, 2}, {0xA722, 0xA72E, 1, 2}, {0xA732, 0xA76E, 1, 2}, {0xA779, 0xA77B, 1, 2},
    {0xA77D, 0xA77D, -35332, 1}, {0xA77E, 0xA786, 1, 2}, {0xA78B, 0xA78B, 1, 1}, {0xA78D, 0xA78D, -42280, 1},
    {0xA790, 0xA792, 1, 2}, {0xA796, 0xA7A8, 1, 2}, {0xA7AA, 0xA7AA, -42308, 1}, {0xA7AB, 0xA7AB, -42319, 1},
    {0xA7AC, 0xA7AC, -42315, 1}, {0xA7AD, 0xA7AD, -42305, 1}, {0xA7AE, 0xA7AE, -42308, 1}, {0xA7B0, 0xA7B0, -42258, 1},
    {0xA7B1, 0xA7B1, -42282, 1}, {0xA7B2, 0xA7B2, -42261, 1}, {0xA7B3, 0xA7B3, 928, 1}, {0xA7B4, 0xA7C2, 1, 2},
    {0xA7C4, 0xA7C4, -48, 1}, {0xA7C5, 0xA7C5, -42307, 1}, {0xA7C6, 0xA7C6, -35384, 1}, {0xA7C7, 0xA7C9, 1, 2},
    {0xA7D0, 0xA7D0, 1, 1}, {0xA7D6, 0xA7D8, 1, 2}, {0xA7F5, 0xA7F5, 1, 1}, {0xFF21, 0xFF3A, 32, 1},
    {0x10400, 0x10427, 40, 1}, {0x104B0, 0x104D3, 40, 1}, {0x10570, 0x1057A, 39, 1}, {0x1057C, 0x1058A, 39, 1},
    {0x1058C, 0x10592, 39, 1}, {0x10594, 0x10595, 39, 1}, {0x10C80, 0x10CB2, 64, 1}, {0x118A0, 0x118BF, 32, 1},
    {0x16E40, 0x16E5F, 32, 1}, {0x1E900, 0x1E921, 34, 1},
};

static const lumart_case case_upper[200] = {
    {0x61, 0x7A, -32, 1}, {0xB5, 0xB5, 743, 1}, {0xE0, 0xF6, -32, 1}, {0xF8, 0xFE, -32, 1},
    {0xFF, 0xFF, 121, 1}, {0x101, 0x12F, -1, 2}, {0x131, 0x131, -232, 1}, {0x133, 0x137, -1, 2},
    {0x13A, 0x148, -1, 2}, {0x14B, 0x177, -1, 2}, {0x17A, 0x17E, -1, 2}, {0x17F, 0x17F, -300, 1},
    {0x180, 0x180, 195, 1}, {0x183, 0x185, -1, 2}, {0x188, 0x188, -1, 1}, {0x18C, 0x18C, -1, 1},
    {0x192, 0x192, -1, 1}, {0x195, 0x195, 97, 1}, {0x199, 0x199, -1, 1}, {0x19A, 0x19A, 163, 1},
    {0x19E, 0x19E, 130, 1}, {0x1A1, 0x1A5, -1, 2}, {0x1A8, 0x1A8, -1, 1}, {0x1AD, 0x1AD, -1, 1},
    {0x1B0, 0x1B0, -1, 1}, {0x1B4, 0x1B6, -1, 2}, {0x1B9, 0x1B9, -1, 1}, {0x1BD, 0x1BD, -1, 1},
    {0x1BF, 0x1BF, 56, 1}, {0x1C5, 0x1C5, -1, 1}, {0x1C6, 0x1C6, -2, 1}, {0x1C8, 0x1C8, -1, 1},
    {0x1C9, 0x1C9, -2, 1}, {0x1CB, 0x1CB, -1, 1}, {0x1CC, 0x1CC, -2, 1}, {0x1CE, 0x1DC, -1, 2},
    {0x1DD, 0x1DD, -79, 1}, {0x1DF, 0x1EF, -1, 2}, {0x1F2, 0x1F2, -1, 1}, {0x1F3, 0x1F3, -2, 1},
    {0x1F5, 0x1F5, -1, 1}, {0x1F9, 0x21F, -1, 2}, {0x223, 0x233, -1, 2}, {0x23C, 0x23C, -1, 1},
    {0x23F, 0x240, 10815, 1}, {0x242, 0x242, -1, 1}, {0x247, 0x24F, -1, 2}, {0x250, 0x250, 10783, 1},
    {0x251, 0x251, 10780, 1}, {0x252, 0x252, 10782, 1}, {0x253, 0x253, -210, 1}, {0x254, 0x254, -206, 1},
    {0x256, 0x257, -205, 1}, {0x259, 0x259, -202, 1}, {0x25B, 0x25B, -203, 1}, {0x25C, 0x25C, 42319, 1},
    {0x260, 0x260, -205, 1}, {0x261, 0x261, 42315, 1}, {0x263, 0x263, -207, 1}, {0x265, 0x265, 42280, 1},
    {0x266, 0x266, 42308, 1}, {0x268, 0x268, -209, 1}, {0x269, 0x269, -211, 1}, {0x26A, 0x26A, 42308, 1},
    {0x26B, 0x26B, 10743, 1}, {0x26C, 0x26C, 42305, 1}, {0x26F, 0x26F, -211, 1}, {0x271, 0x271, 10749, 1},
    {0x272, 0x272, -213, 1}, {0x275, 0x275, -214, 1}, {0x27D, 0x27D, 10727, 1}, {0x280, 0x280, -218, 1},
    {0x282, 0x282, 42307, 1}, {0x283, 0x283, -218, 1}, {0x287, 0x287, 42282, 1}, {0x288, 0x288, -218, 1},
    {0x289, 0x289, -69, 1}, {0x28A, 0x28B, -217, 1}, {0x28C, 0x28C, -71, 1}, {0x292, 0x292, -219, 1},
    {0x29D, 0x29D, 42261, 1}, {0x29E, 0x29E, 42258, 1}, {0x345, 0x345, 84, 1}, {0x371, 0x373, -1, 2},
    {0x377, 0x377, -1, 1}, {0x37B, 0x37D, 130, 1}, {0x3AC, 0x3AC, -38, 1}, {0x3AD, 0x3AF, -37, 1},
    {0x3B1, 0x3C1, -32, 1}, {0x3C2, 0x3C2, -31, 1}, {0x3C3, 0x3CB, -32, 1}, {0x3CC, 0x3CC, -64, 1},
    {0x3CD, 0x3CE, -63, 1}, {0x3D0, 0x3D0, -62, 1}, {0x3D1, 0x3D1, -57, 1}, {0x3D5, 0x3D5, -47, 1},
    {0x3D6, 0x3D6, -54, 1}, {0x3D7, 0x3D7, -8, 1}, {0x3D9, 0x3EF, -1, 2}, {0x3F0, 0x3F0, -86, 1},
    {0x3F1, 0x3F1, -80, 1}, {0x3F2, 0x3F2, 7, 1}, {0x3F3, 0x3F3, -116, 1}, {0x3F5, 0x3F5, -96, 1},
    {0x3F8, 0x3F8, -1, 1}, {0x3FB, 0x3FB, -1, 1}, {0x430, 0x44F, -32, 1}, {0x450, 0x45F, -80, 1},
    {0x461, 0x481, -1, 2}, {0x48B, 0x4BF, -1, 2}, {0x4C2, 0x4CE, -1, 2}, {0x4CF, 0x4CF, -15, 1},
    {0x4D1, 0x52F, -1, 2}, {0x561, 0x586, -48, 1}, {0x10D0, 0x10FA, 3008, 1}, {0x10FD, 0x10FF, 3008, 1},
    {0x13F8, 0x13FD, -8, 1}, {0x1C80, 0x1C80, -6254, 1}, {0x1C81, 0x1C81, -6253, 1}, {0x1C82, 0x1C82, -6244, 1},
    {0x1C83, 0x1C84, -6242, 1}, {0x1C85, 0x1C85, -6243, 1}, {0x1C86, 0x1C86, -6236, 1}, {0x1C87, 0x1C87, -6181, 1},
    {0x1C88, 0x1C88, 35266, 1}, {0x1D79, 0x1D79, 35332, 1}, {0x1D7D, 0x1D7D, 3814, 1}, {0x1D8E, 0x1D8E, 35384, 1},
    {0x1E01, 0x1E95, -1, 2}, {0x1E9B, 0x1E9B, -59, 1}, {0x1EA1, 0x1EFF, -1, 2}, {0x1F00, 0x1F07, 8, 1},
    {0x1F10, 0x1F15, 8, 1}, {0x1F20, 0x1F27, 8, 1}, {0x1F30, 0x1F37, 8, 1}, {0x1F40, 0x1F45, 8, 1},
    {0x1F51, 0x1F57, 8, 2}, {0x1F60, 0x1F67, 8, 1}, {0x1F70, 0x1F71, 74, 1}, {0x1F72, 0x1F75, 86, 1},
    {0x1F76, 0x1F77, 100, 1}, {0x1F78, 0x1F79, 128, 1}, {0x1F7A, 0x1F7B, 112, 1}, {0x1F7C, 0x1F7D, 126, 1},
    {0x1F80, 0x1F87, 8, 1}, {0x1F90, 0x1F97, 8, 1}, {0x1FA0, 0x1FA7, 8, 1}, {0x1FB0, 0x1FB1, 8, 1},
    {0x1FB3, 0x1FB3, 9, 1}, {0x1FBE, 0x1FBE, -7205, 1}, {0x1FC3, 0x1FC3, 9, 1}, {0x1FD0, 0x1FD1, 8, 1},
    {0x1FE0, 0x1FE1, 8, 1}, {0x1FE5, 0x1FE5, 7, 1}, {0x1FF3, 0x1FF3, 9, 1}, {0x214E, 0x214E, -28, 1},
    {0x2170, 0x217F, -16, 1}, {0x2184, 0x2184, -1, 1}, {0x24D0, 0x24E9, -26, 1}, {0x2C30, 0x2C5F, -48, 1},
    {0x2C61, 0x2C61, -1, 1}, {0x2C65, 0x2C65, -10795, 1}, {0x2C66, 0x2C66, -10792, 1}, {0x2C68, 0x2C6C, -1, 2},
    {0x2C73, 0x2C73, -1, 1}, {0x2C76, 0x2C76, -1, 1}, {0x2C81, 0x2CE3, -1, 2}, {0x2CEC, 0x2CEE, -1, 2},
    {0x2CF3, 0x2CF3, -1, 1}, {0x2D00, 0x2D25, -7264, 1}, {0x2D27, 0x2D27, -7264, 1}, {0x2D2D, 0x2D2D, -7264, 1},
    {0xA641, 0xA66D, -1, 2}, {0xA681, 0xA69B, -1, 2}, {0xA723, 0xA72F, -1, 2}, {0xA733, 0xA76F, -1, 2},
    {0xA77A, 0xA77C, -1, 2}, {0xA77F, 0xA787, -1, 2}, {0xA78C, 0xA78C, -1, 1}, {0xA791, 0xA793, -1, 2},
    {0xA794, 0xA794, 48, 1}, {0xA797, 0xA7A9, -1, 2}, {0xA7B5, 0xA7C3, -1, 2}, {0xA7C8, 0xA7CA, -1, 2},
    {0xA7D1, 0xA7D1, -1, 1}, {0xA7D7, 0xA7D9, -1, 2}, {0xA7F6, 0xA7F6, -1, 1}, {0xAB53, 0xAB53, -928, 1},
    {0xAB70, 0xABBF, -38864, 1}, {0xFF41, 0xFF5A, -32, 1}, {0x10428, 0x1044F, -40, 1}, {0x104D8, 0x104FB, -40, 1},
    {0x10597, 0x105A1, -39, 1}, {0x105A3, 0x105B1, -39, 1}, {0x105B3, 0x105B9, -39, 1}, {0x105BB, 0x105BC, -39, 1},
    {0x10CC0, 0x10CF2, -64, 1}, {0x118C0, 0x118DF, -32, 1}, {0x16E60, 0x16E7F, -32, 1}, {0x1E922, 0x1E943, -34, 1},
};

#define CASE_N(t) ((int)(sizeof(t) / sizeof((t)[0])))

static uint32_t case_of(const lumart_case* t, int n, uint32_t c) {
    int lo = 0, hi = n - 1, at = -1;
    while (lo <= hi) {
        const int mid = lo + (hi - lo) / 2;
        if (t[mid].lo <= c) {
            at = mid;
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    if (at >= 0 && c <= t[at].hi && (c - t[at].lo) % t[at].stride == 0) return (uint32_t)((int32_t)c + t[at].delta);
    return c;
}

/* One code point of well-formed UTF-8 at p (n bytes left): its length, 0 for a byte that starts none (a game's text
   may hold other encodings: such bytes are kept as they are). */
static int utf8_decode(const unsigned char* p, uint64_t n, uint32_t* cp) {
    const unsigned char b = p[0];
    if (b < 0x80) {
        *cp = b;
        return 1;
    }
    int k;
    uint32_t c;
    unsigned char lo = 0x80, hi = 0xBF;
    if (b >= 0xC2 && b <= 0xDF) {
        k = 2;
        c = b & 0x1F;
    } else if (b >= 0xE0 && b <= 0xEF) {
        k = 3;
        c = b & 0x0F;
        if (b == 0xE0) lo = 0xA0;
        if (b == 0xED) hi = 0x9F; /* no surrogates */
    } else if (b >= 0xF0 && b <= 0xF4) {
        k = 4;
        c = b & 0x07;
        if (b == 0xF0) lo = 0x90;
        if (b == 0xF4) hi = 0x8F;
    } else {
        return 0;
    }
    if ((uint64_t)k > n) return 0;
    for (int i = 1; i < k; i++) {
        const unsigned char x = p[i];
        if (x < (i == 1 ? lo : 0x80) || x > (i == 1 ? hi : 0xBF)) return 0;
        c = (c << 6) | (x & 0x3F);
    }
    *cp = c;
    return k;
}

static int utf8_encode(uint32_t c, char* o) {
    if (c < 0x80) {
        o[0] = (char)c;
        return 1;
    }
    if (c < 0x800) {
        o[0] = (char)(0xC0 | (c >> 6));
        o[1] = (char)(0x80 | (c & 0x3F));
        return 2;
    }
    if (c < 0x10000) {
        o[0] = (char)(0xE0 | (c >> 12));
        o[1] = (char)(0x80 | ((c >> 6) & 0x3F));
        o[2] = (char)(0x80 | (c & 0x3F));
        return 3;
    }
    o[0] = (char)(0xF0 | (c >> 18));
    o[1] = (char)(0x80 | ((c >> 12) & 0x3F));
    o[2] = (char)(0x80 | ((c >> 6) & 0x3F));
    o[3] = (char)(0x80 | (c & 0x3F));
    return 4;
}

int32_t lumart_text_case(lumart_text* out, lumart_text t, uint8_t upper, const char* file, uint32_t line) {
    const unsigned char* p = (const unsigned char*)lumart_text_p(t);
    const uint64_t n = lumart_text_n(t);
    const lumart_case* tab = upper ? case_upper : case_lower;
    const int tn = upper ? CASE_N(case_upper) : CASE_N(case_lower);
    /* Up to the first code point that changes; none: the same text. */
    uint64_t i = 0;
    for (;;) {
        if (i >= n) {
            lm_cp_lumart_text(out, t);
            return 0;
        }
        uint32_t c;
        const int k = utf8_decode(p + i, n - i, &c);
        if (k == 0) {
            i++;
            continue;
        }
        if (case_of(tab, tn, c) != c) break;
        i += (uint64_t)k;
    }
    lumart_sb b;
    lumart_sb_init(&b);
    lumart_sb_bytes(&b, (const char*)p, i);
    while (i < n) {
        uint32_t c;
        const int k = utf8_decode(p + i, n - i, &c);
        if (k == 0) {
            lumart_sb_bytes(&b, (const char*)p + i, 1);
            i++;
            continue;
        }
        char e[4];
        lumart_sb_bytes(&b, e, (uint64_t)utf8_encode(case_of(tab, tn, c), e));
        i += (uint64_t)k;
    }
    return lumart_text_from_sb(out, &b, file, line);
}

uint8_t lumart_text_parse_i64(lumart_text t, int64_t* out) {
    const char* p = lumart_text_p(t);
    const uint64_t n = lumart_text_n(t);
    uint64_t i = 0;
    int neg = 0;
    if (i < n && (p[i] == '+' || p[i] == '-')) {
        neg = p[i] == '-';
        i++;
    }
    if (i == n) return 0;
    const uint64_t lim = neg ? (uint64_t)INT64_MAX + 1u : (uint64_t)INT64_MAX;
    uint64_t v = 0;
    for (; i < n; i++) {
        if (p[i] < '0' || p[i] > '9') return 0;
        const uint64_t d = (uint64_t)(p[i] - '0');
        if (v > (lim - d) / 10u) return 0;
        v = v * 10u + d;
    }
    *out = !neg ? (int64_t)v : v == (uint64_t)INT64_MAX + 1u ? INT64_MIN : -(int64_t)v;
    return 1;
}

static void big_add_small(big* a, uint32_t k) {
    uint64_t carry = k;
    for (int i = 0; i < a->n && carry != 0; i++) {
        const uint64_t s = (uint64_t)a->w[i] + carry;
        a->w[i] = (uint32_t)s;
        carry = s >> 32;
    }
    if (carry != 0 && a->n < BIG_LIMBS) a->w[a->n++] = (uint32_t)carry;
}

static void big_shr1(big* a) {
    for (int i = 0; i < a->n; i++) a->w[i] = (a->w[i] >> 1) | (i + 1 < a->n ? a->w[i + 1] << 31 : 0);
    big_trim(a);
}

static int big_bits(const big* a) {
    return a->n == 0 ? 0 : (a->n - 1) * 32 + bit_length(a->w[a->n - 1]);
}

/* Significant digits kept; any further nonzero digit becomes one more digit 1 (the value then lies strictly between
   the kept digits and the next, which is all rounding needs: an exact tie has no digit past the 768th). */
#define PARSE_DIGITS 800

/* [+-]digits[.digits][e[+-]digits] or [+-].digits[...], nothing else (no space, no inf or nan): the nearest double,
   ties to even, by exact integers. 0 when the text is not a number or the number is past the largest double. */
static uint8_t parse_span(const char* p, uint64_t n, int f32, uint64_t* out) {
    /* The format: significand bits with the hidden one, the smallest normal exponent, the bias, the biased
       exponent of infinity; decimal exponents past which the value is surely zero or surely too large. */
    const int prec = f32 ? 24 : 53, emin = f32 ? -126 : -1022, bias = f32 ? 127 : 1023, inf = f32 ? 255 : 2047;
    const int64_t zero_below = f32 ? -46 : -324, big_above = f32 ? 40 : 310;
    uint64_t i = 0;
    int neg = 0, any = 0, sticky = 0, kept = 0;
    int64_t e10 = 0;
    big d;
    big_set(&d, 0);
    if (i < n && (p[i] == '+' || p[i] == '-')) {
        neg = p[i] == '-';
        i++;
    }
    for (int frac = 0; frac < 2; frac++) {
        if (frac == 1) {
            if (i < n && p[i] == '.') {
                i++;
            } else {
                break;
            }
        }
        for (; i < n && p[i] >= '0' && p[i] <= '9'; i++) {
            const uint32_t dv = (uint32_t)(p[i] - '0');
            any = 1;
            if (kept == 0 && dv == 0) {
                if (frac) e10--;
            } else if (kept < PARSE_DIGITS) {
                big_mul_small(&d, 10);
                big_add_small(&d, dv);
                kept++;
                if (frac) e10--;
            } else {
                if (dv != 0) sticky = 1;
                if (!frac) e10++;
            }
        }
    }
    if (!any) return 0;
    if (i < n && (p[i] == 'e' || p[i] == 'E')) {
        i++;
        int eneg = 0;
        if (i < n && (p[i] == '+' || p[i] == '-')) {
            eneg = p[i] == '-';
            i++;
        }
        if (i == n) return 0;
        int64_t x = 0;
        for (; i < n && p[i] >= '0' && p[i] <= '9'; i++) {
            if (x < 100000000) x = x * 10 + (p[i] - '0');
        }
        e10 += eneg ? -x : x;
    }
    if (i != n) return 0;
    uint64_t bits = 0;
    if (sticky) {
        big_mul_small(&d, 10);
        big_add_small(&d, 1);
        kept++;
        e10--;
    }
    if (d.n == 0 || kept + e10 < zero_below) {
        bits = 0; /* zero, or below half the smallest subnormal (2^-1075 > 10^-324; 2^-150 > 10^-46) */
    } else if (kept + e10 > big_above) {
        return 0;
    } else {
        big num = d, den;
        big_set(&den, 1);
        if (e10 >= 0) {
            big_pow10(&num, (int)e10);
        } else {
            big_pow10(&den, (int)-e10);
        }
        /* num / den scaled by 2^s to 55 bits more than den: a quotient in [2^54, 2^56). */
        const int s = 55 - (big_bits(&num) - big_bits(&den));
        if (s > 0) {
            big_shl(&num, s);
        } else {
            big_shl(&den, -s);
        }
        big_shl(&den, 55);
        uint64_t q = 0;
        for (int b = 55; b >= 0; b--) {
            if (big_cmp(&num, &den) >= 0) {
                big_sub(&num, &den);
                q |= (uint64_t)1 << b;
            }
            big_shr1(&den);
        }
        const int rest = num.n != 0;
        const int e = -s; /* value = (q + rest) * 2^e */
        const int qb = bit_length(q);
        const int top = qb - 1 + e;
        int drop = qb - prec;
        if (top < emin) drop += emin - top;
        if (drop > qb) {
            bits = 0;
        } else {
            uint64_t m = drop >= 64 ? 0 : q >> drop;
            const uint64_t low = drop >= 64 ? q : q & (((uint64_t)1 << drop) - 1);
            const uint64_t half = (uint64_t)1 << (drop - 1);
            if (low > half || (low == half && (rest || (m & 1)))) m++;
            int u = e + drop;
            if (m == (uint64_t)1 << prec) {
                m >>= 1;
                u++;
            }
            if (m >= (uint64_t)1 << (prec - 1)) {
                const int biased = u + (prec - 1) + bias;
                if (biased >= inf) return 0;
                bits = ((uint64_t)biased << (prec - 1)) | (m & (((uint64_t)1 << (prec - 1)) - 1));
            } else {
                bits = m; /* subnormal: u is -1074 (f32: -149) */
            }
        }
    }
    if (neg) bits |= (uint64_t)1 << (f32 ? 31 : 63);
    *out = bits;
    return 1;
}

uint8_t lumart_text_parse_f64(lumart_text t, double* out) {
    uint64_t bits = 0;
    if (!parse_span(lumart_text_p(t), lumart_text_n(t), 0, &bits)) return 0;
    union {
        uint64_t u;
        double f;
    } v;
    v.u = bits;
    *out = v.f;
    return 1;
}

/* ---------------------------------------------------------------- json (D7.4, D4.10)

   One reader over JSON bytes (RFC 8259): white space, values, strings with every escape, numbers by the grammar. A
   read gives 0 and moves past what it read, 1 after a script error it reported (S0911), 2 when the bytes there are
   not that (the decode is then none). Nesting is walked with a stack of 256 levels, never by recursion: a document
   past it does not read. */

#define JR_DEPTH 256

void lumart_jr_init(lumart_jr* r, const void* p, uint64_t n) {
    r->p = (const char*)p;
    r->end = (const char*)p + n;
    r->depth = 0;
}

static void jr_ws(lumart_jr* r) {
    while (r->p < r->end && (*r->p == ' ' || *r->p == '\t' || *r->p == '\n' || *r->p == '\r')) r->p++;
}

uint8_t lumart_jr_done(lumart_jr* r) {
    jr_ws(r);
    return r->p == r->end;
}

static int jr_hex(char c) {
    return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1;
}

/* The end of the string at p (its '"'), past the closing quote; 0 when it is not one (a control byte, a bad
   escape, bytes that are not UTF-8, no end). */
static const char* jr_string_end(const char* p, const char* end) {
    p++;
    while (p < end) {
        const unsigned char c = (unsigned char)*p;
        if (c == '"') return p + 1;
        if (c < 0x20) return 0;
        if (c >= 0x80) {
            /* JSON is UTF-8 (RFC 8259 8.1): a `data` file's bytes come here unchecked. */
            uint32_t cp;
            const int k = utf8_decode((const unsigned char*)p, (uint64_t)(end - p), &cp);
            if (k == 0) return 0;
            p += k;
            continue;
        }
        if (c != '\\') {
            p++;
            continue;
        }
        if (p + 1 >= end) return 0;
        const char e = p[1];
        if (e == 'u') {
            if (end - p < 6) return 0;
            for (int i = 2; i < 6; i++) {
                if (jr_hex(p[i]) < 0) return 0;
            }
            p += 6;
        } else if (e == '"' || e == '\\' || e == '/' || e == 'b' || e == 'f' || e == 'n' || e == 'r' || e == 't') {
            p += 2;
        } else {
            return 0;
        }
    }
    return 0;
}

/* The end of the number at p: -?(0|[1-9][0-9]*)(.[0-9]+)?([eE][+-]?[0-9]+)?; 0 when it is not one. *whole: no
   fraction and no exponent. */
static const char* jr_number_end(const char* p, const char* end, int* whole) {
    *whole = 1;
    if (p < end && *p == '-') p++;
    if (p >= end || *p < '0' || *p > '9') return 0;
    if (*p == '0') {
        p++;
    } else {
        while (p < end && *p >= '0' && *p <= '9') p++;
    }
    if (p < end && *p == '.') {
        *whole = 0;
        p++;
        if (p >= end || *p < '0' || *p > '9') return 0;
        while (p < end && *p >= '0' && *p <= '9') p++;
    }
    if (p < end && (*p == 'e' || *p == 'E')) {
        *whole = 0;
        p++;
        if (p < end && (*p == '+' || *p == '-')) p++;
        if (p >= end || *p < '0' || *p > '9') return 0;
        while (p < end && *p >= '0' && *p <= '9') p++;
    }
    return p;
}

static int jr_word(lumart_jr* r, const char* w, uint64_t n) {
    if ((uint64_t)(r->end - r->p) < n || __builtin_memcmp(r->p, w, n) != 0) return 0;
    r->p += n;
    return 1;
}

/* A key: its string (written to `out` as it stands, when given) and the ':' after it. */
static int jr_key_raw(lumart_jr* r, lumart_sb* out) {
    jr_ws(r);
    if (r->p >= r->end || *r->p != '"') return 0;
    const char* e = jr_string_end(r->p, r->end);
    if (e == 0) return 0;
    if (out != 0) lumart_sb_bytes(out, r->p, (uint64_t)(e - r->p));
    r->p = e;
    jr_ws(r);
    if (r->p >= r->end || *r->p != ':') return 0;
    r->p++;
    if (out != 0) lumart_sb_str(out, ": ");
    return 1;
}

/* One whole value, checked; written to `out` in json.encode's form (", " and ": ", no other space; strings and
   numbers as they stand) when given. 0, or 2 when it is not a value. */
static int32_t jr_value(lumart_jr* r, lumart_sb* out) {
    uint8_t objects[JR_DEPTH / 8];
    int depth = 0;
    for (;;) {
        /* a value */
        jr_ws(r);
        if (r->p >= r->end) return 2;
        const char c = *r->p;
        int closed_now = 0;
        if (c == '[' || c == '{') {
            if (depth == JR_DEPTH) return 2;
            if (c == '{') {
                objects[depth / 8] |= (uint8_t)(1u << (depth % 8));
            } else {
                objects[depth / 8] &= (uint8_t)~(1u << (depth % 8));
            }
            depth++;
            r->p++;
            if (out != 0) lumart_sb_bytes(out, &c, 1);
            jr_ws(r);
            const char close = c == '[' ? ']' : '}';
            if (r->p < r->end && *r->p == close) {
                r->p++;
                depth--;
                if (out != 0) lumart_sb_bytes(out, &close, 1);
                closed_now = 1;
            } else {
                if (c == '{' && !jr_key_raw(r, out)) return 2;
                continue;
            }
        }
        if (!closed_now) {
            const char* e = 0;
            int whole = 0;
            if (c == '"') {
                e = jr_string_end(r->p, r->end);
            } else if (c == '-' || (c >= '0' && c <= '9')) {
                e = jr_number_end(r->p, r->end, &whole);
            } else if (c == 't' || c == 'f' || c == 'n') {
                const char* at = r->p;
                if (jr_word(r, "true", 4) || jr_word(r, "false", 5) || jr_word(r, "null", 4)) e = r->p;
                r->p = at;
            }
            if (e == 0) return 2;
            if (out != 0) lumart_sb_bytes(out, r->p, (uint64_t)(e - r->p));
            r->p = e;
        }
        /* after a value: the closes that follow, then a comma (the next value) or the end */
        for (;;) {
            if (depth == 0) return 0;
            jr_ws(r);
            if (r->p >= r->end) return 2;
            const int obj = (objects[(depth - 1) / 8] >> ((depth - 1) % 8)) & 1;
            const char close = obj ? '}' : ']';
            if (*r->p == ',') {
                r->p++;
                if (out != 0) lumart_sb_str(out, ", ");
                if (obj && !jr_key_raw(r, out)) return 2;
                break;
            }
            if (*r->p != close) return 2;
            r->p++;
            depth--;
            if (out != 0) lumart_sb_bytes(out, &close, 1);
        }
    }
}

int32_t lumart_jr_skip(lumart_jr* r) {
    return jr_value(r, 0);
}

int32_t lumart_jr_raw(lumart_jr* r, lumart_text* out, const char* file, uint32_t line) {
    lumart_sb b;
    lumart_sb_init(&b);
    const int32_t st = jr_value(r, &b);
    if (st != 0) {
        lumart_sb_free(&b);
        return st;
    }
    return lumart_text_from_sb(out, &b, file, line);
}

int32_t lumart_jr_null(lumart_jr* r) {
    jr_ws(r);
    return jr_word(r, "null", 4) ? 0 : 2;
}

uint8_t lumart_jr_is_null(lumart_jr* r) {
    jr_ws(r);
    return (uint64_t)(r->end - r->p) >= 4 && __builtin_memcmp(r->p, "null", 4) == 0;
}

char lumart_jr_next(lumart_jr* r) {
    jr_ws(r);
    return r->p < r->end ? *r->p : 0;
}

int32_t lumart_jr_bool(lumart_jr* r, uint8_t* out) {
    jr_ws(r);
    if (jr_word(r, "true", 4)) {
        *out = 1;
        return 0;
    }
    if (jr_word(r, "false", 5)) {
        *out = 0;
        return 0;
    }
    return 2;
}

/* A whole number's digits: its magnitude (saturated past 2^64 - 1) and sign. */
static const char* jr_whole(lumart_jr* r, int* neg, uint64_t* mag, int* over) {
    jr_ws(r);
    int whole = 0;
    const char* e = r->p < r->end ? jr_number_end(r->p, r->end, &whole) : 0;
    if (e == 0 || !whole) return 0;
    const char* p = r->p;
    *neg = *p == '-';
    if (*neg) p++;
    uint64_t v = 0;
    *over = 0;
    for (; p < e; p++) {
        const uint64_t dv = (uint64_t)(*p - '0');
        if (v > (UINT64_MAX - dv) / 10u) *over = 1;
        v = v * 10u + dv;
    }
    *mag = v;
    return e;
}

int32_t lumart_jr_i64(lumart_jr* r, int64_t lo, int64_t hi, int64_t* out) {
    int neg = 0, over = 0;
    uint64_t mag = 0;
    const char* e = jr_whole(r, &neg, &mag, &over);
    if (e == 0 || over) return 2;
    int64_t v;
    if (neg) {
        if (mag > (uint64_t)INT64_MAX + 1u) return 2;
        v = mag == (uint64_t)INT64_MAX + 1u ? INT64_MIN : -(int64_t)mag;
    } else {
        if (mag > (uint64_t)INT64_MAX) return 2;
        v = (int64_t)mag;
    }
    if (v < lo || v > hi) return 2;
    *out = v;
    r->p = e;
    return 0;
}

int32_t lumart_jr_u64(lumart_jr* r, uint64_t hi, uint64_t* out) {
    int neg = 0, over = 0;
    uint64_t mag = 0;
    const char* e = jr_whole(r, &neg, &mag, &over);
    if (e == 0 || over || (neg && mag != 0) || mag > hi) return 2;
    *out = mag;
    r->p = e;
    return 0;
}

static int32_t jr_float_bits(lumart_jr* r, int f32, uint64_t* bits) {
    jr_ws(r);
    int whole = 0;
    const char* e = r->p < r->end ? jr_number_end(r->p, r->end, &whole) : 0;
    if (e == 0 || !parse_span(r->p, (uint64_t)(e - r->p), f32, bits)) return 2;
    r->p = e;
    return 0;
}

int32_t lumart_jr_f64(lumart_jr* r, double* out) {
    uint64_t bits = 0;
    const int32_t st = jr_float_bits(r, 0, &bits);
    if (st == 0) __builtin_memcpy(out, &bits, 8);
    return st;
}

int32_t lumart_jr_f32(lumart_jr* r, float* out) {
    uint64_t bits = 0;
    const int32_t st = jr_float_bits(r, 1, &bits);
    if (st == 0) {
        const uint32_t b32 = (uint32_t)bits;
        __builtin_memcpy(out, &b32, 4);
    }
    return st;
}

static void sb_utf8(lumart_sb* b, uint32_t c) {
    char u[4];
    int n;
    if (c < 0x80u) {
        u[0] = (char)c;
        n = 1;
    } else if (c < 0x800u) {
        u[0] = (char)(0xC0u | (c >> 6));
        u[1] = (char)(0x80u | (c & 0x3Fu));
        n = 2;
    } else if (c < 0x10000u) {
        u[0] = (char)(0xE0u | (c >> 12));
        u[1] = (char)(0x80u | ((c >> 6) & 0x3Fu));
        u[2] = (char)(0x80u | (c & 0x3Fu));
        n = 3;
    } else {
        u[0] = (char)(0xF0u | (c >> 18));
        u[1] = (char)(0x80u | ((c >> 12) & 0x3Fu));
        u[2] = (char)(0x80u | ((c >> 6) & 0x3Fu));
        u[3] = (char)(0x80u | (c & 0x3Fu));
        n = 4;
    }
    lumart_sb_bytes(b, u, (uint64_t)n);
}

static uint32_t jr_u4(const char* p) {
    return (uint32_t)(jr_hex(p[0]) << 12 | jr_hex(p[1]) << 8 | jr_hex(p[2]) << 4 | jr_hex(p[3]));
}

int32_t lumart_jr_text(lumart_jr* r, lumart_text* out, const char* file, uint32_t line) {
    jr_ws(r);
    if (r->p >= r->end || *r->p != '"') return 2;
    const char* e = jr_string_end(r->p, r->end);
    if (e == 0) return 2;
    lumart_sb b;
    lumart_sb_init(&b);
    const char* p = r->p + 1;
    const char* stop = e - 1;
    while (p < stop) {
        const char* q = p;
        while (q < stop && *q != '\\') q++;
        if (q > p) lumart_sb_bytes(&b, p, (uint64_t)(q - p));
        if (q == stop) break;
        const char x = q[1];
        if (x != 'u') {
            const char c = x == 'b' ? '\b' : x == 'f' ? '\f' : x == 'n' ? '\n' : x == 'r' ? '\r' : x == 't' ? '\t' : x;
            lumart_sb_bytes(&b, &c, 1);
            p = q + 2;
            continue;
        }
        uint32_t c = jr_u4(q + 2);
        p = q + 6;
        if (c >= 0xD800u && c <= 0xDBFFu) {
            /* a high surrogate: its low one must follow */
            if (stop - p < 6 || p[0] != '\\' || p[1] != 'u') {
                lumart_sb_free(&b);
                return 2;
            }
            const uint32_t lo = jr_u4(p + 2);
            if (lo < 0xDC00u || lo > 0xDFFFu) {
                lumart_sb_free(&b);
                return 2;
            }
            c = 0x10000u + ((c - 0xD800u) << 10) + (lo - 0xDC00u);
            p += 6;
        } else if (c >= 0xDC00u && c <= 0xDFFFu) {
            lumart_sb_free(&b);
            return 2;
        }
        sb_utf8(&b, c);
    }
    r->p = e;
    return lumart_text_from_sb(out, &b, file, line);
}

int32_t lumart_jr_open(lumart_jr* r, char open) {
    jr_ws(r);
    if (r->p >= r->end || *r->p != open) return 2;
    /* A reader function per type calls the next for each level: the levels, and so its stack, are bounded. */
    if (r->depth >= JR_DEPTH) return 2;
    r->depth++;
    r->p++;
    return 0;
}

int32_t lumart_jr_more(lumart_jr* r, char close, uint64_t* index, uint8_t* more) {
    jr_ws(r);
    if (r->p >= r->end) return 2;
    if (*r->p == close) {
        r->p++;
        r->depth--;
        *more = 0;
        return 0;
    }
    if (*index != 0) {
        if (*r->p != ',') return 2;
        r->p++;
    }
    (*index)++;
    *more = 1;
    return 0;
}

int32_t lumart_jr_key(lumart_jr* r, lumart_text* out, const char* file, uint32_t line) {
    const int32_t st = lumart_jr_text(r, out, file, line);
    if (st != 0) return st;
    jr_ws(r);
    if (r->p >= r->end || *r->p != ':') return 2;
    r->p++;
    return 0;
}

int32_t lumart_jr_field(lumart_jr* r, lumart_text* out, uint8_t* is_text, const char* file, uint32_t line) {
    const char* at = r->p;
    *is_text = 1;
    const int32_t st = lumart_jr_key(r, out, file, line);
    if (st != 2) return st;
    /* A key that is a JSON string but no text (a lone surrogate) names no field and is never a key asked for. */
    lm_rel_lumart_text(out);
    r->p = at;
    *is_text = 0;
    return jr_key_raw(r, 0) ? 0 : 2;
}

/* A key's string with no escape, its bytes in [*p, *n), its ':' read. */
static int32_t jr_plain_key(lumart_jr* r, const char** p, uint64_t* n) {
    jr_ws(r);
    if (r->p >= r->end || *r->p != '"') return 2;
    const char* q = r->p + 1;
    while (q < r->end && *q != '"' && *q != '\\' && (unsigned char)*q >= 0x20) q++;
    if (q >= r->end || *q != '"') return 2;
    *p = r->p + 1;
    *n = (uint64_t)(q - *p);
    r->p = q + 1;
    jr_ws(r);
    if (r->p >= r->end || *r->p != ':') return 2;
    r->p++;
    return 0;
}

int32_t lumart_jr_key_i64(lumart_jr* r, int64_t lo, int64_t hi, int64_t* out) {
    const char* p;
    uint64_t n;
    if (jr_plain_key(r, &p, &n) != 0) return 2;
    if (n == 0 || (*p != '-' && (*p < '0' || *p > '9'))) return 2;
    lumart_jr k;
    lumart_jr_init(&k, p, n);
    if (lumart_jr_i64(&k, lo, hi, out) != 0 || k.p != k.end) return 2;
    return 0;
}

int32_t lumart_jr_key_u64(lumart_jr* r, uint64_t hi, uint64_t* out) {
    const char* p;
    uint64_t n;
    if (jr_plain_key(r, &p, &n) != 0) return 2;
    if (n == 0 || *p < '0' || *p > '9') return 2;
    lumart_jr k;
    lumart_jr_init(&k, p, n);
    if (lumart_jr_u64(&k, hi, out) != 0 || k.p != k.end) return 2;
    return 0;
}

uint8_t lumart_text_is(lumart_text t, const char* s, uint64_t n) {
    return lumart_text_n(t) == n && (n == 0 || __builtin_memcmp(lumart_text_p(t), s, n) == 0);
}

/* json.Value's methods: the value is one complete JSON text (json.encode's form). */

int32_t lumart_json_text(lumart_text v, lumart_text* out, uint8_t* some, const char* file, uint32_t line) {
    lumart_jr r;
    lumart_jr_init(&r, lumart_text_p(v), lumart_text_n(v));
    const int32_t st = lumart_jr_text(&r, out, file, line);
    *some = st == 0 && lumart_jr_done(&r);
    if (!*some) lm_rel_lumart_text(out);
    return st == 1;
}

uint8_t lumart_json_int(lumart_text v, int32_t* out) {
    lumart_jr r;
    lumart_jr_init(&r, lumart_text_p(v), lumart_text_n(v));
    int64_t x = 0;
    if (lumart_jr_i64(&r, INT32_MIN, INT32_MAX, &x) != 0 || !lumart_jr_done(&r)) return 0;
    *out = (int32_t)x;
    return 1;
}

uint8_t lumart_json_float(lumart_text v, float* out) {
    lumart_jr r;
    lumart_jr_init(&r, lumart_text_p(v), lumart_text_n(v));
    return lumart_jr_f32(&r, out) == 0 && lumart_jr_done(&r);
}

uint8_t lumart_json_bool(lumart_text v, uint8_t* out) {
    lumart_jr r;
    lumart_jr_init(&r, lumart_text_p(v), lumart_text_n(v));
    return lumart_jr_bool(&r, out) == 0 && lumart_jr_done(&r);
}

int32_t lumart_json_list(lumart_text v, lumart_list* out, const lumart_desc* d, uint8_t* some, const char* file, uint32_t line) {
    lumart_jr r;
    lumart_jr_init(&r, lumart_text_p(v), lumart_text_n(v));
    *some = 0;
    lm_rel_lumart_list(out);
    if (lumart_jr_open(&r, '[') != 0) return 0;
    uint64_t i = 0;
    uint8_t more = 1;
    for (;;) {
        if (lumart_jr_more(&r, ']', &i, &more) != 0) break;
        if (!more) {
            *some = lumart_jr_done(&r);
            if (!*some) lm_rel_lumart_list(out);
            return 0;
        }
        lumart_text e = 0;
        const int32_t st = lumart_jr_raw(&r, &e, file, line);
        if (st == 0 && lumart_list_push(out, d, &e, file, line) != 0) {
            lm_rel_lumart_text(&e);
            lm_rel_lumart_list(out);
            return 1;
        }
        lm_rel_lumart_text(&e);
        if (st != 0) {
            lm_rel_lumart_list(out);
            return st == 1;
        }
    }
    lm_rel_lumart_list(out);
    return 0;
}

int32_t lumart_json_map(lumart_text v, lumart_map* out, const lumart_desc* kd, const lumart_desc* vd, uint8_t* some, const char* file, uint32_t line) {
    lumart_jr r;
    lumart_jr_init(&r, lumart_text_p(v), lumart_text_n(v));
    *some = 0;
    lm_rel_lumart_map(out);
    if (lumart_jr_open(&r, '{') != 0) return 0;
    uint64_t i = 0;
    uint8_t more = 1;
    for (;;) {
        if (lumart_jr_more(&r, '}', &i, &more) != 0) break;
        if (!more) {
            *some = lumart_jr_done(&r);
            if (!*some) lm_rel_lumart_map(out);
            return 0;
        }
        lumart_text k = 0, e = 0;
        int32_t st = lumart_jr_key(&r, &k, file, line);
        if (st == 0) st = lumart_jr_raw(&r, &e, file, line);
        uint8_t added = 0;
        if (st == 0 && lumart_map_put(out, kd, vd, &k, &e, &added, file, line) != 0) st = 1;
        lm_rel_lumart_text(&k);
        lm_rel_lumart_text(&e);
        if (st != 0) {
            lm_rel_lumart_map(out);
            return st == 1;
        }
    }
    lm_rel_lumart_map(out);
    return 0;
}

int32_t lumart_json_get(lumart_text v, lumart_text key, lumart_text* out, uint8_t* some, const char* file, uint32_t line) {
    lumart_jr r;
    lumart_jr_init(&r, lumart_text_p(v), lumart_text_n(v));
    *some = 0;
    if (lumart_jr_open(&r, '{') != 0) return 0;
    uint64_t i = 0;
    uint8_t more = 1, found = 0;
    for (;;) {
        if (lumart_jr_more(&r, '}', &i, &more) != 0) break;
        if (!more) {
            *some = found && lumart_jr_done(&r);
            if (!*some) lm_rel_lumart_text(out);
            return 0;
        }
        lumart_text k = 0;
        uint8_t is_text = 0;
        int32_t st = lumart_jr_field(&r, &k, &is_text, file, line);
        /* The last of a key written twice, as map() keeps it. */
        if (st == 0 && is_text && lumart_text_n(k) == lumart_text_n(key) && __builtin_memcmp(lumart_text_p(k), lumart_text_p(key), lumart_text_n(k)) == 0) {
            st = lumart_jr_raw(&r, out, file, line);
            found = st == 0;
        } else if (st == 0) {
            st = lumart_jr_skip(&r);
        }
        lm_rel_lumart_text(&k);
        if (st != 0) {
            lm_rel_lumart_text(out);
            return st == 1;
        }
    }
    lm_rel_lumart_text(out);
    return 0;
}

int32_t lumart_json_at(lumart_text v, int64_t at, lumart_text* out, uint8_t* some, const char* file, uint32_t line) {
    lumart_jr r;
    lumart_jr_init(&r, lumart_text_p(v), lumart_text_n(v));
    *some = 0;
    if (at < 0 || lumart_jr_open(&r, '[') != 0) return 0;
    uint64_t i = 0;
    uint8_t more = 1, found = 0;
    for (;;) {
        if (lumart_jr_more(&r, ']', &i, &more) != 0) break;
        if (!more) {
            *some = found && lumart_jr_done(&r);
            if (!*some) lm_rel_lumart_text(out);
            return 0;
        }
        const int32_t st = i - 1 == (uint64_t)at ? lumart_jr_raw(&r, out, file, line) : lumart_jr_skip(&r);
        if (i - 1 == (uint64_t)at && st == 0) found = 1;
        if (st != 0) {
            lm_rel_lumart_text(out);
            return st == 1;
        }
    }
    lm_rel_lumart_text(out);
    return 0;
}

/* ---------------------------------------------------------------- containers */

static uint64_t list_block(uint64_t cap, const lumart_desc* d) {
    return sizeof(lumart_lb) + cap * d->size;
}

void lumart_list_drop(lumart_list l) {
    if (l->d->rel != 0) {
        for (uint64_t i = 0; i < l->len; i++) l->d->rel(lumart_list_at(l, i));
    }
    lumart_free(l, list_block(l->cap, l->d));
}

int32_t lumart_list_own(lumart_list* l, const lumart_desc* d, uint64_t more, const char* file, uint32_t line) {
    lumart_list o = *l;
    const uint64_t len = o != 0 ? o->len : 0;
    if (o != 0 && o->rc == 1 && o->cap - len >= more) return 0;
    uint64_t cap = o != 0 && o->rc == 1 ? o->cap * 2 : len + more;
    if (cap < len + more) cap = len + more;
    if (cap < 4) cap = 4;
    lumart_list n = (lumart_list)lumart_alloc(list_block(cap, d));
    if (n == 0) return lumart_out_of_memory(file, line, list_block(cap, d));
    n->rc = 1;
    n->reserved = 0;
    n->len = len;
    n->cap = cap;
    n->d = d;
    if (len != 0) __builtin_memcpy(n + 1, o + 1, len * d->size);
    if (o != 0) {
        if (o->rc == 1) {
            lumart_free(o, list_block(o->cap, o->d)); /* moved: the counts go with the bytes */
        } else {
            if (d->ret != 0) {
                for (uint64_t i = 0; i < len; i++) d->ret(lumart_list_at(n, i));
            }
            o->rc--;
        }
    }
    *l = n;
    return 0;
}

int32_t lumart_list_push(lumart_list* l, const lumart_desc* d, const void* v, const char* file, uint32_t line) {
    return lumart_list_insert(l, d, lumart_list_n(*l), v, file, line);
}

int32_t lumart_list_insert(lumart_list* l, const lumart_desc* d, uint64_t at, const void* v, const char* file, uint32_t line) {
    if (lumart_list_own(l, d, 1, file, line)) return 1;
    lumart_list o = *l;
    char* p = (char*)lumart_list_at(o, at);
    if (at < o->len) __builtin_memmove(p + d->size, p, (o->len - at) * d->size);
    __builtin_memcpy(p, v, d->size);
    if (d->ret != 0) d->ret(p);
    o->len++;
    return 0;
}

int32_t lumart_list_remove(lumart_list* l, const lumart_desc* d, uint64_t at, const char* file, uint32_t line) {
    if (lumart_list_own(l, d, 0, file, line)) return 1;
    lumart_list o = *l;
    char* p = (char*)lumart_list_at(o, at);
    if (d->rel != 0) d->rel(p);
    if (at + 1 < o->len) __builtin_memmove(p, p + d->size, (o->len - at - 1) * d->size);
    o->len--;
    return 0;
}

uint8_t lumart_list_eq(lumart_list a, lumart_list b, const lumart_desc* d) {
    const uint64_t n = lumart_list_n(a);
    if (n != lumart_list_n(b)) return 0;
    if (a == b) return 1;
    for (uint64_t i = 0; i < n; i++) {
        if (!d->eq(lumart_list_at(a, i), lumart_list_at(b, i))) return 0;
    }
    return 1;
}

int64_t lumart_list_find(lumart_list l, const lumart_desc* d, const void* v) {
    const uint64_t n = lumart_list_n(l);
    for (uint64_t i = 0; i < n; i++) {
        if (d->eq(lumart_list_at(l, i), v)) return (int64_t)i;
    }
    return -1;
}

int32_t lumart_less_by_type(void* d, const void* a, const void* b, uint8_t* out) {
    *out = ((const lumart_desc*)d)->cmp(a, b) < 0;
    return 0;
}

static void sorter_run(lumart_sorter* s) {
    s->mid = s->lo + s->w < s->n ? s->lo + s->w : s->n;
    s->hi = s->lo + 2 * s->w < s->n ? s->lo + 2 * s->w : s->n;
    s->i = s->lo;
    s->j = s->mid;
    s->k = s->lo;
}

int32_t lumart_sorter_begin(lumart_sorter* s, lumart_list* l, const lumart_desc* d, uint8_t steal, const char* file, uint32_t line) {
    __builtin_memset(s, 0, sizeof(*s));
    s->l = l;
    s->d = d;
    s->steal = steal;
    s->n = lumart_list_n(*l);
    if (s->n < 2) return 0; /* nothing to compare: next gives 0 at once */
    s->size = d->size;
    if (lumart_list_own(&s->out, d, s->n, file, line)) return 1;
    s->tmp = (char*)lumart_alloc(s->n * s->size);
    if (s->tmp == 0) {
        lm_rel_lumart_list(&s->out);
        return lumart_out_of_memory(file, line, s->n * s->size);
    }
    s->orig = *l;
    lm_ret_lumart_list(&s->orig);
    s->src = (char*)(s->orig + 1);
    s->dst = (char*)(s->out + 1);
    s->w = 1;
    s->lo = 0;
    sorter_run(s);
    return 0;
}

/* The last pass written into `done`: the new block takes the elements, the list takes the new block. */
static void sorter_finish(lumart_sorter* s, char* done) {
    char* oa = (char*)(s->out + 1);
    if (done != oa) __builtin_memcpy(oa, done, s->n * s->size);
    lumart_free(s->tmp, s->n * s->size);
    s->tmp = 0;
    s->out->len = s->n;
    lumart_list cur = *s->l;
    *s->l = s->out;
    s->out = 0;
    if (s->steal && cur == s->orig && s->orig->rc == 2) {
        s->orig->len = 0; /* the list's count and the sorter's: the elements moved with their counts */
    } else if (s->d->ret != 0) {
        for (uint64_t i = 0; i < s->n; i++) s->d->ret(oa + i * s->size);
    }
    lm_rel_lumart_list(&s->orig);
    lm_rel_lumart_list(&cur);
}

uint8_t lumart_sorter_next(lumart_sorter* s, uint8_t y_first) {
    const uint64_t size = s->size;
    if (s->pending) {
        s->pending = 0;
        if (y_first) {
            __builtin_memcpy(s->dst + s->k * size, s->src + s->j * size, size);
            s->j++;
        } else {
            __builtin_memcpy(s->dst + s->k * size, s->src + s->i * size, size);
            s->i++;
        }
        s->k++;
    }
    for (;;) {
        if (s->out == 0) return 0;
        if (s->i < s->mid && s->j < s->hi) {
            s->x = s->src + s->i * size;
            s->y = s->src + s->j * size;
            s->pending = 1;
            return 1;
        }
        /* One run ran out: the rest of the other in order. */
        if (s->i < s->mid) {
            __builtin_memcpy(s->dst + s->k * size, s->src + s->i * size, (s->mid - s->i) * size);
            s->k += s->mid - s->i;
            s->i = s->mid;
        }
        if (s->j < s->hi) {
            __builtin_memcpy(s->dst + s->k * size, s->src + s->j * size, (s->hi - s->j) * size);
            s->k += s->hi - s->j;
            s->j = s->hi;
        }
        s->lo += 2 * s->w;
        if (s->lo >= s->n) {
            /* A pass done: what it wrote is the next pass's source; the list's own block is only ever read. */
            char* done = s->dst;
            char* oa = (char*)(s->out + 1);
            s->dst = done == oa ? s->tmp : oa;
            s->src = done;
            s->w *= 2;
            s->lo = 0;
            if (s->w >= s->n) {
                sorter_finish(s, done);
                return 0;
            }
        }
        sorter_run(s);
    }
}

void lm_rel_lumart_sorter(lumart_sorter* s) {
    if (s->tmp != 0) lumart_free(s->tmp, s->n * s->size);
    if (s->out != 0) {
        s->out->len = 0; /* its elements were never counted */
        lm_rel_lumart_list(&s->out);
    }
    lm_rel_lumart_list(&s->orig);
    __builtin_memset(s, 0, sizeof(*s));
}

int32_t lumart_list_sort(lumart_list* l, const lumart_desc* d, lumart_less less, void* env, const char* file, uint32_t line) {
    lumart_sorter s;
    if (lumart_sorter_begin(&s, l, d, 1, file, line)) return 1;
    uint8_t y_first = 0;
    while (lumart_sorter_next(&s, y_first)) {
        y_first = 0;
        const int32_t st = less(env, s.y, s.x, &y_first);
        if (st != 0) {
            lm_rel_lumart_sorter(&s);
            return st;
        }
    }
    return 0;
}

static int32_t push_piece(lumart_list* out, const lumart_desc* d, const char* p, uint64_t n, const char* file, uint32_t line) {
    lumart_text t = 0;
    if (lumart_text_make(&t, p, n, file, line)) return 1;
    const int32_t st = lumart_list_push(out, d, &t, file, line);
    lm_rel_lumart_text(&t);
    return st;
}

int32_t lumart_text_chars(lumart_list* out, lumart_text t, const lumart_desc* d, const char* file, uint32_t line) {
    lm_rel_lumart_list(out);
    const unsigned char* p = (const unsigned char*)lumart_text_p(t);
    const uint64_t n = lumart_text_n(t);
    uint64_t count = 0;
    for (uint64_t i = 0; i < n; count++) {
        uint32_t c;
        const int k = utf8_decode(p + i, n - i, &c);
        i += k != 0 ? (uint64_t)k : 1u;
    }
    if (count != 0 && lumart_list_own(out, d, count, file, line)) return 1;
    for (uint64_t i = 0; i < n;) {
        uint32_t c;
        int k = utf8_decode(p + i, n - i, &c);
        if (k == 0) k = 1;
        if (push_piece(out, d, (const char*)p + i, (uint64_t)k, file, line)) return 1;
        i += (uint64_t)k;
    }
    return 0;
}

int32_t lumart_text_split(lumart_list* out, lumart_text t, lumart_text by, const lumart_desc* d, const char* file, uint32_t line) {
    const uint64_t m = lumart_text_n(by);
    if (m == 0) return lumart_text_chars(out, t, d, file, line);
    lm_rel_lumart_list(out);
    const char* p = lumart_text_p(t);
    const char* s = lumart_text_p(by);
    const uint64_t n = lumart_text_n(t);
    uint64_t from = 0;
    for (uint64_t i = 0; i + m <= n;) {
        if (p[i] == s[0] && __builtin_memcmp(p + i, s, m) == 0) {
            if (push_piece(out, d, p + from, i - from, file, line)) return 1;
            i += m;
            from = i;
        } else {
            i++;
        }
    }
    return push_piece(out, d, p + from, n - from, file, line);
}

/* ---------------------------------------------------------------- bytes */

static uint8_t u8_eq(const void* a, const void* b) {
    return (uint8_t)(*(const uint8_t*)a == *(const uint8_t*)b);
}
static int32_t u8_cmp(const void* a, const void* b) {
    return (int32_t)*(const uint8_t*)a - (int32_t)*(const uint8_t*)b;
}
const lumart_desc lumart_u8 = {1, 0, 0, u8_eq, u8_cmp};

uint8_t lumart_bytes_eq(lumart_bytes a, lumart_bytes b) {
    const uint64_t n = lumart_list_n(a);
    if (n != lumart_list_n(b)) return 0;
    return (uint8_t)(n == 0 || a == b || __builtin_memcmp(lumart_bytes_p(a), lumart_bytes_p(b), n) == 0);
}

/* A new block of n bytes copied from p, into *out (released first). */
static int32_t bytes_make(lumart_bytes* out, const void* p, uint64_t n, const char* file, uint32_t line) {
    lm_rel_lumart_list(out);
    if (n == 0) return 0;
    if (lumart_list_own(out, &lumart_u8, n, file, line)) return 1;
    __builtin_memcpy(lumart_bytes_p(*out), p, n);
    (*out)->len = n;
    return 0;
}

int32_t lumart_record_new(lumart_bytes* out, int32_t size, const char* file, uint32_t line) {
    lm_rel_lumart_list(out);
    const uint64_t n = size > 0 ? (uint64_t)size : 1u;
    if (lumart_list_own(out, &lumart_u8, n, file, line)) return 1;
    __builtin_memset(lumart_bytes_p(*out), 0, n);
    (*out)->len = n;
    return 0;
}

int32_t lumart_record_copy(lumart_bytes* out, const void* at, int32_t size, const char* file, uint32_t line) {
    if (size <= 0) return lumart_record_new(out, size, file, line);
    return bytes_make(out, at, (uint64_t)size, file, line);
}

uint8_t* lumart_record_at(lumart_bytes* b, int32_t size, uint8_t own, const char* file, uint32_t line) {
    const uint64_t n = size > 0 ? (uint64_t)size : 1u;
    if (lumart_list_n(*b) < n) {
        /* A zero value, or a copy shorter than the size row says (it cannot be: each is made at its size): the
           bytes it has kept, the rest zeroed. */
        lumart_bytes grown = 0;
        if (lumart_record_new(&grown, size, file, line)) return 0;
        if (*b != 0) __builtin_memcpy(lumart_bytes_p(grown), lumart_bytes_p(*b), lumart_list_n(*b));
        lm_rel_lumart_list(b);
        *b = grown;
        return lumart_bytes_p(*b);
    }
    if (own && lumart_list_own(b, &lumart_u8, 0u, file, line)) return 0;
    return lumart_bytes_p(*b);
}

int32_t lumart_bytes_slice(lumart_bytes* out, lumart_bytes b, int64_t from, int64_t to, const char* file, uint32_t line) {
    const uint64_t n = lumart_list_n(b);
    if (from < 0 || to < from || (uint64_t)to > n) return range_error(from, to, n, file, line);
    if (from == 0 && (uint64_t)to == n) {
        lm_cp_lumart_list(out, b);
        return 0;
    }
    return bytes_make(out, lumart_bytes_p(b) + from, (uint64_t)(to - from), file, line);
}

int32_t lumart_bytes_of_text(lumart_bytes* out, lumart_text t, const char* file, uint32_t line) {
    return bytes_make(out, lumart_text_p(t), lumart_text_n(t), file, line);
}

/* A service's `n` bytes at `at` copied as bytes (a struct's bytes field read, svc FieldPtr::Bytes); none for 0. */
int32_t lumart_bytes_from(lumart_bytes* out, const void* at, uint64_t n, const char* file, uint32_t line) {
    if (at == 0) n = 0;
    return bytes_make(out, at, n, file, line);
}

/* `n` zeroed bytes in the scratch, aligned to 16, given back with the scratch's mark: the C array a list field of a
   struct is given to a service as for one call (svc FieldPtr::List). Null after the out-of-memory error it reported. */
void* lumart_scratch_zeroed(uint64_t n, const char* file, uint32_t line) {
    static const char zero[64] = {0};
    lumart_sb b;
    lumart_sb_scratch(&b);
    for (uint64_t left = n + 16u; left > 0;) {
        const uint64_t k = left < 64u ? left : 64u;
        lumart_sb_bytes(&b, zero, k);
        left -= k;
    }
    if (lumart_sb_done(&b, file, line)) return 0;
    return (void*)(((uintptr_t)b.p + 15u) & ~(uintptr_t)15u);
}

int32_t lumart_bytes_text(lumart_text* out, uint8_t* some, lumart_bytes b, const char* file, uint32_t line) {
    const uint64_t n = lumart_list_n(b);
    const unsigned char* p = n != 0 ? lumart_bytes_p(b) : 0;
    *some = 0;
    for (uint64_t i = 0; i < n;) {
        uint32_t cp;
        const int k = utf8_decode(p + i, n - i, &cp);
        if (k == 0) return 0;
        i += (uint64_t)k;
    }
    if (lumart_text_make(out, (const char*)p, n, file, line)) return 1;
    *some = 1;
    return 0;
}

/* A range's end for a report: at + n, saturated (at may be near INT64_MAX). */
static int64_t range_end(int64_t at, uint32_t n) {
    return at > (int64_t)0x7FFFFFFFFFFFFFFFll - (int64_t)n ? (int64_t)0x7FFFFFFFFFFFFFFFll : at + (int64_t)n;
}

int32_t lumart_bytes_read(lumart_bytes b, int64_t at, uint32_t n, uint8_t big, uint64_t* out, const char* file, uint32_t line) {
    const uint64_t len = lumart_list_n(b);
    if (at < 0 || (uint64_t)at > len || len - (uint64_t)at < n) return range_error(at, range_end(at, n), len, file, line);
    const uint8_t* p = lumart_bytes_p(b) + at;
    uint64_t v = 0;
    for (uint32_t i = 0; i < n; i++) v |= (uint64_t)p[big ? n - 1 - i : i] << (8 * i);
    *out = v;
    return 0;
}

int32_t lumart_bytes_write(lumart_bytes* b, int64_t at, uint32_t n, uint8_t big, uint64_t v, const char* file, uint32_t line) {
    const uint64_t len = lumart_list_n(*b);
    if (at < 0 || (uint64_t)at > len || len - (uint64_t)at < n) return range_error(at, range_end(at, n), len, file, line);
    if (lumart_list_own(b, &lumart_u8, 0, file, line)) return 1;
    uint8_t* p = lumart_bytes_p(*b) + at;
    for (uint32_t i = 0; i < n; i++) p[big ? n - 1 - i : i] = (uint8_t)(v >> (8 * i));
    return 0;
}

/* ---------------------------------------------------------------- handles */

typedef struct ref_slot {
    const void* object; /* its NameObj subobject; null: empty */
    uint32_t generation;
    uint32_t next_free;
} ref_slot;

static ref_slot* refs;          /* refs[0] is never handed out: the empty handle's */
static uint32_t refs_cap;       /* slots allocated */
static uint32_t refs_used;      /* slots handed out so far, 0 counted */
static uint32_t refs_live;      /* slots holding an object */
static uint32_t refs_free;      /* an emptied slot to hand out again, 0 for none */
static uint32_t* refs_hash;     /* object -> slot, linear probing; 0 empty */
static uint32_t refs_hash_cap;  /* a power of two, at least twice refs_live */
static uint8_t refs_hooked;

static uint32_t ptr_hash(const void* p) {
    uint64_t x = (uint64_t)(uintptr_t)p;
    x ^= x >> 33;
    x *= 0xff51afd7ed558ccdull;
    x ^= x >> 33;
    return (uint32_t)x;
}

/* The cell holding `o`'s slot, or the empty cell where it would go. */
static uint32_t* refs_cell(const void* o) {
    const uint32_t mask = refs_hash_cap - 1;
    uint32_t i = ptr_hash(o) & mask;
    while (refs_hash[i] != 0 && refs[refs_hash[i]].object != o) i = (i + 1) & mask;
    return &refs_hash[i];
}

static int32_t refs_hash_grow(const char* file, uint32_t line) {
    const uint32_t cap = refs_hash_cap != 0 ? refs_hash_cap * 2 : 16;
    uint32_t* h = (uint32_t*)lumart_alloc((uint64_t)cap * 4);
    if (h == 0) return lumart_out_of_memory(file, line, (uint64_t)cap * 4);
    __builtin_memset(h, 0, (uint64_t)cap * 4);
    if (refs_hash != 0) lumart_free(refs_hash, (uint64_t)refs_hash_cap * 4);
    refs_hash = h;
    refs_hash_cap = cap;
    for (uint32_t s = 1; s < refs_used; s++) {
        if (refs[s].object != 0) *refs_cell(refs[s].object) = s;
    }
    return 0;
}

/* Takes cell `c` out, moving back the cells after it that probed past it. */
static void refs_unlink(uint32_t* c) {
    const uint32_t mask = refs_hash_cap - 1;
    uint32_t i = (uint32_t)(c - refs_hash);
    for (uint32_t j = (i + 1) & mask; refs_hash[j] != 0; j = (j + 1) & mask) {
        const uint32_t k = ptr_hash(refs[refs_hash[j]].object) & mask;
        /* j's home k is cyclically outside (i, j]: it may move to i */
        if ((i <= j) ? (k <= i || k > j) : (k <= i && k > j)) {
            refs_hash[i] = refs_hash[j];
            i = j;
        }
    }
    refs_hash[i] = 0;
}

static int32_t refs_gone(LumaCall* call, void* user);
static int32_t refs_freed(LumaCall* call, void* user);
static int32_t refs_released(LumaCall* call, void* user);

/* What tells a handle its object went (lumart.h, lumart_ref_make). */
typedef struct refs_watch {
    const char* name;
    const char* signature;
    LumaHook hook;
} refs_watch;
static const refs_watch refs_watches[3] = {
    {"lumacore::released", "void(const void *, const void *)", refs_released},
    {"NebulaArena::free", "void(void *)", refs_freed},
    {"NameObj::~NameObj", 0, refs_gone},
};
static const LumaFunction* refs_fn[3];

static void refs_unhook(uint32_t n) {
    for (uint32_t i = 0; i < n; i++) (void)luma_rt_core->unhook(luma_rt_self, refs_fn[i], LUMA_BEFORE, refs_watches[i].hook, 0);
}

/* Every watch hooked: 0; else 1 + the index i of the row that could not be hooked, rows 0..i-1 still hooked (the
   caller takes them out with refs_unhook(i) once it has given the lock back). The rows are
   looked up each time (only when the first slot is made). */
static uint32_t refs_hook(void) {
    for (uint32_t i = 0; i < 3; i++) {
        refs_fn[i] = luma_rt_core->function(refs_watches[i].name, refs_watches[i].signature);
        if (refs_fn[i] == 0 || luma_rt_core->hook(luma_rt_self, refs_fn[i], LUMA_BEFORE, refs_watches[i].hook, 0) != 0) {
            return i + 1;
        }
    }
    return 0;
}

/* A spinning thread's pause: the core is told so (x86 PAUSE, arm64 YIELD). */
static inline void refs_pause(void) {
#if defined(__x86_64__) || defined(__i386__) || defined(_M_X64)
    __builtin_ia32_pause();
#elif defined(__aarch64__) || defined(__arm__)
    __asm__ __volatile__("yield");
#endif
}

/* The table's lock: 0 free, 1 held by the game thread, 2 by another thread. A free or a
   release on another thread empties its slot under it, so a slot never outlives its object (k3-dtor-diag saw none
   there, but nothing promises it). The game thread's nested calls (a free hook run by an allocation inside
   lumart_ref_make) find it held by the game thread, the one game thread being this one, and go on without it.
   Returns whether this call took it. */
static uint32_t refs_lock;
static int refs_take(int game) {
    if (game && __atomic_load_n(&refs_lock, __ATOMIC_RELAXED) == 1u) return 0;
    uint32_t expected = 0;
    while (!__atomic_compare_exchange_n(&refs_lock, &expected, game ? 1u : 2u, 1, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
        refs_pause();
        expected = 0;
    }
    return 1;
}
static void refs_give(int taken) {
    if (taken) __atomic_store_n(&refs_lock, 0u, __ATOMIC_RELEASE);
}

/* The hooks out once no slot is held: from the game thread's outermost calls only (the kernel's hook calls are the
   game thread's; a nested call may run inside lumart_ref_make before its slot exists). Another thread leaves them
   on: they find no slot and cost a lookup until the game thread next empties or makes one. */
static void refs_unhook_if_empty(void) {
    if (refs_live == 0 && refs_hooked) {
        refs_unhook(3);
        refs_hooked = 0;
    }
}

static void refs_empty(uint32_t* c) {
    const uint32_t s = *c;
    refs_unlink(c);
    refs[s].object = 0;
    if (++refs[s].generation == 0) refs[s].generation = 1;
    refs[s].next_free = refs_free;
    refs_free = s;
    refs_live--;
}

/* Every slot of an object in [lo, hi) emptied, the lock held. */
static void refs_empty_range(const void* lo, const void* hi) {
    for (uint32_t s = 1; s < refs_used; s++) {
        const char* o = (const char*)refs[s].object;
        if (o != 0 && o >= (const char*)lo && o < (const char*)hi) refs_empty(refs_cell(o));
    }
}

/* The objects in [lo, hi) (one object: [o, o + 1)) go: their slots emptied, on whichever thread. */
static void refs_forget(const void* lo, const void* hi) {
    if (__atomic_load_n(&refs_live, __ATOMIC_RELAXED) == 0) return;
    const int game = luma_rt_core->game_thread() != 0;
    const int taken = refs_take(game);
    if (refs_live != 0) {
        if ((const char*)hi == (const char*)lo + 1) {
            uint32_t* c = refs_cell(lo);
            if (*c != 0) refs_empty(c);
        } else {
            refs_empty_range(lo, hi);
        }
    }
    refs_give(taken);
    if (game && taken) refs_unhook_if_empty();
}

/* NameObj::~NameObj: args[0] is the object going, as a NameObj. */
static int32_t refs_gone(LumaCall* call, void* user) {
    (void)user;
    const char* o = (const char*)call->args[0];
    refs_forget(o, o + 1);
    return 0;
}

/* NebulaArena::free(void *pData), a method: args[0] the arena, args[1] where pData is. */
static int32_t refs_freed(LumaCall* call, void* user) {
    (void)user;
    const char* o = *(const char* const*)call->args[1];
    if (o != 0) refs_forget(o, o + 1);
    return 0;
}

/* lumacore::released(const void *pStart, const void *pEnd): an arena's addresses, its objects about to go. A
   parameter n is args[n] from 1 (args[0] is `this`, none for a free function), pointing at the value. */
static int32_t refs_released(LumaCall* call, void* user) {
    (void)user;
    refs_forget(*(const void* const*)call->args[1], *(const void* const*)call->args[2]);
    return 0;
}

int32_t lumart_ref_make(lumart_ref* out, const void* view, int32_t nameobj, const char* file, uint32_t line) {
    out->slot = 0;
    out->generation = 0;
    const void* o = view != 0 ? luma_rt_core->as_class(luma_rt_self, view, nameobj) : 0;
    if (o == 0) return 0;
    const int game = luma_rt_core->game_thread() != 0;
    const int taken = refs_take(game);
    int32_t st = 0;
    if (refs_hash_cap != 0) {
        const uint32_t s = *refs_cell(o);
        if (s != 0) {
            out->slot = s;
            out->generation = refs[s].generation;
            refs_give(taken);
            return 0;
        }
    }
    if (!refs_hooked) {
        const uint32_t bad = refs_hook();
        if (bad != 0) {
            /* No watch, no handle: one that could not be emptied would hand out freed memory. The rows hooked so
               far come out after the lock is given back, as on every other path. */
            refs_give(taken);
            refs_unhook(bad - 1);
            const char* args[1] = {refs_watches[bad - 1].name};
            return lumart_raise(file, line, 920, 1, args);
        }
        refs_hooked = 1;
    }
    if ((refs_live + 1) * 2 > refs_hash_cap && refs_hash_grow(file, line)) {
        st = 1;
        goto done;
    }
    uint32_t s = refs_free;
    if (s != 0) {
        refs_free = refs[s].next_free;
    } else {
        if (refs_used == refs_cap) {
            const uint32_t cap = refs_cap != 0 ? refs_cap * 2 : 16;
            ref_slot* n = (ref_slot*)lumart_alloc((uint64_t)cap * sizeof(ref_slot));
            if (n == 0) {
                st = lumart_out_of_memory(file, line, (uint64_t)cap * sizeof(ref_slot));
                goto done;
            }
            __builtin_memset(n, 0, (uint64_t)cap * sizeof(ref_slot));
            if (refs != 0) {
                __builtin_memcpy(n, refs, (uint64_t)refs_used * sizeof(ref_slot));
                lumart_free(refs, (uint64_t)refs_cap * sizeof(ref_slot));
            }
            refs = n;
            refs_cap = cap;
            if (refs_used == 0) refs_used = 1;
        }
        s = refs_used++;
    }
    refs[s].object = o;
    if (++refs[s].generation == 0) refs[s].generation = 1;
    *refs_cell(o) = s;
    refs_live++;
    out->slot = s;
    out->generation = refs[s].generation;
done:
    refs_give(taken);
    if (game && taken) refs_unhook_if_empty();
    return st;
}

void* lumart_ref_get(lumart_ref r, int32_t cls) {
    if (r.slot == 0) return 0;
    const int taken = refs_take(luma_rt_core->game_thread() != 0);
    const void* o = r.slot < refs_used && refs[r.slot].generation == r.generation ? refs[r.slot].object : 0;
    refs_give(taken);
    return o != 0 ? (void*)luma_rt_core->as_class(luma_rt_self, o, cls) : 0;
}

void lumart_refs_release(const void* lo, const void* hi) {
    refs_forget(lo, hi);
}

int32_t lumart_index_error(int64_t i, uint64_t len, const char* file, uint32_t line) {
    char a[24], b[24];
    char* fa;
    if (i < 0) {
        fa = digits_u64(a + sizeof(a), 0u - (uint64_t)i);
        *--fa = '-';
    } else {
        fa = digits_u64(a + sizeof(a), (uint64_t)i);
    }
    char* fb = digits_u64(b + sizeof(b), len);
    const char* args[2] = {fa, fb};
    return lumart_raise(file, line, 902, 2, args);
}

static uint64_t map_block(uint64_t cap, uint64_t stride) {
    return sizeof(lumart_mb) + cap * stride;
}

void lumart_map_drop(lumart_map m) {
    for (uint64_t i = 0; i < m->len; i++) {
        if (m->kd->rel != 0) m->kd->rel(lumart_map_key(m, i));
        if (m->vd != 0 && m->vd->rel != 0) m->vd->rel(lumart_map_val(m, i));
    }
    lumart_free(m, map_block(m->cap, m->stride));
}

int32_t lumart_map_own(lumart_map* m, const lumart_desc* kd, const lumart_desc* vd, uint64_t more, const char* file, uint32_t line) {
    lumart_map o = *m;
    const uint64_t len = o != 0 ? o->len : 0;
    if (o != 0 && o->rc == 1 && o->cap - len >= more) return 0;
    const uint64_t voff = (kd->size + 7) & ~(uint64_t)7;
    const uint64_t stride = (voff + (vd != 0 ? vd->size : 0) + 7) & ~(uint64_t)7;
    uint64_t cap = o != 0 && o->rc == 1 ? o->cap * 2 : len + more;
    if (cap < len + more) cap = len + more;
    if (cap < 4) cap = 4;
    lumart_map n = (lumart_map)lumart_alloc(map_block(cap, stride));
    if (n == 0) return lumart_out_of_memory(file, line, map_block(cap, stride));
    n->rc = 1;
    n->reserved = 0;
    n->len = len;
    n->cap = cap;
    n->kd = kd;
    n->vd = vd;
    n->voff = voff;
    n->stride = stride;
    if (len != 0) __builtin_memcpy(n + 1, o + 1, len * stride);
    if (o != 0) {
        if (o->rc == 1) {
            lumart_free(o, map_block(o->cap, o->stride));
        } else {
            for (uint64_t i = 0; i < len; i++) {
                if (kd->ret != 0) kd->ret(lumart_map_key(n, i));
                if (vd != 0 && vd->ret != 0) vd->ret(lumart_map_val(n, i));
            }
            o->rc--;
        }
    }
    *m = n;
    return 0;
}

uint8_t lumart_map_find(lumart_map m, const lumart_desc* kd, const void* k, uint64_t* at) {
    uint64_t lo = 0, hi = lumart_map_n(m);
    while (lo < hi) {
        const uint64_t mid = lo + (hi - lo) / 2;
        const int32_t c = kd->cmp(lumart_map_key(m, mid), k);
        if (c == 0) {
            *at = mid;
            return 1;
        }
        if (c < 0) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    *at = lo;
    return 0;
}

int32_t lumart_map_put(lumart_map* m, const lumart_desc* kd, const lumart_desc* vd, const void* k, const void* v, uint8_t* added, const char* file, uint32_t line) {
    uint64_t at;
    const uint8_t found = lumart_map_find(*m, kd, k, &at);
    *added = !found;
    if (found && vd == 0) return 0;
    if (lumart_map_own(m, kd, vd, found ? 0 : 1, file, line)) return 1;
    lumart_map o = *m;
    if (found) {
        void* pv = lumart_map_val(o, at);
        if (vd->ret != 0) vd->ret((void*)v);
        if (vd->rel != 0) vd->rel(pv);
        __builtin_memcpy(pv, v, vd->size);
        return 0;
    }
    char* p = (char*)lumart_map_key(o, at);
    if (at < o->len) __builtin_memmove(p + o->stride, p, (o->len - at) * o->stride);
    __builtin_memcpy(p, k, kd->size);
    if (kd->ret != 0) kd->ret(p);
    if (vd != 0) {
        __builtin_memcpy(p + o->voff, v, vd->size);
        if (vd->ret != 0) vd->ret(p + o->voff);
    }
    o->len++;
    return 0;
}

int32_t lumart_map_remove(lumart_map* m, const lumart_desc* kd, const lumart_desc* vd, uint64_t at, const char* file, uint32_t line) {
    if (lumart_map_own(m, kd, vd, 0, file, line)) return 1;
    lumart_map o = *m;
    char* p = (char*)lumart_map_key(o, at);
    if (kd->rel != 0) kd->rel(p);
    if (vd != 0 && vd->rel != 0) vd->rel(p + o->voff);
    if (at + 1 < o->len) __builtin_memmove(p, p + o->stride, (o->len - at - 1) * o->stride);
    o->len--;
    return 0;
}

uint8_t lumart_map_eq(lumart_map a, lumart_map b, const lumart_desc* kd, const lumart_desc* vd) {
    const uint64_t n = lumart_map_n(a);
    if (n != lumart_map_n(b)) return 0;
    if (a == b) return 1;
    for (uint64_t i = 0; i < n; i++) {
        if (!kd->eq(lumart_map_key(a, i), lumart_map_key(b, i))) return 0;
        if (vd != 0 && !vd->eq(lumart_map_val(a, i), lumart_map_val(b, i))) return 0;
    }
    return 1;
}

int32_t lumart_key_error(const char* key, const char* file, uint32_t line) {
    const char* args[1] = {key};
    return lumart_raise(file, line, 907, 1, args);
}

int32_t lumart_text_from_game(lumart_text* out, const char* s, const char* file, uint32_t line) {
    uint64_t n = 0;
    if (s != 0) {
        while (s[n] != 0) n++;
    }
    return lumart_text_make(out, s, n, file, line);
}

/* The game's UTF-16 text (`const char16_t *`, to its 0) as mod text: UTF-8, each of the game's message tags in the
   bracket words of the SDK's table (luma_text_tags.h), "[" as "[[", a lone surrogate as U+FFFD. The same reading as LumaLink's
   messages (nebula-import cook/message_merge.rs decode_text), so a game message read and put back is the game's own.
   A tag (0x1A, its length in bytes in the low byte of the next unit and its group in the high byte, its number, then
   its parameters) may hold 0 units: the text ends at the first 0 outside a tag. */
static void sb_tag(lumart_sb* b, uint32_t group, uint32_t id, const uint16_t* p, uint32_t n) {
    const LumaTextTag* t = luma_text_tag_row(group, id, p, n);
    if (t == 0) {
        lumart_sb_str(b, "[tag ");
        lumart_sb_u64(b, group);
        lumart_sb_str(b, " ");
        lumart_sb_u64(b, id);
        for (uint32_t i = 0; i < n; i++) {
            lumart_sb_str(b, " ");
            lumart_sb_u64(b, p[i]);
        }
        lumart_sb_str(b, "]");
        return;
    }
    lumart_sb_str(b, "[");
    lumart_sb_str(b, t->word);
    if (t->arg == LUMA_TEXT_ARG_UNIT) {
        lumart_sb_str(b, " ");
        lumart_sb_u64(b, p[0]);
    } else if (t->arg == LUMA_TEXT_ARG_HIGH) {
        lumart_sb_str(b, " ");
        lumart_sb_u64(b, (uint64_t)(p[0] >> 8));
    } else if (t->arg == LUMA_TEXT_ARG_INDEX) {
        lumart_sb_str(b, " ");
        lumart_sb_u64(b, id);
        lumart_sb_str(b, " ");
        lumart_sb_u64(b, p[3]);
    } else if (t->arg == LUMA_TEXT_ARG_TAG) {
        lumart_sb_str(b, " ");
        lumart_sb_u64(b, id);
    } else if (t->arg == LUMA_TEXT_ARG_NAME) {
        lumart_sb_str(b, " ");
        for (uint32_t i = 0; i < n; i++) {
            const char c = (char)p[i];
            lumart_sb_bytes(b, &c, 1);
        }
    }
    lumart_sb_str(b, "]");
}

static void sb_game16(lumart_sb* b, const uint16_t* s) {
    uint64_t i = 0;
    while (s[i] != 0) {
        const uint32_t u = s[i];
        if (u == 0x1A && s[i + 1] != 0) {
            const uint32_t length = s[i + 1] & 0xFF;
            const uint32_t group = s[i + 1] >> 8;
            if (length >= 6 && length % 2 == 0) {
                sb_tag(b, group, s[i + 2], s + i + 3, length / 2 - 3);
                i += length / 2;
                continue;
            }
        }
        if (u >= 0xD800 && u < 0xDC00 && s[i + 1] >= 0xDC00 && s[i + 1] < 0xE000) {
            sb_utf8(b, 0x10000u + ((u - 0xD800u) << 10) + (s[i + 1] - 0xDC00u));
            i += 2;
            continue;
        }
        if (u == '[') {
            lumart_sb_str(b, "[[");
        } else {
            sb_utf8(b, u >= 0xD800 && u < 0xE000 ? 0xFFFDu : u);
        }
        i++;
    }
}

int32_t lumart_text_from_game16(lumart_text* out, const uint16_t* s, const char* file, uint32_t line) {
    lumart_sb b;
    lumart_sb_init(&b);
    if (s != 0) sb_game16(&b, s);
    return lumart_text_from_sb(out, &b, file, line);
}

/* `text "Key"` reads the game's messages (the mod's lines are cooked into them) through names bound at run time: the
   message by its key, and the holder of the messages, whose game data is null until the game has read its message
   files. A runtime test names one that cannot bind with -D. */
#ifndef LUMART_MESSAGE_FUNCTION
#define LUMART_MESSAGE_FUNCTION "MR::getGameMessageDirect"
#endif
#ifndef LUMART_MESSAGE_HOLDER_FUNCTION
#define LUMART_MESSAGE_HOLDER_FUNCTION "MR::getGameSystemObjHolder"
#endif
#ifndef LUMART_MESSAGE_HOLDER_FIELD
#define LUMART_MESSAGE_HOLDER_FIELD "mMessageHolder"
#endif
#ifndef LUMART_MESSAGE_DATA_FIELD
#define LUMART_MESSAGE_DATA_FIELD "mGameMessageData"
#endif

static int32_t message_bound;  /* 0: not tried; 1: bound; -1: a name did not bind (said once) */
static const LumaFunction* message_fn;
static const LumaFunction* message_holder_fn;
static int32_t message_holder_at, message_data_at;

static const struct {
    uint32_t rc;
    uint32_t reserved;
    uint64_t len;
    char b[1];
} empty_message = {LUMART_FOREVER, 0, 0, {0}};

static int32_t message_bind(void) {
    if (message_bound != 0) return message_bound > 0;
    if (luma_rt_core == 0) return 0;
    const char* missing = 0;
    message_fn = luma_rt_core->function(LUMART_MESSAGE_FUNCTION, "const char16_t *(const char *)");
    message_holder_fn = luma_rt_core->function(LUMART_MESSAGE_HOLDER_FUNCTION, 0);
    message_holder_at = luma_rt_core->field("GameSystemObjHolder", LUMART_MESSAGE_HOLDER_FIELD, 0);
    message_data_at = luma_rt_core->field("MessageHolder", LUMART_MESSAGE_DATA_FIELD, 0);
    if (message_fn == 0) {
        missing = "function " LUMART_MESSAGE_FUNCTION;
    } else if (message_holder_fn == 0) {
        missing = "function " LUMART_MESSAGE_HOLDER_FUNCTION;
    } else if (message_holder_at < 0) {
        missing = "field GameSystemObjHolder." LUMART_MESSAGE_HOLDER_FIELD;
    } else if (message_data_at < 0) {
        missing = "field MessageHolder." LUMART_MESSAGE_DATA_FIELD;
    }
    if (missing != 0) {
        lumart_sb b;
        lumart_sb_init(&b);
        lumart_sb_str(&b, "text \"Key\" reads as its key: the game has no ");
        lumart_sb_str(&b, missing);
        if (lumart_sb_done(&b, "", 0) == 0) lumart_log(b.p);
        lumart_sb_free(&b);
        message_bound = -1;
        return 0;
    }
    message_bound = 1;
    return 1;
}

/* 1 when the game has read its message files; never reads through a null. */
static int32_t messages_exist(void) {
    if (!message_bind()) return 0;
    void* argv[1] = {0};
    char* holder = 0;
    message_holder_fn->invoke(argv, &holder);
    if (holder == 0) return 0;
    char* messages = *(char**)(holder + message_holder_at);
    if (messages == 0) return 0;
    return *(void**)(messages + message_data_at) != 0;
}

/* The game's message of the NUL-ended key, or null; only once messages_exist(). */
static const uint16_t* message_of(const char* key) {
    void* argv[2];
    const uint16_t* s = 0;
    argv[0] = 0;
    argv[1] = (void*)&key;
    message_fn->invoke(argv, &s);
    return s;
}

lumart_text lumart_text_row(lumart_text t) {
    text_row_tail* r = row_tail(t);
    if (r->read != 0) return r->read;
    if (!message_bind()) {
        t->reserved = LUMART_TEXT_ROW_KEY;
        return t;
    }
    if (!messages_exist()) {
        if (r->said == 0) {
            lumart_sb b;
            r->said = 1;
            lumart_sb_init(&b);
            lumart_sb_str(&b, "text \"");
            lumart_sb_bytes(&b, t->b, t->len);
            lumart_sb_str(&b, "\" (");
            lumart_sb_str(&b, r->file);
            lumart_sb_str(&b, ":");
            lumart_sb_u64(&b, r->line);
            lumart_sb_str(&b, ") was read before the game's messages exist and reads as its key there");
            if (lumart_sb_done(&b, "", 0) == 0) lumart_log(b.p);
            lumart_sb_free(&b);
        }
        return t;
    }
    const uint16_t* s = message_of(t->b);
    if (s == 0) {
        t->reserved = LUMART_TEXT_ROW_KEY;
        return t;
    }
    lumart_text m = 0;
    if (lumart_text_from_game16(&m, s, r->file, r->line) != 0) return t;
    r->read = m != 0 ? m : (lumart_text)(void*)&empty_message;
    return r->read;
}

int32_t lumart_text_message(lumart_text* out, const char* key, uint64_t key_n, const char* file, uint32_t line) {
    if (messages_exist()) {
        const uint16_t* s = message_of(key);
        if (s != 0) return lumart_text_from_game16(out, s, file, line);
        return lumart_text_make(out, key, key_n, file, line);
    }
    if (message_bound < 0) return lumart_text_make(out, key, key_n, file, line);
    lumart_text t = (lumart_text)lumart_alloc(row_block(key_n));
    if (t == 0) return lumart_out_of_memory(file, line, row_block(key_n));
    t->rc = 1;
    t->reserved = LUMART_TEXT_ROW;
    t->len = key_n;
    for (uint64_t i = 0; i < key_n; i++) t->b[i] = key[i];
    t->b[key_n] = 0;
    text_row_tail* r = row_tail(t);
    r->read = 0;
    r->file = file;
    r->line = line;
    r->said = 0;
    lumart_text old = *out;
    *out = t;
    lm_rel_lumart_text(&old);
    return 0;
}

int32_t lumart_text_cross(lumart_text* d, lumart_text s, const char* file, uint32_t line) {
    static const char given[] = "a text another mod gave";
    if (s != 0 && s->reserved == LUMART_TEXT_ROW && row_tail(s)->read == 0) {
        if (messages_exist() || message_bound < 0) return lumart_text_make(d, lumart_text_p(s), lumart_text_n(s), file, line);
        return lumart_text_message(d, s->b, s->len, given, row_tail(s)->line);
    }
    return lumart_text_make(d, lumart_text_p(s), lumart_text_n(s), file, line);
}

/* `text "Key"(name: value)`: every "{name}" of `t` replaced by the value's text `v`. */
int32_t lumart_text_fill(lumart_text* out, lumart_text t, const char* name, uint64_t name_n, const char* v, uint64_t v_n,
                         const char* file, uint32_t line) {
    const char* p = lumart_text_p(t);
    const uint64_t n = lumart_text_n(t);
    lumart_sb b;
    lumart_sb_init(&b);
    uint64_t i = 0;
    while (i < n) {
        if (p[i] == '{' && i + name_n + 1 < n && p[i + name_n + 1] == '}') {
            uint64_t k = 0;
            while (k < name_n && p[i + 1 + k] == name[k]) k++;
            if (k == name_n) {
                lumart_sb_bytes(&b, v, v_n);
                i += name_n + 2;
                continue;
            }
        }
        lumart_sb_bytes(&b, p + i, 1);
        i++;
    }
    return lumart_text_from_sb(out, &b, file, line);
}

/* The kept copies (docs/modding/language/compiled-form.md): an open table by hash, grown at three quarters; each copy counted FOREVER. */
static lumart_text* keep_table;
static uint64_t keep_cap, keep_used;

const char* lumart_text_cstr(lumart_text t, const char* file, uint32_t line, const char* function, const char* param) {
    const uint64_t n = lumart_text_n(t);
    const char* p = lumart_text_p(t);
    for (uint64_t i = 0; i < n; i++) {
        if (p[i] == 0) {
            const char* args[2] = {function, param};
            lumart_raise(file, line, 918, 2, args);
            return 0;
        }
    }
    return t == 0 ? "" : p;
}

/* UTF-16 copies of kept texts (lumart_text_keep16), by the kept text's pointer. */
static const char** k16_keys;
static uint16_t** k16_vals;
static uint64_t k16_used, k16_cap;

static uint64_t k16_slot(const char* k, uint64_t cap) {
    uint64_t x = (uint64_t)(uintptr_t)k;
    x ^= x >> 33;
    x *= 0xff51afd7ed558ccdull;
    x ^= x >> 33;
    return x & (cap - 1);
}

const uint16_t* lumart_text_keep16(lumart_text t, const char* file, uint32_t line, const char* function, const char* param) {
    const char* k = lumart_text_keep(t, file, line, function, param);
    if (k == 0) return 0;
    if (k16_used * 4 >= k16_cap * 3) {
        const uint64_t cap = k16_cap != 0 ? k16_cap * 2 : 32;
        const char** keys = (const char**)lumart_alloc(cap * sizeof(const char*));
        uint16_t** vals = (uint16_t**)lumart_alloc(cap * sizeof(uint16_t*));
        if (keys == 0 || vals == 0) {
            if (keys != 0) lumart_free((void*)keys, cap * sizeof(const char*));
            if (vals != 0) lumart_free(vals, cap * sizeof(uint16_t*));
            lumart_out_of_memory(file, line, cap * (sizeof(const char*) + sizeof(uint16_t*)));
            return 0;
        }
        for (uint64_t i = 0; i < cap; i++) {
            keys[i] = 0;
            vals[i] = 0;
        }
        for (uint64_t i = 0; i < k16_cap; i++) {
            if (k16_keys[i] == 0) continue;
            uint64_t j = k16_slot(k16_keys[i], cap);
            while (keys[j] != 0) j = (j + 1) & (cap - 1);
            keys[j] = k16_keys[i];
            vals[j] = k16_vals[i];
        }
        if (k16_keys != 0) lumart_free((void*)k16_keys, k16_cap * sizeof(const char*));
        if (k16_vals != 0) lumart_free(k16_vals, k16_cap * sizeof(uint16_t*));
        k16_keys = keys;
        k16_vals = vals;
        k16_cap = cap;
    }
    uint64_t j = k16_slot(k, k16_cap);
    while (k16_keys[j] != 0) {
        if (k16_keys[j] == k) return k16_vals[j];
        j = (j + 1) & (k16_cap - 1);
    }
    /* UTF-8 to UTF-16: never more units than bytes, and a 0 at the end. */
    const uint8_t* s = (const uint8_t*)k;
    const uint64_t n = lumart_text_n(t);
    uint16_t* u = (uint16_t*)lumart_alloc((n + 1) * sizeof(uint16_t));
    if (u == 0) {
        lumart_out_of_memory(file, line, (n + 1) * sizeof(uint16_t));
        return 0;
    }
    uint64_t o = 0;
    for (uint64_t i = 0; i < n;) {
        const uint32_t b = s[i];
        uint32_t c = 0xFFFDu;
        uint32_t len = 1;
        if (b == '[') {
            /* The bracket words of a message tag, from the SDK's one table (luma_text_tags.h). */
            size_t used = 0;
            const uint32_t units = luma_text_tag_encode((const char*)s + i, (size_t)(n - i), &used, u + o);
            if (units != 0) {
                o += units;
                i += used;
                continue;
            }
        }
        if (b < 0x80u) {
            c = b;
        } else if ((b & 0xE0u) == 0xC0u && i + 1 < n && (s[i + 1] & 0xC0u) == 0x80u) {
            c = ((b & 0x1Fu) << 6) | (s[i + 1] & 0x3Fu);
            len = 2;
            if (c < 0x80u) c = 0xFFFDu;
        } else if ((b & 0xF0u) == 0xE0u && i + 2 < n && (s[i + 1] & 0xC0u) == 0x80u && (s[i + 2] & 0xC0u) == 0x80u) {
            c = ((b & 0x0Fu) << 12) | ((uint32_t)(s[i + 1] & 0x3Fu) << 6) | (s[i + 2] & 0x3Fu);
            len = 3;
            if (c < 0x800u || (c >= 0xD800u && c <= 0xDFFFu)) c = 0xFFFDu;
        } else if ((b & 0xF8u) == 0xF0u && i + 3 < n && (s[i + 1] & 0xC0u) == 0x80u && (s[i + 2] & 0xC0u) == 0x80u &&
                   (s[i + 3] & 0xC0u) == 0x80u) {
            c = ((b & 0x07u) << 18) | ((uint32_t)(s[i + 1] & 0x3Fu) << 12) | ((uint32_t)(s[i + 2] & 0x3Fu) << 6) | (s[i + 3] & 0x3Fu);
            len = 4;
            if (c < 0x10000u || c > 0x10FFFFu) c = 0xFFFDu;
        }
        if (c >= 0x10000u) {
            c -= 0x10000u;
            u[o++] = (uint16_t)(0xD800u | (c >> 10));
            u[o++] = (uint16_t)(0xDC00u | (c & 0x3FFu));
        } else {
            u[o++] = (uint16_t)c;
        }
        i += len;
    }
    u[o] = 0;
    k16_keys[j] = k;
    k16_vals[j] = u;
    k16_used++;
    return u;
}

/* Function values kept for services' callbacks (lumart_fn_keep). */
static lumart_fn* fn_kept;
static uint32_t fn_kept_n, fn_kept_cap;

int32_t lumart_fn_keep(lumart_fn f, const char* file, uint32_t line) {
    if (f == 0 || f->rc == LUMART_FOREVER) return 0;
    for (uint32_t i = 0; i < fn_kept_n; i++) {
        if (fn_kept[i] == f) return 0;
    }
    if (fn_kept_n == fn_kept_cap) {
        const uint32_t cap = fn_kept_cap != 0 ? fn_kept_cap * 2 : 8;
        lumart_fn* table = (lumart_fn*)lumart_alloc((uint64_t)cap * sizeof(lumart_fn));
        if (table == 0) {
            lumart_out_of_memory(file, line, (uint64_t)cap * sizeof(lumart_fn));
            return 1;
        }
        for (uint32_t i = 0; i < fn_kept_n; i++) table[i] = fn_kept[i];
        if (fn_kept != 0) lumart_free(fn_kept, (uint64_t)fn_kept_cap * sizeof(lumart_fn));
        fn_kept = table;
        fn_kept_cap = cap;
    }
    lm_ret_lumart_fn(&f);
    fn_kept[fn_kept_n++] = f;
    return 0;
}

const char* lumart_text_keep(lumart_text t, const char* file, uint32_t line, const char* function, const char* param) {
    const uint64_t n = lumart_text_n(t);
    const char* p = lumart_text_p(t);
    for (uint64_t i = 0; i < n; i++) {
        if (p[i] == 0) {
            const char* args[2] = {function, param};
            lumart_raise(file, line, 918, 2, args);
            return 0;
        }
    }
    if (t == 0) return "";
    if (t->rc == LUMART_FOREVER) return t->b;
    if (keep_used * 4 >= keep_cap * 3) {
        const uint64_t cap = keep_cap != 0 ? keep_cap * 2 : 64;
        lumart_text* table = (lumart_text*)lumart_alloc(cap * sizeof(lumart_text));
        if (table == 0) {
            lumart_out_of_memory(file, line, cap * sizeof(lumart_text));
            return 0;
        }
        for (uint64_t i = 0; i < cap; i++) table[i] = 0;
        for (uint64_t i = 0; i < keep_cap; i++) {
            lumart_text k = keep_table[i];
            if (k == 0) continue;
            uint64_t j = lumart_hash32(k) & (cap - 1);
            while (table[j] != 0) j = (j + 1) & (cap - 1);
            table[j] = k;
        }
        lumart_free(keep_table, keep_cap * sizeof(lumart_text));
        keep_table = table;
        keep_cap = cap;
    }
    uint64_t j = lumart_hash32(t) & (keep_cap - 1);
    while (keep_table[j] != 0) {
        if (lumart_text_eq(keep_table[j], t)) return keep_table[j]->b;
        j = (j + 1) & (keep_cap - 1);
    }
    lumart_text k = text_new(p, n);
    if (k == 0) {
        lumart_out_of_memory(file, line, text_block(n));
        return 0;
    }
    k->rc = LUMART_FOREVER;
    keep_table[j] = k;
    keep_used++;
    return k->b;
}

/* ---------------------------------------------------------------- the log */

void lumart_log(const char* text) {
    if (luma_rt_core != 0) luma_rt_core->log(luma_rt_self, text);
}

int32_t lumart_finite_f32(const float* v, int n, const char* file, uint32_t line, const char* source) {
    for (int i = 0; i < n; i++) {
        uint32_t bits;
        __builtin_memcpy(&bits, &v[i], 4);
        if (((bits >> 23) & 0xFFu) == 0xFFu) {
            const char* args[1] = {source};
            return lumart_raise(file, line, 917, 1, args);
        }
    }
    return 0;
}

int32_t lumart_finite_f64(double v, const char* file, uint32_t line, const char* source) {
    uint64_t bits;
    __builtin_memcpy(&bits, &v, 8);
    if (((bits >> 52) & 0x7FFu) == 0x7FFu) {
        const char* args[1] = {source};
        return lumart_raise(file, line, 917, 1, args);
    }
    return 0;
}

/* ---------------------------------------------------------------- hooks */

/* ---------------------------------------------------------------- math */

static uint64_t f64_bits(double d) {
    uint64_t u;
    __builtin_memcpy(&u, &d, 8);
    return u;
}
static double f64_from(uint64_t u) {
    double d;
    __builtin_memcpy(&d, &u, 8);
    return d;
}
/* 2^k for -1022 <= k <= 1023. */
static double pow2(int k) {
    return f64_from((uint64_t)(1023 + k) << 52);
}

/* The constants below, but the kernels' and atan's polynomials (fdlibm's), are made by gen_pi.py
   from pi computed with integers. */
static const uint32_t two_over_pi[40] = {
    0xA2F9836Eu, 0x4E441529u, 0xFC2757D1u, 0xF534DDC0u, 0xDB629599u, 0x3C439041u,
    0xFE5163ABu, 0xDEBBC561u, 0xB7246E3Au, 0x424DD2E0u, 0x06492EEAu, 0x09D1921Cu,
    0xFE1DEB1Cu, 0xB129A73Eu, 0xE88235F5u, 0x2EBB4484u, 0xE99C7026u, 0xB45F7E41u,
    0x3991D639u, 0x835339F4u, 0x9C845F8Bu, 0xBDF9283Bu, 0x1FF897FFu, 0xDE05980Fu,
    0xEF2F118Bu, 0x5A0A6D1Fu, 0x6D367ECFu, 0x27CB09B7u, 0x4F463F66u, 0x9E5FEA2Du,
    0x7527BAC7u, 0xEBE5F17Bu, 0x3D0739F7u, 0x8A5292EAu, 0x6BFB5FB1u, 0x1F8D5D08u,
    0x56033046u, 0xFC7B6BABu, 0xF0CFBC20u, 0x9AF4361Du,
};
static const double pio2_hi = 1.5707963267948966, pio2_lo = 6.123233995736766e-17;
static const double pio2_1 = 1.5707963267341256, pio2_1t = 6.077100506506192e-11;
static const double pio2_2 = 6.077100506303966e-11, pio2_2t = 2.0222662487959506e-21;
static const double pio2_3 = 2.0222662487111665e-21, pio2_3t = 8.4784276603689e-32;
static const double pi_hi = 3.141592653589793, pi_lo = 1.2246467991473532e-16;
static const double atan_hi[4] = {0.4636476090008061, 0.7853981633974483, 0.982793723247329, 1.5707963267948966};
static const double atan_lo[4] = {2.2698777452961687e-17, 3.061616997868383e-17, 1.3903311031230998e-17, 6.123233995736766e-17};

/* sin on [-pi/4, pi/4] of x + y (y the reduction's tail; iy 0: none). */
static double k_sin(double x, double y, int iy) {
    const double S1 = -1.66666666666666324348e-01, S2 = 8.33333333332248946124e-03, S3 = -1.98412698298579493134e-04,
                 S4 = 2.75573137070700676789e-06, S5 = -2.50507602534068634195e-08, S6 = 1.58969099521155010221e-10;
    const double z = x * x, w = z * z;
    const double r = S2 + z * (S3 + z * S4) + z * w * (S5 + z * S6);
    const double v = z * x;
    if (iy == 0) return x + v * (S1 + z * r);
    return x - ((z * (0.5 * y - v * r) - y) - v * S1);
}

/* cos on [-pi/4, pi/4] of x + y. */
static double k_cos(double x, double y) {
    const double C1 = 4.16666666666666019037e-02, C2 = -1.38888888888741095749e-03, C3 = 2.48015872894767294178e-05,
                 C4 = -2.75573143513906633035e-07, C5 = 2.08757232129817482790e-09, C6 = -1.13596475577881948265e-11;
    const double z = x * x;
    double w = z * z;
    const double r = z * (C1 + z * (C2 + z * C3)) + w * w * (C4 + z * (C5 + z * C6));
    const double hz = 0.5 * z;
    w = 1.0 - hz;
    return w + (((1.0 - w) - hz) + (z * r - x * y));
}

/* a * b = *p + *e exactly (Dekker; no fma, which would be a call where the target has none). */
static void two_prod(double a, double b, double* p, double* e) {
    const double split = 134217729.0; /* 2^27 + 1 */
    *p = a * b;
    const double ca = split * a, cb = split * b;
    const double ah = ca - (ca - a), al = a - ah;
    const double bh = cb - (cb - b), bl = b - bh;
    *e = ((ah * bh - *p) + ah * bl + al * bh) + al * bl;
}

/* Bits [lo, lo + 64) of the 256-bit p (little-endian 32-bit limbs); bits below 0 read as 0. */
static uint64_t bits64(const uint32_t* p, int lo) {
    uint64_t v = 0;
    for (int i = 63; i >= 0; i--) {
        const int b = lo + i;
        v <<= 1;
        if (b >= 0 && b < 256) v |= (uint64_t)((p[b >> 5] >> (b & 31)) & 1u);
    }
    return v;
}

/* Payne-Hanek: x (finite, |x| >= 2^20 pi/2) as n pi/2 + y[0] + y[1], |y| <= pi/4; gives n. x = m 2^e, and of
   x 2/pi only bits from 2^-(e-1) of 2/pi matter (the ones before give multiples of 4): 192 of them times m. */
static int32_t rem_pio2_large(double x, double* y) {
    const uint64_t u = f64_bits(x);
    const int neg = (int)(u >> 63);
    const int e = (int)((u >> 52) & 0x7FF) - 1075;
    const uint64_t m = (u & 0xFFFFFFFFFFFFFull) | (1ull << 52);
    const int K = 192;
    const int s = e - 1 >= 1 ? e - 1 : 1; /* the first bit of 2/pi taken, from 1 */
    const int shift = s + K - 1 - e;      /* x 2/pi = (m W) / 2^shift */
    uint32_t w[6];                         /* W, little-endian limbs */
    for (int i = 0; i < 6; i++) {
        /* bits s + 32 (5 - i) .. + 31 of 2/pi, the first the most significant */
        uint32_t v = 0;
        const int first = s + 32 * (5 - i);
        for (int b = 0; b < 32; b++) {
            const int k = first + b; /* 1-based bit of 2/pi */
            const uint32_t bit = (k - 1) / 32 < 40 ? (two_over_pi[(k - 1) / 32] >> (31 - (k - 1) % 32)) & 1u : 0u;
            v = (v << 1) | bit;
        }
        w[i] = v;
    }
    uint32_t p[8] = {0, 0, 0, 0, 0, 0, 0, 0};
    const uint64_t ml[2] = {m & 0xFFFFFFFFu, m >> 32};
    for (int i = 0; i < 6; i++) {
        uint64_t c = 0;
        for (int j = 0; j < 2; j++) {
            const uint64_t t = (uint64_t)w[i] * ml[j] + p[i + j] + c;
            p[i + j] = (uint32_t)t;
            c = t >> 32;
        }
        p[i + 2] = (uint32_t)c;
    }
    int32_t n = (int32_t)(bits64(p, shift) & 3u);
    uint64_t hi = bits64(p, shift - 64), lo = bits64(p, shift - 128);
    int below = 0; /* the fraction is past a half: y = f - 1, negative */
    if (hi >> 63) {
        n = (n + 1) & 3;
        below = 1;
        lo = ~lo + 1u;
        hi = ~hi + (lo == 0 ? 1u : 0u);
    }
    double fh = 0.0, fl = 0.0;
    if (hi != 0 || lo != 0) {
        const int lz = hi != 0 ? __builtin_clzll(hi) : 64 + __builtin_clzll(lo);
        if (lz >= 64) {
            hi = lo << (lz - 64);
            lo = 0;
        } else if (lz > 0) {
            hi = (hi << lz) | (lo >> (64 - lz));
            lo <<= lz;
        }
        fh = (double)(hi >> 11) * pow2(-53 - lz);
        fl = (double)(((hi & 0x7FFu) << 42) | (lo >> 22)) * pow2(-106 - lz);
    }
    double ph, pe;
    two_prod(fh, pio2_hi, &ph, &pe);
    const double tail = pe + (fh * pio2_lo + fl * pio2_hi);
    double y0 = ph + tail;
    double y1 = tail - (y0 - ph);
    if (below != neg) {
        y0 = -y0;
        y1 = -y1;
    }
    if (neg) n = -n;
    y[0] = y0;
    y[1] = y1;
    return n;
}

/* x (finite, |x| > pi/4) as n pi/2 + y[0] + y[1]. */
static int32_t rem_pio2(double x, double* y) {
    const uint32_t ix = (uint32_t)(f64_bits(x) >> 32) & 0x7FFFFFFFu;
    if (ix >= 0x413921FBu) return rem_pio2_large(x, y);
    const double toint = 6755399441055744.0, invpio2 = 6.36619772367581382433e-01;
    const double fn = (x * invpio2 + toint) - toint;
    const int32_t n = (int32_t)fn;
    double r = x - fn * pio2_1;
    double w = fn * pio2_1t;
    const int j = (int)(ix >> 20);
    double y0 = r - w;
    int i = j - (int)((f64_bits(y0) >> 52) & 0x7FF);
    if (i > 16) {
        double t = r;
        w = fn * pio2_2;
        r = t - w;
        w = fn * pio2_2t - ((t - r) - w);
        y0 = r - w;
        i = j - (int)((f64_bits(y0) >> 52) & 0x7FF);
        if (i > 49) {
            t = r;
            w = fn * pio2_3;
            r = t - w;
            w = fn * pio2_3t - ((t - r) - w);
            y0 = r - w;
        }
    }
    y[0] = y0;
    y[1] = (r - y0) - w;
    return n;
}

double lumart_sin(double x) {
    const uint32_t ix = (uint32_t)(f64_bits(x) >> 32) & 0x7FFFFFFFu;
    if (ix <= 0x3FE921FBu) {
        if (ix < 0x3E500000u) return x; /* |x| < 2^-26 */
        return k_sin(x, 0.0, 0);
    }
    if (ix >= 0x7FF00000u) return x - x;
    double y[2];
    switch (rem_pio2(x, y) & 3) {
    case 0:
        return k_sin(y[0], y[1], 1);
    case 1:
        return k_cos(y[0], y[1]);
    case 2:
        return -k_sin(y[0], y[1], 1);
    default:
        return -k_cos(y[0], y[1]);
    }
}

double lumart_cos(double x) {
    const uint32_t ix = (uint32_t)(f64_bits(x) >> 32) & 0x7FFFFFFFu;
    if (ix <= 0x3FE921FBu) {
        if (ix < 0x3E46A09Eu) return 1.0; /* |x| < 2^-27 sqrt(2) */
        return k_cos(x, 0.0);
    }
    if (ix >= 0x7FF00000u) return x - x;
    double y[2];
    switch (rem_pio2(x, y) & 3) {
    case 0:
        return k_cos(y[0], y[1]);
    case 1:
        return -k_sin(y[0], y[1], 1);
    case 2:
        return -k_cos(y[0], y[1]);
    default:
        return k_sin(y[0], y[1], 1);
    }
}

static double atan_d(double x) {
    const double aT[11] = {3.33333333333329318027e-01, -1.99999999998764832476e-01, 1.42857142725034663711e-01,
                           -1.11111104054623557880e-01, 9.09088713343650656196e-02, -7.69187620504482999495e-02,
                           6.66107313738753120669e-02, -5.83357013379057348645e-02, 4.97687799461593236017e-02,
                           -3.65315727442169155270e-02, 1.62858201153657823623e-02};
    const uint32_t hx = (uint32_t)(f64_bits(x) >> 32), ix = hx & 0x7FFFFFFFu;
    int id;
    if (ix >= 0x44100000u) { /* |x| >= 2^66 */
        if (x != x) return x + x;
        return (hx >> 31) ? -(atan_hi[3] + atan_lo[3]) : atan_hi[3] + atan_lo[3];
    }
    if (ix < 0x3FDC0000u) { /* |x| < 0.4375 */
        if (ix < 0x3E400000u) return x; /* |x| < 2^-27 */
        id = -1;
    } else {
        x = x < 0 ? -x : x;
        if (ix < 0x3FF30000u) {     /* |x| < 1.1875 */
            if (ix < 0x3FE60000u) { /* 7/16 <= |x| < 11/16 */
                id = 0;
                x = (2.0 * x - 1.0) / (2.0 + x);
            } else { /* 11/16 <= |x| < 19/16 */
                id = 1;
                x = (x - 1.0) / (x + 1.0);
            }
        } else {
            if (ix < 0x40038000u) { /* |x| < 2.4375 */
                id = 2;
                x = (x - 1.5) / (1.0 + 1.5 * x);
            } else { /* 2.4375 <= |x| < 2^66 */
                id = 3;
                x = -1.0 / x;
            }
        }
    }
    const double z = x * x, w = z * z;
    const double s1 = z * (aT[0] + w * (aT[2] + w * (aT[4] + w * (aT[6] + w * (aT[8] + w * aT[10])))));
    const double s2 = w * (aT[1] + w * (aT[3] + w * (aT[5] + w * (aT[7] + w * aT[9]))));
    if (id < 0) return x - x * (s1 + s2);
    const double r = atan_hi[id] - ((x * (s1 + s2) - atan_lo[id]) - x);
    return (hx >> 31) ? -r : r;
}

double lumart_atan2(double y, double x) {
    if (x != x || y != y) return x + y;
    const uint64_t ux = f64_bits(x), uy = f64_bits(y);
    const uint32_t ix = (uint32_t)(ux >> 32) & 0x7FFFFFFFu, iy = (uint32_t)(uy >> 32) & 0x7FFFFFFFu;
    const int m = (int)(uy >> 63) | (int)((ux >> 63) << 1); /* bit 0: y below 0, bit 1: x below 0 */
    if (x == 1.0) return atan_d(y);
    if (y == 0.0) {
        switch (m) {
        case 0:
        case 1:
            return y;
        case 2:
            return pi_hi + pi_lo;
        default:
            return -pi_hi - pi_lo;
        }
    }
    if (x == 0.0) return (m & 1) ? -pio2_hi - pio2_lo : pio2_hi + pio2_lo;
    if (ix == 0x7FF00000u && (uint32_t)ux == 0) {
        if (iy == 0x7FF00000u && (uint32_t)uy == 0) {
            switch (m) {
            case 0:
                return 0.5 * pio2_hi + 0.5 * pio2_lo;
            case 1:
                return -0.5 * pio2_hi - 0.5 * pio2_lo;
            case 2:
                return 1.5 * pio2_hi + 1.5 * pio2_lo;
            default:
                return -1.5 * pio2_hi - 1.5 * pio2_lo;
            }
        }
        switch (m) {
        case 0:
            return 0.0;
        case 1:
            return -0.0;
        case 2:
            return pi_hi + pi_lo;
        default:
            return -pi_hi - pi_lo;
        }
    }
    if (iy == 0x7FF00000u && (uint32_t)uy == 0) return (m & 1) ? -pio2_hi - pio2_lo : pio2_hi + pio2_lo;
    const int k = ((int)iy - (int)ix) >> 20;
    double z;
    int mm = m;
    if (k > 60) { /* |y / x| > 2^60 */
        z = pio2_hi + pio2_lo;
        mm &= 1;
    } else if ((m & 2) && k < -60) { /* |y| / x < -2^-60 */
        z = 0.0;
    } else {
        const double q = y / x;
        z = atan_d(q < 0 ? -q : q);
    }
    switch (mm) {
    case 0:
        return z;
    case 1:
        return -z;
    case 2:
        return pi_hi - (z - pi_lo);
    default:
        return (z - pi_lo) - pi_hi;
    }
}

/* ---------------------------------------------------------------- tasks */

uint64_t lumart_now_frame;
uint64_t lumart_now_step;

typedef struct task_slot {
    lumart_task_head* frame; /* null: free */
    uint32_t generation;
    uint32_t next_free;      /* free slots, a list through this field; 0 ends it */
} task_slot;

static task_slot* task_slots;   /* [0] unused: slot 0 is the empty handle */
static uint32_t task_slots_cap;
static uint32_t task_slots_used; /* slots handed out so far, 0 counted */
static uint32_t task_free;
static lumart_task* task_order; /* start order; an entry whose slot moved on to another generation has ended */
static uint32_t task_n, task_cap;

static lumart_task_head* task_frame(lumart_task t) {
    if (t.slot == 0 || t.slot >= task_slots_used) return 0;
    const task_slot* s = &task_slots[t.slot];
    return s->generation == t.generation ? s->frame : 0;
}

/* The slot freed and the frame given back: every handle to it now matches nothing. */
static void task_end(lumart_task_head* h) {
    task_slot* s = &task_slots[h->slot];
    s->frame = 0;
    s->generation = s->generation == 0xFFFFFFFFu ? 1u : s->generation + 1u;
    s->next_free = task_free;
    task_free = h->slot;
    lumart_free(h, h->size);
}

/* One run of the step function; a task stopped while it ran goes on to its way out now. */
static int32_t task_resume(lumart_task_head* h) {
    const lumart_as was = lumart_caller;
    lumart_caller = h->as;
    h->running = 1;
    int32_t r = h->run(h);
    if (r == LUMART_PARKED && h->stop) r = h->run(h);
    h->running = 0;
    lumart_caller = was;
    if (r != LUMART_PARKED) task_end(h);
    return r;
}

void* lumart_task_new(uint64_t size, lumart_step run, lumart_task* out, const char* file, uint32_t line) {
    out->slot = 0;
    out->generation = 0;
    if (task_n == task_cap) {
        const uint32_t cap = task_cap == 0 ? 16u : task_cap * 2u;
        lumart_task* o = (lumart_task*)lumart_alloc((uint64_t)cap * sizeof(lumart_task));
        if (o == 0) {
            lumart_out_of_memory(file, line, (uint64_t)cap * sizeof(lumart_task));
            return 0;
        }
        if (task_n != 0) __builtin_memcpy(o, task_order, (uint64_t)task_n * sizeof(lumart_task));
        if (task_order != 0) lumart_free(task_order, (uint64_t)task_cap * sizeof(lumart_task));
        task_order = o;
        task_cap = cap;
    }
    if (task_free == 0 && task_slots_used + 1u >= task_slots_cap) {
        const uint32_t cap = task_slots_cap == 0 ? 16u : task_slots_cap * 2u;
        task_slot* n = (task_slot*)lumart_alloc((uint64_t)cap * sizeof(task_slot));
        if (n == 0) {
            lumart_out_of_memory(file, line, (uint64_t)cap * sizeof(task_slot));
            return 0;
        }
        __builtin_memset(n, 0, (uint64_t)cap * sizeof(task_slot));
        if (task_slots != 0) {
            __builtin_memcpy(n, task_slots, (uint64_t)task_slots_cap * sizeof(task_slot));
            lumart_free(task_slots, (uint64_t)task_slots_cap * sizeof(task_slot));
        }
        task_slots = n;
        task_slots_cap = cap;
    }
    lumart_task_head* h = (lumart_task_head*)lumart_alloc(size);
    if (h == 0) {
        lumart_out_of_memory(file, line, size);
        return 0;
    }
    __builtin_memset(h, 0, size);
    uint32_t slot;
    if (task_free != 0) {
        slot = task_free;
        task_free = task_slots[slot].next_free;
    } else {
        if (task_slots_used == 0) task_slots_used = 1;
        slot = task_slots_used++;
        task_slots[slot].generation = 1;
    }
    task_slots[slot].frame = h;
    task_slots[slot].next_free = 0;
    h->run = run;
    h->size = size;
    h->slot = slot;
    h->as = lumart_caller;
    out->slot = slot;
    out->generation = task_slots[slot].generation;
    task_order[task_n++] = *out;
    return h;
}

int32_t lumart_task_first(lumart_task t) {
    lumart_task_head* h = task_frame(t);
    return h == 0 ? 0 : task_resume(h);
}

uint8_t lumart_task_alive(lumart_task t) {
    return (uint8_t)(task_frame(t) != 0);
}

void lumart_task_stop(lumart_task t) {
    lumart_task_head* h = task_frame(t);
    if (h == 0) return;
    h->stop = 1;
    if (!h->running) (void)task_resume(h);
}

uint32_t lumart_tasks_pass(uint64_t frame, uint64_t step) {
    lumart_now_frame = frame;
    lumart_now_step = step;
    return task_n;
}

void lumart_tasks_run(uint32_t n) {
    /* No compaction happens between the pass's start and here, so the first n are the ones counted then. */
    if (n > task_n) n = task_n;
    for (uint32_t i = 0; i < n; i++) {
        lumart_task_head* h = task_frame(task_order[i]);
        if (h != 0 && !h->running) (void)task_resume(h);
    }
    /* The ended ones out, the order kept (tasks started during the pass are past n and kept too). */
    uint32_t k = 0;
    for (uint32_t i = 0; i < task_n; i++) {
        if (task_frame(task_order[i]) != 0) task_order[k++] = task_order[i];
    }
    task_n = k;
}

/* Results kept by tasks that ended with a value, until their pending is read (docs/modding/language/language.md, section 11.1a): a short list. */
typedef struct task_result {
    lumart_task t;
    void* v;
    const lumart_desc* d;
} task_result;

static task_result* task_results;
static uint32_t task_results_n;
static uint32_t task_results_cap;

static void task_result_free(uint32_t i) {
    task_result* r = &task_results[i];
    if (r->d->rel != 0) r->d->rel(r->v);
    lumart_free(r->v, r->d->size);
    task_results[i] = task_results[--task_results_n];
}

int32_t lumart_task_result_keep(void* frame, void* v, const lumart_desc* d, const char* file, uint32_t line) {
    const lumart_task_head* h = (const lumart_task_head*)frame;
    if (task_results_n == task_results_cap) {
        const uint32_t cap = task_results_cap == 0 ? 8u : task_results_cap * 2u;
        task_result* n = (task_result*)lumart_alloc((uint64_t)cap * sizeof(task_result));
        if (n == 0) {
            lumart_out_of_memory(file, line, (uint64_t)cap * sizeof(task_result));
            return 1;
        }
        if (task_results_n != 0) __builtin_memcpy(n, task_results, (uint64_t)task_results_n * sizeof(task_result));
        if (task_results != 0) lumart_free(task_results, (uint64_t)task_results_cap * sizeof(task_result));
        task_results = n;
        task_results_cap = cap;
    }
    void* p = lumart_alloc(d->size);
    if (p == 0) {
        lumart_out_of_memory(file, line, d->size);
        return 1;
    }
    /* The counts move with the bytes: the frame's place is zeroed, so its way out gives none back. */
    __builtin_memcpy(p, v, d->size);
    __builtin_memset(v, 0, d->size);
    task_result* r = &task_results[task_results_n++];
    r->t.slot = h->slot;
    r->t.generation = task_slots[h->slot].generation;
    r->v = p;
    r->d = d;
    return 0;
}

static int32_t pending_poll(lumart_task t, void* out, int32_t (*in)(void*, const void*, const char*, uint32_t, const void*), const char* file, uint32_t line) {
    if (task_frame(t) != 0) return 0;
    for (uint32_t i = 0; i < task_results_n; i++) {
        if (lumart_task_eq(task_results[i].t, t)) {
            int32_t r = 0;
            if (in != 0) {
                r = in(out, task_results[i].v, file, line, (const void*)lumart_ref_get);
            } else {
                __builtin_memcpy(out, task_results[i].v, task_results[i].d->size);
                if (task_results[i].d->ret != 0) task_results[i].d->ret(out);
            }
            task_result_free(i);
            return r != 0 ? 3 : 1;
        }
    }
    return 2;
}

const lumart_pending_ops lumart_pending_own = {pending_poll, lumart_task_stop};

/* reserved[0] of a task's head: 1 when a call watches its end (a result-less task's pending). */
void lumart_task_watch(lumart_task t) {
    lumart_task_head* h = task_frame(t);
    if (h != 0) h->reserved[0] = 1;
}

void lumart_task_done(void* frame, const char* file, uint32_t line) {
    static const lumart_desc unit = {1, 0, 0, 0, 0};
    const lumart_task_head* h = (const lumart_task_head*)frame;
    if (h->reserved[0] == 0) return;
    uint8_t v = 0;
    /* Out of memory is reported there; the pending then gives nothing, as a stopped task's. */
    (void)lumart_task_result_keep(frame, &v, &unit, file, line);
}

void lumart_tasks_stop_all(void) {
    /* A task's way out may start another: rounds until none is alive. */
    for (uint32_t round = 0; round < 8; round++) {
        uint32_t alive = 0;
        for (uint32_t i = 0; i < task_n; i++) {
            if (task_frame(task_order[i]) != 0) {
                alive++;
                lumart_task_stop(task_order[i]);
            }
        }
        if (alive == 0) break;
    }
    for (uint32_t i = 0; i < task_n; i++) {
        lumart_task_head* h = task_frame(task_order[i]);
        if (h != 0) {
            lumart_log("lumart: a task did not end when the mod stopped; its frame is freed");
            task_end(h);
        }
    }
    if (task_order != 0) lumart_free(task_order, (uint64_t)task_cap * sizeof(lumart_task));
    if (task_slots != 0) lumart_free(task_slots, (uint64_t)task_slots_cap * sizeof(task_slot));
    /* Results no pending read: given up with the mod. */
    while (task_results_n != 0) task_result_free(task_results_n - 1);
    if (task_results != 0) lumart_free(task_results, (uint64_t)task_results_cap * sizeof(task_result));
    task_results = 0;
    task_results_cap = 0;
    task_order = 0;
    task_slots = 0;
    task_n = task_cap = task_slots_cap = task_slots_used = task_free = 0;
}

static void wait_negative(double n, const char* file, uint32_t line) {
    char buf[32];
    /* The whole part (the value rounded toward zero; -0.5 shows as -0). */
    const double m = -n;
    const uint64_t w = m >= 18446744073709549568.0 ? 0xFFFFFFFFFFFFFFFFull : (uint64_t)m;
    char* p = digits_u64(buf + sizeof(buf), w);
    *--p = '-';
    const char* args[1] = {p};
    (void)lumart_raise(file, line, 919, 1, args);
}

uint64_t lumart_wait_frames(int64_t n, const char* file, uint32_t line) {
    if (n < 0) {
        wait_negative((double)n, file, line);
        n = 0;
    }
    return (uint64_t)n > 0xFFFFFFFFFFFFFFFFull - lumart_now_frame ? 0xFFFFFFFFFFFFFFFFull : lumart_now_frame + (uint64_t)n;
}

uint64_t lumart_wait_seconds(double n, const char* file, uint32_t line) {
    uint64_t steps = 1;
    if (n != n) {
        const char* args[1] = {"wait"};
        (void)lumart_raise(file, line, 917, 1, args);
    } else if (n < 0) {
        wait_negative(n, file, line);
    } else if (n * 60.0 >= 1.5) {
        const double s = n * 60.0 + 0.5;
        steps = s >= 18446744073709549568.0 ? 0xFFFFFFFFFFFFFFFFull : (uint64_t)s;
    }
    /* saturated: an infinite or huge wait lasts to the last step, never wraps to now */
    return steps > 0xFFFFFFFFFFFFFFFFull - lumart_now_step ? 0xFFFFFFFFFFFFFFFFull : lumart_now_step + steps;
}

int32_t lumart_on_game_thread(const char* file, uint32_t line, const char* function) {
    if (luma_rt_core == 0 || luma_rt_core->game_thread()) return 1;
    const char* args[1] = {function};
    lumart_raise(file, line, 901, 1, args);
    return 0;
}

/* ---------------------------------------------------------------- objects (docs/modding/language/classes.md, "make") */

int32_t lumart_make(lumart_text name, lm_vec3 at, lm_vec3 rot, lm_vec3 scale, lumart_list args, uint8_t dead, int64_t count,
                    int32_t class_number, const lumart_desc* d, lumart_list* out, const char* file, uint32_t line) {
    const char* c = lumart_text_cstr(name, file, line, "make", "name");
    if (c == 0) return 1;
    const uint64_t na = lumart_list_n(args);
    if (na > 8) return lumart_wrong_call(file, line, "make", "place.args", "more than 8 numbers", "8 numbers or fewer");
    if (luma_rt_core == 0) return 0;
    /* Outside 1 to 1024 the kernel refuses (-4, said with the name); asked with no place, as nothing is made. */
    if (count < 1 || count > 1024) {
        (void)luma_rt_core->make(luma_rt_self, c, 0, 0, dead ? LUMA_MAKE_DEAD : 0u, 0);
        return 0;
    }
    const uint32_t n = (uint32_t)count;
    const uint64_t size = (uint64_t)n * (sizeof(LumaMakePlace) + sizeof(void*));
    LumaMakePlace* places = (LumaMakePlace*)lumart_alloc(size);
    if (places == 0) return lumart_out_of_memory(file, line, size);
    void** objects = (void**)(places + n);
    LumaMakePlace p;
    p.size = (uint32_t)sizeof(LumaMakePlace);
    p.pos[0] = at.x, p.pos[1] = at.y, p.pos[2] = at.z;
    p.rot[0] = rot.x, p.rot[1] = rot.y, p.rot[2] = rot.z;
    p.scale[0] = scale.x, p.scale[1] = scale.y, p.scale[2] = scale.z;
    for (uint32_t i = 0; i < 8; i++) p.args[i] = i < na ? *(const int32_t*)lumart_list_at(args, i) : -1;
    for (uint32_t i = 0; i < n; i++) {
        places[i] = p;
        objects[i] = 0;
    }
    const int32_t made = luma_rt_core->make(luma_rt_self, c, n, places, dead ? LUMA_MAKE_DEAD : 0u, objects);
    int32_t r = 0;
    /* The objects made are the non-null ones (fewer than asked when a creator makes nothing). */
    for (uint32_t i = 0; made > 0 && i < n && r == 0; i++) {
        if (objects[i] == 0) continue;
        void* part = (void*)luma_rt_core->as_class(luma_rt_self, objects[i], class_number);
        if (part != 0) r = lumart_list_push(out, d, &part, file, line);
    }
    lumart_free(places, size);
    return r;
}

int32_t lumart_set_number(lumart_text name, lumart_text signature, uint32_t place, double value, uint8_t* out, const char* file,
                          uint32_t line) {
    *out = 0;
    const char* n = lumart_text_cstr(name, file, line, "set_number", "name");
    if (n == 0) return 1;
    const char* s = "";
    if (signature != 0) {
        s = lumart_text_cstr(signature, file, line, "set_number", "signature");
        if (s == 0) return 1;
    }
    if (luma_rt_core == 0) return 0;
    /* The calling mod's number: put back when that mod goes (a library acts for its caller, as caller() says). */
    const LumaMod* mod = lumart_caller.mod != 0 ? lumart_caller.mod : luma_rt_self;
    *out = luma_rt_core->set_number(mod, n, s, place, value) == 0;
    return 0;
}

int32_t lumart_set_table(const char* name, uint32_t offset, const void* bytes, uint32_t size, uint8_t* out, const char* file,
                         uint32_t line) {
    (void)file, (void)line;
    *out = 0;
    if (luma_rt_core == 0) return 0;
    /* The calling mod's bytes: put back when that mod goes (a library acts for its caller, as caller() says). */
    const LumaMod* mod = lumart_caller.mod != 0 ? lumart_caller.mod : luma_rt_self;
    *out = luma_rt_core->set_table(mod, name, offset, bytes, size) == 0;
    return 0;
}

int32_t lumart_table(const char* name, uint32_t offset, void* out, uint32_t size, uint8_t* some, const char* file,
                     uint32_t line) {
    (void)file, (void)line;
    *some = 0;
    if (luma_rt_core == 0) return 0;
    const LumaMod* mod = lumart_caller.mod != 0 ? lumart_caller.mod : luma_rt_self;
    LumaTable t;
    if (luma_rt_core->table(mod, name, &t) != 0 || t.data == 0 || offset > t.size || size > t.size - offset) return 0;
    const uint8_t* from = (const uint8_t*)t.data + offset;
    uint8_t* to = (uint8_t*)out;
    for (uint32_t i = 0; i < size; i++) to[i] = from[i];
    *some = 1;
    return 0;
}

int32_t lumart_alias_class(lumart_text name, lumart_text class_name, lumart_list args, uint8_t* out, const char* file,
                           uint32_t line) {
    *out = 0;
    const char* n = lumart_text_cstr(name, file, line, "alias_class", "name");
    if (n == 0) return 1;
    const char* c = lumart_text_cstr(class_name, file, line, "alias_class", "class_name");
    if (c == 0) return 1;
    const uint64_t na = lumart_list_n(args);
    if (na > 8) return lumart_wrong_call(file, line, "alias_class", "args", "more than 8 numbers", "8 numbers or fewer");
    if (luma_rt_core == 0) return 0;
    int32_t a[8];
    for (uint32_t i = 0; i < 8; i++) a[i] = i < na ? *(const int32_t*)lumart_list_at(args, i) : -1;
    /* The calling mod's alias: it goes when that mod goes (a library acts for its caller, as caller() says). */
    const LumaMod* mod = lumart_caller.mod != 0 ? lumart_caller.mod : luma_rt_self;
    *out = luma_rt_core->alias_class(mod, n, c, a) == 0;
    return 0;
}

/* A text of the game's, new, into *place (its old value released). */
static int32_t placement_text(lumart_text* place, const char* s, const char* file, uint32_t line) {
    lumart_text t = 0;
    if (lumart_text_from_game(&t, s, file, line) != 0) return 1;
    lm_rel_lumart_text(place);
    *place = t;
    return 0;
}

int32_t lumart_placement_of(const void* object, uint8_t* some, lumart_text* stage, lumart_text* zone, lumart_text* layer,
                            uint32_t* index, const char* file, uint32_t line) {
    *some = 0;
    if (object == 0 || luma_rt_core == 0) return 0;
    LumaPlacement p;
    p.size = (uint32_t)sizeof(LumaPlacement);
    if (luma_rt_core->placement_of(luma_rt_self, object, &p) != 0) return 0;
    if (placement_text(stage, p.stage, file, line) != 0 || placement_text(zone, p.zone, file, line) != 0 ||
        placement_text(layer, p.layer, file, line) != 0)
        return 1;
    *index = p.index;
    *some = 1;
    return 0;
}

int32_t lumart_placed_at(lumart_text stage, lumart_text zone, lumart_text layer, uint32_t index, int32_t class_number,
                         void** out, const char* file, uint32_t line) {
    *out = 0;
    const char* s = lumart_text_cstr(stage, file, line, "placed_at", "place.stage");
    if (s == 0) return 1;
    const char* z = lumart_text_cstr(zone, file, line, "placed_at", "place.zone");
    if (z == 0) return 1;
    const char* l = lumart_text_cstr(layer, file, line, "placed_at", "place.layer");
    if (l == 0) return 1;
    if (luma_rt_core == 0) return 0;
    LumaPlacement p;
    p.size = (uint32_t)sizeof(LumaPlacement);
    p.stage = s[0] != 0 ? s : 0;
    p.zone = z;
    p.layer = l;
    p.index = index;
    void* o = luma_rt_core->placed_at(luma_rt_self, &p);
    if (o != 0) *out = (void*)luma_rt_core->as_class(luma_rt_self, o, class_number);
    return 0;
}

int32_t lumart_make_gravity(const char* class_name, float range, int32_t priority, uint32_t types, uint8_t inverse,
                            int32_t class_number, void** out, const char* file, uint32_t line) {
    (void)file, (void)line;
    *out = 0;
    if (luma_rt_core == 0) return 0;
    LumaGravityPlace p;
    p.size = (uint32_t)sizeof(LumaGravityPlace);
    p.range = range;
    p.priority = priority;
    p.types = types;
    p.inverse = inverse ? 1u : 0u;
    void* o = luma_rt_core->make_gravity(luma_rt_self, class_name, &p);
    if (o != 0) *out = (void*)luma_rt_core->as_class(luma_rt_self, o, class_number);
    return 0;
}

/* ---------------------------------------------------------------- the end

   In the mod's finish, after the game's grace (no thread is inside one of its hooks) and after luma.h's exits: what
   the runtime keeps for the mod's life, then every pool page. Nothing of the mod runs after it; a start in a later
   load begins from nothing. */
void lumart_end(void) {
    if (luma_rt_core == 0) return;
    if (task_order != 0 || task_slots != 0 || task_results != 0) lumart_tasks_stop_all();
    for (uint32_t i = 0; i < fn_kept_n; i++) lm_rel_lumart_fn(&fn_kept[i]);
    if (fn_kept != 0) lumart_free(fn_kept, (uint64_t)fn_kept_cap * sizeof(lumart_fn));
    fn_kept = 0;
    fn_kept_n = fn_kept_cap = 0;
    /* The UTF-16 copies first: each one's size is its kept text's length, read before that text goes. */
    for (uint64_t i = 0; i < k16_cap; i++) {
        if (k16_keys[i] == 0) continue;
        uint64_t n = 0;
        while (k16_keys[i][n] != 0) n++;
        lumart_free(k16_vals[i], (n + 1) * sizeof(uint16_t));
    }
    if (k16_keys != 0) lumart_free((void*)k16_keys, k16_cap * sizeof(const char*));
    if (k16_vals != 0) lumart_free(k16_vals, k16_cap * sizeof(uint16_t*));
    k16_keys = 0;
    k16_vals = 0;
    k16_used = k16_cap = 0;
    for (uint64_t i = 0; i < keep_cap; i++) {
        if (keep_table[i] != 0) lumart_free(keep_table[i], text_block(keep_table[i]->len));
    }
    if (keep_table != 0) lumart_free(keep_table, keep_cap * sizeof(lumart_text));
    keep_table = 0;
    keep_used = keep_cap = 0;
    /* The handle table: the kernel took its watch hooks out before the stop. */
    if (refs != 0) lumart_free(refs, (uint64_t)refs_cap * sizeof(ref_slot));
    if (refs_hash != 0) lumart_free(refs_hash, (uint64_t)refs_hash_cap * 4);
    refs = 0;
    refs_hash = 0;
    refs_cap = refs_used = refs_live = refs_free = refs_hash_cap = 0;
    refs_hooked = 0;
    lumart_scratch_free_all();
    while (pool_pages != 0) {
        char* before = *(char**)pool_pages;
        luma_rt_core->free(luma_rt_self, pool_pages);
        pool_pages = before;
    }
    for (int c = 0; c < POOL_CLASSES; c++) pool_list[c] = 0;
    pool_page = 0;
    pool_used = 0;
}
