// Renders the app's real screens offscreen with the SDK's own Roboto and Material
// icon fonts, and writes them as PNGs: a way to look at the app without a device.
//
//   flutter test tool/screenshots_test.dart
//   SHOTS_DIR=/some/dir flutter test tool/screenshots_test.dart
//
// It is not part of `flutter test` (that runs test/ only). Pictures are not golden
// files: nothing compares them, and they are not committed.
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market_app/core/api/api_error.dart';
import 'package:market_app/core/providers.dart';
import 'package:market_app/features/auth/auth_models.dart';
import 'package:market_app/main.dart';
import 'package:market_app/ui/app_icon.dart';
import 'package:market_app/ui/buttons.dart';
import 'package:market_app/ui/cards.dart';
import 'package:market_app/ui/states.dart';
import 'package:market_app/ui/theme.dart';
import 'package:material_ui/material_ui.dart';

import '../test/support/fakes.dart';

final _key = GlobalKey();
late final String _dir;

Future<void> _loadFonts() async {
  final root = Platform.environment['FLUTTER_ROOT'] ?? '/root/flutter';
  final fonts = '$root/bin/cache/artifacts/material_fonts';
  Future<void> load(String family, List<String> files) async {
    final loader = FontLoader(family);
    for (final f in files) {
      loader.addFont(
        Future.value(ByteData.view(File('$fonts/$f').readAsBytesSync().buffer)),
      );
    }
    await loader.load();
  }

  await load('Roboto', [
    'Roboto-Regular.ttf',
    'Roboto-Medium.ttf',
    'Roboto-Bold.ttf',
    'Roboto-Light.ttf',
    'Roboto-Black.ttf',
  ]);
  await load('MaterialIcons', ['MaterialIcons-Regular.otf']);
}

Future<void> _shot(WidgetTester tester, String name) async {
  final boundary = tester.renderObject<RenderRepaintBoundary>(find.byKey(_key));
  await tester.runAsync(() async {
    final image = await boundary.toImage(pixelRatio: 3);
    final data = await image.toByteData(format: ui.ImageByteFormat.png);
    File('$_dir/$name.png').writeAsBytesSync(data!.buffer.asUint8List());
  });
}

Future<void> _phone(WidgetTester tester) async {
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

Future<ProviderContainer> _app(
  WidgetTester tester,
  FakeAuthRepository repo,
) async {
  await _phone(tester);
  final container = ProviderContainer(
    overrides: [authRepositoryProvider.overrideWithValue(repo)],
  );
  addTearDown(container.dispose);
  await tester.pumpWidget(
    UncontrolledProviderScope(
      container: container,
      child: RepaintBoundary(key: _key, child: const MarketApp()),
    ),
  );
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 100));
  return container;
}

Future<void> _settle(WidgetTester tester, [int ms = 700]) async {
  await tester.pump(); // start any animation, then let it run
  await tester.pump(Duration(milliseconds: ms));
  await tester.pump(Duration(milliseconds: ms));
}

/// A widget on the app's own theme, in the same phone frame.
Future<void> _page(
  WidgetTester tester,
  Widget child, {
  Brightness brightness = Brightness.light,
}) async {
  await _phone(tester);
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: brightness == Brightness.light
          ? AppTheme.light()
          : AppTheme.dark(),
      home: RepaintBoundary(
        key: _key,
        child: Scaffold(body: SafeArea(child: child)),
      ),
    ),
  );
  await tester.pump(const Duration(milliseconds: 600));
}

void main() {
  setUpAll(() async {
    TestWidgetsFlutterBinding.ensureInitialized();
    _dir = Platform.environment['SHOTS_DIR'] ?? 'build/screenshots';
    Directory(_dir).createSync(recursive: true);
    await _loadFonts();
  });

  testWidgets('welcome', (tester) async {
    await _app(tester, FakeAuthRepository());
    await _settle(tester, 900);
    await _shot(tester, '01-welcome');
  });

  testWidgets('sign-up form, filled', (tester) async {
    await _app(tester, FakeAuthRepository());
    await _settle(tester);
    await tester.tap(find.text('Create an account'));
    await _settle(tester);
    await tester.enterText(
      find.widgetWithText(TextField, 'Your name'),
      'Amaka Obi',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Phone number'),
      '08031112222',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Password'),
      'a-long-password',
    );
    await tester.pump(const Duration(milliseconds: 400));
    await _shot(tester, '02-sign-up');
  });

  testWidgets('code screen, part-entered', (tester) async {
    await _app(tester, FakeAuthRepository());
    await _settle(tester);
    await tester.tap(find.text('Create an account'));
    await _settle(tester);
    await tester.enterText(
      find.widgetWithText(TextField, 'Your name'),
      'Amaka Obi',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Phone number'),
      '08031112222',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Password'),
      'a-long-password',
    );
    await tester.pump();
    await tester.tap(find.text('Continue'));
    await _settle(tester);
    await tester.enterText(find.byType(TextField), '482');
    await tester.pump(); // the ticker takes its start time from this frame
    await tester.pump(const Duration(milliseconds: 500));
    await _shot(tester, '03-code');
  });

  testWidgets('wrong password', (tester) async {
    final repo = FakeAuthRepository()
      ..signInError = const ApiError(401, 'invalid_credentials', 'x');
    await _app(tester, repo);
    await _settle(tester);
    await tester.tap(find.text('I already have an account'));
    await _settle(tester);
    await tester.enterText(
      find.widgetWithText(TextField, 'Phone number'),
      '08031112222',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Password'),
      'nope-nope-nope',
    );
    await tester.pump();
    await tester.tap(find.text('Sign in').last);
    await _settle(tester, 900);
    await _shot(tester, '04-sign-in-error');
  });

  testWidgets('account, light and dark', (tester) async {
    await _app(
      tester,
      FakeAuthRepository(
        stored: const Me(
          id: 'u1',
          displayName: 'Amaka Obi',
          phone: '+2348031112222',
          role: Role.user,
          isSeller: false,
        ),
      ),
    );
    await _settle(tester, 900);
    await _shot(tester, '05-account-light');
    await tester.tap(find.text('Dark'));
    await _settle(tester, 900);
    await _shot(tester, '06-account-dark');
  });

  testWidgets('buttons', (tester) async {
    for (final b in [Brightness.light, Brightness.dark]) {
      await _page(
        tester,
        Builder(
          builder: (context) => Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Text(
                  'Buttons',
                  style: Theme.of(context).textTheme.headlineSmall
                      ?.copyWith(fontWeight: FontWeight.w800),
                ),
                const SizedBox(height: 20),
                AppButton(
                  label: 'Pay now',
                  icon: AppIcons.lock,
                  expand: true,
                  onPressed: () {},
                ),
                const SizedBox(height: 12),
                AppButton(
                  label: 'Paying…',
                  expand: true,
                  loading: true,
                  onPressed: () {},
                ),
                const SizedBox(height: 12),
                AppButton(
                  label: 'Saved',
                  expand: true,
                  done: true,
                  onPressed: () {},
                ),
                const SizedBox(height: 12),
                AppButton(
                  label: 'Add to cart',
                  icon: AppIcons.cart,
                  style: AppButtonStyle.secondary,
                  expand: true,
                  onPressed: () {},
                ),
                const SizedBox(height: 12),
                AppButton(
                  label: 'Report a problem',
                  icon: AppIcons.flag,
                  style: AppButtonStyle.danger,
                  expand: true,
                  onPressed: () {},
                ),
                const SizedBox(height: 12),
                AppButton(
                  label: 'Not now',
                  style: AppButtonStyle.ghost,
                  expand: true,
                  onPressed: () {},
                ),
                const SizedBox(height: 12),
                const AppButton(
                  label: 'Unavailable',
                  expand: true,
                  onPressed: null,
                ),
              ],
            ),
          ),
        ),
        brightness: b,
      );
      await tester.pump(const Duration(milliseconds: 400));
      await _shot(
        tester,
        b == Brightness.light ? '07-buttons-light' : '08-buttons-dark',
      );
    }
  });

  testWidgets('icons', (tester) async {
    await _page(
      tester,
      Builder(
        builder: (context) => Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Every icon moves when pressed',
                style: Theme.of(context).textTheme.titleLarge
                    ?.copyWith(fontWeight: FontWeight.w800),
              ),
              const SizedBox(height: 4),
              Text(
                'Each has its own motion: turn, ring, bounce, pulse, slide.',
                style: TextStyle(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: 16),
              Expanded(
                child: GridView.count(
                  crossAxisCount: 4,
                  mainAxisSpacing: 8,
                  crossAxisSpacing: 8,
                  children: [
                    for (final e in AppIcons.all.entries)
                      PressableCard(
                        padding: const EdgeInsets.all(8),
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            AppIcon(
                              e.value,
                              size: 28,
                              color: Theme.of(context).colorScheme.primary,
                            ),
                            const SizedBox(height: 6),
                            Text(
                              e.key,
                              style: const TextStyle(fontSize: 10),
                              overflow: TextOverflow.ellipsis,
                            ),
                            Text(
                              e.value.behaviour.name,
                              style: TextStyle(
                                fontSize: 9,
                                color: Theme.of(context)
                                    .colorScheme
                                    .onSurfaceVariant,
                              ),
                            ),
                          ],
                        ),
                      ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
    await _shot(tester, '09-icons');
  });

  testWidgets('empty and error states', (tester) async {
    await _page(
      tester,
      Column(
        children: [
          const Expanded(
            child: EmptyState(
              icon: AppIcons.orders,
              title: 'No orders yet',
              hint: 'When you buy something, it shows up here so you can track it.',
            ),
          ),
          const Divider(),
          Expanded(
            child: ErrorState(
              error: const ApiError(
                0,
                'network',
                'Could not reach the server. Check your connection and try again.',
              ),
              onRetry: () {},
            ),
          ),
          const Padding(padding: EdgeInsets.all(24), child: Skeleton(lines: 3)),
        ],
      ),
    );
    await _shot(tester, '10-states');
  });
}
