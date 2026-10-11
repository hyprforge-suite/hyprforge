"""`hf release X.Y.Z [--execute]`: the two-phase release.

Dry run by default, because most of this cannot be taken back: a
crates.io version is permanent, a pushed tag is public, an AUR package
reaches users the moment it is pushed. Read the dry run first.

Why two phases. A component that calls a library change has to be built
by its own CI against the *published* library, so it cannot be pushed
until that library is on crates.io. tools/release.sh pushed everything
at once, which meant such a component commit could not be in the
release at all: 0.1.13 needed a commit split by hand and work parked on
`after-0.1.14` branches. Now that work waits on the component's `next`
branch (`hf land --next`), and:

  Phase A — libraries. Merge every `next` locally and check the whole
  workspace with them. Move every version. Push the clipboard and the
  tray (published from here, so they go with the libraries), commit
  "Release vX.Y.Z (libraries)", tag `libs-vX.Y.Z`; publish.yml publishes
  and waits out crates.io's rate limit by itself. Then wait until the
  index lists every library at X.Y.Z.

  Phase B — components. Each one, now that what it needs is published:
  built alone against crates.io exactly as its CI will, then pushed to
  its main, its `next` deleted. Then the suite commit "Release vX.Y.Z"
  with every pin, tagged `vX.Y.Z` — the AUR package sources the
  submodules at this tag's pins, so the tag must be on the commit with
  the final ones — then component tags and GitHub Releases.

Resumable: each finished step is recorded in target/hf/release-X.Y.Z.json
and a re-run continues from the first unfinished one. release.sh died at
step 2 on a re-run, so 0.1.10's last four steps were finished by hand.
Nothing here retries a failed step or forces anything.
"""

import json
import time
import tomllib
import urllib.request

from . import checks, publishing, testing, ui
from .run import ROOT, have, run

PUBLISHED_COMPONENTS = set(tomllib.loads((ROOT / "tools" / "hflib" / "published.toml").read_text())["components"])


class Release:
    def __init__(self, version: str, execute: bool):
        self.version = version
        self.tag = f"v{version}"
        self.libs_tag = f"libs-v{version}"
        self.execute = execute
        self.state_path = ROOT / "target" / "hf" / f"release-{version}.json"
        self.state = {"done": [], "changed": None, "components_done": []}
        if self.state_path.is_file():
            self.state = json.loads(self.state_path.read_text())
        self.submodules = publishing.submodule_paths()

    # --- plumbing -----------------------------------------------------

    def save(self) -> None:
        if self.execute:
            self.state_path.parent.mkdir(parents=True, exist_ok=True)
            self.state_path.write_text(json.dumps(self.state, indent=2))

    def act(self, cmd: list[str], cwd=None, timeout=600) -> None:
        """Print an action; do it only under --execute. Every effect
        outside the working tree goes through here, so the dry run is a
        complete list of them."""
        where = f" (in {cwd})" if cwd else ""
        ui.note(f"+ {' '.join(cmd)}{where}")
        if self.execute:
            r = run(cmd, cwd=ROOT / cwd if cwd else ROOT, timeout=timeout)
            if not r.ok:
                ui.detail(r.lines()[-10:])
                ui.die(f"stopped: {' '.join(cmd)} failed. Done so far is recorded in {self.state_path}; "
                       "fix the cause and re-run the same command to continue.")

    def step(self, key: str, title: str, fn) -> None:
        if key in self.state["done"]:
            ui.step(f"{title} — done in an earlier run")
            return
        ui.step(title)
        fn()
        if self.execute:
            self.state["done"].append(key)
            self.save()

    # --- preconditions --------------------------------------------------

    def preconditions(self) -> None:
        ui.step("Preconditions")
        current = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        refusals = []
        if not self.state["done"]:
            if tuple(map(int, self.version.split("."))) <= tuple(map(int, current.split("."))):
                refusals.append(f"{self.version} is not newer than the workspace's {current}")
            if run(["git", "rev-parse", "-q", "--verify", f"refs/tags/{self.tag}"]).ok:
                refusals.append(f"{self.tag} already exists")
            if run(["git", "branch", "--show-current"]).out.strip() != "main":
                refusals.append("not on main")
            if run(["git", "status", "--porcelain", "--ignore-submodules=none"]).out.strip():
                refusals.append("the working tree is not clean — commit or stash first, including inside submodules")
            # The pin is what a release ships. A submodule checked out at
            # anything else — a `next` branch, a local commit — is not in
            # the pins the release would commit, and release.sh pushed
            # whatever HEAD happened to be checked out.
            moved = [l.split()[1] for l in run(["git", "submodule", "status"]).lines() if l[:1] in "+-U"]
            if moved:
                refusals.append(f"checked out away from their pins: {', '.join(moved)} — `git submodule update` first; "
                                "work for the release belongs on each component's `next` branch")
            fresh, why = testing.disposable_stamp_is_fresh()
            if not fresh:
                refusals.append(why)
            else:
                ui.ok(f"disposable tier {why}")
        if self.execute and not have("gh"):
            refusals.append("gh is needed for the GitHub Releases")
        for r in refusals:
            if self.execute:
                ui.bad(r)
            else:
                ui.skip(r, "an --execute run stops here")
        if refusals and self.execute:
            ui.die("refusing to start the release")
        if self.state["changed"] is None:
            # Before anything moves a pin.
            prev = run(["git", "describe", "--tags", "--abbrev=0", "--match", "v*"]).out.strip()
            out = run(["python3", "tools/changelog.py", "components-changed", "--since", prev]).out.split()
            self.state["changed"] = out
            self.state["previous"] = prev
            self.save()
        ui.ok(f"{current} -> {self.version}; previous tag {self.state.get('previous')}")
        ui.note(f"components whose pin moved since then: {' '.join(self.state['changed']) or '(none)'}")

    # --- phase A ----------------------------------------------------------

    def nexts(self) -> list[str]:
        """Components with a `next` branch on their repository."""
        out = []
        for path in self.submodules:
            if run(["git", "-C", path, "ls-remote", "--exit-code", "--heads", "origin", "next"], timeout=60).ok:
                out.append(path)
        return out

    def merge_nexts(self) -> None:
        nexts = self.nexts()
        if not nexts:
            ui.ok("no component has a next branch")
        for path in self.submodules:
            # Every submodule onto a local main at its pin, so the commits
            # this release makes are on a branch, not a detached HEAD.
            pin = run(["git", "rev-parse", f"HEAD:{path}"]).out.strip()
            self.act(["git", "-C", path, "checkout", "-q", "-B", "main", pin])
        for path in nexts:
            self.act(["git", "-C", path, "fetch", "-q", "origin", "next"], timeout=120)
            self.act(["git", "-C", path, "merge", "--no-edit", "-m",
                      f"Merge next for {self.tag}: work that needed libraries released with it", "FETCH_HEAD"])

    def check_everything(self) -> None:
        ui.note("+ tools/hf check, then cargo's packager and the clean-chroot AUR build")
        if not self.execute:
            return
        if checks.check(fast=False) != 0:
            ui.die("the checks failed with every next merged in; fix it on the component's next branch and re-run")
        publishing.publish_dry_run()
        publishing.aur_chroot()
        if ui.FAILURES:
            ui.die("a release check failed")

    def move_versions(self) -> None:
        if self.execute:
            for cmd in (["python3", "tools/versions.py", "--set", self.version],
                        ["python3", "tools/changelog.py", "release", self.version, "--since", self.state["previous"], "--write"]):
                r = run(cmd)
                if not r.ok:
                    ui.detail(r.lines()[-10:])
                    ui.die(f"{' '.join(cmd)} failed")
        else:
            ui.detail(run(["python3", "tools/versions.py", "--set", self.version, "--dry-run"]).lines(), limit=40)
            ui.detail(run(["python3", "tools/changelog.py", "release", self.version, "--since",
                           self.state.get("previous") or "HEAD"]).lines()[:10])
        self.act(["cargo", "update", "--workspace", "--offline"], timeout=600)
        self.act(["cargo", "update", "--workspace", "--offline"], cwd="crates/hyprforge-notif", timeout=600)
        self.act(["python3", "tools/aur.py", "--write"])
        self.act(["tools/hf", "check", "--fast"], timeout=7200)

    def _commit_component(self, path: str, message: str) -> None:
        # Every manifest whose version moved, its lockfile where one is
        # tracked (only notif's — git refuses a pathspec that matches
        # nothing, which stopped 0.1.10), and a changelog if one was
        # written for it.
        specs = [":(glob)**/Cargo.toml"]
        if run(["git", "-C", path, "ls-files", "--", ":(glob)**/Cargo.lock"]).out.strip():
            specs.append(":(glob)**/Cargo.lock")
        self.act(["git", "-C", path, "add", "-u", "--", *specs])
        if (ROOT / path / "CHANGELOG.md").exists():
            self.act(["git", "-C", path, "add", "--", "CHANGELOG.md"])
        self.act(["git", "-C", path, "commit", "-q", "-m", message])

    def push_published_components(self) -> None:
        for path in self.submodules:
            if path.split("/")[-1] in PUBLISHED_COMPONENTS:
                self._commit_component(path, f"Release {self.tag}")
                self.act(["git", "-C", path, "push", "origin", "HEAD:main"], timeout=120)

    def libraries_commit(self) -> None:
        published = [p for p in self.submodules if p.split("/")[-1] in PUBLISHED_COMPONENTS]
        self.act(["git", "add", "Cargo.toml", "Cargo.lock", "CHANGELOG.md", "packaging", *published])
        self.act(["git", "commit", "-q", "-m", f"Release {self.tag} (libraries)"])
        self.act(["git", "push", "origin", "main"], timeout=120)
        self.act(["git", "tag", "-a", self.libs_tag, "-m", f"Hyprforge {self.version}: libraries"])
        self.act(["git", "push", "origin", self.libs_tag], timeout=120)

    def wait_for_index(self) -> None:
        names = self.published_crates()
        ui.note(f"publish.yml is publishing {len(names)} crates; it waits out crates.io's 429s by itself "
                "(gh run watch -R hyprforge-suite/hyprforge to follow it)")
        if not self.execute:
            return
        deadline = time.time() + 4 * 3600
        missing = names
        while time.time() < deadline:
            missing = [n for n in names if not on_index(n, self.version)]
            if not missing:
                ui.ok(f"all {len(names)} crates are on crates.io at {self.version}")
                return
            print(f"  … {len(names) - len(missing)}/{len(names)} on the index; waiting", flush=True)
            time.sleep(60)
        ui.die(f"still not on the index after four hours: {', '.join(missing)} — look at the publish run")

    def published_crates(self) -> list[str]:
        meta = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1"], timeout=300).out)
        skip = set(publishing.unpublished_members())
        return sorted(p["name"] for p in meta["packages"] if p["name"] not in skip)

    # --- phase B ----------------------------------------------------------

    def components(self) -> None:
        for path in self.submodules:
            name = path.split("/")[-1]
            if name in PUBLISHED_COMPONENTS or path in self.state["components_done"]:
                continue
            ui.note(f"{name}:")
            self._commit_component(path, f"Release {self.tag}")
            if self.execute:
                built, errors = publishing.standalone_build(path)
                if not built:
                    ui.detail(errors.splitlines())
                    ui.die(f"{name} does not build alone against the published {self.version}; fix it on its main "
                           "and re-run — every library is already published, so nothing upstream needs to move")
                ui.ok(f"{name} builds against the published libraries")
            else:
                ui.note(f"+ build {name} alone against crates.io {self.version}")
            self.act(["git", "-C", path, "push", "origin", "HEAD:main"], timeout=120)
            if run(["git", "-C", path, "ls-remote", "--exit-code", "--heads", "origin", "next"], timeout=60).ok:
                self.act(["git", "-C", path, "push", "origin", "--delete", "next"], timeout=120)
            if self.execute:
                self.state["components_done"].append(path)
                self.save()

    def suite_commit(self) -> None:
        self.act(["git", "add", "Cargo.lock", *self.submodules])
        self.act(["git", "commit", "-q", "-m", f"Release {self.tag}"])
        self.act(["git", "push", "origin", "main"], timeout=120)
        self.act(["git", "tag", "-a", self.tag, "-m", f"Hyprforge {self.version}"])
        self.act(["git", "push", "origin", self.tag], timeout=120)
        for path in self.submodules:
            self.act(["git", "-C", path, "tag", "-a", self.tag, "-m", f"Hyprforge {self.version}"])
            self.act(["git", "-C", path, "push", "origin", self.tag], timeout=120)

    def github_releases(self) -> None:
        for path in self.state["changed"] or []:
            repo = f"hyprforge-suite/{path.split('/')[-1]}"
            notes = self._notes(ROOT / path / "CHANGELOG.md")
            self.act(["gh", "release", "create", self.tag, "-R", repo, "--title", self.tag, "--notes", notes])
        self.act(["gh", "release", "create", self.tag, "-R", "hyprforge-suite/hyprforge",
                  "--title", f"Hyprforge {self.version}", "--notes", self._notes(ROOT / "CHANGELOG.md")])

    def _notes(self, changelog) -> str:
        if not changelog.is_file():
            return f"Hyprforge {self.version}"
        out, inside = [], False
        for line in changelog.read_text().splitlines():
            if line.startswith(f"## [{self.version}]"):
                inside = True
                continue
            if inside and line.startswith("## ["):
                break
            if inside:
                out.append(line)
        return "\n".join(out).strip() or f"Hyprforge {self.version}"

    def aur(self) -> None:
        print(f"""  The AUR push stays a person's: it needs an SSH key registered with
  aur.archlinux.org. Once the tag is on GitHub:

    git clone ssh://aur@aur.archlinux.org/hyprforge.git ~/aur-hyprforge
    cp packaging/aur/hyprforge/{{PKGBUILD,.SRCINFO,*.install}} ~/aur-hyprforge/
    cd ~/aur-hyprforge && makepkg -o && git add -A && git commit -m "{self.version}" && git push

  makepkg -o fetches the tagged sources and runs prepare(): the cheap
  proof the tag and the render agree before anyone else downloads it.""")

    def run(self) -> int:
        self.preconditions()
        self.step("merge", "A1. Every component's next branch, merged locally", self.merge_nexts)
        self.step("check", "A2. Everything this machine can check, with them merged", self.check_everything)
        self.step("versions", "A3. Versions, changelogs, lockfiles, the AUR render", self.move_versions)
        self.step("published-components", "A4. The clipboard and the tray, pushed (published with the libraries)",
                  self.push_published_components)
        self.step("libs", f"A5. The libraries commit, tagged {self.libs_tag}", self.libraries_commit)
        self.step("index", "A6. Waiting for crates.io", self.wait_for_index)
        self.step("components", "B1. Each component: built against the published crates, pushed", self.components)
        self.step("suite", f"B2. The suite commit with every pin, tagged {self.tag}", self.suite_commit)
        self.step("github", "B3. GitHub Releases for what changed", self.github_releases)
        ui.step("B4. The AUR (by hand)")
        self.aur()
        if not self.execute:
            print(f"\n{ui.BOLD}Dry run: nothing above was done. Re-run with --execute.{ui.OFF}")
        return 0


def on_index(name: str, version: str) -> bool:
    """Whether crates.io's sparse index lists `name` at `version` — what
    cargo itself reads, with no token and no API rate limit."""
    if len(name) <= 2:
        path = f"{len(name)}/{name}"
    elif len(name) == 3:
        path = f"3/{name[0]}/{name}"
    else:
        path = f"{name[:2]}/{name[2:4]}/{name}"
    try:
        with urllib.request.urlopen(f"https://index.crates.io/{path}", timeout=30) as r:
            return any(json.loads(l).get("vers") == version for l in r.read().decode().splitlines() if l.strip())
    except Exception:
        return False


def release(version: str, execute: bool) -> int:
    parts = version.split(".")
    if len(parts) != 3 or not all(p.isdigit() for p in parts):
        ui.die(f"{version!r} is not X.Y.Z")
    return Release(version, execute).run()
