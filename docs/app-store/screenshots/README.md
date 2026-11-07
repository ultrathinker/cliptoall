# Screenshots

Three finished screenshots, 2560×1600, PNG, RGB, no alpha — one of the four
sizes App Store Connect accepts for Mac, in the required 16:10 ratio. Upload in
this order; only the first two or three are visible without scrolling.

| File | Shows | Headline |
|---|---|---|
| `01-capture.png` | the dimming overlay with a live selection rectangle | Drag the part that matters |
| `02-results.png` | the results window, with Copy text beside Google and Tineye | Capture. Copy. Done. |
| `03-editor.png` | the editor with a real arrow and box annotation | Annotate before you share |

## How they were made

Captured from the actual signed, sandboxed build running on this machine — not
mocked, not assembled from a design file. Apple rejects screenshots whose
content is not the shipping app.

The app was driven by synthetic input: the global shortcut was sent with
AppleScript, and the selection drag and button clicks with a small Swift helper
posting `CGEvent`s. The raw full-screen captures were then cropped to the app
window and composited onto a plain background with a headline, by another small
Swift tool using CoreGraphics. Both helpers live in the session scratchpad, not
in the repo — they are throwaway, and rebuilding them is a few minutes' work.

Cropping to the window was not cosmetic: the raw captures contained the
developer's Dock, bookmark bar, browser tabs and menu bar extras. None of that
belongs in a store listing.

The subject in the screenshots is a sample HTML page written for the purpose, so
no third party's content or trademark appears.

## What is still missing

Two more would round the set out to five, which is the usual number:

- **Settings → Storage**, to make "your own storage" concrete rather than a
  claim. Needs the Settings window open with the Storage tab showing — take it
  with the S3 fields blank so no credentials are visible.
- **A result with a real link**, showing the URL in the field. The build used
  for these shots is the sandboxed one, which has its own container and no
  connected Google account, so the results window there can only show the
  clipboard state. Take this one from a build with Drive connected, and blur or
  crop the folder name if it reveals anything private.

Both need a signed-in account, which is why they are not here.
