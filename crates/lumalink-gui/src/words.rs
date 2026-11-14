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
    let wanted = std::env::var("LUMALINK_LANGUAGE").ok().or_else(sys_locale::get_locale).unwrap_or_default();
    pick(&wanted)
}

/// The game's language for a language's name in the game or a system language code (`fr-CA`, `zh-Hant-TW`, `pt_BR`).
fn pick(wanted: &str) -> &'static str {
    if let Some(l) = luma_validate::LANGUAGES.iter().find(|l| l.eq_ignore_ascii_case(wanted)) {
        return l;
    }
    let code = wanted.to_ascii_lowercase().replace('_', "-");
    let mut parts = code.split('-');
    let base = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();
    let region = |r: &[&str]| rest.iter().any(|p| r.contains(p));
    match base {
        // The Americas' English is the US one; every other English the European one.
        "en" => if rest.is_empty() || region(&["us", "ca", "mx", "ph"]) { "UsEnglish" } else { "EuEnglish" },
        "fr" => if region(&["ca", "us"]) { "UsFrench" } else { "EuFrench" },
        // Spain's Spanish is the European one; every other the Americas' (as the game's own regions split them).
        "es" => if rest.is_empty() || region(&["es"]) { "EuSpanish" } else { "UsSpanish" },
        "de" => "EuGerman",
        "it" => "EuItalian",
        "nl" => "EuDutch",
        "pt" => "UsPortuguese",
        "ja" => "JpJapanese",
        "ko" => "KrKorean",
        "zh" => if region(&["hant", "tw", "hk", "mo"]) { "TwTradChinese" } else { "CnSimpChinese" },
        _ => "UsEnglish",
    }
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
    // Japanese, Korean, Simplified Chinese, Traditional Chinese: regular, bold, face.
    const WINDOWS: [(&str, &str, u32); 4] = [
        ("C:/Windows/Fonts/YuGothM.ttc", "C:/Windows/Fonts/YuGothB.ttc", 0),
        ("C:/Windows/Fonts/malgun.ttf", "C:/Windows/Fonts/malgunbd.ttf", 0),
        ("C:/Windows/Fonts/msyh.ttc", "C:/Windows/Fonts/msyhbd.ttc", 0),
        ("C:/Windows/Fonts/msjh.ttc", "C:/Windows/Fonts/msjhbd.ttc", 0),
    ];
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
        if cfg!(windows) || cfg!(target_os = "macos") {
            let (regular, heavy, face) = if cfg!(windows) { WINDOWS[which] } else { MAC[which] };
            out.push((std::path::PathBuf::from(if bold { heavy } else { regular }), face));
        } else {
            // One collection holds all four: Japanese, Korean, Simplified, Traditional, in that order.
            for folder in NOTO {
                let file = if bold { "NotoSansCJK-Bold.ttc" } else { "NotoSansCJK-Regular.ttc" };
                out.push((std::path::Path::new(folder).join(file), which as u32));
            }
        }
    }
    out.retain(|(path, _)| path.is_file());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_system_language_picks_the_games() {
        assert_eq!(pick("en-US"), "UsEnglish");
        assert_eq!(pick("en-GB"), "EuEnglish");
        assert_eq!(pick("fr-ca"), "UsFrench");
        assert_eq!(pick("fr-FR"), "EuFrench");
        assert_eq!(pick("es-MX"), "UsSpanish");
        assert_eq!(pick("es-ES"), "EuSpanish");
        assert_eq!(pick("pt_BR"), "UsPortuguese");
        assert_eq!(pick("ja"), "JpJapanese");
        assert_eq!(pick("zh-cn"), "CnSimpChinese");
        assert_eq!(pick("zh-Hans-CN"), "CnSimpChinese");
        assert_eq!(pick("zh-TW"), "TwTradChinese");
        assert_eq!(pick("zh-hant"), "TwTradChinese");
        assert_eq!(pick("sv-se"), "UsEnglish");
        assert_eq!(pick(""), "UsEnglish");
        assert_eq!(pick("KrKorean"), "KrKorean");
    }

    #[test]
    fn every_language_has_its_own_name() {
        for l in luma_validate::LANGUAGES {
            assert!(NAMES.iter().any(|(n, _)| *n == l), "{l}");
        }
    }
}
