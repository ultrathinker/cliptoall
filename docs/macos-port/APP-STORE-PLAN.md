# Mac App Store edition — plan

Goal: publish a sandboxed Mac App Store edition of ClipToAll, built from **this**
repository (no fork), alongside the existing Developer-ID build distributed via
GitHub which keeps the plugin system.

Written 2026-08-16, after three independent analyses (`Sonnet`, `MiniMax`,
`GLM-5.3`) whose full reports are transient; their surviving conclusions are
folded in here. Where a claim below was verified against the code, it says so.

Read `HANDOFF.md` first for how the port works and `WORKLOG-2026-08-16.md` for
what was done most recently.

---

## 0. The decisions this plan assumes

- **One repository, one codebase.** The store edition is a build configuration,
  not a branch or a fork. Measured justification: the Rust backend is 8331 lines
  with 56 `cfg(windows)` occurrences, while the **entire 4998-line frontend has
  zero platform-specific code**. Forking would duplicate all of it to remove
  conditionals that never reach the macOS binary anyway.
- **Apple never sees the source.** Review inspects the built `.app`: private-API
  symbols, entitlements, executable paths, runtime behaviour. `#[cfg(windows)]`
  code is removed at compile time and is not in the artifact. The real "looks
  like a port" risk is the **user-visible surface**, which §2 addresses.
- **Windows keeps plugins.** The store edition compiles them out (guideline
  2.5.2 + the sandbox forbids spawning interpreters).
- **The app name stays the same for now.** A distinct bundle identifier is still
  required so the two editions do not collide over container, TCC grants and
  login items.
- **iCloud Drive is not a destination.** All three analysts independently called
  it a trap: Drive and S3 expose a one-call "make public, return URL" that this
  code already uses; iCloud Drive has no equivalent without CloudKit/Swift
  bridging that does not exist in this Rust/Tauri codebase. Skipped.

---

## 1. Phase 0 — safety net, before touching anything

We are about to make sweeping changes to shared code. Windows CI has **never**
run against this branch. Doing the cleanup blind would mean discovering Windows
breakage weeks later, with no idea which change caused it.

1. **Commit the current work.** 17 files are uncommitted (the ScreenCaptureKit
   migration, the overlay compositor fix, the output-mode default, the GDrive
   content gating, the clickable URL field).
2. **Widen the CI trigger** in `.github/workflows/build.yml` — it currently fires
   only on push/PR to `main`. Add this branch so every push is validated.
3. **Enable the macOS matrix entry.** It is already written as a comment
   (`{ os: macos-latest, target: universal-apple-darwin }`); uncomment it. Gate
   the plugin-build steps on `matrix.os == 'windows-latest'`, since those crates
   are Windows-only.
4. **Push and confirm both jobs are green** before starting §2.

Runtime verification on Windows is a separate matter — CI compiles and runs unit
tests, it never launches the app. Use the Windows machine for that at milestones
(before merging to `main`, before a release), not per push.

---

## 2. Phase 1 — remove the private-API blocker (do this first)

`src-tauri/Cargo.toml` enables Tauri's **`macos-private-api`** feature (verified,
line 16) so the overlay window can be transparent. Apple's binary scan rejects
private API usage outright. This gates everything else — if it cannot be
resolved, the store plan changes shape.

The likely cheap resolution: **the overlay probably does not need transparency
at all.** It draws the dimmed screenshot onto a canvas covering the full screen,
so its content is already opaque edge to edge. The flag may be inherited from an
earlier design.

Spike, roughly half a day:
- build with the feature removed and the overlay window opaque;
- confirm the overlay looks and behaves identically (dimming, selection
  rectangle, plugin hotkeys, cancel paths);
- if something genuinely requires a transparent NSWindow, note that the `objc2`
  stack is already a dependency and setting a clear window background directly
  is available without Tauri's private-API feature.

Only after this is settled does the rest of the work have a point.

---

## 3. Phase 2 — de-Windows the visible surface

This is what the brief called "хорошенько почистить", and it is also the
concrete answer to guideline 4.3: rejections in saturated categories are driven
by an overall impression of low effort, not by a missing marquee feature.

**Help texts** — `src/lib/help-texts.ts` contains 14 Windows-specific strings
(verified). Examples: "log in to Windows", "system tray (bottom-right corner of
the taskbar)", "registers itself in the Windows Registry under HKCU\\...",
"Press Alt+X", "next to the executable", "Windows Photos". Rewrite for macOS
vocabulary: menu bar, Login Items, Application Support, Cmd-based hotkey.

**About window** — same sweep.

**Settings labels** — "Add to Autorun" reads as Windows vocabulary; "Open at
Login" is what a Mac user expects.

**Settings that do nothing on macOS** — audit `src/windows/Settings.svelte` and
its sub-components; hide anything whose backend is Windows-only rather than
leaving a control that silently does nothing.

**Storage tab** — consider putting S3 behind a "show advanced destinations"
disclosure with Drive as the default. "Access Key ID" and "Bucket" are developer
vocabulary that a store audience bounces off immediately.

**Default theme** — currently `crimson` (verified, `settings.rs:89`), a dark red
that reads as "Windows port". A `mac` theme already exists in the allowed list;
consider defaulting to it and following system appearance.

---

## 4. Phase 3 — fix what reads as unfinished

Each of these is a concrete defect found by the analysts and confirmed in the
code. Any one of them could draw a 2.1 ("app does not function as expected")
rejection on its own.

**`Cmd+X` collides with the system Cut shortcut.** ⚠️ The macOS default is
`Cmd+X` (verified, `settings.rs:67`). A registered global hotkey takes the
combination away from every application, and Cmd+X is Cut everywhere on macOS.
**Verify first** (select text anywhere, press Cmd+X, see whether it cuts), then
choose a safer default and migrate existing settings files. This default was set
deliberately earlier in the port; the collision was not considered at the time.

**"Save as file" errors out.** `save_image_to_file` is a stub on non-Windows
returning `Err("save_image_to_file not yet implemented on macOS")` (verified,
`commands/capture.rs:563`), and `Editor.svelte:425` calls it unconditionally,
surfacing that raw string in a modal. Either implement it via a native save
panel or hide the button — shipping it as-is is indefensible.

**No screen-recording permission preflight.** There is no
`CGPreflightScreenCaptureAccess` anywhere in the codebase (verified). If the user
denies Screen Recording, capture silently produces nothing and the app looks
broken. Add a preflight plus a clear dialog that deep-links to the right
System Settings pane.

**Autostart defaults to on.** `autorun: true` (verified, `settings.rs:146`) means
the app adds itself to login items without being asked. Default it off and
register only on an explicit user toggle.

**Media-key hotkeys.** Note for the store build: `global-hotkey-0.7.0` uses
`RegisterEventHotKey` for ordinary keys, which needs **no** Accessibility
permission — verified by reading the crate. It only creates a `CGEventTap`
(which does need Accessibility) inside `start_watching_media_keys()`, reached
only when `is_media_key(hotkey.key)`. So do **not** build an Accessibility
onboarding flow; instead prevent media keys from being chosen as the hotkey in
the sandboxed build, or warn clearly.

---

## 5. Phase 4 — the store build configuration

**Cargo feature for plugins**, default on, off for the store build. Gate the
Rust side (`plugins.rs`, `commands/plugins.rs`, the command registrations in
`main.rs`, the overlay's plugin-hotkey dispatch) and the UI side (the Plugins
settings tab, and the plugin references in the user guide text). Estimates
ranged from one day (Sonnet) to three (GLM); budget two.

**Verify the artifact, not the intent.** After building the store variant,
inspect the binary for process-spawning symbols. Guideline 2.5.2 is enforced by
inspecting what shipped.

**Sandbox entitlements.** `app-sandbox`, `network.client`, and — the subtle one,
found only by GLM and verified — **`network.server`**, because the Google OAuth
flow binds a loopback listener (`TcpListener::bind("127.0.0.1:0")`,
`upload_gdrive.rs:336`). Without it, connecting Drive fails entirely in the
store edition. Also `files.user-selected.read-write` for the save panel.

**Autostart via `SMAppService`.** The current implementation hand-writes a
LaunchAgent plist into `~/Library/LaunchAgents`, which a sandboxed app cannot do.

**Distinct bundle identifier** for the store edition, plus its own icon and
bundle configuration. `minimumSystemVersion: 14.0` to match the floor.

**Consider an accessory activation policy** (no Dock icon, promote only while a
window is visible). It is a small change that makes the app read as a native
menu-bar utility rather than a ported desktop application.

---

## 6. Phase 5 — differentiation

Only after the above. A polished app without a marquee feature passes review
more readily than an app with OCR and a broken Save button.

**On-device OCR** — proposed independently by MiniMax and GLM, and the strongest
answer to "why admit this over the incumbents". Vision framework
(`VNRecognizeTextRequest`) ships with macOS, runs entirely on-device, works
under the sandbox, needs no entitlement and no network. The capture is already
in memory, so it costs no extra I/O. Surface it as "Copy text" in the results
window. Estimates 1–4 days.

**Multi-monitor capture** — currently primary-display only. On a Mac with an
external display this is not a feature, it is an expectation. `sck_capture.rs`
already resolves displays individually; the work is composition and a
multi-window overlay. Estimates 2–5 days.

**Name what already exists but is never presented:** crisp Retina links (the
`exif` output mode fixed today — most competitors get this wrong), the instant
link from the placeholder pool, bring-your-own S3, the double-press mode toggle,
reverse-image search built in.

---

## 7. Phase 6 — submission materials

Not code, and routinely underestimated:

- privacy policy at a public URL (the owner has a website);
- App Privacy questionnaire — this app's honest answer is unusually good: images
  go to storage the user owns, the developer collects nothing;
- listing copy built around "your screenshots go to storage you own, not to our
  servers", which is genuinely uncommon in this category and is the positioning
  all three analysts converged on;
- screenshots, support URL;
- **review notes and a demo video** — a hotkey-driven capture tool is awkward for
  a reviewer to exercise. Explain that the Google sign-in reaches the user's own
  Drive, and that Screen Recording is requested at launch.

---

## 8. Sequencing summary

| Phase | What | Rough cost | Gates what |
|---|---|---|---|
| 0 | Commit, widen CI, enable macOS matrix, confirm green | hours | everything after it |
| 1 | Remove `macos-private-api` (opaque overlay spike) | 0.5 d | the whole store plan |
| 2 | De-Windows the visible surface | 1 d | 4.3 impression |
| 3 | Fix unfinished signals (Cmd+X, save, TCC, autorun) | 1–1.5 d | 2.1 rejection |
| 4 | Store build config: plugins out, entitlements, SMAppService, bundle id | 2–3 d | 2.5.2 + sandbox |
| 5 | OCR, then multi-monitor | 3–9 d | 4.3 differentiation |
| 6 | Submission materials | 1–2 d | submission |

Smallest thing worth submitting: phases 0–4 plus OCR. Phases 0–4 alone would
probably pass technically but leaves the 4.3 argument thin.

---

## 9. Open question to settle before Phase 3

Does `Cmd+X` actually break Cut system-wide while the app runs? Five seconds to
test, and the answer decides whether the hotkey default change is urgent or
merely advisable.
