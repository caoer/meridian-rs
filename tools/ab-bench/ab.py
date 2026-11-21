#!/usr/bin/env python3
"""Compare prebuilt `mrd` release binaries from outside the engine.

Every scenario runs the same CLI commands against each binary ("arm"), each arm
with its own HOME, cache root, daemon and copy of the fixture. Rounds are
interleaved across the arms (order reversed on odd rounds), every sample is
appended to a JSONL file, and `report.py` turns that file into tables.

    ab.py --arm before=/work/bin/mrd-aaaa --arm after=/work/bin/mrd-bbbb \
          --corpusgen /work/bin/corpusgen --root /work/run --out /work/out/run.jsonl \
          [--only read,write] [--rounds-scale 1.0] [--mem-sizes 50000,180000]

Give the arms in landing order. README.md has the scenarios and the traps.
"""

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from abcore import Arm, Sink, cpu_seconds, fanout, mem_kib  # noqa: E402

class Ctx:
    def __init__(self, args):
        self.arm_bins = []
        for spec in args.arm:
            name, _, path = spec.partition("=")
            if not name or not path or not os.access(path, os.X_OK):
                sys.exit(f"--arm wants name=/abs/path/to/mrd, got {spec!r}")
            self.arm_bins.append((name, os.path.realpath(path)))
        if len(self.arm_bins) < 2:
            sys.exit("give at least two --arm name=path, in landing order")
        self.corpusgen = args.corpusgen
        self.root = Path(args.root)
        self.fx = self.root / "_fixtures"
        self.sink = Sink(args.out)
        self.scale = args.rounds_scale
        self.fx.mkdir(parents=True, exist_ok=True)

    def rounds(self, n):
        return max(2, round(n * self.scale))

    def arms(self, scenario):
        out = []
        for name, mrd in self.arm_bins:
            state = self.root / scenario / name
            if state.exists():
                Arm(name, mrd, state).stop_daemons()
                shutil.rmtree(state)
            out.append(Arm(name, mrd, state))
        return out

    def log(self, msg):
        print(time.strftime("%H:%M:%S", time.gmtime()), msg, flush=True)


# ---------------------------------------------------------------- fixtures

def gen_vault(ctx, files):
    """Generated vault (perfsuite corpusgen, profile vault-1gb, first N files)."""
    d = ctx.fx / f"vault-{files}"
    if not (d / ".done").exists():
        subprocess.run(
            [ctx.corpusgen, "gen", "--profile", "vault-1gb",
             "--files", str(files), "--out", str(d)],
            check=True, stdout=subprocess.DEVNULL,
        )
        (d / ".done").write_text("")
    return d


def copy_tree(src, dst):
    subprocess.run(["cp", "-a", str(src), str(dst)], check=True)
    done = Path(dst) / ".done"
    if done.exists():
        done.unlink()
    (Path(dst) / ".git").mkdir(exist_ok=True)  # an empty .git anchors a workspace


def home_config(arm, mounts=()):
    body = "---\ntype: meridian-config\nversion: 1\n---\n\n# This machine\n"
    for name, path in mounts:
        body += f"\n```meridian-mount\nname: {name}\npath: {path}\n```\n"
    (arm.home / "MERIDIAN.md").write_text(body)


def find_key(obj, key):
    if isinstance(obj, dict):
        if key in obj:
            return obj[key]
        for v in obj.values():
            r = find_key(v, key)
            if r is not None:
                return r
    elif isinstance(obj, list):
        for v in obj:
            r = find_key(v, key)
            if r is not None:
                return r
    return None


def warm(arm, ws, args, timeout=900):
    """Repeat one read until the daemon (not the in-process lane) answers it."""
    t0 = time.time()
    last = None
    while time.time() - t0 < timeout:
        _, rc, out, err = arm.run(args, ws, check=False)
        last = (rc, out[-300:], err[-300:])
        if rc == 0:
            try:
                if find_key(json.loads(out), "source") == "daemon":
                    return time.time() - t0
            except ValueError:
                pass
        time.sleep(0.2)
    raise RuntimeError(f"[{arm.name}] never warm: {last}")


def interleave(arms, rounds):
    for r in range(rounds):
        yield r, (arms if r % 2 == 0 else list(reversed(arms)))


def quiet(seconds):
    time.sleep(seconds)


# ---------------------------------------------------------------- scenarios

def s_read(ctx):
    """one warm read through the CLI: 1 client, then 16 at once."""
    S = "read"
    src = gen_vault(ctx, 2000)
    arms = ctx.arms(S)
    for a in arms:
        home_config(a)
        copy_tree(src, a.state / "ws")
        warm(a, a.state / "ws", ["read", "d000/f00000.md", "--json"])
    for r, order in interleave(arms, ctx.rounds(8)):
        for a in order:
            ws = a.state / "ws"
            for i in range(100):
                w, *_ = a.run(["read", f"d000/f{i % 50:05d}.md", "--json"], ws)
                ctx.sink.put(s=S, m="wall_1client", arm=a.name, r=r, v=w)

            def client(c, a=a, ws=ws):
                return [a.run(["read", f"d000/f{(c * 7 + i) % 50:05d}.md", "--json"], ws)[0]
                        for i in range(20)]

            t0 = time.perf_counter()
            walls = fanout(16, client)
            batch = time.perf_counter() - t0
            for ws_ in walls:
                for w in ws_:
                    ctx.sink.put(s=S, m="wall_16clients", arm=a.name, r=r, v=w)
            ctx.sink.put(s=S, m="reads_per_s_16clients", arm=a.name, r=r, v=320 / batch)
        ctx.log(f"{S} round {r}")
    for a in arms:
        a.stop_daemons()


def s_links(ctx):
    """`links` on a page that links into a second declared root (20k files)."""
    S = "links"
    src = gen_vault(ctx, 20000)
    arms = ctx.arms(S)
    for a in arms:
        b = a.state / "B"
        copy_tree(src, b)
        shutil.rmtree(b / ".git")
        (b / "MERIDIAN.md").write_text(
            "---\ntype: meridian-root\nversion: 1\nname: rootb\n---\n\n# B\n")
        home_config(a, [("rootb", b)])
        ws = a.state / "A"
        (ws / ".git").mkdir(parents=True)
        (ws / "claim.md").write_text("# Claim\n\n[[rootb:d000/f00000.md]]\n[[local.md]]\n")
        (ws / "local.md").write_text("# L\n\nbody\n")
        warm(a, ws, ["read", "claim.md", "--json"])
        # let the answer settle to its steady lane (daemon or in-process)
        for _ in range(3):
            a.run(["links", "claim.md", "--json"], ws, check=False)
    for r, order in interleave(arms, ctx.rounds(6)):
        for a in order:
            ws = a.state / "A"
            t_end = time.time() + 20
            n = 0
            while n < 3 or (n < 40 and time.time() < t_end):
                w, rc, out, _ = a.run(["links", "claim.md", "--json"], ws)
                src_ = find_key(json.loads(out), "source")
                ctx.sink.put(s=S, m="wall_links", arm=a.name, r=r, v=w, source=src_)
                n += 1
        ctx.log(f"{S} round {r}")
    for a in arms:
        a.stop_daemons()


def s_bulk(ctx):
    """the first read after many files changed on disk (20k-file vault)."""
    S = "bulk"
    src = gen_vault(ctx, 20000)
    arms = ctx.arms(S)
    for a in arms:
        home_config(a)
        copy_tree(src, a.state / "ws")
        warm(a, a.state / "ws", ["read", "d000/f00000.md", "--json"])
    files = sorted(p.relative_to(src) for p in src.rglob("f*.md"))
    for movers in (2000, 200):
        for r, order in interleave(arms, ctx.rounds(8)):
            for a in order:
                ws = a.state / "ws"
                pid = a.daemon_pid()
                quiet(4)
                c0 = cpu_seconds(pid)
                for rel in files[100:100 + movers]:
                    with open(ws / rel, "a") as f:
                        f.write(f"\nedit {r} {a.name}\n")
                w, *_ = a.run(["read", "d000/f00000.md", "--json"], ws)
                quiet(1.5)
                c1 = cpu_seconds(pid)
                ctx.sink.put(s=S, m=f"wall_first_read_{movers}", arm=a.name, r=r, v=w)
                ctx.sink.put(s=S, m=f"daemon_cpu_{movers}", arm=a.name, r=r, v=c1 - c0)
            ctx.log(f"{S} movers={movers} round {r}")
    for a in arms:
        a.stop_daemons()


def s_probe(ctx):
    """`links` on a page with 2,000 links to non-markdown files in one
    20,000-entry directory, and on a root with 50 ignore rules."""
    S = "probe"
    arms = ctx.arms(S)
    for a in arms:
        home_config(a)
        ws = a.state / "ws"
        (ws / ".git").mkdir(parents=True)
        (ws / "flat").mkdir()
        for i in range(20000):
            (ws / "flat" / f"t{i}.base").write_text("x\n")
        (ws / "hub.md").write_text(
            "# Hub\n\n" + "".join(f"[[flat/t{i}.base]]\n" for i in range(2000)))
        warm(a, ws, ["read", "hub.md", "--json"])
        g = a.state / "glob"
        copy_tree(gen_vault(ctx, 20000), g)
        (g / "meridian").mkdir(exist_ok=True)
        rules = "".join(f'  - "*.zz{i:02d}q*.md"\n' for i in range(50))
        (g / "meridian" / "domain.md").write_text(f"---\nversion: 1\nignore:\n{rules}---\n")
        (g / "q.md").write_text("# Q\n\n[[nowhere]]\n")
        warm(a, g, ["read", "q.md", "--json"])
    for r, order in interleave(arms, ctx.rounds(6)):
        for a in order:
            pid = a.daemon_pid()
            for m, ws, page in (("flat_dir", a.state / "ws", "hub.md"),
                                ("ignore_rules", a.state / "glob", "q.md")):
                c0 = cpu_seconds(pid)
                n = 8
                for _ in range(n):
                    w, *_ = a.run(["links", page, "--json"], ws)
                    ctx.sink.put(s=S, m=f"wall_links_{m}", arm=a.name, r=r, v=w)
                ctx.sink.put(s=S, m=f"daemon_cpu_per_call_{m}", arm=a.name, r=r,
                             v=(cpu_seconds(pid) - c0) / n)
        ctx.log(f"{S} round {r}")
    for a in arms:
        a.stop_daemons()


def s_flight(ctx):
    """one file changes, then 16 readers arrive at once (50k-file vault)."""
    S = "flight"
    src = gen_vault(ctx, 50000)
    arms = ctx.arms(S)
    for a in arms:
        home_config(a)
        copy_tree(src, a.state / "ws")
        warm(a, a.state / "ws", ["read", "d000/f00000.md", "--json"])
    for r, order in interleave(arms, ctx.rounds(10)):
        for a in order:
            ws = a.state / "ws"
            pid = a.daemon_pid()
            quiet(4)
            c0 = cpu_seconds(pid)
            with open(ws / "d000/f00001.md", "a") as f:
                f.write(f"\nedit {r}\n")
            t0 = time.perf_counter()
            walls = fanout(16, lambda c, a=a, ws=ws: a.run(
                ["read", f"d000/f{c + 2:05d}.md", "--json"], ws)[0])
            batch = time.perf_counter() - t0
            quiet(1.5)
            c1 = cpu_seconds(pid)
            ctx.sink.put(s=S, m="daemon_cpu_edit_then_16_reads", arm=a.name, r=r, v=c1 - c0)
            ctx.sink.put(s=S, m="wall_all_16_answered", arm=a.name, r=r, v=batch)
            for w in walls:
                ctx.sink.put(s=S, m="wall_per_read", arm=a.name, r=r, v=w)
        ctx.log(f"{S} round {r}")
    for a in arms:
        a.stop_daemons()


CHECK_PAGE = """---
tags: [type/rule, rules/check]
id: bench-check-{i}
paths:
  - tasks/**
---

# bench-check-{i}

```starlark
def check_change(change):
    if change.doc.frontmatter.get("status") == "forbidden":
        refuse(message = "bench-check-{i}", passing = "other-status")
```
"""

MW_PAGE = """---
tags: [type/rule, rules/middleware]
id: bench-mw
paths:
  - tasks/**
---

# bench-mw

```starlark
def middleware(ctx):
    pass
```
"""

PUT_EDIT = '[{"target":{"fm_key":"status"},"edit":{"put":{"at":"upsert","text":"%s"}}}]'


def make_write_ws(arm, ws, checks):
    (ws / "tasks").mkdir(parents=True)
    (ws / "rules").mkdir()
    (ws / "MERIDIAN.md").write_text(
        "---\ntype: meridian-root\nversion: 1\nname: ws\n---\n\n# The workspace\n")
    (ws / "tasks/card.md").write_text("---\ntype: task\nstatus: a\n---\n\n# a card\n\nbody\n")
    big = "".join(f"## Section {i}\n\n" + ("lorem ipsum dolor sit amet " * 12 + "\n\n") * 3
                  for i in range(45))
    (ws / "tasks/big.md").write_text(f"---\ntype: task\nstatus: a\n---\n\n# a big card\n\n{big}")
    if not checks:
        return
    ids = ["bench-mw"]
    (ws / "rules/mw.md").write_text(MW_PAGE)
    for i in range(checks):
        (ws / f"rules/check-{i:02d}.md").write_text(CHECK_PAGE.format(i=f"{i:02d}"))
        ids.append(f"bench-check-{i:02d}")
    _, _, out, _ = arm.run(["rules", ".", "--json"], ws)
    table = {row["id"]: row["chain"][0]["rev"] for row in json.loads(out)["rules"]["rules"]}
    for rid in ids:
        arm.run(["arm", rid, "--mode", "block", "--rev", table[rid]], ws)


def s_write(ctx):
    """one guarded edit through the CLI, with and without armed rules."""
    S = "write"
    arms = ctx.arms(S)
    variants = (("armed10", 10, "tasks/card.md"), ("armed10_45kB_page", 10, "tasks/big.md"),
                ("no_rules", 0, "tasks/card.md"))
    for a in arms:
        home_config(a)
        for name, checks, page in variants:
            ws = a.state / name
            make_write_ws(a, ws, checks)
            warm(a, ws, ["read", page, "--json"])
            for v in "ba" * 5:
                a.run(["put", page, "--force"], ws, stdin=(PUT_EDIT % v).encode(), check=False)
    for r, order in interleave(arms, ctx.rounds(8)):
        for a in order:
            pid = a.daemon_pid()
            for name, checks, page in variants:
                ws = a.state / name
                n = 40
                c0 = cpu_seconds(pid)
                for i in range(n):
                    w, *_ = a.run(["put", page, "--force"], ws,
                                  stdin=(PUT_EDIT % "ba"[i % 2]).encode())
                    ctx.sink.put(s=S, m=f"wall_put_{name}", arm=a.name, r=r, v=w)
                ctx.sink.put(s=S, m=f"daemon_cpu_per_put_{name}", arm=a.name, r=r,
                             v=(cpu_seconds(pid) - c0) / n)
        ctx.log(f"{S} round {r}")
    for a in arms:
        a.stop_daemons()


def s_model(ctx):
    """a section read of a large page without block ids, a walk over 100
    pins to that page, and `links` over 10,000 links to an ambiguous name."""
    S = "model"
    arms = ctx.arms(S)
    para = ("lorem ipsum dolor sit amet consectetur adipiscing elit " * 10 + "\n\n") * 4
    target = "# Title\n\n" + "".join(f"## S{i}\n\n{para}" for i in range(450))  # ~1 MB
    for a in arms:
        home_config(a)
        ws = a.state / "ws"
        (ws / ".git").mkdir(parents=True)
        (ws / "T.md").write_text(target)
        warm(a, ws, ["read", "T.md", "--json"])
        rows = ""
        for i in range(100):  # one green pin per section S0..S99
            _, _, out, _ = a.run(["read", "T.md", "--section", f"Title/S{i}", "--json"], ws)
            fp = json.loads(out)["read"]["sections"][0]["fingerprint"]
            assert fp.startswith("fp1."), fp
            rows += (f'  - object: "[[T]]"\n    hash: "h"\n    path: ["Title", "S{i}"]\n'
                     f'    fingerprint: "{fp}"\n')
        (ws / "H.md").write_text(
            f"# Holder\n\nbody\n\n```meridian-lock\nversion: 2\npins:\n{rows}```\n")
        amb = a.state / "amb"
        (amb / ".git").mkdir(parents=True)
        (amb / "src").mkdir()
        for i in range(2000):
            (amb / f"d{i:04d}").mkdir()
            (amb / f"d{i:04d}" / "README.md").write_text(f"# R{i}\n\nbody\n")
        (amb / "src/hub.md").write_text(
            "# Hub\n\n" + "".join(f"[[d{i % 2000:04d}/README]] [[README]]\n" for i in range(5000)))
        warm(a, amb, ["read", "src/hub.md", "--json"])
        warm(a, ws, ["read", "H.md", "--json"])
    for r, order in interleave(arms, ctx.rounds(6)):
        for a in order:
            pid = a.daemon_pid()
            for m, ws, args, n in (
                ("section_read_1MB_page", a.state / "ws",
                 ["read", "T.md", "--section", "Title/S7", "--json"], 30),
                ("walk_100_pins", a.state / "ws", ["walk", "H.md", "--json"], 10),
                ("links_10k_ambiguous", a.state / "amb", ["links", "src/hub.md", "--json"], 10),
            ):
                c0 = cpu_seconds(pid)
                for _ in range(n):
                    w, rc, *_ = a.run(args, ws, check=False)
                    ctx.sink.put(s=S, m=f"wall_{m}", arm=a.name, r=r, v=w, rc=rc)
                ctx.sink.put(s=S, m=f"daemon_cpu_per_call_{m}", arm=a.name, r=r,
                             v=(cpu_seconds(pid) - c0) / n)
        ctx.log(f"{S} round {r}")
    for a in arms:
        a.stop_daemons()


class OwnDaemon:
    """A daemon started as our own child, so its exit status carries its CPU and
    peak memory and its stderr is ours to read."""

    def __init__(self, arm, extra_env=None):
        self.arm = arm
        self.log = arm.state / f"daemon-{time.time_ns()}.log"
        self.t0 = time.perf_counter()
        self.p = subprocess.Popen(
            [arm.mrd, "daemon"], cwd=str(arm.state), env=arm.env(extra_env),
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=open(self.log, "wb"))

    def wait_log(self, needle, count=1, timeout=600):
        t0 = time.time()
        while time.time() - t0 < timeout:
            if self.log.read_bytes().count(needle.encode()) >= count:
                return time.perf_counter() - self.t0
            if self.p.poll() is not None:
                raise RuntimeError(f"daemon died: {self.log.read_bytes()[-500:]!r}")
            time.sleep(0.02)
        raise RuntimeError(f"no {count}x {needle!r} in {self.log}")

    def stop(self):
        t0 = time.perf_counter()
        self.p.send_signal(signal.SIGTERM)
        _, status, ru = os.wait4(self.p.pid, 0)
        self.p.returncode = status
        return {"stop_wall": time.perf_counter() - t0, "cpu": ru.ru_utime + ru.ru_stime,
                "maxrss_kib": ru.ru_maxrss}


def s_cache(ctx):
    """restart on a 50k-file vault whose parse cache is on disk: time to the
    first answer, to the re-saved cache, and the daemon's CPU for the cycle.
    Default mode and residency-host mode (the mode that carries records)."""
    S = "cache"
    src = gen_vault(ctx, 50000)
    arms = ctx.arms(S)
    modes = (("default", {}), ("residency_host", {"MRD_RESIDENCY_HOST_BYTES": str(16 << 30)}))
    for a in arms:
        home_config(a)
        for mode, env in modes:
            ws = a.state / f"ws-{mode}"
            copy_tree(src, ws)
            d = OwnDaemon(a, env)
            warm(a, ws, ["read", "d000/f00000.md", "--json"])
            d.wait_log("parse-cache: saved")
            d.stop()
    for r, order in interleave(arms, ctx.rounds(6)):
        for a in order:
            for mode, env in modes:
                ws = a.state / f"ws-{mode}"
                with open(ws / "d000/f00001.md", "a") as f:
                    f.write(f"\nedit {r}\n")
                d = OwnDaemon(a, env)
                t_first = warm(a, ws, ["read", "d000/f00000.md", "--json"])
                t_saved = d.wait_log("parse-cache: saved")
                st = d.stop()
                ctx.sink.put(s=S, m=f"wall_restart_to_first_answer_{mode}", arm=a.name, r=r, v=t_first)
                ctx.sink.put(s=S, m=f"wall_restart_to_cache_saved_{mode}", arm=a.name, r=r, v=t_saved)
                ctx.sink.put(s=S, m=f"daemon_cpu_restart_cycle_{mode}", arm=a.name, r=r, v=st["cpu"])
                ctx.sink.put(s=S, m=f"wall_shutdown_{mode}", arm=a.name, r=r, v=st["stop_wall"])
        ctx.log(f"{S} round {r}")


def gen_headings(ctx, files):
    d = ctx.fx / f"headings-{files}"
    if not (d / ".done").exists():
        d.mkdir(parents=True, exist_ok=True)
        for i in range(files):
            sub = d / f"d{i // 500:03d}"
            sub.mkdir(exist_ok=True)
            body = "".join(f"## Topic {i} part {j}\n\nOne line under it.\n\n" for j in range(40))
            (sub / f"n{i:05d}.md").write_text(f"# Note {i}\n\n{body}")
        (d / ".done").write_text("")
    return d


def s_mem(ctx, sizes=(50000,)):
    """cold start with an empty cache: daemon memory once warm, time and CPU."""
    S = "mem"
    arms = ctx.arms(S)
    corpora = [(f"vault_{n // 1000}k", gen_vault(ctx, n)) for n in sizes]
    corpora.append(("headings_20k", gen_headings(ctx, 20000)))
    pages = {name: first_page(src) for name, src in corpora}
    for a in arms:
        home_config(a)
        for name, src in corpora:
            copy_tree(src, a.state / name)
    for name, _ in corpora:
        for r, order in interleave(arms, ctx.rounds(4)):
            for a in order:
                a.wipe_cache()
                env = {"MRD_MAX_RESIDENT_BYTES": "0"}
                d = OwnDaemon(a, env)
                t_warm = warm(a, a.state / name, ["read", pages[name], "--json"])
                d.wait_log("parse-cache: saved")
                quiet(1.0)
                mem = mem_kib(d.p.pid)
                st = d.stop()
                ctx.sink.put(s=S, m=f"wall_cold_start_{name}", arm=a.name, r=r, v=t_warm)
                ctx.sink.put(s=S, m=f"rss_mib_warm_{name}", arm=a.name, r=r, v=mem["VmRSS"] / 1024)
                ctx.sink.put(s=S, m=f"rss_anon_mib_warm_{name}", arm=a.name, r=r,
                             v=mem.get("RssAnon", 0) / 1024)
                ctx.sink.put(s=S, m=f"peak_rss_mib_{name}", arm=a.name, r=r,
                             v=st["maxrss_kib"] / 1024)
                ctx.sink.put(s=S, m=f"daemon_cpu_cold_cycle_{name}", arm=a.name, r=r, v=st["cpu"])
            ctx.log(f"{S} {name} round {r}")


def first_page(ws):
    for p in sorted(ws.rglob("*.md")):
        return str(p.relative_to(ws))
    raise RuntimeError(f"no page in {ws}")


def stop_all(ctx):
    """End every process running one of the arms' binaries. The driver is meant
    to run alone in its container, so all of them are its own."""
    mine = {mrd for _, mrd in ctx.arm_bins}
    for d in os.listdir("/proc"):
        if d.isdigit():
            try:
                if os.readlink(f"/proc/{d}/exe") in mine:
                    os.kill(int(d), signal.SIGTERM)
            except OSError:
                pass
    time.sleep(1.0)


SCENARIOS = {
    "read": s_read, "links": s_links, "bulk": s_bulk, "probe": s_probe,
    "flight": s_flight, "write": s_write, "model": s_model, "cache": s_cache, "mem": s_mem,
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm", action="append", default=[], metavar="NAME=PATH",
                    help="one prebuilt mrd binary; repeat, in landing order")
    ap.add_argument("--corpusgen", required=True, help="perfsuite's corpusgen binary")
    ap.add_argument("--root", required=True, help="scratch dir for fixtures and arm state")
    ap.add_argument("--out", required=True, help="JSONL of samples, appended")
    ap.add_argument("--only", default="", help="comma-separated scenarios")
    ap.add_argument("--rounds-scale", type=float, default=1.0)
    ap.add_argument("--mem-sizes", default="50000", help="vault sizes for the mem scenario")
    args = ap.parse_args()
    ctx = Ctx(args)
    names = args.only.split(",") if args.only else list(SCENARIOS)
    env = {"argv": sys.argv[1:], "uname": " ".join(os.uname()),
           "cpus_allowed": sorted(os.sched_getaffinity(0)),
           "cpu_model": next((l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo")
                              if l.startswith("model name")), "?"),
           "arms": [name for name, _ in ctx.arm_bins], "binaries": {}}
    for name, mrd in ctx.arm_bins:
        env["binaries"][name] = {
            "version": subprocess.run([mrd, "--version"], capture_output=True,
                                      text=True).stdout.strip(),
            "sha256": subprocess.run(["sha256sum", mrd], capture_output=True,
                                     text=True).stdout.split()[0]}
    ctx.sink.put(s="", m="_env", arm="", r=-1, v=0, env=env)
    for n in names:
        t0 = time.time()
        ctx.log(f"scenario {n} start")
        ctx.sink.put(s=n, m="_start", arm="", r=-1, v=0, loadavg=open("/proc/loadavg").read().split()[:3])
        try:
            if n == "mem":
                s_mem(ctx, tuple(int(x) for x in args.mem_sizes.split(",")))
            else:
                SCENARIOS[n](ctx)
            ctx.sink.put(s=n, m="_done", arm="", r=-1, v=time.time() - t0,
                         loadavg=open("/proc/loadavg").read().split()[:3])
            ctx.log(f"scenario {n} done in {time.time() - t0:.0f}s")
        except Exception as e:  # keep going: one broken scenario must not lose the rest
            ctx.sink.put(s=n, m="_error", arm="", r=-1, v=time.time() - t0, err=repr(e)[:2000])
            ctx.log(f"scenario {n} FAILED after {time.time() - t0:.0f}s: {e!r}"[:3000])
        finally:
            stop_all(ctx)


if __name__ == "__main__":
    main()
