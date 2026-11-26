import 'dart:math';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/chat/chat_controller.dart';
import 'package:market/features/chat/chat_models.dart';
import 'package:market/features/chat/chat_repository.dart';
import 'package:market/features/chat/chat_strings.dart';

import '../../support/fake_chat.dart';

void main() {
  group('ids and merging', () {
    test('a_client_message_id_is_a_random_version_4_uuid', () {
      final ids = {for (var i = 0; i < 30; i++) newClientMessageId()};
      expect(ids, hasLength(30));
      for (final id in ids) {
        expect(
          RegExp(
            r'^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$',
          ).hasMatch(id),
          isTrue,
          reason: id,
        );
      }
    });

    test(
      'messages_merge_in_order_without_duplicates_whatever_order_they_arrive',
      () {
        final have = [fakeMessage(1), fakeMessage(3)];
        final merged = mergeMessages(have, [
          fakeMessage(4),
          fakeMessage(2),
          fakeMessage(3),
          fakeMessage(2),
        ]);
        expect(merged.map((m) => m.seq), [1, 2, 3, 4]);
        expect(mergeMessages(const [], const []), isEmpty);
      },
    );

    test('the_reconnect_delay_grows_and_is_capped_with_jitter', () {
      final r = Random(1);
      final ms = [
        for (var a = 0; a < 10; a++) reconnectDelay(a, r).inMilliseconds,
      ];
      expect(ms.first, inInclusiveRange(750, 1250));
      expect(ms[3], inInclusiveRange(6000, 10000));
      for (final v in ms.skip(6)) {
        expect(v, inInclusiveRange(22500, 37500));
      }
    });
  });

  group('parsing', () {
    test('a_conversation_parses_and_a_broken_one_is_skipped', () {
      final c = Conversation.fromJson({
        'id': 'c1',
        'with': 'Buyer',
        'you': 'seller',
        'store_id': 's1',
        'store_name': 'Shop',
        'order_id': 'o1',
        'branch_name': 'Garki',
        'writable': true,
        'unread': 2,
        'last_message_at': '2026-11-26T10:00:00Z',
      })!;
      expect(c.you, Side.seller);
      expect(c.aboutOrder, isTrue);
      expect(c.branchName, 'Garki');
      expect(c.unread, 2);
      expect(Conversation.fromJson({'id': 'c', 'you': 'robot'}), isNull);
      expect(
        ConversationPage.fromJson({
          'conversations': [
            {'id': 'x'},
          ],
          'next_cursor': null,
        }).items,
        isEmpty,
      );
    });

    test(
      'a_message_keeps_its_author_for_the_team_and_never_prints_its_text',
      () {
        final m = ChatMessage.fromJson({
          'id': 'm',
          'conversation_id': 'c',
          'seq': 3,
          'from': 'seller',
          'body': 'Call me on 0803',
          'by': 'Amina',
          'sent_at': '2026-11-26T10:00:00Z',
        })!;
        expect(m.by, 'Amina');
        expect('$m', isNot(contains('0803')));
        expect(ChatMessage.fromJson({'id': 'm'}), isNull);
      },
    );

    test('every_send_outcome_parses', () {
      final delivered = SendResult.fromJson({
        'outcome': 'delivered',
        'message': {
          'id': 'm',
          'conversation_id': 'c',
          'seq': 1,
          'from': 'buyer',
          'body': 'Hi',
          'sent_at': '2026-11-26T10:00:00Z',
        },
      })!;
      expect(delivered.message!.seq, 1);
      expect(SendResult.fromJson({'outcome': 'held'})!.outcome, Outcome.held);
      final blocked = SendResult.fromJson({
        'outcome': 'blocked',
        'reason': 'Keep contact details inside the app.',
        'strikes': 2,
        'muted_until': '2026-11-27T10:00:00Z',
      })!;
      expect(blocked.strikes, 2);
      expect(blocked.mutedUntil, isNotNull);
      final rewrite = SendResult.fromJson({
        'outcome': 'rewrite',
        'flag_id': 'f1',
        'suggested': 'Please be kind',
      })!;
      expect(rewrite.flagId, 'f1');
      expect(SendResult.fromJson({'outcome': 'teleported'}), isNull);
    });

    test('history_reads_messages_and_only_the_own_pending', () {
      final h = History.fromJson({
        'messages': [
          {
            'id': 'm',
            'conversation_id': 'c',
            'seq': 1,
            'from': 'seller',
            'body': 'Hi',
            'sent_at': '2026-11-26T10:00:00Z',
          },
        ],
        'pending': [
          {
            'id': 'p',
            'client_message_id': 'cm',
            'status': 'held',
            'created_at': '2026-11-26T10:00:00Z',
          },
          {'id': 'x'},
        ],
      });
      expect(h.messages, hasLength(1));
      expect(h.pending.single.clientMessageId, 'cm');
    });

    test('frames_parse_by_type_and_unknown_ones_are_ignored', () {
      final m = ChatFrame.fromJson({
        'type': 'message',
        'id': 'm',
        'conversation_id': 'c1',
        'seq': 5,
        'from': 'seller',
        'body': 'Hi',
        'sent_at': '2026-11-26T10:00:00Z',
      });
      expect(m, isA<MessageFrame>());
      expect(m!.conversationId, 'c1');
      final mod = ChatFrame.fromJson({
        'type': 'moderation',
        'conversation_id': 'c1',
        'message_id': 'x',
        'outcome': {'outcome': 'blocked', 'reason': 'r'},
      });
      expect(mod, isA<ModerationFrame>());
      expect(ChatFrame.fromJson({'type': 'typing'}), isNull);
      expect(ChatFrame.fromJson('nope'), isNull);
      expect(ChatFrame.fromJson({'type': 'message'}), isNull);
    });

    test('a_muted_error_carries_when_the_person_may_post_again', () {
      final e = ApiError.fromResponse(403, {
        'error': {'code': 'muted', 'message': 'x'},
        'muted_until': '2026-11-27T10:00:00Z',
      });
      expect(e.mutedUntil, DateTime.utc(2026, 11, 27, 10));
      expect(
        ApiError.fromResponse(403, {
          'error': {'code': 'other'},
        }).mutedUntil,
        isNull,
      );
    });
  });

  group('words', () {
    test('errors_are_plain', () {
      expect(
        chatError(apiError('chat_rate_limited', status: 429)),
        ChatStrings.slowDown,
      );
      expect(chatError(apiError('chat_closed')), ChatStrings.closed);
      expect(
        chatError(apiError('identity_required', status: 403)),
        ChatStrings.identityNeeded,
      );
      expect(
        chatError(apiError('not_found', status: 404)),
        ChatStrings.notFound,
      );
      expect(chatError(apiError('weird')), isNot(contains('weird')));
    });

    test('strikes_are_kind_and_singular_or_plural', () {
      expect(ChatStrings.strikes(1), contains('warning'));
      expect(ChatStrings.strikes(3), contains('3'));
    });
  });
}
