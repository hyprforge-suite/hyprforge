# Building the image viewer

*Renamed on 2026-09-27: the crate, binary and package are
`hyprforge-media`, because a viewer of pictures, videos and 3D models is
not a photo viewer. "photos" below is the name it had while this plan
was being carried out; the paths are current.*

**Status, 2026-09-23.** Phases 0 to 3 are built: packaging, the window,
the viewer grammar and the actions that touch files. The three open
questions at the end were answered by the owner — PNG, JPEG, WebP and GIF;
Set as Wallpaper in scope; the keyboard grammar merged as
`hyprforge-keys`. Phase 4 is built too: the file manager's preview pane
and its grid thumbnails both decode through `hyprforge-image`. What is
left of it is the spacebar Quick Look overlay. Two gaps are named rather than hidden: an
animated GIF plays its first frame only, and zooming past the size that
was decoded upsamples rather than re-decoding a sharper crop.

**2026-09-25: the shell.** The `Hyprview Photo Viewer` design moved the
window into the file manager's shell — header, Places sidebar, status
bar — with Photo, Grid and Library as modes of one window, a docked
inspector reading the camera's EXIF (`hyprforge_image::camera`), and a
slideshow. That goes past "What the viewer is, in v1" below, on
purpose and in one direction only: the Library is the folder tree the
user already keeps, summarised as cards, so it is still not a
catalogue and still cannot disagree with Files. The editor the same
design draws (crop, straighten, exposure) is not built, for the reason
Phase 3 gives about writing to someone's original; its toolbar button
is shown disabled. The mode switch is drawn in the quiet style rather
than the design's purple, because purple means *selected* here.

**2026-09-26: 3D models.** The viewer took over view3d, the project
author's separate STL/3MF/OBJ viewer, rather than the suite gaining a second
window for it. Its loaders, mesh and camera became `hyprforge-mesh`, a
leaf like `hyprforge-image`; its wgpu renderer moved into this crate
(`src/model/`) as an iced `shader` primitive, because its wgpu has to be
iced's (27, where view3d was on 30). iced's shared render pass has no
depth buffer, so the primitive draws its own pass with its own depth
texture. fstl's teal backdrop became the theme's root surface, handed in
as a uniform and linearised for an sRGB target — the centre-pixel check
reads (25, 26, 33) in the model view and the photo view alike. Porting
the camera turned up a sign error in view3d's zoom-about-the-pointer
that made the point under the cursor slide away; a property test pins
the fix. Not carried: `--screenshot`, and 3D under iced's software
renderer, where the pane says it needs the GPU.

**2026-09-26: videos, the compact viewer, real thumbnails.** A picture
opened on its own opens compact — no sidebar, no inspector — floating on
Hyprland and sized to the picture (`float.rs`); Grid or Library brings
the shell back. Videos play in the pane through libmpv via the new
`hyprforge-video` (the Video Viewer row of the vision doc, folded in
here rather than built as a second app). The first try drew a fresh
`image::Handle` per frame and showed nothing while playing: iced uploads
any image of 2MB or more on a worker thread and draws nothing until it
lands, so a frame replaced every 33ms was never drawn. `film.rs` writes
each frame into one texture instead. Tiles now show a video's first
real frame and a model's drawing, through the shared cache at the
`large` size, which `hyprforge-thumbnails` gained for this. A second
pass fixed what playing turned up: the player thread took one message
per frame drawn while mpv signalled faster than that, so closing hung
for seconds, scrubbing lagged and mute was unreliable; each turn now
drains the channel and collapses a run of seeks into one
(`hyprforge-video`'s `player::batch`). Video, like 3D, needs the GPU:
under iced's software renderer the pane says so.

**2026-10-07: the panes leave the app.** Files' Quick Look wanted a
video that plays and a model that turns, and the honest way to give it
both was not a second copy: `film.rs` and `model/` moved into a new
library, `hyprforge-viewer`, with the player's and the camera's input
handling around them (`VideoPane`, `ModelPane`). Media now draws those
panes and keeps only what is its own — paging, the slideshow, the
inspector, autoreload. The entries above that name `src/film.rs` and
`src/model/` describe where the code was written.

**2026-09-27: where the plan below turned out different.** Read the
rest of this file as the plan it was; these are the places the code
went another way.

- The next/previous order does not come from
  `hyprforge_files_core::sort`: the listing and its order moved into
  `hyprforge-listing`, a leaf both Files and this crate depend on, so
  photos never took the dependency on `files-core` that "What the viewer
  is" argues for (`src/order.rs`).
- There are more than two crates. Besides `hyprforge-image` the viewer
  sits on `hyprforge-keys`, `hyprforge-listing`, `hyprforge-mesh`,
  `hyprforge-video` and `hyprforge-thumbnails`; `hyprforge-image` itself
  also depends on `exif`, for the inspector's camera facts.
- The formats went further than open question 1 recommended: the
  workspace root enables PNG, JPEG, WebP, GIF, BMP, TIFF and ICO in one
  table. AVIF stays off; HEIC `image` cannot do at all.
- Zooming past the decoded size still upsamples. What exists is a
  sharper re-decode when the *window* grows (`sharpen_if_needed`), not
  the crop-at-native-resolution the budget section plans.
- No golden-image test renders the viewer headlessly; the centre-pixel
  comparison was done by hand (see the 3D entry above).

`hyprforge-media`, the "Photo Viewer" row the inventory in
`hyprforge-vision.md` carried as *not started* when this was written. This is the plan for
it, written the way `repo-plan.md` is: the decisions, what each one costs,
and the things that were checked rather than assumed.

The brief is "it should feel like it comes from the same suite as the file
manager". That is not a look — the shared `Theme` already gives it the
look for free. It is a set of seams the file manager has and a viewer
would otherwise grow differently, so the first section is those, and the
rest of the plan is arranged around keeping them.

## What "same suite" actually means here

Six things `hyprforge-files` does that this must do identically, each one
already a rule somewhere in `CLAUDE.md` or the README:

1. **One `hyprforge_look::Theme`, resolved once before the first frame.**
   `hyprforge_ui::theme::init(hyprforge_appearance::look::resolve())`, the
   same line in `main` every app in the suite has. No colour constant
   belongs in this app.
2. **Two config files, three states each.** `files.toml` is state the app
   rewrites whole (window size, view prefs); `files-config.toml` is
   hand-written and only ever read (key bindings, menus). Missing is
   first-run, unparseable is *reported and defaulted*, partly-bad keeps
   every good entry. `media.toml` / `media-config.toml`, same split,
   same three states. This is the 37-binds rule and it is not optional.
3. **The keyboard grammar is config, not code.** Vision pillar 9 says one
   grammar across every app; Files is the only app that has one. See
   "Lifting the keymap" below — this is the one piece of shared
   infrastructure this app should *create*, not just consume.
4. **Every sibling may be absent.** No `appearance.toml` (Settings never
   installed) is first run. `hyprforge-clipd` not running must not stop
   "copy image" — the clipboard *library* is what this uses, never the
   daemon. `hyprforge-files` not installed makes "Show in Files" a logged
   warning, not a crash.
5. **The packaging shape.** Its own `package_hyprforge-media()` in
   `packaging/arch/PKGBUILD`, its own
   `crates/hyprforge-media/packaging/hyprforge-media.desktop`, its own
   `install_media()` in `./hyprforge`, and a line in `--status`.
   Installing only this package on a clean machine must give a working
   viewer.
6. **Tiered tests that gate on what they actually ask.** Almost all of
   this is tier 1: decoding, orientation, fit/zoom arithmetic and the
   filmstrip order need no compositor and no daemon. What is *not* tier 1
   is named honestly below rather than quietly skipped.

## The shape: two crates

```
hyprforge-image     a leaf. Identify, measure, orient, budget, downscale,
                    decode. No iced, no Wayland, no async runtime, no
                    Hyprland. Depends on `image` and nothing of ours.
hyprforge-media    the window. iced, the viewer grammar, the filmstrip,
                    and the actions that touch files.
```

**Why the leaf exists at all**, given that a viewer is the only thing that
needs it today: it is not. `hyprforge-files-core::prefs` already carries a
`preview_pane: bool`, defaulted on, with nothing behind it, and vision
pillar 8 names a shared spacebar-preview component as cross-cutting
between the File Manager and the Photo Viewer. The second consumer is
already half-specified in a shipped struct. Everything in that leaf —
"how big is this without decoding it", "what does EXIF say its rotation
is", "what may I safely decode" — is the same question the preview pane
will ask, and it is all pure enough to test without a GPU.

**Why there is no third crate**, no `hyprforge-media-core` mirroring
`hyprforge-files-core`: that crate exists for one specific reason, which
is that the portal's open/save dialog has to render *the identical view*
and the type system is what enforces it. There is no second host for a
zoomable image canvas. The preview pane is a thumbnail in a pane, not a
reduced copy of this viewer, and pretending otherwise would buy a seam
nothing pulls on. If a Quick Look overlay later wants the real zoom
canvas, that is the moment to extract it — not before.

## Four things checked, not assumed

These are the entries that would otherwise be written into `CLAUDE.md`
after costing a day each. Three were verified against source in the
process of writing this plan.

**iced decodes an image handle at full resolution, and applies EXIF
orientation, but only for two of its three handle kinds.**
`iced_graphics-0.14.0/src/image.rs` does `::image::open(path)` for
`Handle::Path` and `load_from_memory` for `Handle::Bytes`, reads
`exif::Tag::Orientation` from the same bytes and rotates/flips
accordingly — with no dimension cap anywhere. `Handle::Rgba { .. }` is a
straight passthrough of the pixels it is given. So:

- `image(path)` on a 36-megapixel photograph decodes 36 megapixels into
  RGBA8 (144MB) and hands the renderer its own premultiplied copy on top
  — the exact allocation profile that made the lock screen peak at 296MB,
  and `iced_tiny_skia` caches an allocation failure as "no entry" and
  panics on the *next* frame. A viewer that opens whatever a camera
  produced cannot use that path.
- Therefore the viewer builds `Handle::from_rgba` from pixels
  `hyprforge-image` decoded within a budget — **and therefore the viewer
  must apply EXIF orientation itself**, because the passthrough path does
  not. Get exactly one of those two halves right and every portrait phone
  photo is sideways; get both wrong in the same direction and a later
  "fix" double-rotates. The test that pins it is a fixture image per
  orientation value (1, 3, 6, 8) asserting a known corner pixel lands in
  the known corner — cheap, and it fails loudly if iced changes its mind.

**The budget is the whole design of the decode path.** clipmenu refuses
anything over 16 megapixels and falls back to text; a viewer may not
refuse — showing the photograph *is* the app. So the rule is different:
decode to fit the *window*, not the file. Ask the header for dimensions
first (never decode to measure), pick a target from the window's physical
pixel size, and decode-and-downscale into that. Zooming past 1:1 of that
surface re-decodes a *crop* at native resolution, which is bounded by the
window no matter how large the file is. Two consequences worth stating
now: the peak is `window_pixels * 4 * (1 + renderer copy)` plus one
decoder working set, not a function of the file; and "what does this
allocate" is a question the tests ask directly, per `CLAUDE.md`, rather
than only "does it display".

**`image` is configured at the workspace root, and it is PNG and JPEG
only.** `Cargo.toml`'s comment explains why it is declared there:
`iced_graphics` enables `image` with no format features at all, and cargo
unifies features, so the workspace table is what switches decoders on for
iced. A viewer that claims to open `webp`, `gif`, `tiff`, `bmp` and `avif`
— all of which `EntryKind::classify` already calls `Image` — needs those
features added *there*, which turns them on for every app in the suite
that links iced. That is a workspace-wide decision with a binary-size and
build-time cost, not a line in the photos manifest, and it should be made
deliberately once rather than discovered when a `.webp` fails to open.
SVG is not in that list and does not belong in phase 1: it is a
rasterize-at-zoom-level pipeline (`resvg`), not decode-once, and it is a
new dependency tree.

**Logical versus physical pixels, again.** Every zoom and pan coordinate
in this app is one or the other and the code has to say which. Wayland
delivers pointer coordinates in logical surface space by contract; the
buffer this decodes into is physical; `hyprctl` reports logical and `grim`
writes physical, which is why a screenshot cropped at an `hyprctl
clients -j` box on this 1.6-scale machine lands on some other window
entirely. `hyprforge-popup` documents the same split from the other side.
Name the units in the type or the field name, not in a comment.

## What the viewer is, in v1

A window that opens one image, shows it correctly, and moves through the
folder it came from. Not a library, not a catalogue, no tagging, no
editing. "Correctly" is doing the work: right orientation, right colours
(the `web-colors` trap is already handled at the workspace level, and this
app inherits it), fit-to-window by default, bounded memory.

The thing that makes it feel like Files is that it *agrees* with Files:
open a folder in the file manager, double-click the third image, press
Right, and you get the fourth image **in the order Files was showing** —
which means the next/previous order comes from
`hyprforge_files_core::sort` applied to the same prefs, not from
`read_dir` order. That is a dependency worth taking deliberately (photos
-> files-core), and it is the single detail most likely to be noticed if
it is wrong.

## Phases

**Phase 0 — the skeleton, before any pixels.** Two crates in `members`,
manifests with the comment density this repo uses, `media.toml` /
`media-config.toml` in `hyprforge-paths`, the desktop entry, the PKGBUILD
package, `install_media()` and the `--status` line, and the vision table
row moved off "not started". Doing packaging last is how an app ends up
unable to be installed alone; doing it first makes "install only this
package on a clean machine" answerable from day one.

**Phase 1 — open one file and show it right.** `hyprforge-image`: header
measurement, EXIF orientation, the budget, decode-to-fit. The window: fit
to window, theme background, the minimum chrome. PNG and JPEG. Accepts a
path or a `file://` URI on argv, because that is what `gio` hands a
`%U` desktop entry. At the end of this phase the orientation fixtures and
the allocation test exist.

**Phase 2 — the grammar.** Zoom (fit / 1:1 / to-pointer), pan, next and
previous across the folder in Files' order, fullscreen, view-rotation,
and an info panel (dimensions, file size, mtime, and the EXIF a camera
wrote). The filmstrip, built on the same bounded-decode path at thumbnail
size. All of it bound through the config file, none of it hardcoded.

**Phase 3 — the actions that touch files.** Trash via
`hyprforge-fileops`, with the same undo affordance Files has — destructive
defaults to reversible, vision pillar 4. Copy to clipboard via the
`hyprforge-clipboard` *library*. Open-with and "Show in Files", both
tolerant of an absent sibling. Rotation that writes back to the file is a
separate, later decision: it is a destructive edit of someone's original
and needs to say so.

**Phase 4 — pay the shared-component debt.** *Built, except the
spacebar Quick Look overlay: the preview pane and the grid thumbnails
decode through `hyprforge-image`.* Wire
`hyprforge-files-core`'s `preview_pane` and a spacebar Quick Look onto
`hyprforge-image`, which closes vision pillar 8 and retires a `bool` that
currently promises something the file manager does not do.

## Lifting the keymap

*Done: the grammar is `hyprforge-keys`, generic over the action type, and
both `hyprforge-files-core` and this crate depend on it.*

`hyprforge-files-core::keymap` is already the right code — `Combo::parse`,
`Modifiers`, `KeyPress`, the "a binding wins over typing, and Ctrl-held
characters never leak into search" rule — and it depends on nothing but
`std` and `crate::action::Action`. It is hardcoded to that one `Action`
enum, which is the only thing stopping a second app from using it.

The viewer is the second app, so this is the moment: make it generic over
the action type and move it somewhere both can reach. Not into
`hyprforge-ui` (that is the iced layer, and this has no iced in it) —
either its own small leaf or `hyprforge-look`'s neighbour in the
foundation row. Photos then ships its own `Action` enum with its own
`default_keys()`, and the two apps parse `"Ctrl+Shift+N"` identically
because it is one parser, which is what pillar 9 asks for. The
alternative — photos depending on `hyprforge-files-core` for a key parser
and dragging in the whole browser view and `hyprforge-fileops` with it —
is the wrong shape, and copying the file is how two grammars drift.

This is the one refactor of existing code the plan asks for, it is
mechanical, and `hyprforge-files`'s tests cover the behaviour being moved.

## Testing

Tier 1 covers more of this app than of most: orientation, the budget
arithmetic, fit/zoom/pan transforms, the filmstrip order against
`MockBackend`, the config file's three states, and the keymap. Tests named
as properties — `a_portrait_photo_is_not_shown_sideways`,
`no_image_ever_decodes_more_than_the_window_can_show`.

What tier 1 *cannot* answer is whether the pixels are right, and this is
an app whose entire output is pixels. `notif-render`'s golden-image tests
are the precedent: render through `iced_tiny_skia` headlessly and compare.
A centre-pixel comparison against the theme value is also how the
`web-colors` divergence was found in the first place, and is the way to
settle any "these should look the same" question here.

If something genuinely cannot run — no fixture, no decoder feature — it
prints `HYPRFORGE-SKIP: <reason>` and `check.sh` reports it in yellow. A
test that returns early and prints `ok` is a test that lied.

## Open questions

Three decisions that are yours, and that change what gets built. All
three were answered — see the status at the top: PNG, JPEG, WebP and GIF
(then BMP, TIFF and ICO as well), Set as Wallpaper in scope, and the
keymap lifted into `hyprforge-keys`.

1. **Which formats in phase 1.** PNG + JPEG costs nothing (already on).
   Adding webp/gif/tiff/bmp/avif is a workspace-root change affecting
   every iced app in the suite. Recommendation: PNG and JPEG in phase 1,
   then webp and gif as one deliberate workspace commit in phase 2, and
   leave avif/heic alone until someone actually has one.
2. **Is "set as wallpaper" in scope.** It is the most natural action a
   viewer has on this desktop and it is the only one that would make
   photos depend on `hyprforge-ecosystem` and talk to `hyprctl` — a
   Hyprland coupling the file manager deliberately does not have.
   Defensible either way; recommendation is to take it in phase 3 and let
   it degrade quietly where hyprpaper is not running.
3. **Whether the keymap lift happens now or the viewer copies first.**
   Now costs a small refactor before any viewer code exists. Later means
   two grammars that already disagree by the time anyone lifts them.
