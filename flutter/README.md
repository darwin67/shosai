# Shōsai Flutter frontend

This is the Flutter feasibility frontend from RFD 0004 M2. The Linux and macOS
hosts load the generated `flutter_rust_bridge` bindings, open PDF, EPUB, and
CBZ documents through `shosai-core`, and render selectable PDF and EPUB
content. Rust extracts bounded, owned hit-test geometry once per visible
surface; pointer and touch drags consume it locally while Flutter paints
temporary and saved overlays without rerendering the document.

Reader state follows the same Elm-style model/message/update flow as the Iced
frontend. Widgets render the immutable `ReaderModel` and dispatch sealed
`ReaderMessage` values; `ReaderController` runs bridge effects and dispatches
their completion messages while retaining ownership of native resources.
The current slice persists PDF and EPUB highlights through the Rust SQLite
annotation store and supports reopen, navigation, recoloring, notes, and
deletion. Anchor recovery, responsive mobile hosts, and physical-device
validation remain open M2 gates.

Run commands from the repository root through the pinned development
environment:

```sh
.agents/dev make check-flutter
.agents/dev make flutter-dev
.agents/dev make flutter-macos-smoke
.agents/dev make flutter-release
```

The desktop hosts build `crates/shosai-flutter-bridge` directly with the pinned
Cargo toolchain through CMake. Linux bundles are written below
`flutter/build/linux/`, and macOS bundles below `flutter/build/macos/`. macOS
feasibility builds intentionally target the active host architecture; universal
distribution packaging is outside M2.
