# Card

A `surface` panel with a 1px `border` and `radius-lg`; no shadow. Optional parts: `cs-card__head` (title in `heading-3`, one badge or button on the right), `cs-card__body` (`space-6` padding), `cs-card__foot` (`surface-sunken`, right-aligned small buttons).

**Variants** `cs-card--link` is clickable (hover `surface-hover` and `border-strong`; give it `tabindex`, `role` and a visible focus). `aria-selected="true"` draws a 2px `primary` edge. A metric card shows its label in `fg-muted` and its number in `cs-stat` (`data` style, 28px).

**The consumer provides** the title and body; for a clickable card, a single link or button that covers it.

**Don't** nest cards, add a shadow to make one "pop", or color the whole card with a state tone: show state with a Badge.
