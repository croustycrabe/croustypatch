CroustyLab Software is the product-UI variant of CroustyLab: the same cream, ink and one brick red, flattened for tools people stay in all day. Surfaces are separated by tone and 1px hairlines, nothing at rest casts a shadow, and four state colors (success, warning, danger, info) carry system feedback. It ships a light and a dark theme. Use CroustyLab itself for the portfolio and marketing surfaces; use this system for dashboards, consoles, forms and any screen with data.

## Content fundamentals

- **Write plainly, in the product's language.** "Deploy to production", "3 checks failed", "Rollback completed in 42 s". No hype, no exclamation marks, no emoji.
- **Sentence case everywhere.** Buttons, tabs, menu items and column headers read "Save changes", "Environment", "Last deploy". UPPERCASE is for `eyebrow` group labels only.
- **Start action labels with a verb and name the object.** "Delete project", not "OK" or "Yes". The confirm button in a dialog repeats the dialog's verb.
- **Errors say what happened, then what to do.** "Build failed: missing secret DATABASE_URL. Add it in Settings, then retry." Never "Something went wrong".
- **Bracket the page title.** `heading-1` is wrapped as `[ Deployments ]` with the brackets in `primary-text`. Brackets appear on page titles only, never on card, dialog or section titles.
- **Separate facts with a middle dot:** "api-gateway · v2.14.0 · 4 min ago".
- **Numbers, IDs, versions, hashes and durations are data.** Set them in `data` (or `code-sm` for hashes), right-align numeric columns, keep units in the header.
- **Localise every string.** French and English both ship; never hard-code one language.

## Visual foundations

### Flat, by rule

- Resting elements have no shadow. Separate a panel from the canvas with `surface` on `bg` and a 1px `border`, not with elevation.
- Only layers that float over the page carry a shadow: menus, popovers, tooltips and toasts use `shadow-float`; dialogs use `shadow-dialog`. The shadow stays hard and unblurred, the one trait kept from CroustyLab's press shadow.
- There is no press translate. Pressed is a tone step: `surface-hover` on hover, `surface-active` on press, `primary-hover` then `primary-active` on brick.
- No gradients, no textures, no illustrations inside the product frame.

### Color

- Ground is `bg`. Raised panels (cards, inputs, tables, menus, dialogs) are `surface`. Recessed areas (sidebar, table header, code, disabled fields) are `surface-sunken`.
- All text is `fg`; secondary text is `fg-muted`. Use `fg-muted` only on `bg`, `surface`, `surface-sunken` and `surface-hover`, never on `surface-active`.
- `primary` (brick) is the only brand accent: the one primary button per view, selected checkbox and switch, the active tab underline, the progress fill, the focus ring. Brick as text is `primary-text`.
- Edges of anything a user operates (inputs, secondary buttons, checkboxes, menus, dialogs) use `border-strong`. `border` is for dividers and card edges only.
- Each state owns four tokens: solid `x`, text on solid `x-fg`, tinted ground `x-subtle`, and text/icon/border `x-text`. Alerts, toasts and soft badges sit on `x-subtle` with `x-text`; filled badges and destructive buttons use `x` with `x-fg`.
- Mark things with `x-text`, never with `x`. `warning` is amber and too light to read as an icon or border on cream; `warning-text` is the dark amber for that job.
- **State is never color alone.** Every state shows an icon and a word: success `check-circle`, warning `warning`, danger `x-circle`, info `info`. In a dense table a `cs-dot` plus the word may replace the icon. Success is teal and danger is crimson so the pair never relies on red versus green.
- `danger` is crimson, cooler than the brick `primary`. Use `danger` only for errors and destructive actions; never for decoration or emphasis. A destructive primary action in a dialog is `danger`, not `primary`.
- `info` is for neutral system messages only. It is never a link color: links are `primary-text`. Its one action use is `cs-btn--info`, the dark-blue call to action beside a warning alert, where it stands apart from the amber tint.
- Buttons inside an alert hover with a tint of the alert's own tone (`--tone` at 16%), never the neutral `surface-hover`.
- In dark, accents lighten and their filled text turns ink (`primary-fg`, `success-fg`… flip to `#1b2125`). Never hard-code white or black on a fill; use the `-fg` token.

### Type

- Three families. `display` (Rubik One, a single 400 weight) for `heading-1` to `heading-3` only; `sans` (Rubik) for all interface text; `mono` (Roboto Mono) for code, data, IDs and badges. Never bold the display face.
- UI base is `body` at 14px. Controls use `body-strong` or `label`. Hints and errors under fields are `caption` in `fg-muted`.
- Set badge and status text in `tag`. Set tables' numbers in `data` with tabular figures.
- Intentional addition to CroustyLab: Rubik, a sibling of Rubik One, carries running UI text so dense screens do not read as monospace walls. The mono stays the voice of data.

### Borders, radii

- Every edge is `border-width` (1px) solid. `border-width-strong` (2px) appears only on a selected card.
- `radius-md` on controls and alerts, `radius-lg` on cards, tables and dialogs (the sidebar is flush: no radius, a single right-hand hairline), `radius-sm` on checkboxes, badges and menu items, `radius-pill` on switches, progress bars and dots. Buttons are never pills.

### Spacing and density

- 4px base. Controls are `control-md` (36px) by default, `control-sm` in toolbars and table rows, `control-lg` for touch.
- Table rows are `row-md` (40px), `row-dense` (32px) when asked for. Cell padding x is `space-3`.
- Card and alert padding `space-4` to `space-6`; gap between buttons, fields and chips `space-2` or `space-3`; gap between page sections `space-8`. Content is capped at `container` (72rem).

### Motion

- Only color, border and `width` (progress) change, 120ms ease-out; nothing slides or scales. The spinner is the only looping animation.
- Respect `prefers-reduced-motion: reduce`: transitions drop to near zero and the spinner stops.

### States

- **Focus:** `focus-width` (2px) solid `focus` (brick), offset 2px, on every focusable element. Inputs draw it flush against their edge.
- **Hover:** `surface-hover` on neutral surfaces, `primary-hover` on brick. **Pressed:** `surface-active` or `primary-active`.
- **Selected:** `primary-subtle` ground with `primary-text` (nav item, table row), `primary` fill (checkbox, switch), or a 2px `primary` edge (card).
- **Invalid:** `danger-text` border, `danger-subtle` fill, and a `caption` error with the `x-circle` icon.
- **Disabled:** `surface-sunken` ground, `fg-disabled` content, `border` edge, `not-allowed` cursor. Never the only way to say why: add a hint.

## Iconography

- **Phosphor Icons**, always `weight="bold"`, 16px (`icon-sm`) inside buttons, badges and menus, 20px (`icon-md`) in alerts, toasts and navigation. Icons inherit `currentColor`; mark a state by coloring the icon `x-text`.
- State icons: `check-circle`, `warning`, `x-circle`, `info`. Common UI: `caret-down`, `magnifying-glass`, `plus`, `x`, `dots-three`, `trash`, `copy`, `gear`, `house`, `folder`, `git-branch`, `rocket-launch`.
- No emoji, no icon fonts, no other icon sets.

## Code

- Code blocks sit on `surface-sunken` with a 1px `border`, text in `code`. Highlight with four roles only: keywords `primary-text`, strings `success-text`, numbers `info-text`, comments `fg-muted` italic (classes `tk-kw`, `tk-str`, `tk-num`, `tk-com`).

## Logo

- This system ships no logo files. Use the crab mark from CroustyLab (`crab-logo.svg` on light grounds, `crab-logo-mono-cream.svg` on brick or ink) and `app-icon.svg` as the favicon. In the product frame the lockup is the text mark `[ name ]` in `display` 14px with the brackets in `primary-text`.
