# Toast

A short-lived message floating at the bottom-right of the viewport: 360px wide, `surface` ground, `border-strong` edge, `radius-md`, and `shadow-float`. The tone only colors the 20px icon (`cs-tone-success | warning | danger | info`); text stays `fg`.

**Use** for confirmations that need no decision. Auto-dismiss success and info after 5 s; keep warning and danger until closed. Stack newest on top, at most three.

**The consumer provides** a title and one line of text, the state icon and a close button with `aria-label`. Mount the stack in a `role="status"` live region.

**Don't** put the only copy of important information in a toast, or auto-dismiss an error.
