# Loomik brand assets

The project mark is an orange **L** with a recording dot, on a rounded graphite
square. The letter identifies Loomik; the separate dot echoes recording and the
floating camera overlay.

| Asset | Dimensions | Purpose |
| --- | --- | --- |
| `loomik-logo.svg` | 512 × 512, scalable | Editable project logo |
| `loomik-logo.png` | 512 × 512 | Raster logo for documentation and profiles |
| `loomik-readme.svg` | 1600 × 900, scalable | Editable product illustration |
| `loomik-readme.png` | 1600 × 900 | Centered graphic in the project README |

Palette: orange `#EF4B2B`, graphite `#26272B`, blue `#1966E2` for the selected
microphone, and pale gray `#F5F6F8`. Keep the logo's proportions and internal
spacing when resizing. SVG artwork uses Arial with Helvetica/sans-serif fallbacks;
PNG exports preserve the checked layout without requiring fonts on the reader's
machine.

The illustration shows a demo desktop, floating settings/controls, and an anonymous
illustrated camera portrait. It is not a real screen or webcam capture and contains
no user documents, device identifiers, or account details. The project artwork
does not replace the installed app's macOS bundle identity or privacy grants.

These are code-generated vector assets, rendered to PNG with native macOS tools;
no image-generation model or external asset service was used. The source is
[`scripts/make-brand-assets.py`](../../scripts/make-brand-assets.py).

Regenerate the SVGs with Python's standard library:

```sh
python3 scripts/make-brand-assets.py
```

On macOS, regenerate both SVGs and PNGs using Quick Look and `sips`:

```sh
python3 scripts/make-brand-assets.py --png
```

Rendering intermediates go into the gitignored `target/brand-previews/` directory.
After changing artwork, inspect the PNGs at full size and at README display width
before committing them. The assets are covered by the repository's MIT license.
