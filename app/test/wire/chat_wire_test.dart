// Chat wire check: the app's real chat repository and socket against a REAL local
// server. A buyer asks about a product, the owner sees and answers it, the socket
// pushes the answer, and a dropped link catches up by sequence number. The tone check
// needs a language-model key, so a held or rewritten message cannot be seen here; the
// widget tests and the server's own tests cover those.
//
// Skipped unless WIRE_BASE_URL and WIRE_OTP_LOG are set (see "Wire check" in
// README.md). Also needs WIRE_DATABASE_URL (the scratch database) and the server started
// with IDENTITY_PROVIDER=fake and a NIN_INDEX_KEY, as for the selling check.
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/chat/chat_models.dart';
import 'package:market/features/chat/chat_repository.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_repository.dart';

import 'wire_support.dart';

const _identitySecret = 'fake-identity-webhook-secret';
final _databaseUrl = Platform.environment['WIRE_DATABASE_URL'] ?? '';

class _Person {
  _Person(this.api, this.id, this.token);

  final ApiClient api;
  final String id;
  final String token;
  late final sell = ApiSellRepository(api);
  late final chat = ApiChatRepository(api);
}

Future<String> _psql(String sql) async {
  final r = await Process.run('psql', [
    _databaseUrl,
    '-v',
    'ON_ERROR_STOP=1',
    '-t',
    '-A',
    '-c',
    sql,
  ]);
  expect(r.exitCode, 0, reason: '${r.stderr}');
  return (r.stdout as String).trim().split('\n').first;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() => HttpOverrides.global = null);

  Future<_Person> signUp(String name) async {
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
        displayName: name,
      );
    });
    token = await auth.verifyPhone(
      verificationId: challenge.verificationId,
      code: code,
    );
    final me = await api.get('/me') as Map<String, dynamic>;
    return _Person(api, me['id'] as String, token);
  }

  Future<void> deliver(String providerRef, Map<String, Object?> outcome) async {
    final body = utf8.encode(jsonEncode({'ref': providerRef, ...outcome}));
    final sig = Hmac(
      sha256,
      utf8.encode(_identitySecret),
    ).convert(body).toString();
    final client = HttpClient();
    try {
      final req = await client.postUrl(
        Uri.parse('$wireBaseUrl/webhooks/identity/fake'),
      );
      req.headers.set('x-identity-signature', sig);
      req.headers.contentType = ContentType.json;
      req.add(body);
      final res = await req.close();
      await res.drain<void>();
      expect(res.statusCode, 200);
    } finally {
      client.close(force: true);
    }
  }

  final nin =
      (10000000000 + DateTime.now().microsecondsSinceEpoch % 80000000000)
          .toString();

  test(
    'a_buyer_asks_the_owner_answers_and_the_socket_and_catch_up_agree',
    () async {
      if (!isLoopbackBaseUrl(wireBaseUrl)) {
        fail('the wire check only talks to a loopback server');
      }
      if (_databaseUrl.isEmpty) {
        fail('set WIRE_DATABASE_URL to the scratch database');
      }

      // A verified seller with an active store and one published product.
      final admin = await signUp('Wire Admin');
      await _psql("UPDATE users SET role = 'owner' WHERE id = '${admin.id}'");
      final issued = await admin.api.post(
        '/admin/access-codes',
        body: {'label': 'chat', 'max_uses': 1},
      ) as Map<String, dynamic>;
      final seller = await signUp('Chat Seller');
      await seller.sell.redeemCode(issued['code'] as String);
      final start = await seller.sell.startIdentity(nin);
      await deliver('fake-job-${start.attemptId}', {
        'outcome': 'passed',
        'full_name': 'Ada Obi',
        'dob': '1990-01-01',
      });
      final store = await seller.sell.createStore(
        const NewStore(
          name: 'Chat Boutique',
          description: 'Cloth and things',
          pickupAddress: '12 Quiet Close, Wuse 2',
          district: 'fashion',
          areaCouncil: 'amac',
          businessPhone: '08031234567',
          courier: true,
          selfDelivery: false,
        ),
      );
      await _psql(
        "UPDATE stores SET status = 'active' WHERE id = '${store.id}'",
      );
      final product = await _psql(
        "INSERT INTO products (store_id, name, description, category, condition, "
        "photo_keys, weight_grams, base_price_kobo, is_published) "
        "SELECT '${store.id}', 'Chat Dress', 'A dress', slug, 'new', "
        "ARRAY['wire/key'], 500, 1500000, true FROM categories LIMIT 1 "
        'RETURNING id',
      );

      // The owner may not open a chat about their own product.
      await expectLater(
        seller.chat.openProduct(product),
        throwsA(
          isA<ApiError>().having((e) => e.code, 'code', 'own_store_item'),
        ),
      );

      // A buyer asks, and opening twice is the same conversation.
      final buyer = await signUp('Chat Buyer');
      final conv = await buyer.chat.openProduct(product);
      expect(conv.you, Side.buyer);
      expect(conv.with_, 'Chat Boutique');
      expect((await buyer.chat.openProduct(product)).id, conv.id);

      // The buyer's live link is up before the owner answers.
      final socket = WebChatSocket(
        baseUrl: wireBaseUrl,
        readToken: () async => buyer.token,
      );
      final opened = Completer<void>();
      final frames = <ChatFrame>[];
      final sub = socket
          .frames(onOpen: () => opened.complete())
          .listen(frames.add);
      await opened.future.timeout(const Duration(seconds: 10));

      final asked = await buyer.chat.send(
        conv.id,
        clientMessageId: '11111111-1111-4111-8111-111111111111',
        body: 'Is the dress still available?',
      );
      expect(asked.outcome, Outcome.delivered);
      // The same client id is the same message, not a second one.
      final again = await buyer.chat.send(
        conv.id,
        clientMessageId: '11111111-1111-4111-8111-111111111111',
        body: 'Is the dress still available?',
      );
      expect(again.message?.seq, asked.message?.seq);

      // The owner sees it on the selling side and answers.
      final theirs = await seller.chat.conversations();
      final seen = theirs.items.singleWhere((c) => c.id == conv.id);
      expect(seen.you, Side.seller);
      expect(seen.with_, 'Buyer', reason: 'the buyer stays anonymous');
      expect(seen.unread, 1);
      final answered = await seller.chat.send(
        conv.id,
        clientMessageId: '22222222-2222-4222-8222-222222222222',
        body: 'Yes, come and see it.',
      );
      expect(answered.outcome, Outcome.delivered);
      await seller.chat.markRead(conv.id, asked.message!.seq);

      // The buyer's socket got the answer.
      bool answer(ChatFrame f) =>
          f is MessageFrame && f.message.body == 'Yes, come and see it.';
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while (!frames.any(answer) && DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final pushed = (frames.firstWhere(answer) as MessageFrame).message;
      expect(pushed.body, 'Yes, come and see it.');
      expect(
        pushed.by,
        isNull,
        reason: 'a buyer never sees which member wrote',
      );

      // A reconnecting app catches up by sequence, with nothing twice.
      final all = await buyer.chat.history(conv.id);
      expect(all.messages.map((m) => m.body), [
        'Is the dress still available?',
        'Yes, come and see it.',
      ]);
      final after = await buyer.chat.history(
        conv.id,
        after: asked.message!.seq,
      );
      expect(after.messages.single.body, 'Yes, come and see it.');
      expect((await buyer.chat.conversations()).items.single.unread, 1);
      await buyer.chat.markRead(conv.id, pushed.seq);
      expect((await buyer.chat.conversations()).items.single.unread, 0);

      // A stranger cannot read or write it, and learns nothing about it.
      final stranger = await signUp('Stranger');
      await expectLater(
        stranger.chat.history(conv.id),
        throwsA(isA<ApiError>().having((e) => e.status, 'status', 404)),
      );
      await expectLater(
        stranger.chat.send(
          conv.id,
          clientMessageId: '33333333-3333-4333-8333-333333333333',
          body: 'hello',
        ),
        throwsA(isA<ApiError>().having((e) => e.status, 'status', 404)),
      );

      // Leaving the page closes the socket at once, not at the next frame.
      await sub.cancel().timeout(const Duration(seconds: 2));
    },
    skip: wireSkip,
    timeout: const Timeout(Duration(minutes: 2)),
  );
}
