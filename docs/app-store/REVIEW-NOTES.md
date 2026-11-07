# App Review notes

Paste §1–§3 into the **Notes** field of App Review Information in App Store
Connect. §4 is a plan for the attached video, not part of the notes text.

This field matters more for this app than for most. ClipToAll has **no Dock
icon** and **no main window** — it is a menu bar utility driven by a keyboard
shortcut. A reviewer who launches it and sees nothing may reasonably conclude it
does not work. Say so first, in the first line.

---

## 1. Notes text — paste this

```
HOW TO LAUNCH AND TEST

ClipToAll is a menu bar utility. It intentionally has no Dock icon and no main
window — after launching, look for its icon in the menu bar at the top right of
the screen. This is by design for a capture tool that must stay out of the way.

To take a capture:
  1. Press Control-Command-X (or click the menu bar icon).
  2. The screen dims. Drag a rectangle over any area.
  3. Release. The image is copied to your clipboard and a results window opens.

macOS will ask for Screen Recording permission the first time you capture. This
permission is required by macOS for any app that reads screen content; it is
the app's core function and it cannot work without it. Grant it, then quit and
reopen the app — macOS only applies a new Screen Recording grant on relaunch.
If permission is missing, the app explains this and offers a button that opens
the correct System Settings pane.

NO ACCOUNT IS NEEDED TO REVIEW THE APP

There is no ClipToAll account and no server of ours, so there is no demo login
to provide. The capture, the editor and the text recognition all work fully
without signing in to anything:

  • Capture and "Copy image" — no sign-in required.
  • "Edit" — annotate with arrows, boxes, blur and text; no sign-in required.
  • "Copy text" — on-device text recognition via Apple's Vision framework;
    no sign-in and no network required. This is the quickest way to see the
    feature that distinguishes the app: capture any text on screen, press
    Copy text, and paste.

ABOUT THE OPTIONAL GOOGLE DRIVE SIGN-IN

The "Upload" button uploads to the reviewer's OWN cloud storage — a Google
Drive account or an Amazon S3 bucket that the user connects in Settings. We do
not operate any storage; the app has no backend. Signing in with Google is
therefore optional and, if performed, authorizes access to the reviewer's own
Drive only. If you prefer not to sign in, everything except Upload can still be
exercised.

The sign-in uses the standard OAuth loopback flow: the app opens Google's own
consent page in the default browser and listens on 127.0.0.1 for the redirect.
That is why the sandbox entitlement com.apple.security.network.server is
present.

PRIVACY

The app collects nothing. There is no analytics or telemetry SDK linked into
the binary. Credentials and settings are stored locally in the app's sandbox
container. Text recognition is entirely on-device.
```

## 2. If the reviewer asks about the entitlements

- `com.apple.security.network.client` — uploads to Google Drive / Amazon S3.
- `com.apple.security.network.server` — the OAuth redirect listener on
  `127.0.0.1`, used only during sign-in. Without it, connecting Drive is
  impossible.
- `com.apple.security.files.user-selected.read-write` — the "Save as file"
  native save panel.

## 3. Contact

Yevhen Borzenkov — universeissilent42@gmail.com

---

## 4. The demo video — shot list

Not required, but for an app whose main surface is invisible until a shortcut is
pressed it removes the most likely misunderstanding. 40–60 seconds, no
narration needed, no music. Record with QuickTime (File → New Screen Recording)
at the display's native resolution.

1. **0:00–0:05** — Show the menu bar icon. Point out there is no Dock icon.
2. **0:05–0:15** — Press Control-Command-X. Screen dims, drag a rectangle over a
   block of text, release. Results window appears.
3. **0:15–0:25** — Press "Copy text". Switch to TextEdit and paste. The
   recognised text appears. This is the beat that matters most — it is the
   feature the listing leads with.
4. **0:25–0:40** — Press "Edit". Draw an arrow and a box, blur something. Press
   Save.
5. **0:40–0:55** — Open Settings → Storage and show that the destination is the
   user's own Google Drive or S3 bucket, entered by the user. Do not sign in on
   camera.

Keep the desktop clean: no personal files, no unrelated notifications, no other
companies' branding on screen.
