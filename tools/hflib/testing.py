"""`hf test <tier>`: the tiers `hf check` never runs on its own.

  disposable  starts the nested Hyprland, runs the tests that change a
              compositor against it, and takes it down again. Records a
              stamp on success, which `hf release` asks for.
  root        builds as you, then runs only the test binaries as root,
              through cargo's target runner — so `target/` never gains a
              root-owned file and the next unprivileged build still works.
  manual      screenshot tools and benchmarks; pass a filter after `--`.
  live, visible, unit
              the same tiers `hf check` runs, alone.
"""

import json
import os
import subprocess
import time

from . import hyprland, nextest, tiers, ui
from .run import ROOT, run

STAMPS = ROOT / "target" / "hf" / "stamps"

# What the disposable tier vouches for: the crates whose catalogues and
# generators it checks against a real compositor. A release is refused
# when any of these changed since the stamp was written.
DISPOSABLE_COVERS = [
    "crates/hyprforge-appearance",
    "crates/hyprforge-input",
    "crates/hyprforge-system",
    "crates/hyprforge-session",
    "crates/hyprforge-shortcuts",
    "crates/hyprforge-windowrules",
    "crates/hyprforge-ecosystem",
    "crates/hyprforge-core",
    "crates/hyprforge-files",
]


def coverage_fingerprint() -> dict:
    """Tree hash of each covered path at HEAD, plus whether any is dirty."""
    trees = {}
    for path in DISPOSABLE_COVERS:
        trees[path] = run(["git", "rev-parse", f"HEAD:{path}"]).out.strip()
    dirty = run(["git", "status", "--porcelain", "--", *DISPOSABLE_COVERS]).out.strip() != ""
    return {"trees": trees, "dirty": dirty}


def disposable_stamp_is_fresh() -> tuple[bool, str]:
    stamp = STAMPS / "disposable.json"
    if not stamp.is_file():
        return False, "the disposable tier has never passed on this machine — run `tools/hf test disposable`"
    recorded = json.loads(stamp.read_text())
    now = coverage_fingerprint()
    if now["dirty"]:
        return False, "the crates the disposable tier covers have uncommitted changes"
    if recorded.get("dirty"):
        return False, "the last disposable pass was on uncommitted changes — run it again on a commit"
    changed = [p for p, t in now["trees"].items() if recorded["trees"].get(p) != t]
    if changed:
        return False, f"changed since the disposable tier last passed: {', '.join(changed)} — run `tools/hf test disposable`"
    return True, f"passed {time.strftime('%Y-%m-%d %H:%M', time.localtime(recorded['at']))}"


def _disposable(keep: bool, unattended_ok: bool) -> int:
    ui.step("Disposable tier: tests that change a compositor, against a nested one")
    s = hyprland.up(unattended_ok=unattended_ok)
    try:
        nextest.mark_start()
        outcome = nextest.run_tier("disposable", ignored=True, env=hyprland.env_for(s))
        nextest.report(outcome, "disposable", ui)
        if outcome.finished and outcome.tests and not outcome.failed:
            STAMPS.mkdir(parents=True, exist_ok=True)
            (STAMPS / "disposable.json").write_text(json.dumps({**coverage_fingerprint(), "at": time.time()}, indent=2))
    finally:
        if not keep:
            hyprland.down()
    return ui.summary()


def _root(extra: list[str]) -> int:
    ui.step("Root tier: tests that need root, run as root")
    # Credentials first, while there is a terminal to ask on: nextest runs
    # each binary with no stdin to prompt through.
    if subprocess.run(["sudo", "-v"]).returncode != 0:
        ui.bad("sudo refused")
        return ui.summary()
    runner = "sudo --non-interactive --preserve-env"
    env = {"CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER": runner}
    nextest.mark_start()
    outcome = nextest.run_tier("root", ignored=True, env=env, extra=extra)
    nextest.report(outcome, "root", ui)
    # The recorder's own proof, inside its unit's sandbox — root's too.
    from . import sandbox

    sandbox.self_tests(allow_sudo=True)
    return ui.summary()


def test(tier: str, *, keep: bool = False, unattended_ok: bool = False, extra: list[str] | None = None) -> int:
    extra = extra or []
    if tier == "disposable":
        return _disposable(keep, unattended_ok)
    if tier == "root":
        return _root(extra)
    if tier == "unit":
        ui.step("Unit tier")
        nextest.mark_start()
        nextest.report(nextest.run_tier("unit", ignored=False, extra=extra), "unit", ui)
        return ui.summary()
    if tier in ("live", "visible"):
        from .checks import live_tiers

        live_tiers()
        return ui.summary()
    if tier == "manual":
        ui.step("Manual tier: screenshot tools and benchmarks")
        if not extra:
            ui.note("these are tools, not checks; name one: tools/hf test manual -- -E 'test(render_the_card)'")
            for name in nextest.listing("manual", ignored=True):
                print(f"    {name}")
            return 0
        # Straight through, output and all: a benchmark's number is the point.
        cmd = ["cargo", "nextest", "run", *nextest.base_args(), "-P", "manual", "--run-ignored", "only",
               "--no-capture", *extra]
        return subprocess.run(cmd, cwd=ROOT, env={**os.environ, "CARGO_BUILD_JOBS": os.environ.get("CARGO_BUILD_JOBS", "4")}).returncode
    ui.die(f"no tier called {tier!r}; tiers: unit, {', '.join(tiers.IGNORED_TIERS)}")
