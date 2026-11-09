# Field

A label, a control and an optional hint, stacked with `space-1-5`. Controls are `cs-input`, `cs-select` (wrap it in `cs-affix cs-affix--end` with a `caret-down` icon) and `cs-textarea`; all are `control-md` tall (`cs-input--sm` for `control-sm`), `radius-md`, edged in `border-strong`.

**States** hover darkens the edge to `fg-muted`; focus draws a `focus-width` solid `focus` outline flush with the edge; invalid (`aria-invalid="true"`) switches to `danger-text` edge and `danger-subtle` fill; disabled is `surface-sunken` with `fg-disabled`.

**Use** `cs-input--mono` for hashes, IDs, URLs and anything the user will copy. Put an icon inside with `cs-affix--start`.

**The consumer provides** a visible `label` linked by `for`/`id`, the value, and for errors a `cs-hint cs-hint--error` that starts with the `x-circle` icon and says what to fix; link it with `aria-describedby`. Placeholders are examples, never labels.

**Don't** hide the label, show an error with color alone, or validate on every keystroke before the user has left the field.
