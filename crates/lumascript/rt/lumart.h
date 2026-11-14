/* SPDX-License-Identifier: GPL-3.0-only */
/* lumart: the run-time support a compiled LumaScript mod links (docs/modding/language/compiled-form.md, section 7). One C17 source
   set, compiled into each mod for each platform, freestanding: no C library, no system call, nothing imported. Its
   memory is the core's per-mod heap (LumaCore.alloc), its reports go through LumaCore.report.

   Every lumart function that can fail returns int32_t: 0 ok, 1 a script error was reported (the caller ends that run,
   abi section 6). Names are lumart_*; every symbol is hidden (the build's -fvisibility=hidden). */
#ifndef LUMART_H
#define LUMART_H

#include <stddef.h> /* size_t: the generated C's index and size arithmetic */
#include <stdint.h>
#include <luma.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The core and the mod, set by luma.h's begin before start (LUMA_LIFE). */
extern const LumaCore* luma_rt_core;
extern const LumaMod* luma_rt_self;

/* ---------------------------------------------------------------- the language's value types (docs/modding/language/compiled-form.md, section 5) */

typedef struct lm_vec2 { float x, y; } lm_vec2;
typedef struct lm_vec3 { float x, y, z; } lm_vec3;
typedef struct lm_vec4 { float x, y, z, w; } lm_vec4;
typedef struct lm_quat { float x, y, z, w; } lm_quat;
typedef struct lm_mat34 { float m[3][4]; } lm_mat34;
typedef struct lm_color { float r, g, b, a; } lm_color;

/* ---------------------------------------------------------------- memory (docs/modding/language/compiled-form.md, section 7.1) */

/* `size` bytes from the mod's heap, 16-aligned; null when none is left (the caller reports S0911). Blocks up to 512
   bytes come from lumart's pools. `lumart_free` takes the size the block was asked with. */
void* lumart_alloc(uint64_t size);
void lumart_free(void* p, uint64_t size);

/* ---------------------------------------------------------------- script errors (docs/modding/language/compiled-form.md, section 6, draft 3 addition) */

/* Reports the run-time code `code` (S0901...) at `file`:`line` with its arguments (UTF-8, at most 8), through
   LumaCore.report; the kernel says it once and counts repeats. Returns 1, so a caller writes `return lumart_raise(...)`.*/
int32_t lumart_raise(const char* file, uint32_t line, uint32_t code, uint32_t argc, const char* const* args);

/* While a library mod's exported function runs for another mod (docs/modding/language/compiled-form.md): the calling mod and
   the call's place in its source. Every report lumart_raise makes then is the caller's, at that place (lumakit rule
   8), and the export returns 2. `mod` 0: this mod's own code, reported at its own place. The export wrapper sets it on
   entry and puts the one it found back on return, so a library calling a library reports for the first caller's
   call. Game thread only, as every script. */
typedef struct lumart_as {
    const LumaMod* mod;
    const char* file;
    uint32_t line;
} lumart_as;
extern lumart_as lumart_caller;

/* The common ones, each with its arguments written as text. */
/* S0914, a wrong call (lumakit rule 8): "<fn>: <param> is <value>; it must be <must>". */
int32_t lumart_wrong_call(const char* file, uint32_t line, const char* fn, const char* param, const char* value, const char* must);
/* S0914 with lengths: "<fn>: <param> is <n>; it must be <want>". */
int32_t lumart_wrong_length(const char* file, uint32_t line, const char* fn, const char* param, uint64_t n, uint64_t want);
/* S0914 for a length not a whole number of items: "<fn>: <param> is <n>; it must be <of> x <k>" (`samples.len()` is 7;
   it must be `frames x 2`). */
int32_t lumart_wrong_multiple(const char* file, uint32_t line, const char* fn, const char* param, uint64_t n, const char* of,
                              uint64_t k);
int32_t lumart_overflow(const char* file, uint32_t line, const char* type, const char* source);
int32_t lumart_div_zero(const char* file, uint32_t line, const char* source);
int32_t lumart_out_of_memory(const char* file, uint32_t line, uint64_t size);

/* ---------------------------------------------------------------- text being made (log lines, facts, interpolation) */

/* A growing UTF-8 buffer on the mod's heap (or in the scratch, below), always NUL-ended when `ok` and written to. A
   builder whose memory ran out keeps `ok` 0 and its writes do nothing; lumart_sb_done then reports S0911 at the
   caller's line. */
typedef struct lumart_sb {
    char* p;
    uint64_t len, cap;
    int32_t ok;
    int32_t scratch; /* 1: its bytes are in the scratch, never freed by it */
    uint64_t wanted; /* the size that could not be had, for the report */
} lumart_sb;

/* ---------------------------------------------------------------- scratch (docs/modding/language/language.md, section 6.4, docs/modding/language/compiled-form.md, section 7.2)

   Text made for one use and dropped (a log line, an interpolated text before it is copied, a parameter the game
   reads only during the call) is written in the mod's scratch instead of a heap block of its own: chunks of the mod's
   memory, 64 KiB first and each next one twice the one before (or as large as one text needs), kept while the mod
   runs. Uses nest as a stack: the code takes lumart_scratch_mark before and gives back with lumart_scratch_release
   once the text is used (after the game call), and every function that uses it gives back to its own mark on its way
   out, so a script error between the two loses nothing. Game thread only, as all script code (D4.3 A-2). */
typedef lumart_sb lumart_scratch_sb; /* a builder in the scratch: its own C type, so a temporary is never both */
uint64_t lumart_scratch_mark(void);
void lumart_scratch_release(uint64_t mark);
/* `b` an empty builder whose bytes go to the scratch's top. */
void lumart_sb_scratch(lumart_sb* b);
/* The builder's text as a `const char *` for the game: null when it holds a NUL byte (S0918 naming `function` and
   `param`); "" when nothing was written. */
const char* lumart_sb_cstr(lumart_sb* b, const char* file, uint32_t line, const char* function, const char* param);
/* `p` or "" for a builder never written to (its `p` is null). */
static inline const char* lumart_sb_c(const lumart_sb* b) {
    return b->p != 0 ? b->p : "";
}
/* Every chunk back to the mod's memory (the mod stops). */
void lumart_scratch_free_all(void);
/* In the mod's finish, after luma.h's: the runtime's tables (kept functions and texts, their UTF-16 copies, the
   handle table, the task lists) and every pool page given back. */
void lumart_end(void);

void lumart_sb_init(lumart_sb* b);
void lumart_sb_free(lumart_sb* b);
void lumart_sb_bytes(lumart_sb* b, const char* s, uint64_t n);
void lumart_sb_str(lumart_sb* b, const char* s);
void lumart_sb_i64(lumart_sb* b, int64_t v);
void lumart_sb_u64(lumart_sb* b, uint64_t v);
/* The shortest decimal that reads back as the same float (f32 or f64), in JSON's number form (`1`, `0.1`, `-2.5`,
   `1e+30`, `5e-324`); -0 is `-0`. NaN and infinities are written `nan`, `inf`, `-inf` (log text only: a fact or json
   refuses them before, D7.4). */
void lumart_sb_f32(lumart_sb* b, float v);
void lumart_sb_f64(lumart_sb* b, double v);
void lumart_sb_bool(lumart_sb* b, int v);
/* A JSON string: `"..."` with ", \ and control characters escaped. */
void lumart_sb_json_str(lumart_sb* b, const char* s, uint64_t n);
void lumart_sb_vec(lumart_sb* b, const float* v, int n); /* [x, y(, z(, w))] */
/* 0 when the text is complete; else reports S0911 at file:line and gives 1. */
int32_t lumart_sb_done(lumart_sb* b, const char* file, uint32_t line);

/* ---------------------------------------------------------------- text (docs/modding/language/language.md, sections 3, 6.3; docs/modding/language/compiled-form.md, section 5)

   A text is a counted, immutable block of UTF-8, NUL-ended; null is the empty text. Every C variable of a counted
   type owns one count: a store retains the new value before it releases the old (`lm_cp_<type>`), and a function
   releases its variables on its way out (`lm_rel_<type>`). A block counted LUMART_FOREVER is a literal in the
   library's image or a kept copy (docs/modding/language/compiled-form.md): never counted, never freed. */

#define LUMART_FOREVER 0xFFFFFFFFu

typedef struct lumart_tb {
    uint32_t rc;
    uint32_t reserved;
    uint64_t len;
    char b[];
} lumart_tb;

typedef lumart_tb* lumart_text;

/* A literal: `static const lumart_lit<N> lm_s<k> = {LUMART_FOREVER, 0, len, "bytes"};` read as a text. */
#define LUMART_LIT(name) ((lumart_text)(void*)&(name))

void lumart_text_drop(lumart_text t);

static inline void lm_ret_lumart_text(const lumart_text* p) {
    if (*p != 0 && (*p)->rc != LUMART_FOREVER) (*p)->rc++;
}

static inline void lm_rel_lumart_text(lumart_text* p) {
    lumart_text t = *p;
    *p = 0;
    if (t != 0 && t->rc != LUMART_FOREVER && --t->rc == 0) lumart_text_drop(t);
}

static inline void lm_cp_lumart_text(lumart_text* d, lumart_text s) {
    lm_ret_lumart_text(&s);
    lumart_text old = *d;
    *d = s;
    lm_rel_lumart_text(&old);
}

/* A function value (docs/modding/language/language.md, section 5.4): a counted block; `code` is its C function, `int32_t f(lumart_fn self, P..., R*)`,
   called with the block itself first. A closure's block carries its captures after this head; `drop`, the maker's
   code, gives their counts back and frees the block (`size` bytes); a closure without captures and a mod function used as a value
   are static blocks counted LUMART_FOREVER. */
typedef struct lumart_fn_block {
    uint32_t rc;
    uint32_t size;
    void (*drop)(struct lumart_fn_block* b);
    const void* code;
} lumart_fn_block;

typedef lumart_fn_block* lumart_fn;

static inline void lm_ret_lumart_fn(const lumart_fn* p) {
    if (*p != 0 && (*p)->rc != LUMART_FOREVER) (*p)->rc++;
}

static inline void lm_rel_lumart_fn(lumart_fn* p) {
    lumart_fn f = *p;
    *p = 0;
    /* The maker's own drop gives the captures back and frees the block: a value made by one mod and let go by another
       (a library holding a caller's closure) is freed into its maker's memory. */
    if (f != 0 && f->rc != LUMART_FOREVER && --f->rc == 0) f->drop(f);
}

static inline void lm_cp_lumart_fn(lumart_fn* d, lumart_fn s) {
    lm_ret_lumart_fn(&s);
    lumart_fn old = *d;
    *d = s;
    lm_rel_lumart_fn(&old);
}

/* A function value given to a service that calls it back (draw.at's fn): its count taken once, kept until the mod
   ends (the service may call it until replaced, and a call site cannot know when another replaced it). The same block
   twice is kept once. 0, or 1 when no memory is left (S0911 at file:line). */
int32_t lumart_fn_keep(lumart_fn f, const char* file, uint32_t line);

/* A colour as the services take it (LumaColor, 0xRRGGBBAA): each part times 255, rounded, held to 0-255 (NaN: 0). */
static inline uint32_t lumart_unit_byte(float x) {
    if (!(x > 0.0f)) return 0u;
    if (x >= 1.0f) return 255u;
    return (uint32_t)(x * 255.0f + 0.5f);
}

static inline uint32_t lumart_color_rgba8(lm_color c) {
    return (lumart_unit_byte(c.r) << 24) | (lumart_unit_byte(c.g) << 16) | (lumart_unit_byte(c.b) << 8) | lumart_unit_byte(c.a);
}

static inline lm_color lumart_color_of_rgba8(uint32_t v) {
    lm_color c;
    c.r = (float)((v >> 24) & 255u) / 255.0f;
    c.g = (float)((v >> 16) & 255u) / 255.0f;
    c.b = (float)((v >> 8) & 255u) / 255.0f;
    c.a = (float)(v & 255u) / 255.0f;
    return c;
}

/* A text of `text "Key"` made before the game's messages exist is a row: its block's `reserved` is LUMART_TEXT_ROW
   and its bytes are the key. Every read of it goes through lumart_text_row, which reads the game's message the first
   time the messages exist and is the key until then. LUMART_TEXT_ROW_KEY: a row settled as its key (the game has
   no message of it, or the names it is read through do not bind). */
#define LUMART_TEXT_ROW 1u
#define LUMART_TEXT_ROW_KEY 2u

lumart_text lumart_text_row(lumart_text t);

static inline const char* lumart_text_p(lumart_text t) {
    if (t != 0 && t->reserved == LUMART_TEXT_ROW) t = lumart_text_row(t);
    return t != 0 ? t->b : "";
}

static inline uint64_t lumart_text_n(lumart_text t) {
    if (t != 0 && t->reserved == LUMART_TEXT_ROW) t = lumart_text_row(t);
    return t != 0 ? t->len : 0;
}

/* A new text of the `n` bytes at `p` into `*out` (its old value released). Out of memory: S0911 at file:line, 1. */
int32_t lumart_text_make(lumart_text* out, const char* p, uint64_t n, const char* file, uint32_t line);
/* The builder's text into `*out`; the builder emptied. A builder whose memory ran out: S0911, 1. */
int32_t lumart_text_from_sb(lumart_text* out, lumart_sb* b, const char* file, uint32_t line);
/* `len()`: the byte count as an int; a text longer than an int holds is S0904 naming `source`. */
int32_t lumart_text_len(int32_t* out, lumart_text t, const char* file, uint32_t line, const char* source);
uint8_t lumart_text_eq(lumart_text a, lumart_text b);
/* Byte order: -1, 0 or 1 (docs/modding/language/language.md, section 3: `<` on text is by bytes, the same on every platform). */
int32_t lumart_text_cmp(lumart_text a, lumart_text b);
uint8_t lumart_text_starts(lumart_text t, lumart_text x);
uint8_t lumart_text_ends(lumart_text t, lumart_text x);
/* `find(x)`: the byte offset of the first `x` in `t`, or -1 (none). An empty `x` is found at 0. */
int64_t lumart_text_find(lumart_text t, lumart_text x);
/* `slice(from, to)`: bytes [from, to). Outside the text: S0910; off a code point boundary: S0909. */
int32_t lumart_text_slice(lumart_text* out, lumart_text t, int64_t from, int64_t to, const char* file, uint32_t line);
/* `hash32(t)`: FNV-1a, 32 bits, over the text's UTF-8 bytes. */
uint32_t lumart_hash32(lumart_text t);
/* ---------------------------------------------------------------- containers (docs/modding/language/language.md, sections 3, 6.3; docs/modding/language/compiled-form.md, section 5)

   list<T>, map<K, V>, set<T>: counted blocks, copy on write; null is the empty container. Each holds the descriptor
   of its element type (written by the generator in lm.h, one per C type): the element's size, how a count is taken
   and given back (null when nothing in it is counted), == and the order (null when the type has none). A container
   never holds another's address, only values, so no cycle can be made (docs/modding/language/language.md, section 6.3). Maps and sets keep their keys
   sorted by the key type's order: lookups by binary search, and keys(), values() and `for` in key order on every
   platform. */

typedef struct lumart_desc {
    uint64_t size;
    void (*ret)(void* p);
    void (*rel)(void* p);
    uint8_t (*eq)(const void* a, const void* b);
    int32_t (*cmp)(const void* a, const void* b);
} lumart_desc;

typedef struct lumart_lb {
    uint32_t rc;
    uint32_t reserved;
    uint64_t len;
    uint64_t cap;
    const lumart_desc* d;
} lumart_lb;

typedef lumart_lb* lumart_list;

void lumart_list_drop(lumart_list l);

static inline void lm_ret_lumart_list(const lumart_list* p) {
    if (*p != 0) (*p)->rc++;
}

static inline void lm_rel_lumart_list(lumart_list* p) {
    lumart_list l = *p;
    *p = 0;
    if (l != 0 && --l->rc == 0) lumart_list_drop(l);
}

static inline void lm_cp_lumart_list(lumart_list* d, lumart_list s) {
    lm_ret_lumart_list(&s);
    lumart_list old = *d;
    *d = s;
    lm_rel_lumart_list(&old);
}

static inline uint64_t lumart_list_n(lumart_list l) {
    return l != 0 ? l->len : 0;
}

/* Element i's storage (i < len). */
static inline void* lumart_list_at(lumart_list l, uint64_t i) {
    return (char*)(l + 1) + i * l->d->size;
}

/* Makes *l this variable's own (copied when shared) with room for `more` elements; S0911 when there is no memory. */
int32_t lumart_list_own(lumart_list* l, const lumart_desc* d, uint64_t more, const char* file, uint32_t line);
/* Appends a copy of *v (its count taken). */
int32_t lumart_list_push(lumart_list* l, const lumart_desc* d, const void* v, const char* file, uint32_t line);
/* Inserts a copy of *v before element `at` (at <= len). */
int32_t lumart_list_insert(lumart_list* l, const lumart_desc* d, uint64_t at, const void* v, const char* file, uint32_t line);
/* Removes element `at` (< len), giving its count back: call after reading it. */
int32_t lumart_list_remove(lumart_list* l, const lumart_desc* d, uint64_t at, const char* file, uint32_t line);
uint8_t lumart_list_eq(lumart_list a, lumart_list b, const lumart_desc* d);
/* The first index holding a value equal to *v, or -1. */
int64_t lumart_list_find(lumart_list l, const lumart_desc* d, const void* v);
/* ---------------------------------------------------------------- bytes (docs/modding/language/language.md, section 3; docs/modding/language/compiled-form.md, section 5) */
/* A list block of uint8_t elements (lumart_u8): counted, copy on write, its bytes contiguous after the header. */
typedef lumart_list lumart_bytes;
extern const lumart_desc lumart_u8;
static inline void lm_ret_lumart_bytes(const lumart_bytes* p) {
    lm_ret_lumart_list(p);
}
static inline void lm_rel_lumart_bytes(lumart_bytes* p) {
    lm_rel_lumart_list(p);
}
static inline void lm_cp_lumart_bytes(lumart_bytes* d, lumart_bytes s) {
    lm_cp_lumart_list(d, s);
}
static inline uint8_t* lumart_bytes_p(lumart_bytes b) {
    return (uint8_t*)(b + 1);
}
uint8_t lumart_bytes_eq(lumart_bytes a, lumart_bytes b);
/* `slice(from, to)`: bytes [from, to), a new block (the same block when it is all). Outside: S0910. */
int32_t lumart_bytes_slice(lumart_bytes* out, lumart_bytes b, int64_t from, int64_t to, const char* file, uint32_t line);
/* `text.bytes()`: its UTF-8. */
int32_t lumart_bytes_of_text(lumart_bytes* out, lumart_text t, const char* file, uint32_t line);
/* `n` bytes at `at` (none for a null `at`) copied as bytes. */
int32_t lumart_bytes_from(lumart_bytes* out, const void* at, uint64_t n, const char* file, uint32_t line);
/* `n` zeroed bytes in the scratch, aligned to 16, until the scratch's mark is given back; null after an error. */
void* lumart_scratch_zeroed(uint64_t n, const char* file, uint32_t line);
/* `b.text()`: the bytes as a text into *out and *some 1 when they are well-formed UTF-8, else *some 0. 1 after a script
   error (S0911), else 0. */
int32_t lumart_bytes_text(lumart_text* out, uint8_t* some, lumart_bytes b, const char* file, uint32_t line);
/* `read_u16/u32/u64` (n 2, 4, 8; f32 and f64 read their bits): the n bytes at `at`, big end first when `big`. Outside
   the bytes: S0910 (first, end, length). */
int32_t lumart_bytes_read(lumart_bytes b, int64_t at, uint32_t n, uint8_t big, uint64_t* out, const char* file, uint32_t line);
/* `write_*`: the low n bytes of v at `at` (the block made this variable's own first). Outside: S0910. */
int32_t lumart_bytes_write(lumart_bytes* b, int64_t at, uint32_t n, uint8_t big, uint64_t v, const char* file, uint32_t line);
static inline float lumart_f32_of(uint64_t u) {
    const uint32_t w = (uint32_t)u;
    float f;
    __builtin_memcpy(&f, &w, 4);
    return f;
}
static inline double lumart_f64_of(uint64_t u) {
    double f;
    __builtin_memcpy(&f, &u, 8);
    return f;
}
static inline uint64_t lumart_bits_f32(float f) {
    uint32_t w;
    __builtin_memcpy(&w, &f, 4);
    return w;
}
static inline uint64_t lumart_bits_f64(double f) {
    uint64_t w;
    __builtin_memcpy(&w, &f, 8);
    return w;
}

/* ---------------------------------------------------------------- math (D7.3) */
/* The square root as an instruction: the elementwise builtin never sets errno, so it never calls the C library
   (docs/modding/language/compiled-form.md, section 1.1: none is linked); IEEE-exact, NaN below 0. */
static inline float lumart_sqrtf(float x) {
    return __builtin_elementwise_sqrt(x);
}
static inline double lumart_sqrt(double x) {
    return __builtin_elementwise_sqrt(x);
}
/* sin, cos, atan2 without the C library: fdlibm's method (the kernels' polynomials on [-pi/4, pi/4]; Cody-Waite
   reduction by pi/2 in three 33-bit parts up to 2^20 pi/2, Payne-Hanek with 1280 bits of 2/pi beyond), within an ulp
   of the exact value; NaN for an infinity. The f32 forms are the f64 ones rounded once. */
double lumart_sin(double x);
double lumart_cos(double x);
double lumart_atan2(double y, double x);
static inline float lumart_sinf(float x) {
    return (float)lumart_sin((double)x);
}
static inline float lumart_cosf(float x) {
    return (float)lumart_cos((double)x);
}
static inline float lumart_atan2f(float y, float x) {
    return (float)lumart_atan2((double)y, (double)x);
}

/* ---------------------------------------------------------------- handles (docs/modding/language/language.md, section 6.2; docs/modding/language/compiled-form.md, section 7.3) */
/* A handle: a slot of the mod's table and the slot's generation then. {0, 0} is the empty handle. */
typedef struct lumart_ref {
    uint32_t slot;
    uint32_t generation;
} lumart_ref;
/* ref(view): the object's slot, found or added, keyed by its NameObj subobject (as_class with `nameobj`, NameObj's
   class number). While the table holds a slot, `before` hooks empty the slot of each object that goes (bumping its
   generation): on lumacore::released, the objects of an arena going (a stage's objects die with their arena, no
   destructor run: k3-dtor-diag), on NebulaArena::free (one deleted, its NameObj at the block's start), and on
   NameObj::~NameObj (one destroyed through it), on whichever thread the game runs them (the table has a lock; only
   the game thread takes the hooks out). The last slot emptied on the game thread takes the hooks out, so a mod
   holding no handle costs nothing. A watch that cannot be hooked (a row missing, a hook refused) is S0920 naming
   it, and no handle. S0911 when out of memory. */
int32_t lumart_ref_make(lumart_ref* out, const void* view, int32_t nameobj, const char* file, uint32_t line);
/* Plain game records by value: the script's copy is a counted block of the record's size-row bytes, shared
   until written, as bytes are. `size` is the size row (LUMA_SIZE). */
/* A new copy, zeroed (a game function's result is made in it). S0911 when out of memory. */
int32_t lumart_record_new(lumart_bytes* out, int32_t size, const char* file, uint32_t line);
/* A copy of the game's record at `at` into *out (its old value released). S0911 when out of memory. */
int32_t lumart_record_copy(lumart_bytes* out, const void* at, int32_t size, const char* file, uint32_t line);
/* The bytes of copy *b, at least `size` (a copy with none yet is made zeroed); `own`: made this variable's own first
   (a write). Null: out of memory (S0911 reported). */
uint8_t* lumart_record_at(lumart_bytes* b, int32_t size, uint8_t own, const char* file, uint32_t line);
/* The object as class `cls` (a view), or null when the handle is empty. Safe on the game thread only: the slot is
   read under the lock, but another thread's free may follow it, as with any view a script holds. */
void* lumart_ref_get(lumart_ref r, int32_t cls);
/* The kernel's arena release (docs/modding/language/compiled-form.md): every handle to an object in [lo, hi) emptied. */
void lumart_refs_release(const void* lo, const void* hi);
static inline uint8_t lumart_ref_eq(lumart_ref a, lumart_ref b) {
    return (uint8_t)(a.slot == b.slot && a.generation == b.generation);
}
/* `chars()`: each code point as its own text, in order (a byte that starts no UTF-8 alone), into *out (a new list;
   `d` is text's descriptor). */
int32_t lumart_text_chars(lumart_list* out, lumart_text t, const lumart_desc* d, const char* file, uint32_t line);
/* `split(by)`: the pieces between each `by`, empty ones kept ("a,,b" by "," is "a", "", "b"; "" is one ""); an
   empty `by` gives the code points, as chars(). */
int32_t lumart_text_split(lumart_list* out, lumart_text t, lumart_text by, const lumart_desc* d, const char* file, uint32_t line);
/* A stable merge sort the caller drives, so a comparison can be code written in place (sort_by's closure):
   lumart_sorter_next gives each pair to compare, s->x (earlier in the list) and s->y, and takes the answer to the
   pair before it, 1 when y goes first. The elements are merged into a new block, written to the list when the last
   pair is answered; until then the list keeps what it held (the sorter holds a count of it, so a write to the list
   while sorting copies it), so a comparison may read it. Two buffers in turn, no copy back after each pass.
   lm_rel_lumart_sorter gives everything back on a way out before the end (the list unchanged). */
typedef struct lumart_sorter {
    lumart_list* l;
    lumart_list orig;
    lumart_list out;
    const lumart_desc* d;
    char* src;
    char* dst;
    char* tmp;
    uint64_t n, size, w, lo, mid, hi, i, j, k;
    const void* x;
    const void* y;
    uint8_t pending;
    uint8_t steal;
} lumart_sorter;
/* `steal`: when nothing else holds the list at the end, its elements move without their counts being touched. */
int32_t lumart_sorter_begin(lumart_sorter* s, lumart_list* l, const lumart_desc* d, uint8_t steal, const char* file, uint32_t line);
uint8_t lumart_sorter_next(lumart_sorter* s, uint8_t y_first);
void lm_rel_lumart_sorter(lumart_sorter* s);
/* A stable merge sort; `less` gives 1 into *out when a goes before b, or returns 1 for a script error (which ends the
   sort, the list unchanged). */
typedef int32_t (*lumart_less)(void* env, const void* a, const void* b, uint8_t* out);
int32_t lumart_list_sort(lumart_list* l, const lumart_desc* d, lumart_less less, void* env, const char* file, uint32_t line);
/* `less` by the element type's own order. */
int32_t lumart_less_by_type(void* d, const void* a, const void* b, uint8_t* out);
/* An index out of range: S0902 (index, length). Returns 1. */
int32_t lumart_index_error(int64_t i, uint64_t len, const char* file, uint32_t line);

typedef struct lumart_mb {
    uint32_t rc;
    uint32_t reserved;
    uint64_t len;
    uint64_t cap;
    const lumart_desc* kd;
    const lumart_desc* vd; /* null for a set */
    uint64_t voff;         /* a value's place in an entry */
    uint64_t stride;       /* an entry's size */
} lumart_mb;

typedef lumart_mb* lumart_map;

void lumart_map_drop(lumart_map m);

static inline void lm_ret_lumart_map(const lumart_map* p) {
    if (*p != 0) (*p)->rc++;
}

static inline void lm_rel_lumart_map(lumart_map* p) {
    lumart_map m = *p;
    *p = 0;
    if (m != 0 && --m->rc == 0) lumart_map_drop(m);
}

static inline void lm_cp_lumart_map(lumart_map* d, lumart_map s) {
    lm_ret_lumart_map(&s);
    lumart_map old = *d;
    *d = s;
    lm_rel_lumart_map(&old);
}

static inline uint64_t lumart_map_n(lumart_map m) {
    return m != 0 ? m->len : 0;
}

static inline void* lumart_map_key(lumart_map m, uint64_t i) {
    return (char*)(m + 1) + i * m->stride;
}

static inline void* lumart_map_val(lumart_map m, uint64_t i) {
    return (char*)(m + 1) + i * m->stride + m->voff;
}

/* Makes *m this variable's own (copied when shared) with room for `more` entries. */
int32_t lumart_map_own(lumart_map* m, const lumart_desc* kd, const lumart_desc* vd, uint64_t more, const char* file, uint32_t line);
/* Whether *k is a key; *at its index, or where it would go. */
uint8_t lumart_map_find(lumart_map m, const lumart_desc* kd, const void* k, uint64_t* at);
/* Sets *k to *v (a set: vd and v null); 1 into *added when the key is new. Copies, counts taken. */
int32_t lumart_map_put(lumart_map* m, const lumart_desc* kd, const lumart_desc* vd, const void* k, const void* v, uint8_t* added, const char* file, uint32_t line);
/* Makes *m this variable's own; then removes entry `at` (< len), giving its counts back: call after reading it. */
int32_t lumart_map_remove(lumart_map* m, const lumart_desc* kd, const lumart_desc* vd, uint64_t at, const char* file, uint32_t line);
uint8_t lumart_map_eq(lumart_map a, lumart_map b, const lumart_desc* kd, const lumart_desc* vd);
/* A missing key read by `m[k]`: S0907, the key as text. Returns 1. */
int32_t lumart_key_error(const char* key, const char* file, uint32_t line);

/* lower() / upper(): Unicode simple case mapping per code point; bytes that are not UTF-8 kept as they are. */
int32_t lumart_text_case(lumart_text* out, lumart_text t, uint8_t upper, const char* file, uint32_t line);
/* parse_int() / parse_float(): 1 and the value, or 0 (none). */
uint8_t lumart_text_parse_i64(lumart_text t, int64_t* out);
uint8_t lumart_text_parse_f64(lumart_text t, double* out);

/* ---------------------------------------------------------------- json (D7.4, D4.10)

   json.Value is one complete JSON text in json.encode's form, held as a text (counted, shared). A reader walks JSON
   bytes; each read gives 0 and moves past what it read, 1 after a script error it reported (S0911), 2 when the bytes
   there are not that (a decode is then none). As built: an integer reads only from a number with no fraction and no
   exponent (`2.0` is no int); nesting past 256 levels does not read. */
typedef struct lumart_jr {
    const char* p;
    const char* end;
    uint32_t depth; /* arrays and objects open (lumart_jr_open): past 256 the data does not read */
} lumart_jr;
void lumart_jr_init(lumart_jr* r, const void* p, uint64_t n);
uint8_t lumart_jr_done(lumart_jr* r); /* only white space left */
int32_t lumart_jr_null(lumart_jr* r);
uint8_t lumart_jr_is_null(lumart_jr* r); /* `null` next (not read) */
char lumart_jr_next(lumart_jr* r);       /* the next byte past white space (not read), 0 at the end */
int32_t lumart_jr_bool(lumart_jr* r, uint8_t* out);
int32_t lumart_jr_i64(lumart_jr* r, int64_t lo, int64_t hi, int64_t* out); /* a whole number in [lo, hi], exactly */
int32_t lumart_jr_u64(lumart_jr* r, uint64_t hi, uint64_t* out);
int32_t lumart_jr_f64(lumart_jr* r, double* out); /* the nearest double; one past the largest does not read */
int32_t lumart_jr_f32(lumart_jr* r, float* out);  /* the nearest float, rounded once */
int32_t lumart_jr_text(lumart_jr* r, lumart_text* out, const char* file, uint32_t line); /* a string, unescaped */
int32_t lumart_jr_raw(lumart_jr* r, lumart_text* out, const char* file, uint32_t line);  /* any value: its text */
int32_t lumart_jr_skip(lumart_jr* r);
int32_t lumart_jr_open(lumart_jr* r, char open); /* '[' or '{' */
/* Inside an array or object: *more 1 when an element follows (*index counts them; the comma before it read), 0 at
   `close` (read). */
int32_t lumart_jr_more(lumart_jr* r, char close, uint64_t* index, uint8_t* more);
int32_t lumart_jr_key(lumart_jr* r, lumart_text* out, const char* file, uint32_t line); /* a key and its ':' */
/* A key and its ':'; *is_text 0 (and *out empty) for a string that is no text (a lone surrogate): no field's name. */
int32_t lumart_jr_field(lumart_jr* r, lumart_text* out, uint8_t* is_text, const char* file, uint32_t line);
/* A key that is a whole number as json.encode writes a map's (decimal, no escape, no space), in [lo, hi]. */
int32_t lumart_jr_key_i64(lumart_jr* r, int64_t lo, int64_t hi, int64_t* out);
int32_t lumart_jr_key_u64(lumart_jr* r, uint64_t hi, uint64_t* out);
uint8_t lumart_text_is(lumart_text t, const char* s, uint64_t n);
/* json.Value's methods (D4.10): *some 0 for none; 1 returned after a script error (S0911). */
int32_t lumart_json_text(lumart_text v, lumart_text* out, uint8_t* some, const char* file, uint32_t line);
uint8_t lumart_json_int(lumart_text v, int32_t* out);
uint8_t lumart_json_float(lumart_text v, float* out);
uint8_t lumart_json_bool(lumart_text v, uint8_t* out);
int32_t lumart_json_list(lumart_text v, lumart_list* out, const lumart_desc* d, uint8_t* some, const char* file, uint32_t line);
int32_t lumart_json_map(lumart_text v, lumart_map* out, const lumart_desc* kd, const lumart_desc* vd, uint8_t* some, const char* file, uint32_t line);
int32_t lumart_json_get(lumart_text v, lumart_text key, lumart_text* out, uint8_t* some, const char* file, uint32_t line);
int32_t lumart_json_at(lumart_text v, int64_t at, lumart_text* out, uint8_t* some, const char* file, uint32_t line);
/* A decoder's read, in generated code: its 1 or 2 is the function's status. */
#define LUMART_JR(x)                          \
    do {                                      \
        const int32_t lm_jst_ = (x);          \
        if (lm_jst_ != 0) {                   \
            lm_st = lm_jst_;                  \
            goto lm_out;                      \
        }                                     \
    } while (0)

/* ---------------------------------------------------------------- text and the game (docs/modding/language/compiled-form.md) */

/* A `const char *` the game gave, copied into `*out` (null: the empty text). */
int32_t lumart_text_from_game(lumart_text* out, const char* s, const char* file, uint32_t line);
/* A game `const char16_t *` (to its 0, tags kept as bracket words) as text; empty for null. */
int32_t lumart_text_from_game16(lumart_text* out, const uint16_t* s, const char* file, uint32_t line);
/* `text "Key"` at file:line: the game's message of the key in the player's language as text, the key when the game
   has none, or a row (LUMART_TEXT_ROW) when the game's messages do not exist yet. Never calls the game before they
   do. */
int32_t lumart_text_message(lumart_text* out, const char* key, uint64_t key_n, const char* file, uint32_t line);

/* Another mod's text `s` made this mod's own in `*d`. A row not read yet stays a row of the same key, so the line is
   read once the game's messages exist (the game holds one message table, so it is the line the maker would read);
   its place names this mod's own string and the maker's line, never the maker's file pointer, which goes with the
   maker's library. Any other text is copied. */
int32_t lumart_text_cross(lumart_text* d, lumart_text s, const char* file, uint32_t line);
/* `text "Key"(name: v)`: every "{name}" of t replaced by v. */
int32_t lumart_text_fill(lumart_text* out, lumart_text t, const char* name, uint64_t name_n, const char* v, uint64_t v_n,
                         const char* file, uint32_t line);
/* A text as a `const char *` read during one call (a service's text parameter, D12.3): its own bytes, NUL-ended. Null
   when it holds a NUL byte (S0918 naming `function` and `param`). */
const char* lumart_text_cstr(lumart_text t, const char* file, uint32_t line, const char* function, const char* param);
/* Text the game may keep: a pointer valid while the library is loaded, one copy per distinct text. Null when the
   text holds a NUL byte (S0918 naming `function` and `param`) or no memory is left (S0911). */
const char* lumart_text_keep(lumart_text t, const char* file, uint32_t line, const char* function, const char* param);
/* The same text as UTF-16 the game may keep (a `const char16_t *` result): made once per distinct text, kept while the
   library is loaded; a byte that is no UTF-8 is U+FFFD. Null as lumart_text_keep. */
const uint16_t* lumart_text_keep16(lumart_text t, const char* file, uint32_t line, const char* function, const char* param);

/* ---------------------------------------------------------------- the log (docs/modding/language/language.md, section 10) */

/* `log text`: the line through LumaCore.log as the mod. */
void lumart_log(const char* text);
/* NaN or an infinity in a value written as a fact or json: S0917 (D7.4), 1. 0 when every one is finite. */
int32_t lumart_finite_f32(const float* v, int n, const char* file, uint32_t line, const char* source);
int32_t lumart_finite_f64(double v, const char* file, uint32_t line, const char* source);

/* ---------------------------------------------------------------- tasks (docs/modding/language/language.md, section 11, D10.3) */
/* A task is a frame (its locals, the generated struct lm_task_*) and a step function that runs it from the last
   `wait` to the next: no stack is kept between frames and nothing in the game schedules it. lumart keeps the live
   frames in start order and runs them from the mod's frame handler (time's on_frame), after `on frame`. */
typedef struct lumart_task {
    uint32_t slot;       /* 0: no task */
    uint32_t generation; /* the slot's generation when the task started: an ended task's handle matches nothing */
} lumart_task;
/* Runs the task from its step: LUMART_PARKED at a `wait` (the frame keeps everything), 0 at its end or `stop`, 1
   when a script error ended it (reported). */
typedef int32_t (*lumart_step)(void* frame);
#define LUMART_PARKED 2
/* The head of every frame: written by lumart_task_new and the step function only. */
typedef struct lumart_task_head {
    lumart_step run;
    uint64_t size;    /* the frame's size, for lumart_free */
    uint32_t slot;
    uint32_t step;    /* the wait to resume at, 0 for the body's start */
    uint8_t stop;     /* stopped: the next run goes to the way out (defers, counts given back) */
    uint8_t running;  /* its step function is on the stack */
    uint8_t reserved[6];
    lumart_as as;     /* the caller it acts for: lumart_caller when it was made (a library's task started by a mod's
                         call), set again for each of its steps, so caller() and reports are that mod's after a wait */
} lumart_task_head;
/* The frame and world step of the pass running now (time's LumaTimeNow), read by the waits. */
extern uint64_t lumart_now_frame;
extern uint64_t lumart_now_step;
/* A zeroed frame of `size` bytes for step function `run`, its handle in *out, in the list from the next pass; null
   (S0911, *out empty) when no memory is left. */
void* lumart_task_new(uint64_t size, lumart_step run, lumart_task* out, const char* file, uint32_t line);
/* Runs a new task's first step now (a waiting `on start`): its result as lumart_step's. */
int32_t lumart_task_first(lumart_task t);
uint8_t lumart_task_alive(lumart_task t);
/* `t.stop()`: a task not running now runs to its way out at once; one running now (stopping itself, or a function it
   called) at its next `wait` or end. An ended task: nothing. */
void lumart_task_stop(lumart_task t);
/* A result-less task called (docs/modding/language/language.md, section 11.1a): lumart_task_watch marks it, at its call, and its step's way out calls
   lumart_task_done, which keeps a `()` result for its pending when the task ended by itself and was marked. A task
   only `start`ed keeps nothing. */
void lumart_task_watch(lumart_task t);
void lumart_task_done(void* frame, const char* file, uint32_t line);
static inline uint8_t lumart_task_eq(lumart_task a, lumart_task b) {
    return (uint8_t)(a.slot == b.slot && a.generation == b.generation);
}
/* The frame pass, in two: lumart_tasks_pass at the frame handler's start (before `on frame`) keeps the frame's time
   and gives how many tasks the pass runs; lumart_tasks_run then runs those, in start order. A task started later in
   the pass (by `on frame` or by a task) runs from the next frame (11.1). */
uint32_t lumart_tasks_pass(uint64_t frame, uint64_t step);
void lumart_tasks_run(uint32_t n);
/* A task with a result (docs/modding/language/language.md, section 11.1a). Its `return v` keeps the value under the task's handle
   (lumart_task_result_keep, from its frame: the count moves to the kept copy); the task then ends as any. A call of the
   task gives a lumart_pending: the handle and its maker's operations, so a pending made by a library is read and
   stopped by the library's own code. poll: 0 while the task runs; 1 when it ended with a value, now written into
   `out` (zeroed by the caller) by `in`, the reader's own copy (a library's value copied into the caller's memory), or
   as it is when `in` is null (a value holding nothing counted), and given up by the maker; 2 when it ended without one
   (stopped: `p.stop()`, its own `stop`, a script error, or the value already read); 3 when `in` failed (reported).
   `in`'s last argument is the maker's lumart_ref_get: a handle in the value is the maker's, made again in the reader's
   table from its object. */
typedef struct lumart_pending_ops {
    int32_t (*poll)(lumart_task t, void* out, int32_t (*in)(void*, const void*, const char*, uint32_t, const void*), const char* file, uint32_t line);
    void (*stop)(lumart_task t);
} lumart_pending_ops;
typedef struct lumart_pending {
    lumart_task t;
    const lumart_pending_ops* ops; /* null: a pending never set, read as ended without a value */
} lumart_pending;
extern const lumart_pending_ops lumart_pending_own;
/* The value `v` (`d->size` bytes, its counts moved in) kept for the task of frame `frame` until read; 1 (S0911) when no
   memory is left. */
int32_t lumart_task_result_keep(void* frame, void* v, const lumart_desc* d, const char* file, uint32_t line);
/* At the mod's stop: every task stopped (its way out run; a task a way out starts is stopped too), the lists freed.
   A task still alive after eight rounds (it cannot be: a stopped step goes to its way out, which cannot wait, D9.4)
   has its frame freed and a log line. */
void lumart_tasks_stop_all(void);
/* `wait N frames`: the frame to resume at; 0 is the next frame's pass (D10.3: every wait parks first). A negative N
   reports S0919 (D10.3's run-time S0110) and then waits as 0: the task goes on, the report is the mod log's. */
uint64_t lumart_wait_frames(int64_t n, const char* file, uint32_t line);
/* `wait N seconds`: the world step to resume at, N x 60 steps to the nearest (a half up), at least 1 (D10.3, F-D10-1).
   Then as 0 seconds (1 step) after a report: a negative N S0919, a NaN S0917. An infinite N waits until the task is
   stopped (the last step there is). */
uint64_t lumart_wait_seconds(double n, const char* file, uint32_t line);

/* ---------------------------------------------------------------- hooks (docs/modding/language/compiled-form.md, section 4) */

/* 1 on the game thread. Off it, reports S0901 naming the hook's function and gives 0 (D4.3 A-2). */
int32_t lumart_on_game_thread(const char* file, uint32_t line, const char* function);

/* ---------------------------------------------------------------- objects (docs/modding/language/classes.md, "make") */

/* std `make(name, place, count)`: `count` objects by the factory's `name` (LumaCore.make), each at the one place: `at`,
   `rot` (degrees), `scale`, `args` (Obj_arg0 on; the rest -1), `dead` (LUMA_MAKE_DEAD). Each object's part of class
   `class_number` (as_class; LiveActor's) is appended to *out (elements described by `d`), in the order made; an
   object of another class is left out. The kernel's refusals (each said in the mod log with the name) and a count
   outside 1 to 1024 append nothing. 1 after a script error reported: a NUL byte in the name (S0918), more than 8 args
   (S0914), no memory (S0911); else 0. */
int32_t lumart_make(lumart_text name, lm_vec3 at, lm_vec3 rot, lm_vec3 scale, lumart_list args, uint8_t dead, int64_t count,
                    int32_t class_number, const lumart_desc* d, lumart_list* out, const char* file, uint32_t line);
/* alias_class (docs/modding/language/classes.md) for the calling mod (lumart_caller's, else this mod): `name` makes an object of
   the live mod class `class_name`, its placement arguments args[i] (the rest -1, the placement's own). *out 1: the
   alias stands; the kernel says every refusal. 1 after a script error reported: a NUL byte in a name (S0918), more
   than 8 args (S0914); else 0. */
/* set_number (docs/modding/language/numbers.md) for the calling mod (a library acts for its caller): *out 1 when set. `signature`
   null: a file's constant (signature "", place 0). Its text too long or not UTF-8: said at file:line, 1. */
int32_t lumart_set_number(lumart_text name, lumart_text signature, uint32_t place, double value, uint8_t* out, const char* file,
                          uint32_t line);
int32_t lumart_alias_class(lumart_text name, lumart_text class_name, lumart_list args, uint8_t* out, const char* file,
                           uint32_t line);
/* set_table (docs/modding/language/numbers.md) for the calling mod (a library acts for its caller): `size` bytes at `offset`
   in the table `name`, as the game holds them; *out 1 when written (the kernel says every refusal). Returns 0. */
int32_t lumart_set_table(const char* name, uint32_t offset, const void* bytes, uint32_t size, uint8_t* out, const char* file,
                         uint32_t line);
/* table (docs/modding/language/numbers.md): the `size` bytes at `offset` in the table `name` as the game reads them now
   into *out, *some 1; *some 0 when the game has no such table or they are not inside it. Returns 0. */
int32_t lumart_table(const char* name, uint32_t offset, void* out, uint32_t size, uint8_t* some, const char* file,
                     uint32_t line);
/* placement_of (docs/modding/language/classes.md): *some 1 and the placement's texts new in *stage, *zone, *layer (their old
   values released), *index; *some 0: none (no object, none the stage placed). 1 after a script error reported (no
   memory, S0911); else 0. */
int32_t lumart_placement_of(const void* object, uint8_t* some, lumart_text* stage, lumart_text* zone, lumart_text* layer,
                            uint32_t* index, const char* file, uint32_t line);
/* placed_at (docs/modding/language/classes.md): *out the object made from the placement as class `class_number` (null: none; an
   empty stage the main stage). 1 after a script error reported: a NUL byte in a name (S0918); else 0. */
int32_t lumart_placed_at(lumart_text stage, lumart_text zone, lumart_text layer, uint32_t index, int32_t class_number,
                         void** out, const char* file, uint32_t line);
/* make_gravity (docs/modding/language/classes.md): *out an object of this mod's gravity class `class_name` as class `class_number`
   (null: the kernel made none, said). 0. */
int32_t lumart_make_gravity(const char* class_name, float range, int32_t priority, uint32_t types, uint8_t inverse,
                            int32_t class_number, void** out, const char* file, uint32_t line);

#ifdef __cplusplus
}
#endif
#endif
