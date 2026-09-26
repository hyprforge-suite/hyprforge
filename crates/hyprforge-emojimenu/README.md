# hyprforge-emojimenu

An emoji picker for Hyprland — the one GNOME and KDE ship and Hyprland
users otherwise go without.

Bound to a key, it opens a layer-shell popup at the pointer with the full
set of fully-qualified Unicode emoji in sections, the ones you pick most
first, and type-to-filter search. Hold an emoji (or press Shift+Enter)
for its skin tones; the ✋ button beside the search sets the default tone
every emoji is shown in. Two more tabs hold kaomoji and everyday symbols
— arrows, maths, currency, dashes and quotes — and Tab steps between
them. Picking one puts it on the clipboard and pastes it into whatever
had focus.

The kaomoji that use Japanese characters (the shrug's ツ among them) need
a CJK font such as `noto-fonts-cjk`; without one those few draw with
gaps and everything else is unaffected.

Part of [Hyprforge](https://github.com/adamrpostjr/hyprforge), a suite of
native Hyprland desktop applications. This repository is a split of the
`crates/hyprforge-emojimenu` directory there; development happens in the
monorepo and `sync.sh` keeps this copy in step.

## The one hard part

Pasting. `connection.flush()` puts the synthesised key events on the
socket and does *not* wait for the compositor to read them, so a popup
that pastes and exits destroys its virtual keyboard while those events
are still unread — measured at zero bytes delivered. The fix is a round
trip, not a longer sleep: a compositor cannot answer a sync until it has
processed everything queued before it. See `hyprforge-clipboard`'s
`wayland::keyboard::send_combo`.

## Installing

```
cargo install --path .
```

Then bind it. Arch users can build the `hyprforge-emojimenu` package from
the monorepo's `packaging/arch` instead.

It needs no daemon and no settings app: it is a short-lived process that
starts on the keybind and exits when you pick something or press Escape.

## Licence

MIT. See `LICENSE`.
