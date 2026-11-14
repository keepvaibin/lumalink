//! A converted mod's code patches that are the game's numbers: `source/main.lum`, which sets each with
//! LumaCore's set_number while its layer's `when` holds, from the mod's start. The kernel puts every number back when
//! the mod goes. The mod's code is built like any other (`lumalink build`).

use crate::say;
use serde_json::Value;
use std::fmt::Write;

/// Where in the package the program goes.
pub const FILE: &str = "source/main.lum";

/// One layer's numbers: its name, its `when` as luma.json has it (option keys and choice ids of the form), and each
/// number's name (luma_numbers.json) and value.
pub struct LayerNumbers {
    pub layer: String,
    pub when: Option<Value>,
    pub numbers: Vec<(&'static str, f64)>,
}

/// What `text` made: the program (none when no layer has a number it can set), how many numbers it sets, and the
/// layers whose `when` the program cannot test (a language or a platform), whose numbers are left out.
#[derive(Default)]
pub struct Made {
    pub program: Option<String>,
    pub count: usize,
    pub untested: Vec<String>,
}

/// The program for the mod `name`.
pub fn text(name: &str, layers: &[LayerNumbers]) -> Made {
    let mut made = Made::default();
    let mut body = String::new();
    let (mut options, mut regions) = (false, false);
    for l in layers.iter().filter(|l| !l.numbers.is_empty()) {
        let Some(test) = condition(l.when.as_ref(), &mut options, &mut regions) else {
            made.untested.push(l.layer.clone());
            continue;
        };
        let indent = if test.is_some() { "        " } else { "    " };
        let _ = writeln!(body, "    // {}", say!("LumaLink.Numbers.Layer", layer = l.layer));
        if let Some(test) = &test {
            let _ = writeln!(body, "    if {test} {{");
        }
        for (number, value) in &l.numbers {
            let _ = writeln!(body, "{indent}set_number({}, {})", quoted(number), literal(*value));
            made.count += 1;
        }
        if test.is_some() {
            body.push_str("    }\n");
        }
    }
    if made.count == 0 {
        return made;
    }
    let mut out = String::new();
    let _ = writeln!(out, "// {}", say!("LumaLink.Numbers.Header", name = name));
    if options {
        out.push_str("use svc.mods\n");
    }
    if regions {
        out.push_str("use svc.system\n");
    }
    if options || regions {
        out.push('\n');
    }
    out.push_str("on start {\n");
    out.push_str(&body);
    out.push_str("}\n");
    made.program = Some(out);
    made
}

/// `when` as a LumaScript test: Some(None) when it always holds, None when it names what the program cannot test.
fn condition(when: Option<&Value>, options: &mut bool, regions: &mut bool) -> Option<Option<String>> {
    let Some(when) = when.and_then(Value::as_object) else { return Some(None) };
    let mut all = Vec::new();
    for (key, value) in when {
        match key.as_str() {
            "option" => {
                for (option, chosen) in value.as_object()? {
                    let read = format!("(mods.option({}) ?? \"\")", quoted(option));
                    let any: Vec<String> = chosen.as_array()?.iter().map(|c| c.as_str().map(|c| format!("{read} == {}", quoted(c)))).collect::<Option<_>>()?;
                    if any.is_empty() {
                        return None;
                    }
                    *options = true;
                    all.push(one_of(any));
                }
            }
            "region" => {
                let any: Vec<String> = value
                    .as_array()?
                    .iter()
                    .map(|r| match r.as_str()? {
                        "RMGE01" => Some("system.facts().region == system.Region.Us".to_string()),
                        "RMGP01" => Some("system.facts().region == system.Region.Eu".to_string()),
                        "RMGJ01" => Some("system.facts().region == system.Region.Jp".to_string()),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                if any.is_empty() {
                    return None;
                }
                *regions = true;
                all.push(one_of(any));
            }
            _ => return None,
        }
    }
    Some((!all.is_empty()).then(|| all.join(" and ")))
}

fn one_of(any: Vec<String>) -> String {
    if any.len() == 1 {
        any.into_iter().next().unwrap_or_default()
    } else {
        format!("({})", any.join(" or "))
    }
}

/// A LumaScript text literal.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A number as an f64 literal that reads back to the same double.
fn literal(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.1}")
    } else {
        format!("{value:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_layers_numbers_are_set_when_its_option_and_region_hold() {
        let layers = [
            LayerNumbers { layer: "base".into(), when: None, numbers: vec![("Game/System/GameRules.cpp#cMaxCoins", 500.0)] },
            LayerNumbers {
                layer: "core".into(),
                when: Some(json!({ "option": { "core": ["on", "hard"] }, "region": ["RMGP01"] })),
                numbers: vec![("Game/System/GameRules.cpp#cPlayerHealth", 1.0), ("Game/Boss/BossKameckBarrier.cpp#cOffsetY", -12.5)],
            },
            LayerNumbers { layer: "words".into(), when: Some(json!({ "language": ["EuFrench"] })), numbers: vec![("Game/System/GameRules.cpp#cLifeUpHealth", 0.0)] },
        ];
        let made = text("Hard", &layers);
        let p = made.program.unwrap();
        assert_eq!(made.count, 3);
        assert_eq!(made.untested, vec!["words".to_string()]);
        assert!(p.contains("use svc.mods\nuse svc.system\n"), "{p}");
        assert!(p.contains("    set_number(\"Game/System/GameRules.cpp#cMaxCoins\", 500.0)\n"), "{p}");
        assert!(p.contains("if ((mods.option(\"core\") ?? \"\") == \"on\" or (mods.option(\"core\") ?? \"\") == \"hard\") and system.facts().region == system.Region.Eu {"), "{p}");
        assert!(p.contains("        set_number(\"Game/Boss/BossKameckBarrier.cpp#cOffsetY\", -12.5)\n"), "{p}");
        assert!(!p.contains("cLifeUpHealth"), "{p}");
    }

    #[test]
    fn no_numbers_no_program() {
        assert!(text("None", &[]).program.is_none());
    }
}
