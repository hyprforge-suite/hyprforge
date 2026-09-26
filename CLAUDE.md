# Working on Hyprforge

A suite of native Hyprland desktop apps. One cargo workspace, layered so that an
app which has never heard of Hyprland can still use the bottom of it. Start with
the "Workspace layout" section of `README.md`; `hyprforge-vision.md` has the
whole-suite scope.

## The rules that came from being wrong

Each of these cost real debugging. They are here because the mistake was not
obvious in advance.

**Never write anything derived from a keystroke to a log, a panic message or a
debug dump — keysyms included.** A keysym name *is* the character: `XK_a` is
`a`, `XK_comma` is `,`. "Just the keysym" writes the password to disk in a
barely-encoded form, and it reaches the session transcript through tool output.
`Debug for State` is hand-written to render what was typed as `<N chars>`; use
that. To debug input, log the character *count* or whether `utf8` was `Some` —
never the value.

**Measure before optimising anything that talks to another process.** A
suspiciously round delay is someone else's timeout, not slowness here. A
five-second gap before the lock screen appeared was `ext-session-lock-v1`
waiting for surfaces this code was withholding until `locked` arrived; our side
finished in 1ms. Nothing in the test suite caught it, because every test asked
*what* happened and none asked *how long*.

**Test the resource, not just the result.** A 36-megapixel wallpaper peaked at
296MB in the lock screen, and an allocation failure there is unrecoverable —
it caches as "no entry" in `iced_tiny_skia` and panics on the next frame. Ask
what a change allocates, not only whether it works.

**Never collapse "this file could not be read" into "there is nothing
configured".** A missing file is first-run; a file that exists and will not
parse is a problem the user has to hear about. Collapsing the two once cost a
user 37 hand-written binds. `hyprforge-core::hlconfig::storage` documents the
distinction; `look::resolve` follows it by warning and carrying on.

**Never wait on another process without a bound.** `Command::output()` waits
forever, and every external program here — `hyprctl`, `gsettings`, `fc-list` —
can stop answering. Use `hyprforge_core::command::output(.., command::TIMEOUT)`,
or `tokio::time::timeout` in async code. A timeout arrives as
`io::ErrorKind::TimedOut`, so callers that already handle a failed spawn need no
new branch: not answering and not starting are the same problem from their side.

**A shared `Theme` does not give you a shared look — the renderer can
change the colour underneath it.** iced 0.14 enables `web-colors` by
default, which skips the sRGB->linear conversion on the `iced_wgpu`
path. `iced_tiny_skia` has no such feature. So the greeter and the
Settings app drew `rgba(26263aff)` as (107, 107, 130) while the lock
screen drew it as (38, 38, 58) — one extra sRGB encode, and a login
screen visibly washed out beside a lock screen reading the same file.
Nothing in the type system or the tests can see this: both hosts report
the same `Color` and only the pixels differ. `check.sh` asserts the
feature stays off; comparing a screenshot's centre pixel against the
theme value is how it was found, and is the way to settle any
"these two should look the same" question.

**In a last-one-wins format, flagging the duplicate that *wins* deletes the
value that applies.** Every `invalid()` here doubles as a filter: `generate`
skips whatever it reports. So "which row do I mark as the dead one" is not a
wording choice — mark the last occurrence and the generator drops the row in
effect and writes the superseded one instead. `hl.env` did this (editing a
variable and leaving the old row made the *old* value take effect) and so did
hyprpaper's blocks (a new wallpaper left the old image on screen). Both existing
tests asserted the wrong index, one of them contradicting its own doc comment.
`core::supersede` owns the rule now; a test that only checks *that* something
was flagged will not catch this, so assert the generated value too.

**`pkill -f <pattern>` matches the shell running it**, because the pattern
appears in that shell's own command line. It kills the shell and exits 144. Use
`pkill -x <name>` or a literal PID.

`pgrep -f` has the identical problem and is easier to miss, because it looks
like the safe half of the pair — it only *reads*. But
`for p in $(pgrep -f "http.server 8731"); do kill "$p"; done` puts the pattern
in the command line of the very shell running the substitution, so the loop
kills that shell: still exit 144, now with the search and the kill in different
words. When the process is a server, ask the port instead of the process table:
`ss -lptnH 'sport = :8731'` names the pid without any pattern to match against.

And `pkill -x`/`pgrep -x` is not the safe fallback it looks like: a process name
longer than **15 characters** is truncated in `/proc/<pid>/comm`, so
`pgrep -x hyprforge-settings` matches *nothing at all* and reports success doing
it. That left a stale window running beside the one under test. `hyprforge-lock`
fits in fifteen; `hyprforge-settings` and `hyprforge-displayd` do not. When the
name is too long, or when the real session is running a copy of the same binary,
match on `/proc/<pid>/cmdline` and `/proc/<pid>/environ` instead — the nested
instance is the one whose environ says `WAYLAND_DISPLAY=wayland-2`.

**iced reports three things for a key press and only one of them is what was
typed.** `key` is the *unmodified* logical key, `modified_key` has modifiers
applied, `text` is what the press produced. Reading `key` turns `SHIFT + j` into
`j`, so a password loses every capital and symbol, PAM rejects a password the
user typed correctly, and each attempt spends a `faillock` slot. Use `text` for
character input and `key` only to recognise Enter, Escape and Backspace — which
produce text of their own (`\r`, `\u{1b}`, `\u{8}`) and must never reach a
password field.

**`hl.exec_cmd` runs while the config is being parsed**, which is before Hyprland
accepts clients. Anything it launches that needs a Wayland connection has to wait
for `$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY` to exist first. Losing that race made the
greeter exit silently with status 0, which greetd then reported as
`conversation failed` for a password nobody had been asked for.

**Find a Hyprland instance with `hyprctl instances -j`**, keyed on `wl_socket`.
Picking the newest directory under `/run/user/*/hypr/` can hand you the real
session, because it writes its log continuously. Also: `hyprctl keyword` does
not work against a Lua config — use `hyprctl eval`.

**A test that cannot run must not report as one that passed.** libtest has no
skipped state: a test that returns early prints `ok`, indistinguishable from
one that asserted something. With hyprpaper running, a parse test passed
having verified nothing at all. The fix is the `HYPRFORGE-SKIP:` marker
convention — a skipped check `eprintln!`s `HYPRFORGE-SKIP: <reason>` before
returning early, in `crates/hyprforge-ecosystem/tests/generated_configs_parse.rs`
and `crates/hyprforge-network/tests/live_networkmanager.rs` — and `check.sh`
greps `--nocapture` output for the marker and reports each one in yellow, so
a green run still shows what it didn't actually check.

The other half of the same mistake is gating a check on the wrong thing. The
parse tests rode tier 2's one `--ignored` run, so they needed *Hyprland*
running to ask *hyprpaper* a question, and on a machine with the daemons and
no compositor they never ran at all. Splitting them out fixed it — and then
the first live NetworkManager test went straight back into that same run,
gating a check on the network service on whether a compositor was up. Every
tier gates on the thing it actually asks. When you add a live test, the
question to answer before writing it is *what has to be running for this to
mean anything*, and that is rarely what the tier above it needed.

**`git subtree split` without `--rejoin` silently breaks the round trip.**
A plain split produces a good branch and records nothing here, so a change
made in the component repository cannot come back: `git subtree pull` finds
no common ancestor and stops with `fatal: refusing to merge unrelated
histories`. Outbound works, inbound does not, and nothing says so until the
first outside pull request — the exact event the split exists to enable.
`--rejoin` leaves a merge commit here (`Split 'crates/<crate>/' into commit
'<sha>'`) recording which commit the split produced, so a later pull has an
ancestor to find. `split.sh` does this; the rule exists because a hand-run
`git subtree split` does not.

**A `LICENSE` symlink dangles in a split repository.** The foundation crates
symlink `LICENSE` to the repo root, and that is correct for them — `cargo
package` dereferences a symlink into the tarball. Git carries a symlink as a
symlink, so the moment a crate becomes a repository root, `../../LICENSE`
points above it and resolves to nothing. The first split produced an MIT
project with an unreadable licence. A crate that will become a repository
needs a real file; `split.sh` refuses to split one that is still a symlink.

**Cargo keys a git source on the URL string, so every manifest must spell it
identically.** A standalone crate that depends on another standalone crate
resolves two manifests — its own, and the depended-on crate's as fetched
from inside its own repository. Two spellings of the same repository read
as two different sources, and the second one gets fetched over the network.
`hyprforge-settings` depends on `hyprforge-tray`, which names the URL in its
own manifest, and that is the first pair with this shape. The URL is allowed
to change; it has to change everywhere at once, which is what `split.sh`
checks before it will split anything.

**A sibling named by git URL and missing from the root `[patch]` table
builds from GitHub, not from this checkout — and nothing fails.** Cargo
fetches it at the lockfile's commit and builds against that. After the
files, displayd and emojimenu split, seven siblings were missing at once:
Files built its archive, file-operation and browser crates from a days-old
snapshot, and the tray and clipboard their popup shell, so every local fix
to those crates — the archive race fix included — never reached the
binaries or the tests that exercised them. It surfaced only as a type
mismatch while merging a branch that changed one of them. `cargo tree
--workspace | grep github.com` is the instrument; `check.sh`'s "Siblings
build from this checkout" step now refuses a git-named sibling the table
does not redirect.

**A git dependency on a crate that still inherits from the workspace
resolves fine.** Cargo clones the whole repository, so the workspace root
comes along with it. Only the crate being *extracted* needs a self-contained
manifest — this was assumed to be otherwise, and the assumption made the
remaining splits look far more expensive than they turned out to be.
`hyprforge-settings` pulls in fifteen Hyprforge crates and none of them
needed a single change.

**A standalone crate's hand-copied dependency versions drift with no
symptom.** A crate that is its own repository root cannot inherit
`[workspace.dependencies]`, so it spells every version and feature out by
hand. If the root bumps one and the copy is not updated, cargo resolves
both, clippy stays silent and every test still passes. Dropping `features =
["derive"]` from the clipboard's serde — version left alone — was proved
invisible to all of tier 1 before the check existed. `check.sh`'s "Standalone
crate dependency pins" step exists for exactly this, and it discovers which
crates to check by their shape (no workspace inheritance left at all), so a
newly prepared crate is covered automatically rather than silently missed.

**Four separate things go wrong when CI fetches a private git dependency,
and each one reports as if it were the last.** Worth one entry, written as
the sequence it actually was, because the error text moved every time and
read as though nothing had changed:
- An Actions `GITHUB_TOKEN` is scoped to the repository its workflow runs
  in and grants nothing on another private repository in the same account —
  first failure, no credentials reached GitHub at all.
- Cargo fetches git dependencies with libgit2, which does not honour
  `url.<..>.insteadOf`, so a credential configured that way never reaches
  the clone and the exact same "failed to authenticate" comes back even
  though the token and the rewrite were now both correct.
  `CARGO_NET_GIT_FETCH_WITH_CLI: true` makes cargo shell out to the `git`
  binary instead, which does honour it.
- `x-access-token:<token>` is the shape of a GitHub App installation token,
  not a personal access token, and GitHub answers "Invalid username or
  token. Password authentication is not supported." The `AUTHORIZATION:
  basic` extraheader that `actions/checkout` installs for itself works for
  both kinds, passed through `env:` so it never sits on a command line a
  failing step would echo.
- A fine-grained PAT scoped only to the automatic `Metadata: Read-only`
  authenticates but cannot clone, and GitHub reports that shortfall as
  `remote: Write access to repository not granted` with a 403 — naming the
  wrong permission entirely, on a fetch, which sends you looking for a
  problem that is not there. It needs `Contents: Read-only`.

**`gh secret set` accepts an empty value without complaint, and `gh secret
list` then shows the name as if it were set.** A paste that did not take
produced a secret that existed and was empty, and CI failed for a
completely different-looking reason than "no secret". The tell is the
runner's own env dump: GitHub prints `***` for a populated secret, so a bare
`NAME:` with nothing after it means the value never arrived.

**`flush` is not delivery, and a process that exits after flushing loses the
work.** Synthesising a paste through a virtual keyboard reported success and
delivered nothing: `connection.flush()` puts the key events on the socket and
does not wait for the compositor to read them, so a popup that pastes and
exits destroys its virtual keyboard while those events are still unread.
Measured against a window that captures raw bytes — exit immediately: 0
bytes; wait 500ms: the paste arrives. A round trip is the fix, not a longer
wait: a compositor cannot answer a sync until it has processed everything
queued before it, so the reply is proof the keys were taken, where a sleep
would only be a number tuned until it stopped failing. Bounded on its own
thread, because `roundtrip` has no timeout and a wedged compositor must not
hang a popup that already did the useful part of its job. `send_combo` in
`hyprforge-clipboard/src/wayland/keyboard.rs`.

**Check the instrument before trusting what it says about the thing.** Hours
went into "the keystroke never arrives," against a capture rig that
could not capture *real* keystrokes either — the terminal reading it back
was in canonical mode, so nothing reached `cat` until a newline. Every
measurement before that discovery was measuring nothing, and it sent the
paste investigation above through three wrong suspects before landing on the
actual bug. Validate the measuring apparatus against a known-good input
first; a result this clean-looking is not evidence until the rig itself has
been made to fail on purpose.

**An installed binary is not a restarted daemon.** Twice in one day a stale
daemon produced a symptom that looked like a bug somewhere else —
`hyprforge-clipd` answering "unknown variant `set-clipboard`" to a popup
built against a newer socket protocol, which presented as the clipboard
silently not working, and `hyprforge-trayd` still advertising a menu it no
longer served. Both installs had genuinely succeeded; version skew across a
socket after an upgrade is normal, not exceptional, and a component that
answers but cannot do what is asked is a third case beyond "there" and "not
there". `./hyprforge --install` now restarts a user service whose binary it
just replaced, but only if the service was already running (starting
something the user chose not to enable is not this script's decision) and
only if the binary actually changed this run (so re-running the installer
does not interrupt a daemon for nothing) — never greetd, which is the login
manager and takes the session down with it.

**A property that says "none" is not the same as no property.** The tray
answered `/` for its `Menu` property, reading the root path as the spec's
way of saying there is no menu — an object path can't itself be null. waybar
doesn't read it that way: it saw a property, built a dbusmenu client against
the path, got no layout back, and drew an empty four-pixel GTK menu at the
pointer — and, believing it had served the click, never called
`ContextMenu`, which is the one place this suite's own popup gets launched
from. Omitting the `Menu` property entirely, which the StatusNotifierItem
spec allows precisely so an item can say it has none, is what sends a host
down the fallback path instead. The live test had asserted the property
equalled `/` and passed — a claim about our own code, sitting in the tier
whose whole purpose is claims about somebody else's; it now asserts that
reading the property *errors*, because it isn't declared at all.

**A popup that renders a 1x buffer on a scaled output is blurry, and nothing
in the code says so.** Every popup drew into a logical-pixel-sized buffer and
told iced the scale was 1.0; on a 1.6-scale output the compositor stretched
it, and it looked like a font problem. `wp_fractional_scale_v1` with
`wp_viewporter` is the fix, because it's the only pairing that matches 1.6
exactly — integer `set_buffer_scale` rounds it to 2 and is wrong differently.
The division that keeps this safe: the buffer is physical pixels, but the
widget tree, the clip rectangle and every hit-test stay logical, because
Wayland delivers pointer coordinates in logical surface space by contract —
scaling the buffer cannot move what a click lands on.

**Two icon names that both resolve can still look wrong together.** The tray
asked for the full-colour `network-wireless-signal-excellent` beside three
`-symbolic` names, so one colourful icon sat among three monochrome ones —
every name resolved, so the existing "does this name draw something" test
passed throughout. Fixing it took checking, not guessing: the instruction to
use Adwaita as a safe standard was wrong on this machine — the configured
theme's own `Inherits=` reaches `breeze-dark` and nothing else installed
here, so Adwaita was unreachable and every suggested name would have
resolved to nothing. The rule that now catches this class needs no live
theme at all: every icon name this daemon can emit must be `-symbolic`,
except a documented two-name allow-list for the one shared fallback state
that has no symbolic variant anywhere reachable. A second, live-only check
goes further and confirms a whole set resolves through the *same* installed
theme rather than merely somewhere in the chain — two names can each resolve
and still look wrong together if one falls through several levels of
inheritance to reach hicolor while its sibling is drawn directly.

**Publishing the components without publishing the monorepo tests them
against code nobody has.** Every standalone manifest names the monorepo's
URL for every sibling it needs — `hyprforge-settings` alone pulls fifteen
crates that way — so a component pushed while the monorepo is behind gets
built by its own CI against whatever was published days ago. The failure
names the component, not the stale dependency: `cannot find function
`update` in module `hyprforge_tray::prefs`` in a repository whose copy of
that function is right there in the diff you just pushed. `sync.sh`
checked five components and said nothing about the repository all five
depend on; it now checks and pushes the monorepo first, because a
component's CI starts the moment its push lands and resolves siblings
from there while it runs.

**A layer surface that leaves its exclusive zone at `0` is positioned in
what the bar left over, so a margin gets the bar's height added twice.**
Every placement in `hyprforge-popup` is in full-output logical
coordinates — a cursor position from `hyprctl`, or a monitor's `reserved`
top plus the user's own offset — and with the default exclusive zone the
compositor then measures that margin from the bar's *bottom* edge.
Measured with `hyprctl layers`: a bar occupying y 20..50 and a menu that
should open at y=50 opened at y=100. `set_exclusive_zone(-1)` — "do not
move me out of anyone's way" — makes a margin mean what the placement
code already meant by it. The clipboard and emoji popups had it too, and
nobody noticed, because a popup at the pointer looks plausible wherever
it lands. `hyprctl layers` is the instrument for any "why is this popup
*there*" question; it reports the surface the compositor actually
created, which is the only thing that settles it.

**An icon that asks a host to hide it is a control the user cannot
reach.** `Status::Passive` is the StatusNotifierItem spec's way of saying
"nothing to see here", and hosts obey it — so a Wi-Fi icon published
Passive while the radio was off disappeared at exactly the moment its own
menu (which offers to switch the radio back on) became useful. Every item
this daemon builds is one the user switched on in `tray.toml`; a host
hiding it is a decision nobody asked for. All four are `Active` in every
state now, with a test asserting it. The related trap: the *icon name* is
the entire vocabulary for "this is off" — SNI has no opacity and no
sensitivity flag, nothing a host is obliged to dim — so making a hidden
state visible means finding it a name of its own. Two states that shared
one name while one of them was invisible will look identical the moment
both are shown.

**A single-instance lock answers "someone already has it" — never "and
that is fine".** `hyprforge-traymenu` takes an `flock` so a keybind pressed
twice cannot open two popups, and a second copy that finds the lock held
exits quietly and successfully. That is right for the same menu asked for
twice and wrong for a right click on a *different* tray icon: the user is
asking for the Bluetooth menu while the Wi-Fi one is open, and gets nothing
at all until they press escape first. Whoever spawns the popup has to decide
which of those a refusal is, because the lock cannot tell them apart — so
`hyprforge-trayd` closes the menu it already opened before opening another.
It keeps the whole `Child` and not a pid, because the lock is released when
the kernel closes the dead process's descriptors and a zombie nobody waited
on still holds them; `Child::kill` signals *and* reaps, which is what makes
the new popup's own `acquire` succeed rather than race.

**A test that writes an executable and then runs it races every other
test that forks.** Three tests here write a stand-in script and exec it,
and they failed about one run in six of the whole crate while passing 25
times out of 25 on their own — which is the signature of a race with a
*sibling* test rather than a bug in the test. The cause is `ETXTBSY` and
it is a plain Unix rule, not anything specific to this code: while one
thread holds an executable open for writing, a fork in another thread
gives the child a copy of that write descriptor, and exec of that file
then fails with "text file busy". Serialising write-then-exec across
those tests fixes it; a retry loop would only hide it. The general
lesson is the one about flaky tests generally — "passes alone, fails in
the suite" is information, and it names concurrency with a sibling
rather than inviting a re-run.

**`hyprctl` reports logical coordinates; `grim` writes physical pixels.**
On this machine's 1.6-scale output those differ by more than half a
window, so cropping a screenshot at the box `hyprctl clients -j` gives
lands roughly two-thirds of the way up and left of the window you meant —
which on a tiled desktop is reliably some *other* application, rendered
convincingly enough to be mistaken for a bug in your own. Several rounds
of "the icons still look wrong" this session were looking at a terminal.
Multiply the logical box by the output's `scale` before cropping, and
check the window is on the *active* workspace first: `hyprctl dispatch
workspace` silently fails against a Lua config, so a switch may not have
happened. This is the same logical-versus-physical split
`hyprforge-popup` documents for buffers and hit-testing, arriving
through a different door — and it is another instance of the rule about
checking the instrument before trusting what it says about the thing.

**Never run a regex over source code to delete a block.** A non-greedy
`(?:[^\n]*\n)*?` looking for a sixteen-space closing brace finds the first
one anywhere below, and deeply-indented code inside the *next* function
matches long before the end of the block you meant. Deleting one dead match
arm this way silently took `view`, `title`, `theme` and `tabs_bar` with it,
and the brace count stayed off by only one, so the damage read as trivial
when half an `impl` was gone. Delete a block by matching its exact text, or
by slicing between two unambiguous markers you have actually read — and when
the code is uncommitted work you did not write, read `git status` and ask
whoever wrote it for the verbatim text rather than reconstructing behaviour
from inference.

**`git add -A` while an agent is working commits someone else's
half-finished thought under your commit message.** Two agents were
editing other crates when a commit here swept in six lines of one of
their in-progress files — a visibility change that was correct and
needed, but which the commit message said nothing about, because whoever
wrote that message did not know it was there. The agent noticed and
reported it; nothing else would have. A commit whose message does not
describe its contents is the thing this project's whole commit style
exists to prevent, and `-A` is how it happens by accident. Stage the
paths you actually wrote (`git add crates/<the one you touched>`), and
read `git status --short` before committing when anything else is
running.

**A name is a claim; the bytes are the thing — and the browser only
has the name.** Whether `x.zip` is an archive can only be settled by
reading it, and `Browser` does no I/O at all, so double-clicking one
navigates into it on the strength of its *name*. That guess is wrong
for a JPEG somebody renamed, and the wrong way to handle it is an
error where the folder should have been: `apply_dir_loaded` takes the
navigation back and hands the file to whoever opens it instead, which
is what would have happened had the browser known. Two consequences
worth keeping: the failed destination must not land on the *forward*
stack (a Forward button offering to retry what just failed), and
`FilesError::NotAnArchive` has to stay its own variant rather than a
message inside a general one, because it is the only failure here that
is acted on rather than shown.

**A `.gz` and a `.tar.gz` have identical magic bytes, and the extension
is not the tiebreak.** Plenty of tarballs are named `.gz` and plenty of
single files are named `.tgz`. The only honest answer is to decompress
the first 512 bytes and look for tar's `ustar` at offset 257, which is
what `format::sniff` does — one block, once. Getting this wrong does
not fail: it shows someone a single member called `linux-6.6.tar` and
looks like the archive was empty. The same module's other half is that
`Read::read` is allowed to return fewer bytes than asked for and a
decompressor routinely does, so a single `read` call sees 200 bytes of
a perfectly good tar and concludes from the 57 it was short that the
magic was not there.

**Every archive format is last-one-wins, so the member that counts is
the *last* one with a given name.** `tar rf` appends a second
`notes.txt` after the first and every tool that unpacks it writes both
in order, so the file left on disk is the later one. This is the same
rule `core::supersede` owns for `hl.env` and hyprpaper, arriving
through a different door — and getting it backwards here is invisible
in exactly the way that one was: the *listing* looks right, and only
the contents are of a version the archive supersedes.

**A path is not a file, and anything that opens one twice is reading two
files.** Every archive entry point sniffed the format with one open and
read with another, and extraction listed before it unpacked — so a rewrite
renaming a new archive over the name between them left an edit repacking
members it had never unpacked. Three failures in two hundred under load,
none in three hundred idle, which is why it surfaced as a flaky test in
`check.sh` and nowhere else. `hyprforge_archive::pin` opens once and reopens
through `/proc/self/fd/<n>`, which names the held file rather than whoever
has the name now. The trap on the way out: a pinned path has no *name*, and
a `.gz`'s one member is named after its file — extraction briefly wrote
`dump.sql` to disk as `9`. Whatever is derived from a name has to be given
the real one separately; only the bytes come from the pin.

**A library can be fetched; a binary cannot — so a package's binaries
must all live in one crate directory.** `git subtree split --prefix=`
takes one directory, so that directory *is* the published repository.
`hyprforge-tray` was split without `hyprforge-traymenu` and
`hyprforge-clipboard` without `hyprforge-clipmenu`, publishing a tray
daemon whose right-click spawns a program the repository does not
contain, and a clipboard daemon with no way to see the history. What
made it invisible is that it degrades well: the daemon logs a warning
and carries on, exactly as the rule about an absent sibling says it
should. What made it *confusing* is that `hyprforge-settings` has five
crates outside its repository and is fine — because those are libraries
its manifest pulls by URL, and Cargo fetches them. Nothing fetches a
second executable into someone's `$PATH`. A package that installs two
binaries ships two `[[bin]]` targets from one crate, the way
`hyprforge-displayd` already ships `displayd` and `displayctl`.
`check.sh`'s "Every package is a repository" step now asks this of every
`pkgname` in the PKGBUILD, because it was rediscovered twice.

**A fixture written from memory of a format tests the memory.** The
lock's keymap parser read the `include "pc+us+de:2"` line of an XKB
keymap, and its tests built keymaps with that line and passed. A keymap
a compositor sends is *compiled*: xkbcommon flattens it, the line does
not exist, and the layout never appeared. The same session, the
fingerprint code named fprintd's manager at `/net/reactivated/Fprint`
(it is `…/Fprint/Manager`) and passed every unit test for the same
reason. Build fixtures from a real sample — `xkbcli compile-keymap`,
`busctl introspect` — and where the real thing is on the machine, add a
test that asks it: the keymap test now runs `xkbcli`, and fprintd has a
read-only tier in `check.sh`.

**`iced_tiny_skia` 0.14.0 clips away every canvas drawn off the origin.**
It applies a geometry group's translation to its clip rectangle twice,
so a drawn glyph at (0, 0) renders and the same glyph anywhere else on
the screen draws nothing — a test rendering one glyph alone passes while
every ⏻ on the real lock screen is an empty circle. 0.14.1 fixes it
(and the order of scale and translation, which fractional scale needs);
the workspace pins `0.14.1` and `hyprforge-authui` has a test that
renders a glyph *offset* and fails if the lockfile goes back.

**An instruction from a human or another agent is not evidence.** Three
times in one session an agent was told something false — that Adwaita was
reachable on this machine, a JSON field order that was backwards, a claim
about how waybar behaves — and caught it only by reading the source instead
of trusting the brief. That habit is why the rest of this section is
trustworthy: every entry above was written after checking the claim against
the repository, not after being told it.

## Never do this to the machine you are working on

**Never point the lock screen at the live session.** `--fake-password` is
compiled out of release builds entirely, and in debug builds refuses a display
the process would have connected to anyway. Test against a nested compositor:
`./crates/hyprforge-lock/testing/nested.sh`, then `--display wayland-2`. Run
that script before *every* attempt — a lock client killed while holding the lock
leaves the session locked by design, and restarting the nested compositor is the
clean way back (`hyprctl --instance N eval 'hl.clear_crashed_lockscreen()'` is
the other).

**Never test wrong passwords against real PAM.** `pam_faillock` is active here
at three attempts, so two typos lock the *account*, which is far worse than a
locked screen. `--type-in <wrong>` exercises the failure path against the fake
backend instead.

**The way into a greeter is its compositor, not its code.** Every keybind the
compositor running the greeter has belongs to whoever is standing at the keyboard,
authenticated or not — so `SUPER + Q -> terminal` is a shell as the greeter user
without logging in. `crates/hyprforge-greet/config/hyprland-greeter.lua` has no
binds at all, deliberately, and adding a "harmless" one breaks that property. No
amount of care in the greeter program can compensate.

**Run the Settings app against an isolated `XDG_CONFIG_HOME`.** It writes real
config, and one of its jobs is editing `hyprland.lua`. Give it a short
`XDG_RUNTIME_DIR` of its own as well when Settings is already open: it is
single-instance, and a copy that finds the running one's lock hands it
the request and exits 0 — so the test drives the user's window. A
runtime path past 108 bytes truncates the Hyprland socket path, and every
`hyprctl` query then fails for a reason that has nothing to do with the
code.

**Do not start a second `hyprpaper`** to validate a generated file — it takes
over the IPC socket of the running one. Skip the check when it is running.

**This generalises: never start a second instance of one of this suite's own
daemons to inspect it.** The reason is sharper for `hyprforge-clipd` than it
was for `hyprpaper`, because its own design makes it the only writer to the
clipboard history file — there is no IPC socket to fail loudly on, just a
second process now sharing state with the first. Starting one to check for a
CLI flag connected it to the live clipboard. Read the source or `--help`
instead of running a second copy to find out.

## The shape a D-Bus-backed module takes

Four of these now exist — Network over NetworkManager, Bluetooth over
BlueZ, and the tray's item and menu over StatusNotifierItem and
dbusmenu — and they were each better for being built in this order. It is
written down so a fifth does not re-derive it.

**The backend trait and its mock come before the D-Bus client.** Not
after. The machine running tier 1 has no guaranteed NetworkManager, no
adapter, no access point and no bar, so everything above the trait is
testable only if the seam exists first. `hyprforge-displayd`'s
`backend/wlr.rs` is the counter-example: 595 lines with no tests, because
the mock arrived late and the protocol code never got a seam of its own.

**The model is plain data, and the decisions are pure functions over it.**
"Which icon does this state deserve", "which rows does this menu need",
"is this network WPA3" — all of it belongs above the bus, where a test
can reach it. What is left in the client is marshalling, and marshalling
is what the live tier is for.

**Every service gets its own tier in `check.sh`, gated on the thing it
actually asks.** Not on the compositor. See the rule above about a check
that silently never runs.

**The live tier is read-only.** It runs on a machine somebody is using,
quite possibly over the connection or the bar being inspected. A test
that can drop your Wi-Fi, unpair your mouse or start a discovery session
is not worth the coverage. The tray is the one exception worth naming:
registering really does put an icon in the user's bar for a fraction of a
second, and that is the smallest observable form of "a host accepted it".

**Ask what only the real service can answer, and assert that.** Not that
a call returned. `Strength` is a percentage; `NM_DEVICE_TYPE_WIFI` is
still 2; the watcher *lists* the item rather than merely accepting the
registration; `GetLayout`'s reply deserializes as `(u(ia{sv}av))`.
Every one of those is a claim this code makes about somebody else's, and
the compiler cannot see any of them.

**Two failures that always need naming, in both directions.** The daemon
not running is not an empty list — it is its own state with its own
message and a way out of it. And a connection that failed is never
cached, because the message telling the user to start the service keeps
being shown after they do. That one has been got wrong three times.

## Every component runs alone, and is better together

This is a suite, and someone must still be able to install only the
clipboard. Those pull against each other only if "shared at build time"
is confused with "required at run time". They are different things.

**Shared at build time** is what makes it a suite. One
`hyprforge_look::Theme`, one widget vocabulary, one keyboard grammar —
and one place to fix them, which is why `web-colors` was a single
`default-features = false` rather than the same edit in five apps.

**Split at install time** is what makes it honest. `packaging/arch`
builds one source tree into separate packages, so `hyprforge-clipboard`
installs two binaries and nothing else. Packaging granularity and
repository granularity are different questions, and only the first
decides what a user can install.

**So every component has to work with its siblings absent.** That is the
rule, and it is the one that breaks quietly:

- A missing `appearance.toml` is first-run, not an error. The clipboard
  popup draws correctly themed on a machine where the Settings app has
  never been installed, let alone run.
- A service that is not running is a state with a message, never a
  failure. The tray says "NetworkManager isn't running" and keeps its
  other icons.
- A sibling binary that is not installed is a logged warning. Clicking a
  tray icon when `hyprforge-settings` is absent fails to spawn and says
  so; it does not take the daemon down.

The test for a new component is: **install only this package on a clean
machine. Does it work?** If it needs another Hyprforge component to
start, that dependency belongs in the package metadata or the need
belongs in the design — not in a user's surprise.

## Checking your work

```
./check.sh          # everything available on this machine
./check.sh --quick  # tier 1 only: clippy + unit tests, no compositor
```

Clippy must be silent and every test must pass before a commit. Fifteen
gated tiers beyond tier 1 now, each answering a different "does the system
I'm talking to actually agree" question — Hyprland itself, the ecosystem
daemons' parse tests, the system's own `unzip`/`tar`/`7z`, NetworkManager,
BlueZ, hyprsunset, systemd-logind, trash entries written by another
implementation, UPower, power-profiles-daemon, fprintd, the Wayland clipboard, icon
names against the installed theme, the installed shared MIME database, and
a tray host — and each gates on the thing it actually asks rather than
riding another tier's `--ignored` run, for the reason in the
rule above about a check that silently never runs. Tier 1 now also includes
the "Standalone crate dependency pins" step, which compares every split-ready
crate's hand-copied dependency versions against the workspace table — see the
rule above about drift with no symptom — "Siblings build from this
checkout", which fails on a sibling named by git that `[patch]` does not
point back at `crates/` — and "Docs name things that exist",
which fails on a doc naming a source file that is gone or a repository count
that no longer matches, on a Settings page the Settings README does not name
or miscounts, and checks this paragraph's own tier count. The
judgement half of keeping docs true is the `keep-docs-current` skill in
`.claude/skills/`; run it before committing anything that changes what a doc
counts, names or calls unfinished.

`check.sh` does **not** cover the lock screen's live behaviour. Its unit tests
run in tier 1, but proving it locks, draws and unlocks needs the nested
compositor, by hand.

Nor does it cover Files' copy and paste *between applications*: those tests
write a clipboard, so they run only against the nested compositor, and refuse
the session's own display. Start it with `nested.sh`, then
`HYPRFORGE_TEST_NESTED_DISPLAY=wayland-2 cargo test -p hyprforge-files --test
nested_clipboard -- --ignored --test-threads=1`. The live clipboard tier that
`check.sh` does run stays read-only.

`./hyprforge --install` restarts a running user service whose binary it just
replaced — see the rule above about an installed binary not being a
restarted daemon. `--no-restart` opts out for anyone mid-something who wants
the files now and the restart later; it never touches greetd, which
`crates/hyprforge-greet/INSTALL.md` covers committing to deliberately, from
a spare VT.

`./split.sh <crate-name>` extracts one component into a branch that can
become its own repository, history intact, and refuses to run until the
crate is actually ready to leave (see the rules above about the manifest,
the `LICENSE` symlink, and the git URL). `./sync.sh` reports whether each
already-split component's published repository still matches what this
monorepo would produce, and `./sync.sh --push` brings the ones that have
drifted back into sync — see repo-plan.md for both.

## What the layering is for

```
hyprforge-paths     no dependencies at all
hyprforge-process   a bounded subprocess wait (Command::output that gives up);
                    no dependencies at all — a client of NetworkManager, BlueZ
                    or systemd-logind needs this and nothing Hyprland-shaped
hyprforge-look      Color + the runtime Theme; no iced, because the lock screen
                    and greeter paint into a raw Wayland buffer
hyprforge-mime      the freedesktop shared MIME database: what a file is,
                    what opens it, what the default is. A leaf; depends only
                    on hyprforge-paths
hyprforge-archive   zip, tar and 7z: what is inside one as a directory tree,
                    extracting from it, and rewriting it. A leaf with no
                    Hyprforge dependency at all — paths arrive from the
                    caller, it never goes looking for one
hyprforge-keys      the keyboard grammar every app binds keys through; no iced
hyprforge-listing   a directory listing and its order, under both Files and
                    the image viewer so they agree which picture is next
hyprforge-image     bounded, orientation-correct decoding; no iced, no Wayland
hyprforge-ui        the iced layer; knows nothing about Hyprland
hyprforge-core      Hyprland config machinery — a new app should never need it
```

`hyprforge-core` re-exports `hyprforge-process` as `hyprforge_core::command` so
the ~20 existing call sites did not have to churn when it moved out; new code
should depend on `hyprforge-process` directly.

`hyprforge-ui` and `hyprforge-look` are **leaves**. Neither may depend on
`hyprforge-appearance`: `settings -> appearance` already exists, so that edge
would cycle. Each app's composition root does the joining.

**Never add a colour constant to an app.** Every app reads one
`hyprforge_look::Theme`, resolved from settings the user already controls — the
accent is Hyprland's `general:col:active_border`, fonts come from gsettings. If
a colour is missing, it belongs in the Theme. The Settings app and the lock
screen each grew their own palette once and drifted to different accents, which
is the exact failure this suite exists to prevent, occurring inside the suite.

Do not resolve the accent from the catalogue default: `general:col:active_border`
defaults to white, which is a checked claim about what Hyprland does, not a
design choice.

## Writing code here

Match the surrounding style, which is unusually comment-heavy on purpose:
comments explain *why*, especially where the obvious thing is wrong. If a line
looks like a bug and is not — the lock screen's buffer needs no channel swizzle
because `iced_tiny_skia` writes BGRA deliberately — say so where someone would
otherwise "fix" it, and add a test that fails if they do.

Tests are named as sentences describing the property, not the function under
test. Prefer a test that pins a decision (`a_failure_never_blames_the_password`)
over one that exercises a code path.
