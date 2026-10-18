# Design QA — 会话复制（2026-10-18）

- Source visual truth: `docs/testing/chatgpt-copy-redesign-2026-10-18/approved-design.png`，用户确认的第 1 版及最终按钮文案。
- Implementation screenshot: `docs/testing/chatgpt-copy-redesign-2026-10-18/desktop.png`。
- Viewport: 1440 × 832 CSS px；source 1650 × 953 pixels，归一化到 1440 × 832 后对照；浏览器截图 1440 × 832 pixels。IAB 对桌面覆盖视口的截图有重采样，辅以 DOM 读取的 16px 标题、14px 正文及布局几何检查。
- State: 仅合成的来源/目标实例，勾选 1 条会话，右侧显示相应提问和回答；复制后打开默认不勾选。
- Full-view comparison: `docs/testing/chatgpt-copy-redesign-2026-10-18/comparison.png`，同一画布并排显示设计稿和实际组件。
- Focused comparison: `docs/testing/chatgpt-copy-redesign-2026-10-18/footer-comparison.png`，核对底部按钮、目标名称和独立打开选项。

## Findings and comparison history

1. [P2, fixed] 第一轮长列表/长正文使工作区超出桌面视口；限制双栏高度，列表及正文分别滚动，底部操作可见。最终桌面截图无横向溢出（documentWidth = viewport = 1440）。
2. [P2, fixed] 第一轮搜索后仍预览被过滤掉的会话；改为预览当前结果，并提示被当前搜索隐藏的已选会话，避免复制对象不明。
3. [P2, fixed] 次要批量选择及格式说明占据列表空间；移到「更多设置」，不完整扫描仍在主区域提示。
4. [P3, accepted] 账号选择框沿用现有可搜索/可清空下拉，名称与邮箱同一行；保留原有组件体系、键盘操作与错误提示。真实正文按原文显示，不对业务内容擅自改写。
5. [P3, accepted] 设计图使用示例的语义标题；真实数据使用可读的已有标题或真实提问摘录，无有效内容时明确标为未命名，不捏造会话主题。

## Required fidelity surfaces

| Surface | Result |
| --- | --- |
| Fonts / typography | 沿用系统字体与项目字体回退；会话标题 16px、正文 14px，长标题最多两行，正文保留换行。 |
| Spacing / layout | 桌面 40% / 60% 双栏；来源与目标工具条、正文、底部操作的层级与设计一致。640px 窄屏单列，documentWidth = viewport = 640。 |
| Colors / tokens | 使用现有主题变量、蓝色主按钮和浅色表面；深色主题截图验证文字及控件可识别。 |
| Image quality / assets | 无产品图片；图标均复用 Tabler 图标库，未添加手绘图标或占位图片。截图仅作验证证据。 |
| Copy / content | 「复制所选会话（N 条）」和「复制完成后打开目标会话」准确区分两个动作；UUID 和环境指令不再作为标题。 |

## Interactions tested

- 搜索标题/摘要、切换预览、勾选与预览分离、清空选择、无记录时禁用复制。
- 来源及目标选择、高级范围和目录选项、复制范围确认、默认只复制、主动勾选后打开。
- 运行目标的关闭确认及取消；该场景同样仅操作内存中的合成目标。
- 正文加载失败重试、迟到响应保护、100 项批次与中断恢复的回归测试。
- 正文只读回归覆盖 Windows/macOS 两套目录布局、普通/压缩文件、错误版本、错误来源，核对来源文件内容、修改时间及登录标记未改变。
- Console errors: IAB 页面检查为 []。

## Boundaries and cleanup

- 三个生产实例未启动、关闭或写入。界面验证直接复用真实组件，所有服务方法替换为内存测试实现；后端测试使用自动回收的临时目录。
- 浏览器覆盖视口、临时页面和测试服务在验证后恢复/关闭；临时测试目录及脚本移除。仅保留本报告与设计/验证截图。
- 此结果针对组件界面及隔离测试；不能替代真实账号端到端验证或 Windows/macOS CI。

final result: passed

---

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
| Colors and visual tokens | Pass | Surfaces follow the active theme; the current installed-environment switch uses the fixed semantic success color `#00A675` in light and dark modes. Earlier screenshots in this historical comparison used the former green and should not be treated as current visual evidence. |
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

## Dark-mode menu audit and refinement — 2026-09-21

- Tested surface: running macOS Tauri development app at 1600×980 CSS pixels, Ocean theme in dark mode. This is the real application, not a static browser mockup.
- Screenshots captured after clicking each menu: `/tmp/pathmux-audit-01-environment.png`, `/tmp/pathmux-audit-02-workbuddy.png`, `/tmp/pathmux-audit-03-mcp.png`, `/tmp/pathmux-audit-04-extensions.png`, `/tmp/pathmux-audit-05-usage.png`, `/tmp/pathmux-audit-06-diagnostics.png`, `/tmp/pathmux-audit-07-settings.png`.
- Additional extension-state captures: `/tmp/pathmux-audit-plugin-dark-3.png`, `/tmp/pathmux-audit-agent-dark.png`, `/tmp/pathmux-audit-plugin-drawer-dark.png`.
- Help screen capture: `/tmp/pathmux-audit-08-help.png`.
- Final affected-state captures: `/tmp/pathmux-audit-after-settings.png` (category control in title row), `/tmp/pathmux-audit-after-workbuddy.png` (neutral editor/privacy surface), `/tmp/pathmux-audit-after-diagnostics.png` (coherent overview/actions), `/tmp/pathmux-plugin-border-fixed.png` (full selected-tab outline), `/tmp/pathmux-disabled-final.png` (neutral disabled install action).

| Step | Surface | Result / finding |
| --- | --- | --- |
| 1 | Environment | Pass: list items and editor share the dark surface hierarchy. |
| 2 | WorkBuddy | Fixed: the editor and privacy reminder previously used discordant gray-brown fills; they now derive from the shared dark surface with only a restrained reminder tint. |
| 3 | MCP | Pass: overview, toolbar, table, and empty state use coherent dark layers. |
| 4 | Extensions — Skills, Plugins, Agents, install drawer | Fixed: the selected environment and source tabs had a top-only inset highlight; they now have a complete subtle outline. The disabled install button previously inherited a brown-gray Mantine default; disabled actions now use the app's neutral surface. |
| 5 | Usage | Fixed latent issue: populated statistic cards had hard-coded pastel light backgrounds. Their surface and text now mix with semantic dark/light tokens. Current local data is empty, so the populated-card result cannot be visually confirmed from this account. |
| 6 | Diagnostics | Fixed: overview tint and default secondary buttons now use shared dark surface tokens instead of isolated gray-brown values. |
| 7 | Settings | Fixed: theme-category tabs now sit to the right of the section title rather than occupying a full row; narrow windows wrap them below the title. Both categories were clicked in the running app and their card galleries changed without changing the applied theme. |
| 8 | Help | Pass: information cards, timeline, code chips, and text remain within the shared dark material hierarchy. |

Accessibility limits: screenshots verify visual contrast and state affordances only; they are not a substitute for a full keyboard/screen-reader or WCAG conformance audit. The dormant populated-usage state remains a visual verification gap because there are no local usage records under the current filter.

Verification: 31 frontend test files / 168 tests passed and production frontend build passed. The real app was restarted and affected dark-mode screens were re-opened after fixes. No backend code changed in this audit.

Final result: passed for observable states; populated usage-card appearance remains unverified in real local data.

## Plugin selector and card refinement — 2026-09-21

- Visual reference: user's annotated original layout at `/var/folders/03/yv_w5nj92ls8hbxwt4ncq2jm0000gp/T/codex-clipboard-472646d2-4427-46de-8b45-c99e272e7b4e.png`. The three alternative redesigns were rejected; this pass preserves the original long segmented control.
- Real-app captures: `/tmp/pathmux-plugin-refine-dark.png` and `/tmp/pathmux-plugin-refine-light.png`, each captured from the running macOS Tauri app at 1600×980 CSS pixels.
- Selector: pass. The previously undefined rail background token has been replaced with a real surface token. The active tab is inset on all four sides and uses a restrained filled surface rather than blue text on a nearly transparent background. Both environment choices and both color modes were exercised in the app.
- Cards: pass. Minimum height changed from 238 px to 224 px, with small reductions in vertical padding and internal gaps. Identity, description, environment switch, and actions remain legible and unclipped in the real card.
- Redundant status line: removed “正在查看：环境 hq” from the plugin-list heading; the selected tab still communicates the current filter.
- Status switch: replaced an undefined background token for the off state, so both enabled and disabled states have an actual surface fill.

## Plugin action hover states — 2026-09-21

- Source: user's dark-mode button crop `/var/folders/03/yv_w5nj92ls8hbxwt4ncq2jm0000gp/T/codex-clipboard-20b32ae9-66ef-4db5-a978-9c9a3b62a8e8.png` (882×118 px, default state). Its visual design is retained; the requested hover state is an intentional addition, not shown in the source.
- Rendered implementation: real macOS Tauri app at 1600×980 CSS px / 3200×1960 Retina capture. Dark hover evidence: `/tmp/pathmux-plugin-update-hover.png`, `/tmp/pathmux-plugin-uninstall-hover.png`. Light hover evidence: `/tmp/pathmux-plugin-update-hover-light.png`, `/tmp/pathmux-plugin-uninstall-hover-light.png`.
- Focused comparison: the user's button crop and dark hover capture were opened together. Font, icon, dimensions, spacing, border radius, and base color remain aligned with the existing card. No new imagery or copy was introduced.
- Interaction: moved the actual system pointer over each enabled button in both modes. Update subtly brightens with a blue tint; uninstall gains a restrained red tint; each gains a small elevation and has a pressed-state return. Disabled buttons are excluded from hover styling.
- Result: no actionable P0/P1/P2 difference; `final result: passed`.

## Plugin feedback and confirmation refinement — 2026-09-21

- Source visual truth: four user-supplied crops `codex-clipboard-6a58b6ba-aadc-4bf9-8184-97979f63884f.png`, `codex-clipboard-2d34602e-701a-4616-9cd7-1d09df23a145.png`, `codex-clipboard-906e88df-a8ae-416a-a359-ec38b5ced991.png`, and `codex-clipboard-ee264299-9299-4c86-a264-f2f83398c595.png` under `/var/folders/03/yv_w5nj92ls8hbxwt4ncq2jm0000gp/T/`. They identify disliked current states, not a replacement mockup.
- Rendered implementation: running macOS Tauri app at 1600×980 CSS px, Retina 3200×1960 captures. Dark single-environment modal `/tmp/pathmux-plugin-remove-hq-dark-new.png`, light modal `/tmp/pathmux-plugin-remove-modal-light-new.png`, pending switch `/tmp/pathmux-plugin-toggle-pending-dark.png` and `/tmp/pathmux-plugin-toggle-pending-light.png`, dark result toast and restored enabled state `/tmp/pathmux-plugin-toggle-restored-dark.png`.
- Same-state focused comparison: `/tmp/pathmux-modal-hq-qa-comparison.png` places the 1120×550 original dark single-environment modal crop beside an identically sized crop from the rendered app. Full-view app captures above establish context. Source crop and implementation were kept at equal pixel size; the app capture uses 2× device density.
- Finding P2, modal: default charcoal conflicted with the surrounding navy app surface. Fixed with app-surface tokens for dialog, header, text, border, and cancel control. Confirmation remains distinctly destructive.
- Finding P2, feedback: the successful-operation alert occupied permanent page space and the stock notification used an unrelated gray background. Removed the permanent alert; the short toast uses the same light/dark surface as the app. Partial failures retain environment-specific details in an error alert.
- Finding P2, switch: there was no visible in-progress state. The corresponding switch thumb now displays a rotating icon while the real plugin command runs; the other controls cannot accept conflicting operations. Reduced-motion preference leaves the icon static but still visible.
- Fonts, spacing, assets, and copy: the established card/modal type hierarchy and Tabler icons remain unchanged; only dialog colors, feedback placement, and pending state changed. No raster artwork was introduced. Both themes were inspected in the real app; the plugin was disabled and re-enabled and is left enabled, with all environments selected again.
- Final result: passed.
