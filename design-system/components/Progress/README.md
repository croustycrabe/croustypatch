# Progress

A 8px `cs-progress` track (`surface-active`, `radius-pill`) with a fill whose width is set by `--value`. The fill takes the tone's `x-text` color (`cs-tone-primary | success | warning | danger`), so it holds 3:1 against the track. The default tone is `primary`.

**Use** for a measurable task or quota. Put the label on the left and the value in `data` on the right (`cs-progress-row`). For unknown duration use `cs-spinner` (16, 20 or 32px) with an accessible name.

**The consumer provides** `role="progressbar"` with `aria-valuenow`, `aria-valuemin`, `aria-valuemax` and `aria-label`; update `--value` as work advances. Switch the tone to `warning` or `danger` by threshold, and keep the number visible.

**Don't** animate the fill in a loop, show a bar without its number, or change tone without changing the label.
