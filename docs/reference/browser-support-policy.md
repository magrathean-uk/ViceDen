# Browser support policy

The intended test set is Safari, Chrome, Firefox, Edge, and Tor Browser, with Safari a priority. This is a verification target, not a claim that every release has passed in every browser. Record actual browser and OS versions with test evidence.

Use feature detection. Do not block core content by user-agent string. Any compatibility warning should be dismissible.

Keep system fonts and local assets, preserve keyboard and screen-reader navigation, and test forms with restrictive browser settings. Passkeys are not part of the current implementation. Browser derivatives are best effort until exercised explicitly.

Use the [responsive smoke checklist](../development/responsive-smoke.md) after UI changes.
