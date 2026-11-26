import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart' show Override;
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/session/me.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/features/catalog/catalog_repository.dart';
import 'package:market/features/chat/chat_repository.dart';
import 'package:market/features/sell/identity/selfie_capture.dart';
import 'package:market/features/sell/photos/photo_source.dart';
import 'package:market/features/sell/sell_repository.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_catalog_repository.dart';
import 'fake_chat.dart';
import 'fake_sell.dart';

/// A signed-in session that holds [me] and counts how often it was refreshed.
class FixedSession extends SessionController {
  FixedSession(this.me, [this.onRefresh]);

  Me me;
  final void Function()? onRefresh;

  @override
  Future<SessionState> build() async => SessionSignedIn(me);

  @override
  Future<void> refreshMe() async {
    onRefresh?.call();
    state = AsyncData(SessionSignedIn(me));
  }
}

Me meWith({bool seller = false}) => Me(
  id: 'u1',
  displayName: 'Ada',
  phone: '+2348031234567',
  role: 'user',
  isSeller: seller,
);

class SellEnv {
  SellEnv({bool seller = true}) : me = meWith(seller: seller);

  final Me me;

  /// How often the session was asked to reload `/me`.
  int refreshes = 0;

  final FakeSellRepository sell = FakeSellRepository();
  final FakeWorkRepository work = FakeWorkRepository();
  final FakeCatalogRepository catalog = FakeCatalogRepository();
  final FakeChatRepository chat = FakeChatRepository();
  final FakeChatSocket chatSocket = FakeChatSocket();
  SelfieCapture selfie = const FakeSelfieCapture();
  PhotoSource? photos;

  List<Override> get overrides => [
    // A new notifier for each container: one cannot serve two.
    sessionProvider.overrideWith(() => FixedSession(me, () => refreshes++)),
    sellRepositoryProvider.overrideWithValue(sell),
    workRepositoryProvider.overrideWithValue(work),
    catalogRepositoryProvider.overrideWithValue(catalog),
    selfieCaptureProvider.overrideWithValue(selfie),
    chatRepositoryProvider.overrideWithValue(chat),
    chatSocketProvider.overrideWithValue(chatSocket),
    identityPollProvider.overrideWithValue((
      every: const Duration(milliseconds: 100),
      times: 50,
    )),
    if (photos != null) photoSourceProvider.overrideWithValue(photos!),
  ];
}

/// Pumps [child] on a phone-sized surface inside the providers the selling
/// screens read, with [env]'s fakes behind them.
Future<void> pumpSell(
  WidgetTester tester,
  Widget child,
  SellEnv env, {
  List<Override> extra = const [],
  Size size = const Size(400, 1800),
}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1.0;
  addTearDown(tester.view.reset);
  // A fresh tree, so a second call in one test does not reuse the first one's state.
  await tester.pumpWidget(const SizedBox());
  await tester.pumpWidget(
    ProviderScope(
      retry: (_, _) => null,
      overrides: [...env.overrides, ...extra],
      child: MaterialApp(theme: lightTheme(), home: child),
    ),
  );
  await tester.pump();
}

Future<void> settleFrames(WidgetTester tester, [int n = 6]) async {
  for (var i = 0; i < n; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}
