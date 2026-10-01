# Custom window open and close shaders

This is an advanced feature. Ordinary open and close styles stay in
`docs/animations.md`. A custom shader is optional and off unless you set a
fragment-shader path.

`type` still chooses the geometry timeline. The shader, when it compiles,
replaces scale and fade. `launch` and `retract` still travel; in-place types
(`center-out`, `fade`, `shrink`) keep the real window rectangle. Node collapse
never uses a shader.

```rune
animations:
  window-open:
    type "launch"
    duration-ms 220
    curve "ease-out-cubic"
    custom-shader "shaders/open.frag"
  end
  window-close:
    type "retract"
    duration-ms 270
    custom-shader "shaders/close.frag"
  end
end
```

Relative paths resolve from the directory that contains `halley.rune`. `~/`
expands through the current home directory. Halley recompiles when the path or
file mtime changes. A read or compile failure is logged once and the
configured `type` draws instead.

The file is not a full program. Halley wraps it with Smithay's texture-shader
header (`//_DEFINES_`, `v_coords`, `tex`, `alpha`) and an epilogue `main`.
Your source must define one function:

- open: `vec4 open_color(vec3 coords_geo, vec3 size_geo)`
- close: `vec4 close_color(vec3 coords_geo, vec3 size_geo)`

`coords_geo.xy` is 0 to 1 inside the current window geometry and may be
outside that range because the shader runs on a padded quad. `size_geo.xy` is
that geometry in compositor pixels. Return premultiplied alpha.

Uniforms you may use:

- `tex` — the window snapshot
- `halley_progress` — motion value. Open may overshoot with springs or
  elastic. Close is linear wall-clock time; the CPU ease-in-out used by
  shrink/fade is not applied to the shader.
- `halley_clamped_progress` — that value clamped to 0..1
- `halley_random_seed` — stable in `[0, 1)` for the life of the animation
- `halley_tex_scale` and `halley_tex_offset` — map geometry to `tex`
- `halley_geo_size` — same as `size_geo.xy`
- `alpha` — client opacity (window-rule and cluster fade). Do not apply it
  yourself; the epilogue multiplies it.

Sample the snapshot like this:

```glsl
vec2 coords_tex = coords_geo.xy * halley_tex_scale + halley_tex_offset;
vec4 color = texture2D(tex, coords_tex);
```

Custom window shaders own the complete animated silhouette. Halley does not
draw its ordinary geometry-based window shadow during a custom open or close
animation, because that rectangular shadow cannot follow pixels which the
shader moves, fragments, or dissolves.

The shader interface is not a compatibility guarantee. Opening windows that
are also fullscreen, maximized, or arranging keep the live tree. Closing
windows that collapse into a node keep the CPU collapse.

In-flight open and close animations keep the shader path chosen when they
started. A newly compiled program is used on the next frame.
