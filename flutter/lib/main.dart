import 'dart:io';
import 'dart:math' as math;
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
        ReaderFocusTarget,
        ReaderLayout,
        ReaderLayoutChanged,
        ReaderMessage,
        ReaderModel,
        ReaderOpenRequested,
        ReaderSelection,
        ReaderSelectionActionsRequested,
        ReaderSelectionCancelled,
        ReaderSelectionCommitted,
        ReaderSelectionCopyRequested,
        ReaderSelectionEnded,
        ReaderSelectionExtended,
        ReaderSelectionKeyboardExtended,
        ReaderSelectionMovement,
        ReaderSelectionNoteRequested,
        ReaderSelectionPhase,
        ReaderSelectionPointerCancelled,
        ReaderSelectionPointerEnded,
        ReaderSelectionPointerMoved,
        ReaderSelectionPointerPressedOutside,
        ReaderSelectionPointerStarted,
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
  final GlobalKey _pathFieldKey = GlobalKey(debugLabel: 'document path');
  final FocusNode _openFocus = FocusNode(debugLabel: 'open document');
  final FocusNode _readerFocus = FocusNode(debugLabel: 'reader surface');
  final FocusNode _actionFocus = FocusNode(debugLabel: 'selection actions');
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
      focusAdapter: (target) => switch (target) {
        ReaderFocusTarget.surface => _readerFocus.requestFocus(),
        ReaderFocusTarget.actions => _actionFocus.requestFocus(),
      },
      selectionCopier: (text) => Clipboard.setData(ClipboardData(text: text)),
    )..addListener(_modelChanged);
  }

  void _modelChanged() => setState(() {});

  @override
  void dispose() {
    _controller.removeListener(_modelChanged);
    _controller.dispose();
    _path.dispose();
    _openFocus.dispose();
    _readerFocus.dispose();
    _actionFocus.dispose();
    super.dispose();
  }

  void _open() => _controller.dispatch(ReaderOpenRequested(_path.text));

  @override
  Widget build(BuildContext context) {
    final model = _controller.model;
    final compact = MediaQuery.sizeOf(context).width < 600;
    return Scaffold(
      appBar: AppBar(
        title: Text(compact ? 'Shōsai' : 'Shōsai Flutter feasibility slice'),
      ),
      body: SafeArea(
        child: _ResponsiveReaderBody(
          model: model,
          path: _path,
          pathFieldKey: _pathFieldKey,
          openFocus: _openFocus,
          open: _open,
          dispatch: _controller.dispatch,
          readerFocus: _readerFocus,
          actionFocus: _actionFocus,
        ),
      ),
    );
  }
}

enum _ReaderComposition { compact, medium, expanded }

class _ResponsiveReaderBody extends StatelessWidget {
  const _ResponsiveReaderBody({
    required this.model,
    required this.path,
    required this.pathFieldKey,
    required this.openFocus,
    required this.open,
    required this.dispatch,
    required this.readerFocus,
    required this.actionFocus,
  });

  final ReaderModel model;
  final TextEditingController path;
  final GlobalKey pathFieldKey;
  final FocusNode openFocus;
  final VoidCallback open;
  final void Function(ReaderMessage) dispatch;
  final FocusNode readerFocus;
  final FocusNode actionFocus;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final composition = constraints.maxWidth >= 1024
          ? _ReaderComposition.expanded
          : constraints.maxWidth >= 600
          ? _ReaderComposition.medium
          : _ReaderComposition.compact;
      final controls = _ReaderControls(
        model: model,
        path: path,
        pathFieldKey: pathFieldKey,
        openFocus: openFocus,
        open: open,
        horizontal: composition == _ReaderComposition.medium,
      );
      final content = _ReaderContentPane(
        model: model,
        dispatch: dispatch,
        readerFocus: readerFocus,
        actionFocus: actionFocus,
      );
      final key = ValueKey('reader-composition-${composition.name}');
      if (composition == _ReaderComposition.expanded) {
        return Padding(
          key: key,
          padding: const EdgeInsets.all(24),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SizedBox(
                width: 320,
                child: SingleChildScrollView(child: controls),
              ),
              const SizedBox(width: 24),
              Expanded(child: content),
            ],
          ),
        );
      }
      return Padding(
        key: key,
        padding: const EdgeInsets.all(24),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            controls,
            const SizedBox(height: 20),
            Expanded(child: content),
          ],
        ),
      );
    },
  );
}

class _ReaderControls extends StatelessWidget {
  const _ReaderControls({
    required this.model,
    required this.path,
    required this.pathFieldKey,
    required this.openFocus,
    required this.open,
    required this.horizontal,
  });

  final ReaderModel model;
  final TextEditingController path;
  final GlobalKey pathFieldKey;
  final FocusNode openFocus;
  final VoidCallback open;
  final bool horizontal;

  @override
  Widget build(BuildContext context) {
    final field = Semantics(
      textField: true,
      label: 'Document path',
      child: TextField(
        key: pathFieldKey,
        controller: path,
        enabled: !model.busy,
        onSubmitted: (_) => open(),
        decoration: const InputDecoration(
          border: OutlineInputBorder(),
          hintText: '/path/to/book.pdf',
          labelText: 'PDF, EPUB, or CBZ path',
        ),
      ),
    );
    final button = FilledButton.icon(
      focusNode: openFocus,
      onPressed: model.busy ? null : open,
      icon: const Icon(Icons.menu_book),
      label: Text(model.busy ? 'Opening…' : 'Open document'),
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (horizontal)
          Row(
            children: [
              Expanded(child: field),
              const SizedBox(width: 12),
              button,
            ],
          )
        else ...[
          field,
          const SizedBox(height: 12),
          Align(alignment: Alignment.centerLeft, child: button),
        ],
        if (model.error != null) ...[
          const SizedBox(height: 12),
          Semantics(
            liveRegion: true,
            child: Text(
              model.error!,
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ),
        ],
        if (model.selectionError != null && model.document != null)
          Semantics(
            liveRegion: true,
            child: Text('Selection unavailable: ${model.selectionError}'),
          ),
        if (model.selectionActionError != null && model.document != null)
          Semantics(
            liveRegion: true,
            child: Text(
              'Selection action failed: ${model.selectionActionError}',
            ),
          ),
        if (model.annotationError != null && model.document != null)
          Semantics(
            liveRegion: true,
            child: Text(
              model.annotationsReady
                  ? 'Highlight action failed: ${model.annotationError}'
                  : 'Highlights unavailable: ${model.annotationError}',
            ),
          ),
        if (model.relayoutBusy) const LinearProgressIndicator(),
      ],
    );
  }
}

class _ReaderContentPane extends StatelessWidget {
  const _ReaderContentPane({
    required this.model,
    required this.dispatch,
    required this.readerFocus,
    required this.actionFocus,
  });

  final ReaderModel model;
  final void Function(ReaderMessage) dispatch;
  final FocusNode readerFocus;
  final FocusNode actionFocus;

  @override
  Widget build(BuildContext context) => _ReaderLayoutReporter(
    model: model,
    dispatch: dispatch,
    child: model.document == null
        ? const WelcomePanel()
        : _DocumentView(
            document: model.document!,
            image: model.pageImage,
            model: model,
            dispatch: dispatch,
            readerFocus: readerFocus,
            actionFocus: actionFocus,
          ),
  );
}

class _ReaderLayoutReporter extends StatefulWidget {
  const _ReaderLayoutReporter({
    required this.model,
    required this.dispatch,
    required this.child,
  });

  final ReaderModel model;
  final void Function(ReaderMessage) dispatch;
  final Widget child;

  @override
  State<_ReaderLayoutReporter> createState() => _ReaderLayoutReporterState();
}

class _ReaderLayoutReporterState extends State<_ReaderLayoutReporter> {
  bool _scheduled = false;
  ReaderLayout? _observedLayout;
  ReaderLayout? _pendingLayout;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final availableWidth = constraints.maxWidth.isFinite
          ? math.max(1.0, constraints.maxWidth).roundToDouble()
          : widget.model.layout.width;
      final layout = ReaderLayout(
        scale: MediaQuery.devicePixelRatioOf(context),
        width: availableWidth,
        fontSize: MediaQuery.textScalerOf(context).scale(18),
      );
      if (layout != _observedLayout) {
        _observedLayout = layout;
        _pendingLayout = layout;
      }
      if (_pendingLayout != null && !_scheduled) {
        _scheduled = true;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          _scheduled = false;
          final pending = _pendingLayout;
          _pendingLayout = null;
          if (mounted && pending != null) {
            widget.dispatch(ReaderLayoutChanged(pending));
          }
        });
      }
      return widget.child;
    },
  );
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
    required this.readerFocus,
    required this.actionFocus,
  });

  final FlutterDocumentSummary document;
  final ui.Image? image;
  final ReaderModel model;
  final void Function(ReaderMessage) dispatch;
  final FocusNode readerFocus;
  final FocusNode actionFocus;

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
      child: CallbackShortcuts(
        bindings: {
          const SingleActivator(LogicalKeyboardKey.escape): () =>
              dispatch(const ReaderSelectionCancelled()),
          const SingleActivator(LogicalKeyboardKey.keyC, control: true): () =>
              dispatch(const ReaderSelectionCopyRequested()),
          const SingleActivator(LogicalKeyboardKey.keyC, meta: true): () =>
              dispatch(const ReaderSelectionCopyRequested()),
        },
        child: Column(
          children: [
            Expanded(
              child: TapRegion(
                onTapOutside: (event) => dispatch(
                  ReaderSelectionPointerPressedOutside(event.pointer),
                ),
                child: LayoutBuilder(
                  builder: (context, constraints) => Stack(
                    children: [
                      Positioned.fill(
                        child: CallbackShortcuts(
                          bindings: {
                            const SingleActivator(
                              LogicalKeyboardKey.escape,
                            ): () =>
                                dispatch(const ReaderSelectionCancelled()),
                            const SingleActivator(
                              LogicalKeyboardKey.enter,
                            ): () =>
                                dispatch(const ReaderSelectionCommitted()),
                            const SingleActivator(
                              LogicalKeyboardKey.arrowLeft,
                              shift: true,
                            ): () => dispatch(
                              const ReaderSelectionKeyboardExtended(
                                ReaderSelectionMovement.visualLeft,
                              ),
                            ),
                            const SingleActivator(
                              LogicalKeyboardKey.arrowRight,
                              shift: true,
                            ): () => dispatch(
                              const ReaderSelectionKeyboardExtended(
                                ReaderSelectionMovement.visualRight,
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
                            const SingleActivator(
                              LogicalKeyboardKey.home,
                              shift: true,
                            ): () => dispatch(
                              const ReaderSelectionKeyboardExtended(
                                ReaderSelectionMovement.lineStart,
                              ),
                            ),
                            const SingleActivator(
                              LogicalKeyboardKey.end,
                              shift: true,
                            ): () => dispatch(
                              const ReaderSelectionKeyboardExtended(
                                ReaderSelectionMovement.lineEnd,
                              ),
                            ),
                            const SingleActivator(
                              LogicalKeyboardKey.arrowLeft,
                              shift: true,
                              meta: true,
                            ): () => dispatch(
                              const ReaderSelectionKeyboardExtended(
                                ReaderSelectionMovement.lineStart,
                              ),
                            ),
                            const SingleActivator(
                              LogicalKeyboardKey.arrowRight,
                              shift: true,
                              meta: true,
                            ): () => dispatch(
                              const ReaderSelectionKeyboardExtended(
                                ReaderSelectionMovement.lineEnd,
                              ),
                            ),
                            const SingleActivator(
                              LogicalKeyboardKey.contextMenu,
                            ): () => dispatch(
                              const ReaderSelectionActionsRequested(),
                            ),
                            const SingleActivator(
                              LogicalKeyboardKey.f10,
                              shift: true,
                            ): () => dispatch(
                              const ReaderSelectionActionsRequested(),
                            ),
                          },
                          child: Focus(
                            key: const ValueKey('reader-selection-focus'),
                            focusNode: readerFocus,
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
                        Positioned.fill(
                          child: CustomSingleChildLayout(
                            delegate: _SelectionActionsLayout(
                              target: _selectionActionTarget(
                                surface,
                                model,
                                constraints.biggest,
                              ),
                            ),
                            child: _SelectionActions(
                              model: model,
                              dispatch: dispatch,
                              focusNode: actionFocus,
                            ),
                          ),
                        ),
                    ],
                  ),
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
                                  'Highlight ${annotation.unit.toInt() + 1}'
                                  '${_annotationResolutionSuffix(annotation.resolution)}',
                                ),
                              ),
                              IconButton(
                                tooltip: 'Change color',
                                onPressed:
                                    model.annotationOperations.isNotEmpty ||
                                        model.relayoutBusy
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
                                onPressed:
                                    model.annotationOperations.isNotEmpty ||
                                        model.relayoutBusy
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
                                onPressed:
                                    model.annotationOperations.isNotEmpty ||
                                        model.relayoutBusy
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
      ),
    );
  }
}

class _SelectionActions extends StatelessWidget {
  const _SelectionActions({
    required this.model,
    required this.dispatch,
    required this.focusNode,
  });

  final ReaderModel model;
  final void Function(ReaderMessage) dispatch;
  final FocusNode focusNode;

  @override
  Widget build(BuildContext context) {
    final copyEnabled = model.selectedText != null;
    final persistenceEnabled =
        !model.busy &&
        !model.relayoutBusy &&
        model.annotationsReady &&
        model.annotationOperations.isEmpty;
    return Semantics(
      key: const ValueKey('selection-actions'),
      label: 'Selection actions',
      container: true,
      child: Material(
        elevation: 4,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.all(8),
          child: SingleChildScrollView(
            child: Wrap(
              alignment: WrapAlignment.center,
              spacing: 8,
              runSpacing: 8,
              children: [
                TextButton(
                  focusNode: copyEnabled ? focusNode : null,
                  onPressed: !copyEnabled
                      ? null
                      : () => dispatch(const ReaderSelectionCopyRequested()),
                  child: const Text('Copy'),
                ),
                for (final color in FlutterHighlightColor.values)
                  FilledButton(
                    focusNode:
                        !copyEnabled &&
                            persistenceEnabled &&
                            color == FlutterHighlightColor.yellow
                        ? focusNode
                        : null,
                    onPressed: !persistenceEnabled
                        ? null
                        : () =>
                              dispatch(ReaderSelectionCommitted(color: color)),
                    child: Text(_colorName(color)),
                  ),
                TextButton(
                  onPressed: !persistenceEnabled
                      ? null
                      : () => dispatch(const ReaderSelectionNoteRequested()),
                  child: const Text('Add note'),
                ),
                TextButton(
                  focusNode: !copyEnabled && !persistenceEnabled
                      ? focusNode
                      : null,
                  onPressed: () => dispatch(const ReaderSelectionCancelled()),
                  child: const Text('Cancel'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

Rect _selectionActionTarget(
  FlutterSelectionSurface surface,
  ReaderModel model,
  Size viewport,
) {
  final first = model.anchor!;
  final second = model.focus!;
  final start = first < second ? first : second;
  final end = first < second ? second : first;
  Rect? selected;
  for (final endpoint in surface.endpoints) {
    final rangeStart = endpoint.rangeStart.toInt();
    final rangeEnd = endpoint.rangeEnd.toInt();
    final focusedOffset = second == end ? end - 1 : start;
    final include = model.keyboardActionInvocation
        ? rangeStart <= focusedOffset && focusedOffset < rangeEnd
        : rangeStart < end && start < rangeEnd;
    if (!include) {
      continue;
    }
    final rect = endpoint.rect;
    final area = Rect.fromLTRB(rect.left, rect.top, rect.right, rect.bottom);
    selected = selected?.expandToInclude(area) ?? area;
  }
  if (selected == null) return Offset.zero & Size.zero;
  final fitted = applyBoxFit(
    BoxFit.contain,
    Size(surface.width, surface.height),
    viewport,
  ).destination;
  final destination = Alignment.center.inscribe(fitted, Offset.zero & viewport);
  final scale = fitted.width / surface.width;
  return Rect.fromLTRB(
    destination.left + selected.left * scale,
    destination.top + selected.top * scale,
    destination.left + selected.right * scale,
    destination.top + selected.bottom * scale,
  );
}

class _SelectionActionsLayout extends SingleChildLayoutDelegate {
  const _SelectionActionsLayout({required this.target});

  static const _gap = 8.0;
  final Rect target;

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) =>
      BoxConstraints(
        maxWidth: math.max(0, constraints.maxWidth - _gap * 2),
        maxHeight: math.max(0, constraints.maxHeight - _gap * 2),
      );

  @override
  Offset getPositionForChild(Size size, Size childSize) {
    final maxLeft = math.max(_gap, size.width - childSize.width - _gap);
    final left = (target.center.dx - childSize.width / 2).clamp(_gap, maxLeft);
    final above = target.top - childSize.height - _gap;
    final below = target.bottom + _gap;
    final maxTop = math.max(_gap, size.height - childSize.height - _gap);
    final top = above >= _gap ? above : below.clamp(_gap, maxTop);
    return Offset(left, top);
  }

  @override
  bool shouldRelayout(_SelectionActionsLayout oldDelegate) =>
      target != oldDelegate.target;
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

String _annotationResolutionSuffix(FlutterAnnotationResolution resolution) =>
    switch (resolution) {
      FlutterAnnotationResolution.exact => '',
      FlutterAnnotationResolution.recovered => ' — recovered',
      FlutterAnnotationResolution.ambiguous => ' — ambiguous',
      FlutterAnnotationResolution.orphaned => ' — unavailable',
    };

String _colorName(FlutterHighlightColor color) => switch (color) {
  FlutterHighlightColor.yellow => 'Yellow',
  FlutterHighlightColor.green => 'Green',
  FlutterHighlightColor.blue => 'Blue',
  FlutterHighlightColor.pink => 'Pink',
  FlutterHighlightColor.purple => 'Purple',
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
        Offset sourcePosition(Offset position) => Offset(
          ((position.dx - destination.left) * surface.width / destination.width)
              .clamp(0, surface.width),
          ((position.dy - destination.top) *
                  surface.height /
                  destination.height)
              .clamp(0, surface.height),
        );
        FlutterSelectionEndpoint? endpoint(
          Offset position, {
          bool nearest = false,
        }) {
          if (!nearest && !destination.contains(position)) return null;
          final source = sourcePosition(position);
          for (final endpoint in surface.endpoints) {
            final rect = endpoint.rect;
            if (Rect.fromLTRB(
              rect.left,
              rect.top,
              rect.right,
              rect.bottom,
            ).contains(source)) {
              return endpoint;
            }
          }
          if (!nearest || surface.endpoints.isEmpty) return null;
          FlutterSelectionEndpoint? closest;
          double? distance;
          for (final endpoint in surface.endpoints) {
            final rect = endpoint.rect;
            final dx = source.dx.clamp(rect.left, rect.right) - source.dx;
            final dy = source.dy.clamp(rect.top, rect.bottom) - source.dy;
            final candidate = dx * dx + dy * dy;
            if (distance == null || candidate < distance) {
              closest = endpoint;
              distance = candidate;
            }
          }
          return closest;
        }

        return Listener(
          key: const ValueKey('reader-selection-surface'),
          behavior: HitTestBehavior.opaque,
          onPointerDown: (event) {
            final primary =
                event.kind != ui.PointerDeviceKind.mouse ||
                (event.buttons & 1) != 0;
            if (!primary) return;
            final value = endpoint(event.localPosition);
            if (value == null) {
              dispatch(ReaderSelectionPointerPressedOutside(event.pointer));
            } else {
              final source = sourcePosition(event.localPosition);
              dispatch(
                ReaderSelectionPointerStarted(
                  event.pointer,
                  value.offset.toInt(),
                  rangeStart: value.rangeStart.toInt(),
                  rangeEnd: value.rangeEnd.toInt(),
                  x: source.dx,
                  y: source.dy,
                ),
              );
            }
          },
          onPointerMove: (event) {
            final value = endpoint(event.localPosition, nearest: true);
            if (value != null) {
              final source = sourcePosition(event.localPosition);
              dispatch(
                ReaderSelectionPointerMoved(
                  event.pointer,
                  value.offset.toInt(),
                  x: source.dx,
                  y: source.dy,
                ),
              );
            }
          },
          onPointerUp: (event) =>
              dispatch(ReaderSelectionPointerEnded(event.pointer)),
          onPointerCancel: (event) =>
              dispatch(ReaderSelectionPointerCancelled(event.pointer)),
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
                recolorImage: model.document?.format == FlutterBookFormat.epub,
                anchor: model.anchor,
                focus: model.focus,
                savedSelections: model.savedSelections,
                annotations: model.annotations,
              ),
              child: const SizedBox.expand(),
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
    required this.annotations,
  });

  final ui.Image? image;
  final FlutterSelectionSurface surface;
  final Color backgroundColor;
  final Color foregroundColor;
  final bool recolorImage;
  final int? anchor;
  final int? focus;
  final List<ReaderSelection> savedSelections;
  final List<FlutterAnnotation> annotations;

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
    for (final annotation in annotations) {
      if (annotation.unit != BigInt.zero || annotation.textRange != null) {
        continue;
      }
      _paintRectangles(
        canvas,
        annotation.rectangles ?? const [],
        _highlightColor(annotation.color),
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

  void _paintRectangles(
    Canvas canvas,
    List<FlutterSelectionRect> rectangles,
    Color color,
  ) {
    final fill = Paint()
      ..color = color
      ..style = PaintingStyle.fill;
    final border = Paint()
      ..color = color.withAlpha(220)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5;
    for (final rect in rectangles) {
      final area = Rect.fromLTRB(rect.left, rect.top, rect.right, rect.bottom);
      canvas.drawRect(area, fill);
      canvas.drawLine(area.bottomLeft, area.bottomRight, border);
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
      oldDelegate.savedSelections != savedSelections ||
      oldDelegate.annotations != annotations;
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
