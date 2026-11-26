import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/features/chat/chat_controller.dart';
import 'package:market/features/chat/chat_models.dart';
import 'package:market/features/chat/chat_repository.dart';

import '../../support/fake_chat.dart';
import '../../support/pump_sell.dart' show FixedSession, meWith;

class _Env {
  _Env() {
    container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        sessionProvider.overrideWith(() => FixedSession(meWith())),
        chatRepositoryProvider.overrideWithValue(repo),
        chatSocketProvider.overrideWithValue(socket),
        chatPollEveryProvider.overrideWithValue(
          const Duration(milliseconds: 50),
        ),
      ],
    );
  }

  final repo = FakeChatRepository();
  final socket = FakeChatSocket();
  late final ProviderContainer container;

  ConversationController get ctl =>
      container.read(conversationProvider('c1').notifier);
  ConvState get state =>
      container.read(conversationProvider('c1')).requireValue;

  /// Keeps the live link and the conversation alive, as an open screen does.
  Future<void> open() async {
    container.listen(chatFramesProvider, (_, _) {});
    container.listen(conversationProvider('c1'), (_, _) {});
    await container.read(conversationProvider('c1').future);
  }

  Future<void> settle([int ms = 30]) =>
      Future<void>.delayed(Duration(milliseconds: ms));
}

void main() {
  late _Env env;
  setUp(() => env = _Env());
  tearDown(() => env.container.dispose());

  test('history_loads_in_order_with_the_own_pending_ones', () async {
    env.repo.messageRows = [fakeMessage(2), fakeMessage(1, from: Side.buyer)];
    env.repo.pendingRows = [
      OwnPending(
        id: 'p',
        clientMessageId: 'cm1',
        status: 'held',
        createdAt: DateTime(2026),
        body: 'Checking this',
      ),
    ];
    await env.open();
    expect(env.state.messages.map((m) => m.seq), [1, 2]);
    expect(env.state.locals.single.status, LocalStatus.held);
    expect(env.state.locals.single.text, 'Checking this');
  });

  test(
    'a_delivered_send_adds_the_servers_message_and_nothing_local_stays',
    () async {
      await env.open();
      await env.ctl.send('  Is it in stock?  ');
      expect(env.repo.sent.single.body, 'Is it in stock?');
      expect(env.state.locals, isEmpty);
      expect(env.state.messages.single.body, 'Is it in stock?');
      // A message the server says it delivered is never shown twice.
      env.socket.push(MessageFrame(env.state.messages.single));
      await env.settle();
      expect(env.state.messages, hasLength(1));
    },
  );

  test(
    'a_held_message_shows_checking_then_becomes_delivered_by_the_next_frame',
    () async {
      env.repo.results = [const SendResult(outcome: Outcome.held)];
      await env.open();
      await env.ctl.send('Could you hold it for me?');
      expect(env.state.locals.single.status, LocalStatus.held);
      expect(env.state.messages, isEmpty);
      // The server's pending list now has it, then the job delivers it.
      final clientId = env.state.locals.single.clientId;
      env.repo.pendingRows = [
        OwnPending(
          id: 'p',
          clientMessageId: clientId,
          status: 'held',
          createdAt: DateTime(2026),
        ),
      ];
      env.socket.push(
        ModerationFrame('c1', 'p', const SendResult(outcome: Outcome.held)),
      );
      await env.settle();
      expect(env.state.locals.single.status, LocalStatus.held);
      expect(
        env.state.locals.single.text,
        'Could you hold it for me?',
        reason: 'the text is kept from the screen',
      );
      env.repo.pendingRows = [];
      env.repo.messageRows = [
        fakeMessage(1, from: Side.buyer, body: 'Could you hold it for me?'),
      ];
      env.socket.push(MessageFrame(env.repo.messageRows.single));
      await env.settle();
      expect(env.state.locals, isEmpty);
      expect(env.state.messages.single.seq, 1);
    },
  );

  test('a_blocked_message_keeps_the_reason_and_the_text_for_editing_and_counts_strikes', () async {
    env.repo.results = [
      const SendResult(
        outcome: Outcome.blocked,
        reason: 'Keep contact details inside the app.',
        strikes: 1,
      ),
    ];
    await env.open();
    await env.ctl.send('call 0803 123 4567');
    final l = env.state.locals.single;
    expect(l.status, LocalStatus.blocked);
    expect(l.reason, 'Keep contact details inside the app.');
    expect(l.text, 'call 0803 123 4567');
    expect(env.state.strikes, 1);
    expect(env.state.messages, isEmpty);
    env.ctl.dismiss(l.clientId);
    expect(env.state.locals, isEmpty);
  });

  test('a_rewrite_is_sent_by_its_id_with_a_new_message_id_and_no_text_from_the_app', () async {
    env.repo.results = [
      const SendResult(
        outcome: Outcome.rewrite,
        flagId: 'f1',
        suggested: 'Please keep it polite',
        reason: 'Be kind',
      ),
    ];
    await env.open();
    await env.ctl.send('you are useless');
    final first = env.state.locals.single;
    expect(first.status, LocalStatus.rewrite);
    expect(first.suggested, 'Please keep it polite');
    await env.ctl.useSuggestion(first.clientId);
    expect(env.repo.suggestions.single.flag, 'f1');
    expect(env.repo.suggestions.single.client, isNot(first.clientId));
    expect(env.repo.sent, hasLength(1), reason: 'no second send with text');
    expect(env.state.locals, isEmpty);
    expect(env.state.messages.single.body, 'suggested text');
  });

  test(
    'a_network_failure_keeps_the_message_and_the_retry_is_the_same_message',
    () async {
      env.repo.failSend = apiError('offline', status: 0);
      await env.open();
      await env.ctl.send('Hello?');
      final failed = env.state.locals.single;
      expect(failed.status, LocalStatus.failed);
      await env.ctl.retry(failed.clientId);
      expect(env.repo.sent, hasLength(2));
      expect(
        env.repo.sent[1].client,
        env.repo.sent[0].client,
        reason: 'the same client_message_id',
      );
      expect(env.state.locals, isEmpty);
      expect(env.state.messages, hasLength(1));
    },
  );

  test(
    'too_many_sends_in_a_minute_asks_the_person_to_slow_down_and_can_retry',
    () async {
      env.repo.failSend = apiError('chat_rate_limited', status: 429);
      await env.open();
      await env.ctl.send('one');
      expect(env.state.locals.single.status, LocalStatus.failed);
      expect(env.state.locals.single.slowDown, isTrue);
    },
  );

  test(
    'muted_unverified_and_closed_end_the_send_and_set_their_state',
    () async {
      await env.open();
      final until = DateTime.now().add(const Duration(hours: 5));
      env.repo.failSend = apiError('muted', status: 403);
      await env.ctl.send('x');
      expect(env.state.locals, isEmpty);

      // A muted error with a time.
      env.repo.failSend = ApiError(
        status: 403,
        code: 'muted',
        message: 'x',
        mutedUntil: until,
      );
      await env.ctl.send('y');
      expect(env.state.mutedUntil, until);
      expect(env.state.mutedAt(DateTime.now()), isTrue);
      expect(env.state.mutedAt(until.add(const Duration(minutes: 1))), isFalse);

      env.repo.failSend = apiError('identity_required', status: 403);
      await env.ctl.send('z');
      expect(env.state.identityRequired, isTrue);

      env.repo.failSend = apiError('chat_closed', status: 409);
      await env.ctl.send('w');
      expect(env.state.closed, isTrue);
      expect(env.state.locals, isEmpty);
    },
  );

  test(
    'frames_for_this_conversation_merge_in_order_and_others_are_ignored',
    () async {
      await env.open();
      env.socket.push(MessageFrame(fakeMessage(2)));
      env.socket.push(MessageFrame(fakeMessage(1)));
      env.socket.push(MessageFrame(fakeMessage(7, conversation: 'other')));
      env.repo.messageRows = [fakeMessage(1), fakeMessage(2)];
      await env.settle();
      expect(env.state.messages.map((m) => m.seq), [1, 2]);
    },
  );

  test('catching_up_asks_only_for_what_came_after_the_last_message', () async {
    env.repo.messageRows = [fakeMessage(1), fakeMessage(2)];
    await env.open();
    env.repo.messageRows = [...env.repo.messageRows, fakeMessage(3)];
    await env.ctl.catchUp();
    expect(env.repo.historyAfter.last, 2);
    expect(env.state.messages.map((m) => m.seq), [1, 2, 3]);
  });

  test('while_the_live_link_is_down_the_conversation_polls_and_it_stops_when_the_link_is_up', () async {
    env.socket.refuse = true;
    await env.open();
    final before = env.repo.historyCalls;
    await env.settle(220);
    expect(
      env.repo.historyCalls,
      greaterThan(before + 1),
      reason: 'polled while down',
    );
    // The link comes up: polling stops.
    env.socket.refuse = false;
    env.container.read(chatConnectedProvider.notifier).set(true);
    await env.settle(30);
    final settled = env.repo.historyCalls;
    await env.settle(200);
    expect(env.repo.historyCalls, settled);
  });

  test('the_read_mark_is_sent_once_per_new_message', () async {
    env.repo.messageRows = [fakeMessage(1), fakeMessage(2)];
    await env.open();
    await env.ctl.markRead();
    await env.ctl.markRead();
    expect(env.repo.reads, [2]);
    env.repo.messageRows = [...env.repo.messageRows, fakeMessage(3)];
    await env.ctl.catchUp();
    await env.ctl.markRead();
    expect(env.repo.reads, [2, 3]);
  });

  test('a_history_that_cannot_load_is_an_error_not_an_empty_chat', () async {
    env.repo.failHistory = apiError('server', status: 500);
    final sub = env.container.listen(conversationProvider('c1'), (_, _) {});
    await expectLater(
      env.container.read(conversationProvider('c1').future),
      throwsA(anything),
    );
    expect(env.container.read(conversationProvider('c1')).hasError, isTrue);
    sub.close();
  });

  test('a_reconnect_fetches_what_was_missed_while_the_link_was_down', () async {
    env.repo.messageRows = [fakeMessage(1)];
    await env.open();
    await env.settle();
    final link = env.container.read(chatConnectedProvider.notifier);
    link.set(false);
    env.repo.messageRows = [fakeMessage(1), fakeMessage(2)];
    // Back at once, before any poll could run.
    link.set(true);
    await env.settle();
    expect(env.state.messages.map((m) => m.seq), [1, 2]);
  });

  test('a_live_message_after_a_gap_also_brings_the_missing_ones', () async {
    env.repo.messageRows = [fakeMessage(1)];
    await env.open();
    await env.settle();
    env.repo.messageRows = [fakeMessage(1), fakeMessage(2), fakeMessage(3)];
    env.socket.push(MessageFrame(fakeMessage(3)));
    await env.settle();
    expect(env.state.messages.map((m) => m.seq), [1, 2, 3]);
  });

  test('catching_up_reads_every_page_of_what_was_missed', () async {
    env.repo.messageRows = [fakeMessage(1)];
    env.repo.pageSize = 100;
    await env.open();
    env.repo.messageRows = [for (var i = 1; i <= 250; i++) fakeMessage(i)];
    await env.ctl.catchUp();
    expect(env.state.messages, hasLength(250));
  });

  test('a_stopped_message_taken_back_for_editing_does_not_return', () async {
    env.repo.pendingRows = [
      OwnPending(
        id: 'p',
        clientMessageId: 'cm1',
        status: 'withheld',
        createdAt: DateTime(2026),
        reason: 'Not sent.',
      ),
    ];
    await env.open();
    env.ctl.dismiss('cm1');
    await env.ctl.catchUp();
    expect(env.state.locals, isEmpty);
  });

  test('a_rewrite_whose_suggestion_was_sent_does_not_return', () async {
    env.repo.pendingRows = [
      OwnPending(
        id: 'p',
        clientMessageId: 'cm1',
        status: 'withheld',
        createdAt: DateTime(2026),
        reason: 'Rude.',
        flagId: 'f1',
        suggested: 'Kinder',
      ),
    ];
    await env.open();
    await env.ctl.useSuggestion('cm1');
    await env.ctl.catchUp();
    expect(env.state.locals, isEmpty);
  });

  test('leaving_the_chat_closes_the_connection_at_once', () async {
    final listen = env.container.listen(chatFramesProvider, (_, _) {});
    await env.settle(40);
    expect(env.socket.held(0), isTrue);
    listen.close();
    await env.settle(40);
    expect(env.socket.held(0), isFalse);
  });

  test(
    'an_old_connection_ending_late_does_not_mark_the_new_one_down',
    () async {
      final first = env.container.listen(chatFramesProvider, (_, _) {});
      await env.settle(40);
      first.close();
      await env.settle(40);
      env.container.listen(chatFramesProvider, (_, _) {});
      await env.settle(40);
      expect(env.container.read(chatConnectedProvider), isTrue);
      // The server ends the first one later, as it does after a sign-out.
      env.socket.dropAt(0);
      await env.settle(40);
      expect(env.container.read(chatConnectedProvider), isTrue);
      expect(env.socket.connects, 2);
    },
  );

  test('the_socket_reconnects_after_a_drop', () async {
    final listen = env.container.listen(chatFramesProvider, (_, _) {});
    await env.settle(40);
    expect(env.socket.connects, 1);
    expect(env.container.read(chatConnectedProvider), isTrue);
    env.socket.drop();
    await env.settle(40);
    expect(env.container.read(chatConnectedProvider), isFalse);
    // The next attempt comes after the backoff (about a second).
    await env.settle(1400);
    expect(env.socket.connects, greaterThanOrEqualTo(2));
    listen.close();
  });
}
