# Responsive smoke checklist

After a layout change, exercise the running application in the [browser test set](../reference/browser-support-policy.md). Source assertions alone do not establish browser acceptance.

## Viewports

Check 360 × 640, 480 × 800, 720 × 1024, 1024 × 768, 1280 × 800, and 1440 × 900. Include a real mobile browser where available.

## Flows

- [ ] Home, board, thread, profile, login, and invite registration have no unwanted horizontal scroll.
- [ ] Mobile navigation opens, closes from the scrim, and closes with Escape.
- [ ] The inspector hides at narrow widths without hiding essential actions.
- [ ] Notifications, saved threads, and hidden threads remain reachable.
- [ ] Thread and reply forms remain usable with the software keyboard open.
- [ ] Focus is visible, navigation works by keyboard, and controls have accessible names.
- [ ] Dark theme, compact actions, local assets, and reduced-motion behavior remain intact.
- [ ] Invalid input and authentication errors are understandable at each width.

## Evidence

Record the source revision, browser/OS, viewport, flows exercised, and failures. Keep screenshots free of credentials and private forum content. Attach them to the relevant review rather than treating an untested checklist as a passing result.
