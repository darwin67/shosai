import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart'
    show FlutterError, FlutterErrorDetails, Listenable, VoidCallback;
import 'package:shosai_flutter/src/rust/api.dart';

typedef PageDecoder =
    Future<ui.Image> Function(
      Uint8List pixels, {
      required int width,
      required int height,
    });

typedef NoteEditor = Future<String?> Function(String? initialValue);
typedef ReaderFocusAdapter = void Function(ReaderFocusTarget target);
typedef SelectionCopier = Future<void> Function(String text);

const _unchanged = Object();
final _frozenSurfaces = Expando<bool>();
final _frozenAnnotations = Expando<bool>();

final class ReaderLayout {
  const ReaderLayout({this.scale = 1, this.width = 680, this.fontSize = 18});

  final double scale;
  final double width;
  final double fontSize;

  bool get isValid =>
      scale.isFinite &&
      scale > 0 &&
      width.isFinite &&
      width > 0 &&
      fontSize.isFinite &&
      fontSize > 0;

  @override
  bool operator ==(Object other) =>
      other is ReaderLayout &&
      scale == other.scale &&
      width == other.width &&
      fontSize == other.fontSize;

  @override
  int get hashCode => Object.hash(scale, width, fontSize);
}

final class ReaderModel {
  ReaderModel({
    this.document,
    this.pageImage,
    FlutterSelectionSurface? selectionSurface,
    this.selectionPhase = ReaderSelectionPhase.idle,
    this.anchor,
    this.focus,
    this.selectionPointer,
    this.selectionVisualLine,
    this.selectionPreferredX,
    this.keyboardActionInvocation = false,
    List<ReaderSelection> savedSelections = const [],
    List<FlutterAnnotation> annotations = const [],
    Set<String> annotationOperations = const {},
    this.selectionError,
    this.selectionActionError,
    this.annotationError,
    this.annotationsReady = false,
    this.layout = const ReaderLayout(),
    this.relayoutBusy = false,
    this.contentState = ReaderContentState.loading,
    this.error,
    this.busy = false,
    this.generation = 0,
  }) : selectionSurface = selectionSurface == null
           ? null
           : _freezeSurface(selectionSurface),
       savedSelections = List.unmodifiable(savedSelections),
       annotations = List.unmodifiable(annotations.map(_freezeAnnotation)),
       annotationOperations = Set.unmodifiable(annotationOperations);

  final FlutterDocumentSummary? document;
  final ui.Image? pageImage;
  final FlutterSelectionSurface? selectionSurface;
  final ReaderSelectionPhase selectionPhase;
  final int? anchor;
  final int? focus;
  final int? selectionPointer;
  final int? selectionVisualLine;
  final double? selectionPreferredX;
  final bool keyboardActionInvocation;
  final List<ReaderSelection> savedSelections;
  final List<FlutterAnnotation> annotations;
  final Set<String> annotationOperations;
  final String? selectionError;
  final String? selectionActionError;
  final String? annotationError;
  final bool annotationsReady;
  final ReaderLayout layout;
  final bool relayoutBusy;
  final ReaderContentState contentState;
  final String? error;
  final bool busy;
  final int generation;

  String? get selectedText {
    final surface = selectionSurface;
    final first = anchor;
    final second = focus;
    if (surface == null ||
        !surface.copyEligible ||
        first == null ||
        second == null ||
        first == second) {
      return null;
    }
    final start = first < second ? first : second;
    final end = first < second ? second : first;
    final scalars = surface.text.runes.toList(growable: false);
    if (start < 0 || end > scalars.length) return null;
    return String.fromCharCodes(scalars.sublist(start, end));
  }

  ReaderModel copyWith({
    Object? document = _unchanged,
    Object? pageImage = _unchanged,
    Object? selectionSurface = _unchanged,
    ReaderSelectionPhase? selectionPhase,
    Object? anchor = _unchanged,
    Object? focus = _unchanged,
    Object? selectionPointer = _unchanged,
    Object? selectionVisualLine = _unchanged,
    Object? selectionPreferredX = _unchanged,
    bool? keyboardActionInvocation,
    List<ReaderSelection>? savedSelections,
    List<FlutterAnnotation>? annotations,
    Set<String>? annotationOperations,
    Object? selectionError = _unchanged,
    Object? selectionActionError = _unchanged,
    Object? annotationError = _unchanged,
    bool? annotationsReady,
    ReaderLayout? layout,
    bool? relayoutBusy,
    ReaderContentState? contentState,
    Object? error = _unchanged,
    bool? busy,
    int? generation,
  }) {
    return ReaderModel(
      document: identical(document, _unchanged)
          ? this.document
          : document as FlutterDocumentSummary?,
      pageImage: identical(pageImage, _unchanged)
          ? this.pageImage
          : pageImage as ui.Image?,
      selectionSurface: identical(selectionSurface, _unchanged)
          ? this.selectionSurface
          : selectionSurface as FlutterSelectionSurface?,
      selectionPhase: selectionPhase ?? this.selectionPhase,
      anchor: identical(anchor, _unchanged) ? this.anchor : anchor as int?,
      focus: identical(focus, _unchanged) ? this.focus : focus as int?,
      selectionPointer: identical(selectionPointer, _unchanged)
          ? this.selectionPointer
          : selectionPointer as int?,
      selectionVisualLine: identical(selectionVisualLine, _unchanged)
          ? this.selectionVisualLine
          : selectionVisualLine as int?,
      selectionPreferredX: identical(selectionPreferredX, _unchanged)
          ? this.selectionPreferredX
          : selectionPreferredX as double?,
      keyboardActionInvocation:
          keyboardActionInvocation ?? this.keyboardActionInvocation,
      savedSelections: savedSelections == null
          ? this.savedSelections
          : List.unmodifiable(savedSelections),
      annotations: annotations == null
          ? this.annotations
          : List.unmodifiable(annotations),
      annotationOperations: annotationOperations == null
          ? this.annotationOperations
          : Set.unmodifiable(annotationOperations),
      selectionError: identical(selectionError, _unchanged)
          ? this.selectionError
          : selectionError as String?,
      selectionActionError: identical(selectionActionError, _unchanged)
          ? this.selectionActionError
          : selectionActionError as String?,
      annotationError: identical(annotationError, _unchanged)
          ? this.annotationError
          : annotationError as String?,
      annotationsReady: annotationsReady ?? this.annotationsReady,
      layout: layout ?? this.layout,
      relayoutBusy: relayoutBusy ?? this.relayoutBusy,
      contentState: contentState ?? this.contentState,
      error: identical(error, _unchanged) ? this.error : error as String?,
      busy: busy ?? this.busy,
      generation: generation ?? this.generation,
    );
  }
}

enum ReaderSelectionPhase { idle, selecting, selected, committing }

enum ReaderContentState { loading, ready, failed }

enum ReaderFocusTarget { surface, actions }

enum ReaderSelectionMovement {
  previousGrapheme,
  nextGrapheme,
  previousWord,
  nextWord,
  previousLine,
  nextLine,
  lineStart,
  lineEnd,
  visualLeft,
  visualRight,
}

final class ReaderSelection {
  const ReaderSelection(this.start, this.end, [this.color]);

  final int start;
  final int end;
  final FlutterHighlightColor? color;
}

sealed class ReaderMessage {
  const ReaderMessage();
}

final class ReaderOpenRequested extends ReaderMessage {
  const ReaderOpenRequested(this.path);

  final String path;
}

final class ReaderLayoutChanged extends ReaderMessage {
  const ReaderLayoutChanged(this.layout);

  final ReaderLayout layout;
}

final class ReaderSelectionStarted extends ReaderMessage {
  const ReaderSelectionStarted(this.offset);
  final int offset;
}

final class ReaderSelectionExtended extends ReaderMessage {
  const ReaderSelectionExtended(this.offset);
  final int offset;
}

final class ReaderSelectionPointerStarted extends ReaderMessage {
  const ReaderSelectionPointerStarted(
    this.pointer,
    this.offset, {
    this.rangeStart,
    this.rangeEnd,
    this.x,
    this.y,
  });
  final int pointer;
  final int offset;
  final int? rangeStart;
  final int? rangeEnd;
  final double? x;
  final double? y;
}

final class ReaderSelectionPointerPressedOutside extends ReaderMessage {
  const ReaderSelectionPointerPressedOutside(this.pointer);
  final int pointer;
}

final class ReaderSelectionPointerMoved extends ReaderMessage {
  const ReaderSelectionPointerMoved(
    this.pointer,
    this.offset, {
    this.x,
    this.y,
  });
  final int pointer;
  final int offset;
  final double? x;
  final double? y;
}

final class ReaderSelectionPointerEnded extends ReaderMessage {
  const ReaderSelectionPointerEnded(this.pointer);
  final int pointer;
}

final class ReaderSelectionPointerCancelled extends ReaderMessage {
  const ReaderSelectionPointerCancelled(this.pointer);
  final int pointer;
}

final class ReaderSelectionKeyboardExtended extends ReaderMessage {
  const ReaderSelectionKeyboardExtended(this.movement);
  final ReaderSelectionMovement movement;
}

final class ReaderSelectionEnded extends ReaderMessage {
  const ReaderSelectionEnded();
}

final class ReaderSelectionActionsRequested extends ReaderMessage {
  const ReaderSelectionActionsRequested();
}

final class ReaderSelectionCommitted extends ReaderMessage {
  const ReaderSelectionCommitted({
    this.color = FlutterHighlightColor.yellow,
    this.body,
  });
  final FlutterHighlightColor color;
  final String? body;
}

final class ReaderSelectionNoteRequested extends ReaderMessage {
  const ReaderSelectionNoteRequested();
}

final class ReaderSelectionCopyRequested extends ReaderMessage {
  const ReaderSelectionCopyRequested();
}

final class _ReaderSelectionNoteCompleted extends ReaderMessage {
  const _ReaderSelectionNoteCompleted(
    this.generation,
    this.revision,
    this.selectionRevision,
    this.body,
  );
  final int generation;
  final int revision;
  final int selectionRevision;
  final String body;
}

final class _ReaderSelectionEffectFailed extends ReaderMessage {
  const _ReaderSelectionEffectFailed(
    this.generation,
    this.revision,
    this.selectionRevision,
    this.error,
  );
  final int generation;
  final int revision;
  final int selectionRevision;
  final String error;
}

final class ReaderAnnotationUpdated extends ReaderMessage {
  const ReaderAnnotationUpdated(this.id, this.color, this.body);
  final String id;
  final FlutterHighlightColor color;
  final String? body;
}

final class ReaderAnnotationNoteRequested extends ReaderMessage {
  const ReaderAnnotationNoteRequested(this.id);
  final String id;
}

/// Completion from a controller-owned note editor effect.
final class _ReaderAnnotationNoteCompleted extends ReaderMessage {
  const _ReaderAnnotationNoteCompleted(
    this.generation,
    this.revision,
    this.id,
    this.body,
  );
  final int generation;
  final int revision;
  final String id;
  final String body;
}

final class _ReaderAnnotationNoteFailed extends ReaderMessage {
  const _ReaderAnnotationNoteFailed(this.generation, this.revision, this.error);
  final int generation;
  final int revision;
  final String error;
}

final class ReaderAnnotationDeleted extends ReaderMessage {
  const ReaderAnnotationDeleted(this.id);
  final String id;
}

final class ReaderAnnotationNavigated extends ReaderMessage {
  const ReaderAnnotationNavigated(this.id);
  final String id;
}

final class ReaderSelectionCancelled extends ReaderMessage {
  const ReaderSelectionCancelled();
}

final class _ReaderDocumentOpened extends ReaderMessage {
  const _ReaderDocumentOpened({
    required this.generation,
    required this.document,
  });

  final int generation;
  final FlutterDocumentSummary document;
}

final class _ReaderImageDecoded extends ReaderMessage {
  const _ReaderImageDecoded({
    required this.generation,
    required this.pageImage,
  });

  final int generation;
  final ui.Image? pageImage;
}

final class _ReaderSurfaceLoaded extends ReaderMessage {
  const _ReaderSurfaceLoaded({required this.generation, required this.surface});
  final int generation;
  final FlutterSelectionSurface surface;
}

final class _ReaderEpubContentLoaded extends ReaderMessage {
  const _ReaderEpubContentLoaded({
    required this.generation,
    required this.surface,
    required this.pageImage,
  });
  final int generation;
  final FlutterSelectionSurface surface;
  final ui.Image pageImage;
}

final class _ReaderSelectionSupportFailed extends ReaderMessage {
  const _ReaderSelectionSupportFailed(this.generation, this.error);
  final int generation;
  final String error;
}

final class _ReaderRelayoutCompleted extends ReaderMessage {
  const _ReaderRelayoutCompleted({
    required this.generation,
    required this.revision,
    required this.cancellation,
    required this.layout,
    required this.surface,
    required this.pageImage,
    required this.annotations,
  });

  final int generation;
  final int revision;
  final BigInt cancellation;
  final ReaderLayout layout;
  final FlutterSelectionSurface surface;
  final ui.Image pageImage;
  final List<FlutterAnnotation> annotations;
}

final class _ReaderRelayoutFailed extends ReaderMessage {
  const _ReaderRelayoutFailed({
    required this.generation,
    required this.revision,
    required this.error,
  });

  final int generation;
  final int revision;
  final String error;
}

final class _ReaderRelayoutFinished extends ReaderMessage {
  const _ReaderRelayoutFinished(this.cancellation);

  final BigInt cancellation;
}

final class _ReaderAnnotationListFailed extends ReaderMessage {
  const _ReaderAnnotationListFailed(this.generation, this.revision, this.error);
  final int generation;
  final int revision;
  final String error;
}

final class _ReaderAnnotationsChanged extends ReaderMessage {
  const _ReaderAnnotationsChanged(
    this.generation,
    this.revision,
    this.operationId,
    this.selectionRevision,
    this.items, [
    this.error,
  ]);
  final int generation;
  final int revision;
  final String? operationId;
  final int? selectionRevision;
  final List<FlutterAnnotation>? items;
  final String? error;
}

final class _ReaderOpenFailed extends ReaderMessage {
  const _ReaderOpenFailed({
    required this.generation,
    required this.document,
    required this.error,
  });

  final int generation;
  final FlutterDocumentSummary? document;
  final String error;
}

final class _ReaderOperationFinished extends ReaderMessage {
  const _ReaderOperationFinished({
    required this.generation,
    required this.cancellation,
  });

  final int generation;
  final BigInt cancellation;
}

final class _ReaderAnnotationCreateFinished extends ReaderMessage {
  const _ReaderAnnotationCreateFinished(this.cancellation);
  final BigInt cancellation;
}

final class _ReaderAnnotationOperationFinished extends ReaderMessage {
  const _ReaderAnnotationOperationFinished();
}

final class _ReaderDisposeRequested extends ReaderMessage {
  const _ReaderDisposeRequested();
}

final class ReaderController implements Listenable {
  ReaderController({
    required FlutterBridge bridge,
    required PageDecoder decoder,
    NoteEditor? noteEditor,
    ReaderFocusAdapter? focusAdapter,
    SelectionCopier? selectionCopier,
  }) : _bridge = bridge,
       _decoder = decoder,
       _noteEditor = noteEditor ?? ((_) async => null),
       _focusAdapter = focusAdapter ?? ((_) {}),
       _selectionCopier = selectionCopier ?? ((_) async {});

  final FlutterBridge _bridge;
  final PageDecoder _decoder;
  final NoteEditor _noteEditor;
  final ReaderFocusAdapter _focusAdapter;
  final SelectionCopier _selectionCopier;

  ReaderModel _model = ReaderModel();
  BigInt? _activeCancellation;
  final Set<BigInt> _relayoutCancellations = {};
  final Set<BigInt> _annotationCancellations = {};
  final Map<BigInt, int> _selectionCancellations = {};
  final Set<int> _cancelledSelectionCreates = {};
  int _activeBridgeOperations = 0;
  int _annotationRevision = 0;
  int _layoutRevision = 0;
  int _selectionRevision = 0;
  int _nextOperationId = 0;
  int _noteRevision = 0;
  ReaderLayout _requestedLayout = const ReaderLayout();
  ReaderLayout? _failedLayout;
  bool _closing = false;
  bool _listenersDisposed = false;
  final Set<VoidCallback> _listeners = {};

  ReaderModel get model => _model;

  @override
  void addListener(VoidCallback listener) {
    if (!_listenersDisposed) _listeners.add(listener);
  }

  @override
  void removeListener(VoidCallback listener) {
    _listeners.remove(listener);
  }

  void dispatch(ReaderMessage message) {
    switch (message) {
      case ReaderOpenRequested():
        _openRequested(message);
      case ReaderLayoutChanged():
        _layoutChanged(message.layout);
      case ReaderSelectionStarted():
        _selectionStarted(message.offset);
      case ReaderSelectionExtended():
        _selectionExtended(message.offset);
      case ReaderSelectionPointerStarted():
        _selectionPointerStarted(
          message.pointer,
          message.offset,
          message.rangeStart,
          message.rangeEnd,
          message.x,
          message.y,
        );
      case ReaderSelectionPointerPressedOutside():
        if (_model.selectionPointer == null) {
          _selectionCancelled();
        }
      case ReaderSelectionPointerMoved():
        _selectionPointerMoved(
          message.pointer,
          message.offset,
          message.x,
          message.y,
        );
      case ReaderSelectionPointerEnded():
        _selectionPointerEnded(message.pointer);
      case ReaderSelectionPointerCancelled():
        _selectionPointerCancelled(message.pointer);
      case ReaderSelectionKeyboardExtended():
        _selectionKeyboardExtended(message.movement);
      case ReaderSelectionEnded():
        _selectionEnded();
      case ReaderSelectionActionsRequested():
        if (_model.selectionPhase == ReaderSelectionPhase.selected) {
          _emit(_model.copyWith(keyboardActionInvocation: true));
          _focusAdapter(ReaderFocusTarget.actions);
        }
      case ReaderSelectionCommitted():
        _selectionCommitted(message.color, message.body);
      case ReaderSelectionNoteRequested():
        _selectionNoteRequested();
      case ReaderSelectionCopyRequested():
        _selectionCopyRequested();
      case _ReaderSelectionNoteCompleted():
        if (_isCurrent(message.generation) &&
            message.revision == _noteRevision) {
          if (message.selectionRevision == _selectionRevision) {
            _selectionCommitted(FlutterHighlightColor.yellow, message.body);
          } else {
            _emit(
              _model.copyWith(
                selectionActionError:
                    'The note was not saved because the selection or reader layout changed. Try again.',
              ),
            );
          }
        }
      case _ReaderSelectionEffectFailed():
        if (_isCurrent(message.generation) &&
            message.revision == _noteRevision &&
            message.selectionRevision == _selectionRevision) {
          _emit(_model.copyWith(selectionActionError: message.error));
        }
      case ReaderAnnotationUpdated():
        unawaited(_updateAnnotation(message));
      case ReaderAnnotationNoteRequested():
        _noteRequested(message.id);
      case _ReaderAnnotationNoteCompleted():
        _noteCompleted(message);
      case _ReaderAnnotationNoteFailed():
        if (_isCurrent(message.generation) &&
            message.revision == _noteRevision) {
          _emit(_model.copyWith(annotationError: message.error));
        }
      case ReaderAnnotationDeleted():
        unawaited(_deleteAnnotation(message.id));
      case ReaderAnnotationNavigated():
        _navigateAnnotation(message.id);
      case ReaderSelectionCancelled():
        _selectionCancelled();
      case _ReaderDocumentOpened():
        _documentOpened(message);
      case _ReaderImageDecoded():
        _imageDecoded(message);
      case _ReaderSurfaceLoaded():
        if (_isCurrent(message.generation)) {
          _emit(
            _model.copyWith(
              selectionSurface: _freezeSurface(message.surface),
              contentState: ReaderContentState.ready,
            ),
          );
        } else {
          _releaseSurface(message.surface);
        }
      case _ReaderEpubContentLoaded():
        if (_isCurrent(message.generation)) {
          _emit(
            _model.copyWith(
              selectionSurface: _freezeSurface(message.surface),
              pageImage: message.pageImage,
              contentState: ReaderContentState.ready,
            ),
          );
        } else {
          message.pageImage.dispose();
          _releaseSurface(message.surface);
        }
      case _ReaderSelectionSupportFailed():
        if (_isCurrent(message.generation)) {
          final mandatory = _model.document?.format == FlutterBookFormat.epub;
          _emit(
            _model.copyWith(
              selectionError: message.error,
              contentState: mandatory
                  ? ReaderContentState.failed
                  : _model.contentState,
              error: mandatory ? message.error : _unchanged,
            ),
          );
        }
      case _ReaderRelayoutCompleted():
        _relayoutCompleted(message);
      case _ReaderRelayoutFailed():
        if (_isCurrent(message.generation) &&
            message.revision == _layoutRevision) {
          _failedLayout = _requestedLayout;
          _emit(
            _model.copyWith(
              relayoutBusy: false,
              selectionError: 'Relayout failed: ${message.error}',
            ),
          );
        }
      case _ReaderRelayoutFinished():
        _relayoutCancellations.remove(message.cancellation);
        _bridge.releaseCancellation(id: message.cancellation);
        _activeBridgeOperations -= 1;
        _disposeBridgeIfIdle();
      case _ReaderAnnotationListFailed():
        if (_isCurrent(message.generation) &&
            message.revision == _annotationRevision) {
          _emit(_model.copyWith(annotationError: message.error));
        }
      case _ReaderAnnotationsChanged():
        _annotationsChanged(message);
      case _ReaderOpenFailed():
        _openFailed(message);
      case _ReaderOperationFinished():
        _operationFinished(message);
      case _ReaderAnnotationCreateFinished():
        final selectionRevision = _selectionCancellations[message.cancellation];
        _annotationCancellations.remove(message.cancellation);
        _selectionCancellations.remove(message.cancellation);
        if (selectionRevision != null &&
            !_selectionCancellations.containsValue(selectionRevision)) {
          _cancelledSelectionCreates.remove(selectionRevision);
        }
        _bridge.releaseCancellation(id: message.cancellation);
        _activeBridgeOperations -= 1;
        _disposeBridgeIfIdle();
      case _ReaderAnnotationOperationFinished():
        _activeBridgeOperations -= 1;
        _disposeBridgeIfIdle();
      case _ReaderDisposeRequested():
        _disposeRequested();
    }
  }

  void _openRequested(ReaderOpenRequested message) {
    final path = message.path.trim();
    if (path.isEmpty ||
        _model.busy ||
        _model.annotationOperations.isNotEmpty ||
        _closing) {
      return;
    }

    final generation = _model.generation + 1;
    for (final cancellation in _relayoutCancellations) {
      _bridge.cancel(id: cancellation);
    }
    _layoutRevision += 1;
    final openLayout = _requestedLayout;
    _failedLayout = null;
    _annotationRevision += 1;
    _releaseModelResources();
    late final BigInt cancellation;
    try {
      cancellation = _bridge.createCancellation();
    } on FlutterBridgeError catch (error) {
      _emit(
        _model.copyWith(
          error: error.message,
          generation: generation,
          relayoutBusy: false,
        ),
      );
      return;
    } catch (error) {
      _emit(
        _model.copyWith(
          error: error.toString(),
          generation: generation,
          relayoutBusy: false,
        ),
      );
      return;
    }
    _activeCancellation = cancellation;
    _activeBridgeOperations += 1;
    _emit(
      _model.copyWith(
        document: null,
        pageImage: null,
        error: null,
        selectionSurface: null,
        selectionPhase: ReaderSelectionPhase.idle,
        anchor: null,
        focus: null,
        selectionPointer: null,
        selectionVisualLine: null,
        selectionPreferredX: null,
        savedSelections: const [],
        annotations: const [],
        annotationOperations: const {},
        selectionError: null,
        selectionActionError: null,
        keyboardActionInvocation: false,
        annotationError: null,
        annotationsReady: false,
        relayoutBusy: false,
        contentState: ReaderContentState.loading,
        busy: true,
        generation: generation,
        layout: openLayout,
      ),
    );
    unawaited(_openEffect(path, generation, cancellation, openLayout));
  }

  Future<void> _openEffect(
    String path,
    int generation,
    BigInt cancellation,
    ReaderLayout layout,
  ) async {
    FlutterDocumentSummary? opened;
    try {
      opened = await _bridge.openDocument(
        request: FlutterOpenRequest(localId: path, pathKey: path),
        cancellationId: cancellation,
      );
      final document = opened;
      dispatch(
        _ReaderDocumentOpened(generation: generation, document: document),
      );
      opened = null;
      if (!_isCurrent(generation)) return;

      if (document.format != FlutterBookFormat.cbz) {
        FlutterSelectionSurface? effectSurface;
        try {
          final surface = await _bridge.selectionSurface(
            document: document.handle,
            unit: BigInt.zero,
            scale: layout.scale,
            width: layout.width,
            fontSize: layout.fontSize,
            cancellationId: cancellation,
          );
          effectSurface = surface;
          if (!_isCurrent(generation)) {
            if (surface.raster case final raster?) {
              _bridge.releaseBuffer(handle: raster.handle);
            }
            _releaseSurface(surface);
            effectSurface = null;
            return;
          }
          if (document.format == FlutterBookFormat.epub) {
            final raster = surface.raster;
            if (raster == null) {
              _releaseSurface(surface);
              effectSurface = null;
              throw StateError('EPUB selection surface is missing its raster');
            }
            late final ui.Image image;
            try {
              final pixels = _bridge.takeBuffer(handle: raster.handle);
              premultiplyRgba(pixels);
              image = await _decoder(
                pixels,
                width: raster.width,
                height: raster.height,
              );
            } finally {
              _bridge.releaseBuffer(handle: raster.handle);
            }
            if (!_isCurrent(generation)) {
              image.dispose();
              _releaseSurface(surface);
              effectSurface = null;
              return;
            }
            dispatch(
              _ReaderEpubContentLoaded(
                generation: generation,
                surface: surface,
                pageImage: image,
              ),
            );
            effectSurface = null;
          } else {
            dispatch(
              _ReaderSurfaceLoaded(generation: generation, surface: surface),
            );
            effectSurface = null;
          }
        } catch (error) {
          if (effectSurface case final surface?) _releaseSurface(surface);
          if (!_isCurrent(generation)) return;
          dispatch(_ReaderSelectionSupportFailed(generation, error.toString()));
        }
        final revision = _annotationRevision;
        try {
          final annotations = await _bridge.listAnnotations(
            document: document.handle,
            scale: layout.scale,
            cancellationId: cancellation,
          );
          if (!_isCurrent(generation)) return;
          dispatch(
            _ReaderAnnotationsChanged(
              generation,
              revision,
              null,
              null,
              annotations,
            ),
          );
        } catch (error) {
          if (!_isCurrent(generation)) return;
          dispatch(
            _ReaderAnnotationListFailed(generation, revision, error.toString()),
          );
        }
      }

      if (document.format != FlutterBookFormat.epub) {
        final rendered = await _bridge.renderPage(
          document: document.handle,
          page: BigInt.zero,
          scale: layout.scale,
          cancellationId: cancellation,
        );
        if (!_isCurrent(generation)) {
          _bridge.releaseBuffer(handle: rendered.handle);
          return;
        }
        late final ui.Image image;
        try {
          final pixels = _bridge.takeBuffer(handle: rendered.handle);
          if (document.format == FlutterBookFormat.cbz) {
            premultiplyRgba(pixels);
          }
          image = await _decoder(
            pixels,
            width: rendered.width,
            height: rendered.height,
          );
        } finally {
          _bridge.releaseBuffer(handle: rendered.handle);
        }
        if (!_isCurrent(generation)) {
          image.dispose();
          return;
        }
        dispatch(_ReaderImageDecoded(generation: generation, pageImage: image));
      }
    } on FlutterBridgeError catch (error) {
      dispatch(
        _ReaderOpenFailed(
          generation: generation,
          document: opened,
          error: error.message,
        ),
      );
      opened = null;
    } catch (error) {
      dispatch(
        _ReaderOpenFailed(
          generation: generation,
          document: opened,
          error: error.toString(),
        ),
      );
      opened = null;
    } finally {
      _bridge.releaseCancellation(id: cancellation);
      dispatch(
        _ReaderOperationFinished(
          generation: generation,
          cancellation: cancellation,
        ),
      );
    }
  }

  void _documentOpened(_ReaderDocumentOpened message) {
    if (!_isCurrent(message.generation)) {
      _bridge.releaseDocument(handle: message.document.handle);
      return;
    }
    _emit(_model.copyWith(document: message.document));
  }

  void _layoutChanged(ReaderLayout layout) {
    if (!layout.isValid || _closing) return;
    if (_model.busy) {
      _requestedLayout = layout;
      _failedLayout = null;
      return;
    }
    final document = _model.document;
    if (document == null) {
      _requestedLayout = layout;
      _emit(_model.copyWith(layout: layout));
      return;
    }
    if (layout == _model.layout && _relayoutCancellations.isNotEmpty) {
      _requestedLayout = layout;
      _failedLayout = null;
      _layoutRevision += 1;
      for (final active in _relayoutCancellations) {
        _bridge.cancel(id: active);
      }
      _emit(_model.copyWith(relayoutBusy: false, selectionError: null));
      return;
    }
    if (_model.annotationOperations.isNotEmpty) {
      _requestedLayout = layout;
      _failedLayout = null;
      return;
    }
    if (_model.contentState != ReaderContentState.ready ||
        document.format == FlutterBookFormat.cbz) {
      _requestedLayout = layout;
      _failedLayout = null;
      return;
    }
    if (layout == _requestedLayout || layout == _failedLayout) {
      return;
    }
    _startRelayout(document, layout);
  }

  void _startRelayout(FlutterDocumentSummary document, ReaderLayout layout) {
    final generation = _model.generation;
    late final BigInt cancellation;
    try {
      cancellation = _bridge.createCancellation();
    } catch (error) {
      _failedLayout = layout;
      _emit(
        _model.copyWith(selectionError: 'Relayout failed: ${error.toString()}'),
      );
      return;
    }
    _requestedLayout = layout;
    _failedLayout = null;
    for (final active in _relayoutCancellations) {
      _bridge.cancel(id: active);
    }
    final revision = ++_layoutRevision;
    _relayoutCancellations.add(cancellation);
    _activeBridgeOperations += 1;
    _selectionCancelled();
    _emit(_model.copyWith(relayoutBusy: true, selectionError: null));
    unawaited(
      _relayoutEffect(document, generation, revision, cancellation, layout),
    );
  }

  Future<void> _relayoutEffect(
    FlutterDocumentSummary document,
    int generation,
    int revision,
    BigInt cancellation,
    ReaderLayout layout,
  ) async {
    FlutterSelectionSurface? ownedSurface;
    FlutterBufferHandle? ownedRaster;
    ui.Image? ownedImage;
    try {
      final surface = await _bridge.selectionSurface(
        document: document.handle,
        unit: BigInt.zero,
        scale: layout.scale,
        width: layout.width,
        fontSize: layout.fontSize,
        cancellationId: cancellation,
      );
      ownedSurface = surface;
      ownedRaster = surface.raster?.handle;
      if (!_isCurrentLayout(generation, revision)) return;
      if (document.format == FlutterBookFormat.epub) {
        final raster = surface.raster;
        if (raster == null) {
          throw StateError('EPUB selection surface is missing its raster');
        }
        try {
          final pixels = _bridge.takeBuffer(handle: raster.handle);
          premultiplyRgba(pixels);
          ownedImage = await _decoder(
            pixels,
            width: raster.width,
            height: raster.height,
          );
        } finally {
          _bridge.releaseBuffer(handle: raster.handle);
          ownedRaster = null;
        }
      } else {
        final rendered = await _bridge.renderPage(
          document: document.handle,
          page: BigInt.zero,
          scale: layout.scale,
          cancellationId: cancellation,
        );
        try {
          if (!_isCurrentLayout(generation, revision)) return;
          final pixels = _bridge.takeBuffer(handle: rendered.handle);
          ownedImage = await _decoder(
            pixels,
            width: rendered.width,
            height: rendered.height,
          );
        } finally {
          _bridge.releaseBuffer(handle: rendered.handle);
        }
      }
      if (!_isCurrentLayout(generation, revision)) return;
      final annotations = await _bridge.listAnnotations(
        document: document.handle,
        scale: layout.scale,
        cancellationId: cancellation,
      );
      if (!_isCurrentLayout(generation, revision)) return;
      dispatch(
        _ReaderRelayoutCompleted(
          generation: generation,
          revision: revision,
          cancellation: cancellation,
          layout: layout,
          surface: surface,
          pageImage: ownedImage,
          annotations: annotations,
        ),
      );
      ownedSurface = null;
      ownedImage = null;
    } catch (error) {
      dispatch(
        _ReaderRelayoutFailed(
          generation: generation,
          revision: revision,
          error: error.toString(),
        ),
      );
    } finally {
      ownedImage?.dispose();
      if (ownedRaster case final raster?) {
        _bridge.releaseBuffer(handle: raster);
      }
      if (ownedSurface case final surface?) _releaseSurface(surface);
      dispatch(_ReaderRelayoutFinished(cancellation));
    }
  }

  void _relayoutCompleted(_ReaderRelayoutCompleted message) {
    if (!_isCurrentLayout(message.generation, message.revision) ||
        !_relayoutCancellations.contains(message.cancellation)) {
      message.pageImage.dispose();
      _releaseSurface(message.surface);
      return;
    }
    final oldImage = _model.pageImage;
    final oldSurface = _model.selectionSurface;
    _emit(
      _model.copyWith(
        pageImage: message.pageImage,
        selectionSurface: _freezeSurface(message.surface),
        annotations: message.annotations,
        savedSelections: _savedSelections(message.annotations),
        annotationsReady: true,
        annotationError: _model.annotationsReady
            ? _model.annotationError
            : null,
        layout: message.layout,
        relayoutBusy: false,
        selectionError: null,
        selectionVisualLine: null,
        selectionPreferredX: null,
      ),
    );
    oldImage?.dispose();
    if (oldSurface != null) _releaseSurface(oldSurface);
  }

  bool _isCurrentLayout(int generation, int revision) =>
      _isCurrent(generation) && revision == _layoutRevision;

  void _imageDecoded(_ReaderImageDecoded message) {
    if (!_isCurrent(message.generation)) {
      message.pageImage?.dispose();
      return;
    }
    _emit(
      _model.copyWith(
        pageImage: message.pageImage,
        contentState: ReaderContentState.ready,
      ),
    );
  }

  void _selectionStarted(int offset) {
    if (_model.selectionSurface == null || _model.relayoutBusy || _closing) {
      return;
    }
    _cancelSelectionCreates();
    _selectionRevision += 1;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.selecting,
        anchor: offset,
        focus: offset,
        selectionPointer: null,
        selectionVisualLine: null,
        selectionPreferredX: null,
        selectionActionError: null,
        keyboardActionInvocation: false,
      ),
    );
  }

  void _selectionExtended(int offset) {
    if (_model.selectionPhase != ReaderSelectionPhase.selecting) return;
    _emit(_model.copyWith(focus: offset));
  }

  void _selectionPointerStarted(
    int pointer,
    int offset,
    int? rangeStart,
    int? rangeEnd,
    double? x,
    double? y,
  ) {
    final surface = _model.selectionSurface;
    if (surface == null || _model.relayoutBusy || _closing) return;
    if (_model.selectionPointer case final owner? when owner != pointer) return;
    _focusAdapter(ReaderFocusTarget.surface);
    final anchor = _model.anchor;
    final focus = _model.focus;
    if (_model.selectionPhase == ReaderSelectionPhase.selected &&
        anchor != null &&
        focus != null &&
        (rangeStart ?? offset) >= (anchor < focus ? anchor : focus) &&
        (rangeEnd ?? offset) <= (anchor < focus ? focus : anchor)) {
      return;
    }
    final affinity = _caretNear(surface.visualLines, offset, x, y);
    _cancelSelectionCreates();
    _selectionRevision += 1;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.selecting,
        anchor: offset,
        focus: offset,
        selectionPointer: pointer,
        selectionVisualLine: affinity?.line,
        selectionPreferredX: affinity?.preferredX,
        selectionActionError: null,
      ),
    );
  }

  void _selectionPointerMoved(int pointer, int offset, double? x, double? y) {
    if (_model.selectionPhase != ReaderSelectionPhase.selecting ||
        _model.selectionPointer != pointer) {
      return;
    }
    final affinity = _caretNear(
      _model.selectionSurface!.visualLines,
      offset,
      x,
      y,
    );
    _emit(
      _model.copyWith(
        focus: offset,
        selectionVisualLine: affinity?.line,
        selectionPreferredX: affinity?.preferredX,
      ),
    );
  }

  void _selectionPointerEnded(int pointer) {
    if (_model.selectionPointer != pointer) return;
    _selectionEnded();
  }

  void _selectionPointerCancelled(int pointer) {
    if (_model.selectionPointer != pointer) return;
    _selectionCancelled();
  }

  void _selectionKeyboardExtended(ReaderSelectionMovement movement) {
    final surface = _model.selectionSurface;
    if (surface == null ||
        surface.graphemeBoundaries.length < 2 ||
        _model.relayoutBusy ||
        _closing) {
      return;
    }
    final graphemes = surface.graphemeBoundaries.toList(growable: false);

    final forward = switch (movement) {
      ReaderSelectionMovement.nextGrapheme ||
      ReaderSelectionMovement.nextWord ||
      ReaderSelectionMovement.nextLine ||
      ReaderSelectionMovement.lineEnd ||
      ReaderSelectionMovement.visualRight => true,
      _ => false,
    };
    final current = _model.focus;
    final horizontalMove = switch (movement) {
      ReaderSelectionMovement.visualLeft ||
      ReaderSelectionMovement.visualRight => _horizontalCaret(
        surface.visualLines,
        current,
        _model.selectionVisualLine,
        _model.selectionPreferredX,
        movement == ReaderSelectionMovement.visualRight,
      ),
      _ => null,
    };
    final lineMove = switch (movement) {
      ReaderSelectionMovement.previousLine ||
      ReaderSelectionMovement.nextLine => _lineOffset(
        surface.visualLines,
        current,
        _model.selectionVisualLine,
        _model.selectionPreferredX,
        forward,
      ),
      ReaderSelectionMovement.lineStart ||
      ReaderSelectionMovement.lineEnd => _currentLineEdge(
        surface.visualLines,
        current,
        _model.selectionVisualLine,
        forward,
      ),
      _ => null,
    };
    final boundaries = switch (movement) {
      ReaderSelectionMovement.previousWord ||
      ReaderSelectionMovement.nextWord => surface.wordBoundaries.toList(
        growable: false,
      ),
      _ => graphemes,
    };
    final next =
        lineMove?.offset ??
        horizontalMove?.offset ??
        switch (movement) {
          ReaderSelectionMovement.previousGrapheme ||
          ReaderSelectionMovement.nextGrapheme ||
          ReaderSelectionMovement.previousWord ||
          ReaderSelectionMovement.nextWord => _adjacentOffset(
            boundaries,
            current,
            forward,
          ),
          ReaderSelectionMovement.previousLine ||
          ReaderSelectionMovement.nextLine ||
          ReaderSelectionMovement.lineStart ||
          ReaderSelectionMovement.lineEnd ||
          ReaderSelectionMovement.visualLeft ||
          ReaderSelectionMovement.visualRight => null,
        };
    if (next == null) return;
    final anchor =
        _model.anchor ??
        horizontalMove?.origin ??
        (forward ? graphemes.first : graphemes.last);
    final affinity =
        lineMove ??
        (horizontalMove == null
            ? null
            : (
                offset: horizontalMove.offset,
                line: horizontalMove.line,
                preferredX: horizontalMove.preferredX,
              )) ??
        _caretForOffset(surface.visualLines, next, _model.selectionVisualLine);
    final vertical =
        movement == ReaderSelectionMovement.previousLine ||
        movement == ReaderSelectionMovement.nextLine;
    _cancelSelectionCreates();
    _selectionRevision += 1;
    if (anchor == next) {
      _emit(
        _model.copyWith(
          selectionPhase: ReaderSelectionPhase.idle,
          anchor: anchor,
          focus: anchor,
          selectionPointer: null,
          selectionVisualLine: affinity?.line,
          selectionPreferredX: vertical
              ? _model.selectionPreferredX ?? affinity?.preferredX
              : affinity?.preferredX,
          selectionActionError: null,
          keyboardActionInvocation: false,
        ),
      );
      return;
    }
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.selected,
        anchor: anchor,
        focus: next,
        selectionPointer: null,
        selectionVisualLine: affinity?.line,
        selectionPreferredX: vertical
            ? _model.selectionPreferredX ?? affinity?.preferredX
            : affinity?.preferredX,
        selectionActionError: null,
        keyboardActionInvocation: false,
      ),
    );
  }

  void _selectionEnded() {
    if (_model.selectionPhase != ReaderSelectionPhase.selecting) return;
    final anchor = _model.anchor;
    final focus = _model.focus;
    _emit(
      _model.copyWith(
        selectionPhase: anchor != null && focus != null && anchor != focus
            ? ReaderSelectionPhase.selected
            : ReaderSelectionPhase.idle,
        anchor: anchor,
        focus: focus,
        selectionPointer: null,
        keyboardActionInvocation: false,
      ),
    );
  }

  void _selectionNoteRequested() {
    if (_model.selectionPhase != ReaderSelectionPhase.selected ||
        _model.relayoutBusy) {
      return;
    }
    _emit(_model.copyWith(selectionActionError: null));
    final generation = _model.generation;
    final revision = ++_noteRevision;
    final selectionRevision = _selectionRevision;
    unawaited(() async {
      try {
        final body = await _noteEditor(null);
        if (body != null) {
          dispatch(
            _ReaderSelectionNoteCompleted(
              generation,
              revision,
              selectionRevision,
              body,
            ),
          );
        }
      } catch (error) {
        dispatch(
          _ReaderSelectionEffectFailed(
            generation,
            revision,
            selectionRevision,
            error.toString(),
          ),
        );
      }
    }());
  }

  void _selectionCopyRequested() {
    final text = _model.selectedText;
    if (_model.selectionPhase != ReaderSelectionPhase.selected ||
        text == null) {
      return;
    }
    _emit(_model.copyWith(selectionActionError: null));
    final generation = _model.generation;
    final revision = ++_noteRevision;
    final selectionRevision = _selectionRevision;
    unawaited(() async {
      try {
        await _selectionCopier(text);
      } catch (error) {
        dispatch(
          _ReaderSelectionEffectFailed(
            generation,
            revision,
            selectionRevision,
            error.toString(),
          ),
        );
      }
    }());
  }

  void _selectionCommitted(FlutterHighlightColor color, String? body) {
    final anchor = _model.anchor;
    final focus = _model.focus;
    if (_model.selectionPhase != ReaderSelectionPhase.selected ||
        anchor == null ||
        focus == null ||
        !_model.annotationsReady ||
        _model.busy ||
        _model.relayoutBusy) {
      return;
    }
    final selection = ReaderSelection(
      anchor < focus ? anchor : focus,
      anchor < focus ? focus : anchor,
    );
    final document = _model.document;
    if (document == null) return;
    final generation = _model.generation;
    final selectionRevision = _selectionRevision;
    if (_model.annotationOperations.isNotEmpty || _closing) return;
    final revision = ++_annotationRevision;
    _noteRevision += 1;
    final operationId = 'create:${++_nextOperationId}';
    late final BigInt cancellation;
    try {
      cancellation = _bridge.createCancellation();
    } catch (error) {
      _emit(_model.copyWith(selectionActionError: error.toString()));
      return;
    }
    _annotationCancellations.add(cancellation);
    _selectionCancellations[cancellation] = selectionRevision;
    _activeBridgeOperations += 1;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.committing,
        annotationOperations: {operationId},
        selectionActionError: null,
      ),
    );
    unawaited(() async {
      try {
        final created = await _bridge.createAnnotation(
          document: document.handle,
          unit: BigInt.zero,
          start: BigInt.from(selection.start),
          end: BigInt.from(selection.end),
          displayScale: _model.layout.scale,
          color: color,
          body: body,
          cancellationId: cancellation,
        );
        if (!_isCurrent(generation)) return;
        dispatch(
          _ReaderAnnotationsChanged(
            generation,
            revision,
            operationId,
            selectionRevision,
            [..._model.annotations, created],
          ),
        );
      } catch (error) {
        dispatch(
          _ReaderAnnotationsChanged(
            generation,
            revision,
            operationId,
            selectionRevision,
            null,
            error.toString(),
          ),
        );
      } finally {
        dispatch(_ReaderAnnotationCreateFinished(cancellation));
      }
    }());
  }

  void _setAnnotations(
    List<FlutterAnnotation> annotations, {
    bool? annotationsReady,
  }) {
    _emit(
      _model.copyWith(
        annotations: List.unmodifiable(annotations),
        annotationsReady: annotationsReady,
        savedSelections: _savedSelections(annotations),
      ),
    );
  }

  Future<void> _updateAnnotation(ReaderAnnotationUpdated message) async {
    final document = _model.document;
    if (document == null ||
        !_model.annotationsReady ||
        _model.annotationOperations.isNotEmpty ||
        _model.relayoutBusy ||
        _closing) {
      return;
    }
    final generation = _model.generation;
    late final BigInt cancellation;
    try {
      cancellation = _bridge.createCancellation();
    } on FlutterBridgeError catch (error) {
      if (_isCurrent(generation)) {
        _emit(_model.copyWith(annotationError: error.message));
      }
      return;
    } catch (error) {
      if (_isCurrent(generation)) {
        _emit(_model.copyWith(annotationError: error.toString()));
      }
      return;
    }
    final revision = ++_annotationRevision;
    final operationId = 'update:${message.id}:${++_nextOperationId}';
    _annotationCancellations.add(cancellation);
    _activeBridgeOperations += 1;
    _emit(
      _model.copyWith(
        annotationOperations: {operationId},
        annotationError: null,
      ),
    );
    try {
      final changed = await _bridge.updateAnnotation(
        document: document.handle,
        id: message.id,
        color: message.color,
        body: message.body,
      );
      if (!_isCurrent(generation)) return;
      final items = changed
          ? await _bridge.listAnnotations(
              document: document.handle,
              scale: _model.layout.scale,
              cancellationId: cancellation,
            )
          : _model.annotations;
      if (!_isCurrent(generation)) return;
      dispatch(
        _ReaderAnnotationsChanged(
          generation,
          revision,
          operationId,
          null,
          items,
        ),
      );
    } catch (error) {
      dispatch(
        _ReaderAnnotationsChanged(
          generation,
          revision,
          operationId,
          null,
          null,
          error.toString(),
        ),
      );
    } finally {
      _annotationCancellations.remove(cancellation);
      _bridge.releaseCancellation(id: cancellation);
      dispatch(const _ReaderAnnotationOperationFinished());
    }
  }

  Future<void> _deleteAnnotation(String id) async {
    final document = _model.document;
    if (document == null ||
        !_model.annotationsReady ||
        _model.annotationOperations.isNotEmpty ||
        _model.relayoutBusy ||
        _closing) {
      return;
    }
    final generation = _model.generation;
    final revision = ++_annotationRevision;
    final operationId = 'delete:$id:${++_nextOperationId}';
    _activeBridgeOperations += 1;
    _emit(
      _model.copyWith(
        annotationOperations: {operationId},
        annotationError: null,
      ),
    );
    try {
      final changed = await _bridge.deleteAnnotation(
        document: document.handle,
        id: id,
      );
      if (!_isCurrent(generation)) return;
      dispatch(
        _ReaderAnnotationsChanged(
          generation,
          revision,
          operationId,
          null,
          changed
              ? _model.annotations.where((item) => item.id != id).toList()
              : _model.annotations,
        ),
      );
    } catch (error) {
      dispatch(
        _ReaderAnnotationsChanged(
          generation,
          revision,
          operationId,
          null,
          null,
          error.toString(),
        ),
      );
    } finally {
      dispatch(const _ReaderAnnotationOperationFinished());
    }
  }

  void _noteRequested(String id) {
    if (_closing) return;
    final annotation = _model.annotations
        .where((item) => item.id == id)
        .firstOrNull;
    if (annotation == null) return;
    final generation = _model.generation;
    final revision = ++_noteRevision;
    unawaited(() async {
      try {
        final body = await _noteEditor(annotation.body);
        if (body != null) {
          dispatch(
            _ReaderAnnotationNoteCompleted(generation, revision, id, body),
          );
        }
      } catch (error) {
        dispatch(
          _ReaderAnnotationNoteFailed(generation, revision, error.toString()),
        );
      }
    }());
  }

  void _noteCompleted(_ReaderAnnotationNoteCompleted message) {
    if (!_isCurrent(message.generation) || message.revision != _noteRevision) {
      return;
    }
    final annotation = _model.annotations
        .where((item) => item.id == message.id)
        .firstOrNull;
    if (annotation == null) return;
    if (_model.relayoutBusy) {
      _emit(
        _model.copyWith(
          annotationError:
              'The note was not saved because the reader layout changed. Try again.',
        ),
      );
      return;
    }
    if (_model.annotationOperations.isNotEmpty) {
      _emit(
        _model.copyWith(
          annotationError:
              'Finish the current highlight change, then save the note again.',
        ),
      );
      return;
    }
    unawaited(
      _updateAnnotation(
        ReaderAnnotationUpdated(
          annotation.id,
          annotation.color,
          message.body.isEmpty ? null : message.body,
        ),
      ),
    );
  }

  void _annotationsChanged(_ReaderAnnotationsChanged message) {
    if (!_isCurrent(message.generation)) {
      return;
    }
    final operation = message.operationId;
    if (operation != null) {
      final pending = {..._model.annotationOperations}..remove(operation);
      _emit(_model.copyWith(annotationOperations: pending));
    }
    if (message.revision != _annotationRevision) {
      _startRequestedRelayoutIfReady();
      return;
    }
    if (message.items case final items?) {
      _setAnnotations(items, annotationsReady: operation == null ? true : null);
    }
    if (operation == null) return;
    final createsSelection = operation.startsWith('create:');
    final ownsSelection =
        createsSelection && message.selectionRevision == _selectionRevision;
    final wasCancelled = _cancelledSelectionCreates.contains(
      message.selectionRevision,
    );
    _emit(
      _model.copyWith(
        selectionPhase: ownsSelection
            ? (message.error == null
                  ? ReaderSelectionPhase.idle
                  : ReaderSelectionPhase.selected)
            : null,
        anchor: ownsSelection && message.error == null ? null : _unchanged,
        focus: ownsSelection && message.error == null ? null : _unchanged,
        selectionActionError: ownsSelection && message.error != null
            ? message.error
            : ownsSelection
            ? null
            : _unchanged,
        annotationError: message.error == null
            ? null
            : !ownsSelection && !wasCancelled
            ? createsSelection
                  ? 'An earlier highlight could not be saved: ${message.error}'
                  : message.error
            : _unchanged,
      ),
    );
    _startRequestedRelayoutIfReady();
  }

  void _startRequestedRelayoutIfReady() {
    final document = _model.document;
    if (document != null &&
        !_model.busy &&
        !_model.relayoutBusy &&
        _model.annotationOperations.isEmpty &&
        _model.contentState == ReaderContentState.ready &&
        document.format != FlutterBookFormat.cbz &&
        _requestedLayout != _model.layout &&
        _requestedLayout != _failedLayout) {
      _startRelayout(document, _requestedLayout);
    }
  }

  void _navigateAnnotation(String id) {
    final item = _model.annotations.where((item) => item.id == id).firstOrNull;
    if (item != null && item.unit == BigInt.zero) {
      _cancelSelectionCreates();
      _selectionRevision += 1;
      final range = item.textRange;
      _emit(
        _model.copyWith(
          anchor: range?.start.toInt(),
          focus: range?.end.toInt(),
          selectionPhase: range == null
              ? ReaderSelectionPhase.idle
              : ReaderSelectionPhase.selected,
          selectionPointer: null,
          selectionVisualLine: null,
          selectionPreferredX: null,
          selectionActionError: null,
          keyboardActionInvocation: false,
        ),
      );
      _focusAdapter(ReaderFocusTarget.surface);
    }
  }

  void _selectionCancelled() {
    _cancelSelectionCreates();
    _selectionRevision += 1;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.idle,
        anchor: null,
        focus: null,
        selectionPointer: null,
        selectionVisualLine: null,
        selectionPreferredX: null,
        selectionActionError: null,
        keyboardActionInvocation: false,
      ),
    );
    _focusAdapter(ReaderFocusTarget.surface);
  }

  void _cancelSelectionCreates() {
    for (final entry in _selectionCancellations.entries) {
      if (entry.value == _selectionRevision) {
        _cancelledSelectionCreates.add(entry.value);
        _bridge.cancel(id: entry.key);
      }
    }
  }

  void _openFailed(_ReaderOpenFailed message) {
    final opened = message.document;
    if (opened != null) {
      _bridge.releaseDocument(handle: opened.handle);
    }
    if (_isCurrent(message.generation)) {
      _releaseModelResources();
      _emit(
        _model.copyWith(
          document: null,
          pageImage: null,
          contentState: ReaderContentState.failed,
          error: message.error,
        ),
      );
    }
  }

  void _operationFinished(_ReaderOperationFinished message) {
    if (_activeCancellation == message.cancellation) {
      _activeCancellation = null;
    }
    if (_isCurrent(message.generation)) {
      _emit(_model.copyWith(busy: false));
      _startRequestedRelayoutIfReady();
    }
    _activeBridgeOperations -= 1;
    _disposeBridgeIfIdle();
  }

  bool _isCurrent(int generation) {
    return !_closing && generation == _model.generation;
  }

  void _emit(ReaderModel model) {
    _model = model;
    if (!_listenersDisposed) {
      for (final listener in _listeners.toList(growable: false)) {
        try {
          listener();
        } catch (error, stackTrace) {
          scheduleMicrotask(
            () => FlutterError.reportError(
              FlutterErrorDetails(
                exception: error,
                stack: stackTrace,
                library: 'shosai_flutter',
              ),
            ),
          );
        }
      }
    }
  }

  void _releaseModelResources() {
    final pageImage = _model.pageImage;
    final document = _model.document;
    final surface = _model.selectionSurface;
    _model = _model.copyWith(
      document: null,
      pageImage: null,
      selectionSurface: null,
      selectionPhase: ReaderSelectionPhase.idle,
      anchor: null,
      focus: null,
      selectionPointer: null,
      selectionVisualLine: null,
      selectionPreferredX: null,
    );
    pageImage?.dispose();
    if (surface != null) _releaseSurface(surface);
    if (document != null) {
      _bridge.releaseDocument(handle: document.handle);
    }
  }

  void _releaseSurface(FlutterSelectionSurface surface) {
    _bridge.releaseSelection(handle: surface.handle);
  }

  void _disposeRequested() {
    if (_closing) return;
    _closing = true;
    final cancellation = _activeCancellation;
    if (cancellation != null) {
      _bridge.cancel(id: cancellation);
    }
    for (final cancellation in _annotationCancellations) {
      _bridge.cancel(id: cancellation);
    }
    for (final cancellation in _relayoutCancellations) {
      _bridge.cancel(id: cancellation);
    }
    _model = _model.copyWith(busy: false, generation: _model.generation + 1);
    _disposeBridgeIfIdle();
  }

  void _disposeBridgeIfIdle() {
    if (_closing && _activeBridgeOperations == 0 && !_bridge.isDisposed) {
      _releaseModelResources();
      _bridge.dispose();
    }
    if (_closing && _activeBridgeOperations == 0 && !_listenersDisposed) {
      _listenersDisposed = true;
      _listeners.clear();
    }
  }

  void dispose() {
    dispatch(const _ReaderDisposeRequested());
  }
}

int? _adjacentOffset(List<int> offsets, int? current, bool forward) {
  if (current == null) {
    return forward ? offsets[1] : offsets[offsets.length - 2];
  }
  if (forward) {
    for (final offset in offsets) {
      if (offset > current) return offset;
    }
  } else {
    for (final offset in offsets.reversed) {
      if (offset < current) return offset;
    }
  }
  return null;
}

({int offset, int origin, int line, double preferredX})? _horizontalCaret(
  List<FlutterSelectionVisualLine> lines,
  int? current,
  int? currentLine,
  double? currentX,
  bool right,
) {
  if (lines.isEmpty) return null;
  var line = currentLine;
  var index = -1;
  if (current != null) {
    final candidateLines = <int>[
      if (line != null && line >= 0 && line < lines.length) line,
      for (var candidate = 0; candidate < lines.length; candidate += 1)
        if (candidate != line) candidate,
    ];
    for (final candidateLine in candidateLines) {
      final carets = lines[candidateLine].carets;
      for (var candidate = 0; candidate < carets.length; candidate += 1) {
        if (carets[candidate].offset.toInt() != current) continue;
        if (index < 0 ||
            (currentX != null &&
                (carets[candidate].alongLine - currentX).abs() <
                    (carets[index].alongLine - currentX).abs())) {
          line = candidateLine;
          index = candidate;
        }
      }
      if (index >= 0) break;
    }
  } else {
    line = _navigableLine(lines, right);
    if (line == null) return null;
    index = right ? 0 : lines[line].carets.length - 1;
  }
  if (line == null || index < 0) return null;
  final carets = lines[line].carets;
  final destination = index + (right ? 1 : -1);
  final origin = carets[index];
  FlutterSelectionCaret next;
  var destinationLine = line;
  if (destination >= 0 && destination < carets.length) {
    next = carets[destination];
  } else {
    do {
      destinationLine += right ? 1 : -1;
      if (destinationLine < 0 || destinationLine >= lines.length) return null;
    } while (lines[destinationLine].carets.isEmpty);
    final destinationCarets = lines[destinationLine].carets;
    next = right ? destinationCarets.first : destinationCarets.last;
  }
  return (
    offset: next.offset.toInt(),
    origin: origin.offset.toInt(),
    line: destinationLine,
    preferredX: next.alongLine,
  );
}

({int offset, int line, double preferredX})? _lineOffset(
  List<FlutterSelectionVisualLine> lines,
  int? current,
  int? currentLine,
  double? preferredX,
  bool forward,
) {
  if (lines.isEmpty) return null;
  final originLine = _navigableLine(lines, forward);
  final origin = current == null
      ? originLine == null
            ? null
            : _lineEdge(lines, originLine, forward)
      : _caretForOffset(lines, current, currentLine);
  if (origin == null) return null;
  var destinationLine = origin.line;
  do {
    destinationLine += forward ? 1 : -1;
    if (destinationLine < 0 || destinationLine >= lines.length) return null;
  } while (lines[destinationLine].carets.isEmpty);
  final carets = lines[destinationLine].carets;
  final targetX = preferredX ?? origin.preferredX;
  final caret = carets.reduce(
    (best, candidate) =>
        (candidate.alongLine - targetX).abs() < (best.alongLine - targetX).abs()
        ? candidate
        : best,
  );
  return (
    offset: caret.offset.toInt(),
    line: destinationLine,
    preferredX: targetX,
  );
}

({int offset, int line, double preferredX})? _currentLineEdge(
  List<FlutterSelectionVisualLine> lines,
  int? current,
  int? currentLine,
  bool forward,
) {
  if (lines.isEmpty) return null;
  if (current == null) {
    final line = _navigableLine(lines, forward);
    return line == null ? null : _lineEdge(lines, line, !forward);
  }
  final origin = _caretForOffset(lines, current, currentLine);
  if (origin == null) return null;
  return _lineEdge(lines, origin.line, !forward);
}

int? _navigableLine(List<FlutterSelectionVisualLine> lines, bool forward) {
  final indexes = forward
      ? Iterable<int>.generate(lines.length)
      : Iterable<int>.generate(
          lines.length,
          (index) => lines.length - index - 1,
        );
  for (final index in indexes) {
    if (lines[index].carets.isNotEmpty) return index;
  }
  return null;
}

({int offset, int line, double preferredX})? _caretForOffset(
  List<FlutterSelectionVisualLine> lines,
  int offset,
  int? preferredLine,
) {
  if (preferredLine != null &&
      preferredLine >= 0 &&
      preferredLine < lines.length) {
    for (final caret in lines[preferredLine].carets) {
      if (caret.offset.toInt() == offset) {
        return (
          offset: offset,
          line: preferredLine,
          preferredX: caret.alongLine,
        );
      }
    }
  }
  for (var line = 0; line < lines.length; line += 1) {
    for (final caret in lines[line].carets) {
      if (caret.offset.toInt() == offset) {
        return (offset: offset, line: line, preferredX: caret.alongLine);
      }
    }
  }
  return null;
}

({int offset, int line, double preferredX})? _caretNear(
  List<FlutterSelectionVisualLine> lines,
  int offset,
  double? x,
  double? y,
) {
  if (x == null || y == null) return _caretForOffset(lines, offset, null);
  ({int offset, int line, double preferredX})? best;
  double? bestDistance;
  for (var line = 0; line < lines.length; line += 1) {
    for (final caret in lines[line].carets) {
      if (caret.offset.toInt() != offset) continue;
      final dy = caret.vertical
          ? caret.alongLine - y
          : y.clamp(caret.top, caret.bottom) - y;
      final dx = caret.x - x;
      final distance = dx * dx + dy * dy;
      if (bestDistance == null || distance < bestDistance) {
        best = (offset: offset, line: line, preferredX: caret.alongLine);
        bestDistance = distance;
      }
    }
  }
  return best;
}

({int offset, int line, double preferredX})? _lineEdge(
  List<FlutterSelectionVisualLine> lines,
  int line,
  bool forward,
) {
  final carets = lines[line].carets;
  if (carets.isEmpty) return null;
  final caret = forward ? carets.first : carets.last;
  return (
    offset: caret.offset.toInt(),
    line: line,
    preferredX: caret.alongLine,
  );
}

List<ReaderSelection> _savedSelections(List<FlutterAnnotation> annotations) =>
    List.unmodifiable(
      annotations
          .where((item) => item.unit == BigInt.zero && item.textRange != null)
          .map((item) {
            final range = item.textRange!;
            return ReaderSelection(
              range.start.toInt(),
              range.end.toInt(),
              item.color,
            );
          }),
    );

FlutterSelectionSurface _freezeSurface(FlutterSelectionSurface surface) {
  if (_frozenSurfaces[surface] ?? false) return surface;
  final frozen = FlutterSelectionSurface(
    handle: surface.handle,
    width: surface.width,
    height: surface.height,
    text: surface.text,
    copyEligible: surface.copyEligible,
    resourcePath: surface.resourcePath,
    raster: surface.raster,
    endpoints: List.unmodifiable(surface.endpoints),
    graphemeBoundaries: Uint32List.fromList(
      surface.graphemeBoundaries.toList(growable: false),
    ).asUnmodifiableView(),
    wordBoundaries: Uint32List.fromList(
      surface.wordBoundaries.toList(growable: false),
    ).asUnmodifiableView(),
    visualLines: List.unmodifiable(
      surface.visualLines.map(
        (line) =>
            FlutterSelectionVisualLine(carets: List.unmodifiable(line.carets)),
      ),
    ),
  );
  _frozenSurfaces[frozen] = true;
  return frozen;
}

FlutterAnnotation _freezeAnnotation(FlutterAnnotation annotation) {
  if (_frozenAnnotations[annotation] ?? false) return annotation;
  final frozen = FlutterAnnotation(
    id: annotation.id,
    unit: annotation.unit,
    resolution: annotation.resolution,
    textRange: annotation.textRange,
    quote: annotation.quote,
    rectangles: annotation.rectangles == null
        ? null
        : List.unmodifiable(annotation.rectangles!),
    color: annotation.color,
    body: annotation.body,
  );
  _frozenAnnotations[frozen] = true;
  return frozen;
}

Uint8List premultiplyRgba(Uint8List pixels) {
  for (var offset = 0; offset < pixels.length; offset += 4) {
    final alpha = pixels[offset + 3];
    pixels[offset] = (pixels[offset] * alpha + 127) ~/ 255;
    pixels[offset + 1] = (pixels[offset + 1] * alpha + 127) ~/ 255;
    pixels[offset + 2] = (pixels[offset + 2] * alpha + 127) ~/ 255;
  }
  return pixels;
}
