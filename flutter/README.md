# Shōsai Flutter frontend

This is the Flutter feasibility frontend from RFD 0004 M2. The current Linux
slice loads the generated `flutter_rust_bridge` bindings, opens PDF, EPUB, and
CBZ documents through `shosai-core`, and renders the first PDF or CBZ page.
EPUB scene transfer and highlighting are subsequent M2 work.

Run commands from the repository root through the pinned development
environment:

```sh
.agents/dev make check-flutter
.agents/dev make flutter-dev
.agents/dev make flutter-release
```

The Linux plugin builds `crates/shosai-flutter-bridge` directly with the pinned
Cargo toolchain. Debug and release bundles are written below
`flutter/build/linux/x64/`.
