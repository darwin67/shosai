import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show ExternalLibrary;
import 'package:shosai_flutter/src/rust/api.dart';
import 'package:shosai_flutter/src/rust/frb_generated.dart';

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
      title: 'Shōsai',
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

typedef PageDecoder =
    Future<ui.Image> Function(
      Uint8List pixels, {
      required int width,
      required int height,
    });

class ReaderScreen extends StatefulWidget {
  const ReaderScreen({super.key, this.bridge, this.decoder = _decodeRgba});

  final FlutterBridge? bridge;
  final PageDecoder decoder;

  @override
  State<ReaderScreen> createState() => _ReaderScreenState();
}

class _ReaderScreenState extends State<ReaderScreen> {
  late final FlutterBridge _bridge = widget.bridge ?? FlutterBridge();
  final TextEditingController _path = TextEditingController();

  FlutterDocumentSummary? _document;
  ui.Image? _pageImage;
  BigInt? _activeCancellation;
  String? _error;
  bool _busy = false;
  int _operation = 0;
  int _activeBridgeOperations = 0;
  bool _disposeBridgeWhenIdle = false;

  @override
  void dispose() {
    _operation += 1;
    _disposeBridgeWhenIdle = true;
    final cancellation = _activeCancellation;
    if (cancellation != null) {
      _bridge.cancel(id: cancellation);
    }
    _releaseDocument();
    _path.dispose();
    _disposeBridgeIfIdle();
    super.dispose();
  }

  Future<void> _open() async {
    final path = _path.text.trim();
    if (path.isEmpty || _busy) return;

    final operation = ++_operation;
    _releaseDocument();
    final cancellation = _bridge.createCancellation();
    _activeBridgeOperations += 1;
    _activeCancellation = cancellation;
    setState(() {
      _busy = true;
      _error = null;
    });

    FlutterDocumentSummary? opened;
    try {
      opened = await _bridge.openDocument(
        request: FlutterOpenRequest(localId: path, pathKey: path),
        cancellationId: cancellation,
      );
      if (!mounted || operation != _operation) {
        _bridge.releaseDocument(handle: opened.handle);
        return;
      }
      _document = opened;
      if (opened.format != FlutterBookFormat.epub) {
        final rendered = await _bridge.renderPage(
          document: opened.handle,
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
        if (opened.format == FlutterBookFormat.cbz) {
          premultiplyRgba(pixels);
        }
        final image = await widget.decoder(
          pixels,
          width: rendered.width,
          height: rendered.height,
        );
        if (!mounted || operation != _operation) {
          image.dispose();
          return;
        }
        _pageImage?.dispose();
        _pageImage = image;
      }
    } on FlutterBridgeError catch (error) {
      if (mounted && operation == _operation) {
        _error = error.message;
      }
      _releaseOpenedDocument(opened);
    } catch (error) {
      if (mounted && operation == _operation) {
        _error = error.toString();
      }
      _releaseOpenedDocument(opened);
    } finally {
      _bridge.releaseCancellation(id: cancellation);
      if (_activeCancellation == cancellation) {
        _activeCancellation = null;
      }
      if (mounted && operation == _operation) {
        setState(() => _busy = false);
      }
      _activeBridgeOperations -= 1;
      _disposeBridgeIfIdle();
    }
  }

  void _disposeBridgeIfIdle() {
    if (_disposeBridgeWhenIdle &&
        _activeBridgeOperations == 0 &&
        !_bridge.isDisposed) {
      _bridge.dispose();
    }
  }

  void _releaseDocument() {
    _pageImage?.dispose();
    _pageImage = null;
    final document = _document;
    _document = null;
    if (document != null) {
      _bridge.releaseDocument(handle: document.handle);
    }
  }

  void _releaseOpenedDocument(FlutterDocumentSummary? opened) {
    if (opened != null && _document?.handle == opened.handle) {
      _bridge.releaseDocument(handle: opened.handle);
      _document = null;
    }
  }

  @override
  Widget build(BuildContext context) {
    final document = _document;
    return Scaffold(
      appBar: AppBar(title: const Text('Shōsai Flutter feasibility slice')),
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Semantics(
                textField: true,
                label: 'Document path',
                child: TextField(
                  controller: _path,
                  enabled: !_busy,
                  onSubmitted: (_) => _open(),
                  decoration: const InputDecoration(
                    border: OutlineInputBorder(),
                    hintText: '/path/to/book.pdf',
                    labelText: 'PDF, EPUB, or CBZ path',
                  ),
                ),
              ),
              const SizedBox(height: 12),
              Align(
                alignment: Alignment.centerLeft,
                child: FilledButton.icon(
                  onPressed: _busy ? null : _open,
                  icon: const Icon(Icons.menu_book),
                  label: Text(_busy ? 'Opening…' : 'Open document'),
                ),
              ),
              if (_error != null) ...[
                const SizedBox(height: 12),
                Semantics(
                  liveRegion: true,
                  child: Text(
                    _error!,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ),
              ],
              const SizedBox(height: 20),
              Expanded(
                child: document == null
                    ? const WelcomePanel()
                    : _DocumentView(document: document, image: _pageImage),
              ),
            ],
          ),
        ),
      ),
    );
  }
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
  const _DocumentView({required this.document, required this.image});

  final FlutterDocumentSummary document;
  final ui.Image? image;

  @override
  Widget build(BuildContext context) {
    final title = document.title ?? 'Untitled document';
    if (document.format == FlutterBookFormat.epub) {
      return Semantics(
        label: '$title, EPUB, ${document.logicalUnitCount} chapters',
        child: Center(
          child: Text(
            '$title\n${document.logicalUnitCount} chapters\n\n'
            'EPUB scene transfer is the next M2 slice.',
            textAlign: TextAlign.center,
          ),
        ),
      );
    }
    final page = image;
    if (page == null) {
      return const Center(child: CircularProgressIndicator());
    }
    return Semantics(
      image: true,
      label: '$title, page 1 of ${document.logicalUnitCount}',
      child: CustomPaint(
        painter: _PagePainter(page),
        child: const SizedBox.expand(),
      ),
    );
  }
}

class _PagePainter extends CustomPainter {
  const _PagePainter(this.image);

  final ui.Image image;

  @override
  void paint(Canvas canvas, Size size) {
    final source = Rect.fromLTWH(
      0,
      0,
      image.width.toDouble(),
      image.height.toDouble(),
    );
    final scale = (size.width / source.width).clamp(
      0.0,
      size.height / source.height,
    );
    final destinationSize = Size(source.width * scale, source.height * scale);
    final destination = Alignment.center.inscribe(
      destinationSize,
      Offset.zero & size,
    );
    canvas.drawImageRect(image, source, destination, Paint());
  }

  @override
  bool shouldRepaint(_PagePainter oldDelegate) => oldDelegate.image != image;
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

Future<ui.Image> _decodeRgba(
  Uint8List pixels, {
  required int width,
  required int height,
}) async {
  final buffer = await ui.ImmutableBuffer.fromUint8List(pixels);
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
    buffer.dispose();
  }
}
