# Table

A bordered `cs-table-wrap` (`radius-lg`, 1px `border`, `surface`) around a `cs-table`. The header row is `surface-sunken` with `fg-muted` `caption`-size labels; body rows are `row-md` (40px) tall, or `row-dense` (32px) with `cs-table--dense`, separated by 1px `border`. Hover is `surface-hover`; a selected row (`aria-selected="true"`) is `primary-subtle`.

**Cells** text in `body`; names in `body-strong`; versions and hashes with `cs-mono`; numbers with `cs-num` (right-aligned, `data`, tabular figures) and the unit in the header. Status is a Badge with its icon.

**The consumer provides** the rows, a checkbox column with `aria-label` per row, sortable headers as `cs-th-sort` buttons with `aria-sort` on the active one, and the footer (`cs-table-foot`) for count and paging.

**Don't** zebra-stripe, put actions as bare icons without `aria-label`, or let a number column be left-aligned.
