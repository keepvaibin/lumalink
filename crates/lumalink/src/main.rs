//! The command line: `new`, `check`, `pack`, `list`, `extract`, `explain`, `verify`, `run`, `convert`,
//! `add`, `lsp`, `format`, and `build` and `test`. Every
//! command that reads the game reads the installed one (`lumalink_core::installed`) unless `--game` (and `--names`) name another, or
//! `--no-installed` leaves it out.
//!
//! Exit status: 0 done (Notes may have been shown); 1 the mod has a problem or the command refused, each named;
//! 2 the command could not run (a file that cannot be read, a wrong argument).

use anyhow::{bail, Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use luma_validate::names::Names;
use luma_validate::{Finding, Level, Report};
use lumalink_core::inspect::{self, ColumnNames};
use lumalink_core::jobs::{self, Difference, Install, LumaList, NewMod, PackOutcome};
use lumalink_core::{say, text};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod cli_text;

/// The help of every command and argument is its `LumaLink.Help.*` row, in the chosen language (`cli_text`); the doc
/// comments here are for the code.
#[derive(Parser)]
#[command(name = "lumalink", version, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// The language LumaLink and the validator speak: one of the game's 14, by its name in the game.
    #[arg(long, global = true, value_name = "LANGUAGE", default_value = "UsEnglish")]
    language: String,
}

/// What the validator is given beyond the mod itself. Each input left out keeps its rules from running, and each such
/// rule is named in the report (docs/modding/format/validator.md, section 2).
#[derive(clap::Args, Clone)]
struct Inputs {
    /// The game's function rows (a kernel build's `luma` folder), for the rules on a mod's code. Replaced by the
    /// game's self-description file when it exists.
    #[arg(long, value_name = "FOLDER")]
    names: Option<PathBuf>,
    /// A platform to check the code for (repeat for several). Default: all nine.
    #[arg(long = "platform", value_name = "PLATFORM")]
    platforms: Vec<String>,
    /// Your imported game (Nebula's game.pak), for the rules on layer files and patches: paths, tables, keys, columns.
    #[arg(long, value_name = "FILE")]
    game: Option<PathBuf>,
    /// Check without the installed game: what --game and --names do not name is left out, and its rules are named.
    #[arg(long)]
    no_installed: bool,
    /// Your mod folder: the mods (folders and .luma files) a mod's needs are found in. Without it needs are not checked.
    #[arg(long, value_name = "FOLDER")]
    mods: Option<PathBuf>,
}

impl Inputs {
    /// These inputs as the jobs take them, the installed game's game.pak and self-description folder in place of the
    /// ones not named, each one used said once on stderr (stdout may be a report as JSON).
    fn resolved(&self) -> jobs::Inputs {
        let given = jobs::Inputs { names: self.names.clone(), platforms: self.platforms.clone(), game: self.game.clone(), no_installed: self.no_installed, mods: self.mods.clone() };
        if self.no_installed || (self.game.is_some() && self.names.is_some()) {
            return given;
        }
        given.resolved(installed().as_ref())
    }
}

/// The game installed on this computer, said once on stderr the first time it is used.
fn installed() -> Option<lumalink_core::installed::Installed> {
    static FOUND: std::sync::OnceLock<Option<lumalink_core::installed::Installed>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let found = lumalink_core::installed::Installed::find();
            if let Some(f) = &found {
                eprintln!("{}", say!("LumaLink.Installed", path = f.pak.display()));
            }
            found
        })
        .clone()
}

/// The installed game's game.pak, or the refusal that names the option to give.
fn installed_pak() -> Result<PathBuf> {
    installed().map(|f| f.pak).with_context(|| say!("LumaLink.NoInstalled"))
}

#[derive(Subcommand)]
enum Command {
    /// Start a mod: a folder with its luma.json and an empty base layer.
    New {
        /// The folder to make; it must not exist.
        folder: PathBuf,
        /// The mod's id: 3 to 64 of a-z, 0-9, '-' and '.', starting with a letter.
        #[arg(long)]
        id: String,
        /// The mod's name, as players see it.
        #[arg(long)]
        name: String,
        /// One line on what the mod does.
        #[arg(long, value_name = "TEXT")]
        summary: String,
        /// Who made it (repeat for several).
        #[arg(long = "author", value_name = "AUTHOR", required = true)]
        authors: Vec<String>,
    },
    /// Check a mod folder or a .luma with the game's own validator and list every problem.
    Check {
        /// A mod folder or a .luma file.
        path: PathBuf,
        #[command(flatten)]
        inputs: Inputs,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Write a mod folder as a .luma, after the validator passes it with every rule checked.
    Pack {
        /// The mod folder.
        folder: PathBuf,
        /// The .luma to write. Default: `<id>-<version>.luma` beside the folder.
        #[arg(long, short, value_name = "FILE")]
        output: Option<PathBuf>,
        #[command(flatten)]
        inputs: Inputs,
    },
    /// List what is in a .luma, or (with --game) a kind of thing in your imported game.
    List {
        /// A .luma file; or, with --game: tables, models, messages or files.
        #[arg(value_name = "ITEM")]
        what: String,
        /// Only those whose path or column holds this (tables, models); the folder (files); the language (messages).
        filter: Option<String>,
        /// Your imported game (Nebula's game.pak).
        #[arg(long, value_name = "FILE")]
        game: Option<PathBuf>,
        /// The game's self-description, for the names of table columns.
        #[arg(long = "self", value_name = "FILE")]
        self_description: Option<PathBuf>,
    },
    /// Write files out of a .luma, or (with --game) out of your imported game.
    Extract {
        /// A .luma file; or, with --game, a path in the game.
        #[arg(value_name = "PATH")]
        from: String,
        /// Entries or folders of the .luma to write (all when none).
        #[arg(value_name = "ENTRY")]
        names: Vec<String>,
        /// The folder to write into.
        #[arg(long, short, value_name = "FOLDER")]
        output: PathBuf,
        /// Your imported game (Nebula's game.pak).
        #[arg(long, value_name = "FILE")]
        game: Option<PathBuf>,
        /// The game's self-description, for the names of table columns.
        #[arg(long = "self", value_name = "FILE")]
        self_description: Option<PathBuf>,
    },
    /// Say what a validator code (L203, or 203) means, in a language.
    Explain { code: String },
    /// Check that a .luma is exactly what its folder packs to.
    Verify {
        /// The .luma file.
        #[arg(value_name = "FILE")]
        luma: PathBuf,
        /// The mod folder it was packed from.
        #[arg(long, value_name = "FOLDER")]
        source: PathBuf,
    },
    /// Build a mod's code (LumaScript and C) for every platform.
    Build {
        /// The mod folder.
        folder: PathBuf,
        /// The game's function rows and types file (a kernel build's luma folder), for the rules on a mod's code.
        #[arg(long, value_name = "FOLDER")]
        names: PathBuf,
        /// The folder with the game's luma.h. Default: LUMA_INCLUDE.
        #[arg(long, value_name = "FOLDER")]
        sdk: Option<PathBuf>,
        /// The clang to build with. Default: LUMALINK_CLANG, then LLVM's own folder, then the path.
        #[arg(long, value_name = "FILE")]
        clang: Option<PathBuf>,
        /// Keep the generated C in this folder. Default: a temporary folder, removed after.
        #[arg(long, value_name = "FOLDER")]
        work: Option<PathBuf>,
        /// A platform to build for (repeat for several). Default: all nine.
        #[arg(long = "platform", value_name = "PLATFORM")]
        platforms: Vec<String>,
    },
    /// Start the installed game with a mod: a folder is packed first (as `pack` packs it), then imported into the
    /// game's mods, then the game starts.
    Run {
        /// A mod folder or a .luma file.
        path: PathBuf,
        #[command(flatten)]
        inputs: Inputs,
        /// Arguments for the game after --, as its command line takes them (--seconds, --settings, --data...).
        #[arg(last = true, value_name = "ARGUMENTS")]
        args: Vec<OsString>,
    },
    /// Run a mod's tests through the proof harness: built first, each tests/**/test.json run, each check said.
    Test {
        folder: PathBuf,
        /// A game build's folder with the harness's parts (the host extension and the game half). Default: LUMA_TEST_BIN.
        #[arg(long, value_name = "FOLDER")]
        bin: Option<PathBuf>,
        /// The harness's runner (luma_test.py). Default: LUMA_TEST_RUNNER, then tools/lumacore/luma_test.py above --bin.
        #[arg(long, value_name = "FILE")]
        runner: Option<PathBuf>,
        /// The Python that runs it. Default: LUMALINK_PYTHON, then python.
        #[arg(long, value_name = "FILE")]
        python: Option<PathBuf>,
        /// Where the runs are left. Default: <folder>-tests beside the mod.
        #[arg(long, value_name = "FOLDER")]
        work: Option<PathBuf>,
        /// The run lock. Default: LUMA_TEST_LOCK, then run.lock in the work folder.
        #[arg(long, value_name = "FILE")]
        lock: Option<PathBuf>,
        /// d3d12 or vulkan. Default: the runner's.
        #[arg(long, value_name = "RENDERER")]
        renderer: Option<String>,
    },
    /// Put LumaScript files in the one style: four spaces for each open bracket, one blank line at most.
    Format {
        /// `.lum` files, or folders (every `.lum` under each).
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Write nothing: name each file not in the style, and exit 1 if there is one.
        #[arg(long)]
        check: bool,
    },
    /// Add a character, model or other part to a mod by command.
    Add {
        /// What to add: character, need, conflict, after, before, takes-over, author, text, permission, option,
        /// layer, file, asset, texture, icon, picture, save, script.
        #[arg(value_name = "ITEM")]
        what: String,
        /// What ITEM takes: a model (.glb), a mod's id, a name, a language, a file.
        #[arg(value_name = "VALUE")]
        value: String,
        folder: PathBuf,
        /// character: the game character or model it stands in for: mario, luigi, or a model's name.
        #[arg(long, value_name = "NAME")]
        replaces: Option<String>,
        /// character: written as the mod's own model layers/base/assets/PATH.glb, drawn on that character's skeleton by
        /// the mod's code; the game's model and its parts stay.
        #[arg(long = "as", value_name = "PATH")]
        as_asset: Option<String>,
        /// character: GAME=SOURCE, the game's joint GAME follows the model's joint SOURCE whatever the automatic
        /// match finds; GAME= follows none (repeat for several).
        #[arg(long = "joint", value_name = "GAME=SOURCE")]
        joints: Vec<String>,
        /// need: the versions of the mod that do (a semver range). Default: any.
        #[arg(long, value_name = "RANGE")]
        version: Option<String>,
        /// conflict, permission, save: why, one line shown to the player.
        #[arg(long, value_name = "TEXT")]
        why: Option<String>,
        /// permission net.internet: a host it reaches (repeat for several).
        #[arg(long = "host", value_name = "HOST")]
        hosts: Vec<String>,
        /// permission fs.exchange: a root it reads or writes (repeat for several).
        #[arg(long = "root", value_name = "ROOT")]
        roots: Vec<String>,
        /// option: choice, number, toggle, text or key.
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        /// option: its value until the player sets one.
        #[arg(long, value_name = "VALUE")]
        default: Option<String>,
        /// option: its name on the Mods screen; text: the mod's name in that language.
        #[arg(long, value_name = "TEXT")]
        name: Option<String>,
        /// text: the mod's summary in that language.
        #[arg(long, value_name = "TEXT")]
        summary: Option<String>,
        /// option number: the least value.
        #[arg(long, value_name = "NUMBER", allow_negative_numbers = true)]
        min: Option<f64>,
        /// option number: the most value.
        #[arg(long, value_name = "NUMBER", allow_negative_numbers = true)]
        max: Option<f64>,
        /// option number: the step between values.
        #[arg(long, value_name = "NUMBER")]
        step: Option<f64>,
        /// option choice: a value, or value=name shown (repeat for each, in order).
        #[arg(long = "value", value_name = "VALUE")]
        values: Vec<String>,
        /// option text: the most characters.
        #[arg(long, value_name = "COUNT")]
        max_length: Option<u64>,
        /// layer: it applies when the option has the value (repeat; a key twice: any of its values).
        #[arg(long = "when-option", value_name = "SETTING")]
        when_options: Vec<String>,
        /// layer: it applies in these regions (repeat for several).
        #[arg(long = "when-region", value_name = "REGION")]
        when_regions: Vec<String>,
        /// layer: it applies in these languages' folders (repeat for several).
        #[arg(long = "when-language", value_name = "LANGUAGE")]
        when_languages: Vec<String>,
        /// layer: it applies on these platforms (repeat for several).
        #[arg(long = "when-platform", value_name = "PLATFORM")]
        when_platforms: Vec<String>,
        /// layer: the mod's code chooses it before content loads.
        #[arg(long)]
        when_code: bool,
        /// file: the game path it is placed at; asset: its path in the mod (default its name); texture: the game
        /// texture's name (default the file's name).
        #[arg(long, value_name = "PATH")]
        at: Option<String>,
        /// file, asset, texture: the layer it goes into.
        #[arg(long, value_name = "LAYER", default_value = "base")]
        layer: String,
        /// save: the region the save file is for.
        #[arg(long, value_name = "REGION")]
        region: Option<String>,
        #[command(flatten)]
        inputs: Inputs,
    },
    /// Convert a published mod (Riivolution pack, disc-layout folder, loose files, texture pack, Gecko list...) to a
    /// mod folder in the one form, each file cooked as the game cooks it first.
    Convert {
        /// The mod: a folder, an archive (.zip, .7z, .rar) or a file.
        #[arg(value_name = "PATH")]
        from: PathBuf,
        /// The mod folder to write; it must not exist.
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,
        /// Your imported game (Nebula's game.pak): files are placed and textures named against it.
        #[arg(long, value_name = "FILE")]
        game: Option<PathBuf>,
        /// The mod's name, in place of the pack's or the file's.
        #[arg(long)]
        name: Option<String>,
        /// Who made it (repeat for several), in place of the pack's authors.
        #[arg(long = "author", value_name = "AUTHOR")]
        authors: Vec<String>,
        /// Write the mod without the files the game cannot use, naming each.
        #[arg(long)]
        accept_losses: bool,
    },
    /// The editor server: the Language Server Protocol over stdin and stdout for a mod's LumaScript.
    Lsp {
        #[arg(long, value_name = "FOLDER")]
        names: Option<PathBuf>,
        /// The folder with the game's luma.h, for the services' functions. Default: LUMA_INCLUDE, then the
        /// self-description folder's `sdk`.
        #[arg(long, value_name = "FOLDER")]
        sdk: Option<PathBuf>,
        #[arg(long)]
        no_installed: bool,
    },
    /// Every service function of an SDK and whether scripts can call it: one line each, `function`, its name, then
    /// `script`, `native` (C mods only, by design) or `c` (C mods only); fields scripts cannot read as `field` lines.
    Services {
        /// The folder with the game's luma.h. Default: LUMA_INCLUDE.
        #[arg(long, value_name = "FOLDER")]
        sdk: Option<PathBuf>,
        /// Only what scripts cannot reach.
        #[arg(long)]
        unreachable: bool,
    },
    /// Print the help of LumaLink or of one command.
    Help { command: Option<String> },
}

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let Err(why) = cli_text::choose_language(&args) {
        eprintln!("lumalink: {why}");
        return ExitCode::from(2);
    }
    let cli = match parse(&args) {
        Ok(cli) => cli,
        Err(code) => return code,
    };
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("lumalink: {e:#}");
            ExitCode::from(2)
        }
    }
}

/// The command line read with the help and clap's refusals in the chosen language; Err: help, a version or a refusal
/// was printed, with the exit status (0 for help and a version, 2 for a wrong command line).
fn parse(args: &[std::ffi::OsString]) -> std::result::Result<Cli, ExitCode> {
    let print = |text: String, to_stderr: bool, code: u8| {
        if to_stderr {
            eprint!("{text}");
        } else {
            print!("{text}");
        }
        ExitCode::from(code)
    };
    let cmd = cli_text::localized(Cli::command());
    match cli_text::read(&cmd, args) {
        cli_text::Read::Matches(matches) => Cli::from_arg_matches(&matches).map_err(|e| {
            let (text, to_stderr, code) = cli_text::printed(&e, &cmd, args);
            print(text, to_stderr, code)
        }),
        cli_text::Read::Print { text, to_stderr, code } => Err(print(text, to_stderr, code)),
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    if !text::set_language(&cli.language) {
        bail!(say!("LumaLink.UnknownLanguage", value = cli.language, list = luma_validate::LANGUAGES.join(", ")));
    }
    match cli.command {
        Command::New { folder, id, name, summary, authors } => new_mod(&folder, &id, &name, &summary, &authors),
        Command::Check { path, inputs, json } => {
            let report = validate(&path, &inputs)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&jobs::report_json(&report))?);
            } else {
                print_report(&report);
            }
            Ok(if report.passed() { ExitCode::SUCCESS } else { ExitCode::from(1) })
        }
        Command::Pack { folder, output, inputs } => pack(&folder, output, &inputs),
        Command::List { what, filter, game, self_description } => list(&what, game.as_deref(), self_description.as_deref(), filter.as_deref()),
        Command::Extract { from, names, output, game, self_description } => extract(&from, &names, &output, game.as_deref(), self_description.as_deref()),
        Command::Explain { code } => explain(&code),
        Command::Verify { luma, source } => verify(&luma, &source),
        Command::Build { folder, names, sdk, clang, work, platforms } => build(&folder, &names, sdk, clang, work, &platforms),
        Command::Run { path, inputs, args } => run_game(&path, &inputs, &args),
        Command::Test { folder, bin, runner, python, work, lock, renderer } => test(&folder, bin, runner, python, work, lock, renderer),
        Command::Format { paths, check } => format(&paths, check),
        Command::Add { .. } => add(cli.command),
        Command::Convert { from, folder, game, name, authors, accept_losses } => {
            // The installed game places the files and names the textures when no other is named.
            let game = game.or_else(|| installed().map(|f| f.pak));
            convert(&from, &folder, game, name, authors, accept_losses)
        }
        Command::Lsp { names, sdk, no_installed } => lsp(names, sdk, no_installed),
        Command::Services { sdk, unreachable } => services(sdk, unreachable),
        // parse prints the help and returns before run.
        Command::Help { .. } => Ok(ExitCode::SUCCESS),
    }
}

/// The installed game started with the mod at `path` (a folder, packed first as `pack` packs it, or a
/// .luma), imported into the game's mods by the game's own mod tool (`jobs::install`).
fn run_game(path: &Path, inputs: &Inputs, game_args: &[OsString]) -> Result<ExitCode> {
    let found = installed().with_context(|| say!("LumaLink.NoInstalled"))?;
    let inputs = inputs.resolved();
    match jobs::install(path, &inputs, &found, game_args)? {
        Install::Imported { luma } => println!("{}", say!("LumaLink.Run.Imported", file = luma.display())),
        Install::NotImported { luma, why } => {
            println!("{}", say!("LumaLink.Run.NotImported", file = luma.display(), why = why));
            return Ok(ExitCode::from(1));
        }
        Install::NotPacked(report, outcome) => return print_pack(&report, outcome),
        Install::Refused(report) => {
            print_report(&report);
            return Ok(ExitCode::from(1));
        }
    }
    let pak = inputs.game.clone().unwrap_or_else(|| found.pak.clone());
    let program = jobs::start(&found, &pak, game_args)?;
    println!("{}", say!("LumaLink.Run.Started", program = program.display()));
    Ok(ExitCode::SUCCESS)
}

/// `add character <model.glb> <folder> --replaces <name>` (`jobs::add_character`): the report printed and
/// written beside the folder, then the mod's check.
/// A part of a mod by command (`lumalink_core::add`), or a character. An option the item does not
/// take, or one it needs and was not given, is refused by name before anything is read.
fn add(command: Command) -> Result<ExitCode> {
    use lumalink_core::add::{self, Added, LayerPart, OptionPart, Part};
    let Command::Add {
        what, value, folder, replaces, as_asset, joints, version, why, hosts, roots, kind, default, name, summary, min, max, step, values,
        max_length, when_options, when_regions, when_languages, when_platforms, when_code, at, layer, region, inputs,
    } = command
    else {
        unreachable!("add is called with add")
    };
    if !add::Part::WORDS.contains(&what.as_str()) {
        bail!(say!("LumaLink.Add.What", value = what, list = add::Part::WORDS.join(", ")));
    }
    let given: [(&str, bool); 24] = [
        ("--replaces", replaces.is_some()),
        ("--as", as_asset.is_some()),
        ("--joint", !joints.is_empty()),
        ("--version", version.is_some()),
        ("--why", why.is_some()),
        ("--host", !hosts.is_empty()),
        ("--root", !roots.is_empty()),
        ("--kind", kind.is_some()),
        ("--default", default.is_some()),
        ("--name", name.is_some()),
        ("--summary", summary.is_some()),
        ("--min", min.is_some()),
        ("--max", max.is_some()),
        ("--step", step.is_some()),
        ("--value", !values.is_empty()),
        ("--max-length", max_length.is_some()),
        ("--when-option", !when_options.is_empty()),
        ("--when-region", !when_regions.is_empty()),
        ("--when-language", !when_languages.is_empty()),
        ("--when-platform", !when_platforms.is_empty()),
        ("--when-code", when_code),
        ("--at", at.is_some()),
        ("--layer", layer != "base"),
        ("--region", region.is_some()),
    ];
    // What each item takes, and of that what it needs.
    let (takes, needs): (&[&str], &[&str]) = match what.as_str() {
        "character" => (&["--replaces", "--as", "--joint"], &["--replaces"]),
        "need" => (&["--version"], &[]),
        "conflict" => (&["--why"], &["--why"]),
        "text" => (&["--name", "--summary"], &[]),
        "permission" => (&["--why", "--host", "--root"], &["--why"]),
        "option" => (&["--kind", "--default", "--name", "--min", "--max", "--step", "--value", "--max-length"], &["--kind", "--default", "--name"]),
        "layer" => (&["--when-option", "--when-region", "--when-language", "--when-platform", "--when-code"], &[]),
        "file" => (&["--at", "--layer"], &["--at"]),
        "asset" | "texture" => (&["--at", "--layer"], &[]),
        "save" => (&["--region", "--why"], &["--region", "--why"]),
        _ => (&[], &[]),
    };
    for (flag, is) in given {
        if is && !takes.contains(&flag) {
            bail!(say!("LumaLink.Add.NotFor", flag = flag, what = what));
        }
    }
    for flag in needs {
        if !given.iter().any(|(f, is)| f == flag && *is) {
            bail!(say!("LumaLink.Add.Missing", flag = flag, what = what));
        }
    }
    let check = inputs.resolved();
    if what == "character" {
        let pak = match check.game.clone() {
            Some(pak) => pak,
            None => installed_pak()?,
        };
        let mut choices = Vec::new();
        for j in &joints {
            let Some((g, s)) = j.split_once('=') else { bail!(say!("LumaLink.Character.JointForm", value = j)) };
            choices.push((g.trim().to_string(), s.trim().to_string()));
        }
        let made = jobs::add_character(Path::new(&value), &folder, replaces.as_deref().unwrap_or_default(), as_asset.as_deref(), &choices, &pak, &check)?;
        for line in &made.lines {
            println!("{line}");
        }
        println!("{}", say!("LumaLink.Character.Written", file = made.file.display(), report = made.report_file.display()));
        print_report(&made.report);
        return Ok(if made.report.passed() { ExitCode::SUCCESS } else { ExitCode::from(1) });
    }
    let text = |t: Option<String>| t.unwrap_or_default();
    let from = PathBuf::from(&value);
    let part = match what.as_str() {
        "need" => Part::Need { id: value, version: version.unwrap_or_else(|| "*".into()) },
        "conflict" => Part::Conflict { id: value, why: text(why) },
        "after" => Part::Load { id: value, before: false },
        "before" => Part::Load { id: value, before: true },
        "takes-over" => Part::TakesOver { id: value },
        "author" => Part::Author { name: value },
        "text" => Part::Text { language: value, name, summary },
        "permission" => Part::Permission { name: value, why: text(why), hosts, roots },
        "option" => Part::Option(OptionPart { key: value, kind: text(kind), default: text(default), name: text(name), min, max, step, values, max_length }),
        "layer" => Part::Layer(LayerPart { name: value, options: when_options, regions: when_regions, languages: when_languages, platforms: when_platforms, code: when_code }),
        "file" => Part::File { from, at: text(at), layer },
        "asset" => {
            let at = match at {
                Some(at) => at,
                None => from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            };
            Part::Asset { from, at, layer }
        }
        "texture" => Part::Texture { from, name: at, layer },
        "icon" => Part::Icon { from },
        "picture" => Part::Picture { from },
        "save" => Part::Save { from, region: text(region), why: text(why) },
        "script" => Part::Script { name: value },
        _ => unreachable!("every word of Part::WORDS"),
    };
    match add::add(&folder, &part, &check)? {
        Added::Done { lines, report } => {
            for line in &lines {
                println!("{line}");
            }
            print_report(&report);
            Ok(if report.passed() { ExitCode::SUCCESS } else { ExitCode::from(1) })
        }
        Added::Refused { errors } => {
            for f in &errors {
                print_finding(f);
            }
            println!("{}", say!("LumaLink.Add.Refused", folder = folder.display(), count = errors.len()));
            Ok(ExitCode::from(1))
        }
    }
}

/// The editor server, reading the game from --names, else the installed game's self-description. Without
/// either it serves the language alone (the game's names are then not known, which its diagnostics say).
fn lsp(names: Option<PathBuf>, sdk: Option<PathBuf>, no_installed: bool) -> Result<ExitCode> {
    let folder = names.or_else(|| if no_installed { None } else { installed().and_then(|f| f.self_description()) });
    let sdk = sdk.or_else(|| std::env::var_os("LUMA_INCLUDE").map(PathBuf::from));
    let game = match &folder {
        Some(folder) => {
            let game = lumalink_core::editor::GameView::read(folder, sdk.as_deref()).map_err(|why| anyhow::anyhow!(say!("LumaLink.Rows.BadTypes", path = folder.display(), why = why)))?;
            for e in &game.unread {
                eprintln!("{}", say!("LumaLink.Build.ServiceUnread", why = e));
            }
            game
        }
        None => {
            eprintln!("{}", say!("LumaLink.Lsp.NoGame"));
            lumalink_core::editor::GameView::default()
        }
    };
    let language = luma_validate::LANGUAGES.iter().position(|l| *l == text::language()).unwrap_or(0);
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stdout().lock();
    let code = lumalink_core::lsp::Server::new(game, language).serve(&mut input, &mut output)?;
    Ok(ExitCode::from(code as u8))
}

/// The mod's tests through the harness (`jobs::test`), each verdict and each check said; exit 0 only when
/// every test passed on every renderer.
fn test(folder: &Path, bin: Option<PathBuf>, runner: Option<PathBuf>, python: Option<PathBuf>, work: Option<PathBuf>, lock: Option<PathBuf>, renderer: Option<String>) -> Result<ExitCode> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let Some(bin) = bin.or_else(|| env("LUMA_TEST_BIN")) else {
        bail!(say!("LumaLink.Test.NoBin"));
    };
    let runner = match runner.or_else(|| env("LUMA_TEST_RUNNER")).or_else(|| jobs::find_runner(&bin)) {
        Some(r) => r,
        None => bail!(say!("LumaLink.Test.NoRunner", path = bin.join("tools/lumacore/luma_test.py").display())),
    };
    let python = python.or_else(|| env("LUMALINK_PYTHON")).unwrap_or_else(|| PathBuf::from("python"));
    let work = work.unwrap_or_else(|| {
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "mod".into());
        folder.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).join(format!("{name}-tests"))
    });
    let lock = lock.or_else(|| env("LUMA_TEST_LOCK")).unwrap_or_else(|| work.join("run.lock"));
    let setup = jobs::TestSetup { bin, runner, python, work, lock, renderer };
    // The mod's code is built by `lumalink build` (every platform) before its tests run.
    let names = env("LUMA_NAMES").unwrap_or_else(|| setup.bin.clone());
    let build = |f: &Path| build_for_test(f, &names);
    let mut each = |run: &jobs::TestRun| {
        let name = run.test.display();
        if run.results.is_empty() {
            eprint!("{}", run.said);
            println!("{}", say!("LumaLink.Test.NoResult", test = name, code = run.code));
        }
        for r in &run.results {
            println!("{}", say!("LumaLink.Test.Verdict", test = r.test, renderer = r.renderer, verdict = r.verdict, reason = r.reason));
            for (check, result, actual) in &r.checks {
                println!("{}", say!("LumaLink.Test.Check", result = result, check = check, actual = actual));
            }
        }
    };
    let runs = jobs::test(folder, &setup, &build, &mut each)?;
    let passed = runs.iter().filter(|r| r.passed()).count();
    println!("{}", say!("LumaLink.Test.Summary", passed = passed, count = runs.len(), work = setup.work.display()));
    Ok(if passed == runs.len() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

/// `lumalink build`: the mod's LumaScript checked, written as C and built into `code/<platform>/mod.<ending>` for each
/// platform given, all nine by default (plans 4.3, 8.4).
fn build(folder: &Path, names: &Path, sdk: Option<PathBuf>, clang: Option<PathBuf>, work: Option<PathBuf>, platforms: &[String]) -> Result<ExitCode> {
    Ok(match build_mod(folder, names, sdk, clang, work, platforms)? {
        None => ExitCode::SUCCESS,
        Some(_) => ExitCode::from(1),
    })
}

/// [`build`]'s work: None built, or the count of script errors (each said).
fn build_mod(folder: &Path, names: &Path, sdk: Option<PathBuf>, clang: Option<PathBuf>, work: Option<PathBuf>, platforms: &[String]) -> Result<Option<usize>> {
    use lumalink_core::build::{self, BuildError};
    let mut rows = Names::from_rows_folder(names, 1).map_err(|e| jobs::rows_error(names, e))?;
    rows.api = 1;
    // Without the types file every game type is unknown and each call would be refused one by one: say why once.
    if rows.types.is_none() {
        bail!(say!("LumaLink.Build.NoTypes", folder = names.display(), file = luma_validate::names::TYPES_FILE));
    }
    let Some(sdk) = sdk.or_else(|| std::env::var_os("LUMA_INCLUDE").map(PathBuf::from)) else {
        bail!(say!("LumaLink.Build.NoSdk", folder = "LUMA_INCLUDE"));
    };
    // The services the SDK's headers declare (docs/modding/language/language.md): their calls are kind 5 imports the loader checks.
    // A service header the compiler cannot read is said (its scripts then find no such module, S0027).
    let (game, unread) = lumalink_core::build::game_of(&rows, names, Some(&sdk)).map_err(game_error)?;
    for e in &unread {
        eprintln!("{}", say!("LumaLink.Build.ServiceUnread", why = e));
    }
    let clang = clang.unwrap_or_else(build::default_clang);
    let temp;
    let work = match work {
        Some(w) => w,
        None => {
            temp = tempfile::tempdir().context(say!("LumaLink.Build.NoWork"))?;
            temp.path().to_path_buf()
        }
    };
    match build::build(folder, &game, &sdk, &clang, &work, platforms) {
        Ok(built) => {
            for (file, d) in &built.notes {
                println!("{}", lumalink_core::script::line(file, d));
            }
            for (file, line, function) in &built.kept {
                println!("{}", say!("LumaLink.Build.Kept", file = format!("source/{file}"), line = line, function = function));
            }
            for platform in &built.recorded {
                println!("{}", say!("LumaLink.Build.PlatformRecorded", platform = platform));
            }
            for (_, library, bytes) in &built.libraries {
                println!("{}", say!("LumaLink.Build.Built", path = library.display(), bytes = bytes, c = built.c_bytes, imports = built.imports.len()));
            }
            Ok(None)
        }
        Err(BuildError::Script(errors)) => {
            for (file, d) in &errors {
                println!("{}", lumalink_core::script::line(file, d));
            }
            println!("{}", say!("LumaLink.Build.ScriptErrors", count = errors.len()));
            Ok(Some(errors.len()))
        }
        Err(BuildError::Platforms { failed, built }) => {
            // Each platform that failed by name with what clang said; the ones built were recorded.
            for (platform, code, output) in &failed {
                eprint!("{output}");
                println!("{}", say!("LumaLink.Build.PlatformFailed", platform = platform, code = code));
            }
            let names: Vec<&str> = failed.iter().map(|(p, _, _)| *p).collect();
            bail!(say!("LumaLink.Build.PlatformsFailed", count = failed.len(), list = names.join(", "), built = built.len()))
        }
        Err(BuildError::UnknownPlatform(p)) => bail!(say!("LumaLink.UnknownPlatform", value = p, list = luma_validate::PLATFORMS.join(", "))),
        Err(BuildError::NoLinkerScript(path)) => bail!(say!("LumaLink.Build.NoLinkerScript", path = path.display())),
        Err(BuildError::Read(path, e)) => Err(anyhow::Error::new(e).context(say!("LumaLink.CannotRead", path = path.display()))),
        Err(BuildError::Write(path, e)) => Err(anyhow::Error::new(e).context(say!("LumaLink.Build.CannotWrite", path = path.display()))),
        Err(BuildError::NoId(path)) => bail!(say!("LumaLink.Build.NoId", path = path.display())),
        Err(BuildError::NoScripts(folder)) => bail!(say!("LumaLink.Build.NoScripts", folder = folder.display())),
        Err(BuildError::NoSdk(folder)) => bail!(say!("LumaLink.Build.NoSdk", folder = folder.display())),
        Err(BuildError::NoClang(path, e)) => Err(anyhow::Error::new(e).context(say!("LumaLink.Build.NoClang", path = path.display()))),
        Err(BuildError::ClangVersion { path, found }) => bail!(say!("LumaLink.Build.ClangVersion", expected = build::LLVM, path = path.display(), found = found)),
    }
}

/// Why the game's description does not read, in the chosen language.
fn game_error(e: lumalink_core::build::GameError) -> anyhow::Error {
    use lumalink_core::build::{FactoryError, GameError, TablesError};
    match e {
        GameError::Tables(TablesError::Read(path, e)) => anyhow::Error::new(e).context(say!("LumaLink.CannotRead", path = path.display())),
        GameError::Tables(TablesError::NotJson { path, line, column }) | GameError::Factory(FactoryError::NotJson { path, line, column }) => {
            anyhow::anyhow!(say!("LumaLink.SelfNotJson", path = path.display(), line = line, column = column))
        }
        GameError::Tables(TablesError::Shape(path)) => anyhow::anyhow!(say!("LumaLink.Build.TablesShape", path = path.display())),
        GameError::Factory(FactoryError::Shape(path)) => anyhow::anyhow!(say!("LumaLink.Build.FactoryShape", path = path.display())),
    }
}

/// The build `test` runs first: `lumalink build`'s own, with the game's names (`names`: LUMA_NAMES, else the bin
/// folder) and the SDK from LUMA_INCLUDE, for every platform the mod's luma.json lists in `code.platforms` (the game
/// refuses a mod with a listed platform's library missing, L501), and for this machine's platform when it lists none.
fn build_for_test(folder: &Path, names: &Path) -> Result<()> {
    let listed: Vec<String> = std::fs::read(folder.join("luma.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("code")?.get("platforms")?.as_array().cloned())
        .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let platforms = if listed.is_empty() { lumalink_core::build::HOST.into_iter().map(str::to_string).collect() } else { listed };
    match build_mod(folder, names, None, None, None, &platforms)? {
        None => Ok(()),
        Some(count) => bail!(say!("LumaLink.Test.NotBuilt", folder = folder.display(), why = say!("LumaLink.Build.ScriptErrors", count = count))),
    }
}

/// Runs the validator on a folder or a .luma with the inputs given, the installed game's in place of those not named.
fn validate(path: &Path, inputs: &Inputs) -> Result<Report> {
    jobs::validate(path, &inputs.resolved())
}

fn print_finding(f: &Finding) {
    println!("{}", jobs::finding_line(f));
}

fn print_report(report: &Report) {
    for line in jobs::report_lines(report) {
        println!("{line}");
    }
}

fn new_mod(folder: &Path, id: &str, name: &str, summary: &str, authors: &[String]) -> Result<ExitCode> {
    match jobs::new_mod(folder, id, name, summary, authors)? {
        NewMod::Made(folder) => {
            println!("{}", say!("LumaLink.Made", folder = folder.display()));
            Ok(ExitCode::SUCCESS)
        }
        NewMod::Refused(report) => {
            print_report(&report);
            Ok(ExitCode::from(1))
        }
    }
}

/// The mod at `from` as the mod folder `folder` in the one form, then checked as `check` checks it.
fn convert(from: &Path, folder: &Path, game: Option<PathBuf>, name: Option<String>, authors: Vec<String>, accept_losses: bool) -> Result<ExitCode> {
    use lumalink_core::convert::Given;
    let check = Inputs { names: None, platforms: vec![], game, no_installed: false, mods: None }.resolved();
    match jobs::convert(from, folder, &Given { name, authors, accept_losses }, &check)? {
        jobs::Convert::Losses(losses) => {
            for (path, why) in &losses.0 {
                println!("  {}", say!("LumaLink.Convert.LeftOut", path = path, why = why));
            }
            println!("{losses}");
            Ok(ExitCode::from(1))
        }
        jobs::Convert::Done { converted, report } => {
            for note in &converted.notes {
                let level = match note.level {
                    lumalink_core::plan::Level::Info => say!("LumaLink.Level.Note"),
                    _ => say!("LumaLink.Level.Error"),
                };
                println!("  {level}: {}", note.text);
            }
            for (path, why) in &converted.left_out {
                println!("  {}", say!("LumaLink.Convert.LeftOut", path = path, why = why));
            }
            println!("{}", say!("LumaLink.Convert.Done", folder = folder.display(), files = converted.files, textures = converted.textures));
            print_report(&report);
            Ok(if report.passed() { ExitCode::SUCCESS } else { ExitCode::from(1) })
        }
    }
}

fn pack(folder: &Path, output: Option<PathBuf>, inputs: &Inputs) -> Result<ExitCode> {
    let (report, outcome) = jobs::pack(folder, output, &inputs.resolved())?;
    print_pack(&report, outcome)
}

/// A pack's report and what came of it, with the exit status.
fn print_pack(report: &Report, outcome: PackOutcome) -> Result<ExitCode> {
    print_report(report);
    match outcome {
        PackOutcome::Packed { file, summary } => {
            println!("{}", say!("LumaLink.Packed", file = file.display(), files = summary.files, bytes = summary.bytes, unpacked = summary.unpacked));
            Ok(ExitCode::SUCCESS)
        }
        PackOutcome::Errors => {
            println!("{}", say!("LumaLink.NotPacked.Errors"));
            Ok(ExitCode::from(1))
        }
        PackOutcome::Unchecked => {
            // Pack refuses while any rule could not be checked.
            println!("{}", say!("LumaLink.NotPacked.Unchecked"));
            Ok(ExitCode::from(1))
        }
        PackOutcome::WrittenFails(again) => {
            print_report(&again);
            bail!(say!("LumaLink.WrittenFails"))
        }
    }
}

fn column_names(self_description: Option<&Path>) -> Result<ColumnNames> {
    match self_description {
        Some(path) => ColumnNames::from_self_description(&std::fs::read(path).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?, path),
        None => Ok(ColumnNames::none()),
    }
}

fn list(what: &str, game: Option<&Path>, self_description: Option<&Path>, filter: Option<&str>) -> Result<ExitCode> {
    // A kind of thing in the game, with no game named: the installed one's.
    let found;
    let game = match game {
        None if LIST_KINDS.contains(&what) && !Path::new(what).is_file() => {
            found = installed_pak()?;
            Some(found.as_path())
        }
        other => other,
    };
    let Some(game) = game else {
        if filter.is_some() {
            bail!(say!("LumaLink.FilterForGame"));
        }
        let (entries, damaged) = match jobs::list_luma(Path::new(what))? {
            LumaList::Entries { entries, damaged } => (entries, damaged),
            // Not readable as a .luma: the validator's own words say why.
            LumaList::Unreadable(report) => {
                print_report(&report);
                return Ok(ExitCode::from(1));
            }
        };
        let (stored, deflate) = (say!("LumaLink.Method.Stored"), say!("LumaLink.Method.Deflate"));
        let width = stored.chars().count().max(deflate.chars().count());
        for e in entries {
            println!("{:>12} {:>12} {:<width$} {}", e.unpacked, e.packed, if e.stored { &stored } else { &deflate }, e.name);
        }
        // Every entry's data was read and checked (L100 to L107); a damaged one is named and the list is refused.
        if !damaged.is_empty() {
            damaged.iter().for_each(print_finding);
            return Ok(ExitCode::from(1));
        }
        return Ok(ExitCode::SUCCESS);
    };
    let names = column_names(self_description)?;
    match what {
        "tables" => {
            for t in inspect::list_tables(game, filter, &names)? {
                let columns: Vec<String> = t.columns.iter().map(|(n, k)| format!("{n}:{k}")).collect();
                println!("{}", say!("LumaLink.List.Table", path = t.path, rows = t.rows, columns = columns.join(", ")));
            }
            if names.is_empty() {
                eprintln!("{}", say!("LumaLink.ColumnsByHash"));
            }
        }
        "models" => {
            for m in inspect::list_models(game, filter)? {
                println!("{}", say!("LumaLink.List.Model", path = m.path, count = m.animations.len()));
            }
        }
        "messages" => {
            for (id, text, _) in inspect::list_messages(game, filter.unwrap_or("UsEnglish"))? {
                println!("{id}\t{text}");
            }
        }
        "files" => {
            for (path, size) in inspect::list_files(game, filter.unwrap_or(""))? {
                println!("{size:>12} {path}");
            }
        }
        other => bail!(say!("LumaLink.ListKinds", value = other)),
    }
    Ok(ExitCode::SUCCESS)
}

/// What `list` lists in the game.
const LIST_KINDS: [&str; 4] = ["tables", "models", "messages", "files"];

fn extract(from: &str, names: &[String], output: &Path, game: Option<&Path>, self_description: Option<&Path>) -> Result<ExitCode> {
    // A path in the game, with no game named (not a .luma, nor a file here): the installed one's.
    let found;
    let game = match game {
        None if !from.to_ascii_lowercase().ends_with(".luma") && !Path::new(from).is_file() => {
            found = installed_pak()?;
            Some(found.as_path())
        }
        other => other,
    };
    if let Some(game) = game {
        if !names.is_empty() {
            bail!(say!("LumaLink.OneGamePath"));
        }
        let count = inspect::extract(game, from, output, &column_names(self_description)?)?;
        println!("{}", say!("LumaLink.Extracted", folder = output.display(), count = count));
        return Ok(ExitCode::SUCCESS);
    }
    match jobs::extract_luma(Path::new(from), names, output)? {
        Ok(written) => {
            println!("{}", say!("LumaLink.Extracted", folder = output.display(), count = written.len()));
            Ok(ExitCode::SUCCESS)
        }
        Err(container) => {
            container.iter().for_each(print_finding);
            println!("{}", say!("LumaLink.NotExtracted"));
            Ok(ExitCode::from(1))
        }
    }
}

fn format(paths: &[PathBuf], check: bool) -> Result<ExitCode> {
    let files = jobs::format(paths, check)?;
    let mut off = 0usize;
    for (file, changed) in &files {
        if *changed {
            off += 1;
            if check {
                println!("{}", say!("LumaLink.Format.Off", path = file.display()));
            } else {
                println!("{}", say!("LumaLink.Format.Done", path = file.display()));
            }
        }
    }
    println!("{}", say!("LumaLink.Format.Count", files = files.len(), changed = off));
    Ok(if check && off > 0 { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

/// `lumalink services`: what the SDK's services give scripts, a line a part, tab separated, for the services' pages
/// (they generate from the headers and read whether each function is a script's from here). A header whose face
/// cannot be read is said as `build` says it.
fn services(sdk: Option<PathBuf>, unreachable: bool) -> Result<ExitCode> {
    let Some(sdk) = sdk.or_else(|| std::env::var_os("LUMA_INCLUDE").map(PathBuf::from)) else {
        bail!(say!("LumaLink.Build.NoSdk", folder = "LUMA_INCLUDE"));
    };
    if !sdk.join("luma.h").is_file() {
        bail!(say!("LumaLink.Build.NoSdk", folder = sdk.display()));
    }
    let (services, unread) = lumalink_core::build::services_of_sdk(&sdk);
    for e in &unread {
        eprintln!("note: a service of the SDK is not callable from scripts: {e}");
    }
    for (kind, name, reach) in services.reach() {
        if !(unreachable && reach == "script") {
            println!("{kind}	{name}	{reach}");
        }
    }
    Ok(if unread.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

fn explain(code: &str) -> Result<ExitCode> {
    let (number, level, template) = jobs::explain(code)?;
    let kind = if level == Level::Error { say!("LumaLink.Explain.Error") } else { say!("LumaLink.Explain.Note") };
    println!("L{number} ({kind})\n{template}");
    Ok(ExitCode::SUCCESS)
}

fn verify(luma: &Path, source: &Path) -> Result<ExitCode> {
    match jobs::verify(luma, source)? {
        Ok(bytes) => {
            println!("{}", say!("LumaLink.Verify.Same", luma = luma.display(), folder = source.display(), bytes = bytes));
            Ok(ExitCode::SUCCESS)
        }
        Err(differences) => {
            for d in differences {
                match d {
                    Difference::OnlyInLuma(entry) => println!("{}", say!("LumaLink.Verify.OnlyIn", place = luma.display(), entry = entry)),
                    Difference::OnlyInFolder(entry) => println!("{}", say!("LumaLink.Verify.OnlyIn", place = source.display(), entry = entry)),
                    Difference::Differs(entry) => println!("{}", say!("LumaLink.Verify.Differs", entry = entry)),
                }
            }
            println!("{}", say!("LumaLink.Verify.Not", luma = luma.display(), folder = source.display()));
            Ok(ExitCode::from(1))
        }
    }
}
