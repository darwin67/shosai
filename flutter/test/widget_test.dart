import 'dart:async';
import 'dart:collection';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
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

  test(
    'geometry-only PDF annotations paint their persisted rectangles',
    () async {
      final recorder = ui.PictureRecorder();
      final canvas = Canvas(recorder);
      final annotation = FlutterAnnotation(
        id: 'geometry',
        unit: BigInt.zero,
        resolution: FlutterAnnotationResolution.exact,
        rectangles: const [
          FlutterSelectionRect(left: 10, top: 10, right: 20, bottom: 20),
        ],
        color: FlutterHighlightColor.yellow,
      );
      PagePainter(
        image: null,
        surface: FlutterSelectionSurface(
          handle: FlutterSelectionHandle(registry: BigInt.one, id: BigInt.one),
          width: 100,
          height: 100,
          text: '',
          copyEligible: false,
          endpoints: const [],
          graphemeBoundaries: Uint32List(0),
          wordBoundaries: Uint32List(0),
          visualLines: const [],
        ),
        backgroundColor: Colors.white,
        foregroundColor: Colors.black,
        recolorImage: false,
        anchor: null,
        focus: null,
        savedSelections: const [],
        annotations: [
          annotation,
          FlutterAnnotation(
            id: 'other-page',
            unit: BigInt.one,
            resolution: FlutterAnnotationResolution.exact,
            rectangles: const [
              FlutterSelectionRect(left: 30, top: 30, right: 40, bottom: 40),
            ],
            color: FlutterHighlightColor.yellow,
          ),
        ],
      ).paint(canvas, const Size(100, 100));
      final image = await recorder.endRecording().toImage(100, 100);
      final bytes = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
      final offset = (15 * 100 + 15) * 4;
      expect(bytes, isNotNull);
      final pixels = bytes!;
      expect([
        for (var channel = 0; channel < 3; channel += 1)
          pixels.getUint8(offset + channel),
      ], isNot(everyElement(255)));
      final otherPageOffset = (35 * 100 + 35) * 4;
      expect([
        for (var channel = 0; channel < 3; channel += 1)
          pixels.getUint8(otherPageOffset + channel),
      ], everyElement(255));
      image.dispose();
    },
  );

  test('reader model recursively freezes collection inputs and copies', () {
    final endpoints = <FlutterSelectionEndpoint>[
      FlutterSelectionEndpoint(
        offset: BigInt.zero,
        rangeStart: BigInt.zero,
        rangeEnd: BigInt.one,
        rect: const FlutterSelectionRect(left: 0, top: 0, right: 1, bottom: 1),
      ),
    ];
    final rectangles = <FlutterSelectionRect>[
      const FlutterSelectionRect(left: 0, top: 0, right: 1, bottom: 1),
    ];
    final annotations = <FlutterAnnotation>[
      FlutterAnnotation(
        id: 'one',
        unit: BigInt.zero,
        resolution: FlutterAnnotationResolution.exact,
        rectangles: rectangles,
        color: FlutterHighlightColor.yellow,
      ),
    ];
    final selections = <ReaderSelection>[const ReaderSelection(0, 1)];
    final operations = <String>{'write'};
    final carets = <FlutterSelectionCaret>[
      FlutterSelectionCaret(
        offset: BigInt.zero,
        x: 0,
        alongLine: 0,
        vertical: false,
        top: 0,
        bottom: 1,
      ),
    ];
    final lines = <FlutterSelectionVisualLine>[
      FlutterSelectionVisualLine(carets: carets),
    ];
    final model = ReaderModel(
      selectionSurface: FlutterSelectionSurface(
        handle: FlutterSelectionHandle(registry: BigInt.one, id: BigInt.one),
        width: 1,
        height: 1,
        text: 'a',
        copyEligible: true,
        endpoints: endpoints,
        graphemeBoundaries: Uint32List.fromList([0, 1]),
        wordBoundaries: Uint32List.fromList([0, 1]),
        visualLines: lines,
      ),
      annotations: annotations,
      savedSelections: selections,
      annotationOperations: operations,
    );
    endpoints.clear();
    carets.clear();
    lines.clear();
    rectangles.clear();
    annotations.clear();
    selections.clear();
    operations.clear();

    expect(model.selectionSurface!.endpoints, hasLength(1));
    expect(model.annotations, hasLength(1));
    expect(model.annotations.single.rectangles, hasLength(1));
    expect(model.savedSelections, hasLength(1));
    expect(model.annotationOperations, {'write'});
    expect(
      () => model.selectionSurface!.endpoints.clear(),
      throwsUnsupportedError,
    );
    expect(
      () => model.selectionSurface!.graphemeBoundaries[0] = 1,
      throwsUnsupportedError,
    );
    expect(
      () => model.selectionSurface!.wordBoundaries[0] = 1,
      throwsUnsupportedError,
    );
    expect(model.selectionSurface!.visualLines.single.carets, hasLength(1));
    expect(
      () => model.selectionSurface!.visualLines.clear(),
      throwsUnsupportedError,
    );
    expect(
      () => model.selectionSurface!.visualLines.single.carets.clear(),
      throwsUnsupportedError,
    );
    expect(() => model.annotations.clear(), throwsUnsupportedError);
    expect(
      () => model.annotations.single.rectangles!.clear(),
      throwsUnsupportedError,
    );
    expect(() => model.savedSelections.clear(), throwsUnsupportedError);
    expect(() => model.annotationOperations.clear(), throwsUnsupportedError);
    final copy = model.copyWith();
    expect(copy.selectionSurface, same(model.selectionSurface));
    expect(copy.annotations.single, same(model.annotations.single));
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
    addTearDown(tester.view.resetDevicePixelRatio);
    final bridge = _FakeBridge()..missingSelectionRaster = true;
    bridge.completeOpen(FlutterBookFormat.epub);
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book.epub');
    await tester.tap(find.text('Open document'));
    await bridge.operationFinished.future;
    await tester.pump();

    expect(find.byType(CircularProgressIndicator), findsNothing);
    expect(find.textContaining('missing its raster'), findsWidgets);

    final desiredScale = tester.view.devicePixelRatio == 2 ? 3.0 : 2.0;
    tester.view.devicePixelRatio = desiredScale;
    await tester.pump();
    await tester.pump();
    bridge.missingSelectionRaster = false;
    await tester.enterText(find.byType(TextField), '/tmp/retry.epub');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();
    expect(bridge.selectionLayouts.last.scale, desiredScale);
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
          copyEligible: true,
          raster: FlutterRenderedBuffer(
            handle: _bufferHandle,
            width: 1,
            height: 1,
            byteLen: BigInt.from(4),
          ),
          endpoints: const [],
          graphemeBoundaries: Uint32List(0),
          wordBoundaries: Uint32List(0),
          visualLines: const [],
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

  test('pointer cancellation only clears its owning interaction', () async {
    final bridge = _ControlledBridge();
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');

    controller.dispatch(const ReaderSelectionPointerStarted(7, 1));
    controller.dispatch(const ReaderSelectionPointerMoved(7, 8));
    controller.dispatch(const ReaderSelectionPointerStarted(9, 4));
    controller.dispatch(const ReaderSelectionPointerPressedOutside(9));
    controller.dispatch(const ReaderSelectionPointerCancelled(9));
    expect(controller.model.selectionPhase, ReaderSelectionPhase.selecting);
    expect(controller.model.selectionPointer, 7);
    expect(controller.model.anchor, 1);
    expect(controller.model.focus, 8);

    controller.dispatch(const ReaderSelectionPointerCancelled(7));
    expect(controller.model.selectionPhase, ReaderSelectionPhase.idle);
    expect(controller.model.anchor, isNull);
    controller.dispose();
    await bridge.disposed.future;
  });

  test('an empty pointer click retains a keyboard selection caret', () async {
    final bridge = _ControlledBridge();
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');

    controller.dispatch(const ReaderSelectionPointerStarted(7, 3));
    controller.dispatch(const ReaderSelectionPointerEnded(7));
    expect(controller.model.selectionPhase, ReaderSelectionPhase.idle);
    expect(controller.model.anchor, 3);
    expect(controller.model.focus, 3);
    controller.dispatch(
      const ReaderSelectionKeyboardExtended(
        ReaderSelectionMovement.nextGrapheme,
      ),
    );
    expect(controller.model.selectionPhase, ReaderSelectionPhase.selected);
    expect(controller.model.anchor, 3);
    expect(controller.model.focus, 4);

    controller.dispose();
    await bridge.disposed.future;
  });

  test('keyboard selection crosses a collapsed retained anchor', () async {
    final bridge = _ControlledBridge();
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    controller.dispatch(const ReaderSelectionStarted(3));
    controller.dispatch(const ReaderSelectionExtended(4));
    controller.dispatch(const ReaderSelectionEnded());

    controller.dispatch(
      const ReaderSelectionKeyboardExtended(
        ReaderSelectionMovement.previousGrapheme,
      ),
    );
    expect(controller.model.selectionPhase, ReaderSelectionPhase.idle);
    expect(controller.model.anchor, 3);
    expect(controller.model.focus, 3);
    controller.dispatch(
      const ReaderSelectionKeyboardExtended(
        ReaderSelectionMovement.previousGrapheme,
      ),
    );
    expect(controller.model.selectionPhase, ReaderSelectionPhase.selected);
    expect(controller.model.anchor, 3);
    expect(controller.model.focus, 2);

    controller.dispatch(const ReaderSelectionCommitted());
    await Future<void>.delayed(Duration.zero);
    expect(bridge.createdRanges.single, (BigInt.from(2), BigInt.from(3)));
    controller.dispose();
    await bridge.disposed.future;
  });

  test('line edge movements use Home and End semantics', () async {
    final bridge = _ControlledBridge();
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');

    controller.dispatch(const ReaderSelectionStarted(3));
    controller.dispatch(const ReaderSelectionEnded());
    controller.dispatch(
      const ReaderSelectionKeyboardExtended(ReaderSelectionMovement.lineStart),
    );
    expect(controller.model.anchor, 3);
    expect(controller.model.focus, 1);

    controller.dispatch(const ReaderSelectionStarted(3));
    controller.dispatch(const ReaderSelectionEnded());
    controller.dispatch(
      const ReaderSelectionKeyboardExtended(ReaderSelectionMovement.lineEnd),
    );
    expect(controller.model.anchor, 3);
    expect(controller.model.focus, 4);

    controller.dispose();
    await bridge.disposed.future;
  });

  test('horizontal keyboard movement follows visual bidi carets', () async {
    final bridge = _ControlledBridge(
      selectionVisualLines: [
        FlutterSelectionVisualLine(
          carets: [
            for (final (offset, x) in [(3, 10.0), (2, 20.0), (1, 30.0)])
              FlutterSelectionCaret(
                offset: BigInt.from(offset),
                x: x,
                alongLine: x,
                vertical: false,
                top: 10,
                bottom: 30,
              ),
          ],
        ),
        FlutterSelectionVisualLine(
          carets: [
            for (final (offset, x) in [
              (0, 0.0),
              (1, 10.0),
              (3, 20.0),
              (2, 30.0),
              (3, 40.0),
              (4, 50.0),
            ])
              FlutterSelectionCaret(
                offset: BigInt.from(offset),
                x: x,
                alongLine: x,
                vertical: false,
                top: 50,
                bottom: 70,
              ),
          ],
        ),
      ],
    );
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    controller.dispatch(
      const ReaderSelectionPointerStarted(7, 2, x: 20, y: 20),
    );
    controller.dispatch(const ReaderSelectionPointerEnded(7));
    controller.dispatch(
      const ReaderSelectionKeyboardExtended(
        ReaderSelectionMovement.visualRight,
      ),
    );
    expect(controller.model.anchor, 2);
    expect(controller.model.focus, 1);

    controller.dispatch(const ReaderSelectionStarted(2));
    controller.dispatch(const ReaderSelectionEnded());
    controller.dispatch(
      const ReaderSelectionKeyboardExtended(ReaderSelectionMovement.visualLeft),
    );
    expect(controller.model.anchor, 2);
    expect(controller.model.focus, 3);

    controller.dispatch(const ReaderSelectionCancelled());
    controller.dispatch(
      const ReaderSelectionPointerStarted(8, 3, x: 20, y: 60),
    );
    controller.dispatch(const ReaderSelectionPointerEnded(8));
    for (var index = 0; index < 3; index += 1) {
      controller.dispatch(
        const ReaderSelectionKeyboardExtended(
          ReaderSelectionMovement.visualRight,
        ),
      );
    }
    expect(controller.model.anchor, 3);
    expect(controller.model.focus, 4);

    controller.dispose();
    await bridge.disposed.future;
  });

  test('horizontal keyboard movement crosses retained visual lines', () async {
    final bridge = _ControlledBridge();
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    controller.dispatch(
      const ReaderSelectionPointerStarted(7, 3, x: 40, y: 20),
    );
    controller.dispatch(const ReaderSelectionPointerEnded(7));
    for (var index = 0; index < 3; index += 1) {
      controller.dispatch(
        const ReaderSelectionKeyboardExtended(
          ReaderSelectionMovement.visualRight,
        ),
      );
    }
    expect(controller.model.anchor, 3);
    expect(controller.model.focus, 5);

    controller.dispatch(const ReaderSelectionCancelled());
    controller.dispatch(
      const ReaderSelectionPointerStarted(8, 5, x: 30, y: 70),
    );
    controller.dispatch(const ReaderSelectionPointerEnded(8));
    for (var index = 0; index < 3; index += 1) {
      controller.dispatch(
        const ReaderSelectionKeyboardExtended(
          ReaderSelectionMovement.visualLeft,
        ),
      );
    }
    expect(controller.model.anchor, 5);
    expect(controller.model.focus, 3);

    controller.dispose();
    await bridge.disposed.future;
  });

  test(
    'line movement preserves along-line affinity between vertical lines',
    () async {
      FlutterSelectionCaret caret(int offset, double x, double alongLine) =>
          FlutterSelectionCaret(
            offset: BigInt.from(offset),
            x: x,
            alongLine: alongLine,
            vertical: true,
            top: 0,
            bottom: 100,
          );
      final bridge = _ControlledBridge(
        selectionVisualLines: [
          FlutterSelectionVisualLine(
            carets: [
              caret(1, 10, 10),
              caret(2, 10, 50),
              caret(2, 10, 70),
              caret(3, 10, 90),
            ],
          ),
          FlutterSelectionVisualLine(
            carets: [caret(4, 40, 5), caret(5, 40, 52), caret(8, 40, 95)],
          ),
        ],
      );
      final controller = _epubController(bridge);
      await _openControlled(controller, bridge, '/tmp/book.epub');
      controller.dispatch(const ReaderSelectionStarted(2));
      controller.dispatch(const ReaderSelectionEnded());
      controller.dispatch(
        const ReaderSelectionKeyboardExtended(ReaderSelectionMovement.nextLine),
      );

      expect(controller.model.anchor, 2);
      expect(controller.model.focus, 5);

      controller.dispatch(const ReaderSelectionCancelled());
      controller.dispatch(const ReaderSelectionStarted(1));
      controller.dispatch(const ReaderSelectionEnded());
      for (var step = 0; step < 3; step += 1) {
        controller.dispatch(
          const ReaderSelectionKeyboardExtended(
            ReaderSelectionMovement.visualRight,
          ),
        );
      }
      expect(controller.model.focus, 3);

      controller.dispatch(const ReaderSelectionCancelled());
      controller.dispatch(
        const ReaderSelectionPointerStarted(1, 2, x: 10, y: 69),
      );
      controller.dispatch(const ReaderSelectionPointerEnded(1));
      controller.dispatch(
        const ReaderSelectionKeyboardExtended(
          ReaderSelectionMovement.visualRight,
        ),
      );
      expect(controller.model.focus, 3);
      controller.dispose();
      await bridge.disposed.future;
    },
  );

  test('keyboard movement safely skips non-navigable metadata lines', () async {
    FlutterSelectionVisualLine line(int offset, double top) =>
        FlutterSelectionVisualLine(
          carets: [
            FlutterSelectionCaret(
              offset: BigInt.from(offset),
              x: 10,
              alongLine: 10,
              vertical: false,
              top: top,
              bottom: top + 10,
            ),
          ],
        );
    final bridge = _ControlledBridge(
      selectionVisualLines: [
        const FlutterSelectionVisualLine(carets: []),
        line(1, 10),
        const FlutterSelectionVisualLine(carets: []),
        line(4, 40),
        const FlutterSelectionVisualLine(carets: []),
      ],
    );
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');

    controller.dispatch(
      const ReaderSelectionKeyboardExtended(
        ReaderSelectionMovement.visualRight,
      ),
    );
    expect(controller.model.anchor, 1);
    expect(controller.model.focus, 4);
    controller.dispatch(const ReaderSelectionCancelled());
    controller.dispatch(
      const ReaderSelectionKeyboardExtended(ReaderSelectionMovement.visualLeft),
    );
    expect(controller.model.anchor, 4);
    expect(controller.model.focus, 1);

    controller.dispose();
    await bridge.disposed.future;
  });

  test('saved highlight navigation restores reader keyboard focus', () async {
    final bridge = _ControlledBridge(initialAnnotations: [_annotation('one')]);
    final focusTargets = <ReaderFocusTarget>[];
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) => _testImage(),
      focusAdapter: focusTargets.add,
    );
    await _openControlled(controller, bridge, '/tmp/book.epub');

    controller.dispatch(const ReaderAnnotationNavigated('one'));
    expect(focusTargets.last, ReaderFocusTarget.surface);
    expect(controller.model.selectionPhase, ReaderSelectionPhase.selected);
    controller.dispatch(
      const ReaderSelectionKeyboardExtended(
        ReaderSelectionMovement.nextGrapheme,
      ),
    );
    expect(
      controller.model.anchor,
      _annotation('one').textRange!.start.toInt(),
    );
    expect(
      controller.model.focus,
      _annotation('one').textRange!.end.toInt() + 1,
    );

    controller.dispose();
    await bridge.disposed.future;
  });

  test('saved highlight navigation cancels an outgoing create', () async {
    final bridge = _ControlledBridge(initialAnnotations: [_annotation('one')]);
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    bridge.createCompleter = Completer<FlutterAnnotation>();
    controller.dispatch(const ReaderSelectionStarted(4));
    controller.dispatch(const ReaderSelectionExtended(8));
    controller.dispatch(const ReaderSelectionEnded());
    controller.dispatch(const ReaderSelectionCommitted());
    final cancellation = bridge.createdCancellations.last;

    controller.dispatch(const ReaderAnnotationNavigated('one'));
    expect(bridge.cancelled, contains(cancellation));
    expect(controller.model.anchor, 1);
    expect(controller.model.focus, 3);
    bridge.createCompleter!.completeError(StateError('cancelled'));
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);
    expect(controller.model.annotationError, isNull);

    controller.dispose();
    await bridge.disposed.future;
  });

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

  test(
    'a cancelled old create failure is not attributed to a newer selection',
    () async {
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
      bridge.createCompleter!.completeError(StateError('old create failed'));
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);

      expect(controller.model.selectionPhase, ReaderSelectionPhase.selected);
      expect(controller.model.anchor, 7);
      expect(controller.model.focus, 9);
      expect(controller.model.selectionError, isNull);
      expect(controller.model.annotationError, isNull);
      controller.dispose();
      await bridge.disposed.future;
    },
  );

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

  for (final format in [FlutterBookFormat.epub, FlutterBookFormat.pdf]) {
    test(
      '$format relayout preserves resolved highlights and layout inputs',
      () async {
        final bridge = _ControlledBridge(
          format: format,
          initialAnnotations: _resolutionAnnotations(),
          immediateLists: true,
        );
        final controller = ReaderController(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        );
        await _openControlled(controller, bridge, '/tmp/book');
        const layout = ReaderLayout(scale: 2, width: 320, fontSize: 24);

        controller.dispatch(const ReaderLayoutChanged(layout));
        await bridge.waitForOp(2);

        expect(controller.model.layout, layout);
        expect(controller.model.relayoutBusy, isFalse);
        expect(
          controller.model.annotations.map((item) => item.resolution),
          orderedEquals([
            FlutterAnnotationResolution.recovered,
            FlutterAnnotationResolution.ambiguous,
            FlutterAnnotationResolution.orphaned,
          ]),
        );
        expect(controller.model.savedSelections, hasLength(1));
        expect(bridge.selectionLayouts, [const ReaderLayout(), layout]);
        expect(bridge.listScales, [1, 2]);
        if (format == FlutterBookFormat.pdf) {
          expect(bridge.renderScales, [1, 2]);
        } else {
          expect(bridge.renderScales, isEmpty);
        }
        controller.dispatch(const ReaderSelectionStarted(1));
        controller.dispatch(const ReaderSelectionExtended(3));
        controller.dispatch(const ReaderSelectionEnded());
        controller.dispatch(const ReaderSelectionCommitted());
        await bridge.waitForOp(3);
        expect(bridge.createdScales.single, 2);

        controller.dispose();
        await bridge.disposed.future;
      },
    );
  }

  test('stale EPUB relayout releases its effect-owned raster once', () async {
    final bridge = _ControlledBridge(
      format: FlutterBookFormat.epub,
      immediateLists: true,
    );
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    final staleSelection = Completer<FlutterSelectionSurface>();
    bridge.selectionCompleters.add(staleSelection);
    const first = ReaderLayout(scale: 2, width: 300, fontSize: 20);
    const second = ReaderLayout(scale: 3, width: 400, fontSize: 22);

    controller.dispatch(const ReaderLayoutChanged(first));
    await _waitUntil(() => bridge.selectionCalls == 2);
    controller.dispatch(const ReaderLayoutChanged(second));
    await bridge.waitForOp(2);
    staleSelection.complete(_surface(BigInt.from(20), raster: true));
    await bridge.waitForOp(3);

    final raster = FlutterBufferHandle(
      registry: BigInt.one,
      id: BigInt.from(20),
    );
    final surface = FlutterSelectionHandle(
      registry: BigInt.one,
      id: BigInt.from(20),
    );
    expect(
      bridge.releasedBuffers.where((item) => item == raster),
      hasLength(1),
    );
    expect(
      bridge.releasedSelections.where((item) => item == surface),
      hasLength(1),
    );
    expect(controller.model.layout, second);
    controller.dispose();
    await bridge.disposed.future;
  });

  test('stale PDF render is released without transfer or decode', () async {
    final bridge = _ControlledBridge(
      format: FlutterBookFormat.pdf,
      immediateLists: true,
    );
    var decodeCalls = 0;
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) {
        decodeCalls += 1;
        return _testImage();
      },
    );
    await _openControlled(controller, bridge, '/tmp/book.pdf');
    final staleRender = Completer<FlutterRenderedBuffer>();
    bridge.renderCompleters.add(staleRender);
    const first = ReaderLayout(scale: 2, width: 300, fontSize: 20);
    const second = ReaderLayout(scale: 3, width: 400, fontSize: 22);

    controller.dispatch(const ReaderLayoutChanged(first));
    await _waitUntil(() => bridge.renderCalls == 2);
    controller.dispatch(const ReaderLayoutChanged(second));
    await bridge.waitForOp(2);
    final staleBuffer = _buffer(BigInt.from(20));
    staleRender.complete(staleBuffer);
    await bridge.waitForOp(3);

    expect(decodeCalls, 2, reason: 'only open and current relayout decode');
    expect(bridge.takenBuffers, isNot(contains(staleBuffer.handle)));
    expect(
      bridge.releasedBuffers.where((item) => item == staleBuffer.handle),
      hasLength(1),
    );
    controller.dispose();
    await bridge.disposed.future;
  });

  test('stale decoded image and annotation list never publish', () async {
    final bridge = _ControlledBridge(
      format: FlutterBookFormat.pdf,
      immediateLists: true,
    );
    final staleDecode = Completer<ui.Image>();
    var decodeCalls = 0;
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) {
        decodeCalls += 1;
        return decodeCalls == 2 ? staleDecode.future : _testImage();
      },
    );
    await _openControlled(controller, bridge, '/tmp/book.pdf');
    const first = ReaderLayout(scale: 2, width: 300, fontSize: 20);
    const second = ReaderLayout(scale: 3, width: 400, fontSize: 22);
    controller.dispatch(const ReaderLayoutChanged(first));
    await _waitUntil(() => decodeCalls == 2);
    controller.dispatch(const ReaderLayoutChanged(second));
    await bridge.waitForOp(2);
    final staleImage = await _testImage();
    staleDecode.complete(staleImage);
    await bridge.waitForOp(3);
    expect(staleImage.debugDisposed, isTrue);
    expect(controller.model.layout, second);

    final staleList = Completer<List<FlutterAnnotation>>();
    bridge.listCompleters.add(staleList);
    const third = ReaderLayout(scale: 4, width: 500, fontSize: 24);
    const fourth = ReaderLayout(scale: 5, width: 600, fontSize: 26);
    controller.dispatch(const ReaderLayoutChanged(third));
    await _waitUntil(() => bridge.listCalls == 3);
    controller.dispatch(const ReaderLayoutChanged(fourth));
    await bridge.waitForOp(4);
    staleList.complete([_annotation('stale')]);
    await bridge.waitForOp(5);
    expect(controller.model.layout, fourth);
    expect(controller.model.annotations, isEmpty);

    controller.dispose();
    await bridge.disposed.future;
  });

  test('failed relayout is not automatically retried', () async {
    final bridge = _ControlledBridge(immediateLists: true);
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    final failure = Completer<FlutterSelectionSurface>();
    bridge.selectionCompleters.add(failure);
    const layout = ReaderLayout(scale: 2, width: 300, fontSize: 20);
    controller.dispatch(const ReaderLayoutChanged(layout));
    failure.completeError(StateError('persistent failure'));
    await bridge.waitForOp(2);

    controller.dispatch(const ReaderLayoutChanged(layout));
    await Future<void>.delayed(Duration.zero);
    expect(bridge.selectionCalls, 2);
    expect(controller.model.relayoutBusy, isFalse);
    expect(controller.model.selectionError, contains('persistent failure'));
    controller.dispose();
    await bridge.disposed.future;
  });

  test('failed relayout allocation remains the next open layout', () async {
    final bridge = _ControlledBridge(immediateLists: true);
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    const desired = ReaderLayout(scale: 2, width: 300, fontSize: 20);
    bridge.failCancellationCreation = true;
    controller.dispatch(const ReaderLayoutChanged(desired));

    bridge.failCancellationCreation = false;
    controller.dispatch(const ReaderOpenRequested('/tmp/replacement.epub'));
    await bridge.waitForOp(2);

    expect(bridge.selectionLayouts.last, desired);
    controller.dispose();
    await bridge.disposed.future;
  });

  test(
    'layout changes during open are applied by a follow-up effect',
    () async {
      final bridge = _ControlledBridge(immediateLists: true);
      final controller = _epubController(bridge);
      const layout = ReaderLayout(scale: 2, width: 300, fontSize: 20);
      controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
      controller.dispatch(const ReaderLayoutChanged(layout));
      await bridge.waitForOp(2);

      expect(bridge.selectionLayouts, [const ReaderLayout(), layout]);
      expect(controller.model.layout, layout);
      controller.dispose();
      await bridge.disposed.future;
    },
  );

  test('failed superseding allocation leaves active relayout valid', () async {
    final bridge = _ControlledBridge(immediateLists: true);
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    final activeSelection = Completer<FlutterSelectionSurface>();
    bridge.selectionCompleters.add(activeSelection);
    const active = ReaderLayout(scale: 2, width: 300, fontSize: 20);
    const rejected = ReaderLayout(scale: 3, width: 400, fontSize: 22);
    controller.dispatch(const ReaderLayoutChanged(active));
    await _waitUntil(() => bridge.selectionCalls == 2);
    final activeCancellation = bridge.createdCancellations.last;
    bridge.failCancellationCreation = true;
    controller.dispatch(const ReaderLayoutChanged(rejected));

    expect(controller.model.relayoutBusy, isTrue);
    expect(bridge.cancelled, isNot(contains(activeCancellation)));
    activeSelection.complete(_surface(BigInt.from(20), raster: true));
    await bridge.waitForOp(2);
    expect(controller.model.layout, active);
    expect(controller.model.relayoutBusy, isFalse);
    bridge.failCancellationCreation = false;
    controller.dispatch(const ReaderOpenRequested('/tmp/replacement.epub'));
    await bridge.waitForOp(3);
    expect(bridge.selectionLayouts.last, rejected);
    controller.dispose();
    await bridge.disposed.future;
  });

  test('successful relayout restores annotation readiness', () async {
    final bridge = _ControlledBridge(immediateLists: true)..listFailure = true;
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    expect(controller.model.annotationsReady, isFalse);
    bridge.listFailure = false;
    controller.dispatch(
      const ReaderLayoutChanged(
        ReaderLayout(scale: 2, width: 300, fontSize: 20),
      ),
    );
    await bridge.waitForOp(2);
    expect(controller.model.annotationsReady, isTrue);
    expect(controller.model.annotationError, isNull);
    controller.dispose();
    await bridge.disposed.future;
  });

  test('note completion during relayout reports a retryable error', () async {
    final bridge = _ControlledBridge(
      initialAnnotations: [_annotation('one')],
      immediateLists: true,
    );
    final editor = Completer<String?>();
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) => _testImage(),
      noteEditor: (_) => editor.future,
    );
    await _openControlled(controller, bridge, '/tmp/book.epub');
    controller.dispatch(const ReaderAnnotationNoteRequested('one'));
    final pendingLayout = Completer<FlutterSelectionSurface>();
    bridge.selectionCompleters.add(pendingLayout);
    controller.dispatch(
      const ReaderLayoutChanged(
        ReaderLayout(scale: 2, width: 300, fontSize: 20),
      ),
    );
    await _waitUntil(() => controller.model.relayoutBusy);
    editor.complete('unsaved note');
    await Future<void>.delayed(Duration.zero);

    expect(controller.model.annotationError, contains('Try again'));
    expect(bridge.updateCalls, 0);
    pendingLayout.complete(_surface(BigInt.from(20), raster: true));
    await bridge.waitForOp(2);
    expect(controller.model.annotationError, contains('Try again'));
    controller.dispose();
    await bridge.disposed.future;
  });

  test(
    'selection notes cannot be started or silently lost during relayout',
    () async {
      final bridge = _ControlledBridge(immediateLists: true);
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
      await _openControlled(controller, bridge, '/tmp/book.epub');
      controller.dispatch(const ReaderSelectionStarted(1));
      controller.dispatch(const ReaderSelectionExtended(3));
      controller.dispatch(const ReaderSelectionEnded());
      controller.dispatch(const ReaderSelectionNoteRequested());
      expect(editors, hasLength(1));

      final pendingLayout = Completer<FlutterSelectionSurface>();
      bridge.selectionCompleters.add(pendingLayout);
      controller.dispatch(
        const ReaderLayoutChanged(
          ReaderLayout(scale: 2, width: 300, fontSize: 20),
        ),
      );
      await _waitUntil(() => controller.model.relayoutBusy);
      controller.dispatch(const ReaderSelectionStarted(1));
      controller.dispatch(const ReaderSelectionNoteRequested());
      expect(controller.model.selectionPhase, ReaderSelectionPhase.idle);
      expect(editors, hasLength(1));

      editors.single.complete('preserve this draft');
      await Future<void>.delayed(Duration.zero);
      expect(controller.model.selectionActionError, contains('not saved'));
      expect(bridge.createCalls, 0);
      pendingLayout.complete(_surface(BigInt.from(20), raster: true));
      await bridge.waitForOp(2);
      controller.dispose();
      await bridge.disposed.future;
    },
  );

  test('relayout publishes image surface and annotations atomically', () async {
    final bridge = _ControlledBridge(
      format: FlutterBookFormat.epub,
      immediateLists: true,
    );
    final oldImage = await _testImage();
    final newImage = await _testImage();
    var decodeCalls = 0;
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) async {
        decodeCalls += 1;
        return decodeCalls == 1 ? oldImage : newImage;
      },
    );
    await _openControlled(controller, bridge, '/tmp/book.epub');
    final oldSurface = controller.model.selectionSurface!;
    final selection = Completer<FlutterSelectionSurface>();
    final annotations = Completer<List<FlutterAnnotation>>();
    bridge.selectionCompleters.add(selection);
    bridge.listCompleters.add(annotations);
    final observed = <ReaderModel>[];
    controller.addListener(() => observed.add(controller.model));

    controller.dispatch(
      const ReaderLayoutChanged(
        ReaderLayout(scale: 2, width: 300, fontSize: 20),
      ),
    );
    selection.complete(
      _surface(BigInt.from(20), raster: true, text: 'new surface'),
    );
    await _waitUntil(() => bridge.listCalls == 2);
    expect(controller.model.pageImage, same(oldImage));
    expect(controller.model.selectionSurface, same(oldSurface));
    expect(controller.model.annotations, isEmpty);
    expect(oldImage.debugDisposed, isFalse);

    annotations.complete([_annotation('new')]);
    await bridge.waitForOp(2);
    expect(controller.model.pageImage, same(newImage));
    expect(controller.model.selectionSurface!.text, 'new surface');
    expect(controller.model.annotations.single.id, 'new');
    expect(oldImage.debugDisposed, isTrue);
    expect(
      bridge.releasedSelections.where((item) => item == oldSurface.handle),
      hasLength(1),
    );
    expect(
      observed.where(
        (model) =>
            identical(model.pageImage, newImage) ||
            model.selectionSurface?.text == 'new surface' ||
            model.annotations.isNotEmpty,
      ),
      everyElement(
        predicate<ReaderModel>(
          (model) =>
              identical(model.pageImage, newImage) &&
              model.selectionSurface?.text == 'new surface' &&
              model.annotations.single.id == 'new',
        ),
      ),
    );
    controller.dispose();
    await bridge.disposed.future;
  });

  test('opening allocation failure clears an active relayout state', () async {
    final bridge = _ControlledBridge(immediateLists: true);
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    final pendingLayout = Completer<FlutterSelectionSurface>();
    bridge.selectionCompleters.add(pendingLayout);
    controller.dispatch(
      const ReaderLayoutChanged(
        ReaderLayout(scale: 2, width: 300, fontSize: 20),
      ),
    );
    await _waitUntil(() => controller.model.relayoutBusy);
    bridge.failCancellationCreation = true;
    controller.dispatch(const ReaderOpenRequested('/tmp/replacement.epub'));

    expect(controller.model.relayoutBusy, isFalse);
    expect(controller.model.document, isNull);
    expect(controller.model.error, contains('cancellation tokens'));
    pendingLayout.complete(_surface(BigInt.from(20), raster: true));
    await bridge.waitForOp(2);
    controller.dispose();
    await bridge.disposed.future;
  });

  test('replacement open keeps the latest pending layout', () async {
    final bridge = _ControlledBridge(immediateLists: true);
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/book.epub');
    final pendingLayout = Completer<FlutterSelectionSurface>();
    bridge.selectionCompleters.add(pendingLayout);
    const desired = ReaderLayout(scale: 2, width: 300, fontSize: 20);
    controller.dispatch(const ReaderLayoutChanged(desired));
    await _waitUntil(() => controller.model.relayoutBusy);

    controller.dispatch(const ReaderOpenRequested('/tmp/replacement.epub'));
    await _waitUntil(() => bridge.selectionCalls == 3);

    expect(controller.model.layout, desired);
    expect(bridge.selectionLayouts.last, desired);
    pendingLayout.complete(_surface(BigInt.from(20), raster: true));
    await bridge.waitForOp(3);
    controller.dispose();
    await bridge.disposed.future;
  });

  test('queued CBZ layout is used by the next EPUB open', () async {
    final bridge = _ControlledBridge(
      format: FlutterBookFormat.cbz,
      immediateLists: true,
    );
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) => _testImage(),
    );
    controller.dispatch(const ReaderOpenRequested('/tmp/book.cbz'));
    await bridge.waitForOp(1);
    const desired = ReaderLayout(scale: 2, width: 300, fontSize: 20);
    controller.dispatch(const ReaderLayoutChanged(desired));
    expect(bridge.selectionCalls, 0);
    expect(controller.model.selectionError, isNull);
    expect(controller.model.relayoutBusy, isFalse);

    bridge.format = FlutterBookFormat.epub;
    controller.dispatch(const ReaderOpenRequested('/tmp/book.epub'));
    await bridge.waitForOp(2);
    expect(bridge.selectionLayouts, [desired]);
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

  for (final body in <String?>[null, 'edited note']) {
    test(
      'annotation ${body == null ? 'recolor' : 'note'} handles cancellation allocation failure',
      () async {
        final bridge = _ControlledBridge(
          initialAnnotations: [_annotation('one')],
          immediateLists: true,
        );
        final controller = _epubController(bridge);
        await _openControlled(controller, bridge, '/tmp/a.epub');
        bridge.failCancellationCreation = true;

        controller.dispatch(
          ReaderAnnotationUpdated('one', FlutterHighlightColor.green, body),
        );
        await Future<void>.delayed(Duration.zero);
        expect(
          controller.model.annotationError,
          contains('cancellation tokens'),
        );
        expect(controller.model.annotationOperations, isEmpty);
        expect(bridge.updateCalls, 0);

        bridge.failCancellationCreation = false;
        bridge.updateCompleter = null;
        controller.dispatch(
          ReaderAnnotationUpdated('one', FlutterHighlightColor.blue, body),
        );
        while (controller.model.annotationOperations.isNotEmpty) {
          await Future<void>.delayed(Duration.zero);
        }
        expect(controller.model.annotationError, isNull);
        expect(bridge.updateCalls, 1);

        controller.dispose();
        await bridge.disposed.future;
      },
    );
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

  test('cancelling a selection cancels its in-flight create quietly', () async {
    final bridge = _ControlledBridge();
    final controller = _epubController(bridge);
    await _openControlled(controller, bridge, '/tmp/a.epub');
    bridge.createCompleter = Completer<FlutterAnnotation>();
    controller.dispatch(const ReaderSelectionStarted(1));
    controller.dispatch(const ReaderSelectionExtended(3));
    controller.dispatch(const ReaderSelectionEnded());
    controller.dispatch(const ReaderSelectionCommitted());
    final cancellation = bridge.createdCancellations.last;

    controller.dispatch(const ReaderSelectionCancelled());
    expect(bridge.cancelled, contains(cancellation));
    expect(controller.model.selectionPhase, ReaderSelectionPhase.idle);
    bridge.createCompleter!.completeError(StateError('cancelled'));
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);
    expect(controller.model.selectionActionError, isNull);
    expect(controller.model.annotationError, isNull);
    expect(bridge.releasedCancellations, contains(cancellation));

    controller.dispose();
    await bridge.disposed.future;
  });

  test('selection action retries clear and supersede earlier errors', () async {
    final bridge = _ControlledBridge();
    final copies = <Completer<void>>[];
    final controller = ReaderController(
      bridge: bridge,
      decoder: (pixels, {required width, required height}) => _testImage(),
      selectionCopier: (_) {
        final copy = Completer<void>();
        copies.add(copy);
        return copy.future;
      },
    );
    await _openControlled(controller, bridge, '/tmp/a.epub');
    controller.dispatch(const ReaderSelectionStarted(1));
    controller.dispatch(const ReaderSelectionExtended(3));
    controller.dispatch(const ReaderSelectionEnded());

    controller.dispatch(const ReaderSelectionCopyRequested());
    copies.single.completeError(StateError('clipboard denied'));
    await Future<void>.delayed(Duration.zero);
    expect(controller.model.selectionActionError, contains('clipboard denied'));

    controller.dispatch(const ReaderSelectionCopyRequested());
    expect(controller.model.selectionActionError, isNull);
    controller.dispatch(const ReaderSelectionCopyRequested());
    copies[2].complete();
    copies[1].completeError(StateError('stale failure'));
    await Future<void>.delayed(Duration.zero);
    expect(controller.model.selectionActionError, isNull);

    controller.dispose();
    await bridge.disposed.future;
  });

  for (final createFails in [false, true]) {
    test(
      'a stale Copy failure cannot replace a ${createFails ? 'failed' : 'successful'} save result',
      () async {
        final bridge = _ControlledBridge();
        final copy = Completer<void>();
        final controller = ReaderController(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
          selectionCopier: (_) => copy.future,
        );
        await _openControlled(controller, bridge, '/tmp/a.epub');
        controller.dispatch(const ReaderSelectionStarted(1));
        controller.dispatch(const ReaderSelectionExtended(3));
        controller.dispatch(const ReaderSelectionEnded());
        controller.dispatch(const ReaderSelectionCopyRequested());
        bridge.createCompleter = Completer<FlutterAnnotation>();
        controller.dispatch(const ReaderSelectionCommitted());
        if (createFails) {
          bridge.createCompleter!.completeError(StateError('save failed'));
        } else {
          bridge.createCompleter!.complete(_annotation('created'));
        }
        await Future<void>.delayed(Duration.zero);
        await Future<void>.delayed(Duration.zero);
        copy.completeError(StateError('stale clipboard failure'));
        await Future<void>.delayed(Duration.zero);

        if (createFails) {
          expect(
            controller.model.selectionActionError,
            contains('save failed'),
          );
        } else {
          expect(controller.model.selectionPhase, ReaderSelectionPhase.idle);
          expect(controller.model.selectionActionError, isNull);
        }
        controller.dispose();
        await bridge.disposed.future;
      },
    );
  }

  for (final operation in ['update', 'delete']) {
    test('$operation retry clears its earlier annotation error', () async {
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
        bridge.updateCompleter!.completeError(StateError('update failed'));
      } else {
        bridge.deleteCompleter = Completer<bool>();
        controller.dispatch(const ReaderAnnotationDeleted('one'));
        bridge.deleteCompleter!.completeError(StateError('delete failed'));
      }
      while (controller.model.annotationOperations.isNotEmpty) {
        await Future<void>.delayed(Duration.zero);
      }
      expect(controller.model.annotationError, contains('$operation failed'));

      if (operation == 'update') {
        bridge.updateCompleter = null;
        controller.dispatch(
          const ReaderAnnotationUpdated(
            'one',
            FlutterHighlightColor.blue,
            null,
          ),
        );
      } else {
        bridge.deleteCompleter = null;
        controller.dispatch(const ReaderAnnotationDeleted('one'));
      }
      expect(controller.model.annotationError, isNull);
      while (controller.model.annotationOperations.isNotEmpty) {
        await Future<void>.delayed(Duration.zero);
      }
      expect(controller.model.annotationError, isNull);

      controller.dispose();
      await bridge.disposed.future;
    });
  }

  test(
    'acknowledged note survives a failed refresh and later recolor',
    () async {
      final bridge = _ControlledBridge(
        initialAnnotations: [_annotation('one')],
        immediateLists: true,
      )..updateCompleter = null;
      final controller = _epubController(bridge);
      await _openControlled(controller, bridge, '/tmp/book.epub');
      bridge.listFailure = true;

      controller.dispatch(
        const ReaderAnnotationUpdated(
          'one',
          FlutterHighlightColor.yellow,
          'saved note',
        ),
      );
      await _waitUntil(() => controller.model.annotationOperations.isEmpty);
      expect(controller.model.annotations.single.body, 'saved note');

      controller.dispatch(
        ReaderAnnotationUpdated(
          'one',
          FlutterHighlightColor.green,
          controller.model.annotations.single.body,
        ),
      );
      await _waitUntil(() => controller.model.annotationOperations.isEmpty);
      expect(bridge.storedAnnotations.single.body, 'saved note');
      expect(
        bridge.storedAnnotations.single.color,
        FlutterHighlightColor.green,
      );
      controller.dispose();
      await bridge.disposed.future;
    },
  );

  testWidgets('annotation controls prevent overlapping writes', (tester) async {
    addTearDown(tester.view.resetDevicePixelRatio);
    final bridge = _ControlledBridge(
      initialAnnotations: [_annotation('one')],
      immediateLists: true,
    );
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

    final pendingLayout = Completer<FlutterSelectionSurface>();
    bridge.selectionCompleters.add(pendingLayout);
    tester.view.devicePixelRatio = tester.view.devicePixelRatio == 2 ? 3 : 2;
    await tester.pump();
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
    expect(
      tester
          .widget<IconButton>(
            find.widgetWithIcon(IconButton, Icons.delete_outline),
          )
          .onPressed,
      isNull,
    );
    pendingLayout.complete(_surface(BigInt.from(20), raster: true));
    await tester.pumpAndSettle();
    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets('returning to the displayed layout cancels pending relayout', (
    tester,
  ) async {
    addTearDown(tester.view.resetDevicePixelRatio);
    final bridge = _ControlledBridge(immediateLists: true);
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
    final displayedScale = tester.view.devicePixelRatio;
    final selectionCalls = bridge.selectionCalls;
    final pending = Completer<FlutterSelectionSurface>();
    bridge.selectionCompleters.add(pending);

    tester.view.devicePixelRatio = displayedScale == 2 ? 3 : 2;
    await tester.pump();
    await tester.pump();
    expect(find.byType(LinearProgressIndicator), findsOneWidget);
    final pendingCancellation = bridge.createdCancellations.last;

    tester.view.devicePixelRatio = displayedScale;
    await tester.pump();
    await tester.pump();
    expect(bridge.cancelled, contains(pendingCancellation));
    expect(find.byType(LinearProgressIndicator), findsNothing);

    pending.complete(_surface(BigInt.from(30), raster: true));
    await tester.pumpAndSettle();
    expect(bridge.selectionCalls, selectionCalls + 1);
    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets('viewport changes wait for an annotation write then relayout', (
    tester,
  ) async {
    addTearDown(tester.view.resetDevicePixelRatio);
    final bridge = _ControlledBridge(
      initialAnnotations: [_annotation('one')],
      immediateLists: true,
    );
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
    final selectionCalls = bridge.selectionCalls;
    bridge.updateCompleter = Completer<bool>();
    await tester.tap(find.byTooltip('Change color'));
    await tester.pump();

    final desiredScale = tester.view.devicePixelRatio == 2 ? 3.0 : 2.0;
    tester.view.devicePixelRatio = desiredScale;
    await tester.pump();
    await tester.pump();
    expect(bridge.selectionCalls, selectionCalls);

    bridge.updateCompleter!.complete(true);
    await tester.pumpAndSettle();
    expect(bridge.selectionCalls, selectionCalls + 1);
    expect(bridge.selectionLayouts.last.scale, desiredScale);
    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets(
    'returning to the displayed viewport during an annotation write skips relayout',
    (tester) async {
      addTearDown(tester.view.resetDevicePixelRatio);
      final bridge = _ControlledBridge(
        initialAnnotations: [_annotation('one')],
        immediateLists: true,
      );
      await tester.pumpWidget(
        MaterialApp(
          home: ReaderScreen(
            bridge: bridge,
            decoder: (pixels, {required width, required height}) =>
                _testImage(),
          ),
        ),
      );
      await tester.enterText(find.byType(TextField), '/tmp/a.epub');
      await tester.tap(find.text('Open document'));
      await tester.pumpAndSettle();
      final displayedScale = tester.view.devicePixelRatio;
      final selectionCalls = bridge.selectionCalls;
      bridge.updateCompleter = Completer<bool>();
      await tester.tap(find.byTooltip('Change color'));
      await tester.pump();

      tester.view.devicePixelRatio = displayedScale == 2 ? 3 : 2;
      await tester.pump();
      await tester.pump();
      tester.view.devicePixelRatio = displayedScale;
      await tester.pump();
      await tester.pump();
      bridge.updateCompleter!.complete(true);
      await tester.pumpAndSettle();

      expect(bridge.selectionCalls, selectionCalls);
      await tester.pumpWidget(const SizedBox());
      await bridge.disposed.future;
    },
  );

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

  testWidgets(
    'viewport DPI font and theme changes preserve surfaced anchor states',
    (tester) async {
      tester.view.physicalSize = const Size(1200, 1000);
      tester.view.devicePixelRatio = 2;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final bridge = _ControlledBridge(
        initialAnnotations: _resolutionAnnotations(),
        immediateLists: true,
      );
      Widget app({
        Brightness brightness = Brightness.light,
        double textScale = 1.5,
      }) => MaterialApp(
        theme: ThemeData(brightness: brightness),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(textScaler: TextScaler.linear(textScale)),
          child: child!,
        ),
        home: ReaderScreen(
          key: const ValueKey('reader'),
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      );

      await tester.pumpWidget(app());
      await tester.pump();
      await tester.enterText(find.byType(TextField), '/tmp/book.epub');
      await tester.tap(find.text('Open document'));
      await tester.pumpAndSettle();
      expect(
        bridge.selectionLayouts.single,
        const ReaderLayout(scale: 2, width: 552, fontSize: 27),
      );
      expect(find.text('Highlight 1 — recovered'), findsOneWidget);
      await tester.scrollUntilVisible(
        find.text('Highlight 1 — ambiguous'),
        300,
        scrollable: find.byType(Scrollable).last,
      );
      expect(find.text('Highlight 1 — ambiguous'), findsOneWidget);
      await tester.scrollUntilVisible(
        find.text('Highlight 1 — unavailable'),
        300,
        scrollable: find.byType(Scrollable).last,
      );
      expect(find.text('Highlight 1 — unavailable'), findsOneWidget);

      tester.view.physicalSize = const Size(1400, 1000);
      await tester.pumpAndSettle();
      expect(
        bridge.selectionLayouts.last,
        const ReaderLayout(scale: 2, width: 652, fontSize: 27),
      );

      final callsBeforeTheme = bridge.selectionCalls;
      await tester.pumpWidget(app(brightness: Brightness.dark));
      await tester.pumpAndSettle();
      expect(bridge.selectionCalls, callsBeforeTheme);
      expect(find.text('Highlight 1 — unavailable'), findsOneWidget);

      await tester.pumpWidget(app(brightness: Brightness.dark, textScale: 2));
      await tester.pumpAndSettle();
      expect(
        bridge.selectionLayouts.last,
        const ReaderLayout(scale: 2, width: 652, fontSize: 36),
      );

      tester.view.devicePixelRatio = 3;
      tester.view.physicalSize = const Size(2100, 1500);
      await tester.pumpAndSettle();
      expect(
        bridge.selectionLayouts.last,
        const ReaderLayout(scale: 3, width: 652, fontSize: 36),
      );
      expect(find.text('Highlight 1 — unavailable'), findsOneWidget);

      await tester.pumpWidget(const SizedBox());
      await bridge.disposed.future;
    },
  );

  for (final format in [FlutterBookFormat.pdf, FlutterBookFormat.epub]) {
    for (final device in [
      ui.PointerDeviceKind.mouse,
      ui.PointerDeviceKind.touch,
    ]) {
      testWidgets(
        '$format rendered surface accepts $device selection endpoints',
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

          final surface = find.byKey(
            const ValueKey('reader-selection-surface'),
          );
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
          final gesture = await tester.createGesture(kind: device);
          await gesture.down(surfaceTopLeft + Offset(side * .2, side * .2));
          await tester.pump();
          expect(painter().anchor, 1, reason: 'press must retain the anchor');
          expect(painter().focus, 1);
          await gesture.moveBy(Offset(side * .5, side * .5));
          await gesture.up();
          await tester.pump();

          expect(find.text('Yellow'), findsOneWidget);
          expect(find.text('Copy'), findsOneWidget);
          expect(find.text('Green'), findsOneWidget);
          expect(find.text('Blue'), findsOneWidget);
          expect(find.text('Pink'), findsOneWidget);
          expect(find.text('Purple'), findsOneWidget);
          expect(find.text('Add note'), findsOneWidget);
          expect(find.text('Cancel'), findsOneWidget);
          expect(bridge.selectionCalls, beforeSelectionCalls);
          expect(bridge.renderCalls, beforeRenderCalls);
          expect(bridge.createCalls, 0);
          expect(painter().anchor, 1);
          expect(painter().focus, 8);

          await tester.tap(find.text('Yellow'));
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

  testWidgets('selection actions fit a compact viewport at large text scale', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(400, 300);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final bridge = _ControlledBridge(format: FlutterBookFormat.epub);
    await tester.pumpWidget(
      MaterialApp(
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(textScaler: const TextScaler.linear(2)),
          child: child!,
        ),
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book.epub');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();
    final surface = find.byKey(const ValueKey('reader-selection-surface'));
    final bounds = tester.getRect(surface);
    final side = bounds.shortestSide;
    final contentTopLeft = bounds.center - Offset(side / 2, side / 2);
    final gesture = await tester.createGesture(
      kind: ui.PointerDeviceKind.mouse,
    );
    await gesture.down(contentTopLeft + Offset(side * .2, side * .2));
    await gesture.moveTo(contentTopLeft + Offset(side * .7, side * .7));
    await gesture.up();
    await tester.pump();

    final actions = tester.getRect(
      find.byKey(const ValueKey('selection-actions')),
    );
    expect(actions.left, greaterThanOrEqualTo(0));
    expect(actions.top, greaterThanOrEqualTo(0));
    expect(actions.right, lessThanOrEqualTo(400));
    expect(actions.bottom, lessThanOrEqualTo(300));
    expect(tester.takeException(), isNull);

    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets('keyboard extends, opens, cancels, and commits selection', (
    tester,
  ) async {
    MethodCall? clipboardCall;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') clipboardCall = call;
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    final bridge = _ControlledBridge(format: FlutterBookFormat.epub);
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('reader-selection-surface')));
    await tester.pump();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pump();
    expect(find.text('Yellow'), findsOneWidget);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.home);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pump();
    expect(find.text('Yellow'), findsNothing);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.end);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pump();
    expect(find.text('Yellow'), findsOneWidget);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.home);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pump();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.f10);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pumpAndSettle();
    expect(find.text('Yellow'), findsOneWidget);
    expect(
      tester
          .widget<TextButton>(find.widgetWithText(TextButton, 'Copy'))
          .focusNode!
          .hasFocus,
      isTrue,
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(clipboardCall?.method, 'Clipboard.setData');
    expect(clipboardCall?.arguments, {'text': 'e'});

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    expect(find.text('Yellow'), findsNothing);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.end);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pump();
    expect(find.text('Yellow'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.home);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pump();
    expect(find.text('Yellow'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowRight);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(bridge.createdRanges.single, (BigInt.one, BigInt.from(4)));

    await tester.tap(find.byKey(const ValueKey('reader-selection-surface')));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(bridge.createdRanges.last, (BigInt.one, BigInt.from(4)));

    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets('keyboard actions focus Cancel when other actions are disabled', (
    tester,
  ) async {
    final bridge = _ControlledBridge(
      format: FlutterBookFormat.pdf,
      copyEligible: false,
      listFailure: true,
    );
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book.pdf');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();
    final surface = find.byKey(const ValueKey('reader-selection-surface'));
    final bounds = tester.getRect(surface);
    final side = bounds.shortestSide;
    final topLeft = bounds.center - Offset(side / 2, side / 2);
    final gesture = await tester.createGesture(
      kind: ui.PointerDeviceKind.mouse,
    );
    await gesture.down(topLeft + Offset(side * .2, side * .2));
    await gesture.moveBy(Offset(side * .5, side * .5));
    await gesture.up();
    await tester.pump();
    await tester.tapAt(topLeft + Offset(side * .2, side * .2));
    await tester.pump();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.f10);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pumpAndSettle();

    final cancel = tester.widget<TextButton>(
      find.widgetWithText(TextButton, 'Cancel'),
    );
    expect(
      cancel.focusNode!.hasFocus,
      isTrue,
      reason:
          'primary=${FocusManager.instance.primaryFocus}, cancel=${cancel.focusNode}',
    );
    expect(
      tester
          .widget<TextButton>(find.widgetWithText(TextButton, 'Copy'))
          .onPressed,
      isNull,
    );
    expect(
      tester
          .widget<FilledButton>(find.widgetWithText(FilledButton, 'Yellow'))
          .onPressed,
      isNull,
    );

    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets('a second touch outside cannot cancel the owning drag', (
    tester,
  ) async {
    final bridge = _ControlledBridge(format: FlutterBookFormat.epub);
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book.epub');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();
    final surface = find.byKey(const ValueKey('reader-selection-surface'));
    final bounds = tester.getRect(surface);
    final side = bounds.shortestSide;
    final topLeft = bounds.center - Offset(side / 2, side / 2);
    final owner = await tester.createGesture(kind: ui.PointerDeviceKind.touch);
    await owner.down(topLeft + Offset(side * .2, side * .2));
    await owner.moveBy(Offset(side * .5, side * .5));
    final second = await tester.createGesture(kind: ui.PointerDeviceKind.touch);
    await second.down(tester.getCenter(find.byType(TextField)));
    await tester.pump();

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
    expect(painter.anchor, 1);
    expect(painter.focus, 8);
    await second.up();
    await owner.up();
    await tester.pump();
    expect(find.text('Yellow'), findsOneWidget);

    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets('losing pointer capture cancels a temporary selection', (
    tester,
  ) async {
    final bridge = _ControlledBridge(format: FlutterBookFormat.pdf);
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();

    final surface = find.byKey(const ValueKey('reader-selection-surface'));
    final rect = tester.getRect(surface);
    final side = rect.shortestSide;
    final topLeft = rect.center - Offset(side / 2, side / 2);
    final gesture = await tester.createGesture(
      kind: ui.PointerDeviceKind.mouse,
    );
    await gesture.down(topLeft + Offset(side * .2, side * .2));
    await gesture.moveBy(Offset(side * .5, side * .5));
    await tester.pump();
    expect(
      (tester
                  .widget<CustomPaint>(
                    find.byWidgetPredicate(
                      (widget) =>
                          widget is CustomPaint &&
                          widget.painter is PagePainter,
                    ),
                  )
                  .painter
              as PagePainter)
          .focus,
      8,
      reason: 'the test must establish a non-empty drag before cancellation',
    );
    await gesture.cancel();
    await tester.pump();

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
    expect(painter.anchor, isNull);
    expect(find.text('Yellow'), findsNothing);
    expect(bridge.createCalls, 0);

    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });

  testWidgets('selection note action saves the entered note', (tester) async {
    final bridge = _ControlledBridge(format: FlutterBookFormat.epub);
    await tester.pumpWidget(
      MaterialApp(
        home: ReaderScreen(
          bridge: bridge,
          decoder: (pixels, {required width, required height}) => _testImage(),
        ),
      ),
    );
    await tester.enterText(find.byType(TextField), '/tmp/book');
    await tester.tap(find.text('Open document'));
    await tester.pumpAndSettle();
    final surface = find.byKey(const ValueKey('reader-selection-surface'));
    final rect = tester.getRect(surface);
    final side = rect.shortestSide;
    final topLeft = rect.center - Offset(side / 2, side / 2);
    final gesture = await tester.createGesture(
      kind: ui.PointerDeviceKind.mouse,
    );
    await gesture.down(topLeft + Offset(side * .2, side * .2));
    await gesture.moveBy(Offset(side * .5, side * .5));
    await gesture.up();
    await tester.pump();

    await tester.tap(find.text('Add note'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).last, 'Remember this');
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();

    expect(bridge.storedAnnotations.single.body, 'Remember this');
    expect(bridge.storedAnnotations.single.color, FlutterHighlightColor.yellow);
    await tester.pumpWidget(const SizedBox());
    await bridge.disposed.future;
  });
}

FlutterAnnotation _annotation(String id) => FlutterAnnotation(
  id: id,
  unit: BigInt.zero,
  resolution: FlutterAnnotationResolution.exact,
  textRange: FlutterAnnotationTextRange(start: BigInt.one, end: BigInt.from(3)),
  color: FlutterHighlightColor.yellow,
);

FlutterRenderedBuffer _buffer(BigInt id) => FlutterRenderedBuffer(
  handle: FlutterBufferHandle(registry: BigInt.one, id: id),
  width: 1,
  height: 1,
  byteLen: BigInt.from(4),
);

FlutterSelectionSurface _surface(
  BigInt id, {
  required bool raster,
  String text = 'x',
}) => FlutterSelectionSurface(
  handle: FlutterSelectionHandle(registry: BigInt.one, id: id),
  width: 1,
  height: 1,
  text: text,
  copyEligible: true,
  raster: raster ? _buffer(id) : null,
  endpoints: const [],
  graphemeBoundaries: Uint32List.fromList([0, 1]),
  wordBoundaries: Uint32List.fromList([0, 1]),
  visualLines: const [],
);

Future<void> _waitUntil(bool Function() condition) async {
  while (!condition()) {
    await Future<void>.delayed(Duration.zero);
  }
}

List<FlutterAnnotation> _resolutionAnnotations() => [
  FlutterAnnotation(
    id: 'recovered',
    unit: BigInt.zero,
    resolution: FlutterAnnotationResolution.recovered,
    textRange: FlutterAnnotationTextRange(
      start: BigInt.one,
      end: BigInt.from(3),
    ),
    color: FlutterHighlightColor.yellow,
  ),
  FlutterAnnotation(
    id: 'ambiguous',
    unit: BigInt.zero,
    resolution: FlutterAnnotationResolution.ambiguous,
    color: FlutterHighlightColor.green,
  ),
  FlutterAnnotation(
    id: 'orphaned',
    unit: BigInt.zero,
    resolution: FlutterAnnotationResolution.orphaned,
    color: FlutterHighlightColor.blue,
  ),
];

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
    this.selectionVisualLines,
    this.copyEligible = true,
  }) : initialAnnotations = List.of(initialAnnotations),
       storedAnnotations = List.of(initialAnnotations);

  FlutterBookFormat format;
  final List<FlutterAnnotation> initialAnnotations;
  final List<FlutterAnnotation> storedAnnotations;
  final bool selectionFailure;
  bool listFailure;
  final bool immediateLists;
  final Completer<List<FlutterAnnotation>>? initialListCompleter;
  final List<FlutterSelectionVisualLine>? selectionVisualLines;
  final bool copyEligible;
  final disposed = Completer<void>();
  final createdCancellations = <BigInt>[];
  final releasedCancellations = <BigInt>[];
  final cancelled = <BigInt>[];
  final selectionCompleters = Queue<Completer<FlutterSelectionSurface>>();
  final renderCompleters = Queue<Completer<FlutterRenderedBuffer>>();
  final listCompleters = Queue<Completer<List<FlutterAnnotation>>>();
  final takenBuffers = <FlutterBufferHandle>[];
  final releasedBuffers = <FlutterBufferHandle>[];
  final releasedSelections = <FlutterSelectionHandle>[];
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
  var failCancellationCreation = false;
  var _nextId = BigInt.one;
  final _listedDocuments = <BigInt>{};
  final createdRanges = <(BigInt, BigInt)>[];
  final createdScales = <double>[];
  final selectionLayouts = <ReaderLayout>[];
  final renderScales = <double>[];
  final listScales = <double>[];

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
    if (failCancellationCreation) {
      throw const FlutterBridgeError(
        kind: FlutterBridgeErrorKind.invalidRequest,
        message: 'too many cancellation tokens',
      );
    }
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
    selectionLayouts.add(
      ReaderLayout(scale: scale, width: width, fontSize: fontSize),
    );
    if (selectionFailure) throw StateError('selection failed');
    if (selectionCompleters.isNotEmpty) {
      return selectionCompleters.removeFirst().future;
    }
    return FlutterSelectionSurface(
      handle: FlutterSelectionHandle(registry: BigInt.one, id: cancellationId),
      width: 100,
      height: 100,
      text: 'Selectable fixture text',
      copyEligible: copyEligible,
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
      graphemeBoundaries: Uint32List.fromList([1, 2, 3, 4, 5, 8]),
      wordBoundaries: Uint32List.fromList([1, 4, 8]),
      visualLines:
          selectionVisualLines ??
          [
            FlutterSelectionVisualLine(
              carets: [
                FlutterSelectionCaret(
                  offset: BigInt.one,
                  x: 20,
                  alongLine: 20,
                  vertical: false,
                  top: 10,
                  bottom: 30,
                ),
                FlutterSelectionCaret(
                  offset: BigInt.from(2),
                  x: 30,
                  alongLine: 30,
                  vertical: false,
                  top: 10,
                  bottom: 30,
                ),
                FlutterSelectionCaret(
                  offset: BigInt.from(3),
                  x: 40,
                  alongLine: 40,
                  vertical: false,
                  top: 10,
                  bottom: 30,
                ),
                FlutterSelectionCaret(
                  offset: BigInt.from(4),
                  x: 50,
                  alongLine: 50,
                  vertical: false,
                  top: 10,
                  bottom: 30,
                ),
              ],
            ),
            FlutterSelectionVisualLine(
              carets: [
                FlutterSelectionCaret(
                  offset: BigInt.from(4),
                  x: 20,
                  alongLine: 20,
                  vertical: false,
                  top: 60,
                  bottom: 80,
                ),
                FlutterSelectionCaret(
                  offset: BigInt.from(5),
                  x: 30,
                  alongLine: 30,
                  vertical: false,
                  top: 60,
                  bottom: 80,
                ),
                FlutterSelectionCaret(
                  offset: BigInt.from(8),
                  x: 70,
                  alongLine: 70,
                  vertical: false,
                  top: 60,
                  bottom: 80,
                ),
              ],
            ),
          ],
    );
  }

  @override
  Future<List<FlutterAnnotation>> listAnnotations({
    required FlutterDocumentHandle document,
    required double scale,
    required BigInt cancellationId,
  }) {
    listCalls += 1;
    listScales.add(scale);
    if (listFailure) return Future.error(StateError('annotation list failed'));
    if (listCompleters.isNotEmpty) {
      return listCompleters.removeFirst().future;
    }
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
    required double displayScale,
    required FlutterHighlightColor color,
    String? body,
    required BigInt cancellationId,
  }) async {
    createCalls += 1;
    createdRanges.add((start, end));
    createdScales.add(displayScale);
    final created =
        await (createCompleter?.future ??
            Future.value(
              FlutterAnnotation(
                id: 'created-$createCalls',
                unit: unit,
                resolution: FlutterAnnotationResolution.exact,
                textRange: FlutterAnnotationTextRange(start: start, end: end),
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
          resolution: current.resolution,
          textRange: current.textRange,
          quote: current.quote,
          rectangles: current.rectangles,
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
    renderScales.add(scale);
    if (renderCompleters.isNotEmpty) {
      return renderCompleters.removeFirst().future;
    }
    return FlutterRenderedBuffer(
      handle: FlutterBufferHandle(registry: BigInt.one, id: cancellationId),
      width: 1,
      height: 1,
      byteLen: BigInt.from(4),
    );
  }

  @override
  Uint8List takeBuffer({required FlutterBufferHandle handle}) {
    takenBuffers.add(handle);
    return Uint8List.fromList([255, 255, 255, 255]);
  }

  @override
  bool releaseBuffer({required FlutterBufferHandle handle}) {
    releasedBuffers.add(handle);
    return true;
  }

  @override
  bool releaseSelection({required FlutterSelectionHandle handle}) {
    releasedSelections.add(handle);
    return true;
  }

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
  final selectionLayouts = <ReaderLayout>[];
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
    required double scale,
    required BigInt cancellationId,
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
    required double displayScale,
    required FlutterHighlightColor color,
    String? body,
    required BigInt cancellationId,
  }) async => FlutterAnnotation(
    id: 'annotation',
    unit: unit,
    resolution: FlutterAnnotationResolution.exact,
    textRange: FlutterAnnotationTextRange(start: start, end: end),
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
    selectionLayouts.add(
      ReaderLayout(scale: scale, width: width, fontSize: fontSize),
    );
    if (selectionCompleter case final pending?) return pending.future;
    return FlutterSelectionSurface(
      handle: FlutterSelectionHandle(registry: BigInt.one, id: cancellationId),
      width: 100,
      height: 100,
      text: 'Selectable fixture text',
      copyEligible: true,
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
      graphemeBoundaries: Uint32List(0),
      wordBoundaries: Uint32List(0),
      visualLines: const [],
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
    required double scale,
    required BigInt cancellationId,
  }) async => const [];
  @override
  Future<FlutterAnnotation> createAnnotation({
    required FlutterDocumentHandle document,
    required BigInt unit,
    required BigInt start,
    required BigInt end,
    required double displayScale,
    required FlutterHighlightColor color,
    String? body,
    required BigInt cancellationId,
  }) async => FlutterAnnotation(
    id: 'annotation',
    unit: unit,
    resolution: FlutterAnnotationResolution.exact,
    textRange: FlutterAnnotationTextRange(start: start, end: end),
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
      copyEligible: true,
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
      graphemeBoundaries: Uint32List(0),
      wordBoundaries: Uint32List(0),
      visualLines: const [],
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
