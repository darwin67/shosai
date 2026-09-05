import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shosai_flutter/main.dart';

void main() {
  testWidgets('welcome panel describes the native bridge action', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(home: Scaffold(body: WelcomePanel())),
    );

    expect(find.textContaining('generated Rust bridge'), findsOneWidget);
  });
}
