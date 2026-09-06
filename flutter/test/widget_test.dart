import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shosai_flutter/main.dart';
import 'package:shosai_flutter/src/rust/api.dart';

final _documentHandle = FlutterDocumentHandle(
  registry: BigInt.one,
  id: BigInt.one,
);
final _bufferHandle = FlutterBufferHandle(registry: BigInt.one, id: BigInt.one);

void main() {
  testWidgets('welcome panel describes the native bridge action', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(home: Scaffold(body: WelcomePanel())),
    );

    expect(find.textContaining('generated Rust bridge'), findsOneWidget);
  });

  testWidgets('disposal waits for an in-flight open to release its handles', (
    tester,
  ) async {
    final bridge = _FakeBridge();
    await tester.pumpWidget(MaterialApp(home: ReaderScreen(bridge: bridge)));
    await tester.enterText(find.byType(TextField), '/tmp/book.epub');
    await tester.tap(find.text('Open document'));
    await tester.pump();

    await tester.pumpWidget(const SizedBox());
    bridge.openCompleter.complete(
      FlutterDocumentSummary(
        handle: _documentHandle,
        format: FlutterBookFormat.epub,
        logicalUnitCount: BigInt.one,
      ),
    );
    await bridge.disposed.future;

    expect(tester.takeException(), isNull);
    expect(bridge.releasedDocuments, [_documentHandle]);
    expect(bridge.disposeCount, 1);
    expect(bridge.events, ['cancel', 'document', 'cancellation', 'dispose']);
  });

  testWidgets('disposal waits for render decoding and ordered cleanup', (
    tester,
  ) async {
    final bridge = _FakeBridge(pixels: Uint8List.fromList([255, 64, 0, 128]))
      ..completeOpen(FlutterBookFormat.cbz);
    final decodeStarted = Completer<void>();
    final decode = Completer<ui.Image>();
    Uint8List? decodedPixels;
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) {
            decodedPixels = Uint8List.fromList(pixels);
            decodeStarted.complete();
            return decode.future;
          },
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book.cbz');
    await tester.tap(find.text('Open document'));
    await tester.pump();
    await bridge.renderStarted.future;

    await tester.pumpWidget(const SizedBox());
    bridge.renderCompleter.complete(
      FlutterRenderedBuffer(
        handle: _bufferHandle,
        width: 1,
        height: 1,
        byteLen: BigInt.from(4),
      ),
    );
    await decodeStarted.future;

    expect(tester.takeException(), isNull);
    expect(decodedPixels, [128, 32, 0, 128]);
    expect(bridge.releasedBuffers, [_bufferHandle]);
    expect(bridge.releasedCancellations, isEmpty);
    expect(bridge.disposeCount, 0);

    final recorder = ui.PictureRecorder();
    ui.Canvas(recorder).drawColor(const ui.Color(0xffffffff), ui.BlendMode.src);
    final picture = recorder.endRecording();
    final image = await picture.toImage(1, 1);
    picture.dispose();
    decode.complete(image);
    await bridge.disposed.future;

    expect(tester.takeException(), isNull);
    expect(image.debugDisposed, isTrue);
    expect(bridge.releasedDocuments, [_documentHandle]);
    expect(bridge.releasedCancellations, [BigInt.one]);
    expect(bridge.disposeCount, 1);
    expect(bridge.events, [
      'cancel',
      'document',
      'buffer',
      'cancellation',
      'dispose',
    ]);
  });

  testWidgets('disposal waits for cleanup after an in-flight error', (
    tester,
  ) async {
    final bridge = _FakeBridge();
    await tester.pumpWidget(MaterialApp(home: ReaderScreen(bridge: bridge)));
    await tester.enterText(find.byType(TextField), '/tmp/book.pdf');
    await tester.tap(find.text('Open document'));
    await tester.pump();

    await tester.pumpWidget(const SizedBox());
    bridge.openCompleter.completeError(
      const FlutterBridgeError(
        kind: FlutterBridgeErrorKind.cancelled,
        message: 'cancelled',
      ),
    );
    await bridge.disposed.future;

    expect(tester.takeException(), isNull);
    expect(bridge.releasedCancellations, [BigInt.one]);
    expect(bridge.disposeCount, 1);
    expect(bridge.events, ['cancel', 'cancellation', 'dispose']);
  });

  testWidgets('decoder failure after disposal still completes cleanup', (
    tester,
  ) async {
    final bridge = _FakeBridge()..completeOpen(FlutterBookFormat.pdf);
    final decodeStarted = Completer<void>();
    final decode = Completer<ui.Image>();
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) {
            decodeStarted.complete();
            return decode.future;
          },
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book.pdf');
    await tester.tap(find.text('Open document'));
    await tester.pump();
    await bridge.renderStarted.future;

    await tester.pumpWidget(const SizedBox());
    bridge.renderCompleter.complete(
      FlutterRenderedBuffer(
        handle: _bufferHandle,
        width: 1,
        height: 1,
        byteLen: BigInt.from(4),
      ),
    );
    await decodeStarted.future;
    decode.completeError(StateError('decode failed'));
    await bridge.disposed.future;

    expect(tester.takeException(), isNull);
    expect(bridge.releasedBuffers, [_bufferHandle]);
    expect(bridge.releasedDocuments, [_documentHandle]);
    expect(bridge.releasedCancellations, [BigInt.one]);
    expect(bridge.disposeCount, 1);
    expect(bridge.events, [
      'cancel',
      'document',
      'buffer',
      'cancellation',
      'dispose',
    ]);
  });

  test('dispatch owns reader model transitions and async completion', () async {
    final bridge = _FakeBridge();
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) {
        throw StateError('EPUB must not decode a raster');
      },
    );
    final initial = controller.model;
    final transitions = <ReaderModel>[];
    controller.addListener(() => transitions.add(controller.model));

    controller.dispatch(const ReaderOpenRequested('  /tmp/book.epub  '));

    expect(controller.model, isNot(same(initial)));
    expect(controller.model.busy, isTrue);
    expect(controller.model.generation, 1);
    expect(controller.model.document, isNull);
    expect(bridge.openRequests.single.pathKey, '/tmp/book.epub');

    controller.dispatch(const ReaderOpenRequested('/tmp/ignored.epub'));
    expect(bridge.openRequests, hasLength(1));

    bridge.completeOpen(FlutterBookFormat.epub);
    await bridge.operationFinished.future;

    expect(controller.model.busy, isFalse);
    expect(controller.model.document?.handle, _documentHandle);
    expect(controller.model.error, isNull);
    expect(transitions, hasLength(3));
    expect(transitions[0].busy, isTrue);
    expect(transitions[1].document?.handle, _documentHandle);
    expect(transitions[2].busy, isFalse);

    controller.dispose();
    await bridge.disposed.future;
    expect(bridge.events, ['cancellation', 'document', 'dispose']);
  });

  test('throwing listeners cannot interrupt effect ownership', () async {
    final bridge = _FakeBridge();
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) {
        throw StateError('EPUB must not decode a raster');
      },
    );
    final reported = <Object>[];
    final previousErrorHandler = FlutterError.onError;
    FlutterError.onError = (details) => reported.add(details.exception);
    controller.addListener(() => throw StateError('listener failed'));

    try {
      controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
      bridge.completeOpen(FlutterBookFormat.epub);
      await bridge.operationFinished.future;

      expect(controller.model.busy, isFalse);
      expect(controller.model.document?.handle, _documentHandle);
      expect(reported, hasLength(3));

      controller.dispose();
      await bridge.disposed.future;
      expect(bridge.releasedDocuments, [_documentHandle]);
      expect(bridge.releasedCancellations, [BigInt.one]);
      expect(bridge.disposeCount, 1);
    } finally {
      FlutterError.onError = previousErrorHandler;
    }
  });

  test('premultiplies translucent RGBA pixels in place', () {
    final pixels = Uint8List.fromList([
      255,
      64,
      0,
      128,
      10,
      20,
      30,
      255,
      200,
      100,
      50,
      0,
    ]);

    expect(premultiplyRgba(pixels), same(pixels));
    expect(pixels, [128, 32, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
  });
}

class _FakeBridge implements FlutterBridge {
  _FakeBridge({Uint8List? pixels})
    : pixels = pixels ?? Uint8List.fromList([255, 255, 255, 255]);

  final openCompleter = Completer<FlutterDocumentSummary>();
  final renderCompleter = Completer<FlutterRenderedBuffer>();
  final renderStarted = Completer<void>();
  final operationFinished = Completer<void>();
  final disposed = Completer<void>();
  final Uint8List pixels;
  final openRequests = <FlutterOpenRequest>[];
  final releasedDocuments = <FlutterDocumentHandle>[];
  final releasedBuffers = <FlutterBufferHandle>[];
  final releasedCancellations = <BigInt>[];
  final events = <String>[];
  var disposeCount = 0;

  void completeOpen(FlutterBookFormat format) {
    openCompleter.complete(
      FlutterDocumentSummary(
        handle: _documentHandle,
        format: format,
        logicalUnitCount: BigInt.one,
      ),
    );
  }

  void _ensureAlive() {
    if (isDisposed) throw StateError('bridge used after disposal');
  }

  @override
  bool get isDisposed => disposeCount != 0;

  @override
  void dispose() {
    disposeCount += 1;
    events.add('dispose');
    disposed.complete();
  }

  @override
  bool cancel({required BigInt id}) {
    _ensureAlive();
    events.add('cancel');
    return true;
  }

  @override
  BigInt createCancellation() {
    _ensureAlive();
    return BigInt.one;
  }

  @override
  Future<FlutterDocumentSummary> openDocument({
    required FlutterOpenRequest request,
    required BigInt cancellationId,
  }) {
    _ensureAlive();
    openRequests.add(request);
    return openCompleter.future;
  }

  @override
  bool releaseBuffer({required FlutterBufferHandle handle}) {
    _ensureAlive();
    releasedBuffers.add(handle);
    events.add('buffer');
    return true;
  }

  @override
  bool releaseCancellation({required BigInt id}) {
    _ensureAlive();
    releasedCancellations.add(id);
    events.add('cancellation');
    operationFinished.complete();
    return true;
  }

  @override
  bool releaseDocument({required FlutterDocumentHandle handle}) {
    _ensureAlive();
    releasedDocuments.add(handle);
    events.add('document');
    return true;
  }

  @override
  Future<FlutterRenderedBuffer> renderPage({
    required FlutterDocumentHandle document,
    required BigInt page,
    required double scale,
    required BigInt cancellationId,
  }) {
    _ensureAlive();
    renderStarted.complete();
    return renderCompleter.future;
  }

  @override
  Uint8List takeBuffer({required FlutterBufferHandle handle}) {
    _ensureAlive();
    return Uint8List.fromList(pixels);
  }
}
