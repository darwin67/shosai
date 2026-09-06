import 'dart:async';
import 'dart:typed_data';

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
    await tester.pump();

    expect(tester.takeException(), isNull);
    expect(bridge.releasedDocuments, [_documentHandle]);
    expect(bridge.disposeCount, 1);
  });

  testWidgets('an in-flight render releases its returned buffer on disposal', (
    tester,
  ) async {
    final bridge = _FakeBridge()..completeOpenAsPdf();
    await tester.pumpWidget(MaterialApp(home: ReaderScreen(bridge: bridge)));
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
    await tester.pump();

    expect(tester.takeException(), isNull);
    expect(bridge.releasedBuffers, [_bufferHandle]);
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
  final openCompleter = Completer<FlutterDocumentSummary>();
  final renderCompleter = Completer<FlutterRenderedBuffer>();
  final renderStarted = Completer<void>();
  final releasedDocuments = <FlutterDocumentHandle>[];
  final releasedBuffers = <FlutterBufferHandle>[];
  var disposeCount = 0;

  void completeOpenAsPdf() {
    openCompleter.complete(
      FlutterDocumentSummary(
        handle: _documentHandle,
        format: FlutterBookFormat.pdf,
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
  }

  @override
  bool cancel({required BigInt id}) {
    _ensureAlive();
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
    return openCompleter.future;
  }

  @override
  bool releaseBuffer({required FlutterBufferHandle handle}) {
    _ensureAlive();
    releasedBuffers.add(handle);
    return true;
  }

  @override
  bool releaseCancellation({required BigInt id}) {
    _ensureAlive();
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
  }) {
    _ensureAlive();
    renderStarted.complete();
    return renderCompleter.future;
  }

  @override
  Uint8List takeBuffer({required FlutterBufferHandle handle}) {
    _ensureAlive();
    return Uint8List.fromList([255, 255, 255, 255]);
  }
}
