"""make.py: writes the LumaScript extension's files (grammar, brackets, snippets, package.json) from the language's
word lists, and checks them: every pattern compiles, and every .lum file under FOLDER (default: the LumaLink checkout)
is read without a character the grammar has no rule for outside strings and comments.
The extension's own words (package.json's description, settings and command; the messages extension.js shows) are
LumaLink's text table's rows LumaLink.Editor.Vscode.*, written as VS Code reads a translation: package.nls.json and
package.nls.<locale>.json, l10n/bundle.l10n.<locale>.json, and messages.js (the English, the bundles' keys). The
snippets carry no words: VS Code shows each one's code.
Run: python make.py [FOLDER]   The editor features (errors as you type, completion, hover, go to definition, rename,
formatting) come from `lumalink lsp`, which extension.js starts."""
import glob
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TEXT = os.path.join(HERE, "..", "..", "crates", "lumalink-core", "text", "LumaLinkText.tsv")
# VS Code's display languages and the game's language each takes (the table's columns, luma_validate::LANGUAGES).
LOCALES = {"fr": "EuFrench", "es": "EuSpanish", "de": "EuGerman", "it": "EuItalian", "nl": "EuDutch",
           "pt-br": "UsPortuguese", "ja": "JpJapanese", "ko": "KrKorean", "zh-cn": "CnSimpChinese",
           "zh-tw": "TwTradChinese"}
# package.json's words by their %key%, and the messages extension.js shows by the name messages.js gives each.
PACKAGE_WORDS = {"description": "Description", "setting.lumalink": "Setting.LumaLink", "setting.names": "Setting.Names",
                 "setting.sdk": "Setting.Sdk", "command.restart": "Command.Restart"}
MESSAGES = {"NO_START_LOG": "NoStartLog", "NO_START": "NoStart", "SERVER_LEFT": "ServerLeft", "NOT_JSON": "NotJson",
            "NO_INITIALIZE": "NoInitialize", "NO_RENAME": "NoRename"}
RESERVED = ("use pub let var fn return if else while for in break continue match struct enum class override hook "
            "wrap replace proceed skip state states go wait task stop on option true false none self result as and "
            "or not extern export defer test expect guard start log ref").split()
CONTENT = "model effect sound layout text font movie data nerve".split()
CONTROL = "return if else while for in break continue match go wait stop proceed skip defer guard expect".split()
DECL = ("use pub let var fn struct enum class override hook wrap replace state states task on option extern export "
        "test").split()
CONST = ["true", "false", "none"]
LANG = ["self", "result"]
WORDOPS = ["as", "and", "or", "not"]
OTHERS = [w for w in RESERVED if w not in CONTROL + DECL + CONST + LANG + WORDOPS]
TYPES = ("int i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 bool text bytes vec2 vec3 vec4 quat mat34 color list map set "
         "pending handle json").split()
ID = r"[A-Za-z_][A-Za-z0-9_]*"


def words(ws):
    return r"\b(?:" + "|".join(sorted(ws, key=len, reverse=True)) + r")\b"


def cap(*names):
    return {str(i + 1): {"name": n + ".lumascript"} for i, n in enumerate(names)}


GRAMMAR = {
    "name": "LumaScript", "scopeName": "source.lumascript", "fileTypes": ["lum"],
    "patterns": [{"include": "#comments"}, {"include": "#strings"}, {"include": "#game-name"},
                 {"include": "#colors"}, {"include": "#numbers"}, {"include": "#hooks"},
                 {"include": "#declarations"}, {"include": "#content-literal"}, {"include": "#keywords"},
                 {"include": "#types"}, {"include": "#calls"}, {"include": "#operators"}],
    "repository": {
        "comments": {"patterns": [{"name": "comment.line.double-slash.lumascript", "match": r"//.*$"},
                                  {"include": "#block-comment"}]},
        "block-comment": {"name": "comment.block.lumascript", "begin": r"/\*", "end": r"\*/",
                          "patterns": [{"include": "#block-comment"}]},
        "strings": {"patterns": [
            {"name": "string.quoted.double.raw.lumascript", "begin": r'\br"', "end": r'"'},
            {"name": "string.quoted.double.bytes.lumascript", "begin": r'\bb"', "end": r'"',
             "patterns": [{"name": "constant.character.escape.lumascript", "match": r"\\(?:x[0-9A-Fa-f]{2}|.)"}]},
            {"name": "string.quoted.double.lumascript", "begin": r'"', "end": r'"',
             "patterns": [
                 {"name": "constant.character.escape.lumascript",
                  "match": r"\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]{1,6}\}|.)"},
                 {"name": "constant.character.escape.brace.lumascript", "match": r"\{\{|\}\}"},
                 {"name": "meta.interpolation.lumascript", "begin": r"\{", "end": r"\}",
                  "beginCaptures": {"0": {"name": "punctuation.section.interpolation.begin.lumascript"}},
                  "endCaptures": {"0": {"name": "punctuation.section.interpolation.end.lumascript"}},
                  "patterns": [{"include": "$self"}]}]}]},
        "game-name": {"patterns": [
            {"name": "entity.name.function.game.quoted.lumascript", "match": r"`[^`\r\n]*`"},
            {"name": "entity.name.function.game.lumascript", "match": r"\b" + ID + r"(?:::~?" + ID + r")+"}]},
        "colors": {"name": "constant.other.color.lumascript", "match": r"#(?:[0-9A-Fa-f]{8}|[0-9A-Fa-f]{6})\b"},
        "numbers": {"patterns": [
            {"name": "constant.numeric.hex.lumascript", "match": r"\b0x[0-9A-Fa-f][0-9A-Fa-f_]*\b"},
            {"name": "constant.numeric.binary.lumascript", "match": r"\b0b[01][01_]*\b"},
            {"name": "constant.numeric.float.lumascript",
             "match": r"\b[0-9][0-9_]*(?:\.[0-9][0-9_]*)?(?:[eE][+-]?[0-9]+)?(?:f32|f64)\b"
                      r"|\b[0-9][0-9_]*\.[0-9][0-9_]*(?:[eE][+-]?[0-9]+)?\b|\b[0-9][0-9_]*[eE][+-]?[0-9]+\b"},
            {"name": "constant.numeric.integer.lumascript",
             "match": r"\b[0-9][0-9_]*(?:i8|i16|i32|i64|u8|u16|u32|u64)?\b"}]},
        "hooks": {"patterns": [
            {"match": r"\b(hook)\s+(before|after)\b", "captures": cap("keyword.other.hook", "keyword.other.hook")},
            {"match": r"\b(every)\s+(override)\s+(of)\b",
             "captures": cap("keyword.other.hook", "keyword.other.hook", "keyword.other.hook")},
            {"match": r"\b(wait)\s+(until)\b", "captures": cap("keyword.control", "keyword.control")},
            {"match": r"\b(made)\s+(by)\b(?=\s+[A-Za-z_`])",
             "captures": cap("storage.modifier.class", "storage.modifier.class")},
            {"match": r"\breplaces\b(?=\s+[A-Za-z_`])", "name": "storage.modifier.class.lumascript"},
            {"match": r"\b(?:frames|seconds)\b(?=\s*(?:$|[;})\]]|//))", "name": "keyword.other.unit.lumascript"},
            {"match": r"\b(on)\s+(start|stop|frame|load|option|stage|draw|layers)\b",
             "captures": cap("keyword.other.event", "keyword.other.event")},
            {"match": r"\b(state)\s+(" + ID + ")", "captures": cap("storage.type.state", "entity.name.type.state")},
            {"match": r"\b(?:enter|update|exit)\b(?=\s*\{)", "name": "keyword.other.state.lumascript"},
            {"match": r"\b(test)\s+(game)\b", "captures": cap("storage.type.test", "storage.modifier")},
            {"match": r"\b(use)\s+(optional|c)\b", "captures": cap("keyword.control.import", "storage.modifier")}]},
        "declarations": {"patterns": [
            {"match": r"\b(fn|task)\s+(" + ID + ")", "captures": cap("storage.type.function", "entity.name.function")},
            {"match": r"\b(struct|enum|class|states)\s+(" + ID + ")", "captures": cap("storage.type", "entity.name.type")},
            {"match": r"\b(option)\s+(" + ID + ")", "captures": cap("storage.type.option", "variable.other.option")},
            {"match": r"\b(let|var)\s+(" + ID + ")", "captures": cap("storage.type", "variable.other")}]},
        "content-literal": {"match": words(CONTENT) + r'(?=\s*")', "name": "support.function.content.lumascript"},
        "keywords": {"patterns": [
            {"name": "keyword.control.lumascript", "match": words(CONTROL)},
            {"name": "storage.type.lumascript", "match": words(DECL)},
            {"name": "constant.language.lumascript", "match": words(CONST)},
            {"name": "variable.language.lumascript", "match": words(LANG)},
            {"name": "keyword.operator.word.lumascript", "match": words(WORDOPS)},
            {"name": "keyword.other.lumascript", "match": words(OTHERS)}]},
        "types": {"patterns": [
            {"name": "support.type.primitive.lumascript", "match": words(TYPES)},
            {"name": "entity.name.type.lumascript", "match": r"\b[A-Z][A-Za-z0-9_]*\b"}]},
        "calls": {"match": r"\b([a-z_][A-Za-z0-9_]*)(?=\s*\()", "captures": cap("entity.name.function.call")},
        "operators": {"patterns": [
            {"name": "keyword.operator.arrow.lumascript", "match": r"->|=>"},
            {"name": "keyword.operator.lumascript",
             "match": r"\?\?|\?\.|\.\.=|\.\.|==|!=|<=|>=|<<|>>|[-+*/%&|^]=|[-+*/%&|^<>=!?~]"}]}}}

LANGUAGE = {
    "comments": {"lineComment": "//", "blockComment": ["/*", "*/"]},
    "brackets": [["{", "}"], ["[", "]"], ["(", ")"]],
    "autoClosingPairs": [{"open": "{", "close": "}"}, {"open": "[", "close": "]"}, {"open": "(", "close": ")"},
                         {"open": '"', "close": '"', "notIn": ["string"]}, {"open": "`", "close": "`"},
                         {"open": "/*", "close": "*/", "notIn": ["string"]}],
    "surroundingPairs": [["{", "}"], ["[", "]"], ["(", ")"], ['"', '"'], ["`", "`"]],
    "indentationRules": {"increaseIndentPattern": r"^.*[\{\[\(]\s*(//.*)?$", "decreaseIndentPattern": r"^\s*[\}\]\)]"},
    "wordPattern": ID}

PACKAGE = {
    "name": "lumascript", "displayName": "LumaScript",
    "description": "%description%",
    "version": "1.0.0", "publisher": "keepvaibin", "license": "MIT", "engines": {"vscode": "^1.75.0"},
    "categories": ["Programming Languages", "Linters", "Formatters", "Snippets"],
    "main": "./extension.js", "l10n": "./l10n",
    "activationEvents": ["onLanguage:lumascript", "workspaceContains:**/luma.json"],
    "contributes": {
        "configuration": {"title": "LumaScript", "properties": {
            "lumascript.lumalink": {"type": "string", "default": "lumalink", "markdownDescription": "%setting.lumalink%"},
            "lumascript.names": {"type": "string", "default": "", "markdownDescription": "%setting.names%"},
            "lumascript.sdk": {"type": "string", "default": "", "markdownDescription": "%setting.sdk%"}}},
        "commands": [{"command": "lumascript.restart", "title": "%command.restart%"}],
        "languages": [{"id": "lumascript", "aliases": ["LumaScript", "lumascript"], "extensions": [".lum"],
                       "configuration": "./language-configuration.json"}],
        "grammars": [{"language": "lumascript", "scopeName": "source.lumascript",
                      "path": "./syntaxes/lumascript.tmLanguage.json"}],
        "snippets": [{"language": "lumascript", "path": "./snippets/lumascript.json"}],
        "configurationDefaults": {"[lumascript]": {"editor.tabSize": 4, "editor.insertSpaces": True}}}}

SNIPPETS = {
    "hook before": {"prefix": "hookb", "body": ["hook before ${1:Class::function}(${2}) {", "\t$0", "}"]},
    "hook after": {"prefix": "hooka", "body": ["hook after ${1:Class::function}(${2}) -> ${3:out} {", "\t$0", "}"]},
    "wrap": {"prefix": "wrap", "body": ["wrap ${1:Class::function}(${2}) {", "\t$0", "\tproceed", "}"]},
    "on start": {"prefix": "onstart", "body": ["on start {", "\t$0", "}"]},
    "on frame": {"prefix": "onframe", "body": ["on frame {", "\t$0", "}"]},
    "option": {"prefix": "option", "body": ["option ${1:name}: ${2:bool}"]},
    "class from the game": {"prefix": "class",
                            "body": ["class ${1:Name}: ${2:GameClass} {", "\toverride fn ${3:init}(self${4}) {",
                                     "\t\t$0", "\t}", "}"]},
    "states": {"prefix": "states",
               "body": ["states ${1:Name} for ${2:Class} {", "\tstart ${3:Idle}", "\tstate ${3:Idle} {",
                        "\t\tenter { $0 }", "\t\tupdate { }", "\t}", "}"]},
    "task": {"prefix": "task", "body": ["task ${1:name}(${2}) {", "\t$0", "\twait ${3:60} frames", "}"]},
    "test": {"prefix": "test", "body": ['test "${1:what it proves}" {', "\texpect ${2:condition}", "}"]},
    "fn": {"prefix": "fn", "body": ["fn ${1:name}(${2}) -> ${3:int} {", "\t$0", "}"]}}


def rows():
    """LumaLink.Editor.Vscode.<name> -> {language: text}, from the table (its header names the columns)."""
    with open(TEXT, encoding="utf-8") as f:
        lines = f.read().splitlines()
    head = lines[0].split("\t")
    out = {}
    for line in lines[1:]:
        cells = line.split("\t")
        if cells[0].startswith("LumaLink.Editor.Vscode."):
            out[cells[0][len("LumaLink.Editor.Vscode."):]] = dict(zip(head[1:], cells[1:]))
    return out


def numbered(text, names):
    """`{name}` as VS Code's l10n numbers its arguments: `{0}`, `{1}` in the English's order."""
    return re.sub(r"\{([a-z]+)\}", lambda m: "{%d}" % names.index(m.group(1)), text)


def words():
    """The files that carry the extension's words, by their path in the extension, as JSON values (messages.js as
    text)."""
    t = rows()
    files = {"package.nls.json": {k: t[r]["UsEnglish"] for k, r in PACKAGE_WORDS.items()}}
    names = {m: re.findall(r"\{([a-z]+)\}", t[r]["UsEnglish"]) for m, r in MESSAGES.items()}
    english = {m: numbered(t[r]["UsEnglish"], names[m]) for m, r in MESSAGES.items()}
    for locale, language in LOCALES.items():
        files[f"package.nls.{locale}.json"] = {k: t[r][language] for k, r in PACKAGE_WORDS.items()}
        files[f"l10n/bundle.l10n.{locale}.json"] = {english[m]: numbered(t[r][language], names[m]) for m, r in MESSAGES.items()}
    js = "// Written by make.py from LumaLink's text table (LumaLink.Editor.Vscode.*): the English of each message\n"
    js += "// extension.js shows, which vscode.l10n.t finds in l10n/bundle.l10n.<locale>.json.\n'use strict';\n\n"
    js += "module.exports = {\n" + "".join(f"    {m}: {json.dumps(e, ensure_ascii=False)},\n" for m, e in english.items()) + "};\n"
    files["messages.js"] = js
    return files


def write(path, value):
    full = os.path.join(HERE, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8", newline="\n") as f:
        json.dump(value, f, indent=1, ensure_ascii=False)
        f.write("\n")


def patterns(node, out):
    if isinstance(node, dict):
        for key, value in node.items():
            if key in ("match", "begin", "end"):
                out.append(value)
            patterns(value, out)
    elif isinstance(node, list):
        for value in node:
            patterns(value, out)


def main():
    write("syntaxes/lumascript.tmLanguage.json", GRAMMAR)
    write("language-configuration.json", LANGUAGE)
    write("package.json", PACKAGE)
    write("snippets/lumascript.json", SNIPPETS)
    for path, value in words().items():
        if isinstance(value, str):
            with open(os.path.join(HERE, path), "w", encoding="utf-8", newline="\n") as f:
                f.write(value)
        else:
            write(path, value)
    found = []
    patterns(GRAMMAR, found)
    for pattern in found:
        re.compile(pattern)
    strip = re.compile(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/|`[^`\n]*`', re.S)
    known = re.compile(r"[A-Za-z0-9_\s(){}\[\].,:;#?!<>=+\-*/%&|^~@]")
    odd, files, seen_words = {}, 0, set()
    root = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "..")
    for path in glob.glob(os.path.join(root, "**", "*.lum"), recursive=True):
        try:
            text = open(path, encoding="utf-8").read()
        except (OSError, UnicodeDecodeError):
            continue
        files += 1
        bare = strip.sub(" ", text)
        for ch in bare:
            if not known.match(ch):
                odd[ch] = odd.get(ch, 0) + 1
        seen_words.update(re.findall(r"\b[a-z]+\b", bare))
    unused = [w for w in RESERVED if w not in seen_words]
    print(f"{len(found)} patterns compile; {files} .lum files read; characters with no rule: {odd or 'none'}; "
          f"reserved words no file uses yet: {unused}")


if __name__ == "__main__":
    main()
