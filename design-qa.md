# Design QA — Theme Studio

- Source visual truth: the user's Apple HIG direction plus `/Users/hq/.codex/generated_images/01a0b8ba-2bbd-7442-8450-fe48425127a6/exec-acb86f5a-c173-4d2d-9697-dd60fd5ee390.png` for the Apple Liquid Glass backdrop; the earlier festival source remains `/Users/hq/.codex/generated_images/01a0b8ba-2bbd-7442-8450-fe48425127a6/exec-b9f44926-88cc-4208-ab2f-021ddefc333e.png`.
- Implementation under test: `/Users/hq/workspase/new-claude/src-tauri/target/debug/bundle/macos/PathMux.app`
- Implementation evidence: inline CUA captures of the real macOS app at 980×720; the capture API does not expose a filesystem path.
- QA viewport: 980×720 macOS window, system dark appearance.

## Comparison

| Area | Result | Notes |
| --- | --- | --- |
| Theme taxonomy | Pass | The category control is a two-tab segmented control with no right-side festival legend. |
| Daily layout | Pass | Eight daily cards render as a fixed 4×2 grid at the default and supported compact desktop widths. |
| Apple theme replacement | Pass | “暖砂” is replaced by “苹果”; existing `sand` preferences migrate automatically to the new style. |
| Apple clarity | Pass | SF-compatible system font stack, restrained blue accent, stronger spacing, and clear content hierarchy keep decoration subordinate to content. |
| Apple depth | Pass | The native top chrome, sidebar, header, segmented control, and preview use layered translucent materials; content surfaces use quieter standard materials. |
| Apple dynamic colors | Pass | Light mode uses system background/label values and dark mode uses black, `#1c1c1e`, and `#0a84ff`-based semantic tokens. |
| Visual hierarchy | Pass | Page header, theme studio, cards, live preview, and secondary settings preserve the intended hierarchy. |
| Dark-mode coherence | Pass | Shell, header, surfaces, borders, controls, and charts derive from the active theme tokens. |
| Festival identity | Pass | Spring Festival, Lantern Festival, Dragon Boat Festival, Mid-Autumn Festival, and National Day use distinct cultural imagery rather than flat color fills. |
| Festival mood | Pass | Revised assets use brighter daylight, warm highlights, and clearer celebratory color; dark muddy backgrounds were removed. |
| Sidebar readability | Pass | A dedicated scrim, stronger white text, and text shadows preserve menu readability over festival imagery. |
| Native top chrome | Pass | The macOS overlay drag region changes color with the active theme while keeping native traffic-light controls. |
| Lantern Festival detail | Pass | Tangyuan bowls are visible in the full-app skin and the Lantern Festival preview artwork. |
| Interaction | Pass | All theme cards switch immediately; selected states, tabs, live preview, and whole-app styling update together. |
| Festival automation | Pass | Date mapping is covered for lunar 1/1, 1/15, 5/5, 8/15 and Gregorian 10/1, without overwriting the saved daily preference. |

## Verification

- TypeScript typecheck passed.
- 31 test files and 166 tests passed.
- Production frontend build and debug macOS application bundle completed.
- Real-app interaction verified the Apple card and 4×2 layout, selected/live-preview state, dark-mode content hierarchy, sidebar contrast, and top chrome synchronization, in addition to all five festival skins and Lantern Festival tangyuan.
- The bundle command exits non-zero only after producing the `.app`, because a release updater public key is configured without its private signing key in the local environment.

Final result: passed.

## Plugin Management Redesign — Implementation QA

- Source visual truth: `/Users/hq/.codex/generated_images/01a0b8ba-2bbd-7442-8450-fe48425127a6/exec-ebe1f681-5af1-4080-a310-c048aa2d1bc5.png` (1586×992 px).
- Rendered implementation: real macOS Tauri app built from this workspace.
- Primary implementation screenshot: `/tmp/pathmux-apple-drawer-floating.png` (2184×1664 px including Retina window shadow; 980×720 CSS-pixel app window at device scale factor 2).
- Full-view combined comparison: `/tmp/pathmux-design-qa-comparison.png` (source and implementation normalized to 992 px height).
- Focused comparison: `/tmp/pathmux-design-qa-focus.png` (environment selector, card spacing, action affordances, and fixed-green enabled state).
- Responsive evidence: `/tmp/pathmux-responsive-1600.png`, `/tmp/pathmux-plugin-real-wide.png` (980 px), and `/tmp/pathmux-responsive-800.png`.
- State: Apple dark theme with the floating install panel open; one managed environment and one real discovered plugin. Responsive captures also cover the light theme.

### Findings

- No actionable P0, P1, or P2 differences remain.
- The conceptual source uses nine example plugins and branded icons, while the real app correctly displays the user's one discovered plugin with the neutral package icon. This is expected data/content variance, not design drift.
- P3 follow-up: a denser installation with many long environment names could benefit from an additional stress capture, though wrapping and overflow behavior are already defined.

### Required fidelity surfaces

| Surface | Result | Evidence |
| --- | --- | --- |
| Fonts and typography | Pass | System/SF-compatible typography keeps the same weight hierarchy for title, field label, helper text, card name, and actions; no clipping or ambiguous truncation appears in the tested states. |
| Spacing and layout rhythm | Pass | Environment selector is 68 px high with 10 px vertical padding around 48 px tabs; cards use 22 px inner padding and 20 px grid gaps. The install panel is now inset on all sides with full rounding. |
| Colors and visual tokens | Pass | Surfaces follow the active theme, while `.plugin-env-toggle.enabled` remains the same `#30d158` green in Ocean light/dark and Apple dark captures. |
| Image and icon fidelity | Pass | Existing Tabler icons remain sharp at Retina scale. The neutral package icon is intentional because plugin manifests do not provide a guaranteed branded asset. |
| Copy and content | Pass | The interface keeps only task-relevant labels. Card controls expose state through icon, color, environment name, title, and accessible label without redundant “环境状态/点击切换状态” text. |

### Interaction and responsive verification

- The actual app opened the Plugins page and rendered real plugin/environment data.
- The environment selector visibly retains top and bottom padding.
- The card grid rendered at the intended 3-column (1600 px), 2-column (980 px), and 1-column/stacked-toolbar (800 px) breakpoints in real Tauri windows.
- The install panel overlays the unchanged plugin page under a blur/scrim, defaults to all environments through the normal open action, and supports separate remote and local-package tabs.
- The enabled environment control remains fixed green across themes; unit interaction coverage verifies clicking it routes to the exact environment and toggles enable/disable.
- A real isolated Claude Code 2.1.176 command test added a local marketplace, installed a plugin, and listed it as enabled. This exposed and fixed the incompatible legacy `--yes` option without touching the user's managed environments.

### Comparison history

1. Initial rendered panel was flush against the top, right, and bottom window edges, unlike the floating source panel (P1).
2. Fixed `.plugin-install-drawer-content` with 16 px outer margins, calculated height, a full border, 24 px radius, and a responsive 8 px mobile inset.
3. Recaptured `/tmp/pathmux-apple-drawer-floating.png` and recomposed the full comparison. The panel now reads as an independent floating glass layer and no P0/P1/P2 mismatch remains.

final result: passed
