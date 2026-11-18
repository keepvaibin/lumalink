//! A mod's own `test` blocks under `lumalink test` (docs/modding/reference/lumascript/tests.md).
//!
//! The mod is copied (its tests and built code left out) and built there as a test build for this system
//! (`build::build_as`): the `test` blocks written, which a mod's own build leaves out. The plain tests run in a process
//! of their own with no game: LumaLink started again with [`CHILD_ARG`] loads the test build's library and calls its
//! `luma_tests` with a host ([`child_main`]); a test that reaches the game, a service or another mod's function ends
//! the run there, named. The game tests run in the game, as one more harness test ([`write_game_test`]) whose mod is
//! the test build: they start after every `on start` and run one after another.
//!
//! Either way a test says facts (lumart.h, "tests"): `expect`, `error`, `test` and `tests`, read here into [`Outcome`].

use crate::say;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The argument that makes `lumalink` the plain tests' process: `lumalink --luma-tests-child <library> <luma.json>
/// --language <language>` (its error words in the language of the `lumalink test` that started it).
pub const CHILD_ARG: &str = "--luma-tests-child";

/// The harness test that runs the game tests: its folder's and its name (letters a test.json name never has
/// in a mod's own tests: a dot then a word LumaLink keeps).
pub const GAME_TEST: &str = "lumascript.game-tests";

/// The exit code of the plain tests' process when a test reached the game.
const TOUCHED: i32 = 3;

/// The mod's `test` blocks by name, in module and written order: the plain ones and the `test game` ones.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocks {
    pub plain: Vec<String>,
    pub game: Vec<String>,
}

impl Blocks {
    pub fn is_empty(&self) -> bool {
        self.plain.is_empty() && self.game.is_empty()
    }
}

/// The `test` blocks in the mod's LumaScript; none when it has none or its scripts cannot be read (the build then
/// says why).
pub fn blocks(folder: &Path) -> Blocks {
    let mut out = Blocks::default();
    let Ok(files) = crate::build::scripts(folder) else { return out };
    for (_, path) in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let found = lumascript::on_compiler_stack(|| {
            let (module, _) = lumascript::parse_module(&text);
            let mut found = Vec::new();
            for item in &module.items {
                if let lumascript::ast::ItemKind::Test { game, name, .. } = &item.kind {
                    let text: String = name.parts.iter().map(|p| if let lumascript::ast::StrPart::Text(t) = p { t.as_str() } else { "" }).collect();
                    found.push((*game, text));
                }
            }
            found
        });
        for (game, name) in found.unwrap_or_default() {
            if game {
                out.game.push(name);
            } else {
                out.plain.push(name);
            }
        }
    }
    out
}

/// The mod in `folder` copied to `to` (made anew) for its test build: everything but its built code, its tests and
/// hidden folders; its luma.json lists no platform, so the build records this system's alone.
pub fn copy_for_tests(folder: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        std::fs::remove_dir_all(to).with_context(|| say!("LumaLink.CannotWrite", path = to.display()))?;
    }
    copy_tree(folder, to, true)?;
    let manifest = to.join("luma.json");
    let text = std::fs::read_to_string(&manifest).with_context(|| say!("LumaLink.CannotRead", path = manifest.display()))?;
    let mut v: Value = serde_json::from_str(&text).with_context(|| say!("LumaLink.CannotRead", path = manifest.display()))?;
    if let Some(code) = v.get_mut("code").and_then(Value::as_object_mut) {
        code.insert("platforms".into(), Value::Array(Vec::new()));
    }
    std::fs::write(&manifest, serde_json::to_string_pretty(&v)? + "\n").with_context(|| say!("LumaLink.CannotWrite", path = manifest.display()))?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path, top: bool) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| say!("LumaLink.CannotMake", path = to.display()))?;
    for (path, dir) in crate::jobs::entries_of(from).with_context(|| say!("LumaLink.CannotRead", path = from.display()))? {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') || (top && (name == "code" || name == "tests")) {
            continue;
        }
        let target = to.join(&name);
        if dir {
            copy_tree(&path, &target, false)?;
        } else {
            std::fs::copy(&path, &target).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?;
        }
    }
    Ok(())
}

/// The harness test that runs the test build `under_test` (mod `id`) in the game: it ends once the game tests have all
/// ended (the fact `tests`), at most `limit` seconds, and passes when every one passed.
pub fn write_game_test(folder: &Path, under_test: &Path, id: &str, count: usize, limit: u32) -> Result<()> {
    std::fs::create_dir_all(folder).with_context(|| say!("LumaLink.CannotMake", path = folder.display()))?;
    let whole = std::path::absolute(under_test).with_context(|| say!("LumaLink.CannotRead", path = under_test.display()))?;
    let done = format!("mod.{id}.tests");
    let test = serde_json::json!({
        "harness": 1,
        "name": GAME_TEST,
        "about": "The mod's test game blocks, one after another",
        "mods": [whole.to_string_lossy()],
        "limit_seconds": limit,
        "steps": [
            { "after": { "fact": done, "exists": true }, "timeout": limit.saturating_mul(60), "end": true }
        ],
        "expect": [
            { "fact": done, "has": { "count": count, "passed": count } }
        ]
    });
    let path = folder.join("test.json");
    std::fs::write(&path, serde_json::to_string_pretty(&test)? + "\n").with_context(|| say!("LumaLink.CannotWrite", path = path.display()))?;
    Ok(())
}

/// How one test ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    Pass,
    Fail,
    /// A script error ended it (said in [`Block::errors`]).
    Error,
    /// The mod stopped first.
    Stopped,
    /// It never started: the run ended before it.
    NotRun,
    /// It reached this name of the game, service or other mod's function.
    Touched(String),
    /// The plain tests' process ended without a word, with this code: the test it was in.
    Crashed(i32),
}

/// A failed `expect`.
#[derive(Clone, Debug, PartialEq)]
pub struct Expect {
    pub file: String,
    pub line: u64,
    pub text: String,
    /// For a comparison: its operator and both sides, as JSON.
    pub sides: Option<(String, Value, Value)>,
}

/// A script error in a test: where, and in words (the game's own, from its mod log) or by its code and arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptError {
    pub file: String,
    pub line: u64,
    pub what: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub name: String,
    pub result: Ended,
    pub expects: Vec<Expect>,
    pub errors: Vec<ScriptError>,
}

/// One kind's tests as a run left them, in written order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub tests: Vec<Block>,
    /// The mod's other log lines (plain tests).
    pub log: Vec<String>,
    /// What the process said on its error stream (plain tests).
    pub said: String,
    /// Script errors before any test ran: the options and the modules' values being made (plain tests).
    pub before: Vec<ScriptError>,
}

impl Outcome {
    pub fn passed(&self) -> usize {
        self.tests.iter().filter(|t| t.result == Ended::Pass).count()
    }

    pub fn all_passed(&self) -> bool {
        self.passed() == self.tests.len()
    }

    fn new(names: &[String]) -> Outcome {
        Outcome { tests: names.iter().map(|n| Block { name: n.clone(), result: Ended::NotRun, expects: Vec::new(), errors: Vec::new() }).collect(), ..Outcome::default() }
    }

    /// The first test of `name` that has not ended (two tests may share a name).
    fn running(&mut self, name: &str) -> Option<&mut Block> {
        let at = self.tests.iter().position(|t| t.name == name && t.result == Ended::NotRun).or_else(|| self.tests.iter().rposition(|t| t.name == name))?;
        self.tests.get_mut(at)
    }

    /// One fact the tests said (`key` without the harness's `mod.<id>.`).
    fn fact(&mut self, key: &str, v: &Value) {
        let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let number = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        match key {
            "expect" => {
                let sides = match (v.get("op").and_then(Value::as_str), v.get("left"), v.get("right")) {
                    (Some(op), Some(l), Some(r)) => Some((op.to_string(), l.clone(), r.clone())),
                    _ => None,
                };
                let e = Expect { file: text("file"), line: number("line"), text: text("text"), sides };
                if let Some(t) = self.running(&text("test")) {
                    t.expects.push(e);
                }
            }
            "error" => {
                let args: Vec<String> = v.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let what = say!("LumaLink.Test.Block.Code", code = format!("S{:04}", number("code")), args = args.join(", "));
                let e = ScriptError { file: text("file"), line: number("line"), what };
                match self.running(&text("test")) {
                    Some(t) => t.errors.push(e),
                    None => self.before.push(e),
                }
            }
            "test" => {
                let result = match v.get("result").and_then(Value::as_str) {
                    Some("pass") => Ended::Pass,
                    Some("fail") => Ended::Fail,
                    Some("error") => Ended::Error,
                    Some("stopped") => Ended::Stopped,
                    _ => return,
                };
                if let Some(t) = self.running(&text("name")) {
                    t.result = result;
                }
            }
            _ => {}
        }
    }

    /// The test that was running when the run ended: the first that has not ended.
    fn ended_in(&mut self, result: Ended) {
        if let Some(t) = self.tests.iter_mut().find(|t| t.result == Ended::NotRun) {
            t.result = result;
        }
    }
}

/// The plain tests of the test build `library` run by `exe` (this LumaLink) in a process of their own, the mod's
/// options from `manifest` (its luma.json) at their defaults.
pub fn run_plain(exe: &Path, library: &Path, manifest: &Path, names: &[String]) -> Result<Outcome> {
    let out = std::process::Command::new(exe)
        .arg(CHILD_ARG)
        .arg(library)
        .arg(manifest)
        .arg("--language")
        .arg(crate::text::language())
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| say!("LumaLink.Test.Block.NoProcess", path = exe.display()))?;
    let mut outcome = read_plain(&String::from_utf8_lossy(&out.stdout), names);
    outcome.said = String::from_utf8_lossy(&out.stderr).into_owned();
    match out.status.code() {
        Some(0) => {}
        Some(TOUCHED) => {
            let what = String::from_utf8_lossy(&out.stdout).lines().rev().find_map(|l| l.strip_prefix("touched ").map(str::to_string)).unwrap_or_default();
            outcome.ended_in(Ended::Touched(what));
        }
        code => outcome.ended_in(Ended::Crashed(code.unwrap_or(-1))),
    }
    Ok(outcome)
}

/// The plain tests' process's lines: facts, `touched NAME`, and the mod's own log lines.
fn read_plain(stdout: &str, names: &[String]) -> Outcome {
    let mut outcome = Outcome::new(names);
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("fact ") {
            let (key, json) = rest.split_once(' ').unwrap_or((rest, "null"));
            match serde_json::from_str::<Value>(json) {
                Ok(v) => outcome.fact(key, &v),
                Err(_) => outcome.log.push(line.to_string()),
            }
        } else if !line.starts_with("touched ") {
            outcome.log.push(line.to_string());
        }
    }
    outcome
}

/// The game tests as the harness run in `run_folder` left them (its facts.jsonl): the mod `id`'s facts, and its script
/// errors in the game's words (the mod log's lines, `modlog`) for the test running when each was said.
pub fn game_outcome(run_folder: &Path, id: &str, names: &[String]) -> Outcome {
    let mut outcome = Outcome::new(names);
    let prefix = format!("mod.{id}.");
    let text = std::fs::read_to_string(run_folder.join("facts.jsonl")).unwrap_or_default();
    for line in text.lines() {
        let Ok(f) = serde_json::from_str::<Value>(line) else { continue };
        let (Some(key), Some(value)) = (f.get("key").and_then(Value::as_str), f.get("value")) else { continue };
        if let Some(own) = key.strip_prefix(&prefix) {
            outcome.fact(own, value);
        } else if key == "modlog" && value.get("mod").and_then(Value::as_str) == Some(id) {
            let file = value.get("file").and_then(Value::as_str).unwrap_or_default();
            if file.is_empty() {
                continue;
            }
            let e = ScriptError { file: file.to_string(), line: value.get("line").and_then(Value::as_u64).unwrap_or(0), what: value.get("text").and_then(Value::as_str).unwrap_or_default().to_string() };
            if let Some(t) = outcome.tests.iter_mut().find(|t| t.result == Ended::NotRun) {
                t.errors.push(e);
            }
        }
    }
    outcome
}

/// What `lumalink test` says of one kind's tests, a line each, in the language chosen.
pub fn lines(outcome: &Outcome, game: bool) -> Vec<String> {
    let mut out = Vec::new();
    for e in &outcome.before {
        out.push(say!("LumaLink.Test.Block.Before", file = e.file, line = e.line, what = e.what));
    }
    for t in &outcome.tests {
        let name = &t.name;
        out.push(match &t.result {
            Ended::Pass => say!("LumaLink.Test.Block.Pass", name = name),
            Ended::Fail => say!("LumaLink.Test.Block.Fail", name = name),
            Ended::Error => say!("LumaLink.Test.Block.Error", name = name),
            Ended::Stopped => say!("LumaLink.Test.Block.Stopped", name = name),
            Ended::NotRun => say!("LumaLink.Test.Block.NotRun", name = name),
            Ended::Touched(what) => say!("LumaLink.Test.Block.Touched", name = name, what = what),
            Ended::Crashed(code) => say!("LumaLink.Test.Block.Crashed", name = name, code = code),
        });
        for e in &t.expects {
            out.push(match &e.sides {
                Some((op, l, r)) => say!("LumaLink.Test.Block.ExpectSides", file = e.file, line = e.line, text = e.text, left = l, op = op, right = r),
                None => say!("LumaLink.Test.Block.Expect", file = e.file, line = e.line, text = e.text),
            });
        }
        for e in &t.errors {
            out.push(say!("LumaLink.Test.Block.ScriptError", file = e.file, line = e.line, what = e.what));
        }
    }
    let (passed, count) = (outcome.passed(), outcome.tests.len());
    out.push(if game { say!("LumaLink.Test.Block.GameSummary", passed = passed, count = count) } else { say!("LumaLink.Test.Block.PlainSummary", passed = passed, count = count) });
    out
}

// ---------------------------------------------------------------- the plain tests' process

/// What lumart's LumartTestHost is (lumart.h).
#[repr(C)]
struct Host {
    size: u32,
    say: extern "C" fn(*const std::ffi::c_char),
    touched: extern "C" fn(*const std::ffi::c_char),
    alloc: extern "C" fn(u64, u64) -> *mut std::ffi::c_void,
    free: extern "C" fn(*mut std::ffi::c_void),
    option: extern "C" fn(*const std::ffi::c_char, *mut std::ffi::c_char, u64) -> i64,
}

static OPTIONS: std::sync::OnceLock<std::collections::HashMap<String, String>> = std::sync::OnceLock::new();
static BLOCKS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<usize, std::alloc::Layout>>> = std::sync::OnceLock::new();

fn c_text(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: lumart passes NUL-ended text that lives for the call.
    unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

extern "C" fn host_say(line: *const std::ffi::c_char) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", c_text(line).replace(['\r', '\n'], " "));
    let _ = out.flush();
}

extern "C" fn host_touched(what: *const std::ffi::c_char) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "touched {}", c_text(what));
    let _ = out.flush();
    std::process::exit(TOUCHED);
}

extern "C" fn host_alloc(size: u64, align: u64) -> *mut std::ffi::c_void {
    let Ok(layout) = std::alloc::Layout::from_size_align(size.max(1) as usize, align.max(16) as usize) else { return std::ptr::null_mut() };
    // SAFETY: the layout's size is not zero.
    let p = unsafe { std::alloc::alloc(layout) };
    if !p.is_null() {
        BLOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).insert(p as usize, layout);
    }
    p.cast()
}

extern "C" fn host_free(p: *mut std::ffi::c_void) {
    let layout = BLOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).remove(&(p as usize));
    if let Some(layout) = layout {
        // SAFETY: p came from host_alloc with this layout and is given back once (taken out of the list above).
        unsafe { std::alloc::dealloc(p.cast(), layout) };
    }
}

extern "C" fn host_option(key: *const std::ffi::c_char, buf: *mut std::ffi::c_char, size: u64) -> i64 {
    let Some(value) = OPTIONS.get().and_then(|o| o.get(&c_text(key))) else { return -1 };
    let bytes = value.as_bytes();
    if !buf.is_null() && (bytes.len() as u64) < size {
        // SAFETY: buf holds `size` bytes, more than the text and its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.cast::<u8>(), bytes.len());
            *buf.add(bytes.len()) = 0;
        }
    }
    bytes.len() as i64
}

/// Each option of a luma.json at its default, as the mods service gives it to the mod's code: a number its shortest
/// decimal, a toggle "1" or "0", a choice or a text its own; a key option is the game's input, never asked here.
pub fn option_defaults(manifest: &Value) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for o in manifest.get("options").and_then(Value::as_array).into_iter().flatten() {
        let (Some(key), Some(kind)) = (o.get("key").and_then(Value::as_str), o.get("kind").and_then(Value::as_str)) else { continue };
        let d = o.get("default");
        let value = match (kind, d) {
            ("toggle", Some(Value::Bool(b))) => (if *b { "1" } else { "0" }).to_string(),
            ("number", Some(Value::Number(n))) => match n.as_i64() {
                Some(i) => i.to_string(),
                None => n.as_f64().map(|f| f.to_string()).unwrap_or_default(),
            },
            ("choice" | "text", Some(Value::String(s))) => s.clone(),
            _ => continue,
        };
        out.insert(key.to_string(), value);
    }
    out
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn SetErrorMode(mode: u32) -> u32;
    }

    /// No window of the system's about a crash: the parent says it (SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
    /// SEM_NOOPENFILEERRORBOX, for this process alone).
    pub fn quiet() {
        // SAFETY: sets this process's own error mode.
        unsafe { SetErrorMode(0x8003) };
    }

    pub fn open(path: &std::path::Path) -> *mut c_void {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-ended path.
        unsafe { LoadLibraryW(wide.as_ptr()) }
    }

    pub fn symbol(library: *mut c_void, name: &[u8]) -> *mut c_void {
        // SAFETY: a loaded library and a NUL-ended name.
        unsafe { GetProcAddress(library, name.as_ptr()) }
    }
}

#[cfg(unix)]
mod sys {
    use std::ffi::{c_char, c_int, c_void};
    use std::os::unix::ffi::OsStrExt;

    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(library: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    pub fn quiet() {}

    pub fn open(path: &std::path::Path) -> *mut c_void {
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return std::ptr::null_mut() };
        // SAFETY: a NUL-ended path; RTLD_NOW.
        unsafe { dlopen(c.as_ptr(), 2) }
    }

    pub fn symbol(library: *mut c_void, name: &[u8]) -> *mut c_void {
        // SAFETY: a loaded library and a NUL-ended name.
        unsafe { dlsym(library, name.as_ptr().cast()) }
    }
}

/// The plain tests' process ([`CHILD_ARG`] `<library> <luma.json>`): the library loaded, its `luma_tests` run with this
/// host; the facts and the mod's log on the output, a line each. 0 when the run reached its end; 3 when a test
/// reached the game (the last line names it); 2 when the library or its entry is not there (said on the error stream).
pub fn child_main(args: &[std::ffi::OsString]) -> i32 {
    sys::quiet();
    let (Some(library), Some(manifest)) = (args.first(), args.get(1)) else { return 2 };
    let manifest: Value = std::fs::read(manifest).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let _ = OPTIONS.set(option_defaults(&manifest));
    let library = PathBuf::from(library);
    let handle = sys::open(&library);
    if handle.is_null() {
        eprintln!("{}", say!("LumaLink.CannotRead", path = library.display()));
        return 2;
    }
    let entry = sys::symbol(handle, b"luma_tests\0");
    if entry.is_null() {
        eprintln!("{}", say!("LumaLink.Test.Block.NoEntry", path = library.display()));
        return 2;
    }
    let host = Host { size: std::mem::size_of::<Host>() as u32, say: host_say, touched: host_touched, alloc: host_alloc, free: host_free, option: host_option };
    // SAFETY: luma_tests is the test build's entry, `int32_t luma_tests(const LumartTestHost*)` (lumart.h); the host
    // lives across the call.
    let run: extern "C" fn(*const Host) -> i32 = unsafe { std::mem::transmute(entry) };
    // 1: the mod's values could not be made (its facts say why); 2: the host was refused.
    if run(&host) == 2 {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Facts in written order: an expect with its sides and one without, a script error, each test's end; a test the
    /// run never reached is not run; two tests of one name each get their own.
    #[test]
    fn facts_are_read_per_test() {
        let names: Vec<String> = ["a", "b", "b", "c"].iter().map(|s| s.to_string()).collect();
        let out = read_plain(
            "hello\n\
             fact expect {\"test\": \"a\", \"file\": \"main.lum\", \"line\": 3, \"text\": \"x == 2\", \"op\": \"==\", \"left\": 1, \"right\": 2}\n\
             fact expect {\"test\": \"a\", \"file\": \"main.lum\", \"line\": 4, \"text\": \"ok\"}\n\
             fact test {\"name\": \"a\", \"result\": \"fail\", \"failed\": 2}\n\
             fact error {\"test\": \"b\", \"file\": \"main.lum\", \"line\": 9, \"code\": 905, \"args\": [\"7\", \"5\"]}\n\
             fact test {\"name\": \"b\", \"result\": \"error\", \"failed\": 0}\n\
             fact test {\"name\": \"b\", \"result\": \"pass\", \"failed\": 0}\n",
            &names,
        );
        assert_eq!(out.log, vec!["hello".to_string()]);
        let results: Vec<&Ended> = out.tests.iter().map(|t| &t.result).collect();
        assert_eq!(results, vec![&Ended::Fail, &Ended::Error, &Ended::Pass, &Ended::NotRun]);
        assert_eq!(out.tests[0].expects[0].sides, Some(("==".to_string(), serde_json::json!(1), serde_json::json!(2))));
        assert_eq!(out.tests[0].expects[1].sides, None);
        assert_eq!((out.tests[1].errors[0].file.as_str(), out.tests[1].errors[0].line), ("main.lum", 9));
        assert!(out.tests[1].errors[0].what.contains("S0905") && out.tests[1].errors[0].what.contains("7, 5"), "{:?}", out.tests[1].errors);
        assert!(!out.all_passed() && out.passed() == 1);
    }

    /// The options a plain test reads, as the game gives them unchanged.
    #[test]
    fn options_are_their_defaults_as_the_mods_service_gives_them() {
        let m = serde_json::json!({"options": [
            {"key": "port", "kind": "number", "default": 7000},
            {"key": "speed", "kind": "number", "default": 2.5},
            {"key": "on", "kind": "toggle", "default": true},
            {"key": "who", "kind": "choice", "default": "b"},
            {"key": "say", "kind": "text", "default": "hi"},
            {"key": "jump", "kind": "key", "default": "A"}
        ]});
        let d = option_defaults(&m);
        assert_eq!(d.get("port").map(String::as_str), Some("7000"));
        assert_eq!(d.get("speed").map(String::as_str), Some("2.5"));
        assert_eq!(d.get("on").map(String::as_str), Some("1"));
        assert_eq!(d.get("who").map(String::as_str), Some("b"));
        assert_eq!(d.get("say").map(String::as_str), Some("hi"));
        assert!(!d.contains_key("jump"));
    }

    /// The copy for the test build leaves out the built code, the tests and hidden folders, and lists no platform.
    #[test]
    fn the_copy_leaves_out_code_and_tests() {
        let dir = tempfile::tempdir().unwrap();
        let m = dir.path().join("moon");
        for d in ["source", "code/windows-x64", "tests/t", ".git", "assets/sub"] {
            std::fs::create_dir_all(m.join(d)).unwrap();
        }
        std::fs::write(m.join("luma.json"), r#"{"id": "moon", "code": {"platforms": ["windows-x64", "linux-x64"]}}"#).unwrap();
        std::fs::write(m.join("source/main.lum"), "test \"t\" {\n    expect true\n}\n\ntest game \"g\" {\n    expect true\n}\n").unwrap();
        std::fs::write(m.join("assets/sub/a.png"), b"png").unwrap();
        let to = dir.path().join("copy");
        copy_for_tests(&m, &to).unwrap();
        assert!(to.join("source/main.lum").is_file() && to.join("assets/sub/a.png").is_file());
        assert!(!to.join("code").exists() && !to.join("tests").exists() && !to.join(".git").exists());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(to.join("luma.json")).unwrap()).unwrap();
        assert_eq!(v["code"]["platforms"], serde_json::json!([]));
        assert_eq!(blocks(&m), Blocks { plain: vec!["t".into()], game: vec!["g".into()] });
    }

    /// The game tests' harness test ends on the count and passes only when every one passed.
    #[test]
    fn the_game_test_waits_for_the_count() {
        let dir = tempfile::tempdir().unwrap();
        write_game_test(dir.path(), dir.path(), "moon", 3, 180).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("test.json")).unwrap()).unwrap();
        assert_eq!(v["name"], GAME_TEST);
        assert_eq!(v["steps"][0]["after"]["fact"], "mod.moon.tests");
        assert_eq!(v["expect"][0]["has"], serde_json::json!({"count": 3, "passed": 3}));
    }

    /// The game's facts: the mod's own, and the mod log's lines with a place as the running test's errors.
    #[test]
    fn game_facts_are_read_from_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let facts = [
            r#"{"frame": 0, "key": "mod.moon.test", "value": {"name": "a", "result": "pass", "failed": 0}}"#,
            r#"{"frame": 2, "key": "modlog", "value": {"mod": "moon", "file": "main.lum", "line": 7, "text": "division by zero in a / b"}}"#,
            r#"{"frame": 2, "key": "modlog", "value": {"mod": "other", "file": "x.lum", "line": 1, "text": "not ours"}}"#,
            r#"{"frame": 2, "key": "mod.moon.test", "value": {"name": "b", "result": "error", "failed": 0}}"#,
        ];
        std::fs::write(dir.path().join("facts.jsonl"), facts.join("\n")).unwrap();
        let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let out = game_outcome(dir.path(), "moon", &names);
        assert_eq!(out.tests[0].result, Ended::Pass);
        assert_eq!(out.tests[1].result, Ended::Error);
        assert_eq!(out.tests[1].errors, vec![ScriptError { file: "main.lum".into(), line: 7, what: "division by zero in a / b".into() }]);
        assert_eq!(out.tests[2].result, Ended::NotRun);
    }
}
