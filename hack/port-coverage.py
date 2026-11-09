"""Usage: port-coverage.py <repo-root> [--complete] [--list-missing]. See port-coverage.sh."""
import csv
import pathlib
import re
import sys

MARKER = re.compile(r"^\s*// go: (\S+) (.+?)\s*$")
FN = re.compile(r"^\s*(pub(\(\w+\))? )?(async )?fn \w+")
ROW_NAME = re.compile(r'^\s*name: "(.*)",?\s*$')


def key_of(package, name):
    test, _, case = name.partition("/")
    return (package, test, case or "(no table)")


def scan(path, errors):
    """Yields (key, ignored) for each attached marker in one Rust file."""
    lines = path.read_text().splitlines()
    fn_ignored, attrs_ignored = False, False
    for n, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("#["):
            attrs_ignored = attrs_ignored or stripped.startswith("#[ignore")
        elif FN.match(line):
            fn_ignored, attrs_ignored = attrs_ignored, False
        m = MARKER.match(line)
        if not m:
            continue
        where = f"{path}:{n + 1}"
        key = key_of(m.group(1), m.group(2))
        rest = [l.strip() for l in lines[n + 1:]]
        attrs = []
        while rest and rest[0].startswith("#["):
            attrs.append(rest.pop(0))
        if rest and FN.match(rest[0]):
            if not any(a.startswith(("#[test]", "#[tokio::test")) for a in attrs):
                errors.append(f"{where}: marker is above a function that is not a test")
                continue
            yield key, any(a.startswith("#[ignore") for a in attrs), where
        elif rest and rest[0] == "Case {" and len(rest) > 1 and ROW_NAME.match(rest[1]):
            name = ROW_NAME.match(rest[1]).group(1)
            if name != key[2]:
                errors.append(f'{where}: marker says case "{key[2]}" but the row is named "{name}"')
                continue
            yield key, fn_ignored, where
        else:
            errors.append(f"{where}: marker is not directly above a #[test] fn or a `Case {{` row")


def main():
    root = pathlib.Path(sys.argv[1])
    flags = set(sys.argv[2:])
    with open(root / "docs/phase0/port-denominator.tsv", newline="") as f:
        denominator = [(r["package"], r["test"], r["case"]) for r in csv.DictReader(f, delimiter="\t")]
    known = set(denominator)
    errors = []

    ported, ignored = {}, {}
    files = [p for d in ("crates", "examples") for p in sorted((root / d).rglob("*.rs")) if "target" not in p.parts]
    for path in files:
        for key, is_ignored, where in scan(path, errors):
            if key not in known:
                errors.append(f"{where}: {key[0]} {key[1]}/{key[2]} is not in the denominator")
            elif key in ported or key in ignored:
                errors.append(f"{where}: {key[0]} {key[1]}/{key[2]} is already marked at {(ported | ignored)[key]}")
            else:
                (ignored if is_ignored else ported)[key] = where

    skipped = {}
    with open(root / "docs/port-skips.tsv", newline="") as f:
        for n, r in enumerate(csv.DictReader(f, delimiter="\t"), start=2):
            key, where = (r["package"], r["test"], r["case"]), f"docs/port-skips.tsv:{n}"
            if key not in known:
                errors.append(f"{where}: {key[0]} {key[1]}/{key[2]} is not in the denominator")
            elif not (r["reason"] or "").strip():
                errors.append(f"{where}: skip has no reason")
            elif key in ported:
                errors.append(f"{where}: {key[0]} {key[1]}/{key[2]} is also ported at {ported[key]}")
            else:
                skipped[key] = where

    missing = [k for k in denominator if k not in ported and k not in skipped]
    print(f"denominator {len(denominator)}  ported {len(ported)}  skipped {len(skipped)}  "
          f"missing {len(missing)}  (ignored, counted as missing: {len(ignored)})")
    for key, where in sorted(ported.items()):
        print(f"  ported   {key[0]} {key[1]}/{key[2]}  {pathlib.Path(where).relative_to(root)}")
    for key, where in sorted(ignored.items()):
        print(f"  ignored  {key[0]} {key[1]}/{key[2]}  {pathlib.Path(where).relative_to(root)}")
    if "--list-missing" in flags:
        for key in missing:
            print(f"  missing  {key[0]} {key[1]}/{key[2]}")
    for e in errors:
        print(f"error: {e}", file=sys.stderr)
    if errors or ("--complete" in flags and missing):
        sys.exit(1)


main()
