"""`hf check`: everything this machine can check without changing it.

The order is check.sh's, and most steps are its, moved: the reasons for
each are in the comments here or in the lint scripts under `lints/`.
What changed, and why:

  - Tests run through nextest tiers (`.config/nextest.toml`), one build
    for all of them, instead of one `cargo test -p` per live step.
  - Nothing here changes Hyprland. The tests that do are the
    `disposable` tier, which only `hf test disposable` runs, against a
    nested instance. The session guard proves it after every full run.
  - `publish --dry-run` and building each component against crates.io
    moved to `hf release` and `hf land`, the two moments those answers
    decide something. Here they cost a full rebuild each for an answer
    nothing acted on.
"""

import re
import shutil
import tempfile
from pathlib import Path

from . import nextest, session_guard, tiers, ui
from .run import BUILD, ROOT, cargo_env, have, run

LINTS = ROOT / "tools" / "hflib" / "lints"
NOTIF = ROOT / "crates" / "hyprforge-notif"
NOTIF_CONFIG = ["--config-file", str(ROOT / "tools" / "hflib" / "nextest-notif.toml")]
TARGET_WARN_GB = 200


def _lines(output: str, prefix: str) -> list[str]:
    return [l[len(prefix):].strip() for l in output.splitlines() if l.startswith(prefix)]


# --- preconditions -------------------------------------------------------


def room_to_build() -> bool:
    # Nothing below is trustworthy on a full disk: a build killed by
    # ENOSPC looks like a failed test. Refuse up front, with the number.
    ui.step("Room to build")
    free = shutil.disk_usage(ROOT).free // 2**30
    if free < 20:
        ui.bad(
            f"only {free}GB free on this filesystem — a build needs room, and a full btrfs volume stalls "
            "every write on the machine. `tools/hf clean` reports where it went."
        )
        return False
    ui.ok(f"{free}GB free")
    target = ROOT / "target"
    if target.is_dir():
        r = run(["du", "-s", "--block-size=1G", str(target)], timeout=120)
        try:
            used = int(r.out.split()[0])
        except (IndexError, ValueError):
            used = 0
        if used > TARGET_WARN_GB:
            # The disk filled twice (439GB, then 666GB) with 98GB still
            # free the second time, so free space alone never warned.
            ui.skip(f"target/ is {used}GB", "over 200GB — `tools/hf clean` prunes what is safe to")
    return True


def submodules() -> bool:
    # A clone without --recurse-submodules has every component empty, and
    # cargo then fails with a sentence about cargo, not about the clone.
    ui.step("Components are submodules")
    r = run(["git", "config", "-f", ".gitmodules", "--get-regexp", r"^submodule\..*\.path$"])
    paths = [l.split()[1] for l in r.lines() if l.strip()]
    empty = [p for p in paths if not (ROOT / p / "Cargo.toml").is_file()]
    if empty:
        ui.bad(f"{len(empty)} component submodule(s) are empty ({' '.join(empty)}) — run: git submodule update --init")
        return False
    plain = []
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        text = manifest.read_text()
        if re.search(r"workspace\s*=\s*true", text):
            continue
        rel = str(manifest.parent.relative_to(ROOT))
        if rel not in paths:
            plain.append(rel)
    status = run(["git", "submodule", "status"]).lines()
    moved = [l.split()[1] for l in status if l.startswith("+")]
    on_next = []
    for p in paths:
        branch = run(["git", "-C", p, "branch", "--show-current"]).out.strip()
        if branch == "next":
            on_next.append(p)
    if plain:
        ui.bad(f"standalone crate(s) that are plain directories, not submodules: {' '.join(plain)}")
    else:
        msg = f"{len(paths)} components, each a submodule"
        ui.ok(msg + (f"; {len(moved)} checked out past its pin" if moved else " at its pinned commit"))
    if on_next:
        # Builds here and is fine to work on; a pin may never name it.
        ui.skip(f"{len(on_next)} on its `next` branch ({', '.join(Path(p).name for p in on_next)})",
                "builds in the workspace; not pinnable until the release merges it")
    return True


# --- tier 1: static -------------------------------------------------------


def clippy() -> None:
    ui.step("Lint")
    r = run(["cargo", "clippy", "--workspace", "--all-targets", *tiers.feature_args()], env=cargo_env(), timeout=BUILD)
    warnings = [l for l in r.lines() if re.match(r"^(error|warning)", l)]
    if r.ok and not warnings:
        ui.ok("clippy: no warnings")
        if "hyprforge-procman/netbpf" not in tiers.features():
            ui.skip("procman's netbpf feature not built", "clang is not installed")
    else:
        ui.bad(f"clippy: {len(warnings) or 'build failed,'} warning(s) — run: cargo clippy --workspace --all-targets")
        ui.detail(warnings[:10])


def docs() -> None:
    # docs.rs renders the `//!` docs, and a broken intra-doc link is plain
    # text there; only rustdoc sees it.
    ui.step("Docs build without warnings")
    r = run(["cargo", "doc", "--workspace", "--no-deps"], env={**cargo_env(), "RUSTDOCFLAGS": "-Dwarnings"}, timeout=BUILD)
    if r.ok:
        ui.ok("rustdoc: no warnings")
    else:
        warnings = [l for l in r.lines() if re.match(r"^(error|warning)", l)]
        ui.bad(f"rustdoc: {len(warnings)} warning(s) — run: RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps")
        ui.detail(warnings[:10])


def web_colors() -> None:
    # Both renderers report the same Color; only the pixels differ. See
    # the shared-look rule in CLAUDE.md.
    ui.step("Renderer colour space")
    r = run(["cargo", "tree", "--workspace", "-e", "features"], env=cargo_env(), timeout=BUILD)
    if "web-colors" in r.out:
        ui.bad("iced's `web-colors` is enabled — the wgpu hosts will draw the theme lighter than the tiny-skia ones")
    else:
        ui.ok("iced web-colors: off, so every host draws the same theme the same way")


def _lint(name: str) -> tuple[int, str]:
    r = run(["python3", str(LINTS / f"{name}.py")], timeout=300)
    return r.code, r.out


def dependency_pins() -> None:
    ui.step("Standalone crate dependency pins")
    code, out = _lint("dependency_pins")
    if code != 0:
        ui.bad("a standalone-ready crate's pinned dependency has drifted from the root table")
        ui.detail(out.splitlines())
    else:
        ui.ok(out.strip())


def siblings() -> None:
    ui.step("Siblings build from this checkout")
    _, out = _lint("siblings")
    problems = _lines(out, "PROBLEM ")
    # And what cargo actually resolved: a hyprforge- crate from anywhere
    # but a path did not come from this checkout.
    for cwd in [ROOT, NOTIF]:
        tree = run(["cargo", "tree", "--workspace", "-e", "normal,build,dev", "--prefix", "none"],
                   cwd=cwd, env=cargo_env(), timeout=BUILD)
        outside = sorted({l for l in tree.lines() if l.startswith("hyprforge-") and "(/" not in l})
        where = "" if cwd == ROOT else f" ({cwd.name})"
        problems += [f"resolved from outside this checkout{where}: {l}" for l in outside]
    if problems:
        ui.bad(f"{len(problems)} sibling(s) do not build from this checkout")
        ui.detail(problems)
    else:
        checked = (_lines(out, "CHECKED ") or ["?"])[0]
        ui.ok(f"all {checked} versioned siblings build from crates/")


def packages_are_repositories() -> None:
    ui.step("Every package is a repository")
    _, out = _lint("packages_are_repositories")
    if _lines(out, "SKIP "):
        ui.skip("package/repository check", _lines(out, "SKIP ")[0])
    elif _lines(out, "PROBLEM "):
        ui.bad(f"{len(_lines(out, 'PROBLEM '))} package(s) cannot become a repository as they stand")
        ui.detail(_lines(out, "PROBLEM "))
    else:
        ui.ok(f"{_lines(out, 'CHECKED ')[0]} package(s) each build from one crate directory, and every package carries its licence")


def _scratch_home_env(home: str) -> dict:
    return {
        "HOME": home,
        "XDG_CONFIG_HOME": f"{home}/.config",
        "XDG_DATA_HOME": f"{home}/.local/share",
        "XDG_STATE_HOME": f"{home}/.local/state",
        "XDG_CACHE_HOME": f"{home}/.cache",
    }


def installer_plan() -> None:
    # `--install --all --dry-run` runs every guard the real install does;
    # a real `--install --all` once died halfway on one nobody had run.
    ui.step("The installer can plan every component")
    with tempfile.TemporaryDirectory() as home:
        r = run(["./hyprforge", "--install", "--all", "--dry-run"], env=_scratch_home_env(home), timeout=120)
    if r.ok:
        ui.ok("every component's install plan resolves (dry run)")
    else:
        ui.bad("./hyprforge --install --all --dry-run fails")
        ui.detail([l for l in r.lines() if "error" in l.lower() or "die" in l.lower()][:5])


UNINSTALL_FIXTURES = [
    ".config/systemd/user/hyprforge-displayd.service",
    ".config/systemd/user/hyprforge-trayd.service",
    ".config/systemd/user/hyprforge-clipd.service",
    ".config/systemd/user/notifd.service",
    ".config/hyprforge/display-profiles.toml",
    ".config/hyprforge/emojimenu.toml",
    ".config/hyprforge/setup.toml",
    ".config/hyprforge/clipboard/history",
    ".config/notif/config.toml",
    ".local/share/applications/hyprforge-settings.desktop",
    ".local/share/applications/hyprforge-files.desktop",
    ".local/share/applications/hyprforge-media.desktop",
    ".local/share/dbus-1/services/org.freedesktop.FileManager1.service",
    ".local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.hyprforge.service",
]
UNINSTALL_COMPONENTS = ["greeter", "lock", "displayd", "tray", "clipboard", "notif", "emoji", "files", "media", "mime", "settings"]


def _tree(home: Path) -> list[str]:
    import hashlib

    out = []
    for p in sorted(home.rglob("*")):
        st = p.lstat()
        line = f"{p.relative_to(home)} {oct(st.st_mode)} {st.st_size} {st.st_mtime_ns}"
        if p.is_file():
            line += " " + hashlib.sha256(p.read_bytes()).hexdigest()
        out.append(line)
    return out


def installer_dry_run_touches_nothing() -> None:
    # A dry run that deletes something is worse than none: one did, one
    # prompt away from `rm -rf ~/.config/hyprforge`. Compared by bytes,
    # modes and mtimes, because output is what a dry run is honest about
    # by construction; the question is what it does to the disk.
    ui.step("The installer's dry run touches nothing")
    with tempfile.TemporaryDirectory() as home:
        h = Path(home)
        for f in UNINSTALL_FIXTURES:
            (h / f).parent.mkdir(parents=True, exist_ok=True)
            (h / f).write_text(f"fixture {f}\n")
        before = _tree(h)
        refused = 0
        for c in UNINSTALL_COMPONENTS:
            r = run(["./hyprforge", "--uninstall", f"--{c}", "--force-cleanup", "--dry-run"],
                    env=_scratch_home_env(home), timeout=60)
            refused += 0 if r.ok else 1
        after = _tree(h)
    if before == after:
        ui.ok(f"{len(UNINSTALL_COMPONENTS)} dry-run uninstalls with --force-cleanup left a scratch home unchanged "
              f"({refused} stopped at a guard first)")
    else:
        ui.bad("a dry-run uninstall changed files in the home directory it was given")
        ui.detail(sorted(set(before) ^ set(after))[:10])


def install_parity() -> None:
    ui.step("The installer and the package install the same files")
    _, out = _lint("install_parity")
    problems = _lines(out, "PROBLEM ")
    if problems:
        ui.bad(f"{len(problems)} file(s) installed one way and not the other")
        ui.detail(problems)
    else:
        ui.ok(f"{(_lines(out, 'CHECKED ') or ['?'])[0]} repository files, installed by both ./hyprforge and the PKGBUILD")


def setup_ids() -> None:
    ui.step("Every Set up item is undone by some uninstall")
    _, out = _lint("setup_ids")
    problems = _lines(out, "PROBLEM ")
    if problems:
        ui.bad(f"{len(problems)} Set up item(s) out of step with the installer")
        ui.detail(problems)
    else:
        ui.ok(f"all {(_lines(out, 'CHECKED ') or ['?'])[0]} Set up items are undone by uninstalling their component, or Settings")


def lua_dispatch() -> None:
    ui.step("Nothing shipped dispatches in hyprlang syntax")
    _, out = _lint("lua_dispatch")
    problems = _lines(out, "PROBLEM ")
    if problems:
        ui.bad(f"{len(problems)} `hyprctl dispatch` call(s) a Lua config rejects")
        ui.detail(problems)
    else:
        ui.ok(f"{(_lines(out, 'CHECKED ') or ['?'])[0]} scripts and configs dispatch in Lua")


def docs_name_things() -> None:
    ui.step("Docs name things that exist")
    _, out = _lint("docs_name_things")
    problems = _lines(out, "PROBLEM ")
    if problems:
        ui.bad(f"{len(problems)} doc claim(s) no longer true")
        ui.detail(problems, limit=30)
    else:
        checked = (_lines(out, "CHECKED ") or ["? ? ? ? ?"])[0].split()
        named, repos, gates, pages, shared = (checked + ["?"] * 5)[:5]
        ui.ok(f"{named} source path(s) named in docs all exist; README lists all {repos} standalone repositories; "
              f"the docs count all {gates} live gates; Settings' README names all {pages} pages; "
              f"the design-system catalogue names all {shared} shared exports")


def crate_readmes() -> None:
    ui.step("Library READMEs match their crate docs")
    r = run(["python3", "tools/crate-readme.py", "--check"])
    stale = _lines(r.out, "STALE ")
    if r.ok:
        ui.ok("every library README is its crate doc, rendered")
    elif not stale:
        ui.bad("tools/crate-readme.py failed without naming a stale README")
        ui.detail(r.lines()[-5:])
    else:
        ui.bad(f"{len(stale)} library README(s) differ from their crate doc — run: tools/crate-readme.py --write")
        ui.detail(stale)


def versions() -> None:
    ui.step("Versions agree")
    r = run(["python3", "tools/versions.py", "--check"])
    checked = _lines(r.out, "CHECKED ")
    if not checked:
        ui.bad("tools/versions.py failed without a result")
        ui.detail(r.lines()[-5:])
        return
    want, counted, noted = (checked[0].split() + ["0", "0"])[:3]
    if not r.ok:
        ui.bad(f"{len(_lines(r.out, 'DRIFT '))} version(s) disagree with the workspace's {want}")
        ui.detail(_lines(r.out, "DRIFT "))
    else:
        ui.ok(f"{int(counted) - int(noted)} stated version(s) match the workspace's {want}")
    if int(noted) > 0:
        ui.skip(f"{noted} manifest(s) not held to {want} yet", "not released in lockstep — the next release moves them")


def changelog() -> None:
    ui.step("Changelog has a section for the version")
    import tomllib

    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    log = ROOT / "CHANGELOG.md"
    text = log.read_text() if log.is_file() else None
    if text is None:
        ui.bad("no CHANGELOG.md")
    elif f"## [{version}]" not in text:
        ui.bad(f"CHANGELOG.md has no '## [{version}]' section for the workspace version")
    elif "## [Unreleased]" not in text:
        ui.bad("CHANGELOG.md has no '## [Unreleased]' section for the next release to be written in")
    else:
        ui.ok(f"CHANGELOG.md covers {version}, with an Unreleased section above it")


def aur() -> None:
    ui.step("AUR PKGBUILD matches packaging/arch")
    r = run(["python3", "tools/aur.py", "--check"], timeout=120)
    if r.ok:
        ui.ok("packaging/aur/hyprforge is packaging/arch, rendered")
        if "NOSRCINFO" in r.out:
            ui.skip(".SRCINFO not compared", "makepkg not installed")
    elif not _lines(r.out, "STALE "):
        ui.bad("tools/aur.py failed without naming a stale file")
        ui.detail(r.lines()[-5:])
    else:
        ui.bad("packaging/aur/hyprforge differs from a fresh render — run: tools/aur.py --write")
        ui.detail(_lines(r.out, "STALE "))


def classification() -> None:
    """Every ignored test is in exactly one tier, and every live or
    visible test sits behind exactly one gate."""
    ui.step("Every ignored test has one tier")
    try:
        everything = set(nextest.listing("default", ignored=True))
        by_tier = {t: set(nextest.listing(t, ignored=True)) for t in tiers.IGNORED_TIERS}
        gated = {}
        for tier in ("live", "visible"):
            for g in tiers.gates_for(tier):
                for name in nextest.listing(tier, ignored=True, expr=g.expr):
                    gated.setdefault(name, []).append(g.label)
    except RuntimeError as e:
        ui.bad(f"could not list the tests: {e}")
        return
    problems = []
    seen = {}
    for tier, names in by_tier.items():
        for n in names:
            seen.setdefault(n, []).append(tier)
    for n in sorted(everything):
        where = seen.get(n, [])
        if not where:
            problems.append(f"{n} is #[ignore]d but in no tier — add it to .config/nextest.toml")
        elif len(where) > 1:
            problems.append(f"{n} is in {len(where)} tiers: {', '.join(where)}")
    for tier in ("live", "visible"):
        for n in sorted(by_tier[tier]):
            labels = gated.get(n, [])
            if not labels:
                problems.append(f"{n} is in `{tier}` but behind no gate in tools/hflib/tiers.py, so it never runs")
            elif len(labels) > 1:
                problems.append(f"{n} is behind {len(labels)} gates: {', '.join(labels)}")
    if problems:
        ui.bad(f"{len(problems)} test(s) misclassified")
        ui.detail(problems, limit=30)
    else:
        counts = ", ".join(f"{len(by_tier[t])} {t}" for t in tiers.IGNORED_TIERS)
        ui.ok(f"{len(everything)} ignored tests, each in one tier ({counts}), every live one behind one gate")


# --- tier 1: tests ----------------------------------------------------------


def unit_tests() -> None:
    # A killed run prints no failure line and a run that compiled nothing
    # reports zero tests; both once passed as "0 tests passed" here.
    ui.step("Unit and integration tests")
    nextest.mark_start()
    outcome = nextest.run_tier("unit", ignored=False)
    nextest.report(outcome, "unit", ui)
    # nextest does not run doctests.
    r = run(["cargo", "test", "--workspace", "--doc", *tiers.feature_args()], env=cargo_env(), timeout=BUILD)
    passed = sum(int(m) for m in re.findall(r"test result: \w+\. (\d+) passed", r.out))
    if not r.ok:
        ui.bad("doctests failed — run: cargo test --workspace --doc")
        ui.detail([l for l in r.lines() if "FAILED" in l][:10])
    else:
        ui.ok(f"{passed} doctest(s) passed")


def notif() -> None:
    # notif is its own cargo workspace (zbus picks a different runtime
    # there; see Cargo.toml's `exclude`), so nothing above reaches it.
    ui.step("The notif workspace (its own cargo workspace)")
    r = run(["cargo", "clippy", "--all-targets"], cwd=NOTIF, env=cargo_env(), timeout=BUILD)
    warnings = [l for l in r.lines() if re.match(r"^(error|warning)", l)]
    if r.ok and not warnings:
        ui.ok("notif clippy: no warnings")
    else:
        ui.bad(f"notif clippy: {len(warnings)} warning(s) — run: (cd crates/hyprforge-notif && cargo clippy --all-targets)")
    if run(["cargo", "fmt", "--all", "--check"], cwd=NOTIF, timeout=300).ok:
        ui.ok("notif fmt: clean")
    else:
        ui.bad("notif fmt: not clean — run: (cd crates/hyprforge-notif && cargo fmt --all)")
    nextest.mark_start()
    outcome = nextest.run_tier("default", ignored=False, workspace=NOTIF, extra=NOTIF_CONFIG)
    nextest.report(outcome, "notif", ui)


# --- full: live and visible tiers ---------------------------------------


def live_tiers() -> None:
    before = session_guard.snapshot()
    for tier, heading in (("live", "Live tests: real services, read-only"),
                          ("visible", "Visible tests: briefly on screen, then undone")):
        ui.step(heading)
        open_exprs = []
        for g in tiers.gates_for(tier):
            why = g.check()
            if why:
                ui.skip(g.label, why)
            else:
                open_exprs.append(g.expr)
        if not open_exprs:
            continue
        nextest.mark_start()
        outcome = nextest.run_tier(tier, ignored=True, expr=" | ".join(f"({e})" for e in open_exprs))
        nextest.report(outcome, tier, ui)
    # notif's two font tests ask the installed fonts and nothing else, so
    # that is their gate: check.sh never ran them, and the first run here
    # failed on a machine with no CJK font rather than on anything notif
    # does. Asked by a glyph the test shapes (标, U+6807), not by
    # `:lang=zh`: Font Awesome claims zh coverage and has none of it.
    ui.step("Live tests: notif against the installed fonts")
    cjk = run(["fc-list", ":charset=6807", "family"]).out.strip() if have("fc-list") else ""
    emoji = run(["fc-list", ":charset=1f600", "family"]).out.strip() if have("fc-list") else ""
    if not cjk or not emoji:
        ui.skip("CJK and emoji shaping", "no " + ("CJK" if not cjk else "emoji") + " font installed to shape with")
    else:
        nextest.mark_start()
        outcome = nextest.run_tier("default", ignored=True, workspace=NOTIF, extra=NOTIF_CONFIG)
        nextest.report(outcome, "notif font", ui)

    ui.step("The session is as it was")
    if before is None:
        ui.skip("session guard", "no Hyprland session to guard")
        return
    changed = session_guard.diff(before, session_guard.snapshot() or {})
    if changed:
        ui.bad(f"{len(changed)} thing(s) in the running Hyprland changed during the live tiers — a live test is not "
               "read-only (or someone changed a setting while it ran)")
        ui.detail(changed, limit=30)
    else:
        ui.ok(f"{len(before)} options, binds, rules, monitors and daemon states unchanged")


def check(fast: bool) -> int:
    if not room_to_build() or not submodules():
        return ui.summary()
    clippy()
    docs()
    web_colors()
    dependency_pins()
    siblings()
    packages_are_repositories()
    installer_plan()
    installer_dry_run_touches_nothing()
    install_parity()
    setup_ids()
    lua_dispatch()
    docs_name_things()
    crate_readmes()
    versions()
    changelog()
    aur()
    classification()
    unit_tests()
    notif()
    if fast:
        ui.step("Skipping live tiers (--fast)")
    else:
        live_tiers()
    return ui.summary()


def lint() -> int:
    """The static checks: nothing compiled beyond what listing tests needs."""
    if not submodules():
        return ui.summary()
    web_colors()
    dependency_pins()
    siblings()
    packages_are_repositories()
    install_parity()
    setup_ids()
    lua_dispatch()
    docs_name_things()
    crate_readmes()
    versions()
    changelog()
    aur()
    classification()
    return ui.summary()
