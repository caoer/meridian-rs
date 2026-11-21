"""Shared harness for the mrd before/after benchmark.

One Arm = one prebuilt `mrd` binary with its own HOME and XDG_CACHE_HOME, so
each binary talks only to the daemon it spawned itself. Scenarios call
`arm.run(...)`, which times one CLI process from fork to exit.
"""

import json
import os
import signal
import statistics
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

CLK_TCK = os.sysconf("SC_CLK_TCK")
PAGE = os.sysconf("SC_PAGE_SIZE")


class Arm:
    def __init__(self, name: str, mrd: str, state: Path):
        self.name = name
        self.mrd = str(mrd)
        self.state = Path(state)  # per-arm scratch: home/ and xdg/
        self.home = self.state / "home"
        self.xdg = self.state / "xdg"
        (self.home / ".config").mkdir(parents=True, exist_ok=True)
        self.xdg.mkdir(parents=True, exist_ok=True)

    def env(self, extra=None):
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(self.home),
            "XDG_CACHE_HOME": str(self.xdg),
            "XDG_CONFIG_HOME": str(self.home / ".config"),
            "MERIDIAN_CONFIG": str(self.home / "MERIDIAN.md"),
            "LANG": "C.UTF-8",
        }
        if extra:
            env.update(extra)
        return env

    def run(self, args, cwd, stdin=None, check=True, extra_env=None):
        """One CLI invocation. Returns (wall_seconds, returncode, stdout, stderr)."""
        t0 = time.perf_counter()
        p = subprocess.run(
            [self.mrd, *args],
            cwd=str(cwd),
            env=self.env(extra_env),
            input=stdin,
            capture_output=True,
        )
        wall = time.perf_counter() - t0
        if check and p.returncode != 0:
            raise RuntimeError(
                f"[{self.name}] mrd {' '.join(args)} exited {p.returncode}\n"
                f"stdout: {p.stdout[-800:]!r}\nstderr: {p.stderr[-800:]!r}"
            )
        return wall, p.returncode, p.stdout, p.stderr

    def daemons(self):
        """PIDs whose executable is this arm's binary and whose XDG_CACHE_HOME is
        this arm's — the daemon(s) this arm spawned."""
        want = f"XDG_CACHE_HOME={self.xdg}".encode()
        found = []
        for d in os.listdir("/proc"):
            if not d.isdigit():
                continue
            try:
                if os.readlink(f"/proc/{d}/exe") != self.mrd:
                    continue
                with open(f"/proc/{d}/environ", "rb") as f:
                    if want not in f.read().split(b"\0"):
                        continue
                with open(f"/proc/{d}/cmdline", "rb") as f:
                    cmd = f.read().replace(b"\0", b" ").decode(errors="replace")
            except OSError:
                continue
            found.append((int(d), cmd.strip()))
        return found

    def daemon_pid(self):
        ds = self.daemons()
        if len(ds) != 1:
            raise RuntimeError(f"[{self.name}] expected one daemon, found {ds}")
        return ds[0][0]

    def stop_daemons(self, wait=20.0):
        for pid, _ in self.daemons():
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.time() + wait
        while self.daemons() and time.time() < deadline:
            time.sleep(0.05)
        for pid, _ in self.daemons():
            os.kill(pid, signal.SIGKILL)
        time.sleep(0.1)

    def wipe_cache(self):
        self.stop_daemons()
        subprocess.run(["rm", "-rf", str(self.xdg)], check=True)
        self.xdg.mkdir(parents=True, exist_ok=True)


def cpu_seconds(pid: int) -> float:
    """user+sys CPU of one process, all threads, from /proc/<pid>/stat."""
    with open(f"/proc/{pid}/stat") as f:
        rest = f.read().rsplit(")", 1)[1].split()
    return (int(rest[11]) + int(rest[12])) / CLK_TCK


def mem_kib(pid: int) -> dict:
    out = {}
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            k, _, v = line.partition(":")
            if k in ("VmRSS", "VmHWM", "RssAnon", "RssFile"):
                out[k] = int(v.split()[0])
    return out


def pctl(xs, q):
    xs = sorted(xs)
    if not xs:
        return float("nan")
    i = (len(xs) - 1) * q
    lo, hi = int(i), min(int(i) + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (i - lo)


def summarize(xs):
    return {
        "n": len(xs),
        "median": statistics.median(xs),
        "p10": pctl(xs, 0.10),
        "p90": pctl(xs, 0.90),
        "min": min(xs),
        "max": max(xs),
    }


def fanout(n_clients, fn):
    """Run fn(i) on n_clients threads at once; returns the list of results."""
    with ThreadPoolExecutor(max_workers=n_clients) as ex:
        return list(ex.map(fn, range(n_clients)))


class Sink:
    """Append-only JSONL of every sample, so a crash loses nothing."""

    def __init__(self, path: Path):
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.f = open(self.path, "a", buffering=1)

    def put(self, **row):
        row["t"] = time.time()
        self.f.write(json.dumps(row, separators=(",", ":")) + "\n")
