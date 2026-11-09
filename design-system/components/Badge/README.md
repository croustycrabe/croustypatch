# Badge

A 20px label in `tag` (Roboto Mono 700, 12px) on `radius-sm`, for status, kind and environment. Pick the tone with a class: `cs-tone-neutral | primary | success | warning | danger | info`.

**Variants** soft (default, `x-subtle` ground and `x-text`), `cs-badge--solid` (`x` ground and `x-fg`; for the one status that must jump out), `cs-badge--outline` (`x-text` 1px edge, transparent ground). `cs-count` is a neutral pill for numbers in nav items and tabs.

**The consumer provides** the label and, for state tones, the state icon at 12px: success `check-circle`, warning `warning`, danger `x-circle`, info `info`. In a dense table a `cs-dot` plus the word may replace the icon.

**Don't** show state by color alone, truncate a badge, or use `primary` for a state: brick means brand and selection.
