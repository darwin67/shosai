import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart'
    show ExternalLibrary;
import 'package:shosai_flutter/reader_controller.dart';
import 'package:shosai_flutter/src/rust/api.dart';
import 'package:shosai_flutter/src/rust/frb_generated.dart';

export 'package:shosai_flutter/reader_controller.dart'
    show
        PageDecoder,
        ReaderController,
        ReaderMessage,
        ReaderModel,
        ReaderOpenRequested,
        premultiplyRgba;

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

class ReaderScreen extends StatefulWidget {
  const ReaderScreen({super.key, this.bridge, this.decoder = _decodeRgba});

  final FlutterBridge? bridge;
  final PageDecoder decoder;

  @override
  State<ReaderScreen> createState() => _ReaderScreenState();
}

class _ReaderScreenState extends State<ReaderScreen> {
  final TextEditingController _path = TextEditingController();
  late final ReaderController _controller;

  @override
  void initState() {
    super.initState();
    _controller = ReaderController(
      bridge: widget.bridge ?? FlutterBridge(),
      decoder: widget.decoder,
    )..addListener(_modelChanged);
  }

  void _modelChanged() => setState(() {});

  @override
  void dispose() {
    _controller.removeListener(_modelChanged);
    _controller.dispose();
    _path.dispose();
    super.dispose();
  }

  void _open() => _controller.dispatch(ReaderOpenRequested(_path.text));

  @override
  Widget build(BuildContext context) {
    final model = _controller.model;
    final document = model.document;
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
                  enabled: !model.busy,
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
                  onPressed: model.busy ? null : _open,
                  icon: const Icon(Icons.menu_book),
                  label: Text(model.busy ? 'Opening…' : 'Open document'),
                ),
              ),
              if (model.error != null) ...[
                const SizedBox(height: 12),
                Semantics(
                  liveRegion: true,
                  child: Text(
                    model.error!,
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
                    : _DocumentView(document: document, image: model.pageImage),
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
