# Unica Visual Kit

- `index.html` — primary long-form Brand Canvas
- `logos/` — background-specific SVGs, transparent SVGs, alpha PNG logo sizes, and PNG mark sizes
- `wallpapers/`, `ads/`, `social/` — ready placements and full-bleed social avatars
- `unica-visual-kit.pdf` — printable export of the same canvas
- `unica-visual-assets.zip` — SVG and ready-placement download used by the page
- `unica-visual-kit-desktop.png` and `unica-visual-kit-mobile.png` — previews

Open `index.html` directly. No network connection is required.
The production plugin icon is not changed by this package.

## Palette

| Token | Value | Use |
| --- | --- | --- |
| Blue | `#2563EB` | Mark fill, primary buttons, links on light surfaces |
| Blue on ink | `#7FA8E0` | Links and accents on dark surfaces: the site in dark theme, dark IDE panels. Fills keep plain Blue |
| Signal | `#93C5FD` | The mark's dot, small accents, highlights on Blue |
| Ink | `#0F172A` | Text on light surfaces, dark backgrounds |
| Paper | `#F8FAFC` | Page background |
| Neutral 200 | `#D8DEE8` | Rules and borders |
| Neutral 500 | `#64748B` | Secondary text |

`Blue on ink` exists because plain Blue on Ink reads as a fill, not as a link: white text on it is legible, blue text next to it is not. Keep fills on Blue and move text accents to Blue on ink.
