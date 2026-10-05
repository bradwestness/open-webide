# Frontend design system

Use `frontend/src/components/ui.rs` and the tokens in `frontend/styles.css` when composing controls. Icons come from the Rust `lepticons` Lucide library, behind `ui::Icon`; category features keep the dependency scoped to application controls. No emoji or font-dependent glyphs for action icons.

- `Modal` owns focus, dismissal, header and size. Compose its scrolling content with `DialogBody`, `FormSection`, `FormField`, `CheckboxField` and `FormNotice`. Put its final actions in `DialogActions`, outside the scrolling body. Describe sections in sentence case; keep helper text next to its field. Group related controls with `FormField group=true` rather than placing multiple controls inside a label.
- Use standard `Button` variants and `InlineActions`. Use `IconButton` for compact actions with an accessible label. `Icon` renders decorative 20px SVGs in the current theme color; the parent control supplies meaning. Top-bar actions retain desktop text. Action tooltips appear after 200ms on hover and immediately on keyboard focus, escape clipping, and disappear on Escape, pointer activation, scrolling or cleanup.
- `DisclosurePanel` retains mounted content when collapsed. Live tool calls share counts in first-use order; approval requests force the relevant group open. Thinking retains its separate text and token summary.
- `ToolPanel` owns headings, ordering buttons, dock widths and full-height resize boundaries. Handles sit inside the panel edge independently of content wrappers. Feature panes provide content. `FilesPanel` composes Explorer/Changes selection inside `SearchPane`, which switches between mounted file views and results. `LayoutState` owns limits and fitting; `LayoutActions` saves preferences and widths through user-scoped database settings, with serialized writes and account-generation guards. Browser events belong to `PanelResizer`; local/remote feature operations still use existing Workspace and ProjectGit facades.
- Phone tool sheets use the available width and retain desktop preferences. Collapsing panels and switching file views preserves their mounted state. Editor uses remaining width; each adjacent tool window has the same resize behavior and keyboard support.
- The composer uses a shared browser sizing primitive, grows up to 80% of its pane while reserving transcript/chrome space, and shrinks on programmatic draft changes. Whole-pane drops and paste/picker input all call `Composer::import`, preserving validation and project/session/account guards.

Existing shared CSS classes remain supported while components migrate. Colors, spacing, typography, radii, dialog sizes and control height come from theme/design tokens. Avoid adding private control styles to individual dialogs or panes.

Reasoning, run context and tools use `DisclosurePanel` for left-aligned headers, caret controls and retained collapsed content. Active reasoning adds its timing, token estimate and spinner to the shared header.

`PanelToolbar` groups contextual information and actions beneath the single `ToolPanel` heading; feature panes do not repeat their panel or selected-view titles. Explorer and Changes share file-row spacing, icons and selection treatment.

`BranchPicker` provides the same native branch selection and New branch action in Changes and the footer. `GitActions` owns discovery, checkout and prompt scope, using `ProjectGit` adapters and guarding late results by project, account and host revision.
