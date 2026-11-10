#!/usr/bin/env python3
"""pc-settle.py <run>...: per scenario namespace, before the scenario's cleanup
deletes the ProviderConfig, prints
  gap     ms from the provider's last MR or usage write to its last ProviderConfig write
  margin  ms from that last ProviderConfig write to cleanup's ProviderConfig delete
and the scenarios where a provider ProviderConfig write got a 409. A margin
near 0 means cleanup raced the ProviderConfig controller. Reads audit-raw.jsonl."""
import json, sys
from collections import defaultdict
from datetime import datetime

def ts(s): return datetime.fromisoformat(s.replace('Z', '+00:00')).timestamp()

for run in sys.argv[1:]:
    ev, conflicts = defaultdict(list), set()
    for line in open(f'parity/.work/runs/{run}/audit-raw.jsonl'):
        e = json.loads(line); o = e.get('objectRef', {}); ns = o.get('namespace')
        if not ns: continue
        code = e.get('responseStatus', {}).get('code', 0)
        who = 'p' if 'provider' in e['user']['username'] else 's'
        if who == 'p' and code == 409 and o.get('resource') == 'providerconfigs':
            conflicts.add(ns.removeprefix('parity-'))
        if code < 300:
            ev[ns].append((ts(e['requestReceivedTimestamp']), who, e['verb'], o.get('resource')))
    gaps, margins = [], []
    for ns, xs in ev.items():
        cut = min([t for t, w, v, r in xs if w == 's' and v == 'delete' and r == 'providerconfigs'], default=None)
        if cut is None: continue
        pre = [x for x in xs if x[0] < cut and x[1] == 'p']
        mr = [t for t, w, v, r in pre if r in ('mytypes', 'providerconfigusages')]
        pc = [t for t, w, v, r in pre if r == 'providerconfigs']
        if pc: margins.append(round((cut - max(pc)) * 1000))
        if mr and pc: gaps.append(round((max(pc) - max(mr)) * 1000))
    med = lambda xs: sorted(xs)[len(xs) // 2]
    print(f'{run}: gap median {med(gaps)} | margin min {min(margins)} median {med(margins)} | 409s {sorted(conflicts)}')
