#!/usr/bin/env python3
"""Sample CPU of a process and all its descendants, like `ps` would not.

CEF puts rendering in child processes, so measuring only the host pid hides
most of the cost. Ticks come from /proc/<pid>/stat; the clock tick rate from
os.sysconf('SC_CLK_TCK').

    scripts/tree_cpu.py <pid> <seconds> [label]

Prints one JSON line: label, seconds, cpu_percent, rss_mib, processes.
"""
import json
import os
import sys
import time

HZ = os.sysconf("SC_CLK_TCK")


def snapshot(root: int):
    stats = {}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat", encoding="utf-8", errors="replace") as handle:
                line = handle.read()
            after = line.rsplit(")", 1)[1].split()
            ppid = int(after[1])
            ticks = int(after[11]) + int(after[12])
            rss = int(after[21]) * os.sysconf("SC_PAGE_SIZE")
        except (OSError, IndexError, ValueError):
            continue
        stats[int(entry)] = (ppid, ticks, rss)
    members = {root}
    changed = True
    while changed:
        changed = False
        for pid, (ppid, _, _) in stats.items():
            if ppid in members and pid not in members:
                members.add(pid)
                changed = True
    ticks = sum(stats[pid][1] for pid in members if pid in stats)
    rss = sum(stats[pid][2] for pid in members if pid in stats)
    return ticks, rss, len(members)


def main() -> None:
    root = int(sys.argv[1])
    seconds = float(sys.argv[2])
    label = sys.argv[3] if len(sys.argv) > 3 else "run"
    first, _, _ = snapshot(root)
    start = time.monotonic()
    time.sleep(seconds)
    last, rss, processes = snapshot(root)
    elapsed = time.monotonic() - start
    percent = (last - first) / HZ / elapsed * 100.0
    print(json.dumps({
        "label": label,
        "seconds": round(elapsed, 2),
        "cpu_percent": round(percent, 2),
        "rss_mib": round(rss / 1048576, 1),
        "processes": processes,
    }))


if __name__ == "__main__":
    main()
