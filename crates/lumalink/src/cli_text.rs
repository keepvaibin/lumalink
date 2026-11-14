//! The command line in the chosen language: the help of LumaLink and of every command (`LumaLink.Help.*` rows), and
//! clap's headings and refusals (`LumaLink.Cli.*`). Option names and value names (`--game`, `<FILE>`) are what is
//! typed, so they stay as they are.
//!
//! The language is read from the command line before clap sees it, so `--help --language JpJapanese` is Japanese.

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{Arg, ArgAction, Command};
use lumalink_core::{say, text};
use std::ffi::OsString;

const HELP: &str = "LumaLink.Help.";

/// The row of the line of an argument `id` of command `sub` (None: LumaLink itself).
fn arg_row(sub: Option<&str>, id: &str) -> Option<String> {
    let own = sub.map(|s| format!("{HELP}{s}.{id}"));
    let shared = format!("{HELP}Arg.{id}");
    own.filter(|r| text::has(r)).or_else(|| text::has(&shared).then_some(shared))
}

fn command_row(sub: Option<&str>) -> String {
    match sub {
        Some(s) => format!("{HELP}{s}"),
        None => format!("{HELP}About"),
    }
}

fn line(row: &str) -> String {
    text::say(row, &[])
}

const VALUE: &str = "LumaLink.Cli.Value.";

/// A placeholder (`FOLDER`, `OPTIONS`) in the chosen language.
fn value(name: &str) -> String {
    line(&format!("{VALUE}{name}"))
}

/// The usage line after "Usage: ", as clap makes it, with the placeholders in the chosen language (`[OPTIONS]` is
/// clap's own word).
fn usage(cmd: &Command, sub: Option<&str>) -> String {
    let mut parts = vec!["lumalink".to_string()];
    parts.extend(sub.map(str::to_string));
    // The options a command needs, before the others (`new`'s --id, --name, --summary and --author).
    for a in cmd.get_arguments().filter(|a| !a.is_positional() && a.is_required_set()) {
        let name = a.get_value_names().and_then(|v| v.first()).map_or_else(|| value(&a.get_id().as_str().to_uppercase()), |n| n.to_string());
        let part = format!("--{} <{name}>", a.get_long().unwrap_or(a.get_id().as_str()));
        parts.push(if matches!(a.get_action(), ArgAction::Append) { format!("{part}...") } else { part });
    }
    parts.push(format!("[{}]", value("OPTIONS")));
    for a in cmd.get_positionals() {
        let name = a.get_value_names().and_then(|v| v.first()).map_or_else(|| a.get_id().as_str().to_uppercase(), |n| n.to_string());
        let part = if a.is_required_set() { format!("<{name}>") } else { format!("[{name}]") };
        parts.push(if matches!(a.get_action(), ArgAction::Append) { format!("{part}...") } else { part });
    }
    if cmd.has_subcommands() {
        parts.push(format!("<{}>", value("COMMAND")));
    }
    parts.join(" ")
}

/// `cmd` (LumaLink's, from clap's derive) with every line, heading and the help and version flags in the chosen
/// language.
pub fn localized(cmd: Command) -> Command {
    localize(cmd, None)
}

fn localize(cmd: Command, sub: Option<&str>) -> Command {
    let template = format!("{{about-with-newline}}\n{}: {{usage}}\n\n{{all-args}}{{after-help}}", say!("LumaLink.Cli.Usage"));
    let mut cmd = cmd
        .subcommand_value_name(value("COMMAND"))
        .about(line(&command_row(sub)))
        .long_about(None)
        .help_template(template)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .subcommand_help_heading(say!("LumaLink.Cli.Commands"));
    let args: Vec<String> = cmd.get_arguments().map(|a| a.get_id().to_string()).collect();
    for id in args {
        let row = arg_row(sub, &id);
        cmd = cmd.mut_arg(&id, |a| {
            let heading = if a.is_positional() { say!("LumaLink.Cli.Arguments") } else { say!("LumaLink.Cli.Options") };
            // A row missing is a LumaLink fault the tests catch; a release shows the id.
            let help = row.as_deref().map_or_else(|| format!("{HELP}{}.{id}", sub.unwrap_or("Arg")), line);
            // A default is said in the line ("Default: UsEnglish."), not in clap's English "[default: ...]".
            let has_default = a.get_action().takes_values() && !a.get_default_values().is_empty();
            let names: Vec<String> = a.get_value_names().filter(|_| a.get_action().takes_values()).map_or_else(Vec::new, |v| v.iter().map(|n| value(n.as_str())).collect());
            let a = if names.is_empty() { a } else { a.value_names(names) };
            a.help(help).long_help(None).help_heading(heading).hide_default_value(has_default)
        });
    }
    let subs: Vec<String> = cmd.get_subcommands().map(|s| s.get_name().to_string()).collect();
    for s in subs {
        cmd = cmd.mut_subcommand(&s, |c| localize(c, Some(&s)));
    }
    let shown = usage(&cmd, sub);
    cmd = cmd.override_usage(shown);
    let options = say!("LumaLink.Cli.Options");
    cmd = cmd.arg(Arg::new("help").short('h').long("help").action(ArgAction::Help).help(line(&format!("{HELP}Arg.help"))).help_heading(options.clone()));
    if sub.is_none() {
        cmd = cmd.arg(Arg::new("version").short('V').long("version").action(ArgAction::Version).help(line(&format!("{HELP}Arg.version"))).help_heading(options));
    }
    cmd
}

/// Chooses the language named by `--language X` or `--language=X` (before a `--`). An unknown name is refused here,
/// in English, since no language is chosen.
pub fn choose_language(args: &[OsString]) -> Result<(), String> {
    let mut it = args.iter().skip(1).map(|a| a.to_string_lossy());
    while let Some(arg) = it.next() {
        if arg == "--" {
            break;
        }
        let value = if arg == "--language" {
            match it.next() {
                Some(v) => v.into_owned(),
                None => break,
            }
        } else if let Some(v) = arg.strip_prefix("--language=") {
            v.to_string()
        } else {
            continue;
        };
        if !text::set_language(&value) {
            return Err(say!("LumaLink.UnknownLanguage", value = value, list = luma_validate::LANGUAGES.join(", ")));
        }
    }
    Ok(())
}

fn context(e: &clap::Error, kind: ContextKind) -> Option<String> {
    let joined = |v: Vec<String>| (!v.is_empty()).then(|| v.join(", "));
    match e.get(kind)? {
        ContextValue::String(s) => Some(s.clone()),
        ContextValue::Strings(v) => joined(v.clone()),
        ContextValue::StyledStr(s) => Some(s.to_string()),
        ContextValue::StyledStrs(v) => joined(v.iter().map(ToString::to_string).collect()),
        ContextValue::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The command a wrong command line was for, as typed: `lumalink` and its command when one was named.
fn command_of(cmd: &Command, args: &[OsString]) -> String {
    let named = args.iter().skip(1).map(|a| a.to_string_lossy()).find(|a| a != "help" && cmd.find_subcommand(a.as_ref()).is_some());
    match named {
        Some(sub) => format!("lumalink {sub}"),
        None => "lumalink".to_string(),
    }
}

/// The command line read, or what to print instead (help, a version or a refusal) with the exit status: 0 for help and
/// a version, 2 for a wrong command line.
pub enum Read {
    Matches(clap::ArgMatches),
    Print { text: String, to_stderr: bool, code: u8 },
}

/// What to print for clap's `e`: (the text, whether to stderr, the exit status).
pub fn printed(e: &clap::Error, cmd: &Command, args: &[OsString]) -> (String, bool, u8) {
    match refusal(e, cmd, args) {
        Some(why) => (format!("{why}\n"), true, 2),
        None => (e.render().to_string(), e.use_stderr(), u8::try_from(e.exit_code()).unwrap_or(2)),
    }
}

/// Reads `args` against `cmd` (from [`localized`]); `lumalink help [command]` is read as `lumalink [command] --help`.
pub fn read(cmd: &Command, args: &[OsString]) -> Read {
    let print = |e: clap::Error, args: &[OsString]| {
        let (text, to_stderr, code) = printed(&e, cmd, args);
        Read::Print { text, to_stderr, code }
    };
    let matches = match cmd.clone().try_get_matches_from(args) {
        Ok(m) => m,
        Err(e) => return print(e, args),
    };
    let Some(help) = matches.subcommand_matches("help") else { return Read::Matches(matches) };
    let mut again: Vec<OsString> = vec!["lumalink".into()];
    again.extend(help.get_one::<String>("command").map(Into::into));
    again.push("--help".into());
    match cmd.clone().try_get_matches_from(&again) {
        Err(e) => print(e, &again),
        // `--help` always ends the reading; LumaLink's help, should it not.
        Ok(_) => Read::Print { text: cmd.clone().render_help().to_string(), to_stderr: false, code: 0 },
    }
}

/// What clap's refusal `e` says, in the chosen language; None when `e` is help or a version to print as it is.
pub fn refusal(e: &clap::Error, cmd: &Command, args: &[OsString]) -> Option<String> {
    let arg = || context(e, ContextKind::InvalidArg).unwrap_or_else(|| args.iter().skip(1).map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    let why = match e.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => return None,
        ErrorKind::UnknownArgument => say!("LumaLink.Cli.UnknownArgument", arg = arg()),
        ErrorKind::InvalidSubcommand => say!("LumaLink.Cli.UnknownCommand", name = context(e, ContextKind::InvalidSubcommand).unwrap_or_else(arg)),
        ErrorKind::MissingRequiredArgument => say!("LumaLink.Cli.Missing", args = arg()),
        // No command named.
        ErrorKind::MissingSubcommand => say!("LumaLink.Cli.Missing", args = format!("<{}>", value("COMMAND"))),
        ErrorKind::InvalidValue => match context(e, ContextKind::InvalidValue) {
            Some(value) if !value.is_empty() => say!("LumaLink.Cli.InvalidValue", value = value, arg = arg()),
            _ => say!("LumaLink.Cli.Missing", args = arg()),
        },
        ErrorKind::InvalidUtf8 => say!("LumaLink.Cli.NotText"),
        _ => say!("LumaLink.Cli.Wrong", arg = arg()),
    };
    Some(format!("lumalink: {why}\n{}", say!("LumaLink.Cli.SeeHelp", command = command_of(cmd, args))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use std::collections::BTreeSet;

    fn rows_used(cmd: &Command, sub: Option<&str>, used: &mut BTreeSet<String>, missing: &mut Vec<String>) {
        used.insert(command_row(sub));
        for a in cmd.get_arguments() {
            let id = a.get_id().as_str();
            for name in a.get_value_names().filter(|_| a.get_action().takes_values()).unwrap_or_default() {
                let row = format!("{VALUE}{name}");
                if text::has(&row) {
                    used.insert(row);
                } else {
                    missing.push(format!("{}: {id}: <{name}>", sub.unwrap_or("lumalink")));
                }
            }
            match arg_row(sub, id) {
                Some(r) => {
                    used.insert(r);
                }
                None => missing.push(format!("{}: {id}", sub.unwrap_or("lumalink"))),
            }
        }
        for s in cmd.get_subcommands() {
            rows_used(s, Some(s.get_name()), used, missing);
        }
    }

    /// Every command and argument has its line, and every help row is some command's or argument's.
    #[test]
    fn every_command_and_argument_has_a_line_and_every_line_is_used() {
        let mut used = BTreeSet::from([format!("{HELP}Arg.help"), format!("{HELP}Arg.version"), format!("{VALUE}OPTIONS"), format!("{VALUE}COMMAND")]);
        let mut missing = Vec::new();
        let mut cmd = crate::Cli::command();
        cmd.build();
        rows_used(&cmd, None, &mut used, &mut missing);
        assert!(missing.is_empty(), "arguments without a line: {missing:?}");
        let rows: BTreeSet<String> = text::ids().filter(|id| id.starts_with(HELP) || id.starts_with(VALUE)).map(str::to_string).collect();
        let unused: Vec<_> = rows.difference(&used).collect();
        assert!(unused.is_empty(), "help rows no command or argument uses: {unused:?}");
    }

    /// `text` holds `word` with no letter or digit against either end.
    fn has_word(text: &str, word: &str) -> bool {
        text.match_indices(word).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + word.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    }

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn rendered(args: &[&str]) -> String {
        let args = os(args);
        match read(&localized(crate::Cli::command()), &args) {
            Read::Print { text, code, .. } => {
                assert_eq!(code == 0, args.iter().any(|a| a == "--help"), "{args:?}: {text}");
                text.trim_end().to_string()
            }
            Read::Matches(_) => panic!("{args:?} parsed"),
        }
    }

    /// In each of the 14 languages: LumaLink's help and every command's are in it, with no English line left where
    /// that language has its own; and clap's refusals are made in it. One test, since the language is chosen once
    /// for the whole program.
    #[test]
    fn help_and_refusals_are_in_every_language() {
        let lines: Vec<&str> = text::ids().filter(|id| id.starts_with(HELP) || id.starts_with("LumaLink.Cli.")).filter(|id| !text::template(0, id).unwrap().contains('{')).collect();
        let subs: Vec<String> = crate::Cli::command().get_subcommands().map(|s| s.get_name().to_string()).collect();
        for (i, language) in luma_validate::LANGUAGES.iter().enumerate() {
            choose_language(&os(&["lumalink", "--language", language])).unwrap();
            let own = |id: &str| text::say_in(i, id, &[]);
            let mut pages = vec![(None, rendered(&["lumalink", "--help"]))];
            pages.extend(subs.iter().map(|s| (Some(s.as_str()), rendered(&["lumalink", s, "--help"]))));
            for (sub, page) in &pages {
                assert!(page.contains(&own(&command_row(*sub))), "{language} {sub:?}: {page}");
                assert!(page.contains(&format!("{}: ", own("LumaLink.Cli.Usage"))), "{language} {sub:?}: {page}");
                assert!(page.contains(&own("LumaLink.Cli.Options")), "{language} {sub:?}: {page}");
                for id in &lines {
                    let (e, theirs) = (text::template(0, id).unwrap(), text::template(i, id).unwrap());
                    assert!(theirs == e || !has_word(page, e), "{language} {sub:?}: English line {e:?} left in:\n{page}");
                }
                for clap_english in ["Usage:", "Print help", "Print version", "[default:", "lumalink.exe"] {
                    let ours = clap_english == "Usage:" && own("LumaLink.Cli.Usage") == "Usage";
                    assert!(ours || !page.contains(clap_english), "{language} {sub:?}: clap's {clap_english:?} in:
{page}");
                }
            }
            let root = &pages[0].1;
            assert!(root.contains(&own("LumaLink.Cli.Commands")), "{language}: {root}");
            for s in &subs {
                assert!(root.contains(&own(&format!("{HELP}{s}"))), "{language}: {s}");
            }

            let wrong = rendered(&["lumalink", "check", "x", "--nope"]);
            assert!(wrong.starts_with(&format!("lumalink: {}", text::say_in(i, "LumaLink.Cli.UnknownArgument", &[("arg", &"--nope")]))), "{language}: {wrong}");
            assert!(wrong.ends_with(&text::say_in(i, "LumaLink.Cli.SeeHelp", &[("command", &"lumalink check")])), "{language}: {wrong}");
            let unknown = rendered(&["lumalink", "paint"]);
            assert!(unknown.contains(&text::say_in(i, "LumaLink.Cli.UnknownCommand", &[("name", &"paint")])), "{language}: {unknown}");
            let missing = rendered(&["lumalink", "check"]);
            assert!(missing.contains(&text::say_in(i, "LumaLink.Cli.Missing", &[("args", &format!("<{}>", own("LumaLink.Cli.Value.PATH")))])), "{language}: {missing}");
            let no_value = rendered(&["lumalink", "pack", "x", "--output"]);
            assert!(no_value.contains(&text::say_in(i, "LumaLink.Cli.Missing", &[("args", &format!("--output <{}>", own("LumaLink.Cli.Value.FILE")))])), "{language}: {no_value}");
            let no_command = rendered(&["lumalink", "--language", language]);
            assert!(no_command.contains(&text::say_in(i, "LumaLink.Cli.Missing", &[("args", &format!("<{}>", own("LumaLink.Cli.Value.COMMAND")))])), "{language}: {no_command}");
            let help_unknown = rendered(&["lumalink", "help", "paint"]);
            assert!(help_unknown.ends_with(&text::say_in(i, "LumaLink.Cli.SeeHelp", &[("command", &"lumalink")])), "{language}: {help_unknown}");
        }
        choose_language(&os(&["lumalink"])).unwrap();
        assert!(choose_language(&os(&["lumalink", "--language=Klingon"])).is_err());
        assert!(choose_language(&os(&["lumalink", "--", "--language", "Klingon"])).is_ok());
        text::set_language("UsEnglish");
    }
}
