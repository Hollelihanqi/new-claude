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
- 31 test files and 165 tests passed.
- Production frontend build and debug macOS application bundle completed.
- Real-app interaction verified the Apple card and 4×2 layout, selected/live-preview state, dark-mode content hierarchy, sidebar contrast, and top chrome synchronization, in addition to all five festival skins and Lantern Festival tangyuan.
- The bundle command exits non-zero only after producing the `.app`, because a release updater public key is configured without its private signing key in the local environment.

Final result: passed.
