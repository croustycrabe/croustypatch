# Tabs

A row of `cs-tab` buttons over a 1px `border` baseline. The selected tab (`aria-selected="true"`) turns `fg` with a 2px `primary` underline; others are `fg-muted`. Height `control-md`, text `body-strong`.

**Use** to switch between views of the same object. Four to six tabs at most; beyond that use a menu.

**The consumer provides** `role="tablist"` on the container, `role="tab"`, `aria-selected` and `aria-controls` on each tab, and the panels. Put a `cs-count` after the label for totals.

**Don't** use tabs for steps in a flow or for navigation between pages: that is the Sidebar.
