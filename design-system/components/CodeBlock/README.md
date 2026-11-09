# CodeBlock

A `cs-code` panel on `surface-sunken` with a 1px `border` and `radius-md`, text in `code` (Roboto Mono 13/20). An optional `cs-code__bar` carries the file name in `fg-muted` and a small ghost Copy button.

**Highlighting** four roles only: keywords `tk-kw` (`primary-text`), strings `tk-str` (`success-text`), numbers `tk-num` (`info-text`), comments `tk-com` (`fg-muted`, italic). Function names get `tk-fn` (weight 500). Every role holds 4.5:1 on `surface-sunken` in both themes.

**Inline** `cs-inline-code` for commands and identifiers in sentences; `cs-kbd` for keys, one key per element.

**The consumer provides** the escaped source inside `pre > code`, and the Copy handler. Wrap long lines only when asked; otherwise scroll.

**Don't** add line-by-line colors, new highlight roles or a different background for errors: show an error as an Alert above the block.
