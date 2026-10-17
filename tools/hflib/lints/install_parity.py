#!/usr/bin/env python3
# The installer and the Arch package install the same files.
#
# There are two ways to install Hyprforge — `./hyprforge --install` and
# the packages built from packaging/arch/PKGBUILD — and each names the
# files it copies out of the repository by hand. They drifted: the
# greeter's package shipped the binary alone, without the sysusers and
# tmpfiles entries the installer always put down, so a packaged greeter
# ran with default colours and a fallback scale and nothing said why.
#
# This compares the set of repository files (under a crate's config/,
# packaging/, data/ or assets/) each one names. A difference that is
# deliberate goes in DELIBERATE with the reason. Prints CHECKED / PROBLEM.

import re
from pathlib import Path

SOURCE = re.compile(r"crates/hyprforge-[a-z-]+/(?:config|packaging|data|assets)/[A-Za-z0-9_.@-]+")

DELIBERATE = {
    # The installer writes these two D-Bus activation files into the
    # user's own ~/.local/share with $PREFIX in them, rather than copying
    # the packaged ones that name /usr/bin.
    "crates/hyprforge-files/packaging/hyprforge-files.FileManager1.service": "installer",
    "crates/hyprforge-files/packaging/org.freedesktop.impl.portal.desktop.hyprforge.service": "installer",
}

# Both files name procman's packaging directory once, `local p=crates/…`,
# and every file in it as `$p/name`. Matching only literal paths left
# all of those out of the comparison on both sides at once — the
# recorder's unit, the helper's bus and polkit files — so the check
# reported nothing while comparing nothing. The variable is expanded
# before matching.
LOCAL = re.compile(r"local p=(crates/\S+)")


def named(path: str) -> set[str]:
    text = Path(path).read_text()
    for prefix in LOCAL.findall(text):
        text = text.replace("$p/", prefix.rstrip("/") + "/")
    return set(SOURCE.findall(text))


installer = named("hyprforge")
package = named("packaging/arch/PKGBUILD")
problems = []
for path in sorted(installer - package):
    if path not in DELIBERATE:
        problems.append(f"./hyprforge installs {path}; packaging/arch/PKGBUILD does not")
for path in sorted(package - installer):
    if path not in DELIBERATE:
        problems.append(f"packaging/arch/PKGBUILD installs {path}; ./hyprforge does not")
for path in sorted(installer | package):
    if not Path(path).exists():
        problems.append(f"{path} is installed but does not exist")
for path in DELIBERATE:
    if path not in installer | package:
        problems.append(f"install_parity.py excuses {path}, which neither names any more")

print(f"CHECKED {len(installer | package)}")
for p in problems:
    print(f"PROBLEM {p}")
