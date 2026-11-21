import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/theme/theme.dart';
import 'package:material_ui/material_ui.dart';

/// Pumps [child] in a themed app on a narrow phone-sized surface (320 wide by
/// default), with the accessibility knobs the design system must survive:
/// text scale, reduced motion and a keyboard inset.
Future<void> pumpApp(
  WidgetTester tester,
  Widget child, {
  double textScale = 1.0,
  bool disableAnimations = false,
  EdgeInsets viewInsets = EdgeInsets.zero,
  Size size = const Size(320, 640),
  ThemeData? theme,
  bool scaffold = true,
}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1.0;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      theme: theme ?? lightTheme(),
      home: scaffold ? Scaffold(body: child) : child,
      builder: (context, home) => MediaQuery(
        data: MediaQuery.of(context).copyWith(
          textScaler: TextScaler.linear(textScale),
          disableAnimations: disableAnimations,
          viewInsets: viewInsets,
        ),
        child: home!,
      ),
    ),
  );
}
