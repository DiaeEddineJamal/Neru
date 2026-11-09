# Team mode UX study

Purpose: guide the redesign of the Team screen in Neru (task rail, thread and composer, header, 7-tab side panel, empty state). The current screen is crowded, shows too many controls at once and has inconsistent spacing. This study collects established UX principles, marks what is sourced versus inferred, and turns them into concrete rules.

Evidence note: searches returned no primary Anthropic design document. Claims about Anthropic or Claude design below come from secondary writing or from observing the product, and are labelled.

## Principles

1. **Defer secondary options (progressive disclosure).** Show the primary path first and move advanced or rarely used options to a subsidiary view. It makes the UI easier to learn. Caveat: every layer adds navigation depth, so do not bury frequent actions.
   Sources: NN/g progressive disclosure video; UX Magazine on cognitive load vs discoverability.
2. **Match density to the task.** Density is managed by what is shown by default, not by shrinking everything. Redundant repetition of the same data in several places adds clutter and does not count as density.
   Sources: UX Collective on information density; Algolia post on density and disclosure.
3. **Proximity is the strongest grouping cue.** Elements that sit close together read as related, and this can override colour or shape similarity. Gaps between groups must be visibly larger than gaps within groups.
   Source: NN/g proximity.
4. **Common region (a boundary or card) makes a group explicit.** A boundary can override proximity and similarity. Overuse creates clutter, so use one card pattern and do not nest boxes.
   Source: NN/g common region.
5. **Similarity signals role.** Same role means same look: all task rows alike, all member cards alike, all section headers alike. Differences should mean something (state, priority).
   Source: NN/g Gestalt overview; Toptal Gestalt principles.
6. **Use one spacing scale on a 4/8 base.** Use 4px steps inside small components and 8px multiples between components and sections. Consistent steps reduce decision load and create vertical rhythm. Treat it as a relative system, not a rigid pixel grid.
   Sources: Atlassian spacing; Queensland Design System spacing; spec.fm 8-pt grid.
7. **Start with too much whitespace, then trim.** It is easier to see too much space than too little. Spacing between groups should exceed spacing within groups.
   Source: Refactoring UI (book and summaries).
8. **Hierarchy through weight, colour and spacing, not size alone.** Design in grayscale first. De-emphasize secondary actions (ghost or text buttons) and keep one clear primary action per region.
   Source: Refactoring UI.
9. **Fewer borders.** Separate with spacing, subtle background shifts or soft shadow instead of lines around everything.
   Source: Refactoring UI.
10. **Aesthetic and minimalist design (Nielsen #8).** Remove irrelevant or rarely needed information from each view.
    Source: Nielsen's heuristics summaries.
11. **Recognition over recall (Nielsen #6) and consistency (Nielsen #4).** Keep options visible and labelled (icons plus tooltips or labels) rather than requiring memory, and keep the same pattern for the same job across the screen. Tension with principle 10: hide rarely used things, not frequent ones.
    Source: Nielsen's heuristics summaries.
12. **Calm, warm, typography-led product feel (secondary and inferred).** Secondary analyses describe Claude's interface as warm off-white backgrounds, near-black text, a single terracotta accent and serif type for conversation, as an intentional contrast with cold blue AI products. That is the post's own interpretation, not an Anthropic statement. Our observation (inference): claude.ai leads with text hierarchy over chrome, and its artifacts panel opens beside the chat only when there is content, then closes, which is a progressive-disclosure side panel.
    Sources: rar.design analysis; Double Diamond interview with Jenny Wen (Anthropic design lead; not read in full).

## What this means for Team mode

Spacing and structure
- One scale only: 4, 8, 12, 16, 24, 32, 48 px as tokens. No other literals. Inside a component use 4/8/12, between components 16/24, between page sections 32/48.
- One card pattern: 12-16 px padding, one radius, one surface colour, hairline border or shadow but not both. No card inside a card.
- One list-row pattern for tasks, members, artifacts, changes and activity: fixed height (e.g. 40 or 48), leading icon or avatar, title plus one muted meta line, trailing slot only on hover or focus.
- One section-header pattern: small, medium-weight, muted label, 24 px above and 8 px below, optional single trailing text action.
- Group by space first (principle 3), by card second (principle 4), by border last.

Header
- Show only: task title, member chips (collapse to "+N" after 3), panel toggle, and the primary state control (Stop only while running).
- Move usage pill, help and everything else into one overflow menu, or into the Usage tab. Usage is shown in the header only when near a limit.

Task rail
- Search on top; filters collapse into one filter button with active-count badge. Rows use the list-row pattern; status is one dot, not text.

Thread and composer
- Single centred reading column with a max width; composer docked with one primary send action. Secondary composer controls (attachments, mode) are ghost icons with tooltips.

Side panel
- Tab bar of 7 icons with tooltips and a text label on the active tab only. Panel is closed by default until there is something to show or the user toggles it.
- Each tab opens with the essentials; advanced items sit behind a disclosure.
- Member card shows name, role and status only. Model, permissions, working directory and other settings go behind an "Advanced" disclosure or detail view.
- Consider hiding rarely used tabs (Map, Activity) in an overflow when width is tight.

Empty state
- One headline, one sentence, one primary button ("Start a team task"). Agent cards, templates and the checklist become a secondary, collapsed or scrollable area below, not four equal buttons. Show the checklist only until completed, then remove it.

Colour and emphasis
- Warm neutral surfaces, one accent colour used for at most one element per region (primary button or active state, not both plus badges). Status colours only for real status. Secondary actions use text or ghost style.

Responsive
- At 920 px and below: the rail becomes a drawer, the side panel becomes an overlay sheet, and the header keeps title plus panel toggle and overflow.
- At 640 px: single column, composer full width, tabs scroll horizontally or collapse to a select, tap targets at least 40 px. No horizontal page scroll.

Audit checklist before shipping
- Count simultaneously visible controls per region (target: fewer than 7 in the header, 1 primary action per region).
- Grep for spacing literals outside the token scale.
- Check every repeated pattern uses the shared component (consistency).

## Sources

Found in searches (summaries read from search snippets; full pages were not all opened):
- NN/g, Progressive Disclosure (video): https://www.nngroup.com/videos/progressive-disclosure/
- UX Magazine, Balancing cognitive load and discoverability: https://uxmag.com/articles/balancing-cognitive-load-and-discoverability
- UX Collective, Designing for information density: https://uxdesign.cc/designing-for-information-density-69775165a18e
- Algolia, Information density and progressive disclosure: https://www.algolia.com/blog/ux/information-density-and-progressive-disclosure-search-ux
- NN/g, Proximity (Gestalt): https://www.nngroup.com/articles/gestalt-proximity/
- NN/g, Common region: https://www.nngroup.com/articles/common-region/
- NN/g, Proximity video: https://www.nngroup.com/videos/proximity-gestalt/
- Toptal, Gestalt principles of design: https://www.toptal.com/designers/ui/gestalt-principles-of-design
- Atlassian spacing: https://atlassian.design/foundations/spacing
- Queensland Design System spacing: https://www.designsystem.qld.gov.au/styles/spacing
- spec.fm, 8-Point Grid: https://spec.fm/specifics/8-pt-grid
- VMware Tanzu, 8 point grid and vertical rhythm: https://blogs.vmware.com/tanzu/8-point-grid-vertical-rhythm/
- Refactoring UI (book site): https://refactoringui.com/
- Refactoring UI summaries: https://sglavoie-github-io.pages.dev/posts/2023/09/09/book-summary-refactoring-ui/ and https://bootcamp.uxdesign.cc/refactoring-ui-an-essential-guide-72014f1d77a4
- Nielsen's heuristics summaries: https://thedecisionlab.com/reference-guide/design/nielsens-heuristics and https://uxdesign.cc/10-usability-heuristics-for-ui-design-c60c10e795aa
- rar.design, Claude interface design philosophy (secondary, interpretive): https://rar.design/posts/claude-interface-design-philosophy
- Double Diamond, Jenny Wen, design lead at Anthropic: https://doublediamondnyc.substack.com/p/jenny-wen-design-lead-at-anthropic
- Kinfolk, The Artisans of AI (not read): https://www.kinfolk.com/stories/the-artisans-of-ai/

Gaps: no primary Anthropic design guideline was found; fetch the NN/g articles and the Refactoring UI book directly before quoting them.

## Audit of Team mode (before, 2026-11-09)

Screenshots were taken in the running app at 1335x751 CSS px, plus 1000 and 640 px, in dark and light themes.

Crowding and hierarchy
- The header carried title, member chips, usage pill, overflow menu, help and panel toggle. The roster scrolled sideways and clipped the second member (`@c…`) in a 531 px thread column.
- Member cards showed 4 icon actions, a trash icon, a "Ready" pill, model, effort, a worktree switch and a 2x2 access grid all at once. Two members filled the whole panel.
- The empty state had a hero, a 6-card agent grid, 4 templates and 4 equal buttons. The primary "Start a task" sat below the fold, and "Import chats & skills" was repeated (rail and empty state).
- Every rail row took three lines (title, meta, a filled "Imported" pill), so about 8 tasks fit.

Alignment and spacing
- The panel's tab bar (7 tabs + wide toggle) overflowed 295 px, and the wide toggle was clipped at the right edge.
- Spacing was ad hoc (2, 4, 6, 7, 9, 10, 14, 22, 26 px) although `--space-*` tokens exist in `index.css`.
- `.empty-small` added 20 px of padding inside panels, so empty messages were indented against their headings (Activity, Usage).
- "Route on limits" used the Settings row style (16 px title), louder than the member names above it.
- The thread had a horizontal scrollbar: an unbreakable token in a reply made the scroller 3989 px wide.

Responsiveness
- Below 920 px the task rail was hidden with no way to reach it.
- Below 920 px the panel sheet covered the header, including its own toggle, so it could not be closed. It also opened over the thread by default.

Consistency
- Cards used several paddings (10, 12, 14 px). Section headers mixed uppercase labels, 16 px titles and none. Status was shown both as a pill ("Ready") and as an avatar dot.

## What changed

- Spacing: Team.css and TeamPanels.css now use the `--space-*` scale (4/6/8/12/16/24/32/48).
- Header: shows the title, avatar-only member toggles (tooltip names them), Stop while running, and the panel toggle. The usage pill appears only when a limit is filling up. Usage, the tour and the task options live in the "…" menu.
- Panel: an icon-only tab bar with tooltips, then one section header (title + wide toggle, or close on narrow) for every tab.
- Member cards: avatar, handle and a summary line ("Claude Code · Edit files"), with a status pill only when not ready. Model, effort, access (now one select with its hint) and worktree are behind a disclosure. Terminals, fork, transcript filter and remove are in a per-member menu.
- Route on limits uses a compact setting row under a divider.
- Rail: two-line rows (title + date; avatars, project, one label). Import is a quiet footer row.
- Empty state: headline, one line of copy, then "Start a task" (primary) and "Take the tour". Templates and agents follow as sections with the same header pattern, and "Look again" is the agents section's action.
- Narrow (≤920 px of Team width): the rail becomes a drawer opened from the header, with a scrim. The panel is a sheet with its own close button and starts closed when a task opens.
- Thread: long unbreakable text wraps instead of scrolling sideways.
