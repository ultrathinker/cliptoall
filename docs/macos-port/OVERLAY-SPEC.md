# Overlay Visual & Behavioral Spec (extracted from `overlay.rs`)

> Source of truth for the Phase 3 web overlay's pixel/behavior parity with the
> native Win32 overlay. Extracted 2026-08-08 from `src-tauri/src/overlay.rs`
> (583 lines) — every number here is read directly from that file, not
> approximated. Re-check against `overlay.rs` if it changes before Phase 3
> starts (PLAN.md's "Windows is frozen" principle means it shouldn't, but
> verify).

## 1. Screen dim + mode tint

The whole captured screenshot is shown dimmed to ~58–62% brightness, with a
**very subtle** (~3%) color tint that differs by mode. Pixels are BGRA;
factors below are applied per-channel as `channel * factor / 255`:

| Mode | B factor | G factor | R factor | Effect |
|---|---|---|---|---|
| Link (pink, default) | 150/255 (0.588) | 148/255 (0.580) | 158/255 (0.620) | R relatively boosted → faint pink/magenta |
| Image (green) | 150/255 (0.588) | 158/255 (0.620) | 148/255 (0.580) | G relatively boosted → faint green |

Both dimmed bitmaps are precomputed once when the overlay opens (not
recomputed per frame) — cheap to do the same in JS/canvas with two prebuilt
`ImageData`/canvas layers or a single shader-like pass, swapped by mode.

**Which one shows:** driven by `copy_image_flag` (an atomic bool shared with
the hotkey handler) — link mode by default, image mode after a double-press
of the capture hotkey while the overlay is open (see §4).

## 2. Selection rectangle (while dragging)

- The dimmed background stays dimmed EVERYWHERE except inside the selection
  rect, where the ORIGINAL (bright, undimmed) pixels are shown — i.e. the
  selection is a "window" back to the real screenshot, not a drawn overlay.
- **Border**: 2px solid line, `Rectangle()` (no fill — `NULL_BRUSH`).
  - Link mode: crimson `rgb(200, 50, 90)`
  - Image mode: green `rgb(50, 200, 90)`
  - (Note: this is a DIFFERENT, more saturated color than the background
    tint in §1 — the tint is a faint ambient hint, the border is the vivid
    active-selection color.)
- Minimum size to render/commit a selection: **5×5 px** in either dimension.
  Below that, treated exactly like a cancel (no distinct message).

## 3. Size label

- Text: `"{width} × {height}"` — literal multiplication sign (U+00D7, `×`),
  not the letter `x`.
- Color: same as the border color for the current mode (crimson or green).
- Font: `DEFAULT_GUI_FONT` (Windows system UI font — Segoe UI at the system's
  default UI size, typically 9pt / 12px). Web equivalent: the system font
  stack at a comparable size (~13–14px to visually match Segoe UI 9pt).
- Background: transparent (`SetBkMode(TRANSPARENT)`) — no background box or
  shadow behind the text in the native version.
- Position: horizontally centered over the selection
  (`label_x = sel_x + sel_w/2 - text_width/2`); vertically **above** the
  selection by 20px normally (`sel_y - 20`), but if the selection is within
  25px of the top edge (`sel_y <= 25`), the label instead sits 5px **below**
  the selection (`sel_y + sel_h + 5`) so it's never clipped off-screen.

## 4. Modes and how they switch

- Default mode on overlay open: whatever `copy_image_flag` was set to before
  `show_native_overlay` was called (link, i.e. "false", unless the user's
  configured default mode is image).
- **Double-press of the capture hotkey while the overlay is already open**
  toggles `copy_image_flag` and calls `invalidate_overlay()` — this forces an
  immediate repaint so BOTH the background tint (§1) and the selection
  border/label color (§2–3) switch to the new mode's colors without waiting
  for a mouse move. This logic lives in `main.rs`'s hotkey handler, not in
  `overlay.rs` itself — the web overlay's equivalent needs a way for the
  Rust side to push a "mode changed, repaint" event into the WebviewWindow.

## 5. Shift = square constraint

Pure, unit-tested function (`constrain_square`, still in `overlay.rs`,
directly portable to TS): given the drag anchor `(start_x, start_y)`, the
current cursor `(cur_x, cur_y)`, and the drag-direction signs `sx = sign(dx)`,
`sy = sign(dy)` (a **zero delta counts as positive**, i.e. the square grows
right/down by default on a perfectly horizontal/vertical drag):

```
room_x = sx < 0 ? start_x : (max_w - 1 - start_x)   // clamped to >= 0
room_y = sy < 0 ? start_y : (max_h - 1 - start_y)   // clamped to >= 0
side   = max(|dx|, |dy|) clamped to [0, min(room_x, room_y)]
result = (start_x + sx*side, start_y + sy*side)
```

The side is clamped (not the point) so the result is ALWAYS a perfect square,
even when the drag would run off the screen edge — it just stops growing on
whichever axis has less room, and the OTHER axis is truncated to match (still
square), rather than the shape becoming a non-square rectangle. Existing test
cases to reproduce as-is (`overlay.rs` lines 553–582):

| start | drag to | max | expected |
|---|---|---|---|
| (10,10) | (40,25) | (1000,1000) | (40,40) — dx dominant, extends down-right |
| (100,100) | (60,20) | (1000,1000) | (20,20) — dy dominant, extends up-left |
| (100,100) | (160,40) | (1000,1000) | (160,40) — up-right |
| (50,50) | (50,90) | (1000,1000) | (90,90) — dx==0 → grows right |
| (50,50) | (90,50) | (1000,1000) | (90,90) — dy==0 → grows down |
| (990,10) | (995,900) | (1000,1000) | (999,19) — clamped by right edge, stays square |
| (500,8) | (400,0) | (1000,1000) | (492,0) — clamped by top edge, stays square |
| (42,42) | (42,42) | (1000,1000) | (42,42) — degenerate, zero-sized |

- Shift state is read at the moment of paint/mouse-up (`GetKeyState`, the
  message-synchronized snapshot), NOT a live/async poll — the web overlay
  should track Shift via keydown/keyup event state, not `event.getModifierState`
  polled independently of the triggering event, to avoid the same class of
  race the native code deliberately avoids.
- **Live toggle while stationary**: pressing/releasing Shift while a drag is
  in progress repaints immediately (even with the mouse not moving) so the
  square constraint visibly snaps on/off the instant the key changes.
  Keyboard autorepeat on the down-event is suppressed (native checks lparam
  bit 30) to avoid a flood of no-op repaints while Shift is held.

## 6. Plugin hotkeys

- While the overlay is open (drawing or not), pressing a single
  alphanumeric key that's in the plugin key map immediately ends the overlay
  and returns a `PluginCall { path, function_id }` result — no selection
  needed. Matching is case-insensitive (native uppercases before building
  the VK map).
- Works whether or not Alt is held (native handles both `WM_KEYDOWN` and
  `WM_SYSKEYDOWN`) — the web overlay just needs a normal `keydown` listener.

## 7. Cancel

- **Esc** at any time → cancel (destroy overlay, resolve to "no selection").
- **Right-click or middle-click** at any time (even before a drag starts) →
  cancel.
- A completed drag smaller than 5×5 px → cancel (§2), not an error.

## 8. Cursor

Crosshair (`IDC_CROSS`) for the entire time the overlay is visible — CSS
equivalent: `cursor: crosshair` on the overlay root.

## 9. Window/rendering characteristics (native-specific, informs the web build)

- Single topmost, borderless popup window sized to the FULL virtual screen
  (`screen_left/top/width/height` — all monitors combined on Windows).
  Phase 3's web overlay instead opens one `WebviewWindow` **per monitor**
  (PLAN.md 3.2) since Tauri doesn't span a single window across displays the
  way Win32 does — the Shift-square and label positioning logic must operate
  in EACH window's own local coordinate space, not a shared virtual-screen
  space.
- Double-buffered: everything paints to an offscreen back buffer, one
  `BitBlt` flips it to screen — prevents flicker. `WM_ERASEBKGND` is
  suppressed for the same reason. Canvas rendering (draw to an offscreen
  canvas / use `requestAnimationFrame`, avoid clearing+redrawing the whole
  frame synchronously with layout) should aim for the same flicker-free feel.
- The screenshot is handed to the overlay as a raw pixel buffer already in
  memory (no round-trip through a file) — PLAN.md 3.2 already calls for
  passing it to the web overlay as an `ArrayBuffer`, not base64, for the same
  reason (speed).

## 10. Not covered here (design during Phase 3, not extracted from Windows)

- Exact multi-monitor window coordination (native has one window across all
  monitors; the web overlay is per-monitor — cross-monitor drag behavior is
  a new design question, not a port).
- Any onboarding/permission-related UI (Screen Recording TCC) — unrelated to
  the overlay itself, covered by PLAN.md Phase 5.1.
