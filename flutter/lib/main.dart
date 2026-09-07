import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show ExternalLibrary;
import 'package:shosai_flutter/reader_controller.dart';
import 'package:shosai_flutter/src/rust/api.dart';
import 'package:shosai_flutter/src/rust/frb_generated.dart';

export 'package:shosai_flutter/reader_controller.dart'
    show
        PageDecoder,
        ReaderController,
        ReaderAnnotationDeleted,
        ReaderAnnotationNavigated,
        ReaderAnnotationNoteRequested,
        ReaderAnnotationUpdated,
        ReaderMessage,
        ReaderModel,
        ReaderOpenRequested,
        ReaderSelection,
        ReaderSelectionCancelled,
        ReaderSelectionCommitted,
        ReaderSelectionEnded,
        ReaderSelectionExtended,
        ReaderSelectionKeyboardExtended,
        ReaderSelectionMovement,
        ReaderSelectionPhase,
        ReaderContentState,
        ReaderSelectionStarted,
        premultiplyRgba;

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init(externalLibrary: _nativeLibrary());
  runApp(const ShosaiApp());
}

ExternalLibrary? _nativeLibrary() {
  final executableDirectory = File(Platform.resolvedExecutable).parent.path;
  if (Platform.isLinux) {
    return ExternalLibrary.open(
      '$executableDirectory/lib/libshosai_flutter_bridge.so',
    );
  }
  if (Platform.isMacOS) {
    return ExternalLibrary.open(
      '$executableDirectory/../Frameworks/libshosai_flutter_bridge.dylib',
    );
  }
  return null;
}

class ShosaiApp extends StatelessWidget {
  const ShosaiApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xff745b3e),
          brightness: Brightness.light,
        ),
        useMaterial3: true,
      ),
      darkTheme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xffc5a57c),
          brightness: Brightness.dark,
        ),
        useMaterial3: true,
      ),
      home: const ReaderScreen(),
    );
  }
}

class ReaderScreen extends StatefulWidget {
  const ReaderScreen({super.key, this.bridge, this.decoder = _decodeRgba});

  final FlutterBridge? bridge;
  final PageDecoder decoder;

  @override
  State<ReaderScreen> createState() => _ReaderScreenState();
}

class _ReaderScreenState extends State<ReaderScreen> {
  final TextEditingController _path = TextEditingController();
  late final ReaderController _controller;

  @override
  void initState() {
    super.initState();
    _controller = ReaderController(
      bridge: widget.bridge ?? FlutterBridge(),
      decoder: (pixels, {required width, required height}) =>
          widget.decoder(pixels, width: width, height: height),
      noteEditor: (initialValue) => showDialog<String>(
        context: context,
        builder: (context) => _NoteDialog(initialValue: initialValue),
      ),
    )..addListener(_modelChanged);
  }

  void _modelChanged() => setState(() {});

  @override
  void dispose() {
    _controller.removeListener(_modelChanged);
    _controller.dispose();
    _path.dispose();
    super.dispose();
  }

  void _open() => _controller.dispatch(ReaderOpenRequested(_path.text));

  @override
  Widget build(BuildContext context) {
    final model = _controller.model;
    final document = model.document;
    return Scaffold(
      appBar: AppBar(title: const Text('Shōsai Flutter feasibility slice')),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Semantics(
                textField: true,
                label: 'Document path',
                child: TextField(
                  controller: _path,
                  enabled: !model.busy,
                  onSubmitted: (_) => _open(),
                  decoration: const InputDecoration(
                    border: OutlineInputBorder(),
                    hintText: '/path/to/book.pdf',
                    labelText: 'PDF, EPUB, or CBZ path',
                  ),
                ),
              ),
              const SizedBox(height: 12),
              Align(
                alignment: Alignment.centerLeft,
                child: FilledButton.icon(
                  onPressed: model.busy ? null : _open,
                  icon: const Icon(Icons.menu_book),
                  label: Text(model.busy ? 'Opening…' : 'Open document'),
                ),
              ),
              if (model.error != null) ...[
                const SizedBox(height: 12),
                Semantics(
                  liveRegion: true,
                  child: Text(
                    model.error!,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ),
              ],
              if (model.selectionError != null && model.document != null)
                Text('Selection unavailable: ${model.selectionError}'),
              if (model.annotationError != null && model.document != null)
                Text('Highlights unavailable: ${model.annotationError}'),
              const SizedBox(height: 20),
              Expanded(
                child: document == null
                    ? const WelcomePanel()
                    : _DocumentView(
                        document: document,
                        image: model.pageImage,
                        model: model,
                        dispatch: _controller.dispatch,
                      ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class WelcomePanel extends StatelessWidget {
  const WelcomePanel({super.key});

  @override
  Widget build(BuildContext context) {
    return const Center(
      child: Text(
        'Enter a local document path to exercise the generated Rust bridge.',
        textAlign: TextAlign.center,
      ),
    );
  }
}

class _DocumentView extends StatelessWidget {
  const _DocumentView({
    required this.document,
    required this.image,
    required this.model,
    required this.dispatch,
  });

  final FlutterDocumentSummary document;
  final ui.Image? image;
  final ReaderModel model;
  final void Function(ReaderMessage) dispatch;

  @override
  Widget build(BuildContext context) {
    final title = document.title ?? 'Untitled document';
    final surface = model.selectionSurface;
    final page = image;
    if (model.contentState == ReaderContentState.failed) {
      return Center(child: Text(model.error ?? 'Document content unavailable'));
    }
    if ((document.format == FlutterBookFormat.epub && surface == null) ||
        (document.format != FlutterBookFormat.epub && page == null)) {
      return const Center(child: CircularProgressIndicator());
    }
    if (surface == null) {
      return Semantics(
        label: '$title, page 1 of ${document.logicalUnitCount}.',
        child: Center(
          child: RawImage(image: page, fit: BoxFit.contain),
        ),
      );
    }
    return Semantics(
      label: document.format == FlutterBookFormat.epub
          ? '$title, EPUB chapter 1 of ${document.logicalUnitCount}. Selectable text.'
          : '$title, page 1 of ${document.logicalUnitCount}. Selectable text.',
      child: Column(
        children: [
          Expanded(
            child: CallbackShortcuts(
              bindings: {
                const SingleActivator(LogicalKeyboardKey.escape): () =>
                    dispatch(const ReaderSelectionCancelled()),
                const SingleActivator(LogicalKeyboardKey.enter): () =>
                    dispatch(const ReaderSelectionCommitted()),
                const SingleActivator(
                  LogicalKeyboardKey.arrowLeft,
                  shift: true,
                ): () => dispatch(
                  const ReaderSelectionKeyboardExtended(
                    ReaderSelectionMovement.previousGrapheme,
                  ),
                ),
                const SingleActivator(
                  LogicalKeyboardKey.arrowRight,
                  shift: true,
                ): () => dispatch(
                  const ReaderSelectionKeyboardExtended(
                    ReaderSelectionMovement.nextGrapheme,
                  ),
                ),
                const SingleActivator(
                  LogicalKeyboardKey.arrowLeft,
                  shift: true,
                  control: true,
                ): () => dispatch(
                  const ReaderSelectionKeyboardExtended(
                    ReaderSelectionMovement.previousWord,
                  ),
                ),
                const SingleActivator(
                  LogicalKeyboardKey.arrowRight,
                  shift: true,
                  control: true,
                ): () => dispatch(
                  const ReaderSelectionKeyboardExtended(
                    ReaderSelectionMovement.nextWord,
                  ),
                ),
                const SingleActivator(
                  LogicalKeyboardKey.arrowLeft,
                  shift: true,
                  alt: true,
                ): () => dispatch(
                  const ReaderSelectionKeyboardExtended(
                    ReaderSelectionMovement.previousWord,
                  ),
                ),
                const SingleActivator(
                  LogicalKeyboardKey.arrowRight,
                  shift: true,
                  alt: true,
                ): () => dispatch(
                  const ReaderSelectionKeyboardExtended(
                    ReaderSelectionMovement.nextWord,
                  ),
                ),
                const SingleActivator(
                  LogicalKeyboardKey.arrowUp,
                  shift: true,
                ): () => dispatch(
                  const ReaderSelectionKeyboardExtended(
                    ReaderSelectionMovement.previousLine,
                  ),
                ),
                const SingleActivator(
                  LogicalKeyboardKey.arrowDown,
                  shift: true,
                ): () => dispatch(
                  const ReaderSelectionKeyboardExtended(
                    ReaderSelectionMovement.nextLine,
                  ),
                ),
                const SingleActivator(LogicalKeyboardKey.contextMenu): () =>
                    dispatch(const ReaderSelectionEnded()),
                const SingleActivator(
                  LogicalKeyboardKey.f10,
                  shift: true,
                ): () =>
                    dispatch(const ReaderSelectionEnded()),
              },
              child: Focus(
                key: const ValueKey('reader-selection-focus'),
                autofocus: true,
                child: _SelectableSurface(
                  surface: surface,
                  image: page,
                  model: model,
                  dispatch: dispatch,
                ),
              ),
            ),
          ),
          if (model.selectionPhase == ReaderSelectionPhase.selected)
            Semantics(
              label: 'Selection actions',
              child: Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Wrap(
                  spacing: 8,
                  children: [
                    FilledButton.icon(
                      onPressed:
                          model.busy ||
                              !model.annotationsReady ||
                              model.annotationOperations.isNotEmpty
                          ? null
                          : () => dispatch(const ReaderSelectionCommitted()),
                      icon: const Icon(Icons.highlight),
                      label: const Text('Save highlight'),
                    ),
                    TextButton(
                      onPressed: () =>
                          dispatch(const ReaderSelectionCancelled()),
                      child: const Text('Cancel'),
                    ),
                  ],
                ),
              ),
            ),
          if (model.annotations.isNotEmpty)
            SizedBox(
              height: 64,
              child: ListView(
                scrollDirection: Axis.horizontal,
                children: model.annotations
                    .map(
                      (annotation) => Card(
                        child: Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            TextButton(
                              onPressed: () => dispatch(
                                ReaderAnnotationNavigated(annotation.id),
                              ),
                              child: Text(
                                'Highlight ${annotation.unit.toInt() + 1}',
                              ),
                            ),
                            IconButton(
                              tooltip: 'Change color',
                              onPressed: model.annotationOperations.isNotEmpty
                                  ? null
                                  : () => dispatch(
                                      ReaderAnnotationUpdated(
                                        annotation.id,
                                        _nextColor(annotation.color),
                                        annotation.body,
                                      ),
                                    ),
                              icon: const Icon(Icons.palette_outlined),
                            ),
                            IconButton(
                              tooltip: 'Edit note',
                              onPressed: model.annotationOperations.isNotEmpty
                                  ? null
                                  : () => dispatch(
                                      ReaderAnnotationNoteRequested(
                                        annotation.id,
                                      ),
                                    ),
                              icon: const Icon(Icons.note_alt_outlined),
                            ),
                            IconButton(
                              tooltip: 'Delete highlight',
                              onPressed: model.annotationOperations.isNotEmpty
                                  ? null
                                  : () => dispatch(
                                      ReaderAnnotationDeleted(annotation.id),
                                    ),
                              icon: const Icon(Icons.delete_outline),
                            ),
                          ],
                        ),
                      ),
                    )
                    .toList(),
              ),
            ),
        ],
      ),
    );
  }
}

class _NoteDialog extends StatefulWidget {
  const _NoteDialog({required this.initialValue});
  final String? initialValue;
  @override
  State<_NoteDialog> createState() => _NoteDialogState();
}

class _NoteDialogState extends State<_NoteDialog> {
  late final TextEditingController controller = TextEditingController(
    text: widget.initialValue,
  );
  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Highlight note'),
    content: TextField(controller: controller, autofocus: true),
    actions: [
      TextButton(
        onPressed: () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: () => Navigator.pop(context, controller.text),
        child: const Text('Save'),
      ),
    ],
  );
}

FlutterHighlightColor _nextColor(FlutterHighlightColor color) =>
    switch (color) {
      FlutterHighlightColor.yellow => FlutterHighlightColor.green,
      FlutterHighlightColor.green => FlutterHighlightColor.blue,
      FlutterHighlightColor.blue => FlutterHighlightColor.pink,
      FlutterHighlightColor.pink => FlutterHighlightColor.purple,
      FlutterHighlightColor.purple => FlutterHighlightColor.yellow,
    };

class _SelectableSurface extends StatelessWidget {
  const _SelectableSurface({
    required this.surface,
    required this.image,
    required this.model,
    required this.dispatch,
  });

  final FlutterSelectionSurface surface;
  final ui.Image? image;
  final ReaderModel model;
  final void Function(ReaderMessage) dispatch;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final fitted = applyBoxFit(
          BoxFit.contain,
          Size(surface.width, surface.height),
          constraints.biggest,
        ).destination;
        final destination = Alignment.center.inscribe(
          fitted,
          Offset.zero & constraints.biggest,
        );
        int? endpoint(Offset position) {
          if (!destination.contains(position)) return null;
          final source = Offset(
            (position.dx - destination.left) *
                surface.width /
                destination.width,
            (position.dy - destination.top) *
                surface.height /
                destination.height,
          );
          for (final endpoint in surface.endpoints) {
            final rect = endpoint.rect;
            if (Rect.fromLTRB(
              rect.left,
              rect.top,
              rect.right,
              rect.bottom,
            ).contains(source)) {
              return endpoint.offset.toInt();
            }
          }
          return null;
        }

        return Listener(
          onPointerCancel: (_) => dispatch(const ReaderSelectionCancelled()),
          child: GestureDetector(
            key: const ValueKey('reader-selection-surface'),
            behavior: HitTestBehavior.opaque,
            onTapDown: (details) {
              Focus.of(context).requestFocus();
              if (endpoint(details.localPosition) == null) {
                dispatch(const ReaderSelectionCancelled());
              }
            },
            onPanStart: (details) {
              Focus.of(context).requestFocus();
              final value = endpoint(details.localPosition);
              if (value != null) dispatch(ReaderSelectionStarted(value));
            },
            onPanUpdate: (details) {
              final value = endpoint(details.localPosition);
              if (value != null) dispatch(ReaderSelectionExtended(value));
            },
            onPanEnd: (_) => dispatch(const ReaderSelectionEnded()),
            onPanCancel: () => dispatch(const ReaderSelectionCancelled()),
            child: RepaintBoundary(
              key: const ValueKey('reader-page-paint'),
              child: CustomPaint(
                painter: PagePainter(
                  image: image,
                  surface: surface,
                  backgroundColor: pageColors(
                    Theme.of(context).colorScheme,
                  ).background,
                  foregroundColor: pageColors(
                    Theme.of(context).colorScheme,
                  ).foreground,
                  recolorImage:
                      model.document?.format == FlutterBookFormat.epub,
                  anchor: model.anchor,
                  focus: model.focus,
                  savedSelections: model.savedSelections,
                ),
                child: const SizedBox.expand(),
              ),
            ),
          ),
        );
      },
    );
  }
}

class PagePainter extends CustomPainter {
  const PagePainter({
    required this.image,
    required this.surface,
    required this.backgroundColor,
    required this.foregroundColor,
    required this.recolorImage,
    required this.anchor,
    required this.focus,
    required this.savedSelections,
  });

  final ui.Image? image;
  final FlutterSelectionSurface surface;
  final Color backgroundColor;
  final Color foregroundColor;
  final bool recolorImage;
  final int? anchor;
  final int? focus;
  final List<ReaderSelection> savedSelections;

  @override
  void paint(Canvas canvas, Size size) {
    final source = Rect.fromLTWH(0, 0, surface.width, surface.height);
    final scale = (size.width / source.width).clamp(
      0.0,
      size.height / source.height,
    );
    final destinationSize = Size(source.width * scale, source.height * scale);
    final destination = Alignment.center.inscribe(
      destinationSize,
      Offset.zero & size,
    );
    canvas.save();
    canvas.translate(destination.left, destination.top);
    canvas.scale(scale);
    canvas.drawRect(source, Paint()..color = backgroundColor);
    if (image case final page?) {
      final pixelSource = pageImageSource(page);
      canvas.drawImageRect(
        page,
        pixelSource,
        source,
        Paint()
          ..colorFilter = recolorImage
              ? ColorFilter.mode(foregroundColor, BlendMode.srcIn)
              : null,
      );
    }
    for (final saved in savedSelections) {
      _paintRange(
        canvas,
        saved.start,
        saved.end,
        _highlightColor(saved.color),
        true,
      );
    }
    if (anchor != null && focus != null) {
      _paintRange(
        canvas,
        anchor! < focus! ? anchor! : focus!,
        anchor! < focus! ? focus! : anchor!,
        const Color(0x6690caf9),
        false,
      );
    }
    canvas.restore();
  }

  void _paintRange(Canvas canvas, int start, int end, Color color, bool saved) {
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.fill;
    final border = Paint()
      ..color = color.withAlpha(220)
      ..style = PaintingStyle.stroke
      ..strokeWidth = saved ? 1.5 : 1;
    for (final endpoint in surface.endpoints) {
      final rangeStart = endpoint.rangeStart.toInt();
      final rangeEnd = endpoint.rangeEnd.toInt();
      if (rangeStart >= end || start >= rangeEnd) continue;
      final rect = endpoint.rect;
      final area = Rect.fromLTRB(rect.left, rect.top, rect.right, rect.bottom);
      canvas.drawRect(area, paint);
      if (saved) canvas.drawLine(area.bottomLeft, area.bottomRight, border);
    }
  }

  @override
  bool shouldRepaint(PagePainter oldDelegate) =>
      oldDelegate.image != image ||
      oldDelegate.backgroundColor != backgroundColor ||
      oldDelegate.foregroundColor != foregroundColor ||
      oldDelegate.recolorImage != recolorImage ||
      oldDelegate.anchor != anchor ||
      oldDelegate.focus != focus ||
      oldDelegate.savedSelections != savedSelections;
}

({Color background, Color foreground}) pageColors(ColorScheme scheme) =>
    (background: scheme.surface, foreground: scheme.onSurface);

Rect pageImageSource(ui.Image image) =>
    Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble());

Color _highlightColor(FlutterHighlightColor? color) => switch (color) {
  FlutterHighlightColor.green => const Color(0x6670b77e),
  FlutterHighlightColor.blue => const Color(0x666aa9e9),
  FlutterHighlightColor.pink => const Color(0x66dc7ca5),
  FlutterHighlightColor.purple => const Color(0x668876c5),
  FlutterHighlightColor.yellow || null => const Color(0x66e2bd54),
};

Future<ui.Image> _decodeRgba(
  Uint8List pixels, {
  required int width,
  required int height,
}) async {
  final buffer = await ui.ImmutableBuffer.fromUint8List(pixels);
  try {
    final descriptor = ui.ImageDescriptor.raw(
      buffer,
      width: width,
      height: height,
      pixelFormat: ui.PixelFormat.rgba8888,
    );
    try {
      final codec = await descriptor.instantiateCodec();
      try {
        return (await codec.getNextFrame()).image;
      } finally {
        codec.dispose();
      }
    } finally {
      descriptor.dispose();
    }
  } finally {
    buffer.dispose();
  }
}
