/* NebulaArena::free and lumacore::released: hooks a handle's runtime added before the kernel kept the handles (M-132;
   it hooked NameObj::~NameObj too, which the harness's own mod hooks, so its flag says nothing here). Each hooked one
   counts; a row missing counts 10. */
#include "watch.h"
#include "lumart.h"

static int32_t hooked(const char* name, const char* signature) {
    const LumaFunction* f = luma_rt_core->function(name, signature);
    return f == 0 ? 10 : (f->hooked != 0 && *f->hooked != 0);
}

int32_t watch_hooked(void) {
    return hooked("NebulaArena::free", "void(void *)") + hooked("lumacore::released", "void(const void *, const void *)");
}
