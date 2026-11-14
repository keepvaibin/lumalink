"""game_functions.py SYMBOLS OUT: the function table LumaLink ships for 7.5 (a code patch named by the game function it
touches). SYMBOLS: a symbols.txt of RMGE01 (the first release; region_map.rs reads other releases as it). OUT: zlib of
lines "start size name params" (hex, hex, tab-separated), sorted by start, every function (type:function), the name
demangled from CodeWarrior's mangling (Class::method, operators as C++ spells them) and params the parameter types as
C++ writes them, comma-separated ("" for none, "?" when the mangling was not understood: then name is the symbol).
Numbers and names only: no code of the game."""
import re
import sys
import zlib

OPERATORS = {
    "nw": "operator new", "dl": "operator delete", "nwa": "operator new[]", "dla": "operator delete[]",
    "pl": "operator+", "mi": "operator-", "ml": "operator*", "dv": "operator/", "md": "operator%", "er": "operator^",
    "ad": "operator&", "or": "operator|", "co": "operator~", "nt": "operator!", "as": "operator=", "lt": "operator<",
    "gt": "operator>", "apl": "operator+=", "ami": "operator-=", "amu": "operator*=", "adv": "operator/=",
    "amd": "operator%=", "aer": "operator^=", "aad": "operator&=", "aor": "operator|=", "ls": "operator<<",
    "rs": "operator>>", "ars": "operator>>=", "als": "operator<<=", "eq": "operator==", "ne": "operator!=",
    "le": "operator<=", "ge": "operator>=", "aa": "operator&&", "oo": "operator||", "pp": "operator++",
    "mm": "operator--", "cm": "operator,", "rm": "operator->*", "rf": "operator->", "cl": "operator()",
    "vc": "operator[]",
}
BUILTIN = {"v": "void", "b": "bool", "c": "char", "s": "short", "i": "int", "l": "long", "x": "long long",
           "f": "float", "d": "double", "r": "long double", "w": "wchar_t", "e": "..."}


class Bad(Exception):
    pass


def number(s, i):
    j = i
    while j < len(s) and s[j].isdigit():
        j += 1
    if j == i:
        raise Bad()
    return int(s[i:j]), j


def readable(name):
    """A name as C++ writes it: template arguments demangled (kept as they are when not understood), an anonymous
    namespace as (anonymous namespace)."""
    m = re.fullmatch(r"@unnamed@(\w+)_cpp@", name)
    if m:
        return "(anonymous namespace)"
    lt = name.find("<")
    if lt == -1 or not name.endswith(">"):
        return name
    args, depth, cur = [], 0, ""
    for ch in name[lt + 1:-1]:
        if ch == "," and depth == 0:
            args.append(cur)
            cur = ""
            continue
        depth += ch == "<"
        depth -= ch == ">"
        cur += ch
    args.append(cur)
    out = []
    for a in args:
        try:
            text, j = one_type(a, 0)
            out.append(text if j == len(a) else a)
        except (Bad, IndexError, ValueError):
            out.append(a)
    return name[:lt] + "<" + ", ".join(out) + ">"


def class_name(s, i):
    """A length-prefixed name or Q<n> qualified names at i: (text, next)."""
    if i < len(s) and s[i] == "Q":
        count = int(s[i + 1])
        i += 2
        parts = []
        for _ in range(count):
            n, i = number(s, i)
            if i + n > len(s):
                raise Bad()
            parts.append(readable(s[i:i + n]))
            i += n
        return "::".join(parts), i
    n, i = number(s, i)
    if i + n > len(s):
        raise Bad()
    return readable(s[i:i + n]), i + n


def one_type(s, i):
    """A type at i: (text, next). Pointers and references read right to left as C++ writes them."""
    prefix = []
    while i < len(s) and s[i] in "CVUS":
        prefix.append({"C": "const", "V": "volatile", "U": "unsigned", "S": "signed"}[s[i]])
        i += 1
    if i >= len(s):
        raise Bad()
    c = s[i]
    if c in "PR":
        inner, j = one_type(s, i + 1)
        text = inner + ("*" if c == "P" else "&")
        cv = [p for p in prefix if p in ("const", "volatile")]
        return (text + " " + " ".join(cv)) if cv else text, j
    if c == "A":
        n, j = number(s, i + 1)
        if s[j] != "_":
            raise Bad()
        inner, j = one_type(s, j + 1)
        return "%s[%d]" % (inner, n), j
    if c == "F":
        params, j = param_list(s, i + 1, stop="_")
        if j >= len(s) or s[j] != "_":
            raise Bad()
        ret, j = one_type(s, j + 1)
        return "%s (%s)" % (ret, ", ".join(params)), j
    if c == "M":
        owner, j = class_name(s, i + 1)
        inner, j = one_type(s, j)
        return "%s %s::*" % (inner, owner), j
    if c in BUILTIN:
        return " ".join(prefix + [BUILTIN[c]]), i + 1
    if c.isdigit() or c == "Q":
        name, j = class_name(s, i)
        return " ".join(prefix + [name]), j
    raise Bad()


def param_list(s, i, stop=None):
    out = []
    while i < len(s) and s[i] != stop:
        t, i = one_type(s, i)
        out.append(t)
    return out, i


def demangle(symbol):
    """(name, params): params "(docs/modding/language/types.md)" or "(docs/modding/language/types.md) const"; "?" when the symbol says no parameters or was not
    understood (then name is the symbol)."""
    m = re.fullmatch(r"__sinit_\\(\w+)_cpp", symbol)
    if m:
        return "static initializers of %s.cpp" % m.group(1), "()"
    # The function's own name ends at the first "__" past its first character that a class or "F" follows.
    start = 2 if symbol.startswith("__") else 1
    at = symbol.find("__", start)
    while at != -1:
        base, rest = symbol[:at], symbol[at + 2:]
        try:
            owner = ""
            i = 0
            if rest and (rest[0].isdigit() or rest[0] == "Q"):
                owner, i = class_name(rest, 0)
            const = False
            if i < len(rest) and rest[i] == "C" and i + 1 < len(rest) and rest[i + 1] == "F":
                const = True
                i += 1
            if i < len(rest) and rest[i] == "F":
                params, i = param_list(rest, i + 1, stop="_")
                if i < len(rest) and rest[i] == "_" and "<" in base:
                    _, i = one_type(rest, i + 1)  # a function template's return type
            elif i == len(rest) and owner:
                params = None  # data or a function without its parameters in the symbol
            else:
                raise Bad()
            if i != len(rest):
                raise Bad()
            leaf = owner.split("::")[-1].split("<")[0] if owner else ""
            if base.startswith("__"):
                op = base[2:]
                if op == "ct":
                    base = leaf
                elif op == "dt":
                    base = "~" + leaf
                elif op in OPERATORS:
                    base = OPERATORS[op]
                elif op.startswith("op"):
                    t, j = one_type(op, 2)
                    if j != len(op):
                        raise Bad()
                    base = "operator " + t
            name = (owner + "::" if owner else "") + readable(base)
            if params is None:
                return name, "?"
            if params == ["void"]:
                params = []
            return name, "(" + ", ".join(params) + ")" + (" const" if const else "")
        except (Bad, IndexError, ValueError):
            at = symbol.find("__", at + 1)
    plain = re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", symbol) and "__" not in symbol[2:]
    return symbol, "()" if plain else "?"


LINE = re.compile(r"^(\S+) = \.\w+:0x([0-9A-Fa-f]+); // type:function size:0x([0-9A-Fa-f]+)")


def main():
    rows = []
    for line in open(sys.argv[1], encoding="utf-8"):
        m = LINE.match(line)
        if not m:
            continue
        name, params = demangle(m.group(1))
        rows.append((int(m.group(2), 16), int(m.group(3), 16), name, params))
    rows.sort()
    text = "".join("%X\t%X\t%s\t%s\n" % r for r in rows)
    open(sys.argv[2], "wb").write(zlib.compress(text.encode("utf-8"), 9))
    unknown = sum(1 for r in rows if r[3] == "?")
    print("%d functions, %d not demangled, %d bytes" % (len(rows), unknown, len(zlib.compress(text.encode(), 9))))


if __name__ == "__main__":
    if sys.argv[1] == "--test":
        for s in ["control__10MarioActorFv", "__ct__10MarioActorFv", "__dt__Q22NrvKuribo9HostTypeFv",
                  "isDead__2MRFPC9LiveActor", "getRandom__2MRFll", "__as__Q29JGeometry8TVec3<f>FRCQ29JGeometry8TVec3<f>",
                  "calcAnim__9LiveActorFv", "__start", "fn__FPFi_v", "x__3FooCFUiPCc", "__dt__Q29NrvKuribo8HostTypeFv",
                  "createNameObj<18GroupSwitchWatcher>__28@unnamed@NameObjFactory_cpp@FPCc_P7NameObj",
                  "__sinit_\\Kameck_cpp", "ConvertOffsToPtr<Q44nw4r3lyt3res6TexMap>__Q34nw4r3lyt6detailFPCvUi_PCQ44nw4r3lyt3res6TexMap"]:
            print(s, "->", demangle(s))
    else:
        main()
