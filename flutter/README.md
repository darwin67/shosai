# Shōsai Flutter frontend

This is the Flutter feasibility frontend from RFD 0004 M2. The Linux and macOS
hosts load the generated `flutter_rust_bridge` bindings, open PDF, EPUB, and
CBZ documents through `shosai-core`, and render the first PDF or CBZ page.
EPUB scene transfer and highlighting are subsequent M2 work.

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
