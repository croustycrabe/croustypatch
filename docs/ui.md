# The interface

A flat, app-style interface in the CroustyLab brand. It is a static export (Next.js) served by the Rust server.

## Structure

- **Sidebar** with grouped navigation (overview, fleet, patching, governance, administration), the **Add agent**
  button, and the user menu. It collapses to a drawer on small screens. It stays mounted while pages change.
- **Topbar** with the current section, the **⌘K / Ctrl+K command palette** (pages, hosts, actions) and the language toggle.
- **Pages:** Dashboard · Hosts · Packages · Groups · Tasks · Policies · Waivers · Vulnerabilities · Audit · Agent tokens ·
  Accounts and roles · Settings. Entries and actions are shown only if the signed-in role allows them.
- **Panels:** forms and details that should not hide the page open as a slide-over (policy editor, task detail,
  package hosts, Add agent). Confirmations are small centred dialogs. Feedback is a toast; errors tied to a form stay inline.
- **Hosts** (as in PatchMon): searchable, filterable by status and group, sortable, with bulk patch and scan; a host
  opens into tabs (Overview, Packages, History, Settings). **Packages** shows every package across the fleet and lets
  you update one package on all the hosts that need it.
- **Dashboard:** fleet stat cards, an actionable "needs attention" strip, then draggable widgets (reorder, resize, hide;
  saved per user on the server).

## Add agent

The primary action of the product, reachable from the sidebar, the Hosts page, the Agent tokens page, the empty
states and the palette. See `install.md`. Step 1 chooses mode, group and environment; step 2 shows the one-liner
(Linux curl or wget, FreeBSD, or manual steps), the platforms this server can serve, and a live "waiting for the agent"
line; step 3 confirms the connection and links to the new host.

## Design system

Since phase two the app follows **CroustyLab Software** (`design-system/README.md`), the product-UI variant of the
brand: cream / ink / one brick accent, flat surfaces separated by tone and 1px hairlines, four state colours (success
teal, warning amber, danger crimson, info blue), and a light and a dark theme.

- `frontend/src/styles/tokens.css` and `bundle.css` are copies of `design-system/tokens.css` and
  `design-system/components/bundle.css` (keep them in sync). The Google Fonts `@import` is dropped: the CSP only allows
  self-hosted fonts, loaded with `@fontsource` (Rubik One for headings, Rubik for interface text, Roboto Mono for data).
- `flat.css` is the app layer. Its older variable names (`--ink`, `--paper`, `--hair`…) are aliases of the software
  tokens, so every screen follows both themes. New screens should use the `cs-` classes directly.
- Theme: the saved choice (`cp.theme`), otherwise the system preference, applied before first paint; the sun/moon
  button in the top bar switches it.
- Rules applied: bracketed page titles `[ Hosts ]`, sentence case, state colour always with its icon and word
  (`StateIcon` in badges, alerts and toasts; dot plus word in dense tables), hard unblurred shadow only on floating
  layers, no slide or scale animations, success and info toasts leave after 5 s while errors stay until closed.

## Status colours

The design system's state tones: **success** (teal) online / up to date / succeeded, **neutral** offline / queued /
cancelled, **info** (blue) eco / sending / running / soaking, **warning** (amber) needs patch / awaiting approval /
maintenance / rolling back, **danger** (crimson) alert / quarantined / failed / security updates. They are the `tone-*` classes in
`flat.css`, applied by `HostStatusLabel`, `TaskStatusLabel`, the fleet bar and legend, the dashboard stat cards and
attention rows, and the "needs patch" / "up to date" badges in the hosts table.

## Group colours

Each group has one of nine palette colours (`brick sand moss teal sky plum rose slate ink`) or *automatic*, a stable
pick derived from its name. Choose it when creating or editing a group, or change it from the palette button in the
Groups table. The last swatch opens the system colour picker for any custom colour (stored as `#rrggbb`; the tag
text switches between ink and paper so it stays readable). The coloured `GroupTag` is used in the hosts table, the host settings, the policy editor and the
policy list. The server validates the value: a palette name, a strict `#rrggbb` or empty, otherwise `400`.

## Layout

The app frame is exactly the visible window (`100dvh`) and never scrolls itself. The sidebar, with the user card at
its foot, and the toasts stay on screen at any window size; only the content column scrolls. Covered by the browser
suite at 1280×520, 1024×420 and 800×600.

The menu button in the top bar folds the side menu into an icon rail on desktop (remembered in the browser; tooltips
and accessible names keep the labels) and opens it as a panel on phones, where Escape, a tap outside or choosing an
entry close it.

## Accessibility and behaviour

Keyboard: every control is reachable, panels trap focus and close on Escape, the palette is a combobox with arrow
keys, menus close on Escape or an outside click, and a skip link jumps to the content. Motion respects
`prefers-reduced-motion`. Everything is bilingual (French default); the English table is typed against the French
one, so a missing translation fails the build. Fonts are self-hosted: the interface makes no third-party request.
