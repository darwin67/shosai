# Flutter frontend architecture

The Flutter frontend uses an Elm-style model/message/update/effect boundary.
This keeps asynchronous native resources and stale completions from becoming
implicit widget state.

```diagram
┌────────┐   ReaderMessage   ┌──────────────────┐
│ Widget │──────────────────▶│ ReaderController │
└───▲────┘                   │ update/dispatch  │
    │ immutable ReaderModel  └───────┬──────────┘
    │                                │ owned effect
    └────────────────────────────────┤
                                     ▼
                              ┌─────────────┐
                              │ Rust bridge │
                              └──────┬──────┘
                                     │ typed completion message
                                     └──────────────────────────▶ update
```

## Rules

1. `ReaderModel` is immutable and is the only reader state rendered by widgets.
2. Widgets dispatch sealed `ReaderMessage` values. They do not mutate model or
   native-resource state.
3. Message handling owns state transitions. An asynchronous effect captures the
   document generation and operation revision that authorized it, then reports
   success or failure with a typed completion message.
4. Completion handling rejects stale generations and revisions after every
   asynchronous boundary. Older work must not clear, replace, or report errors
   into newer state.
5. The controller owns cancellation tokens, document and buffer handles, decoded
   images, and effect draining. Disposal cancels work and releases each resource
   exactly once after outstanding effects finish.
6. Rust owns document parsing, text shaping, paint geometry, durable anchors,
   persistence, cancellation, and memory admission. Flutter owns gestures,
   overlays, focus, navigation, dialogs, and responsive composition.
7. Renderer geometry and renderer pixels are one contract. Flutter must not
   independently reshape EPUB text whose hit zones were produced by Rust.
8. Dialogs, pickers, and similar platform effects are injected controller
   adapters. Widgets dispatch an intent; only the controller starts and awaits
   the adapter, and its result returns as a revision-guarded message.
9. Every Rust DTO carrying a retained handle has an explicit owner. Effects
   release unadopted handles on failure or staleness; adopted handles remain
   model-owned until replacement or disposal.

## Testing

Use completer-controlled effects to test stale completion, replacement, and
disposal ordering. Widget tests must exercise gestures or shortcuts through the
rendered surface when validating interaction contracts; dispatching offsets
directly only tests update logic. Native bridge tests cover owned DTO transfer
and create/reopen/update/delete flows for each supported format.
