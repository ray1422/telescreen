# Fallback App Intents metadata

`Metadata.appintents` is a build artifact. Its source is `../Control.swift`.

- `scripts/make-macos-control.sh` makes it with the Xcode tool `appintentsmetadataprocessor`.
- If Xcode is not available, the script copies this hand-written copy and prints a warning. CI sets `TELESCREEN_REQUIRE_XCODE=1`, so CI never uses this copy.
- The format of the file is not public. This copy was written by hand from the format of an installed widget (docs/spikes.md, spike 8).
- Change this copy together with `OpenMenuIntent` in `../Control.swift`: the identifier, the mangled type name (`17TelescreenControl14OpenMenuIntentV`), and the title.
