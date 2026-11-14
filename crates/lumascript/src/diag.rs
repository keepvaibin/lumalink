//! What the compiler says (docs/modding/language/language.md, section 9.1): a code `S<number>`, the place, and parameters by name. The words are
//! LumaLink's (`Script.S<code>` rows and `Script.Want.*` / `Script.Found.*` phrases, in the game's 14 languages); this
//! crate holds no text of its own, as the validator holds none.

use crate::ast::Pos;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    /// Shown as it is (a token, a name).
    Text(String),
    /// A phrase id, itself a translated row.
    Phrase(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: u16,
    pub at: Pos,
    pub args: Vec<(&'static str, Arg)>,
}

impl Diagnostic {
    pub fn new(code: u16, at: Pos) -> Self {
        Diagnostic { code, at, args: Vec::new() }
    }

    pub fn with(mut self, name: &'static str, value: Arg) -> Self {
        self.args.push((name, value));
        self
    }

    /// The row id of its message.
    pub fn row(&self) -> String {
        format!("Script.S{:04}", self.code)
    }

    /// Nested past [`crate::MAX_DEPTH`], at `at`.
    pub fn too_deep(at: Pos) -> Self {
        Diagnostic::new(code::TOO_DEEP, at).with("limit", Arg::Text(crate::MAX_DEPTH.to_string()))
    }
}

/// The build-time codes (S0001 to S0899; S0901 on are lumart's, at run time).
pub mod code {
    /// `{need} was built with another LumaLink (runtime layout {found}, this one {layout})`: its interface file's
    /// layout is not this compiler's (lumi::RUNTIME_LAYOUT), so no function of it would bind.
    pub const NEED_OTHER_LAYOUT: u16 = 160;
    /// `found {found}; expected {expected}`.
    pub const UNEXPECTED: u16 = 1;
    pub const NO_CLOSING_BACKQUOTE: u16 = 2;
    pub const NO_CLOSING_QUOTE: u16 = 3;
    pub const NO_INTERPOLATION_END: u16 = 4;
    /// `{found}` is no token of the language.
    pub const NOT_A_TOKEN: u16 = 5;
    pub const NO_COMMENT_END: u16 = 6;
    pub const UNDERSCORE_NAME: u16 = 10;
    pub const PREFIX_BANG: u16 = 11;
    /// `{found}` is written `{expected}` (`&&`, `||`).
    pub const DOUBLED_OPERATOR: u16 = 12;
    pub const CHAINED_COMPARISON: u16 = 13;
    pub const STRUCT_IN_CONDITION: u16 = 14;
    /// `{found}` takes a plain string.
    pub const CONTENT_INTERPOLATES: u16 = 15;
    pub const BARE_ARM_NAME: u16 = 16;
    /// `{found}` belongs at the top of a module.
    pub const ITEM_IN_BLOCK: u16 = 17;
    pub const POSITIONAL_AFTER_NAMED: u16 = 18;
    pub const SELF_NOT_FIRST: u16 = 19;
    /// A `states` has exactly one `start` line: none, or a second one.
    pub const ONE_START_STATE: u16 = 20;
    pub const UNDERSCORE_PATTERN: u16 = 21;
    /// Nested more than `{limit}` levels deep (`crate::MAX_DEPTH`): brackets, blocks, operators, types, patterns.
    pub const TOO_DEEP: u16 = 22;
    /// `{found}`: a name is written with the letters A to Z, the digits and `_` (docs/modding/language/language.md, section 2).
    pub const NON_ASCII_NAME: u16 = 23;
    /// The compiler could not start the thread it reads on (`{found}`, the system's reason).
    pub const NO_COMPILER_THREAD: u16 = 24;
    /// A call's type arguments end a line and its `(` starts the next: they go on one line (G.4).
    pub const TYPE_ARGS_LINE_END: u16 = 25;

    // ---- the checker (4.2 part b: items, uses, declarations)
    /// `{name}` is declared twice in one scope; the first is at line `{line}`.
    pub const NAME_TWICE: u16 = 26;
    /// `use {found}`: the mod or library has no such module.
    pub const NO_MODULE: u16 = 27;
    /// `use {found}`: the library mod `{need}` is not in luma.json's `needs`.
    pub const NOT_NEEDED: u16 = 28;
    /// Modules that use each other in a cycle: `{chain}`.
    pub const MODULE_CYCLE: u16 = 29;
    /// `{name}` is no name of the module `{module}`.
    pub const NO_NAME_IN_MODULE: u16 = 30;
    /// `{name}` of the module `{module}` is not `pub`.
    pub const NAME_NOT_PUB: u16 = 31;
    /// `use c {found}`: the mod has no such C header.
    pub const NO_C_HEADER: u16 = 32;
    /// A note: `{found}` uses `{what}`, which this game's self-description does not give yet; not checked.
    pub const NOT_DESCRIBED: u16 = 33;
    /// `{found}` is no type.
    pub const UNKNOWN_TYPE: u16 = 34;
    /// `{found}` is no type; the nearest are `{near}`.
    pub const UNKNOWN_TYPE_NEAR: u16 = 35;
    /// `{found}` takes `{expected}` type arguments.
    pub const TYPE_ARG_COUNT: u16 = 36;
    /// The game's `{found}` cannot be used from LumaScript: `{why}`.
    pub const GAME_TYPE_NOT_CROSSING: u16 = 37;
    /// A map's key is an integer, `text`, `bool` or an enum without data; found `{found}`.
    pub const MAP_KEY: u16 = 38;
    /// A class derives from a game class; `{found}` is none.
    pub const BASE_NOT_GAME_CLASS: u16 = 39;
    /// `option {found}`: luma.json has no option with this key.
    pub const OPTION_NOT_IN_LUMA_JSON: u16 = 40;
    /// `option {found}`: luma.json's option gives the type `{expected}`.
    pub const OPTION_TYPE: u16 = 41;
    /// `states ... for {found}`: no class of the mod has this name.
    pub const STATES_FOR: u16 = 42;
    /// `start {found}`: no state of this machine has this name.
    pub const START_NOT_STATE: u16 = 43;
    /// `ref<{found}>`: a `ref` holds a game class or a class of the mod.
    pub const REF_OF: u16 = 44;
    /// Structs and enums that hold each other by value, so have no size: `{chain}`.
    pub const VALUE_CYCLE: u16 = 45;
    /// A note: `{found}` names the language's or the mod's type, which hides the game's of that name; the game's is
    /// written in backquotes.
    pub const NAME_HIDES_GAME: u16 = 46;

    // ---- the checker (4.2 part c: bodies)
    /// `{found}` names nothing here: no local, no name of the module or its uses, no standard name, nothing of the game.
    pub const UNKNOWN_NAME: u16 = 47;
    /// As UNKNOWN_NAME, with the nearest names.
    pub const UNKNOWN_NAME_NEAR: u16 = 48;
    /// A value of `{found}` where `{expected}` goes.
    pub const TYPE_MISMATCH: u16 = 49;
    /// The number `{found}` does not fit `{expected}`.
    pub const LITERAL_FIT: u16 = 50;
    /// `{found}` has no field, function or case `{name}`.
    pub const NO_MEMBER: u16 = 51;
    /// A value of `{found}` is called.
    pub const NOT_CALLABLE: u16 = 52;
    /// `{name}` takes `{expected}` arguments; `{found}` are given.
    pub const ARG_COUNT: u16 = 53;
    /// `{callee}` has no parameter `{name}`.
    pub const ARG_UNKNOWN: u16 = 54;
    /// The argument `{name}` is given twice.
    pub const ARG_TWICE: u16 = 55;
    /// `{callee}` needs the argument `{name}`.
    pub const ARG_MISSING: u16 = 56;
    /// A game function takes its arguments by position, not `{name}:`.
    pub const ARG_NAMED_GAME: u16 = 57;
    /// No signature of the game's `{found}` takes these arguments; its signatures: `{signatures}`.
    pub const GAME_NO_SIGNATURE: u16 = 58;
    /// More than one signature of `{found}` takes these arguments (`{signatures}`): name one in backquotes (7.1).
    pub const GAME_AMBIGUOUS: u16 = 59;
    /// The game's `{found}` cannot be called from LumaScript: its parameter `{param}` is `{why}` (7.2).
    pub const GAME_PARAM_NOT_CROSSING: u16 = 60;
    /// The game's `{found}` cannot be called from LumaScript: its result is `{why}` (7.2).
    pub const GAME_RESULT_NOT_CROSSING: u16 = 61;
    /// `{found}` writes its parameter `{param}`: the argument is written `var x` (5.1, 7.2).
    pub const VAR_ARG_NEEDED: u16 = 62;
    /// `{found}` does not write its parameter `{param}`: the argument has no `var`.
    pub const VAR_ARG_NOT_TAKEN: u16 = 63;
    /// This cannot be written: `{why}` (a phrase).
    pub const NOT_A_PLACE: u16 = 64;
    /// `{op}` does not take `{left}` and `{right}` (3).
    pub const OPERATOR: u16 = 65;
    /// `{op}` does not take `{found}`.
    pub const OPERATOR_ONE: u16 = 66;
    /// A `{found}?` where a `{found}` goes: test it first or give a value.
    pub const MAY_BE_NONE: u16 = 67;
    /// A match covers neither `{cases}` nor has `else` (4).
    pub const MATCH_MISSING: u16 = 68;
    /// `as` converts numbers (and the game's enums to and from integers), not `{found}` to `{expected}`.
    pub const AS_NUMBERS: u16 = 69;
    /// `{found}` (break, continue) outside a loop.
    pub const OUTSIDE_LOOP: u16 = 70;
    /// `return` without a value where the function gives `{expected}`.
    pub const RETURN_NEEDS_VALUE: u16 = 71;
    /// `return` with a value where the function, task or handler gives none.
    pub const RETURN_NO_VALUE: u16 = 72;
    /// `==` and `!=` do not compare `{found}` (3; views and service handles: `same`).
    pub const NOT_EQUATABLE: u16 = 73;
    /// `<` `<=` `>` `>=` compare numbers and text, not `{found}` (3).
    pub const NOT_ORDERED: u16 = 74;
    /// A literal of `{found}` without its field `{name}`, which has no default.
    pub const FIELD_MISSING: u16 = 75;
    /// `{found} { ... }` where `{found}` is no struct (an enum's case is made with `( )`, D4.6).
    pub const NOT_A_STRUCT: u16 = 76;
    /// `[ ]` on a `{found}`.
    pub const NOT_INDEXABLE: u16 = 77;
    /// `{found}` (`[]`, `{}`, `none`) where nothing says its type.
    pub const TYPE_UNKNOWN_HERE: u16 = 78;
    /// `self` outside a struct's or class's function.
    pub const SELF_OUTSIDE: u16 = 79;
    /// A `{found}` in text (interpolation, `log`).
    pub const NOT_SHOWN: u16 = 80;
    /// A part that gives no value where a value goes.
    pub const NO_VALUE: u16 = 81;
    /// An `if` that gives a value has no `else` (4).
    pub const IF_NEEDS_ELSE: u16 = 82;
    /// `{found}` is a `{what}` (a phrase), not a value.
    pub const NOT_A_VALUE: u16 = 83;
    /// The task `{found}` called: it runs with `start` (11.1).
    pub const TASK_CALL: u16 = 84;
    /// `start` on `{found}`, no task (11.1).
    pub const START_NOT_TASK: u16 = 85;
    /// `!` on a `{found}`, which is never none.
    pub const UNWRAP_NOT_OPTIONAL: u16 = 86;
    /// A `let` or `for` pattern that may not match (an enum case, a literal).
    pub const REFUTABLE_PATTERN: u16 = 87;
    /// `ref` of `{found}`, which the game does not destroy through NameObj.
    pub const REF_NOT_DESTROYED: u16 = 88;
    /// `{found}`'s value needs its own type: write the type.
    pub const TYPE_NEEDS_WRITING: u16 = 89;
    /// No signature of the game's `{found}` can be called from LumaScript (`{signatures}`; 7.2).
    pub const GAME_NONE_CROSSING: u16 = 90;
    /// `{found}` changes its object, read-only here (a const view).
    pub const CONST_VIEW_CALL: u16 = 91;
    /// A `{ }` literal with entries both `key: value` and alone.
    pub const MIXED_BRACES: u16 = 92;
    /// `{found}` is no escape of a string (docs/modding/language/language.md).
    pub const BAD_ESCAPE: u16 = 93;
    /// `{found}` is not data json reads or writes (a view, a handle, a function...).
    pub const NOT_JSON_DATA: u16 = 94;
    /// `for` over a `{found}`: a list, set, map, range or a game array goes there.
    pub const NOT_ITERABLE: u16 = 95;
    /// A match on values that cannot all be listed (numbers, text) has no `else`.
    pub const MATCH_NEEDS_ELSE: u16 = 96;
    /// A note: a `pending` result is dropped without `wait`.
    pub const PENDING_DROPPED: u16 = 97;
    /// A game function called on the script's own copy of `{found}`: it takes the game's object, a view (revision 3).
    pub const COPY_NOT_VIEW: u16 = 98;
    /// `wait` takes N frames, N seconds, `until` a condition, a task or a pending result, not a `{found}`.
    pub const WAIT_WHAT: u16 = 99;
    /// The game's `{found}` is a member function: it is called on an object (`x.name(...)`), or on `self` inside a class or hook of its record.
    pub const METHOD_NEEDS_OBJECT: u16 = 100;
    /// Part (d): a function with a result can reach its end without `return` (`{expected}` its result type).
    pub const MISSING_RETURN: u16 = 101;
    /// Part (d): a `guard`'s `else` can end without leaving the function, task or loop.
    pub const GUARD_STAYS: u16 = 102;
    /// Part (d): a view of a game object (`{found}`) would be kept where it outlives its statement (`{why}`, docs/modding/language/language.md, section 6.1).
    pub const VIEW_KEPT: u16 = 103;
    /// Part (d): a view (`{found}`) is used after a `wait` (on line `{line}`) it was made before (11.1).
    pub const VIEW_ACROSS_WAIT: u16 = 104;
    /// Part (d): `wait` where it cannot be (`{why}`).
    pub const WAIT_HERE: u16 = 105;
    /// Part (d): `{found}` (a statement that leaves, or `wait`) inside `defer`.
    pub const DEFER_LEAVES: u16 = 106;
    /// a second `on {what}` in the mod; the first is at `{first}` (its module's file and line: the mod's
    /// modules share one of each).
    pub const ON_TWICE: u16 = 109;
    /// D12.6: `on {event}`, whose events go to the function given to `{call}`.
    pub const SVC_PER_REQUEST: u16 = 107;
    /// D12.6: `{name}` is set by `on {what}`, not called or named.
    pub const SVC_SET_BY: u16 = 108;
    /// D12.6: `{name}`'s `{param}` is `{found}`; it takes `{low}` to `{high}`.
    pub const SVC_RANGE: u16 = 111;
    /// `wait {found}` with a literal below 0.
    pub const WAIT_NEGATIVE: u16 = 110;
    /// Part (e): the hook names `{found}` parameters; `{name}` takes `{expected}`.
    pub const HOOK_PARAMS: u16 = 112;
    /// Part (e): `{found}` is a constructor, a destructor or a tell row: `before` and `after` only (section 8).
    pub const HOOK_TELL: u16 = 113;
    /// Part (e): `every override of {found}`, which is not virtual.
    pub const HOOK_NOT_VIRTUAL: u16 = 114;
    /// Part (e): a hook in class `{class}` on `{found}`, which is no member function of the game class it derives from.
    pub const HOOK_NOT_OF_CLASS: u16 = 115;
    /// Part (e): `{found}` returns nothing: there is no result to name, give a type to, set or skip with.
    pub const HOOK_NO_RESULT: u16 = 116;
    /// Part (e): `skip` outside a `before` hook's own body.
    pub const SKIP_OUTSIDE: u16 = 117;
    /// Part (e): `skip` with a value, but `{found}` returns nothing.
    pub const SKIP_VALUE: u16 = 118;
    /// Part (e): `skip` without the result `{found}` gives (a `{expected}`).
    pub const SKIP_NEEDS_VALUE: u16 = 119;
    /// Part (e): `result` where there is none (`{why}`).
    pub const RESULT_HERE: u16 = 120;
    /// Part (e): `proceed()` where it cannot be (`{why}`).
    pub const PROCEED_HERE: u16 = 121;
    /// Part (e): `proceed()` may run a second time on one path (the first at line `{line}`).
    pub const PROCEED_TWICE: u16 = 122;
    /// Part (e): a path of the hook ends without giving `{found}`'s result (a `{expected}`): no `proceed()` and no `result = ...`.
    pub const RESULT_NOT_GIVEN: u16 = 123;
    /// Part (e): `result` read before `proceed()` or `result = ...` on some path to it.
    pub const RESULT_NOT_YET: u16 = 124;
    /// Part (e): `stop` outside a task, a state machine or a handler that may wait (11.1).
    pub const STOP_HERE: u16 = 125;
    /// Part (e): `{what}` (`go`, `is_state`) outside a state machine and the functions of its class (11.2).
    pub const GO_HERE: u16 = 126;
    /// Part (e): `{found}` is no state of the machine `{machine}`.
    pub const GO_UNKNOWN: u16 = 127;
    /// Part (e): `self` where the object is not there (`{why}`).
    pub const SELF_NOT_BUILT: u16 = 128;
    /// Part (e): a second `{found}` in one state; the first is at line `{line}`.
    pub const STATE_PART_TWICE: u16 = 129;
    /// Part (e): `{what}` from a function of the class names the state `{found}` without its machine (`{machine}`).
    pub const GO_NAME_MACHINE: u16 = 130;
    /// `is_state` takes one state, written `S` or `M.S`.
    pub const STATE_ARG: u16 = 131;
    /// `{what}` (`keep`, `kept`) where it cannot be (`{why}`).
    pub const KEEP_HERE: u16 = 132;
    /// docs/modding/language/compiled-form.md, section 7.x: a list literal of `{count}` elements for `{found}`'s parameter `{param}`, an array of
    /// exactly `{expected}`.
    pub const FIXED_COUNT: u16 = 133;
    /// D12.3: the service's `{found}` cannot be called from LumaScript; its parameter `{param}`: {why}.
    pub const SVC_PARAM: u16 = 134;
    /// D12.5, D12.1: the service's `{found}` cannot be called from LumaScript: {why} (a take, or a member not
    /// in the registry's form).
    pub const SVC_NOT_CALLABLE: u16 = 135;
    /// Language 10: a content literal `{kind} "{found}"` names nothing the mod, the mods it needs or the game has.
    pub const CONTENT_MISSING: u16 = 136;
    /// `text "{found}"`'s rows have the placeholder `{name}`, which the call does not give.
    pub const TEXT_ARG_MISSING: u16 = 137;
    /// `text "{found}"`'s rows have no placeholder `{name}`.
    pub const TEXT_ARG_UNKNOWN: u16 = 138;
    /// `text "{found}"` takes its placeholders by name.
    pub const TEXT_ARG_UNNAMED: u16 = 139;
    /// Language 5.5: `override fn {found}` names no virtual of the game class `{class}` or its bases.
    pub const OVERRIDE_NOT_VIRTUAL: u16 = 140;
    /// Language 5.5 (docs/modding/language/classes.md): `{found}` cannot be derived from: no vtable, or NameObj not at its start.
    pub const NOT_DERIVABLE: u16 = 141;
    /// Language 5.5: `override fn {name}` names {found} parameters; the virtual takes {expected}.
    pub const OVERRIDE_PARAMS: u16 = 142;
    /// Language 5.5: `base.{found}` in `override fn {name}`: `base.` calls the base's own `{name}` only.
    pub const BASE_CALL: u16 = 143;
    /// Language 5.5: `made by` or `replaces` names `{found}`, which is no row of the game's object factory
    /// (docs/modding/language/game-description.md, section 9).
    pub const CLASS_NAME_UNKNOWN: u16 = 144;
    /// Language 5.5: class `{found}` replaces itself.
    pub const REPLACES_ITSELF: u16 = 145;
    /// Language 5.5: class `{found}` has no `made by`, and its base `{base}` is no row of the game's object factory.
    pub const BASE_NOT_MADE: u16 = 146;
    /// Language 5.5: `override fn {found}`: `{class}` has several virtuals `{found}` and the parameter count does not
    /// choose one of `{signatures}`.
    pub const OVERRIDE_AMBIGUOUS: u16 = 147;
    /// Language 5.5: `replaces` written before `made by`.
    pub const MADE_BY_AFTER_REPLACES: u16 = 148;
    /// Language 5.5 (docs/modding/language/classes.md): the factory row `{found}` makes `{made}`, not exactly the base `{base}`; the
    /// kernel would keep the game's object.
    pub const MAKER_MAKES_OTHER: u16 = 149;
    /// docs/modding/language/numbers.md: `{found}` names no table of the game's data.
    pub const TABLE_UNKNOWN: u16 = 150;
    /// docs/modding/language/numbers.md: set_table or table cannot reach or write `{found}`: {why}.
    pub const TABLE_PLACE: u16 = 151;
    /// docs/modding/language/numbers.md: index {found} is outside the array of {count}.
    pub const TABLE_INDEX: u16 = 152;
    /// docs/modding/language/objects.md: `as_class` sees a game object as a class of its own line: `{found}` is not
    /// of `{expected}`'s.
    pub const AS_CLASS: u16 = 153;
    /// The back end does not write `{what}` yet; the build stops here rather than guess.
    pub const BACKEND_NOT_YET: u16 = 800;

    /// Every build-time code this crate gives, for the text table's check.
    pub const ALL: &[u16] = &[
        UNEXPECTED,
        NO_CLOSING_BACKQUOTE,
        NO_CLOSING_QUOTE,
        NO_INTERPOLATION_END,
        NOT_A_TOKEN,
        NO_COMMENT_END,
        UNDERSCORE_NAME,
        PREFIX_BANG,
        DOUBLED_OPERATOR,
        CHAINED_COMPARISON,
        STRUCT_IN_CONDITION,
        CONTENT_INTERPOLATES,
        BARE_ARM_NAME,
        ITEM_IN_BLOCK,
        POSITIONAL_AFTER_NAMED,
        SELF_NOT_FIRST,
        ONE_START_STATE,
        UNDERSCORE_PATTERN,
        TOO_DEEP,
        NON_ASCII_NAME,
        NO_COMPILER_THREAD,
        TYPE_ARGS_LINE_END,
        NAME_TWICE,
        NO_MODULE,
        NOT_NEEDED,
        MODULE_CYCLE,
        NO_NAME_IN_MODULE,
        NAME_NOT_PUB,
        NO_C_HEADER,
        NOT_DESCRIBED,
        UNKNOWN_TYPE,
        UNKNOWN_TYPE_NEAR,
        TYPE_ARG_COUNT,
        GAME_TYPE_NOT_CROSSING,
        MAP_KEY,
        BASE_NOT_GAME_CLASS,
        OPTION_NOT_IN_LUMA_JSON,
        OPTION_TYPE,
        STATES_FOR,
        START_NOT_STATE,
        REF_OF,
        VALUE_CYCLE,
        NAME_HIDES_GAME,
        UNKNOWN_NAME,
        UNKNOWN_NAME_NEAR,
        TYPE_MISMATCH,
        LITERAL_FIT,
        NO_MEMBER,
        NOT_CALLABLE,
        ARG_COUNT,
        ARG_UNKNOWN,
        ARG_TWICE,
        ARG_MISSING,
        ARG_NAMED_GAME,
        GAME_NO_SIGNATURE,
        GAME_AMBIGUOUS,
        GAME_PARAM_NOT_CROSSING,
        GAME_RESULT_NOT_CROSSING,
        VAR_ARG_NEEDED,
        VAR_ARG_NOT_TAKEN,
        NOT_A_PLACE,
        OPERATOR,
        OPERATOR_ONE,
        MAY_BE_NONE,
        MATCH_MISSING,
        AS_NUMBERS,
        OUTSIDE_LOOP,
        RETURN_NEEDS_VALUE,
        RETURN_NO_VALUE,
        NOT_EQUATABLE,
        NOT_ORDERED,
        FIELD_MISSING,
        NOT_A_STRUCT,
        NOT_INDEXABLE,
        TYPE_UNKNOWN_HERE,
        SELF_OUTSIDE,
        NOT_SHOWN,
        NO_VALUE,
        IF_NEEDS_ELSE,
        NOT_A_VALUE,
        TASK_CALL,
        START_NOT_TASK,
        UNWRAP_NOT_OPTIONAL,
        REFUTABLE_PATTERN,
        REF_NOT_DESTROYED,
        TYPE_NEEDS_WRITING,
        GAME_NONE_CROSSING,
        CONST_VIEW_CALL,
        MIXED_BRACES,
        BAD_ESCAPE,
        NOT_JSON_DATA,
        NOT_ITERABLE,
        MATCH_NEEDS_ELSE,
        PENDING_DROPPED,
        COPY_NOT_VIEW,
        WAIT_WHAT,
        METHOD_NEEDS_OBJECT,
        MISSING_RETURN,
        GUARD_STAYS,
        VIEW_KEPT,
        VIEW_ACROSS_WAIT,
        WAIT_HERE,
        DEFER_LEAVES,
        ON_TWICE,
        WAIT_NEGATIVE,
        HOOK_PARAMS,
        HOOK_TELL,
        HOOK_NOT_VIRTUAL,
        HOOK_NOT_OF_CLASS,
        HOOK_NO_RESULT,
        SKIP_OUTSIDE,
        SKIP_VALUE,
        SKIP_NEEDS_VALUE,
        RESULT_HERE,
        PROCEED_HERE,
        PROCEED_TWICE,
        RESULT_NOT_GIVEN,
        RESULT_NOT_YET,
        STOP_HERE,
        GO_HERE,
        GO_UNKNOWN,
        SELF_NOT_BUILT,
        STATE_PART_TWICE,
        GO_NAME_MACHINE,
        STATE_ARG,
        KEEP_HERE,
        FIXED_COUNT,
        SVC_PER_REQUEST,
        SVC_SET_BY,
        SVC_RANGE,
        SVC_PARAM,
        SVC_NOT_CALLABLE,
        CONTENT_MISSING,
        TEXT_ARG_MISSING,
        TEXT_ARG_UNKNOWN,
        TEXT_ARG_UNNAMED,
        OVERRIDE_NOT_VIRTUAL,
        NOT_DERIVABLE,
        OVERRIDE_PARAMS,
        BASE_CALL,
        CLASS_NAME_UNKNOWN,
        REPLACES_ITSELF,
        BASE_NOT_MADE,
        OVERRIDE_AMBIGUOUS,
        MADE_BY_AFTER_REPLACES,
        MAKER_MAKES_OTHER,
        NEED_OTHER_LAYOUT,
        TABLE_UNKNOWN,
        TABLE_PLACE,
        TABLE_INDEX,
        AS_CLASS,
        BACKEND_NOT_YET,
    ];

    /// The codes that are notes: the build goes on (docs/modding/language/language.md, section 9.1).
    pub const NOTES: &[u16] = &[NOT_DESCRIBED, NAME_HIDES_GAME, PENDING_DROPPED];
}

/// The values a code's message is given, by name: its row fills exactly these.
pub fn args_of(code: u16) -> &'static [&'static str] {
    match code {
        code::UNEXPECTED | code::DOUBLED_OPERATOR => &["found", "expected"],
        code::NOT_A_TOKEN | code::CONTENT_INTERPOLATES | code::ITEM_IN_BLOCK | code::NON_ASCII_NAME | code::NO_COMPILER_THREAD => {
            &["found"]
        }
        code::TOO_DEEP => &["limit"],
        code::NAME_TWICE => &["name", "line"],
        code::MODULE_CYCLE | code::VALUE_CYCLE => &["chain"],
        code::NOT_NEEDED => &["found", "need"],
        code::NO_NAME_IN_MODULE | code::NAME_NOT_PUB => &["name", "module"],
        code::NO_MODULE | code::NO_C_HEADER | code::UNKNOWN_TYPE | code::MAP_KEY | code::BASE_NOT_GAME_CLASS | code::OPTION_NOT_IN_LUMA_JSON
        | code::STATES_FOR | code::START_NOT_STATE | code::REF_OF | code::NAME_HIDES_GAME => &["found"],
        code::NOT_DESCRIBED => &["found", "what"],
        code::UNKNOWN_TYPE_NEAR => &["found", "near"],
        code::TYPE_ARG_COUNT | code::OPTION_TYPE => &["found", "expected"],
        code::GAME_TYPE_NOT_CROSSING => &["found", "why"],
        code::UNKNOWN_NAME | code::NOT_CALLABLE | code::MAY_BE_NONE | code::OUTSIDE_LOOP | code::NOT_EQUATABLE | code::NOT_ORDERED | code::NOT_A_STRUCT | code::NOT_INDEXABLE | code::TYPE_UNKNOWN_HERE | code::NOT_SHOWN | code::TASK_CALL | code::START_NOT_TASK | code::UNWRAP_NOT_OPTIONAL | code::REF_NOT_DESTROYED | code::TYPE_NEEDS_WRITING | code::CONST_VIEW_CALL => &["found"],
        code::UNKNOWN_NAME_NEAR => &["found", "near"],
        code::TYPE_MISMATCH | code::LITERAL_FIT | code::AS_NUMBERS => &["found", "expected"],
        code::NO_MEMBER | code::FIELD_MISSING => &["name", "found"],
        code::ARG_COUNT => &["name", "found", "expected"],
        code::ARG_UNKNOWN | code::ARG_MISSING => &["name", "callee"],
        code::ARG_TWICE | code::ARG_NAMED_GAME => &["name"],
        code::GAME_NO_SIGNATURE | code::GAME_AMBIGUOUS | code::GAME_NONE_CROSSING => &["found", "signatures"],
        code::GAME_PARAM_NOT_CROSSING => &["found", "param", "why"],
        code::GAME_RESULT_NOT_CROSSING => &["found", "why"],
        code::VAR_ARG_NEEDED | code::VAR_ARG_NOT_TAKEN => &["found", "param"],
        code::NOT_A_PLACE => &["why"],
        code::OPERATOR => &["op", "left", "right"],
        code::OPERATOR_ONE => &["op", "found"],
        code::MATCH_MISSING => &["cases"],
        code::RETURN_NEEDS_VALUE => &["expected"],
        code::NOT_A_VALUE => &["found", "what"],
        code::BAD_ESCAPE => &["found"],
        code::NOT_JSON_DATA => &["found"],
        code::NOT_ITERABLE => &["found"],
        code::COPY_NOT_VIEW => &["found"],
        code::WAIT_WHAT => &["found"],
        code::METHOD_NEEDS_OBJECT => &["found"],
        code::MISSING_RETURN => &["expected"],
        code::GUARD_STAYS => &[],
        code::VIEW_KEPT => &["found", "why"],
        code::VIEW_ACROSS_WAIT => &["found", "line"],
        code::WAIT_HERE => &["why"],
        code::DEFER_LEAVES => &["found"],
        code::ON_TWICE => &["what", "first"],
        code::WAIT_NEGATIVE | code::HOOK_TELL | code::HOOK_NOT_VIRTUAL | code::HOOK_NO_RESULT | code::SKIP_VALUE => &["found"],
        code::HOOK_PARAMS => &["name", "found", "expected"],
        code::HOOK_NOT_OF_CLASS => &["found", "class"],
        code::SKIP_NEEDS_VALUE | code::RESULT_NOT_GIVEN => &["found", "expected"],
        code::RESULT_HERE | code::PROCEED_HERE | code::SELF_NOT_BUILT => &["why"],
        code::PROCEED_TWICE => &["line"],
        code::SKIP_OUTSIDE | code::RESULT_NOT_YET | code::STOP_HERE | code::STATE_ARG => &[],
        code::GO_HERE | code::BACKEND_NOT_YET => &["what"],
        code::KEEP_HERE => &["what", "why"],
        code::FIXED_COUNT => &["found", "param", "expected", "count"],
        code::SVC_PER_REQUEST => &["event", "call"],
        code::SVC_SET_BY => &["name", "what"],
        code::SVC_RANGE => &["name", "param", "found", "low", "high"],
        code::SVC_PARAM => &["found", "param", "why"],
        code::SVC_NOT_CALLABLE => &["found", "why"],
        code::CONTENT_MISSING => &["kind", "found"],
        code::TEXT_ARG_MISSING | code::TEXT_ARG_UNKNOWN => &["found", "name"],
        code::TEXT_ARG_UNNAMED => &["found"],
        code::OVERRIDE_NOT_VIRTUAL => &["found", "class"],
        code::NOT_DERIVABLE | code::CLASS_NAME_UNKNOWN | code::REPLACES_ITSELF => &["found"],
        code::OVERRIDE_PARAMS => &["name", "found", "expected"],
        code::BASE_NOT_MADE => &["found", "base"],
        code::OVERRIDE_AMBIGUOUS => &["found", "class", "signatures"],
        code::MAKER_MAKES_OTHER => &["found", "made", "base"],
        code::NEED_OTHER_LAYOUT => &["need", "found", "layout"],
        code::TABLE_UNKNOWN => &["found"],
        code::TABLE_PLACE => &["found", "why"],
        code::TABLE_INDEX => &["found", "count"],
        code::AS_CLASS => &["found", "expected"],
        code::BASE_CALL => &["found", "name"],
        code::GO_UNKNOWN => &["found", "machine"],
        code::GO_NAME_MACHINE => &["what", "found", "machine"],
        code::STATE_PART_TWICE => &["found", "line"],
        _ => &[],
    }
}

/// The phrases the diagnostics name, for the text table's check.
pub const PHRASES: &[&str] = &[
    "Script.Found.LineEnd",
    "Script.Found.FileEnd",
    "Script.Want.LineEnd",
    "Script.Want.Name",
    "Script.Want.Type",
    "Script.Want.GameName",
    "Script.Want.Module",
    "Script.Want.Item",
    "Script.Want.AfterPub",
    "Script.Want.Value",
    "Script.Want.ParamType",
    "Script.Want.FieldSep",
    "Script.Want.CaseSep",
    "Script.Want.ArmSep",
    "Script.Want.EntrySep",
    "Script.Want.ClassMember",
    "Script.Want.BeforeAfter",
    "Script.Want.OnWhat",
    "Script.Want.StateOrStart",
    "Script.Want.Pattern",
    "Script.Want.Expression",
    "Script.Want.Member",
    "Script.Want.String",
    "Script.Want.InterpolationEnd",
    "Script.Want.Field",
    "Script.Want.Case",
    "Script.Want.Param",
    "Script.Want.Key",
    "Script.Want.State",
    // Why a game type does not cross (check::why_phrase, one per game::NotCrossing kind).
    "Script.Why.Unreadable",
    "Script.Why.NotKnown",
    "Script.Why.LeftOut",
    "Script.Why.PlatformWidth",
    "Script.Why.CharSign",
    "Script.Why.Buffer",
    "Script.Why.VoidPointer",
    "Script.Why.VoidValue",
    "Script.Why.Function",
    "Script.Why.MemberPointer",
    "Script.Why.PointerToPointer",
    "Script.Why.Volatile",
    "Script.Why.PointerToValue",
    "Script.Why.Kept",
    "Script.Why.MarkNotFit",
    // Why a service's function is not callable (S0134, S0135).
    "Script.Why.SvcBuffer",
    "Script.Why.SvcPointer",
    "Script.Why.SvcType",
    "Script.Why.SvcTake",
    "Script.Why.SvcForm",
    "Script.Why.SvcNative",
    "Script.Why.MatrixRows",
    "Script.Why.ClassByValue",
    "Script.Why.OwnsMemory",
    "Script.Why.CannotDrop",
    "Script.Why.CopiesItself",
    "Script.Why.CannotCopy",
    "Script.Why.Variadic",
    "Script.Why.BitFieldType",
    "Script.Why.Other",
    // What the self-description does not give yet (S0033).
    "Script.What.Derivable",
    "Script.What.Virtuals",
    "Script.What.ServiceFaces",
    // Why a place cannot be written (S0064), and what a name is when it is no value (S0083).
    "Script.Place.Let",
    "Script.Place.Option",
    "Script.Place.Param",
    "Script.Place.Value",
    "Script.Place.ReadOnly",
    "Script.Place.ConstView",
    "Script.Place.Captured",
    "Script.Kind.Type",
    "Script.Kind.Module",
    "Script.Kind.Function",
    "Script.Kind.Task",
    "Script.What.FunctionKinds",
    // Where a view would be kept (S0103), and why a `wait` cannot be where it is (S0105).
    "Script.Keep.Module",
    "Script.Keep.Field",
    "Script.Keep.Closure",
    "Script.Keep.Task",
    "Script.Keep.State",
    "Script.Wait.Where",
    "Script.Wait.Inside",
    // Why `result`, `proceed()` or `self` cannot be where they are (S0120, S0121, S0128).
    "Script.Result.Before",
    "Script.Result.NotHook",
    "Script.Proceed.NotWrap",
    "Script.Proceed.Replace",
    "Script.Self.NotYet",
    "Script.Self.Gone",
    // Where `keep` and `kept` are (S0132).
    "Script.Keep.Stop",
    "Script.Kept.Start",
    // Why set_table or table cannot reach or write a place (S0151).
    "Script.Table.NotLiteral",
    "Script.Table.NeedsIndex",
    "Script.Table.NeedsField",
    "Script.Table.NotArray",
    "Script.Table.NotRecord",
    "Script.Table.Pointer",
    "Script.Table.BitField",
    "Script.Table.Type",
];

/// Whether a code is a note, which the build goes past.
pub fn is_note(code: u16) -> bool {
    code::NOTES.contains(&code)
}
