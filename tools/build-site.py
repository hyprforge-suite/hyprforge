#!/usr/bin/env python3
"""Build the website in site/ into a directory GitHub Pages serves.

    tools/build-site.py [OUT]     default OUT: _site

Each page in site/pages/ is a body, wrapped in site/_layout.html. A page
opens with a comment naming its title, description and which nav entry
it belongs to:

    <!-- title: Files | description: A file manager for Hyprland. | nav: components -->

Screenshots are written as

    <shot name="files-grid" alt="..." caption="..." need="what to capture" ratio="16/10">

and become the real image when docs/images/<name>.png exists, or a
placeholder naming the file to capture until it does. That is the whole
reason this is a build step rather than plain HTML: adding a screenshot
to docs/images is all it takes to put it on the site, and the README and
the component READMEs read the same directory, so there is one set of
renders and nothing to copy by hand.
"""

import html
import re
import shutil
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
IMAGES = ROOT / "docs" / "images"

HEAD = re.compile(r"\A\s*<!--(.*?)-->\s*", re.S)
SHOT = re.compile(r"<shot\s+([^>]*?)/?>")
ATTR = re.compile(r'(\w+)="([^"]*)"')

def png_size(path: Path) -> tuple[int, int]:
    with path.open("rb") as f:
        head = f.read(24)
    if head[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path} is not a PNG")
    return struct.unpack(">II", head[16:24])


def render_shot(attrs: dict[str, str], number: int | None) -> str:
    """A screenshot as a figure plate, or a placeholder plate naming the file.

    `caption` makes it a numbered figure ("Fig. 3"); without one it is a
    bare picture, for places like a catalogue row where a caption would be
    noise. `need` says what the shot should show, for whoever captures it.
    """
    name = attrs["name"]
    alt = html.escape(attrs.get("alt", ""), quote=True)
    extra = attrs.get("class", "")
    caption = attrs.get("caption")
    cap = ""
    if caption:
        cap = (f'<figcaption><span class="fig">Fig. {number}</span>'
               f'<span>{caption}</span></figcaption>')
    image = IMAGES / f"{name}.png"
    if image.exists():
        w, h = png_size(image)
        lazy = "" if attrs.get("eager") else ' loading="lazy" decoding="async"'
        return (f'<figure class="plate {extra}"><div class="frame"><img src="images/{name}.png" '
                f'width="{w}" height="{h}" alt="{alt}"{lazy}></div>{cap}</figure>')
    ratio = attrs.get("ratio", "16/10").replace("/", " / ")
    need = html.escape(attrs.get("need", attrs.get("alt", "")))
    return (
        f'<figure class="plate todo {extra}" style="--ratio:{ratio}">'
        f'<div class="frame" role="img" aria-label="Screenshot to come: {alt}"><div class="todo-in">'
        f'<div class="todo-head">PLATE TO COME</div>'
        f'<div class="todo-need">{need}</div>'
        f'<div class="todo-file">docs/images/{name}.png</div></div></div>{cap}</figure>'
    )


def build(out: Path) -> None:
    layout = (SITE / "_layout.html").read_text()
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    shutil.copytree(SITE / "assets", out / "assets")
    shutil.copytree(IMAGES, out / "images")

    missing = []
    for page in sorted((SITE / "pages").glob("*.html")):
        text = page.read_text()
        m = HEAD.match(text)
        if not m:
            raise SystemExit(f"{page.name}: no <!-- title: ... --> header")
        meta = dict(
            (k.strip(), v.strip())
            for k, v in (part.split(":", 1) for part in m.group(1).split("|"))
        )
        body = text[m.end():]

        # A page that draws a figure of its own before any <shot> (the
        # home page's live lock screen is Fig. 1) says so with `figs: N`.
        figures = int(meta.get("figs", 0))

        def shot(match: re.Match) -> str:
            nonlocal figures
            attrs = dict(ATTR.findall(match.group(1)))
            if not (IMAGES / f"{attrs['name']}.png").exists():
                missing.append(f"{page.name}: {attrs['name']}")
            if "caption" in attrs:
                figures += 1
            return render_shot(attrs, figures)

        body = SHOT.sub(shot, body)
        nav = meta.get("nav", "")
        title = meta["title"]
        full_title = "Hyprforge" if page.stem == "index" else f"{title} · Hyprforge"
        result = (layout
                  .replace("{{title}}", html.escape(full_title))
                  .replace("{{description}}", html.escape(meta.get("description", ""), quote=True))
                  .replace("{{content}}", body))
        # Mark the current section in the nav.
        if nav:
            result = result.replace(f'data-nav="{nav}"', f'data-nav="{nav}" aria-current="page"')
        (out / page.name).write_text(result)

    print(f"built {len(list(out.glob('*.html')))} pages into {out}")
    if missing:
        print(f"{len(missing)} screenshot placeholders:")
        for m in missing:
            print(f"  {m}")


if __name__ == "__main__":
    build(Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "_site")
