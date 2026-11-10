#!/usr/bin/env python3
"""log-messages.py <runA> <runB> [--exit-code]: the distinct log messages of two runs' provider.log,
per level, and what each run has that the other lacks.

A run is a directory holding provider.log (e.g. parity/.work/runs/go-k), or a log file. Go's zap
console format is tab-separated: time, level, [logger,] message, [json fields]. The Rust provider
under --debug writes tracing's pretty format: `<time> <LEVEL> <target>: <message>, <field>: <value>`,
with span and source-location lines below that this script skips. Only message text is compared,
never fields, timing or counts: zap and tracing format fields differently by design.

Logs are not part of trace-diff and this is not a layer of it. With --exit-code the script exits 1
when any level differs, for a run that wants to assert the sets match."""
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

ANSI = re.compile(r'\x1b\[[0-9;]*m')
RUST_LINE = re.compile(r'^\s*\d{4}-\d\d-\d\dT\S+\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s+(.*)$')
TARGET = re.compile(r'^[\w:.-]+: ')
FIELD_START = re.compile(r', [A-Za-z_][\w.-]*: ')
LEVELS = ('TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR')


def parse_go(line):
    parts = line.rstrip('\n').split('\t')
    if len(parts) < 3 or parts[1] not in LEVELS:
        return None
    rest = parts[2:]
    if len(rest) > 1 and rest[-1].startswith('{'):
        try:
            json.loads(rest[-1])
            rest = rest[:-1]
        except ValueError:
            pass
    # With a logger name there are two fields left, the name then the message.
    return parts[1], rest[-1]


def parse_rust(line):
    m = RUST_LINE.match(ANSI.sub('', line))
    if not m:
        return None
    level, rest = m.groups()
    rest = TARGET.sub('', rest, count=1)
    cut = FIELD_START.search(rest)
    return level, rest[:cut.start()] if cut else rest


def read(run):
    path = Path(run)
    if path.is_dir():
        path = path / 'provider.log'
    messages = defaultdict(set)
    parsed = 0
    with open(path, errors='replace') as f:
        first = next((l for l in f if l.strip()), '')
        f.seek(0)
        parse = parse_go if '\t' in first else parse_rust
        for line in f:
            got = parse(line)
            if got:
                messages[got[0]].add(got[1])
                parsed += 1
    return messages, parsed


def main(argv):
    args = [a for a in argv if not a.startswith('--')]
    if len(args) != 2:
        sys.exit(__doc__)
    runs = [(Path(a).name if Path(a).is_dir() else a, *read(a)) for a in args]
    (name_a, a, n_a), (name_b, b, n_b) = runs
    print(f'{name_a}: {n_a} lines, {sum(map(len, a.values()))} distinct messages')
    print(f'{name_b}: {n_b} lines, {sum(map(len, b.values()))} distinct messages')
    differs = False
    for level in LEVELS:
        only_a = sorted(a[level] - b[level])
        only_b = sorted(b[level] - a[level])
        if not (a[level] or b[level]):
            continue
        print(f'\n{level}: {len(a[level] & b[level])} in both')
        for name, only in ((name_a, only_a), (name_b, only_b)):
            if only:
                differs = True
                print(f'  only in {name} ({len(only)}):')
                for m in only:
                    print(f'    {m}')
    if '--exit-code' in argv and differs:
        sys.exit(1)


if __name__ == '__main__':
    main(sys.argv[1:])
