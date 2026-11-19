import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market_app/app/router.dart';
import 'package:market_app/core/api/api_error.dart';
import 'package:market_app/core/providers.dart';
import 'package:market_app/features/auth/auth_controller.dart';
import 'package:market_app/features/auth/phone_format.dart';
import 'package:market_app/main.dart';
import 'package:material_ui/material_ui.dart';

import '../support/fakes.dart';

Future<ProviderContainer> pumpApp(
  WidgetTester tester,
  FakeAuthRepository repo,
) async {
  tester.view.physicalSize = const Size(800, 1600);
  tester.view.devicePixelRatio = 2;
  addTearDown(tester.view.reset);
  final container = ProviderContainer(
    overrides: [authRepositoryProvider.overrideWithValue(repo)],
  );
  addTearDown(container.dispose);
  await tester.pumpWidget(
    UncontrolledProviderScope(container: container, child: const MarketApp()),
  );
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 100));
  return container;
}

Future<void> settle(WidgetTester tester, [int ms = 600]) async {
  await tester.pump(Duration(milliseconds: ms));
  await tester.pump(Duration(milliseconds: ms));
}

void main() {
  group('phone formatting', () {
    test('groups a national number as it is typed', () {
      expect(formatPhoneInput('0803'), '0803');
      expect(formatPhoneInput('08031112222'), '0803 111 2222');
      expect(formatPhoneInput('0803-111-2222'), '0803 111 2222');
      expect(formatPhoneInput('080311122223333'), '0803 111 2222');
    });

    test('keeps the international form', () {
      expect(formatPhoneInput('+2348031112222'), '+234 803 111 2222');
      expect(formatPhoneInput('2348031112222'), '+234 803 111 2222');
    });

    test('sends the server only digits and a plus', () {
      expect(phoneForServer('0803 111 2222'), '08031112222');
      expect(phoneForServer('+234 803 111 2222'), '+2348031112222');
    });
  });

  group('redirect rules', () {
    test('wait on the splash until the session is checked', () {
      expect(
        redirectFor(status: AuthStatus.starting, uri: Uri.parse('/')),
        '/splash',
      );
      expect(
        redirectFor(status: AuthStatus.starting, uri: Uri.parse('/splash')),
        isNull,
      );
    });

    test('a signed-out visitor is sent to the welcome screen, remembering a deep link', () {
      expect(
        redirectFor(status: AuthStatus.signedOut, uri: Uri.parse('/')),
        '/welcome',
      );
      expect(
        redirectFor(status: AuthStatus.signedOut, uri: Uri.parse('/sign-in')),
        isNull,
      );
      expect(
        redirectFor(status: AuthStatus.signedOut, uri: Uri.parse('/order/abc')),
        '/welcome?from=%2Forder%2Fabc',
      );
    });

    test('a signed-in person leaves the sign-in screens, to the deep link if there was one', () {
      expect(
        redirectFor(status: AuthStatus.signedIn, uri: Uri.parse('/welcome')),
        '/',
      );
      expect(
        redirectFor(status: AuthStatus.signedIn, uri: Uri.parse('/splash')),
        '/',
      );
      expect(
        redirectFor(
          status: AuthStatus.signedIn,
          uri: Uri.parse('/welcome?from=%2Forder%2Fabc'),
        ),
        '/order/abc',
      );
      expect(
        redirectFor(status: AuthStatus.signedIn, uri: Uri.parse('/')),
        isNull,
      );
    });

    test('never follows a "from" that leaves the app', () {
      expect(
        redirectFor(
          status: AuthStatus.signedIn,
          uri: Uri.parse('/welcome?from=https%3A%2F%2Fevil.example'),
        ),
        '/',
      );
    });
  });

  group('AuthController', () {
    test('restores a stored session at launch', () async {
      final container = ProviderContainer(
        overrides: [
          authRepositoryProvider.overrideWithValue(
            FakeAuthRepository(stored: testMe),
          ),
        ],
      );
      addTearDown(container.dispose);
      expect(
        container.read(authControllerProvider).status,
        AuthStatus.starting,
      );
      await container.read(authControllerProvider.notifier).start();
      expect(
        container.read(authControllerProvider).status,
        AuthStatus.signedIn,
      );
      expect(container.read(authControllerProvider).user?.firstName, 'Amaka');
    });

    test(
      'goes to sign-in, not a hang, when the launch check is offline',
      () async {
        final repo = FakeAuthRepository()..restoreFails = true;
        final container = ProviderContainer(
          overrides: [authRepositoryProvider.overrideWithValue(repo)],
        );
        addTearDown(container.dispose);
        await container.read(authControllerProvider.notifier).start();
        expect(
          container.read(authControllerProvider).status,
          AuthStatus.signedOut,
        );
      },
    );

    test('a dead session signs the person out', () async {
      final container = ProviderContainer(
        overrides: [
          authRepositoryProvider.overrideWithValue(
            FakeAuthRepository(stored: testMe),
          ),
        ],
      );
      addTearDown(container.dispose);
      final c = container.read(authControllerProvider.notifier);
      await c.start();
      c.sessionEnded();
      expect(
        container.read(authControllerProvider).status,
        AuthStatus.signedOut,
      );
    });
  });

  group('the screens', () {
    testWidgets('a signed-out launch shows the welcome screen', (tester) async {
      await pumpApp(tester, FakeAuthRepository());
      await settle(tester);
      expect(find.text('Create an account'), findsOneWidget);
      expect(find.text('I already have an account'), findsOneWidget);
    });

    testWidgets('a stored session goes straight to the app', (tester) async {
      await pumpApp(tester, FakeAuthRepository(stored: testMe));
      await settle(tester);
      expect(find.text('Amaka Obi'), findsOneWidget);
      expect(find.text('Create an account'), findsNothing);
      // One tab: nothing to switch between, so no bar.
      expect(find.byType(NavigationBar), findsNothing);
    });

    testWidgets('sign-up, then the code, then you are in', (tester) async {
      final repo = FakeAuthRepository();
      await pumpApp(tester, repo);
      await settle(tester);
      await tester.tap(find.text('Create an account'));
      await settle(tester);
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
      await settle(tester);
      expect(repo.calls, contains('signUp 08031112222 Amaka Obi'));
      expect(find.text('Enter the code'), findsOneWidget);
      expect(find.textContaining('0803 111 2222'), findsOneWidget);

      await tester.enterText(find.byType(TextField), '123456');
      await settle(tester);
      expect(repo.calls, contains('verify v-1 123456'));
      expect(find.text('Amaka Obi'), findsOneWidget);
    });

    testWidgets(
      'the continue button waits for a name, a phone and a long enough password',
      (tester) async {
        await pumpApp(tester, FakeAuthRepository());
        await settle(tester);
        await tester.tap(find.text('Create an account'));
        await settle(tester);
        Finder button() => find.byWidgetPredicate(
          (w) =>
              w is Semantics &&
              w.properties.label == 'Continue' &&
              w.properties.button == true,
        );
        expect(
          tester.getSemantics(button()).flagsCollection.isEnabled.name,
          anyOf('isFalse', 'False', 'false'),
        );
        await tester.enterText(
          find.widgetWithText(TextField, 'Your name'),
          'Amaka',
        );
        await tester.enterText(
          find.widgetWithText(TextField, 'Phone number'),
          '08031112222',
        );
        await tester.enterText(
          find.widgetWithText(TextField, 'Password'),
          'short',
        );
        await tester.pump();
        await tester.tap(find.text('Continue'));
        await settle(tester);
        expect(find.text('Enter the code'), findsNothing);
      },
    );

    testWidgets('a taken phone number shows the server\'s message', (
      tester,
    ) async {
      final repo = FakeAuthRepository()
        ..signUpError = const ApiError(
          409,
          'phone_taken',
          'That number already has an account',
        );
      await pumpApp(tester, repo);
      await settle(tester);
      await tester.tap(find.text('Create an account'));
      await settle(tester);
      await tester.enterText(
        find.widgetWithText(TextField, 'Your name'),
        'Amaka',
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
      await settle(tester);
      expect(find.text('That number already has an account'), findsOneWidget);
      expect(find.text('Enter the code'), findsNothing);
    });

    testWidgets(
      'a wrong code is explained, clears the boxes, and lets you try again',
      (tester) async {
        final repo = FakeAuthRepository()
          ..verifyError = const ApiError(
            422,
            'otp_invalid',
            'That code is not right',
          );
        final container = await pumpApp(tester, repo);
        await settle(tester);
        await tester.tap(find.text('Create an account'));
        await settle(tester);
        await tester.enterText(
          find.widgetWithText(TextField, 'Your name'),
          'Amaka',
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
        await settle(tester);
        await tester.enterText(find.byType(TextField), '000000');
        await settle(tester);
        expect(find.text('That code is not right'), findsOneWidget);
        expect(
          container.read(authControllerProvider).status,
          AuthStatus.signedOut,
        );
        // Cleared, so the next try starts from nothing.
        expect(
          tester.widget<TextField>(find.byType(TextField)).controller!.text,
          '',
        );
        repo.verifyError = null;
        await tester.enterText(find.byType(TextField), '123456');
        await settle(tester);
        expect(
          container.read(authControllerProvider).status,
          AuthStatus.signedIn,
        );
      },
    );

    testWidgets(
      'the resend button drains a 30 second ring, then asks for a new code',
      (tester) async {
        final repo = FakeAuthRepository()..needsCode = true;
        await pumpApp(tester, repo);
        await settle(tester);
        await tester.tap(find.text('Create an account'));
        await settle(tester);
        await tester.enterText(
          find.widgetWithText(TextField, 'Your name'),
          'Amaka',
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
        await settle(tester);
        expect(find.textContaining('Send a new code in'), findsOneWidget);
        expect(find.text('Send a new code'), findsNothing);
        await tester.pump(const Duration(seconds: 31));
        expect(find.text('Send a new code'), findsOneWidget);
        await tester.tap(find.text('Send a new code'));
        await settle(tester);
        expect(repo.calls.where((c) => c.startsWith('signIn')).length, 1);
        expect(find.text('We sent you a new code.'), findsOneWidget);
        // The ring starts again.
        expect(find.textContaining('Send a new code in'), findsOneWidget);
      },
    );

    testWidgets('sign-in with the wrong password says so', (tester) async {
      final repo = FakeAuthRepository()
        ..signInError = const ApiError(401, 'invalid_credentials', 'x');
      await pumpApp(tester, repo);
      await settle(tester);
      await tester.tap(find.text('I already have an account'));
      await settle(tester);
      await tester.enterText(
        find.widgetWithText(TextField, 'Phone number'),
        '08031112222',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Password'),
        'nope',
      );
      await tester.pump();
      await tester.tap(find.text('Sign in').last);
      await settle(tester);
      expect(
        find.text('That phone number and password do not match.'),
        findsOneWidget,
      );
    });

    testWidgets('an unverified account is taken to the code screen', (
      tester,
    ) async {
      final repo = FakeAuthRepository()..needsCode = true;
      await pumpApp(tester, repo);
      await settle(tester);
      await tester.tap(find.text('I already have an account'));
      await settle(tester);
      await tester.enterText(
        find.widgetWithText(TextField, 'Phone number'),
        '08031112222',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Password'),
        'a-long-password',
      );
      await tester.pump();
      await tester.tap(find.text('Sign in').last);
      await settle(tester);
      expect(find.text('Enter the code'), findsOneWidget);
    });

    testWidgets('sign in, then sign out from the account screen', (
      tester,
    ) async {
      final repo = FakeAuthRepository();
      final container = await pumpApp(tester, repo);
      await settle(tester);
      await tester.tap(find.text('I already have an account'));
      await settle(tester);
      await tester.enterText(
        find.widgetWithText(TextField, 'Phone number'),
        '08031112222',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Password'),
        'a-long-password',
      );
      await tester.pump();
      await tester.tap(find.text('Sign in').last);
      await settle(tester);
      expect(find.text('Amaka Obi'), findsOneWidget);

      await tester.tap(find.text('Sign out'));
      await settle(tester);
      expect(
        find.text(
          'Your orders and chats stay safe. You can sign back in any time.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Sign out').last);
      await settle(tester, 800);
      expect(repo.calls, contains('signOut'));
      expect(
        container.read(authControllerProvider).status,
        AuthStatus.signedOut,
      );
      expect(find.text('Create an account'), findsOneWidget);
    });

    testWidgets('the theme switch works', (tester) async {
      await pumpApp(tester, FakeAuthRepository(stored: testMe));
      await settle(tester);
      await tester.tap(find.text('Dark'));
      await settle(tester);
      final app = tester.widget<MaterialApp>(find.byType(MaterialApp));
      expect(app.themeMode, ThemeMode.dark);
    });
  });

  test('platform channel for haptics is not required in unit tests', () {
    TestWidgetsFlutterBinding.ensureInitialized();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (_) async => null);
  });
}
