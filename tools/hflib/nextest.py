"""Running the tiers through cargo-nextest, and reading what happened.

Every invocation is `--workspace` with the same features, so cargo
resolves one feature set and builds one set of test binaries that every
tier reuses. check.sh ran `-p <crate>` per live step, and each `-p`
resolved features for that crate alone — a partial rebuild per step.

A result is read from three places, because each can lie alone: the
exit status (a killed run prints no failure line), the JUnit report's
count (zero tests is never a pass), and each passing test's stored
stderr (a skipped check prints HYPRFORGE-SKIP and returns early, which
libtest calls a pass).
"""

import json
import re
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path

from . import tiers
from .run import BUILD, ROOT, Result, cargo_env, run

SKIP_MARKER = re.compile(r"HYPRFORGE-SKIP:\s*([^\n(]*)")


@dataclass
class Outcome:
    result: Result
    tests: int = 0
    failed: list[tuple[str, str]] = field(default_factory=list)  # (name, output)
    skips: list[tuple[str, str]] = field(default_factory=list)  # (name, reason)

    @property
    def finished(self) -> bool:
        # nextest exits 100 when tests failed, which is still a run that
        # finished and has a report to read.
        return self.result.code in (0, 100) and not self.result.timed_out


def base_args(workspace: Path | None = None) -> list[str]:
    if workspace is None:
        return ["--workspace", *tiers.feature_args()]
    # notif: its own workspace, none of the suite's features.
    return ["--workspace"]


def listing(profile: str, ignored: bool, expr: str | None = None, workspace: Path | None = None) -> list[str]:
    """`binary-id test-name` for every test the profile (and expr) selects."""
    cmd = ["cargo", "nextest", "list", *base_args(workspace), "-P", profile, "--message-format", "json"]
    if ignored:
        cmd += ["--run-ignored", "only"]
    if expr:
        cmd += ["-E", expr]
    r = run(cmd, cwd=workspace or ROOT, env=cargo_env(), timeout=BUILD)
    if not r.ok:
        raise RuntimeError(f"nextest list failed:\n{r.out[-2000:]}")
    # The JSON is the last line; cargo's build chatter comes before it.
    doc = json.loads(r.out.strip().splitlines()[-1])
    names = []
    for suite in doc["rust-suites"].values():
        for name, case in suite["testcases"].items():
            if case["filter-match"]["status"] == "matches":
                names.append(f"{suite['binary-id']} {name}")
    return sorted(names)


def run_tier(
    profile: str,
    *,
    ignored: bool,
    expr: str | None = None,
    env: dict | None = None,
    workspace: Path | None = None,
    extra: list[str] | None = None,
    prefix: list[str] | None = None,
) -> Outcome:
    cmd = [*(prefix or []), "cargo", "nextest", "run", *base_args(workspace), "-P", profile,
           "--no-fail-fast", "--color", "never", "--status-level", "fail", "--final-status-level", "fail"]
    if ignored:
        cmd += ["--run-ignored", "only"]
    if expr:
        cmd += ["-E", expr]
    cmd += extra or []
    full_env = {**cargo_env(), **(env or {})}
    r = run(cmd, cwd=workspace or ROOT, env=full_env, timeout=BUILD)
    outcome = Outcome(r)
    report = (workspace or ROOT) / "target" / "nextest" / profile / "junit.xml"
    if report.is_file() and report.stat().st_mtime >= _started_at(r):
        _read_junit(report, outcome)
    return outcome


_RUN_STARTED = {}


def _started_at(result: Result) -> float:
    # A report older than this run belongs to an earlier one and must
    # not be read as this run's result. Recorded by mark_start().
    return _RUN_STARTED.get("t", 0.0)


def mark_start() -> None:
    import time

    _RUN_STARTED["t"] = time.time() - 1


def _read_junit(path: Path, outcome: Outcome) -> None:
    root = ET.parse(path).getroot()
    for case in root.iter("testcase"):
        outcome.tests += 1
        name = f"{case.get('classname')} {case.get('name')}"
        failure = case.find("failure")
        if failure is None:
            failure = case.find("error")
        text = "\n".join(
            (el.text or "") for el in case if el.tag in ("system-out", "system-err", "failure", "error")
        )
        if failure is not None:
            outcome.failed.append((name, text))
            continue
        for reason in SKIP_MARKER.findall(text):
            outcome.skips.append((name, reason.strip()))


def report(outcome: Outcome, what: str, ui) -> None:
    """Turn an outcome into ticks, crosses and yellow dashes."""
    if outcome.failed:
        ui.bad(f"{len(outcome.failed)} {what} test(s) failed")
        for name, text in outcome.failed[:10]:
            ui.detail([name])
            tail = [l for l in text.splitlines() if l.strip()][-12:]
            for line in tail:
                print(f"        {line}")
    elif not outcome.finished:
        ui.bad(f"{what}: nextest did not finish (exit {outcome.result.code}) — no result to read")
        ui.detail(outcome.result.lines()[-8:])
    elif outcome.tests == 0:
        ui.bad(f"{what}: zero tests ran — that is not a pass")
    else:
        ui.ok(f"{outcome.tests} {what} test(s) passed")
    for reason in sorted({r for _, r in outcome.skips}):
        ui.skip("  a check inside them was skipped", reason)
