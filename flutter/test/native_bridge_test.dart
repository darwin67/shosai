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
