#!/usr/bin/env python3
"""Pipelined workspace build + parallel test binary runner for meridian-rs CI.

Why this exists (and what invariants it preserves):
  - `cargo test --workspace` compiles all ~358 test binaries, then executes them
    ONE BINARY AT A TIME sequentially. On an 8-core `tier: rs` VM, a single
    slow test (`view::store::escalation_tests::reads_ending_across_raises_never_leave_the_wrong_limit`,
    ~248s) and several 15-63s timer/load binaries serialized behind all other
    binaries, pushing the step to 925-1,526s even at 100% sccache hit rate.
  - This runner performs ONE `cargo test --workspace --locked --no-run
    --message-format=json-render-diagnostics` build (preserving exact workspace
    feature unification and sccache keys) and streams its artifact JSON:
      1. The instant `view::lib` finishes linking (before the ~160 heavy
         `mrd`/`registry` integration test binaries link), it spawns `view::lib`
         in the background so its ~248s run overlaps with linking.
      2. Once `--no-run` exits (and prints the cgroup link-phase memory stats),
         it runs ALL remaining dynamically-discovered test executables AND
         `cargo test --workspace --locked --doc --no-fail-fast -q` across a
         bounded worker pool (longest-job-first ordering hint; ZERO hardcoded
         filtering, so any future crate or test target is automatically run).
      3. `testsuite` binaries are serialized with each other via a lock so the
         11 tracked `crates/testsuite/data/parity/corpus/*/.meridian/stable-read.probe`
         files are never truncated concurrently.
      4. Preserves `tools/test.sh` selector clearing, `--no-fail-fast` semantics,
         `target-ci/suite.log`, and the `evidence lines` > 0 gate.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import pathlib
import subprocess
import sys
import threading
import time
from typing import Any

# Mirror tools/test.sh: clear inherited binary selectors so tests always run
# against the checkout's freshly built engine.
CLEARED_SELECTORS = (
    "CCC_MRD_BIN",
    "MERIDIAN_MRD_BIN",
    "MERIDIAN_DAEMON_BIN",
    "MRD_BIN",
)

# Ordering hint ONLY (never a filter): launch known slowest binaries first so
# they never form a late serial tail. All other dynamically-discovered targets
# run right after them with priority 100.
PRIORITY_HINTS: dict[tuple[str, str, str], int] = {
    ("view", "view", "lib"): 0,
    ("registry", "registry", "lib"): 1,
    ("registry", "load_peak", "test"): 2,
    ("registry", "policy_door_linear_load", "test"): 3,
    ("registry", "sub_push", "test"): 4,
    ("mrd", "mrd", "lib"): 5,
    ("cache", "drawer_lock_release", "test"): 6,
    ("testsuite", "main", "test"): 7,
    ("mrd", "sql_e2e", "test"): 8,
    ("wire-serve", "door_linear_load", "test"): 9,
    ("registry", "sql_op", "test"): 10,
}

# Target eligible to start immediately during `--no-run` as soon as Cargo emits
# its linked executable (overlaps its ~248s run with linking the remaining ~160
# `mrd`/`registry` integration test binaries without adding heavy RAM load).
EARLY_START_TARGETS = {
    ("view", "view", "lib"),
}


class MemorySampler:
    """1-second cgroup v2 anon peak + host MemAvailable low-water sampler."""

    def __init__(self) -> None:
        self._stop = threading.Event()
        self.peak_anon_bytes = 0
        self.min_mem_avail_kb = 0
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._thread.join(timeout=2.0)

    def _sample_once(self) -> None:
        try:
            with open("/sys/fs/cgroup/memory.stat", encoding="utf-8") as f:
                for line in f:
                    if line.startswith("anon "):
                        val = int(line.split()[1])
                        if val > self.peak_anon_bytes:
                            self.peak_anon_bytes = val
                        break
        except OSError:
            pass
        try:
            with open("/proc/meminfo", encoding="utf-8") as f:
                for line in f:
                    if line.startswith("MemAvailable:"):
                        val = int(line.split()[1])
                        if self.min_mem_avail_kb == 0 or val < self.min_mem_avail_kb:
                            self.min_mem_avail_kb = val
                        break
        except OSError:
            pass

    def _run(self) -> None:
        while not self._stop.is_set():
            self._sample_once()
            self._stop.wait(1.0)

    def report(self, rc: int) -> None:
        self._sample_once()
        mem_total_kb = 0
        try:
            with open("/proc/meminfo", encoding="utf-8") as f:
                for line in f:
                    if line.startswith("MemTotal:"):
                        mem_total_kb = int(line.split()[1])
                        break
        except OSError:
            pass

        mem_peak_bytes = 0
        try:
            raw = pathlib.Path("/sys/fs/cgroup/memory.peak").read_text(encoding="utf-8").strip()
            mem_peak_bytes = int(raw)
        except (OSError, ValueError):
            pass

        oom_kill = "0"
        try:
            with open("/sys/fs/cgroup/memory.events", encoding="utf-8") as f:
                for line in f:
                    if line.startswith("oom_kill "):
                        oom_kill = line.split()[1]
                        break
        except OSError:
            pass

        def gib(b: int) -> str:
            return f"{b / 1073741824:.2f} GiB"

        print(f"link step memory (cargo test --workspace --no-run, exit {rc}):", flush=True)
        print(f"  container anon peak (1 s samples): {gib(self.peak_anon_bytes)}", flush=True)
        print(f"  container memory.peak (incl. page cache): {gib(mem_peak_bytes)}", flush=True)
        print(
            f"  machine MemAvailable low: {gib(self.min_mem_avail_kb * 1024)} "
            f"of MemTotal {gib(mem_total_kb * 1024)}",
            flush=True,
        )
        print(f"  container oom_kill: {oom_kill}", flush=True)


def parse_pkg_name(package_id: str) -> str:
    # Handles both Cargo v2 (`path+file:///.../crates/view#1.2.0`) and legacy formats.
    tail = package_id.rsplit("/", 1)[-1]
    if "#" in tail:
        left, right = tail.split("#", 1)
        if "@" in right:
            return right.split("@", 1)[0]
        return left
    return tail.split()[0]


def clear_inherited_selectors() -> None:
    for selector in CLEARED_SELECTORS:
        if selector in os.environ:
            print(f"test environment: clearing inherited {selector}", file=sys.stderr, flush=True)
            del os.environ[selector]


def main() -> int:
    parser = argparse.ArgumentParser(description="Pipelined parallel workspace test runner")
    parser.add_argument("--suite-log", default="target-ci/suite.log")
    parser.add_argument(
        "--workers",
        type=int,
        default=int(
            os.environ.get("CI_TEST_WORKERS")
            or max(2, min(6, (os.cpu_count() or 4) // 2))
        ),
    )
    parser.add_argument(
        "--test-threads",
        type=int,
        default=int(os.environ.get("CI_TEST_THREADS") or 4),
    )
    args = parser.parse_args()

    clear_inherited_selectors()

    t_overall = time.monotonic()
    target_dir = os.environ.get("CARGO_TARGET_DIR", "target")
    pathlib.Path(target_dir, "tmp").mkdir(parents=True, exist_ok=True)
    pathlib.Path(args.suite_log).parent.mkdir(parents=True, exist_ok=True)

    sampler = MemorySampler()
    sampler.start()

    build_cmd = [
        "cargo",
        "test",
        "--workspace",
        "--locked",
        "--no-run",
        "--message-format=json-render-diagnostics",
    ]
    build_proc = subprocess.Popen(
        build_cmd,
        stdout=subprocess.PIPE,
        stderr=None,  # stream compiler diagnostics directly to CI log
        text=True,
        bufsize=1,
    )

    discovered: list[dict[str, Any]] = []
    seen_exes: set[str] = set()
    early_procs: list[tuple[dict[str, Any], subprocess.Popen[str], float, list[str], threading.Thread]] = []
    early_exes: set[str] = set()

    assert build_proc.stdout is not None
    for raw_line in build_proc.stdout:
        line = raw_line.strip()
        if not line.startswith("{"):
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (
            msg.get("reason") == "compiler-artifact"
            and msg.get("executable")
            and msg.get("profile", {}).get("test")
        ):
            exe = msg["executable"]
            if exe in seen_exes:
                continue
            seen_exes.add(exe)
            target = msg.get("target") or {}
            kinds = target.get("kind") or ["test"]
            kind = kinds[0]
            pkg = parse_pkg_name(msg.get("package_id", ""))
            tgt_name = target.get("name", pathlib.Path(exe).name)
            mdir = str(pathlib.Path(msg["manifest_path"]).parent)
            item = {
                "pkg": pkg,
                "target": tgt_name,
                "kind": kind,
                "exe": exe,
                "manifest_dir": mdir,
            }
            discovered.append(item)

            if (pkg, tgt_name, kind) in EARLY_START_TARGETS:
                env = os.environ.copy()
                env["CARGO_MANIFEST_DIR"] = mdir
                env["CARGO_PKG_NAME"] = pkg
                cmd = [exe, "--quiet", f"--test-threads={args.test_threads}"]
                p = subprocess.Popen(
                    cmd,
                    cwd=mdir,
                    env=env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
                # Drain stdout/stderr continuously in a background thread so an
                # early-started target can never stall on a full 64 KiB pipe buffer.
                buf: list[str] = []
                def _drain(proc: subprocess.Popen[str] = p, out_buf: list[str] = buf) -> None:
                    assert proc.stdout is not None
                    out_buf.append(proc.stdout.read())
                drainer = threading.Thread(target=_drain, daemon=True)
                drainer.start()
                early_procs.append((item, p, time.monotonic(), buf, drainer))
                early_exes.add(exe)
                elapsed_build = time.monotonic() - t_overall
                print(
                    f"[parallel-test] early-started {pkg}::{tgt_name} ({kind}) "
                    f"at t+{elapsed_build:.1f}s (artifact #{len(discovered)})",
                    flush=True,
                )

    build_rc = build_proc.wait()
    sampler.stop()
    sampler.report(build_rc)
    t_build_done = time.monotonic()

    if build_rc != 0:
        for _, p, _, _, drainer in early_procs:
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    p.kill()
            drainer.join(timeout=2)
        pathlib.Path(args.suite_log).write_text("", encoding="utf-8")
        print("evidence lines: 0", flush=True)
        print(
            "NO MEASUREMENT — this red is not a test verdict; the build died before any test ran",
            flush=True,
        )
        return build_rc

    remaining = [item for item in discovered if item["exe"] not in early_exes]
    remaining.sort(
        key=lambda it: (
            PRIORITY_HINTS.get((it["pkg"], it["target"], it["kind"]), 100),
            it["pkg"],
            it["target"],
        )
    )

    print(
        f"[parallel-test] build finished in {t_build_done - t_overall:.1f}s; "
        f"running {len(remaining)} remaining test binaries + doctests "
        f"(workers={args.workers}, test_threads={args.test_threads}, "
        f"early_in_flight={len(early_procs)})",
        flush=True,
    )

    testsuite_lock = threading.Lock()

    def run_binary(item: dict[str, Any]) -> dict[str, Any]:
        t0 = time.monotonic()
        env = os.environ.copy()
        env["CARGO_MANIFEST_DIR"] = item["manifest_dir"]
        env["CARGO_PKG_NAME"] = item["pkg"]
        cmd = [item["exe"], "--quiet", f"--test-threads={args.test_threads}"]
        if item["pkg"] == "testsuite":
            with testsuite_lock:
                proc = subprocess.run(
                    cmd,
                    cwd=item["manifest_dir"],
                    env=env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                )
        else:
            proc = subprocess.run(
                cmd,
                cwd=item["manifest_dir"],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            )
        dt = time.monotonic() - t0
        return {
            "label": f"{item['pkg']}::{item['target']} ({item['kind']})",
            "rc": proc.returncode,
            "dt": dt,
            "out": proc.stdout,
        }

    def run_doctests() -> dict[str, Any]:
        t0 = time.monotonic()
        proc = subprocess.run(
            ["cargo", "test", "--workspace", "--locked", "--doc", "--no-fail-fast", "-q"],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        dt = time.monotonic() - t0
        return {
            "label": "workspace::doctests (doc)",
            "rc": proc.returncode,
            "dt": dt,
            "out": proc.stdout,
        }

    results: list[dict[str, Any]] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futs = [pool.submit(run_binary, it) for it in remaining]
        futs.append(pool.submit(run_doctests))
        for fut in concurrent.futures.as_completed(futs):
            results.append(fut.result())

    # Collect early-started background targets (e.g. view::lib).
    for item, p, t0, buf, drainer in early_procs:
        rc = p.wait()
        drainer.join()
        dt = time.monotonic() - t0
        results.append(
            {
                "label": f"{item['pkg']}::{item['target']} ({item['kind']})",
                "rc": rc,
                "dt": dt,
                "out": "".join(buf),
            }
        )

    t_done = time.monotonic()

    # Write suite.log preserving every `test result:` evidence line and full
    # output of any failing target.
    evidence_count = 0
    failures: list[dict[str, Any]] = []
    with open(args.suite_log, "w", encoding="utf-8") as logf:
        for res in results:
            for ln in res["out"].splitlines():
                if ln.startswith("test result:"):
                    evidence_count += 1
                    logf.write(ln + "\n")
            if res["rc"] != 0:
                failures.append(res)
                logf.write(f"\n=== FAIL {res['label']} (exit {res['rc']}) ===\n")
                logf.write(res["out"])
                if not res["out"].endswith("\n"):
                    logf.write("\n")

    print(
        f"[parallel-test] suite finished in {t_done - t_overall:.1f}s total "
        f"(build {t_build_done - t_overall:.1f}s, post-build tail {t_done - t_build_done:.1f}s, "
        f"targets {len(discovered)} binaries + doctests)",
        flush=True,
    )
    print("[parallel-test] top 10 slowest targets:", flush=True)
    for res in sorted(results, key=lambda r: r["dt"], reverse=True)[:10]:
        status = "ok" if res["rc"] == 0 else f"FAIL({res['rc']})"
        print(f"  {res['dt']:>7.2f}s  {status:<8}  {res['label']}", flush=True)

    if failures:
        for res in failures:
            print(f"\n=== FAIL {res['label']} (exit {res['rc']}, {res['dt']:.2f}s) ===", flush=True)
            print(res["out"], flush=True)

    print(f"evidence lines: {evidence_count}", flush=True)

    if failures:
        if evidence_count == 0:
            print(
                "NO MEASUREMENT — this red is not a test verdict; the build died before any test ran",
                flush=True,
            )
        return 1
    if evidence_count <= 0:
        print("NO MEASUREMENT — green exit with zero evidence lines", flush=True)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
