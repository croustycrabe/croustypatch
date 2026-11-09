# Button

A flat, 1px-edged control for actions. Height `control-md` (36px), `radius-md`, text in `body-strong`, a 16px Phosphor bold icon before the label. Classes live in `components/bundle.css`.

**Use** `cs-btn--primary` for the one main action of a view (it can appear again only in a separate dialog). `cs-btn` (secondary, `border-strong` edge) for the rest, `cs-btn--ghost` for toolbar and tertiary actions, `cs-btn--danger` for destructive confirmations, `cs-btn--info` (dark blue, `info` fill) for the call to action beside a warning alert.

**Sizes** `cs-btn--sm` (`control-sm`, `label`), default, `cs-btn--lg` (`control-lg`, `body-lg`). Add `cs-btn--icon` for a square icon-only button and give it an `aria-label`.

**States** hover `surface-hover` / `primary-hover`; pressed `surface-active` / `primary-active`; focus `focus-width` solid `focus`, offset 2px; disabled `surface-sunken` with `fg-disabled` text. Loading: set `aria-busy="true"` and put a `cs-spinner` before the label; keep the label's verb in the present participle.

**The consumer provides** the label, an optional leading icon, `type`, and `disabled` or `aria-disabled`. Explain a disabled button with a nearby hint.

**Don't** use `cs-btn--info` as a general secondary action, use two primaries side by side, round a button into a pill, add a shadow, or use `danger` for anything that is not destructive.
