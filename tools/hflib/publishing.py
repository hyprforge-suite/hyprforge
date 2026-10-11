"""The checks that only mean something when code is about to leave.

These were check.sh steps, run on every full check, each a rebuild or a
network round: does every library package for crates.io, is every pin
on its repository's main, does every component build alone against the
published crates, does the AUR package build in a clean chroot. None of
the answers decided anything on an ordinary day. They decide something
at two moments — landing a component (`hf land`) and cutting a release
(`hf release`) — so they run there.
"""

import subprocess
import tempfile
from pathlib import Path

from . import ui
from .run import BUILD, ROOT, cargo_env, have, run


def submodule_paths() -> list[str]:
    r = run(["git", "config", "-f", ".gitmodules", "--get-regexp", r"^submodule\..*\.path$"])
    return [l.split()[1] for l in r.lines() if l.strip()]


def component_path(name: str) -> str:
    name = name.removeprefix("crates/").rstrip("/")
    if not name.startswith("hyprforge-"):
        name = f"hyprforge-{name}"
    path = f"crates/{name}"
    if path not in submodule_paths():
        ui.die(f"{path} is not a component submodule")
    return path


def pins_on_main(paths: list[str] | None = None, fetch: bool = True) -> list[str]:
    """Paths whose pinned commit is not on origin/main.

    A pin naming a commit nobody can fetch is a monorepo nobody else can
    clone. `fetch=False` asks the local origin/main ref only — what the
    pre-commit hook does, so a commit never waits on the network.
    """
    unpushed = []
    for path in paths or submodule_paths():
        pin = run(["git", "rev-parse", f"HEAD:{path}"]).out.strip()
        if fetch:
            run(["git", "-C", path, "fetch", "-q", "origin", "main"], timeout=120)
        if not run(["git", "-C", path, "merge-base", "--is-ancestor", pin, "origin/main"]).ok:
            unpushed.append(path)
    return unpushed


def staged_pins_not_on_main() -> list[tuple[str, str]]:
    """Staged gitlinks (pin bumps) whose commit is not on the local
    origin/main of that submodule. No network."""
    bad = []
    r = run(["git", "diff", "--cached", "--raw", "--no-abbrev"])
    for line in r.lines():
        # :160000 160000 <old> <new> M\tpath
        meta, _, path = line.partition("\t")
        fields = meta.split()
        if len(fields) >= 4 and fields[1] == "160000":
            sha = fields[3]
            if not run(["git", "-C", path, "merge-base", "--is-ancestor", sha, "origin/main"]).ok:
                bad.append((path, sha[:12]))
    return bad


def standalone_build(path: str, ref: str = "HEAD") -> tuple[bool, str]:
    """Build a component the way its own CI will: outside this
    workspace, where [patch.crates-io] does not reach, against the
    libraries crates.io has.

    `git archive` extracted with `-m` so every file is dated now: cargo
    leaves a package's directory out of its fingerprint, and an
    extraction dated older than the last one in the shared target would
    be judged fresh and never compiled. One shared target, so the
    components share one build of iced.
    """
    target = ROOT / "target" / "standalone"
    with tempfile.TemporaryDirectory(prefix="hf-standalone-") as d:
        archive = subprocess.run(["git", "-C", str(ROOT / path), "archive", ref], capture_output=True)
        if archive.returncode != 0:
            return False, archive.stderr.decode(errors="replace")
        subprocess.run(["tar", "-x", "-m", "-C", d], input=archive.stdout, check=True)
        r = run(["cargo", "check", "--all-targets", "--quiet"], cwd=d,
                env={**cargo_env(), "CARGO_TARGET_DIR": str(target)}, timeout=BUILD)
    if r.ok:
        return True, ""
    errors = [l for l in r.lines() if l.startswith("error")]
    return False, "\n".join(errors[:10] or r.lines()[-15:])


def publish_dry_run() -> None:
    """Does cargo's packager agree that every library is publishable?"""
    ui.step("Every library crate packages for crates.io")
    excluded = unpublished_members()
    args = ["cargo", "publish", "--workspace", "--dry-run", "--allow-dirty"]
    for name in excluded:
        args += ["--exclude", name]
    r = run(args, env=cargo_env(), timeout=BUILD)
    if r.ok:
        ui.ok("every library packages and builds from its own tarball")
    elif "already exists on crates.io index" in r.out:
        # Between releases the version is already published; packaging
        # without verifying still checks the metadata and file lists.
        r = run(["cargo", "package", "--workspace", "--no-verify", "--allow-dirty",
                 *[a for n in excluded for a in ("--exclude", n)]], env=cargo_env(), timeout=BUILD)
        if r.ok:
            ui.skip("packaged, not verified", "this version is already on crates.io; a version bump checks it fully")
        else:
            ui.bad("cargo package fails")
            ui.detail([l for l in r.lines() if l.startswith("error")][:10])
    else:
        ui.bad("cargo publish --dry-run fails")
        ui.detail([l for l in r.lines() if l.startswith("error")][:10])


def unpublished_members() -> list[str]:
    """Workspace members that are not published: every component except
    those other components use as libraries, plus anything marked
    `publish = false`. Derived, so a new component cannot be published
    by accident and a new library cannot be left out."""
    import json
    import tomllib

    meta = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1"], timeout=300).out)
    published_components = set(tomllib.loads((ROOT / "tools" / "hflib" / "published.toml").read_text())["components"])
    components = {Path(p).name for p in submodule_paths()}
    out = []
    for pkg in meta["packages"]:
        name = pkg["name"]
        if pkg.get("publish") == []:
            out.append(name)
        elif name in components and name not in published_components:
            out.append(name)
    return sorted(out)


def aur_chroot() -> None:
    ui.step("The AUR package builds in a clean chroot")
    if not have("extra-x86_64-build"):
        ui.skip("clean-chroot build", "devtools not installed — pacman -S devtools")
        return
    d = Path(tempfile.mkdtemp(prefix="hf-aur-", dir=ROOT / "target"))
    if not run(["python3", "tools/aur.py", "--local-into", str(d)], timeout=600).ok:
        ui.bad("tools/aur.py --local-into failed")
        return
    r = run(["extra-x86_64-build"], cwd=d, timeout=BUILD * 2)
    (d.with_suffix(".log")).write_text(r.out)
    if r.ok:
        ui.ok(f"{len(list(d.glob('*.pkg.tar.*')))} package(s) built in a clean chroot")
    else:
        ui.bad(f"the AUR package did not build in a clean chroot — log: {d.with_suffix('.log')}")
        ui.detail(r.lines()[-15:])
