import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/features/chat/chat_models.dart';
import 'package:market/features/chat/chat_repository.dart';

import '../../support/fakes.dart';

ApiClient _client(FakeAdapter a) => ApiClient(
  baseUrl: 'https://api.test',
  readToken: () async => 'tok',
  onUnauthorized: () {},
  adapter: a,
);

Map<String, dynamic> _conv() => {
  'id': 'c1',
  'you': 'buyer',
  'with': 'Ada Fashion',
  'unread': 0,
  'writable': true,
  'last_message_at': '2026-11-26T10:00:00Z',
};

void main() {
  test('opening_sends_only_the_id_and_reads_the_conversation', () async {
    final a = FakeAdapter()
      ..reply(200, _conv())
      ..reply(200, _conv());
    final repo = ApiChatRepository(_client(a));
    final c = await repo.openProduct('p1');
    await repo.openOrder('o1');
    expect(c.id, 'c1');
    expect(c.you, Side.buyer);
    expect(a.requests[0].path, '/chats');
    expect(a.requests[0].body, {'product_id': 'p1'});
    expect(a.requests[1].body, {'order_id': 'o1'});
  });

  test('history_and_list_use_cursors_and_never_put_text_in_the_url', () async {
    final a = FakeAdapter()
      ..reply(200, {'messages': [], 'pending': []})
      ..reply(200, {'conversations': [], 'next': null});
    final repo = ApiChatRepository(_client(a));
    await repo.history('c1', after: 7);
    await repo.conversations(before: 'cursor');
    expect(a.requests[0].path, '/chats/c1/messages');
    expect(a.requests[0].query['after'], '7');
    expect(a.requests[1].query['before'], 'cursor');
  });

  test('send_carries_the_client_id_and_a_suggestion_carries_no_text', () async {
    final a = FakeAdapter()
      ..reply(200, {'outcome': 'held'})
      ..reply(200, {'outcome': 'blocked', 'reason': 'Not here', 'strikes': 2})
      ..reply(200, {'outcome': 'weird'});
    final repo = ApiChatRepository(_client(a));
    final held = await repo.send('c1', clientMessageId: 'x1', body: 'hi');
    expect(held.outcome, Outcome.held);
    expect(a.requests[0].path, '/chats/c1/messages');
    expect(a.requests[0].body, {'client_message_id': 'x1', 'body': 'hi'});

    final blocked = await repo.sendSuggestion(
      'c1',
      'f1',
      clientMessageId: 'x2',
    );
    expect(blocked.strikes, 2);
    expect(a.requests[1].path, '/chats/c1/suggestions/f1/send');
    expect(a.requests[1].body, {'client_message_id': 'x2'});

    // An outcome the app does not know is an error, never a guess.
    expect(
      () => repo.send('c1', clientMessageId: 'x3', body: 'hi'),
      throwsA(isA<Object>()),
    );
  });

  test('mark_read_posts_the_sequence', () async {
    final a = FakeAdapter()..reply(204);
    await ApiChatRepository(_client(a)).markRead('c1', 5);
    expect(a.requests.single.path, '/chats/c1/read');
    expect(a.requests.single.body, {'seq': 5});
  });

  test('reconnect_delay_grows_and_stops_at_thirty_seconds_with_jitter', () {
    final d = [for (var i = 0; i < 9; i++) reconnectDelay(i).inMilliseconds];
    expect(d.first, inInclusiveRange(750, 1250));
    expect(d.last, inInclusiveRange(22500, 37500));
  });
}
