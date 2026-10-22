#!/usr/bin/env python3
"""Recover the aiserver.v1 protobuf schema from the Grok Bot desktop app.

The app bundles protobuf-es generated code in `dist/electron-main/proto.cjs`
inside `Contents/Resources/app.asar`. Minification keeps three things intact,
which is all we need (see docs/schema.md for the full story):

  * messages:  IDENT=class e extends BASE{... static $(){return["Name|1 f 9|2 g #0?",REF0,...]}}
  * enums:     IDENT=a(s,PKG,"Name",[[0,"UNSPECIFIED"],[1,"FOO"],...],1)
  * services:  {typeName:"aiserver.v1.FooService",methods:{x:{name:"X",I:REQ,O:RES,kind:i.Unary},...}}

Field descriptor grammar (one per `|`):  `<no> <name> <type>[?|*] [<oneof>]`
  type  = scalar code | `#k` (k-th REF after the string) | `K,V` (map)
  `?` = proto3 optional, `*` = repeated.

Usage:
  extract_schema.py [--app /Applications/Grok\\ Bot.app] [--out schema/]
Writes `<out>/aiserver.proto` and `<out>/services.txt`; both are sorted and
deterministic so `git diff` after an app update shows exactly what changed.
"""
import argparse, os, plistlib, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from asar import read_header, walk  # noqa: E402

BUNDLE = "dist/electron-main/proto.cjs"

SCALARS = {
    "1": "double", "2": "float", "3": "int64", "4": "uint64", "5": "int32",
    "6": "fixed64", "7": "fixed32", "8": "bool", "9": "string", "12": "bytes",
    "13": "uint32", "15": "sfixed32", "16": "sfixed64", "17": "sint32", "18": "sint64",
}
WKT = ("Timestamp", "Empty", "Struct", "Value", "ListValue", "Duration", "Any", "FieldMask")

MSG_RE = re.compile(
    r'([\w$]+)=class e extends [\w$]+\{(?:(?!=class e extends).)*?'
    r'static \$\(\)\{return\["([^"]*)"((?:,[\w$]+)*)\]\}',
    re.S,
)
ENUM_RE = re.compile(r'([\w$]+)=a\(s,[\w$]+,"([\w.]+)",\[((?:\[\d+,"\w+"\],?)*)\]')
WKT_RE = re.compile(r'([\w$]+)=class e extends [\w$]+\{(?:(?!=class e extends).){0,4000}?typeName="google\.protobuf\.(\w+)"', re.S)
SVC_RE = re.compile(r'\{typeName:"([\w.]+Service)",methods:\{')
METHOD_RE = re.compile(r'name:"(\w+)",I:([\w$]+),O:([\w$]+),kind:[\w$]+\.(\w+)')


def read_bundle(app):
    asar_path = os.path.join(app, "Contents/Resources/app.asar")
    header, base = read_header(asar_path)
    entry = dict(walk(header))[BUNDLE]
    with open(asar_path, "rb") as f:
        f.seek(base + int(entry["offset"]))
        return f.read(entry["size"]).decode("utf-8")


def app_version(app):
    with open(os.path.join(app, "Contents/Info.plist"), "rb") as f:
        return plistlib.load(f).get("CFBundleShortVersionString", "?")


def balanced(src, start):
    """Return src[start:end] where end closes the brace opened just before start."""
    depth, i = 1, start
    while depth:
        depth += {"{": 1, "}": -1}.get(src[i], 0)
        i += 1
    return src[start:i - 1]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--app", default="/Applications/Grok Bot.app")
    ap.add_argument("--out", default="schema")
    args = ap.parse_args()
    src = read_bundle(args.app)

    names, messages, enums, warnings = {}, {}, {}, []

    def bind(ident, name):
        if not name.startswith("google."):
            name = name.replace(".", "_")  # nested types are emitted flattened
        if names.get(ident, name) != name:
            warnings.append(f"identifier {ident} bound to both {names[ident]} and {name}")
        names[ident] = name

    for ident, wkt in WKT_RE.findall(src):
        if wkt in WKT:
            bind(ident, f"google.protobuf.{wkt}")
    for ident, name, values in ENUM_RE.findall(src):
        bind(ident, name)
        enums[name.replace(".", "_")] = re.findall(r'\[(\d+),"(\w+)"\]', values)

    for m in MSG_RE.finditer(src):
        ident, desc, refs = m.group(1), m.group(2), m.group(3)
        name, *fields = desc.split("|")
        bind(ident, name)
        messages[name.replace(".", "_")] = (fields, [r for r in refs.split(",") if r])

    def type_of(code, refs, ctx):
        if code.startswith("#"):
            ref = refs[int(code[1:])]
            if ref == "e":  # self-reference inside its own class body
                return ctx
            if ref not in names:
                warnings.append(f"{ctx}: unresolved reference {ref}")
                return f"UNRESOLVED_{ref}"
            return names[ref]
        return SCALARS.get(code) or f"UNKNOWN_SCALAR_{code}"

    def render_message(name, fields, refs):
        plain, oneofs = [], {}
        for f in fields:
            no, fname, ty, *oneof = f.split(" ")
            label = ""
            if ty.endswith("?"):
                label, ty = "optional ", ty[:-1]
            elif ty.endswith("*"):
                label, ty = "repeated ", ty[:-1]
            if "," in ty:
                k, v = ty.split(",")
                t = f"map<{type_of(k, refs, name)}, {type_of(v, refs, name)}>"
                label = ""
            else:
                t = type_of(ty, refs, name)
            line = f"{t} {fname} = {no};"
            if oneof:
                oneofs.setdefault(oneof[0], []).append(line)
            else:
                plain.append(f"{label}{line}")
        out = [f"message {name} {{"]
        out += [f"  {l}" for l in plain]
        for o, ls in oneofs.items():
            out.append(f"  oneof {o} {{")
            out += [f"    {l}" for l in ls]
            out.append("  }")
        out.append("}")
        return "\n".join(out)

    services = {}
    for m in SVC_RE.finditer(src):
        body = balanced(src, m.end())
        services[m.group(1)] = [
            (n, names.get(i, f"UNRESOLVED_{i}"), names.get(o, f"UNRESOLVED_{o}"), k)
            for n, i, o, k in METHOD_RE.findall(body)
        ]

    version = app_version(args.app)
    os.makedirs(args.out, exist_ok=True)
    proto = [
        f"// GENERATED by tools/extract_schema.py from Grok Bot {version} ({BUNDLE}).",
        "// Reconstructed from minified protobuf-es output: field numbers, names, types,",
        "// labels and oneofs are exact; nested types are flattened (Outer_Inner) and",
        "// everything is placed in one package. Do not edit by hand.",
        'syntax = "proto3";',
        "package aiserver.v1;",
        "",
    ]
    used = sorted({n.split(".")[-1] for n in names.values() if n.startswith("google.")})
    proto += [f'import "google/protobuf/{re.sub(r"(?<!^)(?=[A-Z])", "_", w).lower()}.proto";'
              for w in used if w != "ListValue" and w != "Value"]
    proto.append("")
    for svc in sorted(services):
        proto.append(f"service {svc.rsplit('.', 1)[1]} {{")
        for n, i, o, k in services[svc]:
            req = f"stream {i}" if k in ("ClientStreaming", "BiDiStreaming") else i
            res = f"stream {o}" if k in ("ServerStreaming", "BiDiStreaming") else o
            proto.append(f"  rpc {n}({req}) returns ({res});")
        proto += ["}", ""]
    for name in sorted(enums):
        proto.append(f"enum {name} {{")
        # protobuf-es strips the conventional SCREAMING_SNAKE prefix. We re-add one
        # derived from the full flattened name so values stay unique package-wide;
        # it may differ from the server's real prefix (clients match by suffix).
        prefix = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", name).upper()
        proto += [f"  {prefix}_{v} = {n};" for n, v in enums[name]]
        proto += ["}", ""]
    for name in sorted(messages):
        if name in enums:  # same short name in two packages; keep the enum
            print(f"warning: {name} is both an enum and a message; message skipped", file=sys.stderr)
            continue
        proto += [render_message(name, *messages[name]), ""]

    with open(os.path.join(args.out, "aiserver.proto"), "w") as f:
        f.write("\n".join(proto))
    with open(os.path.join(args.out, "services.txt"), "w") as f:
        f.write(f"# GENERATED from Grok Bot {version}. One line per RPC: Service/Method  Request -> Response  [kind]\n")
        for svc in sorted(services):
            for n, i, o, k in services[svc]:
                f.write(f"{svc}/{n}  {i} -> {o}" + ("" if k == "Unary" else f"  [{k}]") + "\n")

    print(f"Grok Bot {version}: {len(services)} services, "
          f"{sum(map(len, services.values()))} rpcs, {len(messages)} messages, {len(enums)} enums",
          file=sys.stderr)
    for w in sorted(set(warnings)):
        print(f"warning: {w}", file=sys.stderr)


if __name__ == "__main__":
    main()
