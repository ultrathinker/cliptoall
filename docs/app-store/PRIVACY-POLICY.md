# Privacy Policy — ClipToAll

_Last updated: 16 August 2026_

Publish this at a stable public URL and paste that URL into App Store Connect.
A privacy policy URL is required for every app, including apps that collect
nothing. Check the claims below against the code before publishing — they are
written to be true of the current build, and they should stay true.

---

## The short version

ClipToAll does not collect anything. There is no account to create, no server of
ours for your screenshots to pass through, and no analytics in the app. Your
screenshots go to storage **you** control, using credentials **you** provide.

## What the app stores, and where

Everything is stored locally on your Mac, inside the app's own sandbox
container. Nothing is sent to the developer.

| What | Where | Why |
|---|---|---|
| Your settings (hotkey, theme, output options) | a settings file in the app's container | to remember your preferences |
| Google Drive authorization token | the app's container, obtained through Google's own sign-in flow | so uploads can reach your Drive without asking you to sign in each time |
| Amazon S3 access key and secret, if you use S3 | encrypted, in the app's container | so uploads can reach your bucket |
| Screenshots you take | a temporary folder inside the container, until you close the results window | to let you edit, copy or upload the capture |
| An optional log file | the app's container, only while you turn logging on | troubleshooting |

You can remove all of it by deleting the app and its container.

## What leaves your Mac

Only what you ask to leave, and only to services you chose:

- **Your screenshot**, when you upload it — to your Google Drive or to your
  Amazon S3 bucket. The developer has no access to either.
- **Sign-in with Google**, when you connect Drive — handled by Google's own
  authorization page in your browser. ClipToAll never sees your password.
- **A reverse-image search**, only if you press "Google" or "Tineye" in the
  results window — this opens your browser at that service with the link to your
  uploaded image. Those services have their own privacy policies.

Nothing else. There is no telemetry, no crash reporting service, no advertising
identifier, no third-party analytics SDK.

## Text recognition

"Copy text" reads the text in your capture using Apple's Vision framework, which
runs entirely on your Mac. The image is not uploaded for recognition, and no
recognition service is contacted — this works with the network switched off.

## Permissions the app asks for

- **Screen Recording** — required by macOS for any app that captures the screen.
  ClipToAll asks the first time you take a capture, not at launch. Without it,
  macOS returns a blank image and the app cannot function.
- **Open at Login** — only if you switch it on in Settings. Off by default.

## Children

ClipToAll is a general-purpose utility, is not directed at children, and
collects no data from anyone.

## Changes

If this policy changes, the date at the top changes with it. Material changes
will be noted in the app's release notes.

## Contact

Yevhen Borzenkov — universeissilent42@gmail.com
