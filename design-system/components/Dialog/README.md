# Dialog

A modal `cs-dialog` on a `cs-overlay` scrim (`overlay`). 440px (`dialog-w`), `surface` ground, `border-strong` edge, `radius-lg`, `shadow-dialog`. Title in `heading-3`, body in `body`, footer on `surface-sunken` with the buttons right-aligned, the confirming one last.

**Use** for decisions and short forms that block the page. For a destructive action make the confirm button `cs-btn--danger` and repeat the verb and object ("Delete project"). Otherwise the confirm is `cs-btn--primary`.

**The consumer provides** `role="dialog"` (or `alertdialog` for destructive), `aria-labelledby`, focus trap, Escape to close, focus return to the trigger, and a close button with `aria-label`.

**Don't** open a dialog from a dialog, put a table inside one, or use "OK" as a button label.
