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

const _unchanged = Object();

final class ReaderModel {
  const ReaderModel({
    this.document,
    this.pageImage,
    this.selectionSurface,
    this.selectionPhase = ReaderSelectionPhase.idle,
    this.anchor,
    this.focus,
    this.savedSelections = const [],
    this.annotations = const [],
    this.error,
    this.busy = false,
    this.generation = 0,
  });

  final FlutterDocumentSummary? document;
  final ui.Image? pageImage;
  final FlutterSelectionSurface? selectionSurface;
  final ReaderSelectionPhase selectionPhase;
  final int? anchor;
  final int? focus;
  final List<ReaderSelection> savedSelections;
  final List<FlutterAnnotation> annotations;
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
    List<ReaderSelection>? savedSelections,
    List<FlutterAnnotation>? annotations,
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
      savedSelections: savedSelections ?? this.savedSelections,
      annotations: annotations ?? this.annotations,
      error: identical(error, _unchanged) ? this.error : error as String?,
      busy: busy ?? this.busy,
      generation: generation ?? this.generation,
    );
  }
}

enum ReaderSelectionPhase { idle, selecting, selected, committing }

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

final class ReaderSelectionEnded extends ReaderMessage {
  const ReaderSelectionEnded();
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

final class _ReaderDisposeRequested extends ReaderMessage {
  const _ReaderDisposeRequested();
}

final class ReaderController implements Listenable {
  ReaderController({
    required FlutterBridge bridge,
    required PageDecoder decoder,
  }) : _bridge = bridge,
       _decoder = decoder;

  final FlutterBridge _bridge;
  final PageDecoder _decoder;

  ReaderModel _model = const ReaderModel();
  BigInt? _activeCancellation;
  int _activeBridgeOperations = 0;
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
      case ReaderSelectionEnded():
        _selectionEnded();
      case ReaderSelectionCommitted():
        _selectionCommitted();
      case ReaderAnnotationUpdated():
        unawaited(_updateAnnotation(message));
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
          _emit(_model.copyWith(selectionSurface: message.surface));
        }
      case _ReaderOpenFailed():
        _openFailed(message);
      case _ReaderOperationFinished():
        _operationFinished(message);
      case _ReaderDisposeRequested():
        _disposeRequested();
    }
  }

  void _openRequested(ReaderOpenRequested message) {
    final path = message.path.trim();
    if (path.isEmpty || _model.busy || _closing) return;

    final generation = _model.generation + 1;
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
        savedSelections: const [],
        annotations: const [],
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
        final surface = await _bridge.selectionSurface(
          document: document.handle,
          unit: BigInt.zero,
          scale: 1,
          width: 680,
          fontSize: 18,
          cancellationId: cancellation,
        );
        dispatch(
          _ReaderSurfaceLoaded(generation: generation, surface: surface),
        );
        final annotations = await _bridge.listAnnotations(
          document: document.handle,
        );
        if (_isCurrent(generation) && annotations.isNotEmpty) {
          _setAnnotations(annotations);
        }
      }

      if (document.format != FlutterBookFormat.epub) {
        final rendered = await _bridge.renderPage(
          document: document.handle,
          page: BigInt.zero,
          scale: 1,
          cancellationId: cancellation,
        );
        late final Uint8List pixels;
        try {
          pixels = _bridge.takeBuffer(handle: rendered.handle);
        } finally {
          _bridge.releaseBuffer(handle: rendered.handle);
        }
        if (document.format == FlutterBookFormat.cbz) {
          premultiplyRgba(pixels);
        }
        final image = await _decoder(
          pixels,
          width: rendered.width,
          height: rendered.height,
        );
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
    _emit(_model.copyWith(pageImage: message.pageImage));
  }

  void _selectionStarted(int offset) {
    if (_model.selectionSurface == null || _closing) return;
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.selecting,
        anchor: offset,
        focus: offset,
      ),
    );
  }

  void _selectionExtended(int offset) {
    if (_model.selectionPhase != ReaderSelectionPhase.selecting) return;
    _emit(_model.copyWith(focus: offset));
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
      ),
    );
  }

  void _selectionCommitted() {
    final anchor = _model.anchor;
    final focus = _model.focus;
    if (_model.selectionPhase != ReaderSelectionPhase.selected ||
        anchor == null ||
        focus == null) {
      return;
    }
    final selection = ReaderSelection(
      anchor < focus ? anchor : focus,
      anchor < focus ? focus : anchor,
    );
    final document = _model.document;
    if (document == null) return;
    _emit(_model.copyWith(selectionPhase: ReaderSelectionPhase.committing));
    unawaited(() async {
      try {
        final created = await _bridge.createAnnotation(
          document: document.handle,
          unit: BigInt.zero,
          start: BigInt.from(selection.start),
          end: BigInt.from(selection.end),
          color: FlutterHighlightColor.yellow,
        );
        if (!_closing && _model.document?.handle == document.handle) {
          _setAnnotations([..._model.annotations, created]);
          _emit(
            _model.copyWith(
              selectionPhase: ReaderSelectionPhase.idle,
              anchor: null,
              focus: null,
            ),
          );
        }
      } catch (error) {
        if (!_closing) {
          _emit(
            _model.copyWith(
              selectionPhase: ReaderSelectionPhase.selected,
              error: error.toString(),
            ),
          );
        }
      }
    }());
  }

  void _setAnnotations(List<FlutterAnnotation> annotations) {
    _emit(
      _model.copyWith(
        annotations: List.unmodifiable(annotations),
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
    if (document == null) return;
    try {
      final changed = await _bridge.updateAnnotation(
        document: document.handle,
        id: message.id,
        color: message.color,
        body: message.body,
      );
      if (changed && !_closing && _model.document?.handle == document.handle) {
        _setAnnotations(
          await _bridge.listAnnotations(document: document.handle),
        );
      }
    } catch (error) {
      if (!_closing) _emit(_model.copyWith(error: error.toString()));
    }
  }

  Future<void> _deleteAnnotation(String id) async {
    final document = _model.document;
    if (document == null) return;
    try {
      final changed = await _bridge.deleteAnnotation(
        document: document.handle,
        id: id,
      );
      if (changed && !_closing && _model.document?.handle == document.handle) {
        _setAnnotations(
          _model.annotations.where((item) => item.id != id).toList(),
        );
      }
    } catch (error) {
      if (!_closing) _emit(_model.copyWith(error: error.toString()));
    }
  }

  void _navigateAnnotation(String id) {
    final item = _model.annotations.where((item) => item.id == id).firstOrNull;
    if (item != null && item.unit == BigInt.zero) {
      _emit(
        _model.copyWith(
          anchor: item.start.toInt(),
          focus: item.end.toInt(),
          selectionPhase: ReaderSelectionPhase.selected,
        ),
      );
    }
  }

  void _selectionCancelled() {
    _emit(
      _model.copyWith(
        selectionPhase: ReaderSelectionPhase.idle,
        anchor: null,
        focus: null,
      ),
    );
  }

  void _openFailed(_ReaderOpenFailed message) {
    final opened = message.document;
    if (opened != null) {
      _bridge.releaseDocument(handle: opened.handle);
    }
    if (_isCurrent(message.generation)) {
      _releaseModelResources();
      _emit(
        _model.copyWith(document: null, pageImage: null, error: message.error),
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
    _model = _model.copyWith(
      document: null,
      pageImage: null,
      selectionSurface: null,
      selectionPhase: ReaderSelectionPhase.idle,
      anchor: null,
      focus: null,
    );
    pageImage?.dispose();
    if (document != null) {
      _bridge.releaseDocument(handle: document.handle);
    }
  }

  void _disposeRequested() {
    if (_closing) return;
    _closing = true;
    final cancellation = _activeCancellation;
    if (cancellation != null) {
      _bridge.cancel(id: cancellation);
    }
    _releaseModelResources();
    _model = _model.copyWith(
      document: null,
      pageImage: null,
      busy: false,
      generation: _model.generation + 1,
    );
    _disposeBridgeIfIdle();
  }

  void _disposeBridgeIfIdle() {
    if (_closing && _activeBridgeOperations == 0 && !_bridge.isDisposed) {
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

Uint8List premultiplyRgba(Uint8List pixels) {
  for (var offset = 0; offset < pixels.length; offset += 4) {
    final alpha = pixels[offset + 3];
    pixels[offset] = (pixels[offset] * alpha + 127) ~/ 255;
    pixels[offset + 1] = (pixels[offset + 1] * alpha + 127) ~/ 255;
    pixels[offset + 2] = (pixels[offset + 2] * alpha + 127) ~/ 255;
  }
  return pixels;
}
