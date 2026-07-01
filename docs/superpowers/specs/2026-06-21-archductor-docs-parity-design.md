# Archductor Docs Parity Design

## Goal

Make Linux Archductor feel much closer to `conductor.build/docs` without
rewriting the product or losing the denser Linux-specific workflow areas that
already exist.

This is a visual and interaction-quality pass, not a product-scope change.
The target is to remove the current "generic AI app" feel and replace it with
Archductor-like hierarchy, spacing, restraint, and clarity.

## Customer Impact

The current app works, but its visual quality lowers trust and makes the
product feel rougher than the workflow it already supports.

This pass should improve:

- first impression when opening the app
- perceived product quality and focus
- clarity of navigation and page hierarchy
- readability in dashboard, project, history, and workspace views
- consistency between the product goal and the interface users actually see

## Chosen Direction

Use the **Hybrid Linux Docs** direction.

That means:

- the overall shell should feel like `conductor.build/docs`
- the app should keep its current structure instead of being rebuilt into a
  literal docs website
- denser workspace tools should stay denser, but fit the same design system
- Linux flavor should be visible, but secondary to Archductor parity

This is the smallest change that can materially improve quality without
breaking the current workspace-heavy product shape.

## Non-Goals

- No product architecture rewrite
- No page routing rewrite
- No new major features
- No attempt to make every workspace surface as roomy as a documentation page
- No speculative theming system beyond what this pass needs

## Design Principles

### 1. Archductor first

The app should read as "Archductor desktop control plane" before it reads
as "Linux customization layer" or "AI tool".

### 2. Calm shell, dense tools

Use docs-style calmness for the shell:

- lighter page framing
- quiet headers
- restrained accent use
- simpler borders
- fewer heavy dark blocks
- more whitespace between major sections

Keep tool density where density is useful:

- terminal
- checks
- transcript
- diff/review
- process/status areas

### 3. Simpler hierarchy

The current interface uses too many similar dark panels and too many competing
surfaces. The redesign should make the information hierarchy obvious with:

- background/surface contrast
- typography
- spacing
- section grouping
- selective use of mono and status accents

### 4. Linux flavor as seasoning

Linux-specific feel should come from:

- stronger terminal surfaces
- sharper utility panes
- mono labels where appropriate
- subtle green/neutral status accents

It should not come from turning the whole app into a dark terminal dashboard.

## Reference Surface

`conductor.build/docs` is the source of truth for:

- visual rhythm
- spacing restraint
- typography hierarchy
- navigation calmness
- page framing
- border weight
- surface simplicity

Linux Archductor should not copy docs literally where the product shape differs.
The docs site is the reference for taste and hierarchy, not a strict layout
template for every screen.

## Scope

### In scope

- shell styling tokens
- sidebar styling and workspace list treatment
- header bar styling
- page container spacing
- dashboard card and board styling
- projects page styling
- history page styling
- workspace command center styling
- tabs, lists, buttons, inputs, chips, and utility panels
- terminal, transcript, checks, and diff styling adjustments for better
  integration with the shell

### Out of scope

- changing workflow logic
- changing data models
- reworking launch targets, state flow, or page ownership
- building a full user-facing theme editor

## Shell Design

The shell should move from a heavy dark AI-app feel to a quieter docs-inspired
frame.

### Sidebar

Target behavior:

- lighter and calmer than today
- clearer grouping between top-level nav and workspace list
- better active-state contrast without chunky button feel
- workspace rows should read like deliberate list entries, not dashboard chips

Expected changes:

- soften sidebar background and border contrast
- reduce button-like appearance of navigation items
- improve workspace row spacing and text hierarchy
- make repo section headers look editorial instead of terminal-like

### Header Bar

Target behavior:

- less visually heavy
- clearer utility role
- more neutral chrome

Expected changes:

- simplify header background and border
- improve icon button affordance and spacing
- reduce "toolbar blob" feel

### Page Containers

Target behavior:

- better whitespace rhythm
- clearer separation between page title area and content
- less crowded body padding

Expected changes:

- normalize section spacing
- normalize internal padding across main screens
- make title/subtitle relationships more consistent

## Workspace Design

The workspace view is the hardest area because it combines product shell,
status dashboard, transcript, terminal, checks, and review workflows in one
place.

The decision is:

- keep current workspace structure
- do not force it into a docs article layout
- bring it into the same visual system as the shell

### Workspace outer frame

The page header, summary strips, metric cards, and top-level sections should
become calmer and closer to docs-style hierarchy.

### Workspace inner tool surfaces

Terminal, transcript, checks, and diff surfaces should stay darker and denser
than the shell, but they should look intentional rather than bolted on.

That means:

- fewer unrelated background colors
- consistent radii and border treatment
- clearer contrast between shell surface and tool surface
- stronger typography consistency

## Component Direction

### Typography

Shift toward a cleaner docs-style hierarchy:

- stronger page titles
- quieter metadata
- less all-caps shouting
- more deliberate mono usage

Mono should be reserved for:

- branches
- paths
- commands
- ports
- terminal/transcript content
- compact system metadata

### Colors

Use a calmer base palette with:

- light or near-light shell surfaces
- neutral borders
- dark text on shell surfaces
- selective green/graphite/Linux accents

Dense tool panes may remain dark for utility, but dark should stop being the
default for every surface.

### Borders and Radius

Use simpler panel framing:

- thinner-feeling borders
- more consistent radius scale
- fewer heavy card stacks
- less contrast noise between neighboring panels

### Buttons and Inputs

Controls should feel more like Archductor docs utility controls and less like
generic SaaS dashboard controls.

Expected changes:

- quieter default buttons
- clearer primary actions
- better hover/focus states
- inputs that integrate with the shell instead of floating as separate themes

## Interaction Direction

The app should feel calmer to use even when the workflow is dense.

### Navigation

- clearer selected states
- more predictable hover behavior
- less visual jumpiness

### Workspace selection

- selected rows should be obvious without looking like oversized pills
- metadata should help scanning instead of adding noise

### Tabs and switches

- active states should be clearer
- tab chrome should be simplified
- avoid overly dark segmented-control feel where not needed

## Implementation Approach

Keep the diff small and direct.

### Step 1. Extract and reorganize styling

Move the large inline CSS blob into a focused theme module or stylesheet source
that is easier to edit intentionally.

This pass should also define a small token set for:

- shell background
- shell surface
- tool surface
- text
- muted text
- border
- accent
- spacing
- radius

### Step 2. Restyle shell primitives

Update the shared primitives first:

- sidebar
- nav rows
- workspace rows
- header bar
- page headers
- cards
- panels
- labels
- inputs
- buttons
- switchers

This gives most screens the new feel without rewriting screen logic.

### Step 3. Restyle top-level pages

Apply the new system to:

- dashboard
- projects
- history

These are the highest-value pages for first impression and product trust.

### Step 4. Restyle workspace command center

Bring the workspace page into the same system while preserving density where it
matters.

Focus on:

- command center strip
- metric cards
- command panels
- workspace tabs
- review/check surfaces
- session panel framing

### Step 5. Polish dense tool panes

Adjust terminal, transcript, diff, and checks styles so they:

- still feel useful and high-contrast
- no longer clash with the shell
- use one coherent visual language

## Risks

### Risk: docs parity gets too literal

If the shell becomes too roomy, the workspace tools will feel inefficient.

Mitigation:

- keep the docs influence strongest in shell and framing
- keep dense tools denser on purpose

### Risk: purely cosmetic pass leaves structure problems visible

Some screens may still feel rough after token changes alone.

Mitigation:

- allow small layout cleanups inside existing widgets
- avoid broad rewrites unless a specific screen proves impossible to fix with
  targeted adjustments

### Risk: Linux flavor overwhelms Archductor parity

Too much dark chrome or mono usage will pull the app back toward "developer
tool dashboard".

Mitigation:

- keep Linux flavor mostly in utility panes and small accents
- keep the shell restrained

## Testing

Verification should focus on visual quality and no-regression behavior.

Required checks:

- `cargo fmt --all -- --check`
- `cargo test -p linux-archductor-gtk`
- GTK app launch after styling changes

Manual review targets:

- dashboard
- sidebar
- projects page
- history page
- workspace page
- terminal/transcript area
- checks/review surfaces

## Success Criteria

This pass is successful if:

- the app no longer reads like a generic AI dashboard
- the shell clearly feels closer to `conductor.build/docs`
- Linux flavor is visible but restrained
- workspace tools still feel efficient
- the implementation is mostly styling and targeted layout cleanup, not a
  product rewrite

## Recommendation

Proceed with a staged implementation plan built around shell-first restyling,
then workspace integration, then dense-pane polish.
