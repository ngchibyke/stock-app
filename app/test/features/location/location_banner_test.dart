import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/strings.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/features/location/location_banner.dart';
import 'package:market/features/location/location_check.dart';
import 'package:market/features/location/location_prefs.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _FakeCheck implements LocationCheck {
  _FakeCheck({this.area = Area.outside, this.grant = true});

  final Area area;
  final bool grant;
  int permissionRequests = 0;

  @override
  Future<bool> requestPermission() async {
    permissionRequests++;
    return grant;
  }

  @override
  Future<Area> currentArea() async => area;
}

void main() {
  var now = DateTime(2026, 11, 25, 10);

  Future<_FakeCheck> pump(
    WidgetTester tester, {
    Area area = Area.outside,
    bool grant = true,
    Map<String, Object> stored = const {},
  }) async {
    SharedPreferences.setMockInitialValues(stored);
    final check = _FakeCheck(area: area, grant: grant);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          locationCheckProvider.overrideWithValue(check),
          locationClockProvider.overrideWithValue(() => now),
        ],
        child: MaterialApp(
          theme: lightTheme(),
          home: const Scaffold(body: LocationNotice()),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return check;
  }

  setUp(() => now = DateTime(2026, 11, 25, 10));

  testWidgets('first open asks; Not now never prompts and shows no banner', (
    tester,
  ) async {
    final check = await pump(tester);
    expect(find.text(Strings.locationAskTitle), findsOneWidget);
    await tester.tap(find.text(Strings.locationNotNow));
    await tester.pumpAndSettle();
    expect(check.permissionRequests, 0);
    expect(find.text(Strings.locationComingSoon), findsNothing);
    final prefs = await SharedPreferences.getInstance();
    expect(prefs.getBool(LocationPrefs.askedKey), isTrue);
  });

  testWidgets('Allow outside the FCT shows the banner', (tester) async {
    final check = await pump(tester);
    await tester.tap(find.text(Strings.locationAllow));
    await tester.pumpAndSettle();
    expect(check.permissionRequests, 1);
    expect(find.text(Strings.locationComingSoon), findsOneWidget);
  });

  testWidgets('Allow inside the FCT shows no banner', (tester) async {
    await pump(tester, area: Area.inside);
    await tester.tap(find.text(Strings.locationAllow));
    await tester.pumpAndSettle();
    expect(find.text(Strings.locationComingSoon), findsNothing);
  });

  testWidgets('a refused or failed check shows no banner', (tester) async {
    await pump(tester, area: Area.unknown, grant: false);
    await tester.tap(find.text(Strings.locationAllow));
    await tester.pumpAndSettle();
    expect(find.text(Strings.locationComingSoon), findsNothing);
  });

  testWidgets('already asked and outside: banner without the sheet', (
    tester,
  ) async {
    final check = await pump(tester, stored: {LocationPrefs.askedKey: true});
    expect(find.text(Strings.locationAskTitle), findsNothing);
    expect(check.permissionRequests, 0);
    expect(find.text(Strings.locationComingSoon), findsOneWidget);
  });

  testWidgets('closed today stays hidden today and returns tomorrow', (
    tester,
  ) async {
    await pump(tester, stored: {LocationPrefs.askedKey: true});
    await tester.tap(find.bySemanticsLabel(Strings.locationClose));
    await tester.pumpAndSettle();
    expect(find.text(Strings.locationComingSoon), findsNothing);

    final stored = {
      LocationPrefs.askedKey: true,
      LocationPrefs.closedOnKey: '2026-11-25',
    };
    await tester.pumpWidget(const SizedBox());
    await pump(tester, stored: stored);
    expect(find.text(Strings.locationComingSoon), findsNothing);

    await tester.pumpWidget(const SizedBox());
    now = DateTime(2026, 11, 26, 9);
    await pump(tester, stored: stored);
    expect(find.text(Strings.locationComingSoon), findsOneWidget);
  });

  testWidgets('the sheet fits at 200 percent text on a small phone', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(320, 480);
    tester.view.devicePixelRatio = 1;
    tester.platformDispatcher.textScaleFactorTestValue = 2.0;
    addTearDown(tester.view.reset);
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    await pump(tester);
    expect(tester.takeException(), isNull);
    expect(find.text(Strings.locationAskTitle), findsOneWidget);
    await tester.ensureVisible(find.text(Strings.locationNotNow));
    await tester.tap(find.text(Strings.locationNotNow));
    await tester.pumpAndSettle();
    expect(find.text(Strings.locationAskTitle), findsNothing);
  });
}
