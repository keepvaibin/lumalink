//! The syntax tree of one LumaScript file (language appendix G). It keeps every name, literal and place as written;
//! what they mean (types, the game's names, content) is the checker's, which reads this tree.

/// A place in the file: line and column, both from 1; the column counts characters (UTF-8 scalar values).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

/// A name of the mod's own, or a word after `.` or `::`, with its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub at: Pos,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub at: Pos,
    /// `pub` before it.
    pub public: bool,
    pub kind: ItemKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Use(Use),
    /// `option key: type`.
    Option { key: Ident, ty: Type },
    Binding(Box<Binding>),
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Class(ClassDecl),
    /// `task f(params) -> T { body }`: with a result, a call `f(args)` gives `pending<T>` (docs/modding/language/language.md, section 11.1a).
    Task { name: Ident, params: Vec<Param>, ret: Option<Type>, body: Block },
    Hook(Hook),
    On(On),
    States(States),
    /// `test ["game"] "name" { ... }`.
    Test { game: bool, name: StrLit, body: Block },
    /// `extern c fn name(params) [-> type]`.
    Extern { name: Ident, params: Vec<Param>, ret: Option<Type> },
    /// `export c fn name(params) [-> type] { ... }`.
    Export { name: Ident, params: Vec<Param>, ret: Option<Type>, body: Block },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Use {
    /// `use c "header.h"`.
    C(StrLit),
    /// `use [optional] a.b.c [.{x, y}] [as n]`, or `use [optional] ./a/b [as n]` (`local`).
    Module { optional: bool, local: bool, path: Vec<Ident>, names: Vec<Ident>, alias: Option<Ident> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub at: Pos,
    /// `var` (it may change) or `let`.
    pub var: bool,
    pub pattern: Pattern,
    pub ty: Option<Type>,
    pub value: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub at: Pos,
    /// `var`: the function may change it.
    pub var: bool,
    /// `self`: a member function's first parameter.
    pub is_self: bool,
    pub name: Option<Ident>,
    /// None in a hook's or an override's parameter list, which may leave the type to the game's.
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructDecl {
    /// `c struct`: laid out as C lays it out.
    pub c: bool,
    pub name: Ident,
    pub fields: Vec<Field>,
    pub fns: Vec<FnDecl>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Ident,
    pub ty: Type,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumDecl {
    pub name: Ident,
    pub cases: Vec<Case>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    pub name: Ident,
    /// `Case(a: T)` (false) or `Case { a: T }` (true), when it carries values.
    pub braces: bool,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClassDecl {
    pub name: Ident,
    /// The game class it derives from.
    pub base: GameName,
    /// `made by <row>`: the factory row that makes the base (classes  `maker`); none: the base's own name.
    pub maker: Option<GameName>,
    /// `replaces <name>`: a name the game's stages place whose objects are made as this class instead.
    pub replaces: Option<GameName>,
    pub members: Vec<ClassMember>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClassMember {
    Binding(Box<Binding>),
    Fn(FnDecl),
    Override(FnDecl),
    Hook(Hook),
}

/// A name of the game: `MR::getPlayerPos`, `Kuribo::~Kuribo`, or a backquoted one (`` `Game/Enemy/Kuribo.cpp#x` ``).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameName {
    pub at: Pos,
    /// As written, without backquotes; parts joined by `::`.
    pub text: String,
    pub quoted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookKind {
    Before,
    After,
    Wrap,
    Replace,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hook {
    pub at: Pos,
    pub kind: HookKind,
    /// `every override of`.
    pub every_override: bool,
    pub target: GameName,
    /// None: no parameter list written.
    pub params: Option<Vec<Param>>,
    pub ret: Option<HookRet>,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HookRet {
    /// An after's name for the function's result (`-> inside`).
    Name(Ident),
    Type(Type),
}

#[derive(Clone, Debug, PartialEq)]
pub struct On {
    pub at: Pos,
    pub event: OnEvent,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OnEvent {
    Start,
    Stop,
    Frame,
    Load,
    /// `on step` and `on step(now: time.Now)`: once per world step; its parameter, when written.
    Step(Option<Param>),
    Option(Ident),
    Stage(StrLit),
    Draw(Vec<Ident>),
    Layers(Type),
    /// `<module>.<event> [(names)]`; `ty` the type written on the first name (`on mods.stopped(e:
    /// mods.StoppedEvent)`).
    Event { path: Vec<Ident>, names: Option<Vec<Ident>>, ty: Option<Type> },
}

impl OnEvent {
    /// `on mods.stopped(gone)` of lumakit rule 4 (a mod that called this library is gone; `gone: u64`), as against
    /// svc.mods' STOPPED event, which needs its record type written.
    pub fn is_kit_stopped(&self) -> bool {
        let OnEvent::Event { path, ty, .. } = self else { return false };
        let named = matches!(path.as_slice(), [a, b] if a.name == "mods" && b.name == "stopped");
        let plain = ty.as_ref().is_none_or(|t| t.optional == 0 && matches!(&t.kind, TypeKind::Named { parts, args, .. } if args.is_empty() && parts.len() == 1 && parts[0].name == "u64"));
        named && plain
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct States {
    pub name: Ident,
    pub class: Ident,
    pub states: Vec<State>,
    pub start: Ident,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: Ident,
    pub parts: Vec<StatePart>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatePart {
    Enter(Block),
    Update(Block),
    Exit(Block),
    Stmt(Box<Stmt>),
}

// ---------------------------------------------------------------- types

#[derive(Clone, Debug, PartialEq)]
pub struct Type {
    pub at: Pos,
    pub kind: TypeKind,
    /// How many `?` follow it.
    pub optional: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    Tuple(Vec<Type>),
    Fn { params: Vec<Type>, ret: Option<Box<Type>> },
    Ref(Vec<Type>),
    /// `a.b.C<args>`, `Game::Type`, or a backquoted game name; `sep` holds the separators between the parts
    /// (`.` or `::`).
    Named { parts: Vec<Ident>, seps: Vec<Sep>, quoted: bool, args: Vec<Type> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sep {
    Dot,
    Colons,
}

// ---------------------------------------------------------------- statements

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub at: Pos,
    pub stmts: Vec<Stmt>,
    pub end: Pos,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub at: Pos,
    pub kind: StmtKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Binding(Box<Binding>),
    Fn(FnDecl),
    Return(Option<Expr>),
    Break,
    Continue,
    Stop,
    Skip(Option<Expr>),
    Go(Vec<Ident>),
    Defer(Box<Stmt>),
    Log { fact: Option<Expr>, value: Expr },
    Expect(Expr),
    /// `fail <text>`: the function ends with a script error of the mod's own words (S0921).
    Fail(Expr),
    Guard { conds: Vec<Cond>, otherwise: Block },
    While { cond: Expr, body: Block },
    For { pattern: Pattern, iter: Expr, body: Block },
    Expr(Expr),
    Assign { target: Expr, op: AssignOp, value: Expr },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

/// One condition of `if`, `guard`, `while let`: `let pattern = value` or a value.
#[derive(Clone, Debug, PartialEq)]
pub struct Cond {
    pub pattern: Option<Pattern>,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub at: Pos,
    pub kind: PatternKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Tuple(Vec<Pattern>),
    /// A number, string, colour, `true`, `false`, `none`, or a negative number.
    Literal(Box<Expr>),
    /// A name (it binds), or a case `E.Case`, `E.Case(a, b)`, `E.Case { a, b }`.
    Path { path: Vec<Ident>, args: Option<Vec<Pattern>>, fields: Option<Vec<Ident>> },
}

// ---------------------------------------------------------------- expressions

/// An expression's id: unique in its module, given by the parser; the checker's typed tree is keyed by it.
pub type ExprId = u32;

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub id: ExprId,
    pub at: Pos,
    pub kind: ExprKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// A number as written (`0x1F`, `1_000`, `2.0f64`); its value and type are the checker's.
    Number(String),
    /// `#FFD700` or `#FFD700CC`, as written.
    Color(String),
    Str(StrLit),
    True,
    False,
    None,
    SelfValue,
    /// An after's or a replace's `result`.
    Result,
    /// `proceed()`.
    Proceed,
    /// `ref(args)`.
    Ref(Vec<Arg>),
    If { arms: Vec<(Vec<Cond>, Block)>, otherwise: Option<Block> },
    Match { subject: Box<Expr>, arms: Vec<Arm> },
    Closure { params: Vec<Param>, ret: Option<Box<Type>>, body: Block },
    /// `(a)`: kept, so `(Point { .. })` in a condition stays visible as written.
    Paren(Box<Expr>),
    Tuple(Vec<Expr>),
    List(Vec<Expr>),
    /// `{}`, `{k: v, ...}` (each with its value) or `{a, b}` (none).
    Braces(Vec<(Expr, Option<Expr>)>),
    /// `model "path"`, `text "id"(args)` and the other content words.
    Content { word: String, path: StrLit, args: Option<Vec<Arg>> },
    Game(GameName),
    /// A name, with type arguments written right before a call (`low_bits<u16>`).
    Name { name: Ident, targs: Vec<Type> },
    StructLit { path: Vec<Ident>, fields: Vec<(Ident, Option<Expr>)> },
    Call { callee: Box<Expr>, args: Vec<Arg> },
    Index { base: Box<Expr>, index: Box<Expr> },
    /// `.name` or `?.name` (`optional`), with type arguments right before a call.
    Member { base: Box<Expr>, name: Ident, optional: bool, targs: Vec<Type> },
    /// Postfix `!`.
    Unwrap(Box<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Binary { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    /// `wait n [frames|seconds]`.
    Wait { amount: Box<Expr>, unit: Option<WaitUnit> },
    WaitUntil(Box<Expr>),
    /// `start task(...)`.
    Start(Box<Expr>),
    /// `value as ty`; `narrow`: `value as? ty`, a game object's view narrowed to a class, `ty?` (as_class).
    As { value: Box<Expr>, ty: Box<Type>, narrow: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitUnit {
    Frames,
    Seconds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Coalesce,
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Range,
    RangeIncl,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Shr,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub at: Pos,
    pub name: Option<Ident>,
    /// `var x`: the callee may change it.
    pub var: bool,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    /// None: `else =>`.
    pub pattern: Option<Pattern>,
    pub body: ArmBody,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArmBody {
    Block(Block),
    Stmt(Box<Stmt>),
}

/// A string literal: its pieces, text with escapes still as written (the checker reads them), and expressions.
#[derive(Clone, Debug, PartialEq)]
pub struct StrLit {
    pub at: Pos,
    pub prefix: StrPrefix,
    pub parts: Vec<StrPart>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrPrefix {
    None,
    Raw,
    Bytes,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Text(String),
    Expr(Expr),
}

impl StrLit {
    /// Whether it holds an interpolated expression.
    pub fn interpolates(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, StrPart::Expr(_)))
    }
}
