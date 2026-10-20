# Change: Interactive browser sources

## Why
A browser source shows a web page or a local HTML file, but nothing can be clicked or typed in
it. Pages built to be used (a music player with a genre picker, a timer with buttons, a poll
widget) are stuck on their first screen. A tester reported exactly this with a local music page.
The browser helper already takes mouse, wheel and keyboard input for the Chat panel's pages; it
just isn't connected to browser sources.

## What Changes
- **Interactive** switch in each browser source's settings. Off by default.
- While it's on, the pointer over that source in the preview goes to the page: moving, clicking
  and the wheel, mapped from canvas pixels to page pixels through the item's transform and crop.
  Typing goes to the page while it has focus (after a click on it); **Esc** gives the keyboard
  back to the studio.
- **Hold Alt** to edit an interactive source like any other (select, move, resize, Alt-drag to
  crop). Without Alt, clicks on it never start a drag.
- The cursor shows a pointer over interactive sources, so it's clear why clicks don't move it.
- Works the same for web addresses and local files, in the Stage and in studio mode's Preview.

## Impact
- Affected specs: browser-source (new requirement).
- Affected code: `crates/estream-capture/src/media/browser.rs` (the `interactive` setting;
  input for a source's page), `apps/estream/src/app.rs` and `interact.rs` (routing preview
  pointer and keys), `apps/estream/ui` (wheel over the preview).
