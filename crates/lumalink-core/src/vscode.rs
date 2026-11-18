//! LumaScript in Visual Studio Code, set up from the window: Setup installs the extension packed
//! (`editors/lumascript.vsix` beside LumaLink's programs, made by installer/vsix.py from editors/vscode), and
//! [`install`] hands it to VS Code's own command line (`code --install-extension`), the way VS Code installs any
//! extension file. The extension then starts `lumalink lsp` (editors/vscode/extension.js finds the installed LumaLink
//! when it is not on the PATH).

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The packed extension beside this LumaLink, when Setup installed it.
pub fn extension() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let file = exe.parent()?.join("editors").join("lumascript.vsix");
    file.is_file().then_some(file)
}

/// VS Code's command line: on the PATH, else where its installers put it.
pub fn find_code() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["code.cmd", "code-insiders.cmd"] } else { &["code", "code-insiders"] };
    if let Some(path) = std::env::var_os("PATH") {
        for folder in std::env::split_paths(&path) {
            for name in names {
                let file = folder.join(name);
                if file.is_file() {
                    return Some(file);
                }
            }
        }
    }
    known_places().into_iter().find(|f| f.is_file())
}

fn known_places() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        // The user installer's folder, then the system installer's.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(local).join("Programs").join("Microsoft VS Code").join("bin").join("code.cmd"));
        }
        if let Some(programs) = std::env::var_os("ProgramFiles") {
            out.push(PathBuf::from(programs).join("Microsoft VS Code").join("bin").join("code.cmd"));
        }
    } else if cfg!(target_os = "macos") {
        let app = Path::new("Visual Studio Code.app").join("Contents/Resources/app/bin/code");
        out.push(Path::new("/Applications").join(&app));
        if let Some(home) = std::env::var_os("HOME") {
            out.push(PathBuf::from(home).join("Applications").join(&app));
        }
    } else {
        for f in ["/usr/bin/code", "/usr/local/bin/code", "/snap/bin/code", "/var/lib/flatpak/exports/bin/com.visualstudio.code"] {
            out.push(PathBuf::from(f));
        }
    }
    out
}

/// The program and script VS Code's code.cmd runs (`"%~dp0..\Code.exe" "%~dp0..\<commit>\resources\app\out\cli.js" %*`),
/// with `%~dp0` as `folder`, code.cmd's own folder. None when the file is not that.
fn code_cmd_parts(text: &str, folder: &Path) -> Option<(PathBuf, PathBuf)> {
    let line = text.lines().find(|l| l.contains("cli.js"))?;
    let quoted: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    let [program, cli, ..] = quoted.as_slice() else { return None };
    let expand = |part: &str| -> Option<PathBuf> {
        let rest = part.strip_prefix("%~dp0")?;
        Some(folder.join(rest.replace('\\', std::path::MAIN_SEPARATOR_STR)))
    };
    let (program, cli) = (expand(program)?, expand(cli)?);
    (program.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) && cli.ends_with("cli.js")).then_some((program, cli))
}

/// What [`install`] could not do.
#[derive(Debug, PartialEq, Eq)]
pub enum Missing {
    /// This LumaLink has no packed extension (a build, not an installed copy).
    Extension,
    /// VS Code is not installed, or not where it is looked for.
    Code,
}

/// Installs (or updates) the LumaScript extension in VS Code. `Ok(Err(..))`: what is missing.
pub fn install() -> Result<std::result::Result<(), Missing>> {
    let Some(vsix) = extension() else { return Ok(Err(Missing::Extension)) };
    let Some(code) = find_code() else { return Ok(Err(Missing::Code)) };
    let mut command = if cfg!(windows) {
        // code.cmd only sets ELECTRON_RUN_AS_NODE and runs Code.exe with VS Code's cli.js: the same, without cmd.exe.
        let text = std::fs::read_to_string(&code).with_context(|| code.display().to_string())?;
        let Some((program, cli)) = code_cmd_parts(&text, code.parent().unwrap_or(Path::new("."))) else { return Ok(Err(Missing::Code)) };
        let mut c = Command::new(program);
        c.arg(cli).env("ELECTRON_RUN_AS_NODE", "1").env_remove("VSCODE_DEV");
        c
    } else {
        Command::new(&code)
    };
    command.arg("--install-extension").arg(&vsix).arg("--force");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = command.output().with_context(|| format!("{} could not start", code.display()))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let said = said.trim();
        bail!("{}", if said.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { said.to_string() });
    }
    Ok(Ok(()))
}

#[cfg(test)]
mod tests {
    /// The whole path on a machine with VS Code: LUMASCRIPT_VSIX_TEST names a .vsix (installer/vsix.py's), put beside
    /// the test program as Setup puts it; VSCODE_EXTENSIONS points VS Code at a throwaway extensions folder.
    #[test]
    #[ignore = "needs VS Code, LUMASCRIPT_VSIX_TEST and VSCODE_EXTENSIONS"]
    fn installs_into_vs_code() {
        let vsix = std::env::var_os("LUMASCRIPT_VSIX_TEST").expect("LUMASCRIPT_VSIX_TEST");
        assert!(std::env::var_os("VSCODE_EXTENSIONS").is_some(), "VSCODE_EXTENSIONS");
        let beside = std::env::current_exe().unwrap().parent().unwrap().join("editors");
        std::fs::create_dir_all(&beside).unwrap();
        std::fs::copy(vsix, beside.join("lumascript.vsix")).unwrap();
        let result = super::install();
        let _ = std::fs::remove_dir_all(&beside);
        assert_eq!(result.unwrap(), Ok(()));
    }

    #[test]
    fn code_cmd_is_read_without_running_it() {
        let text = concat!(
            "@echo off\r\nsetlocal\r\nset VSCODE_DEV=\r\nset ELECTRON_RUN_AS_NODE=1\r\n",
            r#""%~dp0..\Code.exe" "%~dp0..\69047191fc\resources\app\out\cli.js" %*"#,
            "\r\nendlocal\r\n"
        );
        let folder = std::path::Path::new("vscode").join("bin");
        let (program, cli) = super::code_cmd_parts(text, &folder).unwrap();
        assert!(program.ends_with("Code.exe") && program.starts_with(&folder));
        assert!(cli.ends_with("cli.js") && cli.to_string_lossy().contains("69047191fc"));
        assert!(super::code_cmd_parts("@echo off\r\nstart something.exe\r\n", &folder).is_none());
        // Only paths beside code.cmd are run.
        assert!(super::code_cmd_parts(r#""C:\evil.exe" "%~dp0cli.js""#, &folder).is_none());
    }

    /// The extension's words are the text table's rows LumaLink.Editor.Vscode.*, in every language VS Code shows
    /// (editors/vscode/make.py writes them): package.json names each by its %key%; package.nls.json,
    /// package.nls.<locale>.json and l10n/bundle.l10n.<locale>.json hold them as the table does; messages.js gives
    /// extension.js the English, the bundles' keys; and extension.js shows no words of its own.
    #[test]
    fn the_extension_says_its_words_in_every_language() {
        use serde_json::{Map, Value};
        const PACKAGE: [(&str, &str); 5] = [
            ("description", "LumaLink.Editor.Vscode.Description"),
            ("setting.lumalink", "LumaLink.Editor.Vscode.Setting.LumaLink"),
            ("setting.names", "LumaLink.Editor.Vscode.Setting.Names"),
            ("setting.sdk", "LumaLink.Editor.Vscode.Setting.Sdk"),
            ("command.restart", "LumaLink.Editor.Vscode.Command.Restart"),
        ];
        const MESSAGES: [(&str, &str); 6] = [
            ("NO_START_LOG", "LumaLink.Editor.Vscode.NoStartLog"),
            ("NO_START", "LumaLink.Editor.Vscode.NoStart"),
            ("SERVER_LEFT", "LumaLink.Editor.Vscode.ServerLeft"),
            ("NOT_JSON", "LumaLink.Editor.Vscode.NotJson"),
            ("NO_INITIALIZE", "LumaLink.Editor.Vscode.NoInitialize"),
            ("NO_RENAME", "LumaLink.Editor.Vscode.NoRename"),
        ];
        const LOCALES: [(&str, &str); 10] = [
            ("fr", "EuFrench"),
            ("es", "EuSpanish"),
            ("de", "EuGerman"),
            ("it", "EuItalian"),
            ("nl", "EuDutch"),
            ("pt-br", "UsPortuguese"),
            ("ja", "JpJapanese"),
            ("ko", "KrKorean"),
            ("zh-cn", "CnSimpChinese"),
            ("zh-tw", "TwTradChinese"),
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("editors").join("vscode");
        let text = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap_or_else(|e| panic!("editors/vscode/{p}: {e} (run make.py)"));
        let read = |p: &str| -> Value { serde_json::from_str(&text(p)).unwrap_or_else(|e| panic!("{p}: {e}")) };
        let language = |name: &str| crate::text::LANGUAGES.iter().position(|l| *l == name).unwrap();
        let row = |l: usize, id: &str| crate::text::template(l, id).filter(|t| !t.is_empty()).unwrap_or_else(|| panic!("{id} in {}", crate::text::LANGUAGES[l]));
        // `{name}`s in their order, and a text with each as VS Code's l10n numbers its arguments.
        let names = |t: &str| -> Vec<String> {
            let mut out = Vec::new();
            for part in t.split('{').skip(1) {
                if let Some((n, _)) = part.split_once('}') {
                    if !n.is_empty() && n.bytes().all(|b| b.is_ascii_lowercase()) && !out.iter().any(|o| o == n) {
                        out.push(n.to_string());
                    }
                }
            }
            out
        };
        let numbered = |t: &str, names: &[String]| names.iter().enumerate().fold(t.to_string(), |t, (i, n)| t.replace(&format!("{{{n}}}"), &format!("{{{i}}}")));

        let package = read("package.json");
        assert_eq!(package["description"], "%description%");
        assert_eq!(package["l10n"], "./l10n");
        let settings = &package["contributes"]["configuration"]["properties"];
        for (setting, key) in [("lumascript.lumalink", "setting.lumalink"), ("lumascript.names", "setting.names"), ("lumascript.sdk", "setting.sdk")] {
            assert_eq!(settings[setting]["markdownDescription"], format!("%{key}%"), "{setting}");
        }
        assert_eq!(package["contributes"]["commands"][0]["title"], "%command.restart%");
        let snippets = read("snippets/lumascript.json");
        assert!(snippets.as_object().unwrap().values().all(|s| s.get("description").is_none()), "a snippet's words are English only");

        let nls = |l: usize| Value::Object(PACKAGE.iter().map(|(k, id)| (k.to_string(), Value::from(row(l, id)))).collect());
        let en = language("UsEnglish");
        assert_eq!(read("package.nls.json"), nls(en));
        for (locale, name) in LOCALES {
            let l = language(name);
            assert_eq!(read(&format!("package.nls.{locale}.json")), nls(l), "{locale}");
            let bundle: Map<String, Value> = MESSAGES
                .iter()
                .map(|(_, id)| {
                    let n = names(row(en, id));
                    assert_eq!(names(row(l, id)).len(), n.len(), "{id} in {name}: its placeholders");
                    (numbered(row(en, id), &n), Value::from(numbered(row(l, id), &n)))
                })
                .collect();
            assert_eq!(read(&format!("l10n/bundle.l10n.{locale}.json")), Value::Object(bundle), "{locale}");
        }
        let messages = text("messages.js");
        let extension = text("extension.js");
        for (name, id) in MESSAGES {
            let english = numbered(row(en, id), &names(row(en, id)));
            assert!(messages.contains(&format!("    {name}: {},", serde_json::to_string(&english).unwrap())), "messages.js {name}");
            assert!(extension.contains(&format!("vscode.l10n.t(M.{name}")), "extension.js shows {name}");
        }
        // What extension.js shows or logs is one of those, or the server's own words.
        for call in ["showErrorMessage(", "appendLine(", "new Error("] {
            for (at, _) in extension.match_indices(call) {
                let after = &extension[at + call.len()..];
                assert!(!after.starts_with(['\'', '"', '`']), "extension.js: words of its own after {call}: {}", &after[..after.len().min(60)]);
            }
        }
    }

    #[test]
    fn known_places_are_absolute() {
        for f in super::known_places() {
            assert!(f.is_absolute(), "{}", f.display());
        }
    }
}
