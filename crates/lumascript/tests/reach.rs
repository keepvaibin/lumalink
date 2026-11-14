//! The service parts the SDK's headers declare, read by the rules that make them a script's: each call checked, written
//! as C and compiled against the SDK's own headers (`LUMA_SDK`). What stays unreachable is draw.native alone.

use luma_validate::names::Types;
use lumascript::check::{self, ModFacts, Source};
use lumascript::emit;
use lumascript::game::Game;
use lumascript::svc::Services;
use std::collections::HashMap;

const GAME: &str = r#"{"types": [{"kind": "alias", "name": "s32", "of": "int", "record": null}], "globals": [], "left_out": []}"#;

/// Every service of the SDK's headers, or None when LUMA_SDK is not set.
fn services() -> Option<(std::path::PathBuf, Services)> {
    let Some(sdk) = std::env::var_os("LUMA_SDK") else {
        eprintln!("skipped: LUMA_SDK is not set (the SDK's headers)");
        return None;
    };
    let mut headers = std::collections::BTreeMap::new();
    for e in std::fs::read_dir(&sdk).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("luma") && name.ends_with(".h") {
            headers.insert(name, std::fs::read_to_string(e.path()).unwrap());
        }
    }
    let (services, errors) = Services::of_sdk(&headers);
    assert!(errors.is_empty(), "{errors:?}");
    Some((sdk.into(), services))
}

/// The program's files, the checker saying nothing, each C file compiled against the SDK.
fn build(sdk: &std::path::Path, services: &Services, text: &str) -> Vec<(String, String)> {
    let (module, d) = lumascript::parse_module(text);
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = Game::new(HashMap::new(), Some(Types::from_json(GAME.as_bytes()).unwrap()));
    game.services = services.clone();
    let files = lumascript::on_compiler_stack(|| {
        let checked = check::check(&sources, &[], ModFacts::of_game(&game));
        let errors: Vec<_> = checked.diags.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
        assert!(errors.is_empty(), "{errors:?}");
        emit::emit("reach", &sources, &checked, &game).unwrap_or_else(|d| panic!("{d:?}")).files
    })
    .unwrap();
    let clang = std::env::var_os("LUMA_CLANG").unwrap_or_else(|| "C:/Program Files/LLVM/bin/clang.exe".into());
    let dir = std::env::temp_dir().join(format!("lumascript-reach-{}-{}", std::process::id(), text.len()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, t) in &files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, t).unwrap();
    }
    std::fs::create_dir_all(dir.join("source")).unwrap();
    for (i, (rel, t)) in files.iter().enumerate().filter(|(_, (rel, _))| rel.ends_with(".c")) {
        let out = std::process::Command::new(&clang)
            .current_dir(&dir)
            .arg("--target=x86_64-pc-windows-msvc")
            .args(emit::C_FLAGS)
            .arg("-I").arg(dir.join("lum")).arg("-I").arg(sdk).arg("-I").arg(dir.join("source"))
            .arg("-c").arg(rel).arg("-o").arg(dir.join(format!("{i}.o")))
            .output()
            .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
        assert!(out.status.success(), "{rel} does not compile:\n{}\n{t}", String::from_utf8_lossy(&out.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
    files
}

fn main_c(files: &[(String, String)]) -> String {
    files.iter().find(|(p, _)| p == "lum/main.c").map(|(_, c)| c.clone()).unwrap()
}

/// The program linked as a library and run in SERVICE_HOST: the host's lines.
fn run(sdk: &std::path::Path, services: &Services, text: &str) -> Vec<String> {
    let files = build(sdk, services, text);
    let clang = std::env::var_os("LUMA_CLANG").unwrap_or_else(|| "C:/Program Files/LLVM/bin/clang.exe".into());
    let dir = std::env::temp_dir().join(format!("lumascript-reach-run-{}-{}", std::process::id(), text.len()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, t) in &files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, t).unwrap();
    }
    std::fs::create_dir_all(dir.join("source")).unwrap();
    let c_files: Vec<&String> = files.iter().map(|(rel, _)| rel).filter(|rel| rel.ends_with(".c")).collect();
    let out = std::process::Command::new(&clang)
        .current_dir(&dir)
        .arg("--target=x86_64-pc-windows-msvc")
        .args(emit::C_FLAGS)
        .arg("-I").arg(dir.join("lum")).arg("-I").arg(sdk).arg("-I").arg(dir.join("source"))
        .args(["-shared", "-fuse-ld=lld", "-Wl,/NOENTRY", "-Wl,/NODEFAULTLIB", "-Wl,/NOIMPLIB"])
        .args(&c_files)
        .arg("-o").arg(dir.join("mod.dll"))
        .output()
        .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
    assert!(out.status.success(), "the library does not link:\n{}", String::from_utf8_lossy(&out.stderr));
    std::fs::write(dir.join("host.c"), SERVICE_HOST).unwrap();
    let out = std::process::Command::new(&clang)
        .current_dir(&dir)
        .arg("--target=x86_64-pc-windows-msvc")
        .arg("-I").arg(sdk)
        .arg("host.c").arg("-o").arg(dir.join("host.exe"))
        .output()
        .unwrap_or_else(|e| panic!("clang {clang:?}: {e}"));
    assert!(out.status.success(), "the host does not build:\n{}", String::from_utf8_lossy(&out.stderr));
    let ran = std::process::Command::new(dir.join("host.exe")).arg(dir.join("mod.dll")).output().unwrap();
    let lines: Vec<String> = String::from_utf8_lossy(&ran.stdout).lines().map(str::to_string).collect();
    assert!(ran.status.success(), "exit {:?}: {lines:?}", ran.status);
    let _ = std::fs::remove_dir_all(&dir);
    lines
}

/// A host for one mod library with three services as a game gives them: audio (a stream of 2 channels, handle 5,
/// that prints what it is pushed), assets (bytes lends "lent!") and capture.camera (open keeps the closure; the host
/// then gives it one FRAME of 2 x 2 pixels, stride 8). Reports print their arguments. It runs begin, start, the
/// frame, stop and finish, then prints "held <blocks>".
const SERVICE_HOST: &str = r#"#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <malloc.h>
#include "luma.h"
#include "luma_audio.h"
#include "luma_assets.h"
#include "luma_capture.h"
static int live;
static void* alloc(const LumaMod* m, uint64_t s, uint64_t al) { (void)m; live++; return _aligned_malloc((size_t)s, (size_t)(al < 16 ? 16 : al)); }
static void release(const LumaMod* m, void* p) { (void)m; if (p != 0) live--; _aligned_free(p); }
static void say(const LumaMod* m, const char* t) { (void)m; printf("log %s\n", t); }
static void report(const LumaMod* m, const char* file, uint32_t line, uint32_t code, const char* const* args, uint32_t n) {
    (void)m;
    printf("report %s:%u %u", file, line, code);
    for (uint32_t i = 0; i < n; i++) printf(" [%s]", args[i]);
    printf("\n");
}
static int32_t game_thread(void) { return 1; }
static void zero(void** args, void* result) { (void)args; if (result != 0) *(void**)result = 0; }
static LumaFunction any = { "any", "", zero, 0 };
static const LumaFunction* function(const char* name, const char* signature) { (void)name; (void)signature; return &any; }
static int32_t field(const char* c, const char* name, const char* type) { (void)c; (void)name; (void)type; return 0; }
static int32_t hook(const LumaMod* m, const LumaFunction* f, uint32_t k, LumaHook h, void* u) { (void)m; (void)f; (void)k; (void)h; (void)u; return 0; }

static int32_t stream_open(const LumaMod* m, const LumaAudioStreamDesc* d, LumaEventFn fn, void* user, LumaHandle* out) {
    (void)m; (void)fn; (void)user;
    printf("stream_open %u channels\n", d->channels);
    *out = 5;
    return LUMA_OK;
}
static int32_t stream_channels(const LumaMod* m, LumaHandle h, uint32_t* out) {
    (void)m;
    if (h != 5) return LUMA_E_HANDLE;
    *out = 2;
    return LUMA_OK;
}
static int64_t stream_push(const LumaMod* m, LumaHandle h, const float* samples, uint64_t frames) {
    (void)m;
    printf("stream_push %llu %llu frames", (unsigned long long)h, (unsigned long long)frames);
    for (uint64_t i = 0; i < frames * 2u; i++) printf(" %g", samples[i]);
    printf("\n");
    return (int64_t)frames;
}
static const char lent[] = "lent!";
static int32_t bytes(const LumaMod* m, const char* path, uint32_t flags, const void** data, uint64_t* n) {
    (void)m; (void)flags;
    printf("bytes %s\n", path);
    *data = lent;
    *n = 5;
    return LUMA_OK;
}
static LumaEventFn camera_fn;
static void* camera_user;
static int32_t camera_open(const LumaMod* m, const LumaCaptureCameraDesc* d, LumaEventFn fn, void* user, LumaHandle* out) {
    (void)m; (void)d;
    camera_fn = fn;
    camera_user = user;
    *out = 9;
    return LUMA_OK;
}
static LumaAudioService audio;
static LumaAssetsService assets;
static LumaCaptureCameraService camera;
static const void* service(const LumaMod* m, const char* name, uint32_t major, uint32_t minor) {
    (void)m; (void)major; (void)minor;
    if (strcmp(name, LUMA_AUDIO_NAME) == 0) return &audio;
    if (strcmp(name, LUMA_ASSETS_NAME) == 0) return &assets;
    if (strcmp(name, LUMA_CAPTURE_CAMERA_NAME) == 0) return &camera;
    return 0;
}
static LumaCore core;
int main(int argc, char** argv) {
    if (argc < 2) return 2;
    core.size = sizeof core;
    core.api = LUMA_API;
    core.log = say;
    core.report = report;
    core.function = function;
    core.field = field;
    core.hook = hook;
    core.unhook = hook;
    core.service = service;
    core.game_thread = game_thread;
    core.alloc = alloc;
    core.free = release;
    audio.stream_open = stream_open;
    audio.stream_channels = stream_channels;
    audio.stream_push = stream_push;
    assets.bytes = bytes;
    camera.open = camera_open;
    HMODULE dll = LoadLibraryA(argv[1]);
    if (dll == 0) { printf("no library\n"); return 3; }
    const LumaModInfo* info = (const LumaModInfo*)(void*)GetProcAddress(dll, "luma_mod");
    if (info == 0) { printf("no luma_mod\n"); return 4; }
    const LumaMod* self = (const LumaMod*)(void*)&core;
    for (uint32_t i = 0; i < info->import_count; i++) {
        const LumaImport* r = &info->imports[i];
        switch (r->kind) {
        case LUMA_IMPORT_FUNCTION: *(const LumaFunction**)r->slot = &any; break;
        case LUMA_IMPORT_SERVICE: *(const void**)r->slot = service(self, r->name, 1, 0); break;
        case LUMA_IMPORT_FIELD: case LUMA_IMPORT_CLASS: *(int32_t*)r->slot = 0; break;
        case LUMA_IMPORT_SIZE: *(int32_t*)r->slot = 8; break;
        default: printf("import kind %u\n", r->kind); return 5;
        }
    }
    info->begin(&core, self);
    printf("start %d\n", (int)info->start(&core, self));
    if (camera_fn != 0) {
        static const uint8_t pixels[16] = { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16 };
        LumaCaptureCameraFrame frame = { sizeof frame, 2, 2, 8, 77, pixels };
        LumaEvent ev = { sizeof ev, LUMA_CAPTURE_CAMERA_EVENT_FRAME, LUMA_OK, 0, 9, &frame, sizeof frame, 0 };
        camera_fn(camera_user, &ev);
    }
    info->stop();
    info->finish();
    printf("held %d\n", live);
    return 0;
}
"#;

/// A const pointer to another service's struct of a header this one includes (draw.view's camera takes three
/// draw.Vec3), a struct of the service's own header that no service's name prefixes (assets.AssetInfo out of find,
/// a list of assets.AssetFile into provide, content.model.PlayerPart out of player_parts), and another service's
/// struct of the same header as a list out (input.dev's list gives input.DeviceInfo).
#[test]
fn structs_of_values_cross_in_and_out() {
    let Some((sdk, services)) = services() else { return };
    for name in ["draw.view.camera", "assets.find", "assets.provide", "content.model.player_parts", "input.dev.list"] {
        let (svc, f) = name.rsplit_once('.').unwrap();
        let shape = &services.faces[svc].function(f).unwrap().shape;
        assert!(matches!(shape, lumascript::svc::Shape::Callable { .. }), "{name}: {shape:?}");
    }
    let text = "use svc.draw
use svc.draw.view
use svc.assets
use svc.content.model
use svc.input.dev

var seen = 0

fn look(v: view.Region) {
    let up = draw.Vec3 { x: 0.0, y: 1.0, z: 0.0 }
    view.camera(v, draw.Vec3 { x: 0.0, y: 0.0, z: -5.0 }, draw.Vec3 { x: 0.0, y: 0.0, z: 0.0 }, up, 60.0)
}

on start {
    if let info = assets.find(\"ObjectData/Kuribo.arc\", 0) {
        seen += info.entries as int
    }
    let files = [assets.AssetFile { path: \"a.txt\", data: \"abc\".bytes() }]
    assets.provide(\"Luma/reach\", files)
    for p in model.player_parts() {
        log \"{p.name} {p.joint}\"
    }
    if let devices = dev.list() {
        for d in devices {
            log \"{d.name} {d.vendor}\"
        }
    }
    log \"{seen}\"
}
";
    let c = main_c(&build(&sdk, &services, text));
    assert!(c.contains("->camera(luma_rt_self, "), "{c}");
}

/// An out buffer whose size and length are 32 bits with other names (assets.entry's `name_size` and `needed`, the
/// needed size counting the 0 byte), and bytes in with a 32-bit size (input.dev.write), its length checked to fit.
#[test]
fn buffers_with_32_bit_sizes_cross() {
    let Some((sdk, services)) = services() else { return };
    for name in ["assets.entry", "input.dev.write", "capture.camera.open", "draw.view.follow"] {
        let (svc, f) = name.rsplit_once('.').unwrap();
        let shape = &services.faces[svc].function(f).unwrap().shape;
        assert!(matches!(shape, lumascript::svc::Shape::Callable { .. }), "{name}: {shape:?}");
    }
    let text = "use svc.assets
use svc.input.dev

fn send(d: dev.Request) {
    dev.write(d, \"ab\".bytes())
}

on start {
    if let r = assets.entry(\"ObjectData\", 0, 3) {
        let (name, info) = r
        log \"{name} {info.bytes}\"
    }
}
";
    let c = main_c(&build(&sdk, &services, text));
    assert!(c.contains("->entry(luma_rt_self, ") && c.contains("->write(luma_rt_self, "), "{c}");
    assert!(c.contains("> 0xFFFFFFFFu && lumart_overflow("), "{c}");
}

/// Numbers whose count is never the script's: audio.stream_push's samples (frames = their number over
/// stream_channels' answer, a length not a whole number of frames a script error naming both), a struct's list of
/// floats with its count before it (anim.How.marks), and four floats the comment counts (content.model.set_param's
/// rgba, a vec4). Bytes the service lends (assets.bytes), copied, and an event's struct (capture.camera's FRAME) with
/// its pixels, stride x height bytes.
#[test]
fn counts_come_from_the_service_never_the_script() {
    let Some((sdk, services)) = services() else { return };
    if !std::fs::read_to_string(sdk.join("luma_audio.h")).unwrap().contains("stream_channels") {
        eprintln!("skipped: the SDK's audio is before 1.1");
        return;
    }
    for name in ["audio.stream_push", "content.model.set_param", "anim.play_on_model", "assets.bytes", "capture.camera.open"] {
        let (svc, f) = name.rsplit_once('.').unwrap();
        let shape = &services.faces[svc].function(f).unwrap().shape;
        assert!(matches!(shape, lumascript::svc::Shape::Callable { .. }), "{name}: {shape:?}");
    }
    let text = "use svc.audio
use svc.anim
use svc.assets
use svc.content.model
use svc.capture.camera

var size = 0

fn push(s: audio.Request, samples: list<f32>) {
    log \"{audio.stream_push(s, samples)}\"
}

fn tint(m: model.Model) {
    model.set_param(m, \"body\", \"tint\", vec4(1.0, 0.5, 0.25, 1.0))
}

fn marks(m: model.Model) {
    let how = anim.How { rate: 1.0, blend: 0, loop: 1, start: 0.0, marks: [2.0, 8.0] }
    let p = anim.play_on_model(m, \"Wait\", how, fn(e: anim.Event) {
        match e {
            anim.Event.Mark(socket, value, data) => log \"{value} {data}\"
            else => return
        }
    })
}

on start {
    if let b = assets.bytes(\"ObjectData/Kuribo.arc\", 0) {
        size = b.len()
    }
    let c = camera.open(camera.Desc { width: 0, height: 0, fps: 0 }, fn(e: camera.Event) {
        match e {
            camera.Event.Frame(socket, status, data) => { size = data.pixels.len() + data.width as int }
            else => return
        }
    })
}
";
    let c = main_c(&build(&sdk, &services, text));
    assert!(c.contains("->stream_channels(luma_rt_self, ") && c.contains("lumart_wrong_multiple("), "{c}");
    assert!(c.contains("->set_param(luma_rt_self, ") && c.contains("((const float[4]){"), "{c}");
    assert!(c.contains("lumart_wrong_length(") && c.contains("(uint64_t)s->stride * (uint64_t)s->height"), "{c}");
    assert!(c.contains("lumart_bytes_from(&"), "{c}");
}

/// The same run against services: 4 floats on a stream of 2 channels go as 2 frames; lent bytes come back as the
/// script's own; a FRAME's pixels are stride x height bytes; 3 floats on 2 channels stop the script at the call with
/// S0914 naming both numbers (the start ends there), and the service is not called. Every block is given back.
#[test]
fn counts_and_lent_bytes_run() {
    let Some((sdk, services)) = services() else { return };
    if !std::fs::read_to_string(sdk.join("luma_audio.h")).unwrap().contains("stream_channels") {
        eprintln!("skipped: the SDK's audio is before 1.1");
        return;
    }
    let text = "use svc.audio
use svc.assets
use svc.capture.camera

fn wrong(s: audio.Request) {
    log \"{audio.stream_push(s, [0.5, -0.5, 0.25])}\"
}

on start {
    if let b = assets.bytes(\"ObjectData/Kuribo.arc\", 0) {
        if let t = b.text() {
            log \"{b.len()} {t}\"
        }
    }
    let c = camera.open(camera.Desc { width: 0, height: 0, fps: 0 }, fn(e: camera.Event) {
        match e {
            camera.Event.Frame(socket, status, data) => log \"frame {data.width} {data.height} {data.stride} {data.pixels.len()} {data.time_ns}\"
            else => return
        }
    })
    if let s = audio.stream_open(audio.StreamDesc { rate: 48000, channels: 2, bus: 1, volume: 1.0 }, fn(e: audio.Event) { }) {
        log \"{audio.stream_push(s, [0.5, -0.5, 0.25, -0.25])}\"
        wrong(s)
    }
}
";
    let lines = run(&sdk, &services, text);
    assert_eq!(
        lines,
        [
            "bytes ObjectData/Kuribo.arc",
            "log 5 lent!",
            "stream_open 2 channels",
            "stream_push 5 2 frames 0.5 -0.5 0.25 -0.25",
            "log 2",
            "report main.lum:6 914 [audio.stream_push] [samples.len()] [3] [frames x 2]",
            "start 1",
            "log frame 2 2 8 16 77",
            "held 0",
        ]
    );
}

/// draw.native asks for permission native: every function of it is C's alone, said in its own words at a call (S0135,
/// Script.Why.SvcNative), and `Services::reach` (what `lumalink services` prints) marks it and its unreadable fields
/// `native`; nothing else of the SDK is less than `script`.
#[test]
fn draw_native_is_c_only_in_its_own_words() {
    let Some((_, services)) = services() else { return };
    let reach = services.reach();
    let not: Vec<_> = reach.iter().filter(|r| r.2 != "script").collect();
    assert!(not.iter().all(|r| r.1.starts_with("draw.native.") && r.2 == "native"), "{not:?}");
    for f in ["draw.native.at", "draw.native.backend"] {
        assert!(reach.contains(&("function", f.to_string(), "native")), "{f}: {not:?}");
    }
    assert!(reach.contains(&("function", "audio.stream_push".to_string(), "script")));
    let (module, d) = lumascript::parse_module("use svc.draw.native\n\non start {\n    log \"{native.backend()}\"\n}\n");
    assert!(d.is_empty(), "{d:?}");
    let sources = [Source { path: "main".into(), module }];
    let mut game = Game::new(HashMap::new(), Some(Types::from_json(GAME.as_bytes()).unwrap()));
    game.services = services.clone();
    let said = lumascript::on_compiler_stack(|| check::check(&sources, &[], ModFacts::of_game(&game)).diags).unwrap();
    let errors: Vec<_> = said.iter().filter(|(_, d)| !lumascript::diag::is_note(d.code)).collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let d = &errors[0].1;
    assert_eq!((d.code, d.at.line), (lumascript::diag::code::SVC_NOT_CALLABLE, 4), "{d:?}");
    assert!(d.args.iter().any(|(k, v)| *k == "why" && matches!(v, lumascript::diag::Arg::Phrase("Script.Why.SvcNative"))), "{d:?}");
}

/// What stays unreachable from scripts, as `Services::unreachable` lists it: draw.native's alone (C only). Checked on
/// an SDK whose audio gives stream_channels (1.1).
#[test]
fn what_stays_unreachable_is_listed() {
    let Some((sdk, services)) = services() else { return };
    if !std::fs::read_to_string(sdk.join("luma_audio.h")).unwrap().contains("stream_channels") {
        eprintln!("skipped: the SDK's audio is before 1.1");
        return;
    }
    let names: Vec<String> = services.unreachable().into_iter().map(|(n, _)| n).filter(|n| !n.starts_with("draw.native.")).collect();
    assert_eq!(names, Vec::<String>::new());
    assert!(services.unreachable().iter().any(|(n, _)| n == "draw.native.at"));
}
