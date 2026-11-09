"""Usage: diff.py <go-crds.json> <rust-crds.json>

Each file is a stream of concatenated CustomResourceDefinition JSON documents. Prints one line
per structural difference under `spec` and exits 1 when there is any, or when a CRD exists on
one side only. `description` values are excluded and counted separately.
"""
import json
import sys


def load(path):
    text, decoder, out, i = open(path).read(), json.JSONDecoder(), {}, 0
    while True:
        while i < len(text) and text[i].isspace():
            i += 1
        if i >= len(text):
            return out
        doc, i = decoder.raw_decode(text, i)
        out[doc["metadata"]["name"]] = doc["spec"]


def walk(go, rust, path, structural, descriptions):
    if isinstance(go, dict) and isinstance(rust, dict):
        for key in sorted(set(go) | set(rust)):
            here = f"{path}/{key}"
            if key == "description" and not isinstance(go.get(key, ""), dict) and not isinstance(rust.get(key, ""), dict):
                if go.get(key) != rust.get(key):
                    descriptions.append(here)
            elif key not in go:
                structural.append(f"+rust {here} = {json.dumps(rust[key])[:120]}")
            elif key not in rust:
                structural.append(f"-go   {here} = {json.dumps(go[key])[:120]}")
            else:
                walk(go[key], rust[key], here, structural, descriptions)
    elif isinstance(go, list) and isinstance(rust, list) and len(go) == len(rust):
        for n, (g, r) in enumerate(zip(go, rust)):
            walk(g, r, f"{path}[{n}]", structural, descriptions)
    elif go != rust:
        structural.append(f"~     {path}: go={json.dumps(go)[:100]} rust={json.dumps(rust)[:100]}")


def main():
    go, rust = load(sys.argv[1]), load(sys.argv[2])
    failed = False
    for name in sorted(set(go) | set(rust)):
        if name not in go or name not in rust:
            print(f"== {name}: only in {'rust' if name not in go else 'go'}")
            failed = True
            continue
        structural, descriptions = [], []
        walk(go[name], rust[name], "spec", structural, descriptions)
        print(f"== {name}: {len(structural)} structural differences "
              f"(descriptions excluded; {len(descriptions)} differ)")
        for line in structural:
            print("  " + line)
        failed = failed or bool(structural)
    sys.exit(1 if failed else 0)


main()
