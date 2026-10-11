"""`hf sandbox`: the shipped systemd units, checked, and code run inside them.

Every test of the procman recorder passed while its unit had
`PrivateNetwork=yes`, which hides every exit from it: the tests ran
outside the sandbox. A unit's sandbox is part of the program, so this
runs the program inside it.

Two halves:

  - `systemd-analyze verify` on every shipped unit, with ExecStart
    pointed at a stand-in so a misspelled property is caught without the
    binary being installed. D-Bus activation files share the `.service`
    suffix and are not units; they are told apart by their section.
  - For each unit whose binary has a `--self-test`, that self-test run
    by `systemd-run` with every sandboxing property the unit file sets —
    read from the file, so the check cannot drift from what ships. A
    system unit needs root for that; `hf sandbox` asks sudo when there is
    a terminal to ask on, and says it skipped otherwise.
"""

import re
import subprocess
import sys
import tempfile
from pathlib import Path

from . import ui
from .run import ROOT, cargo_env, have, run

# Units whose binary proves its own sandbox. (unit file, binary, cargo
# args to build it, system or user)
SELF_TESTS = [
    (
        "crates/hyprforge-procman/packaging/hyprforge-procman-recorder.service",
        "hyprforge-procman-recorder",
        ["-p", "hyprforge-procman", "--bin", "hyprforge-procman-recorder", "--features", "hyprforge-procman/netbpf"],
        "system",
    ),
]

# Properties that describe how a service runs, not what it may touch;
# systemd-run sets its own.
NOT_SANDBOX = {"Type", "ExecStart", "ExecStop", "ExecReload", "Restart", "RestartSec", "BusName", "Environment"}


def units() -> list[Path]:
    r = run(["git", "ls-files", "--recurse-submodules", "*.service"])
    out = []
    for rel in r.lines():
        p = ROOT / rel
        if p.is_file() and "[Service]" in p.read_text():
            out.append(p)
    return out


def is_user_unit(unit: Path) -> bool:
    text = unit.read_text()
    return "default.target" in text or "graphical-session.target" in text


def service_properties(unit: Path) -> list[tuple[str, str]]:
    props, section = [], None
    for line in unit.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("["):
            section = line
            continue
        if section == "[Service]" and "=" in line:
            k, v = line.split("=", 1)
            if k not in NOT_SANDBOX:
                props.append((k, v))
    return props


def verify() -> None:
    ui.step("Shipped units are valid systemd units")
    if not have("systemd-analyze"):
        ui.skip("unit verification", "systemd-analyze not installed")
        return
    problems, checked = [], 0
    with tempfile.TemporaryDirectory() as d:
        for unit in units():
            text = re.sub(r"^ExecStart=.*$", "ExecStart=/usr/bin/true", unit.read_text(), flags=re.M)
            copy = Path(d) / unit.name
            copy.write_text(text)
            args = ["systemd-analyze", "--man=no", "--generators=no"]
            if is_user_unit(unit):
                args.append("--user")
            r = run([*args, "verify", str(copy)], timeout=60)
            checked += 1
            # Exit 0 with a warning is how systemd reports a misspelled
            # key ("Unknown key 'ProtectClok', ignoring") — a property
            # silently not applied — so any output at all is a problem.
            noise = [l.replace(str(copy), unit.name) for l in r.lines() if l.strip()]
            if not r.ok or noise:
                problems += [f"{unit.relative_to(ROOT)}: {l}" for l in (noise or [f"exit {r.code}"])[:5]]
    if problems:
        ui.bad(f"{len(problems)} problem(s) in shipped units")
        ui.detail(problems)
    else:
        ui.ok(f"{checked} unit(s) verify cleanly")


def self_tests(allow_sudo: bool) -> None:
    ui.step("Self-tests inside each unit's own sandbox")
    for unit_rel, binary, build, scope in SELF_TESTS:
        unit = ROOT / unit_rel
        r = run(["cargo", "build", "-q", *build], env=cargo_env(), timeout=3600)
        if not r.ok:
            ui.bad(f"{binary} did not build")
            ui.detail(r.lines()[-10:])
            continue
        exe = ROOT / "target" / "debug" / binary
        props = [a for k, v in service_properties(unit) for a in ("-p", f"{k}={v}")]
        cmd = ["systemd-run", "--wait", "--pipe", "--collect", "--quiet", *props, str(exe), "--self-test"]
        if scope == "user":
            cmd.insert(1, "--user")
        elif not allow_sudo:
            ui.skip(f"{binary} under {unit.name}", "a system unit's sandbox needs root — run from a terminal, or `hf test root`")
            continue
        else:
            if subprocess.run(["sudo", "-v"]).returncode != 0:
                ui.skip(f"{binary} under {unit.name}", "sudo refused")
                continue
            cmd = ["sudo", "--non-interactive", *cmd]
        r = run(cmd, timeout=120)
        lines = [l for l in r.lines() if l.strip()]
        if r.ok:
            ui.ok(f"{binary}, inside {unit.name}'s sandbox: " + "; ".join(l.removeprefix("ok: ") for l in lines if l.startswith("ok:")))
        else:
            ui.bad(f"{binary} fails its self-test inside {unit.name}'s sandbox")
            ui.detail(lines)


def sandbox() -> int:
    verify()
    self_tests(allow_sudo=sys.stdin.isatty())
    return ui.summary()
