# hyprforge-thumbnails

The freedesktop thumbnail cache: find a file's thumbnail if it is still current, and store a new one as small as it can be.

The freedesktop thumbnail cache: `~/.cache/thumbnails`.

One cache for the whole desktop, by specification. A thumbnail is a
PNG named by the MD5 of its source's URI, carrying that URI and the
source's modification time in two tEXt chunks, `Thumb::URI` and
`Thumb::MTime`. A lookup is current only while both still match — so
a file is thumbnailed once, and again only after it changes.

# Why the shared cache rather than one of our own

It is the smallest cache there can be. A picture some other file
manager or image viewer has already thumbnailed costs this one
nothing, and one this one makes costs them nothing — this machine had
85 of them in `large/` before Files wrote a single one. A private
cache would store every one of those a second time.

# As small as it can be

- The `normal` size, 128 pixels, which is what the file manager's
  listing draws — and `large`, 256, for the photo viewer's grid, whose
  tiles are twice that wide on a scaled screen. Nothing bigger: the
  specification's `x-large` and `xx-large` are for sizes no window here
  draws.
- Encoded with the PNG encoder's strongest compression and adaptive
  filtering, and without an alpha channel when every pixel is opaque —
  which a photograph, a video frame and a PDF page all are, and which
  takes a quarter off before compression starts.
- A file that cannot be thumbnailed is recorded under `fail/` as the
  specification describes — a 1×1 image carrying the same two chunks —
  so a broken video is not handed to ffmpeg on every visit.
- `Cache::prune` removes thumbnails whose file is gone.

What *not* to cache is the caller's decision, because it knows what a
thumbnail costs to make: a picture already small enough to decode
directly gains nothing from a second copy of itself.

# The URI has to be GLib's, byte for byte

The file name is a hash of the URI, so a URI escaped differently
finds nothing — and worse, writes a second thumbnail beside the one
another program made. `file_uri` escapes exactly the characters
`g_filename_to_uri` does, which was measured with `gio info` rather
than read from a specification: `;` is escaped, `!$&'()*+,=:@` are not.

## Where this lives

Part of [Hyprforge](https://github.com/hyprforge-suite/hyprforge), a suite of native Hyprland desktop
applications. This crate is `crates/hyprforge-thumbnails` there; it is published to
crates.io so the suite's applications can be built from their own
repositories, and its API follows the suite. Issues and pull requests go
to the suite repository.

```
cargo add hyprforge-thumbnails
```

This README is rendered from the crate's `//!` documentation by
`tools/crate-readme.py`; edit `src/lib.rs`, not this file.

## Licence

MIT. See `LICENSE`.
