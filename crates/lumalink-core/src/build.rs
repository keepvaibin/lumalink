//! `lumalink build` (docs/modding/language/compiled-form.md, section 1): a mod's LumaScript read, checked against the game's description,
//! written as C (the back end, `lumascript::emit`) and built with clang and lld into `code/<platform>/mod.<ending>`.
//! Nothing of it runs in the game but the library it makes.
//!
//! The C is written once; the nine platforms of luma-format 6 differ in the target and the flags only ([`TARGETS`],
//! proved from this machine with no target's SDK). Every
//! platform listed is built; one that cannot be is an error naming it, after the others are built.

use lumascript::check::{self, ContentNames, ModFacts, OptionKind, OptionSpec, Source};
use lumascript::emit;
use lumascript::game::Game;
use lumascript::Diagnostic;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The LLVM release every library is built with (docs/modding/language/compiled-form.md, sections 1.1, 1.2: the same sources give the same bytes).
pub const LLVM: &str = "22.1.6";

/// The flags of abi 1.1 every platform shares (kept with the back end, whose tests compile with them).
pub const COMMON_FLAGS: &[&str] = lumascript::emit::C_FLAGS;

/// The flags a mod's C++ sources take in place of `-std=c17` (docs/modding/language/compiled-form.md, section 9): no exceptions, no RTTI, no standard library.
pub const CXX_FLAGS: &[&str] = &["-std=c++20", "-fno-exceptions", "-fno-rtti"];

/// What one build made.
#[derive(Debug)]
pub struct Built {
    /// Each platform built, in luma-format 6's order: its library and the library's size.
    pub libraries: Vec<(&'static str, PathBuf, u64)>,
    /// The generated C's size, lumart not counted.
    pub c_bytes: u64,
    pub imports: Vec<(String, String, &'static str)>,
    /// Game calls passing made text (docs/modding/language/compiled-form.md): source file, line, function.
    pub kept: Vec<(String, u32, String)>,
    /// The platforms written into the mod's luma.json's `code.platforms` by this build (those it lacked).
    pub recorded: Vec<&'static str>,
    /// The functions it gives other mods (name, signature); with any, `code/<id>.lumi` was written.
    pub exports: Vec<(String, String)>,
    /// The checker's notes (warnings), by file (`source/<module>.lum`): said, the build goes on.
    pub notes: Vec<(String, Diagnostic)>,
}

#[derive(Debug)]
pub enum BuildError {
    Read(PathBuf, std::io::Error),
    Write(PathBuf, std::io::Error),
    NoId(PathBuf),
    NoScripts(PathBuf),
    /// Every error, by file (`source/<module>.lum`); notes are not here.
    Script(Vec<(String, Diagnostic)>),
    NoSdk(PathBuf),
    /// The SDK's linker script the ELF platforms link with (luma.h, docs/modding/lumalink/loader.md, section 8) is not there.
    NoLinkerScript(PathBuf),
    /// A platform asked for that luma-format 6 does not have.
    UnknownPlatform(String),
    NoClang(PathBuf, std::io::Error),
    ClangVersion { path: PathBuf, found: String },
    /// Each listed platform clang could not build, by name, with its exit code and output; `built` were built (and
    /// recorded in luma.json).
    Platforms { failed: Vec<(&'static str, i32, String)>, built: Vec<&'static str> },
}

/// One platform of luma-format 6 as `lumalink build` makes it: the LLVM triple, the library's name, the flags each of
/// its compiles and its link take beyond [`COMMON_FLAGS`], and its link's own. `{sdk}` in a flag is the SDK's folder.
#[derive(Debug)]
pub struct Target {
    pub platform: &'static str,
    pub triple: &'static str,
    pub library: &'static str,
    pub code: &'static [&'static str],
    pub link: &'static [&'static str],
}

/// PE (Windows): no entry, no default library, no import library, no time stamp.
const PE: &[&str] = &["-shared", "-fuse-ld=lld", "-Wl,/NOENTRY", "-Wl,/NODEFAULTLIB", "-Wl,/NOIMPLIB", "-Wl,/Brepro"];
/// ELF (Linux, Android): position-independent; constructors kept from the system's loader, in luma_ctors by the SDK's
/// linker script (luma.h, docs/modding/lumalink/loader.md, section 8), so not in .init_array.
const ELF_CODE: &[&str] = &["-fPIC", "-fno-use-init-array"];
const ELF: &[&str] = &[
    "-shared", "-fuse-ld=lld", "-Wl,--build-id=none", "-Wl,--hash-style=gnu", "-Wl,-z,defs", "-Wl,-z,noexecstack", "-Wl,-T,{sdk}/luma_ctors.ld",
];
/// Android 15 devices may use 16 KiB pages: a library aligned to less does not load there.
const ANDROID: &[&str] = &[
    "-shared", "-fuse-ld=lld", "-Wl,--build-id=none", "-Wl,--hash-style=gnu", "-Wl,-z,defs", "-Wl,-z,noexecstack", "-Wl,-T,{sdk}/luma_ctors.ld",
    "-Wl,-z,max-page-size=16384",
];
/// Android's arm64 default calls the __aarch64_cas* helpers of a runtime a mod does not link.
const ANDROID_ARM_CODE: &[&str] = &["-fPIC", "-fno-use-init-array", "-mno-outline-atomics"];
/// Mach-O (macOS, iOS): no symbol left to the system, signed ad hoc (iOS is signed again on the device).
const MACHO_CODE: &[&str] = &["-fPIC"];
const MACOS: &[&str] = &[
    "-dynamiclib", "-fuse-ld=lld", "-Wl,-undefined,error", "-Wl,-adhoc_codesign", "-Wl,-install_name,@rpath/mod.dylib",
    "-Wl,-platform_version,macos,12.0,12.0",
];
const IOS: &[&str] = &[
    "-dynamiclib", "-fuse-ld=lld", "-Wl,-undefined,error", "-Wl,-adhoc_codesign", "-Wl,-install_name,@rpath/mod.dylib",
    "-Wl,-platform_version,ios,15.0,15.0",
];

/// The nine, in luma-format 6's order (`luma_validate::PLATFORMS`).
pub static TARGETS: [Target; 9] = [
    Target { platform: "windows-x64", triple: "x86_64-pc-windows-msvc", library: "mod.dll", code: &[], link: PE },
    Target { platform: "windows-arm64", triple: "aarch64-pc-windows-msvc", library: "mod.dll", code: &[], link: PE },
    Target { platform: "linux-x64", triple: "x86_64-unknown-linux-gnu", library: "mod.so", code: ELF_CODE, link: ELF },
    Target { platform: "linux-arm64", triple: "aarch64-unknown-linux-gnu", library: "mod.so", code: ELF_CODE, link: ELF },
    Target { platform: "macos-arm64", triple: "arm64-apple-macos12.0", library: "mod.dylib", code: MACHO_CODE, link: MACOS },
    Target { platform: "macos-x64", triple: "x86_64-apple-macos12.0", library: "mod.dylib", code: MACHO_CODE, link: MACOS },
    Target { platform: "android-arm64", triple: "aarch64-linux-android24", library: "mod.so", code: ANDROID_ARM_CODE, link: ANDROID },
    Target { platform: "android-x64", triple: "x86_64-linux-android24", library: "mod.so", code: ELF_CODE, link: ANDROID },
    Target { platform: "ios-arm64", triple: "arm64-apple-ios15.0", library: "mod.dylib", code: MACHO_CODE, link: IOS },
];

/// The platform's build, or None when luma-format 6 has no such platform.
pub fn target(platform: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.platform == platform)
}

/// The clang to use: `LUMALINK_CLANG`, then LLVM's own folder, then `clang` on the path.
pub fn default_clang() -> PathBuf {
    if let Some(p) = std::env::var_os("LUMALINK_CLANG") {
        return PathBuf::from(p);
    }
    let llvm = Path::new("C:/Program Files/LLVM/bin/clang.exe");
    if cfg!(windows) && llvm.is_file() {
        return llvm.to_path_buf();
    }
    PathBuf::from("clang")
}

/// The modules under `source/`: (module path with `/`, without `.lum`; file), sorted by path (docs/modding/language/compiled-form.md, section 1.2).
fn scripts(folder: &Path) -> Result<Vec<(String, PathBuf)>, BuildError> {
    let source = folder.join("source");
    let mut out = Vec::new();
    if !source.is_dir() {
        return Ok(out);
    }
    for entry in walkdir::WalkDir::new(&source).sort_by_file_name() {
        let entry = entry.map_err(|e| BuildError::Read(source.clone(), e.into()))?;
        let path = entry.path();
        if entry.file_type().is_file() && path.extension().is_some_and(|e| e == "lum") {
            let rel = path.strip_prefix(&source).unwrap_or(path).with_extension("");
            let module = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            out.push((module, path.to_path_buf()));
        }
    }
    out.sort();
    Ok(out)
}

/// The mod's own C and C++ under `source/` (docs/modding/language/language.md, section 13, docs/modding/language/compiled-form.md, section 9), each sorted by path (docs/modding/language/compiled-form.md, section 1.2): C files, C++ files,
/// and the headers `use c` may name (paths from `source/`, with `/`).
pub struct CSources {
    pub c: Vec<PathBuf>,
    pub cxx: Vec<PathBuf>,
    pub headers: Vec<String>,
}

pub fn c_sources(folder: &Path) -> Result<CSources, BuildError> {
    let source = folder.join("source");
    let mut out = CSources { c: Vec::new(), cxx: Vec::new(), headers: Vec::new() };
    if !source.is_dir() {
        return Ok(out);
    }
    for entry in walkdir::WalkDir::new(&source).sort_by_file_name() {
        let entry = entry.map_err(|e| BuildError::Read(source.clone(), e.into()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "c" => out.c.push(path.to_path_buf()),
            "cpp" | "cc" | "cxx" => out.cxx.push(path.to_path_buf()),
            "h" | "hpp" | "hh" => {
                let rel = path.strip_prefix(&source).unwrap_or(path);
                out.headers.push(rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"));
            }
            _ => {}
        }
    }
    out.c.sort();
    out.cxx.sort();
    out.headers.sort();
    Ok(out)
}

/// What the build reads from the mod's luma.json: its id, its version, and the ids of the mods it needs.
struct Manifest {
    id: String,
    version: String,
    needs: Vec<String>,
    options: Vec<OptionSpec>,
}

fn manifest(folder: &Path) -> Result<Manifest, BuildError> {
    let path = folder.join("luma.json");
    let bytes = std::fs::read(&path).map_err(|e| BuildError::Read(path.clone(), e))?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| BuildError::NoId(path.clone()))?;
    let id = json.get("id").and_then(|v| v.as_str()).map(str::to_string).ok_or(BuildError::NoId(path))?;
    let version = json.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();
    // luma-format: `needs` is a list of {id, version}; check says what is wrong with any other form.
    let needs = json
        .get("needs")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|n| n.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect())
        .unwrap_or_default();
    // luma-format: `options` is a list of {key, kind, default, ...}; `option` declarations are checked against it.
    // An entry check refuses (no key, a kind of no five) is left out here: check says what is wrong with it.
    let options = json.get("options").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(option_spec).collect()).unwrap_or_default();
    Ok(Manifest { id, version, needs, options })
}

/// One `options` entry as the checker reads it: its key and kind; a number is whole when its default and the min, max
/// and step present are all whole numbers.
fn option_spec(o: &serde_json::Value) -> Option<OptionSpec> {
    let key = o.get("key")?.as_str()?.to_string();
    let kind = match o.get("kind")?.as_str()? {
        "toggle" => OptionKind::Toggle,
        "number" => {
            let whole = ["default", "min", "max", "step"].iter().filter_map(|k| o.get(*k)).all(|v| v.as_f64().is_some_and(|n| n.fract() == 0.0));
            OptionKind::Number { whole }
        }
        "text" => OptionKind::Text,
        "choice" => OptionKind::Choice,
        "key" => OptionKind::Key,
        _ => return None,
    };
    Some(OptionSpec { key, kind })
}

/// The services an SDK include folder's `luma_*.h` declare ([`lumascript::svc::Services::of_sdk`]), and the ones whose
/// face could not be read with why. A folder that cannot be listed gives none.
pub fn services_of_sdk(sdk: &Path) -> (lumascript::svc::Services, Vec<lumascript::svc::ServicesError>) {
    let mut headers = std::collections::BTreeMap::new();
    if let Ok(dir) = std::fs::read_dir(sdk) {
        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("luma_") && name.ends_with(".h") {
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    headers.insert(name, text);
                }
            }
        }
    }
    lumascript::svc::Services::of_sdk(&headers)
}

/// Whether the SDK's luma.h gives LumaModInfo a `runtime_layout` field (the mod's record then carries the runtime
/// layout). An SDK without it, or none to read, builds the record as before.
pub fn mod_info_has_layout(sdk: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(sdk.join("luma.h")) else { return false };
    let Some(end) = text.find("} LumaModInfo;") else { return false };
    let start = text[..end].rfind("typedef struct LumaModInfo").unwrap_or(0);
    // The fields without their comments: a comment that names the field is not the field.
    let mut code = String::new();
    let mut rest = &text[start..end];
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map_or("", |e| &r[e + 2..]);
        } else if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map_or("", |e| &r[e..]);
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            code.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(|w| w == "runtime_layout")
}

/// Why the game's tables do not read ([`game_tables`]).
#[derive(Debug)]
pub enum TablesError {
    Read(PathBuf, std::io::Error),
    NotJson { path: PathBuf, line: usize, column: usize },
    /// `tables` is not a list of {name, type, count, size}.
    Shape(PathBuf),
}

impl std::fmt::Display for TablesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TablesError::Read(path, e) => write!(f, "{}: {e}", path.display()),
            TablesError::NotJson { path, line, column } => write!(f, "{}:{line}:{column}: not JSON", path.display()),
            TablesError::Shape(path) => write!(f, "{}: `tables` is not a list of name, type, count and size", path.display()),
        }
    }
}

/// The tables of the game's data (docs/modding/language/numbers.md: luma_numbers.json's `tables`, name to element
/// type, count and size) from a self-description folder: its `luma_numbers.json`, or the one in its `luma_types`
/// folder or beside it (a build folder's `luma`). None when there is none: `set_table` and `table` are then refused.
pub fn game_tables(names: &Path) -> Result<Option<std::collections::HashMap<String, lumascript::game::GameTable>>, TablesError> {
    let file = "luma_numbers.json";
    let candidates = [names.join(file), names.join("luma_types").join(file), names.join("..").join("luma_types").join(file)];
    let Some(path) = candidates.into_iter().find(|p| p.is_file()) else { return Ok(None) };
    let bytes = std::fs::read(&path).map_err(|e| TablesError::Read(path.clone(), e))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| TablesError::NotJson { path: path.clone(), line: e.line(), column: e.column() })?;
    let shape = || TablesError::Shape(path.clone());
    let rows = value.get("tables").and_then(|t| t.as_array()).ok_or_else(shape)?;
    let mut out = std::collections::HashMap::with_capacity(rows.len());
    for row in rows {
        let text = |k: &str| row.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let number = |k: &str| row.get(k).and_then(|v| v.as_u64());
        let (Some(name), Some(ty), Some(count), Some(size)) = (text("name"), text("type"), number("count"), number("size")) else {
            return Err(shape());
        };
        out.insert(name, lumascript::game::GameTable { ty, count, size });
    }
    Ok(Some(out))
}

/// Why the game's object factory does not read ([`factory_rows`]).
#[derive(Debug)]
pub enum FactoryError {
    NotJson { path: PathBuf, line: usize, column: usize },
    /// `factory` is not a list of {name, class}.
    Shape(PathBuf),
}

/// The game's object factory (docs/modding/language/game-description.md, section 9: `factory`, [{name, class, creator}]
/// in cCreateTable's order) from the placement facts beside the rows (smg_game.luma-placement.json), for the checker's
/// `made by` and `replaces`; None when the folder has no such file (the kernel then refuses a wrong row at define_class).
pub fn factory_rows(names: &Path) -> Result<Option<Vec<(String, Option<String>)>>, FactoryError> {
    let path = names.join("smg_game.luma-placement.json");
    let Ok(bytes) = std::fs::read(&path) else { return Ok(None) };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| FactoryError::NotJson { path: path.clone(), line: e.line(), column: e.column() })?;
    let shape = || FactoryError::Shape(path.clone());
    let rows = value.get("factory").and_then(|f| f.as_array()).ok_or_else(shape)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let name = row.get("name").and_then(|n| n.as_str()).ok_or_else(shape)?;
        let class = match row.get("class") {
            Some(serde_json::Value::String(c)) => Some(c.clone()),
            Some(serde_json::Value::Null) => None,
            _ => return Err(shape()),
        };
        out.push((name.to_string(), class));
    }
    Ok(Some(out))
}

/// Why the game a mod is checked against cannot be made ([`game_of`]).
#[derive(Debug)]
pub enum GameError {
    Tables(TablesError),
    Factory(FactoryError),
}

/// The game a mod's LumaScript is checked against: the self-description's rows (`rows`, read from `names`), its
/// object factory and its tables, and the services the SDK folder's headers declare (none without an SDK). The
/// services whose face does not read come back with it. `lumalink build` and the editor server both make it here.
pub fn game_of(rows: &luma_validate::names::Names, names: &Path, sdk: Option<&Path>) -> Result<(Game, Vec<lumascript::svc::ServicesError>), GameError> {
    let mut game = Game::from_names(rows);
    game.factory = factory_rows(names).map_err(GameError::Factory)?;
    game.tables = game_tables(names).map_err(GameError::Tables)?;
    let mut unread = Vec::new();
    if let Some(sdk) = sdk {
        let (services, errors) = services_of_sdk(sdk);
        game.services = services;
        unread = errors;
    }
    Ok((game, unread))
}

/// The interface file of library mod `id` (docs/modding/language/compiled-form.md) for a mod in `folder`: the needed mod's
/// own folder beside it (`../<id>/code/<id>.lumi`), as LumaLink's mod folder and a modder's workspace both hold mods.
/// None when it has none (a needed mod with no functions for others).
pub fn interface_of(folder: &Path, id: &str) -> Option<PathBuf> {
    let p = folder.parent()?.join(id).join("code").join(format!("{id}.lumi"));
    p.is_file().then_some(p)
}

/// The names content literals are checked against (docs/modding/language/language.md, section 10): the mod's own assets
/// (`layers/<layer>/assets/<path>`, docs/modding/format/luma.md, section 5) by their path, and the needed mods' beside it
/// (`../<id>/`) as `<id>:<path>`. A sound is `<path>.sound.json` (sound.md), a movie `<path>.movie.json`
/// (movies.md), a font `<path>.font.json` (layout.md 2a), a model `<path>.glb` or `.gltf` but not its levels
/// (`<path>Middle`, `<path>Low`, `.lodN`) nor collision or a camera (models.md); `data` is any asset by its path
/// with its ending. Effects (names inside effects.json), layouts, nerves and texts (the game's message tables are not
/// in the build) are given no set, so the checker leaves them unchecked. A bare model name may be one of the game's
/// archives (ObjectData, MapPartsData), which the build has no list of: one not the mod's is left to `check --game`.
pub fn content_names(folder: &Path, needs: &[String]) -> ContentNames {
    let mut content = ContentNames::default();
    content.game_unlisted.insert("model".to_string());
    for word in ["sound", "movie", "font", "model", "data"] {
        content.names.insert(word.to_string(), Default::default());
    }
    add_assets(&mut content, folder, "");
    if let Some(parent) = folder.parent() {
        for id in needs {
            add_assets(&mut content, &parent.join(id), &format!("{id}:"));
        }
    }
    content
}

fn add_assets(content: &mut ContentNames, folder: &Path, prefix: &str) {
    let Ok(layers) = std::fs::read_dir(folder.join("layers")) else { return };
    let mut paths = Vec::new();
    for layer in layers.flatten() {
        let assets = layer.path().join("assets");
        for entry in walkdir::WalkDir::new(&assets).into_iter().flatten().filter(|e| e.file_type().is_file()) {
            if let Ok(rel) = entry.path().strip_prefix(&assets) {
                let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                paths.push(parts.join("/"));
            }
        }
    }
    let mut models = std::collections::BTreeSet::new();
    for p in &paths {
        let lower = p.to_ascii_lowercase();
        for (ending, word) in [(".sound.json", "sound"), (".movie.json", "movie"), (".font.json", "font")] {
            if lower.ends_with(ending) {
                content.names.get_mut(word).unwrap().insert(format!("{prefix}{}", &p[..p.len() - ending.len()]));
            }
        }
        let stem = [".glb", ".gltf"].iter().find(|e| lower.ends_with(*e)).map(|e| &p[..p.len() - e.len()]);
        if let Some(stem) = stem {
            let inner = stem.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            let level = inner.as_deref().is_some_and(|e| e.len() > 3 && e.starts_with("lod") && e[3..].bytes().all(|b| b.is_ascii_digit()));
            if !level && !matches!(inner.as_deref(), Some("kcl" | "canm" | "bdl" | "bmd")) {
                models.insert(stem.to_string());
            }
        }
        content.names.get_mut("data").unwrap().insert(format!("{prefix}{p}"));
    }
    let levels: Vec<String> = models
        .iter()
        .filter(|m| ["Middle", "Low"].iter().any(|s| m.strip_suffix(s).is_some_and(|base| models.contains(base))))
        .cloned()
        .collect();
    for m in models.into_iter().filter(|m| !levels.contains(m)) {
        content.names.get_mut("model").unwrap().insert(format!("{prefix}{m}"));
    }
}

/// One needed mod's interface file as read from the disk: its id, its path and its text.
#[derive(Clone, Debug)]
pub struct Interface {
    pub id: String,
    pub path: PathBuf,
    pub text: String,
}

/// What a mod's LumaScript is checked with besides its modules: the facts of its luma.json, its C headers, the content
/// its literals may name and the interfaces of the mods it needs. `lumalink build` and the editor server both read
/// them here, so the editor's diagnostics are the build's.
#[derive(Clone, Debug)]
pub struct CheckInputs {
    pub id: String,
    pub version: String,
    pub needs: Vec<String>,
    pub options: Vec<OptionSpec>,
    pub headers: Vec<String>,
    pub content: ContentNames,
    pub interfaces: Vec<Interface>,
}

impl CheckInputs {
    /// The inputs of the mod in `folder`. Err: luma.json cannot be read or has no id, or a file cannot be read.
    pub fn read(folder: &Path) -> Result<CheckInputs, BuildError> {
        let Manifest { id, version, needs, options } = manifest(folder)?;
        let headers = c_sources(folder)?.headers;
        let mut interfaces = Vec::new();
        for need in &needs {
            if let Some(path) = interface_of(folder, need) {
                let text = std::fs::read_to_string(&path).map_err(|e| BuildError::Read(path.clone(), e))?;
                interfaces.push(Interface { id: need.clone(), path, text });
            }
        }
        let content = content_names(folder, &needs);
        Ok(CheckInputs { id, version, needs, options, headers, content, interfaces })
    }

    /// The needed mods' interfaces as the checker's libraries; each one refused is said by `<id>.lumi` (with
    /// `: module <path>` for a module that does not read). Parses: call it on the compiler's stack.
    pub fn libraries(&self) -> (Vec<check::Library>, Vec<(String, Diagnostic)>) {
        let mut libraries = Vec::new();
        let mut errors = Vec::new();
        for Interface { id: need, text, .. } in &self.interfaces {
            // A needed mod built with another runtime layout: none of its functions would bind at load (its
            // signatures are taken over the layout), so it is named here with what to do.
            let layout = lumascript::lumi::layout_of(text);
            if layout != lumascript::lumi::RUNTIME_LAYOUT {
                errors.push((
                    format!("{need}.lumi"),
                    Diagnostic::new(lumascript::diag::code::NEED_OTHER_LAYOUT, lumascript::ast::Pos { line: 1, col: 1 })
                        .with("need", lumascript::diag::Arg::Text(need.clone()))
                        .with("found", lumascript::diag::Arg::Text(layout.to_string()))
                        .with("layout", lumascript::diag::Arg::Text(lumascript::lumi::RUNTIME_LAYOUT.to_string())),
                ));
                continue;
            }
            match lumascript::lumi::read(text) {
                Ok(modules) => libraries.push(check::Library { id: need.clone(), modules }),
                Err(e) => errors.extend(e.into_iter().map(|(m, d)| (format!("{need}.lumi: module {m}"), d))),
            }
        }
        (libraries, errors)
    }

    /// The checker's facts over these inputs and `game`.
    pub fn facts<'a>(&'a self, game: &'a Game) -> ModFacts<'a> {
        ModFacts { c_headers: &self.headers, needs: &self.needs, options: &self.options, content: Some(&self.content), ..ModFacts::of_game(game) }
    }
}

/// Checks and writes the mod's C into `work/lum/` (the front end and the back end, on the compiler's stack).
pub fn generate(folder: &Path, game: &Game, work: &Path) -> Result<(String, emit::Emitted, Vec<(String, Diagnostic)>), BuildError> {
    let inputs = CheckInputs::read(folder)?;
    let (id, version) = (inputs.id.clone(), inputs.version.clone());
    let files = scripts(folder)?;
    if files.is_empty() {
        return Err(BuildError::NoScripts(folder.to_path_buf()));
    }
    let mut texts = Vec::new();
    for (module, path) in &files {
        let text = std::fs::read_to_string(path).map_err(|e| BuildError::Read(path.clone(), e))?;
        texts.push((module.clone(), text));
    }
    let result = lumascript::on_compiler_stack(|| {
        let mut errors: Vec<(String, Diagnostic)> = Vec::new();
        let mut sources = Vec::new();
        for (module, text) in &texts {
            let (parsed, diags) = lumascript::parse_module(text);
            errors.extend(diags.into_iter().map(|d| (format!("source/{module}.lum"), d)));
            sources.push(Source { path: module.clone(), module: parsed });
        }
        // The standard module after the mod's own (`MakePlace`).
        sources.push(check::std_source());
        let (libraries, refused) = inputs.libraries();
        errors.extend(refused);
        if !errors.is_empty() {
            return Err(errors);
        }
        let checked = check::check(&sources, &libraries, inputs.facts(game));
        let (notes, said): (Vec<(String, Diagnostic)>, Vec<(String, Diagnostic)>) =
            checked.diags.iter().map(|(m, d)| (format!("source/{m}.lum"), d.clone())).partition(|(_, d)| lumascript::diag::is_note(d.code));
        if !said.is_empty() {
            return Err(said);
        }
        let module_texts: Vec<String> = texts.iter().map(|(_, t)| t.clone()).collect();
        emit::emit_with(&id, &version, &sources, &module_texts, &libraries, &checked, game)
            .map(|e| (e, notes))
            .map_err(|e| e.into_iter().map(|(m, d)| (format!("source/{m}.lum"), d)).collect())
    });
    let (emitted, notes) = match result {
        Ok(Ok(e)) => e,
        Ok(Err(errors)) => return Err(BuildError::Script(errors)),
        Err(d) => return Err(BuildError::Script(vec![(String::new(), d)])),
    };
    // A library mod's interface, beside its libraries (item 7): written when it gives other mods a function.
    if let (Some(lumi), false) = (&emitted.lumi, emitted.exports.is_empty()) {
        let dir = folder.join("code");
        std::fs::create_dir_all(&dir).map_err(|e| BuildError::Write(dir.clone(), e))?;
        let path = dir.join(format!("{id}.lumi"));
        std::fs::write(&path, lumi).map_err(|e| BuildError::Write(path.clone(), e))?;
    }
    for (rel, text) in &emitted.files {
        let path = work.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| BuildError::Write(parent.to_path_buf(), e))?;
        }
        std::fs::write(&path, text).map_err(|e| BuildError::Write(path.clone(), e))?;
    }
    Ok((id, emitted, notes))
}

/// The clang's release, from `clang --version` (`clang version 22.1.6`).
fn clang_version(clang: &Path) -> Result<String, BuildError> {
    let out = Command::new(clang).arg("--version").output().map_err(|e| BuildError::NoClang(clang.to_path_buf(), e))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let found = text.split_whitespace().skip_while(|w| *w != "version").nth(1).unwrap_or("").to_string();
    Ok(found)
}

/// The targets `platforms` names, in luma-format 6's order and once each; all nine when it names none.
pub fn targets(platforms: &[String]) -> Result<Vec<&'static Target>, BuildError> {
    if platforms.is_empty() {
        return Ok(TARGETS.iter().collect());
    }
    for p in platforms {
        if target(p).is_none() {
            return Err(BuildError::UnknownPlatform(p.clone()));
        }
    }
    Ok(TARGETS.iter().filter(|t| platforms.iter().any(|p| p == t.platform)).collect())
}

/// Builds the mod in `folder` for `platforms` (all nine when empty): its C written once to `work`, each library to
/// `folder/code/<platform>/mod.<ending>`, each platform built recorded in luma.json. A platform clang cannot build is
/// [`BuildError::Platforms`] naming it, after every other is built.
pub fn build(folder: &Path, game: &Game, sdk: &Path, clang: &Path, work: &Path, platforms: &[String]) -> Result<Built, BuildError> {
    let wanted = targets(platforms)?;
    if !sdk.join("luma.h").is_file() {
        return Err(BuildError::NoSdk(sdk.to_path_buf()));
    }
    if wanted.iter().any(|t| t.link.iter().any(|f| f.contains("{sdk}"))) && !sdk.join("luma_ctors.ld").is_file() {
        return Err(BuildError::NoLinkerScript(sdk.join("luma_ctors.ld")));
    }
    // clang runs in `work`: every path it is given is absolute.
    let absolute = |p: &Path| std::path::absolute(p).map_err(|e| BuildError::Read(p.to_path_buf(), e));
    let (folder, sdk, work) = (absolute(folder)?, absolute(sdk)?, absolute(work)?);
    let (folder, sdk, work) = (folder.as_path(), sdk.as_path(), work.as_path());
    let found = clang_version(clang)?;
    if found != LLVM {
        return Err(BuildError::ClangVersion { path: clang.to_path_buf(), found });
    }
    let mut game = game.clone();
    game.mod_info_layout = mod_info_has_layout(sdk);
    let (_, emitted, notes) = generate(folder, &game, work)?;
    let c_bytes = emitted.files.iter().filter(|(p, _)| p.ends_with(".c") || p.ends_with(".h")).filter(|(p, _)| !p.contains("lumart")).map(|(_, t)| t.len() as u64).sum();
    let own = c_sources(folder)?;
    let generated: Vec<&str> = emitted.files.iter().map(|(rel, _)| rel.as_str()).filter(|rel| rel.ends_with(".c")).collect();
    let mut libraries = Vec::new();
    let mut failed = Vec::new();
    for t in wanted {
        match build_one(t, folder, sdk, clang, work, &generated, &own)? {
            Ok((library, bytes)) => libraries.push((t.platform, library, bytes)),
            Err((code, output)) => failed.push((t.platform, code, output)),
        }
    }
    let mut recorded = Vec::new();
    for (platform, _, _) in &libraries {
        if record_platform(folder, platform)? {
            recorded.push(*platform);
        }
    }
    if !failed.is_empty() {
        return Err(BuildError::Platforms { failed, built: libraries.iter().map(|(p, _, _)| *p).collect() });
    }
    Ok(Built { libraries, c_bytes, imports: emitted.imports, kept: emitted.kept, recorded, exports: emitted.exports, notes })
}

/// clang's exit code and what it said, when it stopped.
type Stopped = (i32, String);

fn run(cmd: &mut Command, clang: &Path) -> Result<Result<(), Stopped>, BuildError> {
    let out = cmd.output().map_err(|e| BuildError::NoClang(clang.to_path_buf(), e))?;
    if out.status.success() {
        return Ok(Ok(()));
    }
    let mut output = String::from_utf8_lossy(&out.stderr).into_owned();
    output.push_str(&String::from_utf8_lossy(&out.stdout));
    Ok(Err((out.status.code().unwrap_or(-1), output)))
}

/// One platform's library from the C in `work` (paths absolute): the mod's C++ objects first, then the link.
fn build_one(t: &Target, folder: &Path, sdk: &Path, clang: &Path, work: &Path, generated: &[&str], own: &CSources) -> Result<Result<(PathBuf, u64), Stopped>, BuildError> {
    let out_dir = folder.join("code").join(t.platform);
    std::fs::create_dir_all(&out_dir).map_err(|e| BuildError::Write(out_dir.clone(), e))?;
    let library = out_dir.join(t.library);
    // A library from an earlier build is never taken for this one's.
    if library.exists() {
        std::fs::remove_file(&library).map_err(|e| BuildError::Write(library.clone(), e))?;
    }
    let lum = work.join("lum");
    let source = folder.join("source");
    let sdk_text = sdk.display().to_string().replace('\\', "/");
    let link: Vec<String> = t.link.iter().map(|f| f.replace("{sdk}", &sdk_text)).collect();
    let msvc = t.triple.ends_with("-windows-msvc");
    // docs/modding/language/compiled-form.md, section 1.2: no path of this machine in the library; the mod's own sources are named from its folder.
    let maps = [format!("-ffile-prefix-map={}=.", work.display()), format!("-ffile-prefix-map={}=source", source.display())];
    // The mod's C++ (docs/modding/language/compiled-form.md, section 9), each file to an object of its own with the same flags but the language's.
    let mut objects = Vec::new();
    for (i, file) in own.cxx.iter().enumerate() {
        let obj = work.join("obj").join(t.platform).join(format!("{i}.o"));
        if let Some(parent) = obj.parent() {
            std::fs::create_dir_all(parent).map_err(|e| BuildError::Write(parent.to_path_buf(), e))?;
        }
        let mut cxx = Command::new(clang);
        cxx.current_dir(work);
        cxx.arg(format!("--target={}", t.triple));
        cxx.args(COMMON_FLAGS.iter().filter(|f| !f.starts_with("-std=") && **f != "-Werror=implicit-function-declaration")).args(CXX_FLAGS);
        cxx.args(t.code).args(&maps);
        if msvc {
            // No time stamp in the object (4.9: the same bytes for the same sources; the link has /Brepro).
            cxx.arg("-mno-incremental-linker-compatible");
        }
        cxx.arg("-I").arg(&lum).arg("-I").arg(sdk).arg("-I").arg(&source);
        cxx.arg("-c").arg(file).arg("-o").arg(&obj);
        if let Err(stopped) = run(&mut cxx, clang)? {
            return Ok(Err(stopped));
        }
        objects.push(obj);
    }
    let mut cmd = Command::new(clang);
    cmd.current_dir(work);
    cmd.arg(format!("--target={}", t.triple)).args(COMMON_FLAGS).args(t.code);
    cmd.args(&maps);
    cmd.arg("-I").arg(&lum).arg("-I").arg(sdk);
    if source.is_dir() {
        cmd.arg("-I").arg(&source);
    }
    cmd.args(&link);
    cmd.arg("-o").arg(&library);
    cmd.args(generated);
    // The mod's own C, then its C++ objects, in path order.
    cmd.args(&own.c);
    cmd.args(&objects);
    if let Err(stopped) = run(&mut cmd, clang)? {
        return Ok(Err(stopped));
    }
    let bytes = std::fs::metadata(&library).map_err(|e| BuildError::Read(library.clone(), e))?.len();
    Ok(Ok((library, bytes)))
}

/// The platform this LumaLink runs on, as luma-format 6 names it: what `lumalink test` builds, as the game it runs is
/// this machine's. None on a host no game build runs on.
pub const HOST: Option<&str> = if cfg!(all(windows, target_arch = "x86_64")) {
    Some("windows-x64")
} else if cfg!(all(windows, target_arch = "aarch64")) {
    Some("windows-arm64")
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("linux-x64")
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    Some("linux-arm64")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("macos-arm64")
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some("macos-x64")
} else {
    None
};

/// The platforms in the validator's order (docs/modding/format/luma.md, section 6): `code.platforms` is kept in it.
const PLATFORMS: &[&str] = &luma_validate::PLATFORMS;

/// The platform just built written into the mod's luma.json (`"code": {"platforms": [...]}`, docs/modding/format/luma.md, section 4), so check
/// and pack find the library without the modder editing the manifest; the file is written only when it changes, and
/// then the result is true (the build says so).
fn record_platform(folder: &Path, platform: &str) -> Result<bool, BuildError> {
    let path = folder.join("luma.json");
    let text = std::fs::read_to_string(&path).map_err(|e| BuildError::Read(path.clone(), e))?;
    match with_platform(&text, platform) {
        Some(new) if new != text => {
            std::fs::write(&path, new).map_err(|e| BuildError::Write(path.clone(), e))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// `text` (a luma.json) with `platform` in `code.platforms`, everything else as the modder wrote it (the edit is made
/// in the text, not by writing the parsed value again): `code` added after the last key when missing, `platforms`
/// first in `code` when missing, the list written again in luma-format's order when the platform is not in it. None
/// when it is there already, or when the file is no JSON object or `code`/`platforms` is of another kind (check says
/// what is wrong with those).
pub fn with_platform(text: &str, platform: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(text).ok()?.as_object()?;
    let open = text.find('{')?;
    let (members, close) = object_members(text, open)?;
    let list = format!("[\"{platform}\"]");
    let new = match members.iter().rev().find(|m| m.key == "code") {
        None => {
            let entry = format!("\"code\": {{ \"platforms\": {list} }}");
            match members.last() {
                None => format!("{}{entry}{}", &text[..open + 1], &text[close..]),
                Some(last) => {
                    // The first key's own line and indent, when the object is written one key a line.
                    let before = &text[open + 1..members[0].at];
                    let (nl, indent) = match before.rfind('\n') {
                        Some(n) => (if text.contains("\r\n") { "\r\n" } else { "\n" }, &before[n + 1..]),
                        None => ("", " "),
                    };
                    format!("{},{nl}{indent}{entry}{}", &text[..last.end], &text[last.end..])
                }
            }
        }
        Some(code) => {
            if text.as_bytes()[code.start] != b'{' {
                return None;
            }
            let (inner, _) = object_members(text, code.start)?;
            match inner.iter().rev().find(|m| m.key == "platforms") {
                None => {
                    let entry = format!("\"platforms\": {list}");
                    match inner.first() {
                        None => format!("{} {entry} {}", &text[..code.start + 1], &text[code.end - 1..]),
                        Some(f) => format!("{}{entry}, {}", &text[..f.at], &text[f.at..]),
                    }
                }
                Some(p) => {
                    let had: Vec<String> = serde_json::from_str(&text[p.start..p.end]).ok()?;
                    if had.iter().any(|h| h == platform) {
                        return None;
                    }
                    let mut all = had;
                    all.push(platform.to_string());
                    // Known platforms in luma-format's order, any other after them as written (check names those).
                    let rank = |s: &String| PLATFORMS.iter().position(|p| p == s).unwrap_or(PLATFORMS.len());
                    all.sort_by_key(rank);
                    let items: Vec<String> = all.iter().map(|s| serde_json::to_string(s).unwrap_or_default()).collect();
                    format!("{}[{}]{}", &text[..p.start], items.join(", "), &text[p.end..])
                }
            }
        }
    };
    // Only a text that reads back as JSON with the platform in it is written.
    let back: serde_json::Value = serde_json::from_str(&new).ok()?;
    back.get("code")?.get("platforms")?.as_array()?.iter().any(|v| v.as_str() == Some(platform)).then_some(new)
}

/// One key of a JSON object in a text: where its key starts, where its value starts and ends.
struct Member {
    key: String,
    at: usize,
    start: usize,
    end: usize,
}

/// The members of the object whose `{` is at `open`, and where its `}` is.
fn object_members(text: &str, open: usize) -> Option<(Vec<Member>, usize)> {
    let b = text.as_bytes();
    let mut i = skip_ws(b, open + 1);
    let mut out = Vec::new();
    if b.get(i) == Some(&b'}') {
        return Some((out, i));
    }
    loop {
        let at = i;
        let key_end = skip_value(b, i)?;
        let key: String = serde_json::from_str(&text[at..key_end]).ok()?;
        i = skip_ws(b, key_end);
        if b.get(i) != Some(&b':') {
            return None;
        }
        let start = skip_ws(b, i + 1);
        let end = skip_value(b, start)?;
        out.push(Member { key, at, start, end });
        i = skip_ws(b, end);
        match b.get(i) {
            Some(b',') => i = skip_ws(b, i + 1),
            Some(b'}') => return Some((out, i)),
            _ => return None,
        }
    }
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

/// The end of the JSON value starting at `i` (the text is known to be JSON).
fn skip_value(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => {
            let mut j = i + 1;
            while j < b.len() {
                match b[j] {
                    b'\\' => j += 2,
                    b'"' => return Some(j + 1),
                    _ => j += 1,
                }
            }
            None
        }
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = skip_value(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n') {
                j += 1;
            }
            Some(j)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{content_names, generate, mod_info_has_layout, option_spec, targets, with_platform, BuildError, TARGETS};
    use lumascript::check::{OptionKind, OptionSpec};

    /// luma.json's options as `option` declarations are checked: a number is whole when every number given is;
    /// an entry with no key or another kind is check's to say, left out here.
    #[test]
    fn options_are_read_with_their_kinds() {
        let json: serde_json::Value = serde_json::from_str(
            r#"[{"key": "port", "kind": "number", "default": 7000, "min": 1, "max": 65535, "step": 1},
                {"key": "speed", "kind": "number", "default": 1, "min": 0.5, "max": 2},
                {"key": "on", "kind": "toggle", "default": true}, {"key": "who", "kind": "choice", "default": "a"},
                {"key": "say", "kind": "text", "default": ""}, {"key": "jump", "kind": "key", "default": "A"},
                {"kind": "toggle"}, {"key": "x", "kind": "slider"}]"#,
        )
        .unwrap();
        let read: Vec<OptionSpec> = json.as_array().unwrap().iter().filter_map(option_spec).collect();
        let kinds: Vec<(&str, OptionKind)> = read.iter().map(|o| (o.key.as_str(), o.kind)).collect();
        assert_eq!(kinds, vec![
            ("port", OptionKind::Number { whole: true }),
            ("speed", OptionKind::Number { whole: false }),
            ("on", OptionKind::Toggle),
            ("who", OptionKind::Choice),
            ("say", OptionKind::Text),
            ("jump", OptionKind::Key),
        ]);
    }

    /// The SDK's LumaModInfo has the runtime layout field only when a field (not a comment) names it.
    #[test]
    fn the_sdk_record_field_is_read_from_luma_h() {
        let dir = tempfile::tempdir().unwrap();
        let sdk = |body: &str| {
            std::fs::write(dir.path().join("luma.h"), format!("typedef struct LumaModInfo {{\n    uint32_t size;\n{body}}} LumaModInfo;\n")).unwrap();
            mod_info_has_layout(dir.path())
        };
        assert!(sdk("    uint32_t runtime_layout; /* the runtime's layout */\n"));
        assert!(!sdk("    /* no runtime_layout yet */\n    // runtime_layout later\n"));
        assert!(!sdk(""));
        assert!(!mod_info_has_layout(&dir.path().join("none")));
    }

    /// A needed mod whose interface file carries another runtime layout is S0160, naming it, before anything is built.
    #[test]
    fn a_need_built_with_another_runtime_layout_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let write = |p: &str, text: &str| {
            let p = dir.path().join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("moon/luma.json", r#"{"format": 1, "id": "moon", "version": "1.0.0", "name": "Moon", "needs": [{"id": "lumakit", "version": "1.0.0"}]}"#);
        write("moon/source/main.lum", "on start { log \"a\" }\n");
        write("lumakit/code/lumakit.lumi", "// lumakit 1.0.0: what it gives other mods. Declarations only;\n\nmodule hud\npub fn ping()  // lumi1:0011223344556677\n");
        let game = lumascript::game::Game::default();
        match generate(&dir.path().join("moon"), &game, &dir.path().join("work")) {
            Err(BuildError::Script(errors)) => {
                let (file, d) = &errors[0];
                assert_eq!(file, "lumakit.lumi");
                assert_eq!(d.code, lumascript::diag::code::NEED_OTHER_LAYOUT);
                let arg = |n: &str| d.args.iter().find(|(k, _)| *k == n).map(|(_, v)| v.clone());
                assert_eq!(arg("need"), Some(lumascript::diag::Arg::Text("lumakit".into())));
                assert_eq!(arg("found"), Some(lumascript::diag::Arg::Text("1".into())));
                assert_eq!(arg("layout"), Some(lumascript::diag::Arg::Text(lumascript::lumi::RUNTIME_LAYOUT.to_string())));
                let line = crate::script::line(file, d);
                assert!(line.contains("rebuild lumakit with a current LumaLink"), "{line}");
            }
            other => panic!("{:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn content_names_come_from_the_mods_assets() {
        let dir = tempfile::tempdir().unwrap();
        let file = |p: &str| {
            let p = dir.path().join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        };
        for p in [
            "gold/layers/base/assets/Throw.sound.json",
            "gold/layers/base/assets/Gold/Ball.glb",
            "gold/layers/base/assets/Gold/BallMiddle.glb",
            "gold/layers/base/assets/Gold/BallLow.glb",
            "gold/layers/base/assets/Gold/Ball.lod1.glb",
            "gold/layers/base/assets/Gold/Ball.kcl.glb",
            "gold/layers/base/assets/Low.glb",
            "gold/layers/night/assets/Lyrics.font.json",
            "gold/layers/night/assets/Lyrics.ttf",
            "gold/layers/night/assets/movies/Trailer.movie.json",
            "gold/layers/night/assets/lyrics.json",
            "gold/layers/base/files/ObjectData/Kuribo.arc/Kuribo.glb",
            "lib/layers/base/assets/Pop.sound.json",
        ] {
            file(p);
        }
        let c = content_names(&dir.path().join("gold"), &["lib".to_string(), "absent".to_string()]);
        let names = |w: &str| c.names[w].iter().cloned().collect::<Vec<_>>();
        assert_eq!(names("sound"), ["Throw", "lib:Pop"]);
        assert_eq!(names("model"), ["Gold/Ball", "Low"]);
        assert_eq!(names("font"), ["Lyrics"]);
        assert_eq!(names("movie"), ["movies/Trailer"]);
        assert!(c.names["data"].contains("lyrics.json") && c.names["data"].contains("lib:Pop.sound.json"));
        assert!(!c.names["data"].iter().any(|n| n.contains("Kuribo")));
        assert!(!c.names.contains_key("effect") && c.texts.is_none());
    }

    #[test]
    fn the_nine_platforms_are_luma_formats_in_its_order() {
        let names: Vec<&str> = TARGETS.iter().map(|t| t.platform).collect();
        assert_eq!(names, luma_validate::PLATFORMS);
        for t in &TARGETS {
            let ending = t.library.rsplit('.').next().unwrap();
            let (form, link_only) = match ending {
                "dll" => ("windows-msvc", "-Wl,/NOENTRY"),
                "so" => ("linux", "-Wl,-T,{sdk}/luma_ctors.ld"),
                _ => ("apple", "-Wl,-adhoc_codesign"),
            };
            assert!(t.triple.contains(form) && t.link.contains(&link_only), "{t:?}");
            assert_eq!(t.platform.starts_with("android"), t.link.contains(&"-Wl,-z,max-page-size=16384"), "{t:?}");
        }
        assert!(TARGETS[6].code.contains(&"-mno-outline-atomics") && !TARGETS[7].code.contains(&"-mno-outline-atomics"));
        // Asked in any order and twice: luma-format's order, once; an unknown one by name.
        let picked = targets(&["ios-arm64".into(), "linux-x64".into(), "ios-arm64".into()]).unwrap();
        assert_eq!(picked.iter().map(|t| t.platform).collect::<Vec<_>>(), ["linux-x64", "ios-arm64"]);
        assert!(matches!(targets(&["ps2".into()]), Err(BuildError::UnknownPlatform(p)) if p == "ps2"));
        assert_eq!(targets(&[]).unwrap().len(), 9);
    }

    #[test]
    fn code_is_added_after_the_last_key_in_the_files_own_layout() {
        let text = "{\n  \"id\": \"m\",\n  \"version\": \"1.0.0\"\n}\n";
        assert_eq!(with_platform(text, "windows-x64").unwrap(), "{\n  \"id\": \"m\",\n  \"version\": \"1.0.0\",\n  \"code\": { \"platforms\": [\"windows-x64\"] }\n}\n");
        let crlf = "{\r\n    \"id\": \"m\"\r\n}";
        assert_eq!(with_platform(crlf, "windows-x64").unwrap(), "{\r\n    \"id\": \"m\",\r\n    \"code\": { \"platforms\": [\"windows-x64\"] }\r\n}");
        assert_eq!(with_platform("{\"id\": \"m\"}", "windows-x64").unwrap(), "{\"id\": \"m\", \"code\": { \"platforms\": [\"windows-x64\"] }}");
        assert_eq!(with_platform("{}", "windows-x64").unwrap(), "{\"code\": { \"platforms\": [\"windows-x64\"] }}");
        // A brace and a quote inside a string are text.
        assert_eq!(with_platform("{\"id\": \"a\\\"}\"}", "windows-x64").unwrap(), "{\"id\": \"a\\\"}\", \"code\": { \"platforms\": [\"windows-x64\"] }}");
    }

    #[test]
    fn platforms_are_added_in_order_and_once() {
        let text = "{\"id\": \"m\", \"code\": {\"platforms\": [\"android-arm64\"], \"libraries\": {}}, \"x\": [1, {\"a\": \"}\"}]}";
        assert_eq!(with_platform(text, "windows-x64").unwrap(), "{\"id\": \"m\", \"code\": {\"platforms\": [\"windows-x64\", \"android-arm64\"], \"libraries\": {}}, \"x\": [1, {\"a\": \"}\"}]}");
        assert_eq!(with_platform("{\"code\": {\"platforms\": [\"windows-x64\"]}}", "windows-x64"), None);
        assert_eq!(with_platform("{\"code\": {\"libraries\": {}}}", "windows-x64").unwrap(), "{\"code\": {\"platforms\": [\"windows-x64\"], \"libraries\": {}}}");
        assert_eq!(with_platform("{\"code\": {}}", "windows-x64").unwrap(), "{\"code\": { \"platforms\": [\"windows-x64\"] }}");
        assert_eq!(with_platform("{\"code\": {\"platforms\": []}}", "windows-x64").unwrap(), "{\"code\": {\"platforms\": [\"windows-x64\"]}}");
    }

    #[test]
    fn what_check_refuses_is_left_to_check() {
        assert_eq!(with_platform("[1]", "windows-x64"), None);
        assert_eq!(with_platform("{\"code\": 3}", "windows-x64"), None);
        assert_eq!(with_platform("{\"code\": {\"platforms\": \"windows-x64\"}}", "windows-x64"), None);
        assert_eq!(with_platform("{\"id\": 1", "windows-x64"), None);
    }
}
