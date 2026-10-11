"""Running other programs, always with a bound.

CLAUDE.md: never wait on another process without a bound. Builds get a
long one (an hour), everything else a short one; a timeout comes back as
a result with `timed_out` set rather than an exception, so a step can
say "did not answer" instead of crashing the run.
"""

import os
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

SHORT = 60
BUILD = 3600


@dataclass
class Result:
    code: int
    out: str
    timed_out: bool = False

    @property
    def ok(self) -> bool:
        return self.code == 0 and not self.timed_out

    def lines(self) -> list[str]:
        return self.out.splitlines()


def run(cmd, *, cwd=None, env=None, timeout=SHORT, stdin=subprocess.DEVNULL) -> Result:
    """Run `cmd`, capturing stdout and stderr together."""
    full_env = None
    if env is not None:
        full_env = dict(os.environ)
        for k, v in env.items():
            if v is None:
                full_env.pop(k, None)
            else:
                full_env[k] = v
    try:
        p = subprocess.run(
            cmd,
            cwd=cwd or ROOT,
            env=full_env,
            stdin=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            errors="replace",
            timeout=timeout,
        )
        return Result(p.returncode, p.stdout)
    except subprocess.TimeoutExpired as e:
        out = e.stdout or ""
        if isinstance(out, bytes):
            out = out.decode(errors="replace")
        return Result(124, out, timed_out=True)
    except FileNotFoundError as e:
        return Result(127, str(e))


def have(program: str) -> bool:
    return shutil.which(program) is not None


def cargo_env() -> dict:
    """CARGO_BUILD_JOBS capped at four unless the caller set it.

    Sixteen linkers writing two-gigabyte test binaries at once stalled
    this whole machine once (disk-full-build-stall); four is the ceiling
    unless someone says otherwise.
    """
    return {"CARGO_BUILD_JOBS": os.environ.get("CARGO_BUILD_JOBS", "4")}
