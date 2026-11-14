//! LumaScript's compiler: the LumaScript language, read here and compiled to
//! C, which LumaLink builds to machine code for each platform. Nothing of it runs in the game.

pub mod ast;
pub mod check;
pub mod ctype;
pub mod diag;
pub mod emit;
pub mod fmt;
pub mod game;
pub mod lex;
pub mod lumi;
pub mod parse;
pub mod svc;

pub use diag::{Arg, Diagnostic};

/// How deep a program may nest: brackets, blocks, operators (`a + b + c` is two deep), `not` and `-`, types, patterns
/// and interpolations. Past it the compiler says S0022 at the place and reads on. Every tree it builds is held to about
/// this depth, so each later pass may walk one by recursion on [`COMPILER_STACK`].
pub const MAX_DEPTH: u32 = 512;

/// The stack of the thread the compiler works on. A process's main thread may have 1 MB (Windows), too little for
/// [`MAX_DEPTH`]. At the limit the costliest form (strings in interpolations) needs 2.3 MB in a release build and
/// 15.1 MB in a debug build (2026-11-11, `cargo run --example depth_stack`), so this holds it 27 and 4 times over.
/// Only the pages used are committed.
pub const COMPILER_STACK: usize = 64 << 20;

/// Runs `f` on a thread of [`COMPILER_STACK`] bytes and gives its result. Every pass that reads or walks a program
/// runs inside it (CLI and GUI alike); a panic in it goes on in the caller. No thread to be had is S0024, never a
/// crash.
pub fn on_compiler_stack<T: Send>(f: impl FnOnce() -> T + Send) -> Result<T, Diagnostic> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new().name("lumascript".into()).stack_size(COMPILER_STACK).spawn_scoped(scope, f);
        match thread {
            Ok(thread) => match thread.join() {
                Ok(out) => Ok(out),
                Err(panic) => std::panic::resume_unwind(panic),
            },
            Err(why) => Err(Diagnostic::new(diag::code::NO_COMPILER_THREAD, ast::Pos { line: 1, col: 1 })
                .with("found", Arg::Text(why.to_string()))),
        }
    })
}

fn read<T>(src: &str, f: impl FnOnce(&mut parse::Parser) -> T) -> (T, Vec<Diagnostic>) {
    let mut diags = Vec::new();
    let toks = lex::lex(src, &mut diags);
    let out = f(&mut parse::Parser::new(toks, &mut diags));
    // In the file's order; of two at one place only the first, which the second follows from (an unclosed string,
    // then the parser finding it).
    diags.sort_by_key(|d| (d.at.line, d.at.col));
    diags.dedup_by_key(|d| (d.at.line, d.at.col));
    (out, diags)
}

/// Reads a module file: its items, and every error found in it, in the order of the file. It reads on the compiler's
/// thread; the tree it gives back is walked there too ([`on_compiler_stack`]).
pub fn parse_module(src: &str) -> (ast::Module, Vec<Diagnostic>) {
    on_compiler_stack(|| read(src, |p| p.module())).unwrap_or_else(|d| (ast::Module { items: Vec::new() }, vec![d]))
}

/// Reads one module of a library's interface file (`<mod id>.lumi`, docs/modding/language/compiled-form.md): as
/// [`parse_module`], but a function is its head alone (it is given an empty body).
pub fn parse_interface(src: &str) -> (ast::Module, Vec<Diagnostic>) {
    on_compiler_stack(|| {
        read(src, |p| {
            p.interface = true;
            p.module()
        })
    })
    .unwrap_or_else(|d| (ast::Module { items: Vec::new() }, vec![d]))
}

/// Reads statements without the items around them (a test's or a sample's body), on the compiler's thread.
pub fn parse_statements(src: &str) -> (Vec<ast::Stmt>, Vec<Diagnostic>) {
    on_compiler_stack(|| read(src, |p| p.statements())).unwrap_or_else(|d| (Vec::new(), vec![d]))
}
