# Mac App Store submission — checklist

Every field App Store Connect asks for, with our answer or where to find it.
Written 2026-08-16 against Apple's current requirements. Sources at the bottom.

Anything marked **TODO** needs a decision or an action from the owner; everything
else is already prepared in this folder.

---

## 1. Before you can submit anything

| Item | State |
|---|---|
| Apple Developer Program membership | have it |
| Bundle identifier `net.appshub.cliptoall` | set in `tauri.conf.json` |
| App record created in App Store Connect | **TODO** |
| Apple Distribution certificate + Mac App Store provisioning profile | **TODO** — the Developer ID cert we sign with today is for direct distribution, *not* for the store; the store build needs a different identity |
| Build uploaded via Transporter or `xcrun altool` | **TODO** |

Note on signing: the `.app` we build today is signed `Developer ID Application`
and is correct for GitHub distribution. The store submission needs the same
code re-signed with `Apple Distribution` plus a provisioning profile embedded.
Nothing in the source changes — this is a signing step, not a code change.

## 2. Product page

| Field | Our answer |
|---|---|
| App name | ClipToAll |
| Subtitle | see `LISTING.md` |
| Description | see `LISTING.md` |
| Keywords | see `LISTING.md` |
| Promotional text | see `LISTING.md` |
| Screenshots | `screenshots/` — 3 ready, 2560×1600, see that folder's README |
| App preview video | optional, skipped |
| Category | Productivity (`public.app-category.productivity`, already in `tauri.conf.json`) |
| Age rating | 4+ — no objectionable content of any kind |
| Support URL | **TODO** — a page on the owner's site |
| Marketing URL | optional |
| Privacy policy URL | **TODO** — publish `PRIVACY-POLICY.md` and paste the URL |
| Copyright | © 2026 Yevhen Borzenkov |

**Screenshots are mandatory and must be exactly** 1280×800, 1440×900,
2560×1600 or 2880×1800, 16:10, PNG or JPEG, RGB, flattened, no alpha. Ours are
2560×1600 with no alpha. Only the first two or three are visible without
scrolling, so keep the current order.

## 3. App Privacy (the "nutrition label")

Required before submission, even for an app that collects nothing.

**Our answer to the first question is "No, we do not collect data from this
app."** That is unusually clean for this category and is worth leaning on in the
listing. It is also true — verify it yourself before submitting, from the code:

- no analytics or telemetry SDK is linked (check `Cargo.toml` and `package.json`);
- the only outbound hosts are Google's OAuth/Drive APIs and the S3 endpoint the
  user configured — both are *the user's own* accounts;
- OCR runs on-device through Vision; nothing leaves the machine;
- the Google token and the S3 keys are stored locally (Keychain / the app
  container), never transmitted anywhere but to Google/AWS themselves.

One honest caveat to keep in mind if a reviewer asks: the "Google" and "Tineye"
buttons open a reverse-image search **in the user's browser** with the image URL
in the query string. That is a user-initiated navigation, not data collection by
us, and it only happens when the button is pressed.

## 4. App Review Information

| Field | Our answer |
|---|---|
| Sign-in required? | Yes — but to the *user's own* Google account |
| Demo account | Not applicable, and say so: there is no account on our side to demo. See `REVIEW-NOTES.md` for how the reviewer can exercise the app without connecting Drive at all |
| Contact name / phone / email | **TODO** |
| Notes | `REVIEW-NOTES.md` — paste it in |
| Attachment | a short screen recording is strongly recommended, see `REVIEW-NOTES.md` §4 |

This is the field most likely to decide the outcome. The app has no Dock icon,
lives in the menu bar and is driven by a global shortcut — a reviewer who does
not read the notes may conclude it does not launch. Do not leave it blank.

## 5. Technical checks before uploading

- [ ] `LSMinimumSystemVersion` is 14.0 — already set
- [ ] Sandbox entitlement present in the signature:
      `codesign -d --entitlements :- ClipToAll.app` must list
      `com.apple.security.app-sandbox`, `network.client`, `network.server`,
      `files.user-selected.read-write` (verified on the current build)
- [ ] No private API: `macos-private-api` was removed — confirm it has not
      returned with `grep -r macos-private-api src-tauri/`
- [ ] Plugins compiled out on macOS: `grep -rn "mod plugins" src-tauri/src/main.rs`
      must show the `#[cfg(windows)]` gate
- [ ] Launch the built `.app` and exercise: capture, upload, editor, Copy text,
      Open at Login. Every automated gate was green today at a moment when the
      app did not start at all — build and run it before you submit
- [ ] Screen Recording prompt appears and its "Open Settings" button works
- [ ] Icon renders correctly at every size in Finder and the Dock

## 6. Known risks

**Guideline 4.3 — saturated category.** Screenshot utilities are crowded. Our
answer is on-device OCR plus "your storage, not ours"; both are in the listing
copy and the review notes. This is the most likely reason for a rejection.

**The icon.** The mark is a red cross on a white circle. The Red Cross emblem is
protected by the Geneva Conventions and by national law in many countries, and
Apple has rejected apps over it before. The owner has considered this and
decided to submit as-is; if the rejection comes, it will name the reason and the
fix is a colour change.

**Screen Recording.** Requested at first capture, not at launch, and explained
in the notes. If the reviewer denies it, the app shows a dialog with a direct
link to the right System Settings pane rather than failing silently.

---

Sources:
[Submitting to the App Store](https://developer.apple.com/app-store/submitting/),
[App Privacy Details](https://developer.apple.com/app-store/app-privacy-details/),
[App Review](https://developer.apple.com/distribute/app-review/),
[App Review Guidelines](https://developer.apple.com/app-store/review/guidelines/),
[Mac screenshot sizes 2026](https://www.lazyscreenshots.com/blog/app-store-screenshots-mac/)
