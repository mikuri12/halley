# Fonts

Halley uses one global font for compositor-owned text:

```rune
font:
  family "monospace"
  size 11
end
```

`family` accepts Cosmic Text generic families (`monospace`, `sans-serif`,
`serif`, `cursive`, and `fantasy`) or an installed family name. Weight and
style suffixes are supported, including `Bold`, `Semi Bold`, `Extra Bold`,
`Light`, `Italic`, and `Oblique`. Cosmic Text supplies fallback fonts when the
selected family does not contain a requested glyph.

`size` is the compositor UI size and is clamped from 6 through 96. Every
compositor-owned label, badge, dialog, notification, and overlay uses this
exact configured size. Individual overlays do not apply hidden legacy size
offsets; hierarchy comes from colour, chrome, and placement instead. Node
markers reserve their center for real application icons and remain blank while
an icon is loading or unavailable. Text is measured and rasterized at its
final physical size, then composed without stretching.

The section live-reloads with the rest of `halley.rune`. A successful font
change clears the text cache and redraws immediately; it does not require a
restart. Old Halley did not expose font mutation through `halleyctl`, so the
configuration file remains the control surface.
