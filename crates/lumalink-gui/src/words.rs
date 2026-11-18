//! The window's language and the fonts for it. Its words are LumaLink's rows (`lumalink_core::text`) in the game's 14
//! languages; the language is `LUMALINK_LANGUAGE` (a language's name in the game, or a system language code), else
//! the system's, else UsEnglish, and the menu at the top changes it.

/// Each of the game's 14 languages by its own name, as the menu lists them.
const NAMES: [(&str, &str); 14] = [
    ("UsEnglish", "English (US)"),
    ("EuEnglish", "English (UK)"),
    ("UsFrench", "Français (Canada)"),
    ("EuFrench", "Français (France)"),
    ("UsSpanish", "Español (Latinoamérica)"),
    ("EuSpanish", "Español (España)"),
    ("EuGerman", "Deutsch"),
    ("EuItalian", "Italiano"),
    ("EuDutch", "Nederlands"),
    ("UsPortuguese", "Português"),
    ("JpJapanese", "日本語"),
    ("KrKorean", "한국어"),
    ("CnSimpChinese", "简体中文"),
    ("TwTradChinese", "繁體中文"),
];

/// A language's own name.
pub fn name(language: &str) -> &'static str {
    NAMES.iter().find(|(l, _)| *l == language).map_or("English (US)", |(_, n)| n)
}

/// The language to start in.
pub fn language() -> &'static str {
    let wanted = std::env::var("LUMALINK_LANGUAGE").ok().filter(|l| !l.is_empty()).or_else(sys_locale::get_locale).unwrap_or_default();
    lumalink_core::text::language_for(&wanted)
}

/// Which of the four the rounded font does not hold all of the language is in: Japanese 0, Korean 1, Simplified 2,
/// Traditional 3; None for the others.
fn east_asian() -> Option<usize> {
    match lumalink_core::text::language() {
        "JpJapanese" => Some(0),
        "KrKorean" => Some(1),
        "CnSimpChinese" => Some(2),
        "TwTradChinese" => Some(3),
        _ => None,
    }
}

/// Whether the language's text needs the system's font throughout: the rounded font holds Japanese and only some of
/// the others' characters.
pub fn system_font_first() -> bool {
    east_asian().is_some_and(|which| which > 0)
}

/// The system's fonts for languages the rounded font does not hold, this language's own first, each with its face in a
/// collection. A line reads from one font: two fonts' characters do not share a baseline.
pub fn system_fonts(bold: bool) -> Vec<(std::path::PathBuf, u32)> {
    // Windows' own folder, wherever it was put (N-4068: it was taken to be C:\Windows, so with Windows on another
    // drive Korean and Chinese had no font).
    let windows = std::env::var_os("SystemRoot").map_or_else(|| std::path::PathBuf::from("C:/Windows"), std::path::PathBuf::from);
    let mut out = candidates(&windows, bold);
    out.retain(|(path, _)| path.is_file());
    out
}

/// Where [`system_fonts`] looks, in order, with Windows in `windows`.
fn candidates(windows: &std::path::Path, bold: bool) -> Vec<(std::path::PathBuf, u32)> {
    // Japanese, Korean, Simplified Chinese, Traditional Chinese: regular, bold, face.
    const WINDOWS: [(&str, &str, u32); 4] = [("YuGothM.ttc", "YuGothB.ttc", 0), ("malgun.ttf", "malgunbd.ttf", 0), ("msyh.ttc", "msyhbd.ttc", 0), ("msjh.ttc", "msjhbd.ttc", 0)];
    const MAC: [(&str, &str, u32); 4] = [
        ("/System/Library/Fonts/ヒラギノ角ゴシック W4.ttc", "/System/Library/Fonts/ヒラギノ角ゴシック W7.ttc", 0),
        ("/System/Library/Fonts/AppleSDGothicNeo.ttc", "/System/Library/Fonts/AppleSDGothicNeo.ttc", 0),
        ("/System/Library/Fonts/STHeiti Medium.ttc", "/System/Library/Fonts/STHeiti Medium.ttc", 1),
        ("/System/Library/Fonts/STHeiti Medium.ttc", "/System/Library/Fonts/STHeiti Medium.ttc", 0),
    ];
    const NOTO: [&str; 3] = ["/usr/share/fonts/opentype/noto", "/usr/share/fonts/noto-cjk", "/usr/share/fonts/google-noto-cjk"];
    // Korean first for every language not East Asian: the language menu shows all four scripts.
    let own = east_asian().unwrap_or(1);
    let order = [own, (own + 1) % 4, (own + 2) % 4, (own + 3) % 4];
    let mut out: Vec<(std::path::PathBuf, u32)> = Vec::new();
    for which in order {
        if cfg!(windows) {
            let (regular, heavy, face) = WINDOWS[which];
            out.push((windows.join("Fonts").join(if bold { heavy } else { regular }), face));
        } else if cfg!(target_os = "macos") {
            let (regular, heavy, face) = MAC[which];
            out.push((std::path::PathBuf::from(if bold { heavy } else { regular }), face));
        } else {
            // One collection holds all four: Japanese, Korean, Simplified, Traditional, in that order.
            for folder in NOTO {
                let file = if bold { "NotoSansCJK-Bold.ttc" } else { "NotoSansCJK-Regular.ttc" };
                out.push((std::path::Path::new(folder).join(file), which as u32));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N-4068: on Windows the fonts are looked for in the Windows folder the system names, on whatever drive.
    #[cfg(windows)]
    #[test]
    fn windows_fonts_are_looked_for_where_windows_is() {
        let windows = std::path::Path::new("E:/Win");
        for bold in [false, true] {
            let found = candidates(windows, bold);
            assert_eq!(found.len(), 4);
            assert!(found.iter().all(|(path, _)| path.starts_with("E:/Win/Fonts")), "{found:?}");
        }
        assert!(candidates(windows, true).iter().any(|(path, _)| path.ends_with("malgunbd.ttf")));
    }

    #[test]
    fn every_language_has_its_own_name() {
        for l in luma_validate::LANGUAGES {
            assert!(NAMES.iter().any(|(n, _)| *n == l), "{l}");
        }
    }
}
