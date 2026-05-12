# Responsive Density Rules

Dbunk is a developer database application, so the active workspace must keep the
largest possible share of the window. The release UI refresh optimizes for
half-window, quarter-window, and one-sixth-window use without making narrow
layouts feel broken.

## Priorities

1. Protect the center workspace first: query editor, results grid, table grid,
   Redis CLI, and key inspector.
2. Collapse optional sidebars before shrinking the workspace below usefulness.
3. Use container width decisions where possible; viewport breakpoints are only a
   fallback.
4. Keep behavior predictable when users resize, pin, unpin, hover, or use
   keyboard/touch controls.

## Sidebar Rules

- The global left sidebar, relational query sidebar, relational row details
  sidebar, and Redis keyspace sidebar are optional in compact layouts.
- Compact layouts default optional sidebars to collapsed.
- On pointer devices with hover support, hovering the relevant window edge may
  reveal a collapsed sidebar as an overlay.
- Touch and keyboard users can always use explicit toggle buttons instead of
  relying on hover.
- Revealed compact sidebars overlay the workspace. They do not push the grid or
  editor around during hover.
- Pinning a sidebar reserves space only if the center workspace can retain at
  least `560px`. If not, the pinned sidebar remains a persistent overlay and
  closes on outside click.
- Pin choices are persisted by sidebar type, not by individual table or query
  tab.
- Redis keeps the CLI and key inspector as primary workspace content. Its
  keyspace browser follows the same compact sidebar rules with Redis-specific
  widths.

## Density Rules

- Shared component tokens should shrink first: buttons, inputs, cards, toolbar
  heights, table row heights, and icon sizes.
- Pane-specific spacing should only override shared density when the workflow
  needs it.
- Non-essential actions collapse to icon-only on compact widths with accessible
  labels and `title` text.
- Primary, commit, and destructive actions keep text while space allows: Run,
  Save changes, Insert, and Delete selected. At the smallest widths, they may
  also become icon-only if placement and labels remain clear.
- Tables and editors should scroll rather than wrap into unusable layouts.

