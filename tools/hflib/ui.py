"""Step / ok / bad / skip, and the summary that decides the exit status.

The same vocabulary check.sh had, so a run reads the same: a step is a
question, each answer under it is a tick, a cross or a yellow dash. A
skip is never silent and never a pass — see CLAUDE.md on a test that
cannot run reporting as one that passed.
"""

import os
import sys

_COLOUR = sys.stdout.isatty() and os.environ.get("NO_COLOR") is None
BOLD, DIM, RED, GREEN, YELLOW, OFF = (
    ("\033[1m", "\033[2m", "\033[31m", "\033[32m", "\033[33m", "\033[0m")
    if _COLOUR
    else ("",) * 6
)

FAILURES: list[str] = []
SKIPS: list[str] = []


def step(title: str) -> None:
    print(f"\n{BOLD}==> {title}{OFF}", flush=True)


def ok(message: str) -> None:
    print(f"  {GREEN}✓{OFF} {message}", flush=True)


def bad(message: str) -> None:
    print(f"  {RED}✗{OFF} {message}", flush=True)
    FAILURES.append(message)


def skip(message: str, why: str) -> None:
    print(f"  {YELLOW}–{OFF} {message} {DIM}({why}){OFF}", flush=True)
    SKIPS.append(message)


def detail(lines, limit: int = 20) -> None:
    """Indented bullet lines under the last answer, at most `limit`."""
    lines = [l for l in lines if l.strip()]
    for line in lines[:limit]:
        print(f"    • {line}", flush=True)
    if len(lines) > limit:
        print(f"    {DIM}… and {len(lines) - limit} more{OFF}", flush=True)


def note(message: str) -> None:
    print(f"  {DIM}{message}{OFF}", flush=True)


def summary() -> int:
    step("Summary")
    if FAILURES:
        print(f"  {RED}{len(FAILURES)} check(s) failed:{OFF}")
        for f in FAILURES:
            print(f"    • {f}")
        return 1
    if SKIPS:
        print(f"  {GREEN}Everything that ran passed;{OFF} {YELLOW}{len(SKIPS)} not checked (above, in yellow).{OFF}")
    else:
        print(f"  {GREEN}Everything passed.{OFF}")
    return 0


def die(message: str) -> "NoReturn":  # noqa: F821
    print(f"{RED}✗ {message}{OFF}", file=sys.stderr, flush=True)
    raise SystemExit(1)
