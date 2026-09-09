# LuvvyLoL update

Source: Luvvydev/leaguemacoverlay, main commit `7ba28c90d6e6d4ca18d0ea2a9020347aba5a9187`.
Prepared September 9, 2026. Source version remains 0.21.2; this is not a published release.

## Changes

1. Account identity and client credentials are checked during polling. A changed account clears account-specific state and reconnects with the new profile, match history and LP bucket. Preferences are retained.
2. Added 24 historical player labels with descriptions, thresholds and minimum samples. They use available normal/ranked matches from the last 30 days, excluding custom games. The main roster shows applicable labels, and the TAB overlay shows up to three per player plus an expandable full list. Unknown or unavailable histories do not become a false first-time label.
3. Larger default text throughout the app, brighter secondary text, opaque panels, substantially reduced background art, roomier match rows and responsive layout. Text size is selectable from Standard through Largest. The overlay grows with the selected size, stays within the monitor and scrolls when content exceeds its height.
4. See POROFESSOR-COMPARISON.md for the feature audit and remaining gaps. This update does not claim full parity.
5. Unknown ranks no longer default to Gold's 1,200-point base. That old fallback minus Bronze IV at 55 LP yields exactly the screenshot's erroneous -745. The original stored entry is unavailable, so this is a plausible reproduced cause, not confirmation of the user's stored data. Apex tiers share a continuous LP base. New samples explicitly identify Solo/Duo and store win/loss counters. Daily LP requires samples covering the day's observed Solo/Duo games and matching their results. Otherwise it says Unavailable. Flex and other modes cannot contaminate Solo/Duo LP. Legacy samples are excluded from validated calculations. LP samples older than 32 days are pruned during periodic refresh to bound storage.
6. Saved Flash key preference, default F, applied to standard champion select, manual spell alternatives, apply-build actions and Swiftplay loadouts. A manual D/F preference is supported; automatic learning from manual swaps is not included.
7. Shared build selection in the main live view and TAB overlay. Auto adapts to a clear completed-item match in an available path. Manual selection locks the path for that game. Varus has AP, AD on-hit and AD lethality templates, validated against current item metadata. Other champions expose their available provider alternatives. The chosen path updates purchase order, boots, recall advice, component costs and AP/AD defensive and penetration advice. Existing completed purchases stay in the plan. Generic components and common situational items do not force a switch.

## Controls

- Use Text size and Flash key near the top of the app.
- Hold TAB to view the overlay normally.
- Hold Shift + TAB to click build controls, inspect labels or scroll. Releasing Shift restores click-through behavior.
- Open Change build style, select a path to lock it, or select Auto adapt to follow purchases.
- Build selection resets between games. Display and Flash preferences persist.

## Build on your Mac

From the extracted LuvvyLoL folder, with Node, pnpm, Rust and Xcode Command Line Tools installed:

```sh
pnpm install --frozen-lockfile
node scripts/test-update.mjs
pnpm build
pnpm tauri build
```

The packaged app should appear under `src-tauri/target/release/bundle/macos/`; the DMG under `src-tauri/target/release/bundle/dmg/`.

## Verification

Passed:

- TypeScript compilation and Vite production build.
- 20 focused JavaScript checks for rank conversion, daily LP boundaries, history labels, AP/AD selection, locking and component accounting.
- Actual spell-order helper compiled and tested with Rust: all five D/F/no-Flash cases pass.
- Rust syntax parsing of lib.rs and its modules.
- Backend models, LCU and OP.GG modules compiled in an isolated Linux check with the actual config data types. This excludes Tauri window orchestration.
- Git diff whitespace checks and archive integrity checks.
- Varus template IDs checked against Riot Data Dragon 16.18.1.

Not verified here:

- Full macOS Tauri build: cross-check stopped at an Objective-C dependency requiring Apple's build tools. No app or DMG is included.
- Visual screenshot comparison: the browser security policy blocked the local preview. Responsive CSS was reviewed, but no successful rendered visual QA is claimed.
- Live account switching, Swiftplay spell placement, Shift + TAB interactions, monitor scaling and recommendation changes in an actual match require testing on macOS.

Suggested live check: switch accounts without restarting; verify profile and history refresh. Confirm Flash is on F after auto-apply in both draft and Swiftplay. Start a Varus match, buy Nashor's Tooth, verify Auto selects AP, then lock AD and confirm further AP purchases do not override it. Test scrolling the overlay with Shift + TAB and all four text sizes on the second monitor.

## Files

Modified: src/App.tsx, src/App.css, src-tauri/src/lib.rs, src-tauri/src/lcu.rs, src-tauri/src/models.rs, src-tauri/src/config.rs.
Added: src/recommendationLogic.ts, scripts/test-update.mjs, src-tauri/capabilities/overlay.json, UPDATE-NOTES.md, POROFESSOR-COMPARISON.md.

No original repository files were removed. Unsupported LP samples are excluded from display calculations; the periodic tracker retains a rolling 32 days. The ZIP contains the full tracked source tree with these changes, excluding installed dependencies, Git history, build products and temporary verification files. Nothing was pushed to GitHub.
