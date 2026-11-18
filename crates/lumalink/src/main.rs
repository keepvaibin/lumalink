//! The command line: `new`, `check`, `pack`, `list`, `extract`, `explain`, `verify`, `run`, `convert`,
//! `add`, `lsp`, `format`, and `build` and `test`. Every
//! command that reads the game reads the installed one (`lumalink_core::installed`) unless `--game` (and `--names`) name another, or
//! `--no-installed` leaves it out.
//!
//! Exit status: 0 done (Notes may have been shown); 1 the mod has a problem or the command refused, each named;
//! 2 the command could not run (a file that cannot be read, a wrong argument).

use anyhow::{bail, Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use luma_validate::{Finding, Level, Report};
use lumalink_core::inspect::{self, ColumnNames};
use lumalink_core::jobs::{self, Difference, Install, LumaList, NewMod, PackOutcome};
use lumalink_core::{say, text, update};
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
    /// Your mod folder: the mods (folders and .luma files) a mod's needs are found in. Without it, they are looked for beside the mod, in the installed game's mods and in the mods that come with the game.
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

/// The test extension: a build folder's own, else the test kit's (the runner's HOST["extension"]).
const TEST_EXTENSION: &str = if cfg!(windows) { "nebula_test_extension.dll" } else { "libnebula_test_extension.so" };

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

/// The game's self-description folder: `names` when given, else the installed game's, else the refusal that names
/// the option to give.
fn names_or_installed(names: Option<PathBuf>) -> Result<PathBuf> {
    match names {
        Some(n) => Ok(n),
        None => installed().and_then(|f| f.self_description()).with_context(|| say!("LumaLink.NoNames")),
    }
}

/// The SDK folder (the game's luma.h): `sdk` when given, else LUMA_INCLUDE, else the `sdk` folder of the game's
/// self-description `names`, where Nebula installs the headers of the version that runs.
fn sdk_folder(sdk: Option<PathBuf>, names: Option<&Path>) -> Option<PathBuf> {
    jobs::sdk_folder(sdk, names)
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
        /// The mod's id: 3 to 64 of a-z, 0-9, '-' and '.', starting with a letter, with no '..' and not ending in '.'.
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
    /// List what is in a .luma, or a kind of thing in your imported game.
    List {
        /// A .luma file, or what to list from your imported game: tables, models, messages, sounds or files.
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
    /// Write files out of a .luma, or out of your imported game.
    Extract {
        /// A .luma file, or a path in your imported game.
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
        /// The game's self-description folder (function rows and types), for the rules on a mod's code. Default: the
        /// installed game's.
        #[arg(long, value_name = "FOLDER")]
        names: Option<PathBuf>,
        /// The folder with the game's luma.h. Default: LUMA_INCLUDE, then the self-description folder's sdk.
        #[arg(long, value_name = "FOLDER")]
        sdk: Option<PathBuf>,
        /// The clang to build with. Default: LUMALINK_CLANG, then the clang that comes with LumaLink, then LLVM's own
        /// folder, then the path.
        #[arg(long, value_name = "FILE")]
        clang: Option<PathBuf>,
        /// Keep the generated C in this folder. Default: a temporary folder, removed after.
        #[arg(long, value_name = "FOLDER")]
        work: Option<PathBuf>,
        /// A platform to build for (repeat for several). Default: every platform the game runs on, and every one the mod lists already.
        #[arg(long = "platform", value_name = "PLATFORM")]
        platforms: Vec<String>,
        /// The modder's mod folder: the mods a mod needs are taken from it and from the mods the game comes with.
        #[arg(long, value_name = "FOLDER")]
        mods: Option<PathBuf>,
        /// Leave out the installed game: --names names the self-description, and no needed mod comes from its mods.
        #[arg(long)]
        no_installed: bool,
    },
    /// Start the installed game with a mod: a folder is packed first (as `pack` packs it), then imported into the
    /// game's mods, then the game starts. A mod folder with code plays in dev mode (lumalink_core::dev): each change of
    /// its source is built and reloaded in the running game until the game is closed.
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
        /// A game build's folder with the harness's parts (the host extension and the game half). Default: LUMA_TEST_BIN,
        /// then the installed game's version folder.
        #[arg(long, value_name = "FOLDER")]
        bin: Option<PathBuf>,
        /// The harness's runner (luma_test.py). Default: LUMA_TEST_RUNNER, then the test kit's tools/lumacore/luma_test.py.
        /// With a --bin build folder that has its own test parts: the one in its luma/test, then tools/lumacore/luma_test.py
        /// above it.
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
        /// d3d12, d3d11, vulkan or metal. Default: each test's own list.
        #[arg(long, value_name = "RENDERER")]
        renderer: Option<String>,
        /// Your imported game (Nebula's game.pak), the one each run plays. Default: the game.pak of the install the bin
        /// folder belongs to, then the installed game's.
        #[arg(long, value_name = "FILE")]
        game: Option<PathBuf>,
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
        /// The folder with the game's luma.h. Default: LUMA_INCLUDE, then the installed game's sdk.
        #[arg(long, value_name = "FOLDER")]
        sdk: Option<PathBuf>,
        /// Only what scripts cannot reach.
        #[arg(long)]
        unreachable: bool,
        /// Each function's form as a script calls it instead: `form`, the service, the function, then the form, or
        /// `-` for one a script does not call by name; a `wait` line after it for a request a task may wait.
        #[arg(long, conflicts_with = "unreachable")]
        forms: bool,
    },
    /// Look for a newer LumaLink and install it (Windows); elsewhere, name its page.
    Update {
        /// Only say whether a newer version is out.
        #[arg(long)]
        check: bool,
        /// Whether LumaLink looks for updates by itself (the window when it opens, a command once a day): on or off.
        #[arg(long, value_name = "SETTING", value_parser = ["on", "off"])]
        check_at_start: Option<String>,
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
    // An installed LumaLink looks for a newer one once a day, behind the command, and tells of it after a later
    // command; never for the editor server (its output is the protocol), the update command, the help, or output that
    // is not a terminal (scripts, CI).
    use std::io::IsTerminal;
    let notices = !matches!(cli.command, Command::Lsp { .. } | Command::Update { .. } | Command::Help { .. })
        && std::io::stderr().is_terminal()
        && update::checks_by_itself();
    if notices {
        std::thread::spawn(|| update::refresh(update::NOTICE_INTERVAL));
    }
    let code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("lumalink: {e:#}");
            ExitCode::from(2)
        }
    };
    if notices {
        if let Some(release) = update::notice() {
            eprintln!("\n{}", say!("LumaLink.Update.Notice", version = release.version, current = update::VERSION));
        }
    }
    code
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
            // Code checked against one game and built against another fails in ways (L505) that do not say so.
            let resolved = inputs.resolved();
            if let Some(names) = resolved.names.as_deref().filter(|_| path.is_dir() && jobs::has_source(&path)) {
                if jobs::code_out_of_date(&path, names, sdk_folder(None, Some(names)).as_deref(), &jobs::NeedsFrom::from(&resolved)) {
                    eprintln!("{}", say!("LumaLink.Check.CodeNotBuiltHere", folder = path.display(), names = names.display()));
                }
            }
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
        Command::Build { folder, names, sdk, clang, work, platforms, mods, no_installed } => {
            let names = if no_installed { names.with_context(|| say!("LumaLink.NoNames"))? } else { names_or_installed(names)? };
            build(&folder, &names, sdk, clang, work, &platforms, &jobs::NeedsFrom { mods, no_installed })
        }
        Command::Run { path, inputs, args } => run_game(&path, &inputs, &args),
        Command::Test { folder, bin, runner, python, work, lock, renderer, game } => test(&folder, bin, runner, python, work, lock, renderer, game),
        Command::Format { paths, check } => format(&paths, check),
        Command::Add { .. } => add(cli.command),
        Command::Convert { from, folder, game, name, authors, accept_losses } => {
            // The installed game places the files and names the textures when no other is named.
            let game = game.or_else(|| installed().map(|f| f.pak));
            convert(&from, &folder, game, name, authors, accept_losses)
        }
        Command::Lsp { names, sdk, no_installed } => lsp(names, sdk, no_installed),
        Command::Services { sdk, unreachable, forms } => services(sdk, unreachable, forms),
        Command::Update { check, check_at_start } => update_lumalink(check, check_at_start.as_deref()),
        // parse prints the help and returns before run.
        Command::Help { .. } => Ok(ExitCode::SUCCESS),
    }
}

/// `update`: the setting when given; then the release list read now, and a newer version installed by its Setup (a
/// LumaLink Setup installed) or its page named.
fn update_lumalink(check: bool, setting: Option<&str>) -> Result<ExitCode> {
    if let Some(setting) = setting {
        let mut settings = update::Settings::load();
        settings.check_for_updates = setting == "on";
        settings.save()?;
        println!("{}", if settings.check_for_updates { say!("LumaLink.Update.AtStartOn") } else { say!("LumaLink.Update.AtStartOff") });
        if !check {
            return Ok(ExitCode::SUCCESS);
        }
    }
    println!("{}", say!("LumaLink.Update.Checking"));
    // Asked for: a skipped version is named too.
    let Some(release) = update::check_now()?.filter(|r| update::offered(r, None)) else {
        println!("{}", say!("LumaLink.Update.Newest", version = update::VERSION));
        return Ok(ExitCode::SUCCESS);
    };
    println!("{}", say!("LumaLink.Update.Available", version = release.version, current = update::VERSION));
    // Only a LumaLink Setup installed installs its update; any other copy (a build, a package manager's) is pointed to
    // the release's page.
    let installs = update::installed_copy();
    if check || !installs {
        println!("{}", if installs { say!("LumaLink.Update.HowTo") } else { say!("LumaLink.Update.DownloadFrom", page = release.page) });
        return Ok(ExitCode::SUCCESS);
    }
    {
        use std::io::{IsTerminal, Write};
        let terminal = std::io::stderr().is_terminal();
        let mut shown = u64::MAX;
        let mut progress = |got: u64, whole: u64| {
            let percent = (got * 100).checked_div(whole).unwrap_or(0);
            if terminal && whole > 0 && percent != shown {
                shown = percent;
                eprint!("\r{}", say!("LumaLink.Update.Downloading", version = release.version, percent = percent));
                let _ = std::io::stderr().flush();
            }
        };
        let setup = update::download(&release, &mut progress, &std::sync::atomic::AtomicBool::new(false));
        if terminal {
            eprintln!();
        }
        let setup = setup.map_err(|e| anyhow::anyhow!(e.to_string()))?;
        update::start_setup(&setup, false)?;
        println!("{}", say!("LumaLink.Update.Started", version = release.version));
    }
    Ok(ExitCode::SUCCESS)
}

/// The installed game started with the mod at `path` (a folder, packed first as `pack` packs it, or a
/// .luma), imported into the game's mods by the game's own mod tool (`jobs::install`).
fn run_game(path: &Path, inputs: &Inputs, game_args: &[OsString]) -> Result<ExitCode> {
    let found = installed().with_context(|| say!("LumaLink.NoInstalled"))?;
    let inputs = inputs.resolved();
    // A mod folder with code is built first, and played in dev mode (reload.md 5).
    let coded = path.is_dir() && jobs::has_source(path);
    let names = found.self_description();
    let build = |folder: &Path| -> Result<(Vec<jobs::BuildLine>, bool)> {
        let names = names.as_deref().with_context(|| say!("LumaLink.NoNames"))?;
        let built = jobs::build_mod(folder, names, jobs::sdk_folder(None, Some(names)), None, None, &[], &jobs::NeedsFrom::from(&inputs))?;
        let mut lines = built.lines;
        if let Some(jobs::BuildFailed::Platforms(why)) = &built.failed {
            lines.push(jobs::BuildLine { text: why.clone(), error: true, to_stderr: true });
        }
        Ok((lines, built.failed.is_none()))
    };
    let print_lines = |lines: &[jobs::BuildLine]| {
        for line in lines {
            if line.to_stderr { eprintln!("{}", line.text.trim_end()) } else { println!("{}", line.text.trim_end()) }
        }
    };
    if coded {
        let (lines, ok) = build(path)?;
        print_lines(&lines);
        if !ok {
            return Ok(ExitCode::from(1));
        }
    }
    let (id, mods) = match jobs::install(path, &inputs, &found, game_args)? {
        Install::Imported { luma, id, mods } => {
            println!("{}", say!("LumaLink.Run.Imported", file = luma.display()));
            (id, mods)
        }
        Install::NotImported { luma, why } => {
            println!("{}", say!("LumaLink.Run.NotImported", file = luma.display(), why = why));
            return Ok(ExitCode::from(1));
        }
        Install::NotPacked(report, outcome) => return print_pack(&report, outcome),
        Install::Refused(report) => {
            print_report(&report);
            return Ok(ExitCode::from(1));
        }
    };
    let pak = inputs.game.clone().unwrap_or_else(|| found.pak.clone());
    let dev = if coded { id.clone() } else { None };
    if coded && dev.is_none() {
        println!("{}", say!("LumaLink.Run.NoDevId"));
    }
    let mut args = game_args.to_vec();
    if let Some(id) = &dev {
        args.extend(["--mod-dev".into(), id.into()]);
    }
    let (program, game) = jobs::start_game(&found, &pak, &args)?;
    println!("{}", say!("LumaLink.Run.Started", program = program.display()));
    let Some(id) = dev else { return Ok(ExitCode::SUCCESS) };
    println!("{}", say!("LumaLink.Run.DevMode", id = id));
    let mut watch = lumalink_core::dev::Watch::new(path, &id, &mods, game);
    let mut install = |folder: &Path| -> Result<Option<String>> {
        Ok(match jobs::install(folder, &inputs, &found, game_args)? {
            Install::Imported { .. } => None,
            Install::NotImported { why, .. } => Some(why),
            Install::NotPacked(report, _) | Install::Refused(report) => {
                print_report(&report);
                Some(say!("LumaLink.Run.NotPacked"))
            }
        })
    };
    let mut build = build;
    while !watch.ended() {
        std::thread::sleep(std::time::Duration::from_millis(500));
        for event in watch.poll(&mut build, &mut install)? {
            dev_event(&event, &print_lines);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// One thing dev mode saw, said (`lumalink run`).
fn dev_event(event: &lumalink_core::dev::Event, print_lines: &dyn Fn(&[jobs::BuildLine])) {
    use lumalink_core::dev::Event;
    match event {
        Event::Building => println!("{}", say!("LumaLink.Run.Building")),
        Event::Built { lines, .. } => print_lines(lines),
        Event::NotInstalled(why) => eprintln!("{}", say!("LumaLink.Run.NotReloaded", why = why)),
        Event::Asked(n) => println!("{}", say!("LumaLink.Run.Asked", n = n)),
        Event::Reloaded(answer) if answer.ok => println!("{}", say!("LumaLink.Run.Reloaded")),
        Event::Reloaded(answer) => eprintln!("{}", say!("LumaLink.Run.NotReloaded", why = answer.why)),
        Event::ContentChanged => println!("{}", say!("LumaLink.Run.ContentLater")),
        Event::GameEnded => println!("{}", say!("LumaLink.Run.GameEnded")),
    }
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
    let sdk = sdk_folder(sdk, folder.as_deref());
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
#[allow(clippy::too_many_arguments)]
fn test(folder: &Path, bin: Option<PathBuf>, runner: Option<PathBuf>, python: Option<PathBuf>, work: Option<PathBuf>, lock: Option<PathBuf>, renderer: Option<String>, game: Option<PathBuf>) -> Result<ExitCode> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    // Without a bin folder: the installed game's version folder, with the installed game.pak for each run (the folder
    // has no pak.path). A bin folder that is an install's version folder (<install>/versions/<version>) plays that
    // install's content/game.pak; a build folder, its own pak.path.
    let (bin, pak) = match bin.or_else(|| env("LUMA_TEST_BIN")) {
        Some(bin) => {
            let install = bin.parent().filter(|v| v.file_name().is_some_and(|n| n == "versions")).and_then(Path::parent);
            let beside = install.map(|i| i.join("content").join("game.pak")).filter(|p| p.is_file());
            (bin, beside)
        }
        None => match installed().and_then(|i| i.version.clone().map(|v| (v, i.pak.clone()))) {
            Some((version, pak)) => (version, Some(pak)),
            None => bail!(say!("LumaLink.Test.NoBin")),
        },
    };
    let pak = game.or(pak);
    // The test parts: a build folder's own (a Nebula tree's build), else the test kit installed with LumaLink (the
    // game ships none): its runner, its test extension, and the game half built from its source.
    let kit = if bin.join(TEST_EXTENSION).is_file() { None } else { Some(jobs::test_kit().filter(|k| k.join(TEST_EXTENSION).is_file()).with_context(|| say!("LumaLink.Test.NoKit"))?) };
    let runner = runner.or_else(|| env("LUMA_TEST_RUNNER")).or_else(|| match &kit {
        Some(k) => Some(k.join("tools/lumacore/luma_test.py")).filter(|p| p.is_file()),
        None => jobs::find_runner(&bin),
    });
    let Some(runner) = runner else {
        bail!(say!("LumaLink.Test.NoRunner", path = kit.as_deref().unwrap_or(bin.as_path()).join("tools/lumacore/luma_test.py").display()));
    };
    let python = python.or_else(|| env("LUMALINK_PYTHON")).unwrap_or_else(|| PathBuf::from("python"));
    let work = work.unwrap_or_else(|| {
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "mod".into());
        folder.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).join(format!("{name}-tests"))
    });
    // A work folder inside the mod folder would be copied into the game with the mod, each run inside the last.
    let whole = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    if whole(&work).starts_with(whole(folder)) {
        bail!(say!("LumaLink.Test.WorkInside", work = work.display(), folder = folder.display()));
    }
    let lock = lock.or_else(|| env("LUMA_TEST_LOCK")).unwrap_or_else(|| work.join("run.lock"));
    // The mod's code is built by `lumalink build` (every platform) before its tests run.
    // The game's self-description folder is `luma` in its program folder, the mods it comes with in its `mods`.
    let names = env("LUMA_NAMES").unwrap_or_else(|| Some(bin.join("luma")).filter(|p| p.is_dir()).unwrap_or_else(|| bin.clone()));
    let build = |f: &Path| build_for_test(f, &names);
    let (extension, half) = match &kit {
        Some(k) => {
            let host: Vec<String> = lumalink_core::build::HOST.iter().map(|h| h.to_string()).collect();
            let half = |f: &Path| jobs::build_c_mod(f, sdk_folder(None, Some(&names)), &host);
            (Some(k.join(TEST_EXTENSION)), Some(jobs::build_game_half(k, &work, &half)?))
        }
        None => (None, None),
    };
    let setup = jobs::TestSetup { bin, pak, runner, python, work, lock, renderer, extension, half };
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
        for why in &run.unreadable {
            println!("{why}");
        }
        if !run.results.is_empty() {
            for renderer in run.missing() {
                println!("{}", say!("LumaLink.Test.NoResultOn", test = name, renderer = renderer));
            }
        }
    };
    let runs = jobs::test(folder, &setup, &build, &mut each)?;
    let passed = runs.iter().filter(|r| r.passed()).count();
    println!("{}", say!("LumaLink.Test.Summary", passed = passed, count = runs.len(), work = setup.work.display()));
    Ok(if passed == runs.len() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

/// `lumalink build`: the mod's LumaScript checked, written as C and built into `code/<platform>/mod.<ending>` for each
/// platform given; by default every one the game runs on and every one the mod lists already (build::default_platforms).
fn build(folder: &Path, names: &Path, sdk: Option<PathBuf>, clang: Option<PathBuf>, work: Option<PathBuf>, platforms: &[String], needs: &jobs::NeedsFrom) -> Result<ExitCode> {
    Ok(match build_mod(folder, names, sdk, clang, work, platforms, needs)? {
        None => ExitCode::SUCCESS,
        Some(_) => ExitCode::from(1),
    })
}

/// [`build`]'s work: None built, or the count of script errors (each said).
fn build_mod(folder: &Path, names: &Path, sdk: Option<PathBuf>, clang: Option<PathBuf>, work: Option<PathBuf>, platforms: &[String], needs: &jobs::NeedsFrom) -> Result<Option<usize>> {
    let built = jobs::build_mod(folder, names, sdk_folder(sdk, Some(names)), clang, work, platforms, needs)?;
    for line in &built.lines {
        if line.to_stderr {
            eprint!("{}", line.text);
        } else {
            println!("{}", line.text);
        }
    }
    match built.failed {
        Some(jobs::BuildFailed::Script(count)) => Ok(Some(count)),
        Some(jobs::BuildFailed::Platforms(why)) => bail!(why),
        None => Ok(None),
    }
}

/// The build `test` runs first: `lumalink build`'s own, with the game's names (`names`: LUMA_NAMES, else the bin
/// folder's `luma`, else the bin folder) and the SDK from LUMA_INCLUDE, for every platform the mod's luma.json lists in `code.platforms` (the game
/// refuses a mod with a listed platform's library missing, L501), and for this machine's platform when it lists none.
fn build_for_test(folder: &Path, names: &Path) -> Result<()> {
    let listed: Vec<String> = std::fs::read(folder.join("luma.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("code")?.get("platforms")?.as_array().cloned())
        .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let platforms = if listed.is_empty() { lumalink_core::build::HOST.into_iter().map(str::to_string).collect() } else { listed };
    // The game under test is the bin folder's, not the installed one: no needed mod comes from the installed game.
    match build_mod(folder, names, None, None, None, &platforms, &jobs::NeedsFrom { mods: None, no_installed: true })? {
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
    let inputs = inputs.resolved();
    // Code not built from what the folder holds now, with this LumaLink and against the self-description pack checks
    // with, is built first, as the window and `lumalink run` do: what is packed is the source.
    if jobs::has_source(folder) {
        let names = inputs.names.clone().with_context(|| say!("LumaLink.NoNames"))?;
        // Built with the needed mods the check finds them in.
        let needs = jobs::NeedsFrom::from(&inputs);
        if jobs::code_out_of_date(folder, &names, sdk_folder(None, Some(&names)).as_deref(), &needs) && build_mod(folder, &names, None, None, None, &[], &needs)?.is_some() {
            println!("{}", say!("LumaLink.NotPacked.Errors"));
            return Ok(ExitCode::from(1));
        }
    }
    let (report, outcome) = jobs::pack(folder, output, &inputs)?;
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

/// Table columns' names: from `self_description` when given, else the installed game's self-description, else none,
/// then the names of the files in `store` (columns no name covers are shown by their hash, said on stderr when the
/// game's names are missing).
fn column_names(self_description: Option<&Path>, store: &Path) -> Result<ColumnNames> {
    let installed_self = || installed().and_then(|f| f.self_description()).map(|n| n.join("smg_game.luma-self.json")).filter(|p| p.is_file());
    let mut names = match self_description.map(Path::to_path_buf).or_else(installed_self) {
        Some(path) => ColumnNames::from_self_description(&std::fs::read(&path).with_context(|| say!("LumaLink.CannotRead", path = path.display()))?, &path)?,
        None => ColumnNames::none(),
    };
    if names.is_empty() {
        eprintln!("{}", say!("LumaLink.ColumnsByHash"));
    }
    names.add_store_names(store)?;
    Ok(names)
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
    match what {
        "tables" => {
            for t in inspect::list_tables(game, filter, &column_names(self_description, game)?)? {
                let columns: Vec<String> = t.columns.iter().map(|(n, k)| format!("{n}:{k}")).collect();
                println!("{}", say!("LumaLink.List.Table", path = t.path, rows = t.rows, columns = columns.join(", ")));
            }
        }
        "models" => {
            for m in inspect::list_models(game, filter)? {
                inspect::model_lines(&m, filter.is_some()).iter().for_each(|line| println!("{line}"));
            }
        }
        "sounds" => {
            for name in inspect::list_sounds(game, filter)? {
                println!("{name}");
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
const LIST_KINDS: [&str; 5] = ["tables", "models", "messages", "sounds", "files"];

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
        let count = inspect::extract(game, from, output, &column_names(self_description, game)?)?;
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
fn services(sdk: Option<PathBuf>, unreachable: bool, forms: bool) -> Result<ExitCode> {
    let names = installed().and_then(|f| f.self_description());
    let Some(sdk) = sdk_folder(sdk, names.as_deref()) else {
        // No game is installed: the place it would have looked is LUMA_INCLUDE.
        let looked = names.as_deref().map_or_else(|| "LUMA_INCLUDE".to_string(), |n| n.join("sdk").display().to_string());
        bail!(say!("LumaLink.Build.NoSdk", folder = looked));
    };
    if !sdk.join("luma.h").is_file() {
        bail!(say!("LumaLink.Build.NoSdk", folder = sdk.display()));
    }
    let (services, unread) = lumalink_core::build::services_of_sdk(&sdk);
    for e in &unread {
        eprintln!("{}", say!("LumaLink.Build.ServiceUnread", why = e));
    }
    if forms {
        for (service, function, form, wait) in lumascript::check::service_forms(&services) {
            println!("form	{service}	{function}	{}", form.as_deref().unwrap_or("-"));
            if let Some(wait) = wait {
                println!("wait	{service}	{function}	{wait}");
            }
        }
    } else {
        for (kind, name, reach) in services.reach() {
            if !(unreachable && reach == "script") {
                println!("{kind}	{name}	{reach}");
            }
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
