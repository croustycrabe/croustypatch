# Choice

Native `input` elements restyled with `appearance: none`: `cs-check` wraps a checkbox or radio and its label, `cs-switch` wraps a `role="switch"` checkbox. Box 16px (switch 36×20), `border-strong` edge, `radius-sm` for checkboxes, `radius-pill` for radios and switches.

**Use** a checkbox for independent options and bulk selection (its mixed state is `:indeterminate`), a radio group when exactly one applies, a switch for a setting that takes effect immediately. A switch's label names the setting, not the action: "Auto-deploy", not "Turn on auto-deploy".

**States** checked fills `primary` with a `primary-fg` mark; the switch also moves its knob 16px, so state never rests on color alone. Disabled uses `surface-sunken` and `fg-disabled`.

**The consumer provides** the `name`, `checked` and `disabled` attributes and the label text. Add a `small` line inside the label for a one-line explanation.

**Don't** use a switch inside a form that needs a Save button, or put two radio groups under one `name`.
