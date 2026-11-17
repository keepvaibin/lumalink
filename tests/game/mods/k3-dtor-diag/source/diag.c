/* Counts before-hook calls on the game thread and off it: NameObj::~NameObj (0, 1), ArenaOwned::~ArenaOwned (2, 3),
   NebulaArena::free(void *) (4, 5), lumacore::released (6, 7). */
#include "diag.h"
#include "lumart.h"

static int32_t counts[8];

static void count(int32_t at) {
    __atomic_fetch_add(&counts[at + (luma_rt_core->game_thread() ? 0 : 1)], 1, __ATOMIC_RELAXED);
}
static int32_t on_nameobj(LumaCall* call, void* user) {
    (void)call;
    (void)user;
    count(0);
    return 0;
}
static int32_t on_arena_owned(LumaCall* call, void* user) {
    (void)call;
    (void)user;
    count(2);
    return 0;
}
static int32_t on_free(LumaCall* call, void* user) {
    (void)call;
    (void)user;
    count(4);
    return 0;
}
static int32_t on_released(LumaCall* call, void* user) {
    (void)call;
    (void)user;
    count(6);
    return 0;
}

static int32_t hook(const char* name, const char* signature, LumaHook h) {
    const LumaFunction* f = luma_rt_core->function(name, signature);
    return f != 0 && luma_rt_core->hook(luma_rt_self, f, LUMA_BEFORE, h, 0) == 0;
}

/* How many of the four were hooked. */
int32_t diag_start(void) {
    return hook("NameObj::~NameObj", 0, on_nameobj) + hook("ArenaOwned::~ArenaOwned", 0, on_arena_owned) +
           hook("NebulaArena::free", "void(void *)", on_free) +
           hook("lumacore::released", "void(const void *, const void *)", on_released);
}

int32_t diag_count(int32_t which) {
    return which >= 0 && which < 8 ? __atomic_load_n(&counts[which], __ATOMIC_RELAXED) : -1;
}
