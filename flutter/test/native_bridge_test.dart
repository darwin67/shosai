import 'dart:convert';
import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show ExternalLibrary;
import 'package:flutter_test/flutter_test.dart';
import 'package:shosai_flutter/src/rust/api.dart';
import 'package:shosai_flutter/src/rust/frb_generated.dart';

void main() {
  final supported = Platform.isLinux || Platform.isMacOS;

  setUpAll(() async {
    if (!supported) return;
    final library = Platform.isMacOS
        ? '../target/debug/libshosai_flutter_bridge.dylib'
        : '../target/debug/libshosai_flutter_bridge.so';
    await RustLib.init(externalLibrary: ExternalLibrary.open(library));
  });

  tearDownAll(() {
    if (supported) RustLib.dispose();
  });

  test(
    'opens and renders a PDF through the native bridge',
    () async {
      final bridge = FlutterBridge();
      final cancellation = bridge.createCancellation();
      FlutterDocumentHandle? document;
      FlutterBufferHandle? buffer;
      try {
        final summary = await bridge.openDocument(
          request: const FlutterOpenRequest(
            localId: 'native-bridge-test',
            pathKey: '../crates/shosai-core/tests/fixtures/sample.pdf',
          ),
          cancellationId: cancellation,
        );
        document = summary.handle;
        expect(summary.format, FlutterBookFormat.pdf);

        final surface = await bridge.selectionSurface(
          document: summary.handle,
          unit: BigInt.zero,
          scale: 1,
          width: 680,
          fontSize: 18,
          cancellationId: cancellation,
        );
        expect(surface.width, greaterThan(0));
        expect(surface.height, greaterThan(0));
        expect(surface.endpoints, isNotEmpty);
        expect(
          surface.endpoints.every(
            (endpoint) =>
                endpoint.rangeStart < endpoint.rangeEnd &&
                endpoint.offset >= endpoint.rangeStart &&
                endpoint.offset <= endpoint.rangeEnd,
          ),
          isTrue,
        );
        expect(bridge.releaseSelection(handle: surface.handle), isTrue);
        expect(bridge.releaseSelection(handle: surface.handle), isFalse);

        final rendered = await bridge.renderPage(
          document: summary.handle,
          page: BigInt.zero,
          scale: 1,
          cancellationId: cancellation,
        );
        buffer = rendered.handle;
        final pixels = bridge.takeBuffer(handle: rendered.handle);
        expect(bridge.releaseBuffer(handle: rendered.handle), isTrue);
        expect(bridge.releaseBuffer(handle: rendered.handle), isFalse);
        buffer = null;

        expect(rendered.width, greaterThan(0));
        expect(rendered.height, greaterThan(0));
        expect(pixels, hasLength(rendered.byteLen.toInt()));
        expect(pixels.length, rendered.width * rendered.height * 4);
      } on FlutterBridgeError catch (error) {
        fail('${error.kind}: ${error.message}');
      } finally {
        if (buffer != null) bridge.releaseBuffer(handle: buffer);
        if (document != null) bridge.releaseDocument(handle: document);
        bridge.releaseCancellation(id: cancellation);
        bridge.dispose();
      }
    },
    skip: supported ? false : 'native bridge smoke test supports desktop hosts',
  );

  test(
    'opens and lays out an EPUB selection surface through the native bridge',
    () async {
      final bridge = FlutterBridge();
      final cancellation = bridge.createCancellation();
      FlutterDocumentHandle? document;
      try {
        final summary = await bridge.openDocument(
          request: const FlutterOpenRequest(
            localId: 'native-epub-test',
            pathKey: '../crates/shosai-core/tests/fixtures/sample.epub',
          ),
          cancellationId: cancellation,
        );
        document = summary.handle;
        expect(summary.format, FlutterBookFormat.epub);
        final surface = await bridge.selectionSurface(
          document: summary.handle,
          unit: BigInt.zero,
          scale: 1,
          width: 680,
          fontSize: 18,
          cancellationId: cancellation,
        );
        expect(surface.text, isNotEmpty);
        expect(surface.resourcePath, isNotEmpty);
        expect(surface.endpoints, isNotEmpty);
        final raster = surface.raster!;
        expect(raster.width, greaterThan(0));
        expect(raster.height, greaterThan(0));
        final pixels = bridge.takeBuffer(handle: raster.handle);
        expect(pixels, hasLength(raster.width * raster.height * 4));
        expect(
          pixels.any((component) => component != 0),
          isTrue,
          reason: 'Flutter must receive Rust rasterized glyph pixels',
        );
        expect(bridge.releaseBuffer(handle: raster.handle), isTrue);
        expect(bridge.releaseBuffer(handle: raster.handle), isFalse);
        expect(bridge.releaseSelection(handle: surface.handle), isTrue);
        expect(
          surface.endpoints.every(
            (endpoint) =>
                endpoint.rangeStart < endpoint.rangeEnd &&
                endpoint.offset >= endpoint.rangeStart &&
                endpoint.offset <= endpoint.rangeEnd &&
                endpoint.rect.left >= 0 &&
                endpoint.rect.top >= 0 &&
                endpoint.rect.right <= surface.width &&
                endpoint.rect.bottom <= surface.height,
          ),
          isTrue,
        );
      } finally {
        if (document != null) bridge.releaseDocument(handle: document);
        bridge.releaseCancellation(id: cancellation);
        bridge.dispose();
      }
    },
    skip: supported ? false : 'native bridge smoke test supports desktop hosts',
  );

  Future<void> verifyPersistedHighlight({
    required String format,
    required FlutterBookFormat expectedFormat,
  }) async {
    final directory = await Directory.systemTemp.createTemp(
      'shosai-native-persistence-',
    );
    final databasePath = '${directory.path}/annotations.sqlite';
    final fixture = '../crates/shosai-core/tests/fixtures/sample.$format';
    final localId = 'native-$format-persistence-test';
    FlutterBridge? bridge;
    FlutterDocumentHandle? document;
    BigInt? cancellation;

    Future<FlutterDocumentSummary> open() async {
      bridge = FlutterBridge.withDatabasePath(databasePath: databasePath);
      cancellation = bridge!.createCancellation();
      final summary = await bridge!.openDocument(
        request: FlutterOpenRequest(localId: localId, pathKey: fixture),
        cancellationId: cancellation!,
      );
      document = summary.handle;
      return summary;
    }

    void release() {
      if (document != null) bridge!.releaseDocument(handle: document!);
      if (cancellation != null) {
        bridge!.releaseCancellation(id: cancellation!);
      }
      bridge?.dispose();
      bridge = null;
      document = null;
      cancellation = null;
    }

    try {
      final firstOpen = await open();
      expect(firstOpen.format, expectedFormat);
      final surface = await bridge!.selectionSurface(
        document: document!,
        unit: BigInt.zero,
        scale: 1,
        width: 680,
        fontSize: 18,
        cancellationId: cancellation!,
      );
      final endpoint = surface.endpoints.firstWhere(
        (value) => value.rangeStart < value.rangeEnd,
      );
      expect(endpoint.rect.right, greaterThan(endpoint.rect.left));
      expect(endpoint.rect.bottom, greaterThan(endpoint.rect.top));
      if (surface.raster case final raster?) {
        expect(bridge!.releaseBuffer(handle: raster.handle), isTrue);
      }
      expect(bridge!.releaseSelection(handle: surface.handle), isTrue);
      final created = await bridge!.createAnnotation(
        document: document!,
        unit: BigInt.zero,
        start: endpoint.rangeStart,
        end: endpoint.rangeEnd,
        displayScale: 1,
        color: FlutterHighlightColor.yellow,
        cancellationId: cancellation!,
      );
      expect(created.textRange?.start, endpoint.rangeStart);
      expect(created.textRange?.end, endpoint.rangeEnd);
      expect(created.quote, isNotEmpty);
      if (expectedFormat == FlutterBookFormat.pdf) {
        // Text-backed highlights paint from their range on the retained
        // selection surface; only geometry-only PDF annotations export rects.
        expect(created.rectangles, isEmpty);
      } else {
        expect(created.rectangles, isEmpty);
      }
      expect(await File(databasePath).exists(), isTrue);

      release();
      await open();
      var listed = await bridge!.listAnnotations(
        document: document!,
        scale: 1,
        cancellationId: cancellation!,
      );
      expect(listed, hasLength(1));
      expect(listed.single.id, created.id);
      expect(listed.single.color, FlutterHighlightColor.yellow);
      expect(listed.single.textRange, isNotNull);
      expect(listed.single.quote, created.quote);
      expect(
        listed.single.rectangles,
        orderedEquals(created.rectangles ?? const []),
      );

      expect(
        await bridge!.updateAnnotation(
          document: document!,
          id: created.id,
          color: FlutterHighlightColor.purple,
          body: 'native bridge note',
        ),
        isTrue,
      );
      listed = await bridge!.listAnnotations(
        document: document!,
        scale: 1,
        cancellationId: cancellation!,
      );
      expect(listed, hasLength(1));
      expect(listed.single.color, FlutterHighlightColor.purple);
      expect(listed.single.body, 'native bridge note');

      expect(
        await bridge!.deleteAnnotation(document: document!, id: created.id),
        isTrue,
      );
      expect(
        await bridge!.listAnnotations(
          document: document!,
          scale: 1,
          cancellationId: cancellation!,
        ),
        isEmpty,
      );
    } finally {
      release();
      await directory.delete(recursive: true);
    }
  }

  for (final testCase in [
    (format: 'pdf', expectedFormat: FlutterBookFormat.pdf),
    (format: 'epub', expectedFormat: FlutterBookFormat.epub),
  ]) {
    test(
      'persists a ${testCase.format.toUpperCase()} highlight through the native bridge',
      () => verifyPersistedHighlight(
        format: testCase.format,
        expectedFormat: testCase.expectedFormat,
      ),
      skip: supported
          ? false
          : 'native bridge persistence test supports desktop hosts',
    );
  }

  test(
    'preserves straight alpha when rendering a translucent CBZ page',
    () async {
      const archive =
          'UEsDBBQAAAAIAGSMJV1wvAGaQAAAAEYAAAAIAAAAcGFnZS5wbmfrDPBz5+WS4mJgYOD19HAJAtKMIMzBBiTlRY90giVcHEMq5iT/eH/gAgMDqyPjgY/Zf5YBJRg8Xf1c1jklNAEAUEsBAhQDFAAAAAgAZIwlXXC8AZpAAAAARgAAAAgAAAAAAAAAAAAAAIABAAAAAHBhZ2UucG5nUEsFBgAAAAABAAEANgAAAGYAAAAAAA==';
      final directory = await Directory.systemTemp.createTemp(
        'shosai-flutter-cbz-',
      );
      final fixture = File('${directory.path}/transparent.cbz');
      await fixture.writeAsBytes(base64Decode(archive));

      final bridge = FlutterBridge();
      final cancellation = bridge.createCancellation();
      FlutterDocumentHandle? document;
      FlutterBufferHandle? buffer;
      try {
        final summary = await bridge.openDocument(
          request: FlutterOpenRequest(
            localId: 'transparent-cbz-test',
            pathKey: fixture.path,
          ),
          cancellationId: cancellation,
        );
        document = summary.handle;
        expect(summary.format, FlutterBookFormat.cbz);

        final rendered = await bridge.renderPage(
          document: summary.handle,
          page: BigInt.zero,
          scale: 1,
          cancellationId: cancellation,
        );
        buffer = rendered.handle;
        final pixels = bridge.takeBuffer(handle: rendered.handle);
        expect(pixels, [255, 64, 0, 128]);
        expect(bridge.releaseBuffer(handle: rendered.handle), isTrue);
        buffer = null;
      } finally {
        if (buffer != null) bridge.releaseBuffer(handle: buffer);
        if (document != null) bridge.releaseDocument(handle: document);
        bridge.releaseCancellation(id: cancellation);
        bridge.dispose();
        await directory.delete(recursive: true);
      }
    },
    skip: supported ? false : 'native bridge smoke test supports desktop hosts',
  );
}
