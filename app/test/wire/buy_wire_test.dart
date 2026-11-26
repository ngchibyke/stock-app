// Buying wire check: the buyer's routes, driven with the app's real repositories
// against a REAL local server. A payment cannot happen on a local server (no payment
// provider), so this covers everything up to it: the address, the payment return page
// in the app config, delivery options for an empty cart (refused), orders, the inbox and push
// registration. Skipped unless WIRE_BASE_URL and WIRE_OTP_LOG are set (see README.md).
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/config/app_config.dart';
import 'package:market/features/addresses/address.dart';
import 'package:market/features/addresses/address_repository.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/buy/buy_models.dart';
import 'package:market/features/buy/buy_repository.dart';

import 'wire_support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() => HttpOverrides.global = null);

  test(
    'a_buyer_reaches_the_payment_step_and_reads_orders_and_notices',
    () async {
      if (!isLoopbackBaseUrl(wireBaseUrl)) {
        fail('the wire check only talks to a loopback server');
      }
      String? token;
      final api = ApiClient(
        baseUrl: wireBaseUrl,
        readToken: () async => token,
        onUnauthorized: () {},
      );
      final auth = ApiAuthRepository(api);
      late CodeChallenge challenge;
      final code = await codeAfter(() async {
        challenge = await auth.signUp(
          phone: nextPhone(),
          password: freshPassword(),
          displayName: 'Wire Buyer',
        );
      });
      token = await auth.verifyPhone(
        verificationId: challenge.verificationId,
        code: code,
      );

      // The app learns where the provider returns the buyer.
      final raw =
          await api.get('/app/config', auth: false) as Map<String, dynamic>;
      final config = AppConfig.fromJson(raw);
      expect(
        config.paymentReturnUrl,
        isNotNull,
        reason: 'the server answers payment_return_url',
      );

      // An address, then delivery options for an empty cart.
      final addresses = ApiAddressRepository(api);
      final address = await addresses.create(
        const NewAddress(
          recipientName: 'Wire Buyer',
          phone: '08031234567',
          addressLine: '4 Quiet Close, Maitama',
          landmark: null,
          areaCouncil: AreaCouncil.amac,
          postcode: null,
        ),
      );
      final checkout = ApiCheckoutRepository(api);
      await expectLater(
        checkout.options(address.id),
        throwsA(isA<ApiError>().having((e) => e.code, 'code', 'cart_empty')),
        reason: 'an empty cart has nothing to send',
      );
      // A made-up checkout is not found, in the server's own envelope.
      await expectLater(
        checkout.status('00000000-0000-0000-0000-000000000000'),
        throwsA(isA<ApiError>().having((e) => e.status, 'status', 404)),
      );

      // Orders and the inbox are empty and parse.
      final orders = ApiBuyOrdersRepository(api);
      expect((await orders.orders()).items, isEmpty);
      await expectLater(
        orders.order('00000000-0000-0000-0000-000000000000'),
        throwsA(isA<ApiError>()),
      );
      final inbox = ApiInboxRepository(api);
      final box = await inbox.inbox();
      expect(box.items, isEmpty);
      expect(box.unread, 0);
      await inbox.markRead(all: true);

      // Push registration: the token is accepted and removed.
      await inbox.registerPush(
        'wire-device-token-${DateTime.now().microsecondsSinceEpoch}',
        'android',
      );
      await inbox.removePush();
      expect(DisputeReason.values, isNotEmpty);
    },
    skip: wireSkip,
  );
}
