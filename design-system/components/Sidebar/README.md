# Sidebar

The app's primary navigation, attached flush to the left of the screen: add `cs-sidebar--flush` so it fills the full height with no margin and no radius and keeps one hairline on its right. Width is `sidebar-w` (240px), ground `surface-sunken`. It holds the text lockup `cs-brand` (`[ name ]`, brackets in `primary-text`) with a toggle button in `cs-sidebar__head`, then groups of `cs-nav` lists, each opened by a `cs-nav__label` (`eyebrow`), and an optional `cs-sidebar__foot` pinned to the bottom.

**Items** `cs-nav__item` is `row-dense` (32px) with a 20px icon and `body-strong` text; hover `surface-hover`; the current page (`aria-current="page"`) is `primary-subtle` with `primary-text`. Put the label in `cs-nav__text` and counts in `cs-count` (pushed right).

**Open and closed** add `cs-sidebar--rail` to collapse it to an icon-only strip of `sidebar-rail-w` (64px): labels, group titles, counts, the lockup and the footer disappear, the toggle stays, and a hairline separates the groups. Give every item a `title` so the name shows on hover, and keep the same toggle button for both states.

**Page title** next to the sidebar, `heading-1` wraps its words in brick brackets as in the example.

**The consumer provides** `<nav>` landmarks with `aria-label`, real links, `aria-current` on the one active item, and the open/closed state with `aria-expanded` on the toggle.

**Don't** use more than two groups, nest items, or mark the current page with color alone: keep the icon and the label.
