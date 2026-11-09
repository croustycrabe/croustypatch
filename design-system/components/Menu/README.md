# Menu

A floating list of actions: `cs-menu` on `surface` with `border-strong`, `radius-md`, 4px padding and `shadow-float`. Items are `row-dense` (32px), `label` text with a 16px icon; hover is `surface-hover`; a checked item (`aria-checked="true"`) is `primary-text`; a destructive item is `danger-text`, set last after a `cs-menu__sep`. Group names use `cs-menu__label` (`eyebrow`). Shortcuts go in `cs-menu__keys` in `code-sm`.

**Tooltip** `cs-tooltip` is `inverse-bg` with `inverse-fg` `caption` text and `shadow-float`. One short phrase, no punctuation; it names an icon-only control, never carries essential information.

**The consumer provides** the trigger with `aria-haspopup` and `aria-expanded`, `role="menu"`/`menuitem`, arrow-key navigation, Escape to close and focus return.

**Don't** nest menus more than one level, put form controls in a menu, or use a menu for fewer than three actions.
