# Window Rules — what's left

The Window Rules module's plumbing is done: TOML storage, Lua codegen, the
one-time `require()` setup flow, `hyprctl reload` on save, rule ordering,
enable/disable, and non-destructive draft editing. What's thin is *coverage* —
9 matchers and 9 effects out of a much larger surface — and there is no way to
build a rule from a window that's actually open.

These five are independent; the numbering is a recommendation, not a
dependency order. Field names must come from the Hyprland wiki
(Context7 `/hyprwm/hyprland-wiki`, `content/Configuring/Basics/Window-Rules.md`),
never from memory — see item 4 for why.

---

## 1. Build a rule from a window that's open

**Why.** Today you type a class from memory and hope. The app never asks the
compositor what's running, even though every other part of the suite works the
other way round — Displays shows you real monitors instead of making you type
connector names. This is vision pillar 6 ("learns by behavior, not tutorials")
and it's the single most error-prone step in the module: a typo'd class
produces a rule that silently never matches, with nothing to tell you so.

**The data is already there.** `hyprctl clients -j` returns, per window:
`class`, `initialClass`, `title`, `initialTitle`, `floating`, `xwayland`,
`fullscreen`, `contentType`, `tags`, `workspace {id, name}`, `monitor`. That
maps almost 1:1 onto `Matcher`, so the picker fills the form rather than
inventing a parallel concept.

**Changes.**
- New `hyprforge-windowrules/src/clients.rs`: a `Client` struct plus
  `list_clients() -> Result<Vec<Client>, ClientsError>` shelling out to
  `hyprctl clients -j` and deserializing with serde. Keep it in the library,
  not the GUI, so it's testable against captured JSON.
- Deserialize from a fixture, not a live compositor: commit a real
  `hyprctl clients -j` capture as a test fixture. The struct should ignore
  unknown fields, so a Hyprland update adding a key doesn't break the picker.
- In `modules/window_rules.rs`: a "Pick a window…" button next to the Class
  field opening a list of open windows (class + title + workspace, so two
  Ghostty windows are distinguishable). Choosing one fills `class` and
  `title` into the draft.
- Fetch async via the established pattern — `Task::perform(list_clients(),
  Message::ClientsLoaded)`, mirroring `fetch_modes` in
  `modules/displays.rs:1883`. Discard a late response if the draft closed, the
  way `ModesLoaded` checks `editor.selected` still matches.
- Not running Hyprland, or `hyprctl` missing, must degrade to the plain text
  fields with a note — never an error dialog. Same posture as
  `main.rs:104`.

**Design call to make.** Whether picking a window fills `class` alone (broad:
every Ghostty window) or `class` + `title` (narrow: this one). Suggest
class-only by default with title offered as a second click, since a title is
usually the volatile part.

**Verification.** Unit-test deserialization against the fixture, including a
window with an empty `initialTitle` and one with `tags`. Then, live: open the
picker with several windows up, pick one, confirm the generated Lua matches
that window and no other.

---

## 2. Workspace → monitor mapping (`hl.workspace_rule`)

**Why.** "Steam on the external display" still can't be expressed. Per-window
`monitor` lives in the static-effects table, which is unverified (item 4), but
the workspace route is confirmed:

```lua
hl.workspace_rule({ workspace = "name:gaming", monitor = "desc:...", default = true })
```

This is the better design anyway. It composes with the Displays module, which
already fingerprints monitors by EDID identity — so a mapping can survive a
replug in a way a raw `DP-3` string cannot.

**Changes.**
- `model.rs`: a `WorkspaceRule { workspace, monitor, default, ... }` struct,
  separate from `Rule` — different Lua call, different semantics, and mixing
  them into one list would make ordering meaningless.
- `storage.rs`: a second array in the same TOML (`[[workspace_rule]]`
  alongside `[[rule]]`). `RuleFile` gains a field; `load`/`save` signatures
  change to carry both. Keep one file — it's one user-facing concept.
- `codegen.rs`: emit `hl.workspace_rule({ ... })` entries. Order relative to
  window rules doesn't matter to Hyprland, but emit them first for
  readability.
- GUI: a second section in the module listing workspace→monitor mappings, with
  the monitor chosen from a dropdown of what displayd currently knows rather
  than typed. `DisplaydProxy` already exposes the connected set.
- Resolve the identity question: store `desc:<EDID description>` (survives
  replug, matches how Hyprland addresses monitors) rather than the connector.

**Verification.** Codegen tests for each field combination. Live: map a
workspace to the external, replug it, confirm the mapping still applies.

---

## 3. The rest of the effects

**Why.** Mechanical volume. The complete **dynamic** effects table is verified
and in hand, so this is safe, repetitive work — no research needed.

**The ten worth doing first:** `opaque`, `no_anim`, `no_focus`, `stay_focused`,
`dim_around`, `keep_aspect_ratio`, `border_size`, `min_size`/`max_size`,
`animation`, `idle_inhibit`.

Types are known: booleans except `border_size` (integer), `min_size`/`max_size`
(vec2, e.g. `{ 800, 600 }`), and `animation`/`idle_inhibit` (strings —
`idle_inhibit` takes `none`/`always`/`focus`/`fullscreen`, so a dropdown, not
a text field).

**Changes.** Per field, the same four edits: an `Option` on `Effects`, a
codegen arm, a `RuleDraft` field with its `Message`, and an entry in
`fully_populated_rule()`. Note `no_border` does **not** exist — use
`border_size = 0`.

**Watch out.** `fully_populated_rule()` in `modules/window_rules.rs` is what
makes `draft_round_trips_every_field` meaningful. A field added to the model
but not to that fixture makes the test pass vacuously while the field silently
erases itself on the user's next edit — which is exactly the failure the
round-trip invariant exists to catch. Extend the fixture in the same commit as
the field.

**Verification.** The existing round-trip test, plus a codegen assertion per
field.

---

## 4. The static effects (blocked on verification)

**Why blocked.** `pin`, `fullscreen`, `maximize`, `center`, per-window
`monitor`, `no_initial_focus`, `suppress_event`, `no_close_for` live in a
table I could not retrieve — Context7 caps at 3 queries per question, and the
dynamic table consumed the budget. Their *existence* is confirmed; their exact
field names and argument types are not.

**Do not guess.** Generating a plausible-but-wrong field name fails silently:
Hyprland ignores what it doesn't recognise, the rule looks saved, and nothing
reports that it does nothing. This is the same shape as the scale bug — a
setting that disagrees with reality and says nothing — and it cost several
rounds to find. `no_initial_focus` and `no_focus` are the only two seen
verbatim (in the wiki's xwaylandvideobridge example), and that example is in
the pre-0.55 block syntax, so even those want confirming in Lua form.

**Changes.** First a fresh Context7 query for
`content/Configuring/Basics/Window-Rules.md`'s static effects table. Then
implement as in item 3. If the query doesn't return the table, the fallback is
reading Hyprland's own source for the rule parser — slower, but authoritative.

---

## 5. Prove the generated Lua actually loads

**Why this outranks its position.** Every test asserts on strings we generate;
nothing has ever confirmed Hyprland *accepts* them. That's precisely the gap
the scale bug lived in — arithmetic self-consistent, compositor disagreeing,
no test able to tell.

**There is a specific reason to think this is already broken.**
`main.rs:93` documents that `hyprctl` exits 0 even when the Lua call itself
errored, and reads the response body instead. But `apply.rs` checks only
`output.status.success()` on `hyprctl reload`. If `reload` behaves the same
way, a syntax error in our generated file is reported to the user as a
successful save — a dead end of exactly the kind vision pillar 3 forbids.
**Verify this first; it may be a bug fix rather than a test.**

**Changes.**
- Establish what `hyprctl reload` returns on a bad config: exit code, stdout
  body, stderr. Do this against a scratch `XDG_CONFIG_HOME`, never the live
  config.
- If it reports failure only in the body, fix `apply()` to check the body —
  same treatment `main.rs` already gives `dispatch` — and surface the parse
  error verbatim in the GUI.
- Add `crates/hyprforge-windowrules/tests/live_lua.rs`, `#[ignore]`d, modelled
  on `hyprforge-displayd/tests/live_smoke.rs`: generate a file exercising every
  supported field, load it, assert no parse error. Not in the default run —
  it needs a live compositor — but it's the only test that can catch a field
  name we invented.

**Verification.** Deliberately corrupt a generated file, confirm `apply()`
returns an error and the GUI shows it. Then run the live test with every field
populated and confirm a clean load.

---

## Not planned here

Rule templates ("make this app a scratchpad"), import of existing hand-written
rules (ruled out by the never-round-trip-the-user's-config pillar), and
per-rule preview of the generated Lua. Worth revisiting once the above lands.
