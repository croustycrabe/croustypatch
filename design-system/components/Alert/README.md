# Alert

An inline message that stays on the page until fixed or dismissed. A grid of icon, text and optional close button, `space-3` column gap, `space-3` × `space-4` padding, `radius-md`, a 1px edge in the tone's `x-text` on the `x-subtle` ground. Pick the tone with `cs-tone-success | warning | danger | info`.

**Use** for results of an operation the user must act on or know about, near what it concerns. Use a Toast for confirmations that need no action.

**The consumer provides** a title that states the outcome ("Build failed"), one line of text saying what to do, the tone's icon at 20px (`check-circle`, `warning`, `x-circle`, `info`) and, optionally, up to two small buttons. Ghost buttons inside an alert hover with a tint of the alert's tone (16%), not the neutral gray; a warning alert's main action can use `cs-btn--info` for contrast with the amber. Give danger alerts `role="alert"`, the others `role="status"`.

**Don't** stack more than three alerts, write "Error" as the whole title, or use color without the icon and the words.
