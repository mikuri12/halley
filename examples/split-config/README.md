# Split Config Example

This directory shows how to split a Halley configuration with `gather`.
`halley.rune` is the entry point. Gathered paths are resolved relative to the
file that contains them, so keep these files together when copying the example.

The example is intentionally smaller than `../halley.rune`: omitted settings
keep Halley's built-in defaults. It separates visual styling, Field behavior,
and input policy while leaving inline sections such as `keybinds`, `autostart`,
and `rules` in the root file.

An unaliased `gather` deep-merges its sections with gathered values taking
precedence on duplicate keys. Keep each setting in one file when possible;
later gathered files take precedence over earlier gathered files on collisions.

Halley watches the root and every nested `gather` dependency. After an accepted
reload it also refreshes the dependency set, so adding or replacing a gathered
file does not require restarting the compositor.

Verify the complete gathered configuration with:

```sh
halleyctl config verify --config examples/split-config/halley.rune
```

Startup never rewrites any existing single-file or split configuration. Inspect
an optional structural migration with `halleyctl config migrate --dry-run
--config PATH`, then run it again without `--dry-run` after reviewing the
result. A gathered root cannot identify which source file owns a section, so
run the command directly against the file that owns the affected section.
