## Design system: CroustyLab Software

Build every UI with the CroustyLab Software design system in `design-system/croustylab-software/`.

- Read `design-system/croustylab-software/README.md` first: content rules, visual foundations, states, iconography.
- Load `tokens.css` and `components/bundle.css` in the app. Use CSS variables from `tokens.css` (`var(--primary)`, `var(--surface)`, `var(--space-4)`…); never hard-code colors, radii or fonts.
- Components are CSS classes with the `cs-` prefix (`cs-btn`, `cs-input`, `cs-alert`, `cs-table`…). Each has a guideline in `components/<Name>/README.md` and live markup in `components/<Name>/preview.html`: copy that markup.
- Themes: light by default; `data-theme="dark"` on a wrapper switches to dark.
- Flat by rule: no resting shadows, 1px borders. State colors (success, warning, danger, info) always come with an icon and a word.
- Icons: Phosphor Icons, bold weight (`@phosphor-icons/react` or the bold SVGs).
