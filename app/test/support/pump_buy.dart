import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart' show Override;
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/theme/theme.dart';
import 'package:market/features/addresses/address_repository.dart';
import 'package:market/features/buy/buy_repository.dart';
import 'package:market/features/buy/inbox.dart';
import 'package:market/features/buy/payment_page.dart';
import 'package:market/features/cart/cart_repository.dart';
import 'package:market/features/sell/photos/photo_source.dart';
import 'package:market/features/sell/photos/photo_upload.dart';
import 'package:market/features/sell/sell_repository.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_address_repository.dart';
import 'fake_buy.dart';
import 'fake_cart_repository.dart';
import 'fake_sell.dart';
import 'pump_sell.dart';

class FakeConfig extends AppConfigNotifier {
  FakeConfig(this.config);
  final AppConfig config;
  @override
  Future<AppConfig> build() async => config;
}

class FakePush implements PushService {
  FakePush([this.token = 'tok']);
  final String? token;
  int calls = 0;
  @override
  Future<String?> enable() async {
    calls++;
    return token;
  }
}

/// A stand-in for the platform browser: shows a button that plays the provider
/// sending the buyer back.
Widget fakePaymentView(
  BuildContext context,
  String url,
  String? returnUrl,
  VoidCallback onReturn,
) => Column(
  children: [
    Text('PAY $url'),
    TextButton(onPressed: onReturn, child: const Text('provider returns')),
  ],
);

class BuyEnv {
  BuyEnv();

  final FakeCheckoutRepository checkout = FakeCheckoutRepository();
  final FakeBuyOrdersRepository orders = FakeBuyOrdersRepository();
  final FakeInboxRepository inbox = FakeInboxRepository();
  final FakeWorkRepository work = FakeWorkRepository();
  final FakeSellRepository sell = FakeSellRepository();
  final FakeAddressRepository addresses = FakeAddressRepository([
    testAddress(),
  ]);
  final FakeCartRepository cart = FakeCartRepository([fakeLine('v1')]);
  final FakePush push = FakePush();
  PhotoSource? photos;
  AppConfig config = const AppConfig(
    paymentReturnUrl: 'https://app.test/checkout/return',
  );

  List<Override> get overrides => [
    sessionProvider.overrideWith(() => FixedSession(meWith())),
    checkoutRepositoryProvider.overrideWithValue(checkout),
    buyOrdersRepositoryProvider.overrideWithValue(orders),
    inboxRepositoryProvider.overrideWithValue(inbox),
    workRepositoryProvider.overrideWithValue(work),
    sellRepositoryProvider.overrideWithValue(sell),
    addressRepositoryProvider.overrideWithValue(addresses),
    cartRepositoryProvider.overrideWithValue(cart),
    pushServiceProvider.overrideWithValue(push),
    appConfigProvider.overrideWith(() => FakeConfig(config)),
    paymentViewProvider.overrideWithValue(fakePaymentView),
    paymentPollProvider.overrideWithValue((
      every: const Duration(milliseconds: 100),
      times: 4,
    )),
    objectPutterProvider.overrideWithValue(_NoPut()),
    if (photos != null) photoSourceProvider.overrideWithValue(photos!),
  ];
}

class _NoPut implements ObjectPutter {
  @override
  Future<void> put(ticket, bytes) async {}
}

Future<void> pumpBuy(
  WidgetTester tester,
  Widget child,
  BuyEnv env, {
  Size size = const Size(400, 1800),
}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1.0;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(const SizedBox());
  await tester.pumpWidget(
    ProviderScope(
      retry: (_, _) => null,
      overrides: env.overrides,
      child: MaterialApp(theme: lightTheme(), home: child),
    ),
  );
  await tester.pump();
}
