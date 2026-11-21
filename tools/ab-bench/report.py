#!/usr/bin/env python3
"""Turn ab.py's JSONL into tables.

    report.py run.jsonl [--json out.json] > report.md

For each (scenario, metric, arm): the median of all samples, the 10th-90th
percentile, n, and the range of per-round medians. Each arm is compared with
the arm before it ("step") and the first arm with the last ("all"). A
comparison is a gain or a loss only when the two arms' per-round medians do not
overlap at all and the medians differ by at least 3%; otherwise it is "none".
With 6 rounds a side, two identical builds come out disjoint about once in 460.
"""

import json
import statistics
import sys
from collections import defaultdict

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from ab import SCENARIOS  # noqa: E402
from abcore import pctl  # noqa: E402

HIGHER_IS_BETTER = ("reads_per_s",)
MIN_EFFECT = 0.03


def load(path):
    data = defaultdict(lambda: defaultdict(list))  # (s, m) -> arm -> [(r, v)]
    meta = []
    for line in open(path):
        row = json.loads(line)
        if row["m"].startswith("_"):
            meta.append(row)
            continue
        data[(row["s"], row["m"])][row["arm"]].append((row["r"], row["v"]))
    return data, meta


def stats(samples):
    vals = [v for _, v in samples]
    by_round = defaultdict(list)
    for r, v in samples:
        by_round[r].append(v)
    rm = sorted(statistics.median(v) for v in by_round.values())
    return {"n": len(vals), "rounds": len(rm), "median": statistics.median(vals),
            "p10": pctl(vals, 0.1), "p90": pctl(vals, 0.9), "rmin": rm[0], "rmax": rm[-1]}


def verdict(metric, a, b):
    """a = earlier commit, b = later commit."""
    if a["median"] == 0:
        return "none", 0.0
    delta = (b["median"] - a["median"]) / a["median"]
    better_low = not metric.startswith(HIGHER_IS_BETTER)
    disjoint_down = b["rmax"] < a["rmin"]
    disjoint_up = b["rmin"] > a["rmax"]
    if abs(delta) < MIN_EFFECT or not (disjoint_down or disjoint_up):
        return "none", delta
    improved = disjoint_down if better_low else disjoint_up
    return ("gain" if improved else "loss"), delta


def fmt(metric, x):
    if metric.startswith(("wall", "daemon_cpu")):
        return f"{x * 1000:.1f} ms" if x < 10 else f"{x:.2f} s"
    if "mib" in metric:
        return f"{x:.0f} MiB"
    return f"{x:.1f}"


def main():
    path = sys.argv[1]
    data, meta = load(path)
    out = {"comparisons": [], "arms": {}}
    print("| Scenario | Metric | Compared | Earlier median | Later median | Δ | Spread (p10–p90; round medians) / n | Verdict |")
    print("|---|---|---|---|---|---|---|---|")
    order = next((row["env"].get("arms") or list(row["env"]["binaries"])
                  for row in meta if row["m"] == "_env"), [])
    for s in SCENARIOS:
        for (sc, m), arms in sorted(data.items()):
            if sc != s:
                continue
            st = {arm: stats(v) for arm, v in arms.items()}
            out["arms"][f"{s}/{m}"] = st
            present = [a for a in order if a in st] or sorted(st)
            pairs = [("step", x, y) for x, y in zip(present, present[1:])]
            if len(present) > 2:
                pairs.append(("all", present[0], present[-1]))
            for kind, x, y in pairs:
                v, d = verdict(m, st[x], st[y])
                spread = (f"{fmt(m, st[x]['p10'])}–{fmt(m, st[x]['p90'])}; "
                          f"{fmt(m, st[x]['rmin'])}–{fmt(m, st[x]['rmax'])} / {st[x]['n']}"
                          f" → {fmt(m, st[y]['p10'])}–{fmt(m, st[y]['p90'])}; "
                          f"{fmt(m, st[y]['rmin'])}–{fmt(m, st[y]['rmax'])} / {st[y]['n']}")
                print(f"| {s} | {m} | {x} → {y} | {fmt(m, st[x]['median'])} | "
                      f"{fmt(m, st[y]['median'])} | {d * 100:+.1f}% | {spread} | {v} |")
                out["comparisons"].append({"scenario": s, "metric": m, "kind": kind, "from": x,
                                           "to": y, "delta": d, "verdict": v,
                                           "a": st[x], "b": st[y]})
    print()
    for row in meta:
        if row["m"] in ("_done", "_error"):
            print(f"- {row['s']}: {row['m'][1:]} after {row['v']:.0f}s, load average at end "
                  f"{' '.join(row.get('loadavg', []))} {row.get('err', '')}")
        if row["m"] == "_env":
            print(f"- env: {json.dumps(row['env'])}")
    if "--json" in sys.argv:
        json.dump(out, open(sys.argv[sys.argv.index("--json") + 1], "w"), indent=1)


if __name__ == "__main__":
    main()
