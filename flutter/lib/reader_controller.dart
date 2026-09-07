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

const _unchanged = Object();

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
    List<ReaderSelection> savedSelections = const [],
    List<FlutterAnnotation> annotations = const [],
    Set<String> annotationOperations = const {},
    this.selectionError,
    this.annotationError,
    this.annotationsReady = false,
    this.contentState = ReaderContentState.loading,
    this.error,
    this.busy = false,
    this.generation = 0,
  }) : selectionSurface = selectionSurface == null
           ? null
           : _freezeSurface(selectionSurface),
       savedSelections = List.unmodifiable(savedSelections),
       annotations = List.unmodifiable(annotations),
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
  final List<ReaderSelection> savedSelections;
  final List<FlutterAnnotation> annotations;
  final Set<String> annotationOperations;
  final String? selectionError;
  final String? annotationError;
  final bool annotationsReady;
  final ReaderContentState contentState;
  final String? error;
  final bool busy;
  final int generation;

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
    List<ReaderSelection>? savedSelections,
    List<FlutterAnnotation>? annotations,
    Set<String>? annotationOperations,
    Object? selectionError = _unchanged,
    Object? annotationError = _unchanged,
    bool? annotationsReady,
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
      annotationError: identical(annotationError, _unchanged)
          ? this.annotationError
          : annotationError as String?,
      annotationsReady: annotationsReady ?? this.annotationsReady,
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

final class ReaderSelectionStarted extends ReaderMessage {
  const ReaderSelectionStarted(this.offset);
  final int offset;
}

final class ReaderSelectionExtended extends ReaderMessage {
  const ReaderSelectionExtended(this.offset);
  final int offset;
}

final class ReaderSelectionPointerStarted extends ReaderMessage {
  const ReaderSelectionPointerStarted(this.pointer, this.offset);
  final int pointer;
  final int offset;
}

final class ReaderSelectionPointerMoved extends ReaderMessage {
  const ReaderSelectionPointerMoved(this.pointer, this.offset);
  final int pointer;
  final int offset;
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
  const ReaderSelectionCommitted();
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
  }) : _bridge = bridge,
       _decoder = decoder,
       _noteEditor = noteEditor ?? ((_) async => null),
       _focusAdapter = focusAdapter ?? ((_) {});

  final FlutterBridge _bridge;
  final PageDecoder _decoder;
  final NoteEditor _noteEditor;
  final ReaderFocusAdapter _focusAdapter;

  ReaderModel _model = ReaderModel();
  BigInt? _activeCancellation;
  final Set<BigInt> _annotationCancellations = {};
  int _activeBridgeOperations = 0;
  int _annotationRevision = 0;
  int _selectionRevision = 0;
  int _nextOperationId = 0;
  int _noteRevision = 0;
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
      case ReaderSelectionStarted():
        _selectionStarted(message.offset);
      case ReaderSelectionExtended():
        _selectionExtended(message.offset);
      case ReaderSelectionPointerStarted():
        _selectionPointerStarted(message.pointer, message.offset);
      case ReaderSelectionPointerMoved():
        _selectionPointerMoved(message.pointer, message.offset);
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
          _focusAdapter(ReaderFocusTarget.actions);
        }
      case ReaderSelectionCommitted():
        _selectionCommitted();
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
        _annotationCancellations.remove(message.cancellation);
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
    _annotationRevision += 1;
    _releaseModelResources();
    late final BigInt cancellation;
    try {
      cancellation = _bridge.createCancellation();
    } on FlutterBridgeError catch (error) {
      _emit(_model.copyWith(error: error.message, generation: generation));
      return;
    } catch (error) {
      _emit(_model.copyWith(error: error.toString(), generation: generation));
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
        annotationError: null,
        annotationsReady: false,
        contentState: ReaderContentState.loading,
        busy: true,
        generation: generation,
      ),
    );
    unawaited(_openEffect(path, generation, cancellation));
  }

  Future<void> _openEffect(
    String path,
    int generation,
    BigInt cancellation,
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
            scale: 1,
            width: 680,
            fontSize: 18,
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
          scale: 1,
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
    if (_model.selectionSurface == null || _closing) return;
    _selectionRevision += 1;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.selecting,
        anchor: offset,
        focus: offset,
        selectionPointer: null,
        selectionVisualLine: null,
        selectionPreferredX: null,
      ),
    );
  }

  void _selectionExtended(int offset) {
    if (_model.selectionPhase != ReaderSelectionPhase.selecting) return;
    _emit(_model.copyWith(focus: offset));
  }

  void _selectionPointerStarted(int pointer, int offset) {
    if (_model.selectionSurface == null || _closing) return;
    _focusAdapter(ReaderFocusTarget.surface);
    _selectionRevision += 1;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.selecting,
        anchor: offset,
        focus: offset,
        selectionPointer: pointer,
        selectionVisualLine: null,
        selectionPreferredX: null,
      ),
    );
  }

  void _selectionPointerMoved(int pointer, int offset) {
    if (_model.selectionPhase != ReaderSelectionPhase.selecting ||
        _model.selectionPointer != pointer) {
      return;
    }
    _emit(_model.copyWith(focus: offset));
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
    if (surface == null || surface.graphemeBoundaries.length < 2 || _closing) {
      return;
    }
    final graphemes = surface.graphemeBoundaries.toList(growable: false);

    final forward = switch (movement) {
      ReaderSelectionMovement.nextGrapheme ||
      ReaderSelectionMovement.nextWord ||
      ReaderSelectionMovement.nextLine => true,
      _ => false,
    };
    final current = _model.focus;
    final lineMove = switch (movement) {
      ReaderSelectionMovement.previousLine ||
      ReaderSelectionMovement.nextLine => _lineOffset(
        surface.visualLines,
        current,
        _model.selectionVisualLine,
        _model.selectionPreferredX,
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
          ReaderSelectionMovement.nextLine => null,
        };
    if (next == null) return;
    final anchor =
        _model.anchor ?? (forward ? graphemes.first : graphemes.last);
    final affinity =
        lineMove ??
        _caretForOffset(surface.visualLines, next, _model.selectionVisualLine);
    final vertical =
        movement == ReaderSelectionMovement.previousLine ||
        movement == ReaderSelectionMovement.nextLine;
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
        anchor: anchor == focus ? null : anchor,
        focus: anchor == focus ? null : focus,
        selectionPointer: null,
      ),
    );
  }

  void _selectionCommitted() {
    final anchor = _model.anchor;
    final focus = _model.focus;
    if (_model.selectionPhase != ReaderSelectionPhase.selected ||
        anchor == null ||
        focus == null ||
        !_model.annotationsReady ||
        _model.busy) {
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
    final operationId = 'create:${++_nextOperationId}';
    late final BigInt cancellation;
    try {
      cancellation = _bridge.createCancellation();
    } catch (error) {
      _emit(_model.copyWith(selectionError: error.toString()));
      return;
    }
    _annotationCancellations.add(cancellation);
    _activeBridgeOperations += 1;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.committing,
        annotationOperations: {operationId},
      ),
    );
    unawaited(() async {
      try {
        final created = await _bridge.createAnnotation(
          document: document.handle,
          unit: BigInt.zero,
          start: BigInt.from(selection.start),
          end: BigInt.from(selection.end),
          color: FlutterHighlightColor.yellow,
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
        savedSelections: List.unmodifiable(
          annotations
              .where((item) => item.unit == BigInt.zero)
              .map(
                (item) => ReaderSelection(
                  item.start.toInt(),
                  item.end.toInt(),
                  item.color,
                ),
              ),
        ),
      ),
    );
  }

  Future<void> _updateAnnotation(ReaderAnnotationUpdated message) async {
    final document = _model.document;
    if (document == null ||
        !_model.annotationsReady ||
        _model.annotationOperations.isNotEmpty ||
        _closing) {
      return;
    }
    final generation = _model.generation;
    final revision = ++_annotationRevision;
    final operationId = 'update:${message.id}:${++_nextOperationId}';
    _activeBridgeOperations += 1;
    _emit(_model.copyWith(annotationOperations: {operationId}));
    try {
      final changed = await _bridge.updateAnnotation(
        document: document.handle,
        id: message.id,
        color: message.color,
        body: message.body,
      );
      if (!_isCurrent(generation)) return;
      final items = changed
          ? await _bridge.listAnnotations(document: document.handle)
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
      dispatch(const _ReaderAnnotationOperationFinished());
    }
  }

  Future<void> _deleteAnnotation(String id) async {
    final document = _model.document;
    if (document == null ||
        !_model.annotationsReady ||
        _model.annotationOperations.isNotEmpty ||
        _closing) {
      return;
    }
    final generation = _model.generation;
    final revision = ++_annotationRevision;
    final operationId = 'delete:$id:${++_nextOperationId}';
    _activeBridgeOperations += 1;
    _emit(_model.copyWith(annotationOperations: {operationId}));
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
    if (message.revision != _annotationRevision) return;
    if (message.items case final items?) {
      _setAnnotations(items, annotationsReady: operation == null ? true : null);
    }
    if (operation == null) return;
    final ownsSelection =
        operation.startsWith('create:') &&
        message.selectionRevision == _selectionRevision;
    _emit(
      _model.copyWith(
        selectionPhase: ownsSelection
            ? (message.error == null
                  ? ReaderSelectionPhase.idle
                  : ReaderSelectionPhase.selected)
            : null,
        anchor: ownsSelection && message.error == null ? null : _unchanged,
        focus: ownsSelection && message.error == null ? null : _unchanged,
        selectionError: ownsSelection && message.error != null
            ? message.error
            : _unchanged,
        annotationError: !ownsSelection && message.error != null
            ? 'An earlier highlight could not be saved: ${message.error}'
            : _unchanged,
      ),
    );
  }

  void _navigateAnnotation(String id) {
    final item = _model.annotations.where((item) => item.id == id).firstOrNull;
    if (item != null && item.unit == BigInt.zero) {
      _selectionRevision += 1;
      _emit(
        _model.copyWith(
          anchor: item.start.toInt(),
          focus: item.end.toInt(),
          selectionPhase: ReaderSelectionPhase.selected,
          selectionPointer: null,
          selectionVisualLine: null,
          selectionPreferredX: null,
        ),
      );
    }
  }

  void _selectionCancelled() {
    _selectionRevision += 1;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.idle,
        anchor: null,
        focus: null,
        selectionPointer: null,
        selectionVisualLine: null,
        selectionPreferredX: null,
      ),
    );
    _focusAdapter(ReaderFocusTarget.surface);
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

({int offset, int line, double preferredX})? _lineOffset(
  List<FlutterSelectionVisualLine> lines,
  int? current,
  int? currentLine,
  double? preferredX,
  bool forward,
) {
  if (lines.isEmpty) return null;
  final origin = current == null
      ? _lineEdge(lines, forward ? 0 : lines.length - 1, forward)
      : _caretForOffset(lines, current, currentLine);
  if (origin == null) return null;
  final destinationLine = origin.line + (forward ? 1 : -1);
  if (destinationLine < 0 || destinationLine >= lines.length) return null;
  final carets = lines[destinationLine].carets;
  if (carets.isEmpty) return null;
  final targetX = preferredX ?? origin.preferredX;
  final caret = carets.reduce(
    (best, candidate) =>
        (candidate.x - targetX).abs() < (best.x - targetX).abs()
        ? candidate
        : best,
  );
  return (
    offset: caret.offset.toInt(),
    line: destinationLine,
    preferredX: targetX,
  );
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
        return (offset: offset, line: preferredLine, preferredX: caret.x);
      }
    }
  }
  for (var line = 0; line < lines.length; line += 1) {
    for (final caret in lines[line].carets) {
      if (caret.offset.toInt() == offset) {
        return (offset: offset, line: line, preferredX: caret.x);
      }
    }
  }
  return null;
}

({int offset, int line, double preferredX})? _lineEdge(
  List<FlutterSelectionVisualLine> lines,
  int line,
  bool forward,
) {
  final carets = lines[line].carets;
  if (carets.isEmpty) return null;
  final caret = forward ? carets.first : carets.last;
  return (offset: caret.offset.toInt(), line: line, preferredX: caret.x);
}

FlutterSelectionSurface _freezeSurface(FlutterSelectionSurface surface) =>
    FlutterSelectionSurface(
      handle: surface.handle,
      width: surface.width,
      height: surface.height,
      text: surface.text,
      resourcePath: surface.resourcePath,
      raster: surface.raster,
      endpoints: List.unmodifiable(surface.endpoints),
      graphemeBoundaries: Uint32List.fromList(
        surface.graphemeBoundaries.toList(growable: false),
      ),
      wordBoundaries: Uint32List.fromList(
        surface.wordBoundaries.toList(growable: false),
      ),
      visualLines: List.unmodifiable(
        surface.visualLines.map(
          (line) => FlutterSelectionVisualLine(
            carets: List.unmodifiable(line.carets),
          ),
        ),
      ),
    );

Uint8List premultiplyRgba(Uint8List pixels) {
  for (var offset = 0; offset < pixels.length; offset += 4) {
    final alpha = pixels[offset + 3];
    pixels[offset] = (pixels[offset] * alpha + 127) ~/ 255;
    pixels[offset + 1] = (pixels[offset + 1] * alpha + 127) ~/ 255;
    pixels[offset + 2] = (pixels[offset + 2] * alpha + 127) ~/ 255;
  }
  return pixels;
}
