#!/usr/bin/env python3
"""Plan 024: one table from two hosts' suite output, with the 10% criterion.

    summarize.py <tauri-dir> <native-dir>

For every metric the value compared is the median across runs of the run's
own p95 (or of its single value). "Worse" means higher: latency, frame
interval, long-frame share, memory and CPU are all lower-is-better.
"""
import json
from pathlib import Path
import statistics
import sys


def load(directory, pattern):
    return [json.loads(path.read_text()) for path in sorted(Path(directory).glob(pattern))]


def metric(directory, pattern, pick):
    values = [pick(run) for run in load(directory, pattern)]
    values = [value for value in values if value is not None]
    return (statistics.median(values), values) if values else (None, [])


ROWS = [
    ('Typing latency p95 (ms)', 'latency-*.json', lambda r: r['summary'].get('p95')),
    ('Typing latency p50 (ms)', 'latency-*.json', lambda r: r['summary'].get('p50')),
    ('Idle CPU (% of one core)', 'idle.json', lambda r: r['cpuPercentOfOneCore']),
    ('Idle footprint (MiB)', 'idle.json', lambda r: r['footprintMiB'].get('p50')),
]
for fixture, label in (('wide', 'wide rows'), ('large', 'large cells'), ('many', 'many rows')):
    ROWS += [
        (f'Scroll {label}: frame interval p95 (ms)', f'scroll-{fixture}-down-*.json',
         lambda r: r['summary'].get('p95')),
        (f'Scroll {label}: long-frame share (%)', f'scroll-{fixture}-down-*.json',
         lambda r: 100 * r['longFrameShare']),
        (f'Footprint with {label} loaded (MiB)', f'footprint-{fixture}.json',
         lambda r: r['footprintMiB'].get('p50')),
    ]
ROWS += [
    ('Scroll wide rows sideways: frame interval p95 (ms)', 'scroll-wide-right-*.json',
     lambda r: r['summary'].get('p95')),
    ('Scroll wide rows sideways: long-frame share (%)', 'scroll-wide-right-*.json',
     lambda r: 100 * r['longFrameShare']),
    ('Startup: first paint (ms)', 'startup-*.json', lambda r: r['firstLargePaintMs']),
    ('Startup: main content, last paint of half the window or more (ms)', 'startup-*.json',
     lambda r: max((t for t, c in zip(r['largePaintTimesMs'], r['largePaintCoverage']) if c >= 0.5),
                   default=None)),
    ('Startup: settled, last paint of 5% of the window or more (ms)', 'startup-*.json',
     lambda r: r['lastLargePaintMs']),
]

if __name__ == '__main__':
    tauri, native = sys.argv[1], sys.argv[2]
    print('| Metric | Tauri | Native | Native vs Tauri | Runs (Tauri / native) |')
    print('| --- | ---: | ---: | ---: | --- |')
    for name, pattern, pick in ROWS:
        (a, a_runs), (b, b_runs) = metric(tauri, pattern, pick), metric(native, pattern, pick)
        if a is None or b is None:
            print(f'| {name} | {a if a is not None else "n/a"} | {b if b is not None else "n/a"} | n/a | |')
            continue
        change = (b - a) / a * 100 if a else float('inf') if b else 0.0
        verdict = 'worse by more than 10%' if change > 10 else ''
        runs = ' '.join(f'{v:.1f}' for v in a_runs) + ' / ' + ' '.join(f'{v:.1f}' for v in b_runs)
        print(f'| {name} | {a:.1f} | {b:.1f} | {change:+.0f}% {verdict} | {runs} |')
