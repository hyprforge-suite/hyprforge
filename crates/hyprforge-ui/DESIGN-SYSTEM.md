# The Hyprforge design system

What every Hyprforge window is built from, where each piece comes from, and
where a new one goes.

It is code, not a style guide. Every entry below names something an app
calls. Two crates hold it:

- **`hyprforge-look`**: the tokens. One `Theme`, no iced, so the lock
  screen, the greeter and a daemon with no window read the same struct.
- **`hyprforge-ui`**: the iced half. Sizes, drawn marks and widgets, built
  only from those tokens.

`hyprforge-files-core` keeps Files' own sizes (grid, menus, sidebar) in its
`density.rs`. Everything that is not about files lives here.

## The rule

**A widget that is not about one app's subject belongs in `hyprforge-ui`,
written once.** Files' path bar is about files and stays in
`hyprforge-files-core`. The panel that hangs under it is not, and lives here
as `suggestions`, so Settings' search can use the same one.

The test before writing a widget in an app: *would another app draw this
the same way?* If yes, it goes here, even with one user, because the second
user arrives with its own copy otherwise. That already happened twice:

- Media's path bar (`hyprforge-media/src/view.rs`) duplicates Files'
  breadcrumbs.
- Settings' search palette (`hyprforge-settings/src/main.rs`) is the same
  list-under-a-field this crate now has.

Both are owed a move onto the shared pieces.

And never a colour constant in an app. A missing colour is a missing
`Theme` role. See "Tokens".

## Tokens: `hyprforge-look`'s `Theme`

Resolved once per process by `hyprforge-appearance`'s `look::resolve`, from
settings the user already controls, and installed with
`hyprforge_ui::theme::init`. "Follows Hyprland" means the value is read from
Hyprland's own option, through the `appearance.toml` the Settings app keeps.
It is not read from the live compositor. A value set by hand in
`hyprland.lua` and never touched in Settings is not seen yet.

| Token | Means | Source |
|---|---|---|
| `accent` | selected, and nothing else | **Hyprland** `general:col:active_border` |
| `rounding` | the window's corner; every radius derives from it | **Hyprland** `decoration:rounding` |
| `font`, `font_size` | body text | desktop: gsettings `font-name` |
| `mono_font` | paths, keys, config lines | desktop: gsettings `monospace-font-name`, if fontconfig has it |
| `font_scale` | multiplies every text size | desktop: gsettings `text-scaling-factor` |
| `surfaces.root` → `sidebar` → `card` → `row` | four elevation steps | `Theme` default |
| `surfaces.card_border` | hairlines, the "you are here" chip | `Theme` default |
| `surfaces.text`, `text_dim` | content, metadata | `Theme` default |
| `success`, `warning`, `error` | state | `Theme` default |
| `info` | "this is somewhere else": remote, mounted | `Theme` default |

Known gaps, written down so nobody assumes otherwise:

- Surfaces and state colours do not follow Hyprland. Hyprland has no
  notion of either. `general:col:inactive_border` is the nearest candidate
  for a border token and is not read.
- Border *widths* are 1px constants in the widgets. `general:border_size`
  is not read.
- None of the Hyprland values are asked of the running compositor. See the
  note above the table.

## Sizes: `density`

Every size is a ratio to `theme::BASE_TEXT_SIZE` at 100% scale, so a user at
125% gets taller rows rather than clipped text. Never write a pixel height
into an app.

| Function | What |
|---|---|
| `ROW_TEXT_BASE`, `META_TEXT_BASE`, `SECTION_LABEL_BASE` | the type ramp: content, metadata, section labels |
| `row_height`, `setting_row_height`, `SETTING_ROW_GAP` | a list row; a Settings row, and the gap between them |
| `bar_height`, `field_height`, `glyph_button` | the header bar, a field in it, a square icon button |
| `outer_radius` → `card_radius` → `inner_radius` → `nested_radius` | the radius ladder, each a fixed fraction of `rounding`: window, card, control, control-inside-a-control |

## Marks: `glyph`

Drawn, not typed, because a font's symbols are a different shape on every
machine:

- `nav` (back/forward/up), `sidebar`, `side_panel`
- `view` (list/grid/columns)
- `page` (each Settings page)
- `signal` (with `signal_bars`, the strength-to-bars rule), `battery`

## Widgets: `widgets`

Each is `hyprforge_ui::widgets::<name>`.

| Group | Widgets |
|---|---|
| Text | `scaled_text` (every text goes through it, so `font_scale` reaches the screen), `meta_text`, `hint_text`, `section_label`, `spaced_caps`, `config_line` |
| Selection | `selectable_row_style`: the one rule, accent means selected and hover never shares it. `Tint` for a row mark's role |
| Fields | `inset_field_style` (the recessed box Files' path bar and every search share), `inset_input_style`, `search_field`, and `token_field`: a search field holding finished tokens between the magnifier and the cursor (Files' `ext:rs` chips) |
| Under a field | `anchored` (an `Anchored`): hangs one element under another, at its width, as an overlay; placed from the anchor's own layout, because a window cannot ask where a widget landed. `suggestions` (of `Suggestion` rows): the list that hangs there, with a heading line, an empty message and the shared row look |
| Controls | `toggle` and `toggle_style`, `value_slider`, `stepped_slider` with `step_index`, `slider_style`, `segmented`, `segmented_choice`, `segment_style` with `SegmentLook`, `dropdown_style`, `dropdown_menu_style`, `tri_state` |
| Buttons | `primary_button`, `secondary_button`, `danger_button` |
| Layout | `section`, `divider`, `vertical_divider`, `row_field`, `page_header`, `setting_list`, `setting_row`, `setting_row_style`, `hero_card`, `status_dot`, `pending_bar`, `pending_label` |
| Panels | `panel_tabs`: words across a docked panel's top, the chosen one over an accent underline. `fact_row` and `fact`: a dim label at a fixed width and its value beside it, so a panel's values line up (Files' Properties inspector) |
| Badges | `chip` (something the system says about a row, in a state colour), `removable_chip` (something the person typed and can take back: neutral, with a ×), `keycap` |
| Dialogs | `confirm_dialog` |
| Time | `countdown_ring`, with `remaining_fraction` |
| Progress | `progress_line` (with `progress_line_style`): a thin bar in the foreground colour on the `row` step, never the accent; drawn only when there is an honest fraction |
| Floating | `popover_card`: the surface a popover opens on, the context menu's own (see Patterns). `scrim`: a card centred over the window dimmed with `scrim_color` (the root step at `SCRIM_ALPHA`), a press on the dim layer dismissing it, and nothing beneath reachable — Files' Quick Look |

## Patterns

Not widgets, but decided once:

- **On is filled.** A control that is on right now has a fill. Off or
  disabled has none and dims its mark. It never disappears, because a
  vanishing button reflows the row under the pointer.
- **Purple means selected.** Nothing else takes the accent. A mode, a
  hover or a focused field uses an elevation step or the dim text colour.
- **A floating list is the context menu's surface.** `sidebar`, a 1px
  `card_border` outline, `inner_radius`. `suggestions` and Files' context
  menu agree.
- **Progress is not selection.** A bar fills with the foreground colour,
  because iced's default bar takes the palette's primary, which is the
  accent. Unknown progress is said in words ("counting…"), never drawn
  as a bar that fills at an invented rate.
- **Overlays are opaque.** A click on a panel's own padding must not reach
  the listing beneath it.
- **Check the pixels.** "Do these look the same" is settled by comparing a
  screenshot's pixel against the token, not by reading the code. This is
  how the `web-colors` drift was found (CLAUDE.md).
