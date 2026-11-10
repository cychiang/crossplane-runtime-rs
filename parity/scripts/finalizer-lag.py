#!/usr/bin/env python3
"""finalizer-lag.py <run>...: per scenario namespace, ms from the scenario's
delete of the MR to the provider's update that removes its finalizer, when
that wait is longer than 20s (the MR sat out the 30s creation grace period
over a chain of rate-limited requeues). The sum carries every requeue's
reconcile time, so it measures per-cycle lag far better than one interval.
Reads audit-raw.jsonl."""
import json, sys
from collections import defaultdict
from datetime import datetime

def ts(s): return datetime.fromisoformat(s.replace('Z', '+00:00')).timestamp()

for run in sys.argv[1:]:
    ev = defaultdict(list)
    for line in open(f'parity/.work/runs/{run}/audit-raw.jsonl'):
        e = json.loads(line); o = e.get('objectRef', {})
        if o.get('resource') != 'mytypes' or e.get('responseStatus', {}).get('code', 0) >= 300: continue
        who = 'p' if 'provider' in e['user']['username'] else 's'
        ev[o['namespace']].append((ts(e['requestReceivedTimestamp']), who, e['verb'], o.get('subresource')))
    out = []
    for ns, xs in sorted(ev.items()):
        xs.sort()
        dels = [t for t, w, v, s in xs if w == 's' and v == 'delete']
        if not dels: continue
        fin = [t for t, w, v, s in xs if w == 'p' and v == 'update' and not s and t > dels[0]]
        if fin and fin[-1] - dels[0] > 20: out.append(round((fin[-1] - dels[0]) * 1000))
    print(run, 'n=%d' % len(out), 'median=%d' % sorted(out)[len(out) // 2] if out else '', sorted(out))
