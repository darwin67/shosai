import 'dart:async';
import 'dart:collection';
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
  testWidgets('application does not publish a desktop window title', (
    tester,
  ) async {
    late MaterialApp app;
    await tester.pumpWidget(
      Builder(
        builder: (context) {
          app = const ShosaiApp().build(context) as MaterialApp;
          return const SizedBox();
        },
      ),
    );

    expect(app.title, isEmpty);
    expect(app.onGenerateTitle, isNull);
  });

  testWidgets('welcome panel describes the native bridge action', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(home: Scaffold(body: WelcomePanel())),
    );

    expect(find.textContaining('generated Rust bridge'), findsOneWidget);
  });

  test('page colors retain strong content contrast in dark mode', () {
    final colors = pageColors(const ColorScheme.dark());
    expect(
      ThemeData.estimateBrightnessForColor(colors.background),
      isNot(ThemeData.estimateBrightnessForColor(colors.foreground)),
    );
  });

  test('page image source uses non-unit raster pixel dimensions', () async {
    final recorder = ui.PictureRecorder();
    ui.Canvas(recorder);
    final picture = recorder.endRecording();
    final image = await picture.toImage(4, 2);
    try {
      expect(pageImageSource(image), const Rect.fromLTWH(0, 0, 4, 2));
    } finally {
      image.dispose();
      picture.dispose();
    }
  });

  test('reader model recursively freezes collection inputs and copies', () {
    final endpoints = <FlutterSelectionEndpoint>[
      FlutterSelectionEndpoint(
        offset: BigInt.zero,
        rangeStart: BigInt.zero,
        rangeEnd: BigInt.one,
        rect: const FlutterSelectionRect(left: 0, top: 0, right: 1, bottom: 1),
      ),
    ];
    final annotations = <FlutterAnnotation>[_annotation('one')];
    final selections = <ReaderSelection>[const ReaderSelection(0, 1)];
    final operations = <String>{'write'};
    final model = ReaderModel(
      selectionSurface: FlutterSelectionSurface(
        handle: FlutterSelectionHandle(registry: BigInt.one, id: BigInt.one),
        width: 1,
        height: 1,
        text: 'a',
        endpoints: endpoints,
      ),
      annotations: annotations,
      savedSelections: selections,
      annotationOperations: operations,
    );
    endpoints.clear();
    annotations.clear();
    selections.clear();
    operations.clear();

    expect(model.selectionSurface!.endpoints, hasLength(1));
    expect(model.annotations, hasLength(1));
    expect(model.savedSelections, hasLength(1));
    expect(model.annotationOperations, {'write'});
    expect(
      () => model.selectionSurface!.endpoints.clear(),
      throwsUnsupportedError,
    );
    expect(() => model.annotations.clear(), throwsUnsupportedError);
    expect(() => model.savedSelections.clear(), throwsUnsupportedError);
    expect(() => model.annotationOperations.clear(), throwsUnsupportedError);
    final copy = model.copyWith();
    expect(
      () => copy.selectionSurface!.endpoints.clear(),
      throwsUnsupportedError,
    );
    expect(() => copy.annotations.clear(), throwsUnsupportedError);
  });

  testWidgets('CBZ image-only completion displays instead of spinning', (
    tester,
  ) async {
    final bridge = _FakeBridge()..completeOpen(FlutterBookFormat.cbz);
    final image = await _testImage();
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) async => image,
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book.cbz');
    await tester.tap(find.text('Open document'));
    await bridge.renderStarted.future;
    bridge.renderCompleter.complete(
      FlutterRenderedBuffer(
        handle: _bufferHandle,
        width: 1,
        height: 1,
        byteLen: BigInt.from(4),
      ),
    );
    await bridge.operationFinished.future;
    await tester.pump();
    expect(find.byType(RawImage), findsOneWidget);
    expect(find.byType(CircularProgressIndicator), findsNothing);
    await tester.pumpWidget(const SizedBox());
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

  testWidgets('disposal prevents stale render decoding and orders cleanup', (
    tester,
  ) async {
    final bridge = _FakeBridge(pixels: Uint8List.fromList([255, 64, 0, 128]))
      ..completeOpen(FlutterBookFormat.cbz);
    var decodeCalls = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) {
            decodeCalls += 1;
            return _testImage();
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
    await bridge.disposed.future;

    expect(tester.takeException(), isNull);
    expect(decodeCalls, 0);
    expect(bridge.releasedDocuments, [_documentHandle]);
    expect(bridge.releasedCancellations, [BigInt.one]);
    expect(bridge.disposeCount, 1);
    expect(bridge.events, [
      'cancel',
      'buffer',
      'cancellation',
      'document',
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

  testWidgets('disposal does not start PDF decoding after render completes', (
    tester,
  ) async {
    final bridge = _FakeBridge()..completeOpen(FlutterBookFormat.pdf);
    var decodeCalls = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) {
            decodeCalls += 1;
            return _testImage();
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
    await bridge.disposed.future;

    expect(tester.takeException(), isNull);
    expect(decodeCalls, 0);
    expect(bridge.releasedBuffers, [_bufferHandle]);
    expect(bridge.releasedDocuments, [_documentHandle]);
    expect(bridge.releasedCancellations, [BigInt.one]);
    expect(bridge.disposeCount, 1);
    expect(bridge.events, [
      'cancel',
      'buffer',
      'cancellation',
      'selection',
      'document',
      'dispose',
    ]);
  });

  for (final disposedDuringDecode in [false, true]) {
    for (final decodeFails in [false, true]) {
      test(
        'PDF buffer stays retained through ${disposedDuringDecode ? 'disposed ' : ''}${decodeFails ? 'failed' : 'successful'} decoding',
        () async {
          final bridge = _FakeBridge()..completeOpen(FlutterBookFormat.pdf);
          final decoded = Completer<ui.Image>();
          final decodeStarted = Completer<void>();
          final controller = ReaderController(
            bridge: bridge,
            decoder: (pixels, {required width, required height}) {
              decodeStarted.complete();
              return decoded.future;
            },
          );
          controller.dispatch(const ReaderOpenRequested('/tmp/book.pdf'));
          await bridge.renderStarted.future;
          bridge.renderCompleter.complete(
            FlutterRenderedBuffer(
              handle: _bufferHandle,
              width: 1,
              height: 1,
              byteLen: BigInt.from(4),
            ),
          );
          await decodeStarted.future;
          expect(bridge.releasedBuffers, isEmpty);
          if (disposedDuringDecode) {
            controller.dispose();
            expect(bridge.disposeCount, 0);
            expect(bridge.releasedBuffers, isEmpty);
          }

          if (decodeFails) {
            decoded.completeError(StateError('decode failed'));
          } else {
            decoded.complete(await _testImage());
          }
          await bridge.operationFinished.future;
          if (disposedDuringDecode) {
            await bridge.disposed.future;
          } else {
            controller.dispose();
            await bridge.disposed.future;
          }

          expect(bridge.releasedBuffers, [_bufferHandle]);
          expect(
            bridge.events.where((event) => event == 'buffer'),
            hasLength(1),
          );
          expect(
            bridge.events.indexOf('buffer'),
            lessThan(bridge.events.indexOf('cancellation')),
          );
        },
      );
    }
  }

  test(
    'EPUB raster handle remains retained until decoding completes',
    () async {
      final bridge = _FakeBridge(selectionRaster: true)
        ..completeOpen(FlutterBookFormat.epub);
      final decoded = Completer<ui.Image>();
      final controller = ReaderController(
        bridge: bridge,
        decoder: (pixels, {required width, required height}) {
          expect(width, 2);
          expect(height, 1);
          return decoded.future;
        },
      );

      controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
      await Future<void>.delayed(Duration.zero);
      expect(controller.model.contentState, ReaderContentState.loading);
      expect(controller.model.selectionSurface, isNull);
      expect(bridge.releasedBuffers, isEmpty);
      decoded.complete(await _testImage());
      await bridge.operationFinished.future;
      expect(bridge.releasedBuffers, [_bufferHandle]);
      controller.dispose();
      await bridge.disposed.future;
    },
  );

  test('EPUB raster releases exactly once on decode failure', () async {
    final bridge = _FakeBridge(selectionRaster: true)
      ..completeOpen(FlutterBookFormat.epub);
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) =>
          Future.error(StateError('EPUB decode failed')),
    );

    controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
    await bridge.operationFinished.future;
    expect(controller.model.error, contains('EPUB decode failed'));
    expect(bridge.releasedBuffers, [_bufferHandle]);
    controller.dispose();
    await bridge.disposed.future;
  });

  testWidgets('missing mandatory EPUB raster is a terminal content failure', (
    tester,
  ) async {
    final bridge = _FakeBridge()..missingSelectionRaster = true;
    bridge.completeOpen(FlutterBookFormat.epub);
    await tester.pumpWidget(MaterialApp(home: ReaderScreen(bridge: bridge)));
    await tester.enterText(find.byType(TextField), '/tmp/book.epub');
    await tester.tap(find.text('Open document'));
    await bridge.operationFinished.future;
    await tester.pump();

    expect(find.byType(CircularProgressIndicator), findsNothing);
    expect(find.textContaining('missing its raster'), findsWidgets);
    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  test(
    'pending selection completion releases ownership after disposal',
    () async {
      final bridge = _FakeBridge();
      bridge.selectionCompleter = Completer<FlutterSelectionSurface>();
      bridge.completeOpen(FlutterBookFormat.epub);
      final controller = ReaderController(
        bridge: bridge,
        decoder: (pixels, {required width, required height}) {
          fail('stale selection must not decode');
        },
      );
      controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
      await Future<void>.delayed(Duration.zero);
      controller.dispose();
      final selection = FlutterSelectionHandle(
        registry: BigInt.one,
        id: BigInt.from(9),
      );
      bridge.selectionCompleter!.complete(
        FlutterSelectionSurface(
          handle: selection,
          width: 1,
          height: 1,
          text: 'x',
          raster: FlutterRenderedBuffer(
            handle: _bufferHandle,
            width: 1,
            height: 1,
            byteLen: BigInt.from(4),
          ),
          endpoints: const [],
        ),
      );
      await bridge.disposed.future;
      expect(bridge.releasedBuffers, [_bufferHandle]);
      expect(bridge.releasedSelections, [selection]);
      expect(bridge.listCalls, 0);
      expect(bridge.renderCalls, 0);
    },
  );

  test(
    'EPUB raster releases after stale disposal decoder completion',
    () async {
      final bridge = _FakeBridge(selectionRaster: true)
        ..completeOpen(FlutterBookFormat.epub);
      final decoded = Completer<ui.Image>();
      final controller = ReaderController(
        bridge: bridge,
        decoder: (pixels, {required width, required height}) => decoded.future,
      );

      controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
      await Future<void>.delayed(Duration.zero);
      controller.dispose();
      expect(bridge.releasedBuffers, isEmpty);
      decoded.complete(await _testImage());
      await bridge.disposed.future;
      expect(bridge.releasedBuffers, [_bufferHandle]);
    },
  );

  test('dispatch owns reader model transitions and async completion', () async {
    final bridge = _FakeBridge();
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) => _testImage(),
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
    expect(transitions, hasLength(5));
    expect(transitions[0].busy, isTrue);
    expect(transitions[1].document?.handle, _documentHandle);
    expect(transitions[2].selectionSurface?.text, 'Selectable fixture text');
    expect(transitions[3].annotationsReady, isTrue);
    expect(transitions[4].busy, isFalse);

    controller.dispose();
    await bridge.disposed.future;
    expect(bridge.events, [
      'buffer',
      'cancellation',
      'selection',
      'document',
      'dispose',
    ]);
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
      await Future<void>.delayed(Duration.zero);

      expect(controller.model.busy, isFalse);
      expect(controller.model.document?.handle, _documentHandle);
      expect(reported, hasLength(5));

      controller.dispose();
      await bridge.disposed.future;
      expect(bridge.releasedDocuments, [_documentHandle]);
      expect(bridge.releasedCancellations, [BigInt.one]);
      expect(bridge.disposeCount, 1);
    } finally {
      FlutterError.onError = previousErrorHandler;
    }
  });

  test('throwing error reporters cannot interrupt effect ownership', () async {
    final bridge = _FakeBridge();
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) {
        throw StateError('EPUB must not decode a raster');
      },
    );
    final reported = <Object>[];
    final previousErrorHandler = FlutterError.onError;

    await runZonedGuarded(() async {
      FlutterError.onError = (details) => throw details.exception;
      controller.addListener(() => throw StateError('listener failed'));
      controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
      bridge.completeOpen(FlutterBookFormat.epub);
      await bridge.operationFinished.future;
      controller.dispose();
      await bridge.disposed.future;
      await Future<void>.delayed(Duration.zero);
    }, (error, stackTrace) => reported.add(error));
    FlutterError.onError = previousErrorHandler;

    expect(reported, hasLength(5));
    expect(bridge.releasedDocuments, [_documentHandle]);
    expect(bridge.releasedCancellations, [BigInt.one]);
    expect(bridge.disposeCount, 1);
  });

  test('cancellation allocation failure clears prior ownership', () async {
    final bridge = _FakeBridge();
    final image = await _testImage();
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) async => image,
    );
    controller.dispatch(const ReaderOpenRequested('/tmp/first.pdf'));
    bridge.completeOpen(FlutterBookFormat.pdf);
    await bridge.renderStarted.future;
    bridge.renderCompleter.complete(
      FlutterRenderedBuffer(
        handle: _bufferHandle,
        width: 1,
        height: 1,
        byteLen: BigInt.from(4),
      ),
    );
    await bridge.operationFinished.future;
    expect(controller.model.pageImage, same(image));

    bridge.failCancellationCreation = true;
    expect(
      () => controller.dispatch(const ReaderOpenRequested('/tmp/second.pdf')),
      returnsNormally,
    );

    expect(controller.model.document, isNull);
    expect(controller.model.pageImage, isNull);
    expect(controller.model.error, 'too many cancellation tokens');
    expect(image.debugDisposed, isTrue);
    expect(bridge.releasedDocuments, [_documentHandle]);

    controller.dispose();
    await bridge.disposed.future;
    expect(bridge.releasedDocuments, [_documentHandle]);
    expect(bridge.disposeCount, 1);
  });

  group('current operation failures release owned resources', () {
    test('open failure publishes an error', () async {
      final bridge = _FakeBridge();
      final controller = _epubController(bridge);
      controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
      bridge.openCompleter.completeError(StateError('open failed'));

      await _expectFailedOperation(controller, bridge, 'open failed');
      expect(bridge.releasedDocuments, isEmpty);
      expect(bridge.releasedBuffers, isEmpty);
    });

    test('render failure releases the document', () async {
      final bridge = _FakeBridge()..completeOpen(FlutterBookFormat.pdf);
      final controller = _epubController(bridge);
      controller.dispatch(const ReaderOpenRequested('/tmp/book.pdf'));
      await bridge.renderStarted.future;
      bridge.renderCompleter.completeError(StateError('render failed'));

      await _expectFailedOperation(controller, bridge, 'render failed');
      expect(bridge.releasedDocuments, [_documentHandle]);
      expect(bridge.releasedBuffers, isEmpty);
    });

    test('buffer transfer failure still releases the buffer', () async {
      final bridge = _FakeBridge()
        ..completeOpen(FlutterBookFormat.pdf)
        ..failBufferTransfer = true;
      final controller = _epubController(bridge);
      controller.dispatch(const ReaderOpenRequested('/tmp/book.pdf'));
      await bridge.renderStarted.future;
      bridge.renderCompleter.complete(
        FlutterRenderedBuffer(
          handle: _bufferHandle,
          width: 1,
          height: 1,
          byteLen: BigInt.from(4),
        ),
      );

      await _expectFailedOperation(controller, bridge, 'transfer failed');
      expect(bridge.releasedDocuments, [_documentHandle]);
      expect(bridge.releasedBuffers, [_bufferHandle]);
    });

    test('decode failure releases the document', () async {
      final bridge = _FakeBridge()..completeOpen(FlutterBookFormat.pdf);
      final controller = ReaderController(
        bridge: bridge,
        decoder: (pixels, {required width, required height}) {
          throw StateError('decode failed');
        },
      );
      controller.dispatch(const ReaderOpenRequested('/tmp/book.pdf'));
      await bridge.renderStarted.future;
      bridge.renderCompleter.complete(
        FlutterRenderedBuffer(
          handle: _bufferHandle,
          width: 1,
          height: 1,
          byteLen: BigInt.from(4),
        ),
      );

      await _expectFailedOperation(controller, bridge, 'decode failed');
      expect(bridge.releasedDocuments, [_documentHandle]);
      expect(bridge.releasedBuffers, [_bufferHandle]);
    });
  });

  test(
    'successful replacement and failure retry share one controller',
    () async {
      final firstDocument = FlutterDocumentHandle(
        registry: BigInt.one,
        id: BigInt.one,
      );
      final secondDocument = FlutterDocumentHandle(
        registry: BigInt.one,
        id: BigInt.from(2),
      );
      final finalDocument = FlutterDocumentHandle(
        registry: BigInt.one,
        id: BigInt.from(3),
      );
      final bridge = _SequentialBridge(
        Queue<Object>.of([
          FlutterDocumentSummary(
            handle: firstDocument,
            format: FlutterBookFormat.pdf,
            logicalUnitCount: BigInt.one,
          ),
          FlutterDocumentSummary(
            handle: secondDocument,
            format: FlutterBookFormat.epub,
            logicalUnitCount: BigInt.one,
          ),
          'replacement failed',
          FlutterDocumentSummary(
            handle: finalDocument,
            format: FlutterBookFormat.epub,
            logicalUnitCount: BigInt.one,
          ),
        ]),
      );
      final images = <ui.Image>[];
      final controller = ReaderController(
        bridge: bridge,
        decoder: (pixels, {required width, required height}) async {
          final image = await _testImage();
          images.add(image);
          return image;
        },
      );

      controller.dispatch(const ReaderOpenRequested('/tmp/first.pdf'));
      await bridge.waitForFinishedOperations(1);
      expect(controller.model.document?.handle, firstDocument);
      expect(controller.model.pageImage, same(images.single));

      controller.dispatch(const ReaderOpenRequested('/tmp/second.epub'));
      await bridge.waitForFinishedOperations(2);
      expect(controller.model.document?.handle, secondDocument);
      expect(controller.model.error, isNull);
      expect(images.first.debugDisposed, isTrue);
      expect(bridge.releasedDocuments, [firstDocument]);

      controller.dispatch(const ReaderOpenRequested('/tmp/failure.epub'));
      await bridge.waitForFinishedOperations(3);
      expect(controller.model.document, isNull);
      expect(controller.model.error, contains('replacement failed'));
      expect(bridge.releasedDocuments, [firstDocument, secondDocument]);

      controller.dispatch(const ReaderOpenRequested('/tmp/retry.epub'));
      await bridge.waitForFinishedOperations(4);
      expect(controller.model.document?.handle, finalDocument);
      expect(controller.model.error, isNull);
      expect(controller.model.generation, 4);
      expect(bridge.releasedCancellations, [
        BigInt.one,
        BigInt.from(2),
        BigInt.from(3),
        BigInt.from(4),
      ]);
      expect(bridge.releasedBuffers, hasLength(3));

      controller.dispose();
      expect(bridge.releasedDocuments, [
        firstDocument,
        secondDocument,
        finalDocument,
      ]);
      expect(bridge.disposeCount, 1);
    },
  );

  testWidgets('a rebuilt screen uses its replacement decoder', (tester) async {
    final bridge = _FakeBridge();
    final image = await _testImage();
    var firstDecoderCalls = 0;
    var secondDecoderCalls = 0;
    Future<ui.Image> firstDecoder(
      Uint8List pixels, {
      required int width,
      required int height,
    }) async {
      firstDecoderCalls += 1;
      return image;
    }

    Future<ui.Image> secondDecoder(
      Uint8List pixels, {
      required int width,
      required int height,
    }) async {
      secondDecoderCalls += 1;
      return image;
    }

    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(bridge: bridge, decoder: firstDecoder),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book.pdf');
    await tester.tap(find.text('Open document'));
    await tester.pump();
    bridge.completeOpen(FlutterBookFormat.pdf);
    await bridge.renderStarted.future;
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(bridge: bridge, decoder: secondDecoder),
      ),
    );
    bridge.renderCompleter.complete(
      FlutterRenderedBuffer(
        handle: _bufferHandle,
        width: 1,
        height: 1,
        byteLen: BigInt.from(4),
      ),
    );
    await bridge.operationFinished.future;

    expect(firstDecoderCalls, 0);
    expect(secondDecoderCalls, 1);

    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
    expect(image.debugDisposed, isTrue);
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

  test(
    'selection follows the shared interaction phases without bridge calls',
    () async {
      final bridge = _FakeBridge();
      final controller = _epubController(bridge);
      controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
      bridge.completeOpen(FlutterBookFormat.epub);
      await bridge.operationFinished.future;

      controller.dispatch(const ReaderSelectionStarted(12));
      expect(controller.model.selectionPhase, ReaderSelectionPhase.selecting);
      controller.dispatch(const ReaderSelectionExtended(4));
      controller.dispatch(const ReaderSelectionEnded());
      expect(controller.model.selectionPhase, ReaderSelectionPhase.selected);
      expect(controller.model.anchor, 12);
      expect(controller.model.focus, 4);

      controller.dispatch(const ReaderSelectionCommitted());
      expect(controller.model.selectionPhase, ReaderSelectionPhase.committing);
      await Future<void>.delayed(Duration.zero);
      expect(controller.model.selectionPhase, ReaderSelectionPhase.idle);
      expect(controller.model.savedSelections.single.start, 4);
      expect(controller.model.savedSelections.single.end, 12);
      expect(bridge.openRequests, hasLength(1));

      controller.dispose();
      await bridge.disposed.future;
    },
  );

  test('an old create cannot clear a newer selection', () async {
    final bridge = _ControlledBridge();
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/a.epub');
    bridge.createCompleter = Completer<FlutterAnnotation>();
    controller.dispatch(const ReaderSelectionStarted(1));
    controller.dispatch(const ReaderSelectionExtended(3));
    controller.dispatch(const ReaderSelectionEnded());
    controller.dispatch(const ReaderSelectionCommitted());
    controller.dispatch(const ReaderSelectionStarted(7));
    controller.dispatch(const ReaderSelectionExtended(9));
    controller.dispatch(const ReaderSelectionEnded());
    controller.dispatch(const ReaderSelectionCommitted());
    expect(bridge.createCalls, 1, reason: 'a busy save must be rejected');
    bridge.createCompleter!.complete(_annotation('old-create'));
    await Future<void>.delayed(Duration.zero);

    expect(controller.model.annotations.single.id, 'old-create');
    expect(controller.model.annotationOperations, isEmpty);
    expect(controller.model.selectionPhase, ReaderSelectionPhase.selected);
    expect(controller.model.anchor, 7);
    expect(controller.model.focus, 9);
    controller.dispose();
    await bridge.disposed.future;
  });

  for (final createFails in [false, true]) {
    test(
      'initial annotation load cannot be overtaken by a ${createFails ? 'failed' : 'successful'} save',
      () async {
        final initialList = Completer<List<FlutterAnnotation>>();
        final bridge = _ControlledBridge(initialListCompleter: initialList);
        final controller = _epubController(bridge);
        controller.dispatch(const ReaderOpenRequested('/tmp/a.epub'));
        while (bridge.selectionCalls == 0 || bridge.listCalls == 0) {
          await Future<void>.delayed(Duration.zero);
        }

        controller.dispatch(const ReaderSelectionStarted(4));
        controller.dispatch(const ReaderSelectionExtended(8));
        controller.dispatch(const ReaderSelectionEnded());
        controller.dispatch(const ReaderSelectionCommitted());
        expect(bridge.createCalls, 0);
        expect(controller.model.annotationsReady, isFalse);

        initialList.complete([_annotation('existing')]);
        await bridge.waitForOp(1);
        expect(controller.model.annotationsReady, isTrue);
        bridge.createCompleter = Completer<FlutterAnnotation>();
        controller.dispatch(const ReaderSelectionCommitted());
        expect(bridge.createCalls, 1);
        if (createFails) {
          bridge.createCompleter!.completeError(StateError('create failed'));
        } else {
          bridge.createCompleter!.complete(_annotation('created'));
        }
        await Future<void>.delayed(Duration.zero);
        await Future<void>.delayed(Duration.zero);

        expect(
          controller.model.annotations.map((item) => item.id),
          createFails
              ? orderedEquals(['existing'])
              : orderedEquals(['existing', 'created']),
        );
        expect(
          controller.model.selectionPhase,
          createFails
              ? ReaderSelectionPhase.selected
              : ReaderSelectionPhase.idle,
        );
        controller.dispose();
        await bridge.disposed.future;
      },
    );
  }

  test('reopen waits for an accepted annotation mutation', () async {
    final bridge = _ControlledBridge(
      initialAnnotations: [_annotation('one')],
      immediateLists: true,
    );
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/a.epub');
    final generation = controller.model.generation;
    bridge.updateCompleter = Completer<bool>();
    controller.dispatch(
      const ReaderAnnotationUpdated('one', FlutterHighlightColor.green, null),
    );
    controller.dispatch(const ReaderOpenRequested('/tmp/a.epub'));
    expect(controller.model.generation, generation);
    expect(bridge.openCalls, 1);

    bridge.updateCompleter!.complete(true);
    while (controller.model.annotationOperations.isNotEmpty) {
      await Future<void>.delayed(Duration.zero);
    }
    controller.dispatch(const ReaderOpenRequested('/tmp/a.epub'));
    await bridge.waitForOp(2);

    expect(bridge.openCalls, 2);
    expect(controller.model.generation, generation + 1);
    expect(
      controller.model.annotations.single.color,
      FlutterHighlightColor.green,
    );
    expect(
      controller.model.savedSelections.single.color,
      FlutterHighlightColor.green,
    );
    controller.dispose();
    await bridge.disposed.future;
  });

  test(
    'closing rejects update and delete intents while effects drain',
    () async {
      final bridge = _ControlledBridge(
        initialAnnotations: [_annotation('one')],
      );
      final controller = _epubController(bridge);
      await _openControlled(controller, bridge, '/tmp/a.epub');
      bridge.updateCompleter = Completer<bool>();
      controller.dispatch(
        const ReaderAnnotationUpdated('one', FlutterHighlightColor.green, null),
      );
      expect(bridge.updateCalls, 1);
      controller.dispose();
      controller.dispatch(
        const ReaderAnnotationUpdated('one', FlutterHighlightColor.blue, null),
      );
      controller.dispatch(const ReaderAnnotationDeleted('one'));
      expect(bridge.updateCalls, 1);
      expect(bridge.deleteCalls, 0);
      bridge.updateCompleter!.complete(false);
      await bridge.disposed.future;
    },
  );

  test('closing alone rejects annotation writes while open drains', () async {
    final bridge = _ControlledBridge(
      format: FlutterBookFormat.pdf,
      initialAnnotations: [_annotation('one')],
    );
    final decoded = Completer<ui.Image>();
    final decodeStarted = Completer<void>();
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) {
        decodeStarted.complete();
        return decoded.future;
      },
    );
    controller.dispatch(const ReaderOpenRequested('/tmp/a.pdf'));
    await decodeStarted.future;
    expect(controller.model.document, isNotNull);
    expect(controller.model.annotationsReady, isTrue);
    expect(controller.model.annotationOperations, isEmpty);

    controller.dispose();
    controller.dispatch(
      const ReaderAnnotationUpdated('one', FlutterHighlightColor.green, null),
    );
    controller.dispatch(const ReaderAnnotationDeleted('one'));
    expect(bridge.updateCalls, 0);
    expect(bridge.deleteCalls, 0);

    decoded.complete(await _testImage());
    await bridge.disposed.future;
  });

  for (final operation in ['update', 'delete']) {
    test('rejected save does not invalidate an accepted $operation', () async {
      final bridge = _ControlledBridge(
        initialAnnotations: [_annotation('one')],
        immediateLists: true,
      );
      final controller = _epubController(bridge);
      await _openControlled(controller, bridge, '/tmp/a.epub');
      if (operation == 'update') {
        bridge.updateCompleter = Completer<bool>();
        controller.dispatch(
          const ReaderAnnotationUpdated(
            'one',
            FlutterHighlightColor.green,
            null,
          ),
        );
      } else {
        bridge.deleteCompleter = Completer<bool>();
        controller.dispatch(const ReaderAnnotationDeleted('one'));
      }
      controller.dispatch(const ReaderSelectionStarted(1));
      controller.dispatch(const ReaderSelectionExtended(3));
      controller.dispatch(const ReaderSelectionEnded());
      controller.dispatch(const ReaderSelectionCommitted());
      expect(bridge.createCalls, 0);

      if (operation == 'update') {
        bridge.updateCompleter!.complete(true);
      } else {
        bridge.deleteCompleter!.complete(true);
      }
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);
      expect(controller.model.annotationOperations, isEmpty);
      expect(controller.model.selectionPhase, ReaderSelectionPhase.selected);
      if (operation == 'update') {
        expect(
          controller.model.annotations.single.color,
          FlutterHighlightColor.green,
        );
      } else {
        expect(controller.model.annotations, isEmpty);
      }
      controller.dispose();
      await bridge.disposed.future;
    });
  }

  test(
    'note sessions reject stale results and merge the current color',
    () async {
      final bridge = _ControlledBridge(
        initialAnnotations: [_annotation('one')],
        immediateLists: true,
      )..updateCompleter = null;
      final editors = <Completer<String?>>[];
      final controller = ReaderController(
        bridge: bridge,
        decoder: (pixels, {required width, required height}) => _testImage(),
        noteEditor: (_) {
          final editor = Completer<String?>();
          editors.add(editor);
          return editor.future;
        },
      );
      await _openControlled(controller, bridge, '/tmp/a.epub');

      controller.dispatch(const ReaderAnnotationNoteRequested('one'));
      controller.dispatch(
        const ReaderAnnotationUpdated('one', FlutterHighlightColor.green, null),
      );
      await Future<void>.delayed(Duration.zero);
      expect(
        controller.model.annotations.single.color,
        FlutterHighlightColor.green,
      );
      editors.single.complete('current note');
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);
      expect(
        controller.model.annotations.single.color,
        FlutterHighlightColor.green,
      );
      expect(controller.model.annotations.single.body, 'current note');

      controller.dispatch(const ReaderAnnotationNoteRequested('one'));
      controller.dispatch(const ReaderAnnotationNoteRequested('one'));
      editors[1].complete('stale note');
      await Future<void>.delayed(Duration.zero);
      expect(controller.model.annotations.single.body, 'current note');
      editors[2].complete('new note');
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);
      expect(controller.model.annotations.single.body, 'new note');

      final updatesBeforeReplacement = bridge.updateCalls;
      controller.dispatch(const ReaderAnnotationNoteRequested('one'));
      await _openControlled(controller, bridge, '/tmp/b.epub');
      editors[3].complete('old document note');
      await Future<void>.delayed(Duration.zero);
      expect(bridge.updateCalls, updatesBeforeReplacement);

      controller.dispatch(const ReaderAnnotationNoteRequested('one'));
      controller.dispatch(const ReaderAnnotationDeleted('one'));
      await Future<void>.delayed(Duration.zero);
      editors[4].complete('after delete');
      await Future<void>.delayed(Duration.zero);
      expect(controller.model.annotations, isEmpty);

      controller.dispose();
      await bridge.disposed.future;
    },
  );

  test(
    'disposal cancels and releases an in-flight annotation create',
    () async {
      final bridge = _ControlledBridge();
      final controller = _epubController(bridge);
      await _openControlled(controller, bridge, '/tmp/a.epub');
      bridge.createCompleter = Completer<FlutterAnnotation>();
      controller.dispatch(const ReaderSelectionStarted(1));
      controller.dispatch(const ReaderSelectionExtended(3));
      controller.dispatch(const ReaderSelectionEnded());
      controller.dispatch(const ReaderSelectionCommitted());
      final createCancellation = bridge.createdCancellations.last;

      controller.dispose();
      expect(bridge.cancelled, contains(createCancellation));
      expect(bridge.disposeCount, 0);
      bridge.createCompleter!.completeError(StateError('cancelled'));
      await bridge.disposed.future;
      expect(bridge.releasedCancellations, contains(createCancellation));
      expect(bridge.events.last, 'dispose');
    },
  );

  testWidgets('annotation controls prevent overlapping writes', (tester) async {
    final bridge = _ControlledBridge(initialAnnotations: [_annotation('one')]);
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/a.epub');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();
    bridge.updateCompleter = Completer<bool>();
    await tester.tap(find.byTooltip('Change color'));
    await tester.pump();
    expect(
      tester
          .widget<IconButton>(
            find.widgetWithIcon(IconButton, Icons.palette_outlined),
          )
          .onPressed,
      isNull,
    );
    expect(
      tester
          .widget<IconButton>(
            find.widgetWithIcon(IconButton, Icons.note_alt_outlined),
          )
          .onPressed,
      isNull,
    );
    await tester.tap(find.byTooltip('Edit note'));
    expect(bridge.updateCalls, 1);
    bridge.updateCompleter!.complete(false);
    await tester.pumpAndSettle();
    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  for (final failure in ['selection', 'annotations']) {
    testWidgets('PDF $failure failure keeps the decoded page visible', (
      tester,
    ) async {
      final bridge = _ControlledBridge(
        format: FlutterBookFormat.pdf,
        selectionFailure: failure == 'selection',
        listFailure: failure == 'annotations',
      );
      final image = await _testImage();
      await tester.pumpWidget(
        MaterialApp(
          home: ReaderScreen(
            bridge: bridge,
            decoder: (pixels, {required width, required height}) async => image,
          ),
        ),
      );
      await tester.enterText(find.byType(TextField), '/tmp/a.pdf');
      await tester.tap(find.text('Open document'));
      await tester.pumpAndSettle();
      if (failure == 'selection') {
        expect(find.byType(RawImage), findsOneWidget);
      } else {
        expect(find.byType(CircularProgressIndicator), findsNothing);
        expect(
          find.bySemanticsLabel(RegExp('Selectable text')),
          findsOneWidget,
        );
      }
      expect(
        find.textContaining(
          failure == 'selection'
              ? 'Selection unavailable:'
              : 'Highlights unavailable:',
        ),
        findsOneWidget,
      );
      await tester.pumpWidget(const SizedBox());
      await bridge.disposed.future;
    });
  }

  testWidgets('note dialog dismisses through its exit transition safely', (
    tester,
  ) async {
    final bridge = _ControlledBridge(initialAnnotations: [_annotation('one')]);
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/a.epub');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Edit note'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    expect(tester.takeException(), isNull);
    expect(find.text('Highlight note'), findsNothing);
    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets('selectable PDF preserves full-color page pixels', (
    tester,
  ) async {
    final bridge = _ControlledBridge(format: FlutterBookFormat.pdf);
    final page = await _colorTestImage();
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) async => page,
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/color.pdf');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();

    final painter =
        tester
                .widget<CustomPaint>(
                  find.byWidgetPredicate(
                    (widget) =>
                        widget is CustomPaint && widget.painter is PagePainter,
                  ),
                )
                .painter!
            as PagePainter;
    expect(painter.image, same(page));
    expect(painter.recolorImage, isFalse);

    await tester.pumpWidget(const SizedBox());
    expect(bridge.disposeCount, 1);
  });

  for (final format in [FlutterBookFormat.pdf, FlutterBookFormat.epub]) {
    testWidgets(
      '$format rendered surface selects, saves, and reopens highlight',
      (tester) async {
        final bridge = _ControlledBridge(format: format);
        await tester.pumpWidget(
          MaterialApp(
            home: ReaderScreen(
              bridge: bridge,
              decoder: (pixels, {required width, required height}) =>
                  _testImage(),
            ),
          ),
        );
        await tester.enterText(find.byType(TextField), '/tmp/book');
        await tester.tap(find.text('Open document'));
        await tester.pumpAndSettle();

        final surface = find.byKey(const ValueKey('reader-selection-surface'));
        final rect = tester.getRect(surface);
        final side = rect.shortestSide;
        final surfaceTopLeft = rect.center - Offset(side / 2, side / 2);
        final beforeSelectionCalls = bridge.selectionCalls;
        final beforeRenderCalls = bridge.renderCalls;
        PagePainter painter() =>
            tester
                    .widget<CustomPaint>(
                      find.byWidgetPredicate(
                        (widget) =>
                            widget is CustomPaint &&
                            widget.painter is PagePainter,
                      ),
                    )
                    .painter!
                as PagePainter;
        expect(painter().anchor, isNull);
        expect(painter().savedSelections, isEmpty);
        await tester.dragFrom(
          surfaceTopLeft + Offset(side * .2, side * .2),
          Offset(side * .5, side * .5),
        );
        await tester.pump();

        expect(find.text('Save highlight'), findsOneWidget);
        expect(bridge.selectionCalls, beforeSelectionCalls);
        expect(bridge.renderCalls, beforeRenderCalls);
        expect(bridge.createCalls, 0);
        expect(painter().anchor, 1);
        expect(painter().focus, 8);

        await tester.tap(find.text('Save highlight'));
        await tester.pumpAndSettle();
        expect(bridge.createCalls, 1);
        expect(bridge.createdRanges.single, (BigInt.one, BigInt.from(8)));
        expect(find.text('Highlight 1'), findsOneWidget);
        expect(painter().savedSelections.single.start, 1);
        expect(painter().savedSelections.single.end, 8);

        await tester.enterText(find.byType(TextField), '/tmp/book');
        await tester.tap(find.text('Open document'));
        await tester.pumpAndSettle();
        expect(find.text('Highlight 1'), findsOneWidget);
        expect(painter().savedSelections.single.start, 1);
        expect(painter().savedSelections.single.end, 8);

        await tester.pumpWidget(const SizedBox());
        await bridge.disposed.future;
      },
    );
  }
}

FlutterAnnotation _annotation(String id) => FlutterAnnotation(
  id: id,
  unit: BigInt.zero,
  start: BigInt.one,
  end: BigInt.from(3),
  color: FlutterHighlightColor.yellow,
);

Future<void> _openControlled(
  ReaderController controller,
  _ControlledBridge bridge,
  String path,
) async {
  final target = bridge.finishedOperations + 1;
  controller.dispatch(ReaderOpenRequested(path));
  await bridge.waitForOp(target);
}

final class _ControlledBridge implements FlutterBridge {
  _ControlledBridge({
    this.format = FlutterBookFormat.epub,
    List<FlutterAnnotation> initialAnnotations = const [],
    this.selectionFailure = false,
    this.listFailure = false,
    this.immediateLists = false,
    this.initialListCompleter,
  }) : initialAnnotations = List.of(initialAnnotations),
       storedAnnotations = List.of(initialAnnotations);

  final FlutterBookFormat format;
  final List<FlutterAnnotation> initialAnnotations;
  final List<FlutterAnnotation> storedAnnotations;
  final bool selectionFailure;
  final bool listFailure;
  final bool immediateLists;
  final Completer<List<FlutterAnnotation>>? initialListCompleter;
  final disposed = Completer<void>();
  final createdCancellations = <BigInt>[];
  final releasedCancellations = <BigInt>[];
  final cancelled = <BigInt>[];
  final events = <String>[];
  Completer<bool>? updateCompleter = Completer<bool>();
  Completer<bool>? deleteCompleter;
  Completer<FlutterAnnotation>? createCompleter;
  Completer<List<FlutterAnnotation>>? pendingList;
  var listCalled = Completer<void>();
  var listCalls = 0;
  var openCalls = 0;
  var updateCalls = 0;
  var createCalls = 0;
  var deleteCalls = 0;
  var selectionCalls = 0;
  var renderCalls = 0;
  var finishedOperations = 0;
  var disposeCount = 0;
  var _nextId = BigInt.one;
  final _listedDocuments = <BigInt>{};
  final createdRanges = <(BigInt, BigInt)>[];

  Future<void> waitForOp(int count) async {
    while (finishedOperations < count) {
      await Future<void>.delayed(Duration.zero);
    }
    await Future<void>.delayed(Duration.zero);
  }

  void _alive() {
    if (isDisposed) throw StateError('bridge used after disposal');
  }

  @override
  bool get isDisposed => disposeCount != 0;

  @override
  BigInt createCancellation() {
    _alive();
    final id = _nextId;
    _nextId += BigInt.one;
    createdCancellations.add(id);
    return id;
  }

  @override
  Future<FlutterDocumentSummary> openDocument({
    required FlutterOpenRequest request,
    required BigInt cancellationId,
  }) async {
    openCalls += 1;
    return FlutterDocumentSummary(
      handle: FlutterDocumentHandle(registry: BigInt.one, id: cancellationId),
      format: format,
      logicalUnitCount: BigInt.one,
    );
  }

  @override
  Future<FlutterSelectionSurface> selectionSurface({
    required FlutterDocumentHandle document,
    required BigInt unit,
    required double scale,
    required double width,
    required double fontSize,
    required BigInt cancellationId,
  }) async {
    selectionCalls += 1;
    if (selectionFailure) throw StateError('selection failed');
    return FlutterSelectionSurface(
      handle: FlutterSelectionHandle(registry: BigInt.one, id: cancellationId),
      width: 100,
      height: 100,
      text: 'Selectable fixture text',
      raster: format == FlutterBookFormat.epub
          ? FlutterRenderedBuffer(
              handle: FlutterBufferHandle(
                registry: BigInt.one,
                id: cancellationId,
              ),
              width: 1,
              height: 1,
              byteLen: BigInt.from(4),
            )
          : null,
      endpoints: [
        FlutterSelectionEndpoint(
          offset: BigInt.one,
          rangeStart: BigInt.one,
          rangeEnd: BigInt.from(2),
          rect: const FlutterSelectionRect(
            left: 10,
            top: 10,
            right: 30,
            bottom: 30,
          ),
        ),
        FlutterSelectionEndpoint(
          offset: BigInt.from(8),
          rangeStart: BigInt.from(8),
          rangeEnd: BigInt.from(9),
          rect: const FlutterSelectionRect(
            left: 60,
            top: 60,
            right: 80,
            bottom: 80,
          ),
        ),
      ],
    );
  }

  @override
  Future<List<FlutterAnnotation>> listAnnotations({
    required FlutterDocumentHandle document,
  }) {
    listCalls += 1;
    if (listFailure) return Future.error(StateError('annotation list failed'));
    if (immediateLists) return Future.value(List.of(storedAnnotations));
    if (_listedDocuments.add(document.id)) {
      if (initialListCompleter case final completer?) {
        return completer.future;
      }
      return Future.value(List.of(storedAnnotations));
    }
    pendingList = Completer<List<FlutterAnnotation>>();
    if (!listCalled.isCompleted) listCalled.complete();
    return pendingList!.future;
  }

  @override
  Future<FlutterAnnotation> createAnnotation({
    required FlutterDocumentHandle document,
    required BigInt unit,
    required BigInt start,
    required BigInt end,
    required FlutterHighlightColor color,
    String? body,
    required BigInt cancellationId,
  }) async {
    createCalls += 1;
    createdRanges.add((start, end));
    final created =
        await (createCompleter?.future ??
            Future.value(
              FlutterAnnotation(
                id: 'created-$createCalls',
                unit: unit,
                start: start,
                end: end,
                color: color,
                body: body,
              ),
            ));
    storedAnnotations.removeWhere((item) => item.id == created.id);
    storedAnnotations.add(created);
    return created;
  }

  @override
  Future<bool> updateAnnotation({
    required FlutterDocumentHandle document,
    required String id,
    required FlutterHighlightColor color,
    String? body,
  }) async {
    updateCalls += 1;
    final changed = await (updateCompleter?.future ?? Future.value(true));
    if (changed) {
      final index = storedAnnotations.indexWhere((item) => item.id == id);
      if (index >= 0) {
        final current = storedAnnotations[index];
        storedAnnotations[index] = FlutterAnnotation(
          id: current.id,
          unit: current.unit,
          start: current.start,
          end: current.end,
          color: color,
          body: body,
        );
      }
    }
    return changed;
  }

  @override
  Future<bool> deleteAnnotation({
    required FlutterDocumentHandle document,
    required String id,
  }) async {
    deleteCalls += 1;
    final changed = await (deleteCompleter?.future ?? Future.value(true));
    if (changed) storedAnnotations.removeWhere((item) => item.id == id);
    return changed;
  }

  @override
  Future<FlutterRenderedBuffer> renderPage({
    required FlutterDocumentHandle document,
    required BigInt page,
    required double scale,
    required BigInt cancellationId,
  }) async {
    renderCalls += 1;
    return FlutterRenderedBuffer(
      handle: FlutterBufferHandle(registry: BigInt.one, id: cancellationId),
      width: 1,
      height: 1,
      byteLen: BigInt.from(4),
    );
  }

  @override
  Uint8List takeBuffer({required FlutterBufferHandle handle}) =>
      Uint8List.fromList([255, 255, 255, 255]);

  @override
  bool releaseBuffer({required FlutterBufferHandle handle}) => true;

  @override
  bool releaseSelection({required FlutterSelectionHandle handle}) => true;

  @override
  bool releaseDocument({required FlutterDocumentHandle handle}) => true;

  @override
  bool releaseCancellation({required BigInt id}) {
    _alive();
    releasedCancellations.add(id);
    finishedOperations += 1;
    events.add('release $id');
    return true;
  }

  @override
  bool cancel({required BigInt id}) {
    _alive();
    cancelled.add(id);
    events.add('cancel $id');
    return true;
  }

  @override
  void dispose() {
    disposeCount += 1;
    events.add('dispose');
    disposed.complete();
  }
}

class _FakeBridge implements FlutterBridge {
  _FakeBridge({Uint8List? pixels, this.selectionRaster = false})
    : pixels = pixels ?? Uint8List.fromList(List.filled(8, 255));

  final openCompleter = Completer<FlutterDocumentSummary>();
  final renderCompleter = Completer<FlutterRenderedBuffer>();
  final renderStarted = Completer<void>();
  final operationFinished = Completer<void>();
  final disposed = Completer<void>();
  final Uint8List pixels;
  final bool selectionRaster;
  final openRequests = <FlutterOpenRequest>[];
  final releasedDocuments = <FlutterDocumentHandle>[];
  final releasedBuffers = <FlutterBufferHandle>[];
  final releasedSelections = <FlutterSelectionHandle>[];
  final releasedCancellations = <BigInt>[];
  final events = <String>[];
  var disposeCount = 0;
  var failCancellationCreation = false;
  var failBufferTransfer = false;
  var missingSelectionRaster = false;
  var listCalls = 0;
  var renderCalls = 0;
  var updateCalls = 0;
  var deleteCalls = 0;
  Completer<FlutterSelectionSurface>? selectionCompleter;
  FlutterBookFormat? completedFormat;

  void completeOpen(FlutterBookFormat format) {
    completedFormat = format;
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
  Future<List<FlutterAnnotation>> listAnnotations({
    required FlutterDocumentHandle document,
  }) async {
    listCalls += 1;
    return const [];
  }

  @override
  Future<FlutterAnnotation> createAnnotation({
    required FlutterDocumentHandle document,
    required BigInt unit,
    required BigInt start,
    required BigInt end,
    required FlutterHighlightColor color,
    String? body,
    required BigInt cancellationId,
  }) async => FlutterAnnotation(
    id: 'annotation',
    unit: unit,
    start: start,
    end: end,
    color: color,
    body: body,
  );
  @override
  Future<bool> updateAnnotation({
    required FlutterDocumentHandle document,
    required String id,
    required FlutterHighlightColor color,
    String? body,
  }) async {
    updateCalls += 1;
    return true;
  }

  @override
  Future<bool> deleteAnnotation({
    required FlutterDocumentHandle document,
    required String id,
  }) async {
    deleteCalls += 1;
    return true;
  }

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
    if (failCancellationCreation) {
      throw const FlutterBridgeError(
        kind: FlutterBridgeErrorKind.invalidRequest,
        message: 'too many cancellation tokens',
      );
    }
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
  bool releaseSelection({required FlutterSelectionHandle handle}) {
    _ensureAlive();
    releasedSelections.add(handle);
    events.add('selection');
    return true;
  }

  @override
  bool releaseCancellation({required BigInt id}) {
    _ensureAlive();
    releasedCancellations.add(id);
    events.add('cancellation');
    if (!operationFinished.isCompleted) operationFinished.complete();
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
    renderCalls += 1;
    renderStarted.complete();
    return renderCompleter.future;
  }

  @override
  Future<FlutterSelectionSurface> selectionSurface({
    required FlutterDocumentHandle document,
    required BigInt unit,
    required double scale,
    required double width,
    required double fontSize,
    required BigInt cancellationId,
  }) async {
    if (selectionCompleter case final pending?) return pending.future;
    return FlutterSelectionSurface(
      handle: FlutterSelectionHandle(registry: BigInt.one, id: cancellationId),
      width: 100,
      height: 100,
      text: 'Selectable fixture text',
      raster:
          !missingSelectionRaster &&
              (selectionRaster || completedFormat == FlutterBookFormat.epub)
          ? FlutterRenderedBuffer(
              handle: _bufferHandle,
              width: 2,
              height: 1,
              byteLen: BigInt.from(8),
            )
          : null,
      endpoints: [],
    );
  }

  @override
  Uint8List takeBuffer({required FlutterBufferHandle handle}) {
    _ensureAlive();
    if (failBufferTransfer) throw StateError('transfer failed');
    return Uint8List.fromList(pixels);
  }
}

final class _SequentialBridge implements FlutterBridge {
  _SequentialBridge(this.openResults);

  final Queue<Object> openResults;
  final releasedDocuments = <FlutterDocumentHandle>[];
  final releasedBuffers = <FlutterBufferHandle>[];
  final releasedCancellations = <BigInt>[];
  var _nextCancellation = BigInt.one;
  var _nextBuffer = BigInt.one;
  var disposeCount = 0;
  FlutterBookFormat? currentFormat;

  Future<void> waitForFinishedOperations(int count) async {
    while (releasedCancellations.length < count) {
      await Future<void>.delayed(Duration.zero);
    }
    await Future<void>.delayed(Duration.zero);
  }

  void _ensureAlive() {
    if (isDisposed) throw StateError('bridge used after disposal');
  }

  @override
  bool get isDisposed => disposeCount != 0;

  @override
  Future<List<FlutterAnnotation>> listAnnotations({
    required FlutterDocumentHandle document,
  }) async => const [];
  @override
  Future<FlutterAnnotation> createAnnotation({
    required FlutterDocumentHandle document,
    required BigInt unit,
    required BigInt start,
    required BigInt end,
    required FlutterHighlightColor color,
    String? body,
    required BigInt cancellationId,
  }) async => FlutterAnnotation(
    id: 'annotation',
    unit: unit,
    start: start,
    end: end,
    color: color,
    body: body,
  );
  @override
  Future<bool> updateAnnotation({
    required FlutterDocumentHandle document,
    required String id,
    required FlutterHighlightColor color,
    String? body,
  }) async => true;
  @override
  Future<bool> deleteAnnotation({
    required FlutterDocumentHandle document,
    required String id,
  }) async => true;

  @override
  void dispose() => disposeCount += 1;

  @override
  bool cancel({required BigInt id}) {
    _ensureAlive();
    return true;
  }

  @override
  BigInt createCancellation() {
    _ensureAlive();
    final id = _nextCancellation;
    _nextCancellation += BigInt.one;
    return id;
  }

  @override
  Future<FlutterDocumentSummary> openDocument({
    required FlutterOpenRequest request,
    required BigInt cancellationId,
  }) async {
    _ensureAlive();
    final result = openResults.removeFirst();
    if (result case FlutterDocumentSummary summary) {
      currentFormat = summary.format;
      return summary;
    }
    throw StateError(result as String);
  }

  @override
  bool releaseBuffer({required FlutterBufferHandle handle}) {
    _ensureAlive();
    releasedBuffers.add(handle);
    return true;
  }

  @override
  bool releaseSelection({required FlutterSelectionHandle handle}) => true;

  @override
  bool releaseCancellation({required BigInt id}) {
    _ensureAlive();
    releasedCancellations.add(id);
    return true;
  }

  @override
  bool releaseDocument({required FlutterDocumentHandle handle}) {
    _ensureAlive();
    releasedDocuments.add(handle);
    return true;
  }

  @override
  Future<FlutterRenderedBuffer> renderPage({
    required FlutterDocumentHandle document,
    required BigInt page,
    required double scale,
    required BigInt cancellationId,
  }) async {
    _ensureAlive();
    final id = _nextBuffer;
    _nextBuffer += BigInt.one;
    return FlutterRenderedBuffer(
      handle: FlutterBufferHandle(registry: BigInt.one, id: id),
      width: 1,
      height: 1,
      byteLen: BigInt.from(4),
    );
  }

  @override
  Future<FlutterSelectionSurface> selectionSurface({
    required FlutterDocumentHandle document,
    required BigInt unit,
    required double scale,
    required double width,
    required double fontSize,
    required BigInt cancellationId,
  }) async {
    return FlutterSelectionSurface(
      handle: FlutterSelectionHandle(registry: BigInt.one, id: cancellationId),
      width: 100,
      height: 100,
      text: 'Selectable fixture text',
      raster: currentFormat == FlutterBookFormat.epub
          ? FlutterRenderedBuffer(
              handle: FlutterBufferHandle(
                registry: BigInt.one,
                id: cancellationId,
              ),
              width: 1,
              height: 1,
              byteLen: BigInt.from(4),
            )
          : null,
      endpoints: [],
    );
  }

  @override
  Uint8List takeBuffer({required FlutterBufferHandle handle}) {
    _ensureAlive();
    return Uint8List.fromList([255, 255, 255, 255]);
  }
}

ReaderController _epubController(FlutterBridge bridge) {
  return ReaderController(
    bridge: bridge,
    decoder: (pixels, {required width, required height}) => _testImage(),
  );
}

Future<void> _expectFailedOperation(
  ReaderController controller,
  _FakeBridge bridge,
  String message,
) async {
  await bridge.operationFinished.future;
  await Future<void>.delayed(Duration.zero);
  expect(controller.model.busy, isFalse);
  expect(controller.model.document, isNull);
  expect(controller.model.pageImage, isNull);
  expect(controller.model.error, contains(message));
  controller.dispose();
  await bridge.disposed.future;
  expect(bridge.disposeCount, 1);
}

Future<ui.Image> _testImage() async {
  final recorder = ui.PictureRecorder();
  ui.Canvas(recorder).drawColor(const ui.Color(0xffffffff), ui.BlendMode.src);
  final picture = recorder.endRecording();
  try {
    return await picture.toImage(1, 1);
  } finally {
    picture.dispose();
  }
}

Future<ui.Image> _colorTestImage() async {
  final recorder = ui.PictureRecorder();
  final canvas = ui.Canvas(recorder);
  canvas.drawRect(
    const ui.Rect.fromLTWH(0, 0, 1, 1),
    ui.Paint()..color = const ui.Color(0xffff0000),
  );
  canvas.drawRect(
    const ui.Rect.fromLTWH(1, 0, 1, 1),
    ui.Paint()..color = const ui.Color(0xff0000ff),
  );
  final picture = recorder.endRecording();
  try {
    return await picture.toImage(2, 1);
  } finally {
    picture.dispose();
  }
}
