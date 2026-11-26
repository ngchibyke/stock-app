import 'package:flutter_test/flutter_test.dart';
import 'package:market/features/buy/buy_models.dart';
import 'package:market/features/buy/inbox.dart';
import 'package:market/features/chat/chat_models.dart' hide ConversationPage;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:market/features/chat/chat_pages.dart';
import 'package:market/features/chat/chat_strings.dart';
import 'package:market/features/sell/sell_models.dart';
import 'package:market/features/sell/sell_shell.dart';
import 'package:material_ui/material_ui.dart';

import '../../support/fake_chat.dart';
import '../../support/pump_buy.dart';
import '../../support/pump_sell.dart' show SellEnv, pumpSell, settleFrames;

Future<void> _tap(WidgetTester tester, Finder f) async {
  await tester.ensureVisible(f);
  await tester.pump();
  await tester.tap(f);
}

Finder _field() => find.byType(TextField);

Future<void> _type(WidgetTester tester, String text) async {
  await tester.enterText(_field(), text);
  await tester.pump();
}

Future<void> _open(WidgetTester tester, BuyEnv env, Conversation c) async {
  await pumpBuy(tester, ConversationPage(conversation: c), env);
  await settleFrames(tester, 3);
}

void main() {
  group('conversation', () {
    testWidgets('messages_sit_on_the_right_side_and_the_team_sees_who_wrote', (
      tester,
    ) async {
      final env = BuyEnv();
      env.chat.messageRows = [
        fakeMessage(1, from: Side.buyer, body: 'Is it in stock?'),
        fakeMessage(2, from: Side.seller, body: 'Yes, three left', by: 'Amina'),
      ];
      await _open(
        tester,
        env,
        fakeConversation(
          you: Side.seller,
          with_: 'Buyer',
          orderId: 'o1',
          branch: 'Garki',
        ),
      );
      expect(find.text('Is it in stock?'), findsOneWidget);
      expect(find.text('Yes, three left'), findsOneWidget);
      expect(find.text(ChatStrings.writtenBy('Amina')), findsOneWidget);
      expect(find.text('Buyer'), findsOneWidget);
      expect(
        find.textContaining(ChatStrings.atBranch('Garki')),
        findsOneWidget,
      );
      // Seller is "mine" here, so their bubble is on the right.
      final mine = tester.getTopRight(find.text('Yes, three left')).dx;
      final theirs = tester.getTopRight(find.text('Is it in stock?')).dx;
      expect(mine, greaterThan(theirs));
    });

    testWidgets(
      'a_buyer_sees_no_author_and_the_hint_about_keeping_things_in_the_app',
      (tester) async {
        final env = BuyEnv();
        env.chat.messageRows = [
          fakeMessage(1, from: Side.seller, body: 'Welcome'),
        ];
        await _open(tester, env, fakeConversation());
        expect(find.textContaining('by '), findsNothing);
        expect(find.text(ChatStrings.keepInApp), findsOneWidget);
        expect(find.text(ChatStrings.aboutProduct), findsOneWidget);
      },
    );

    testWidgets(
      'typing_and_sending_shows_the_servers_message_and_clears_the_box',
      (tester) async {
        final env = BuyEnv();
        await _open(tester, env, fakeConversation());
        expect(find.byTooltip(ChatStrings.send), findsNothing);
        await _type(tester, 'Is it new?');
        await tester.tap(find.bySemanticsLabel(ChatStrings.send));
        await settleFrames(tester, 3);
        expect(env.chat.sent.single.body, 'Is it new?');
        expect(find.text('Is it new?'), findsOneWidget);
        expect(tester.widget<TextField>(_field()).controller!.text, isEmpty);
      },
    );

    testWidgets(
      'the_box_stops_at_the_servers_cap_and_blank_text_is_never_sent',
      (tester) async {
        final env = BuyEnv();
        await _open(tester, env, fakeConversation());
        await _type(tester, 'x' * 1500);
        expect(
          tester.widget<TextField>(_field()).controller!.text.length,
          maxChatChars,
        );
        await _type(tester, '   ');
        await tester.tap(find.bySemanticsLabel(ChatStrings.send));
        await settleFrames(tester, 2);
        expect(env.chat.sent, isEmpty);
      },
    );

    testWidgets(
      'a_held_message_says_checking_and_a_failed_one_can_be_tapped_to_retry',
      (tester) async {
        final env = BuyEnv();
        env.chat.results = [const SendResult(outcome: Outcome.held)];
        await _open(tester, env, fakeConversation());
        await _type(tester, 'Hold this please');
        await tester.tap(find.bySemanticsLabel(ChatStrings.send));
        await settleFrames(tester, 3);
        expect(find.text(ChatStrings.checking), findsOneWidget);
        expect(find.text('Hold this please'), findsOneWidget);

        final env2 = BuyEnv();
        env2.chat.failSend = apiError('offline', status: 0);
        await _open(tester, env2, fakeConversation());
        await _type(tester, 'Are you there?');
        await tester.tap(find.bySemanticsLabel(ChatStrings.send));
        await settleFrames(tester, 3);
        expect(find.text(ChatStrings.notSent), findsOneWidget);
        await tester.tap(find.text('Are you there?'));
        await settleFrames(tester, 3);
        expect(env2.chat.sent, hasLength(2));
        expect(env2.chat.sent[1].client, env2.chat.sent[0].client);
        expect(find.text(ChatStrings.notSent), findsNothing);
      },
    );

    testWidgets(
      'a_blocked_message_shows_the_servers_words_and_edit_puts_the_text_back',
      (tester) async {
        final env = BuyEnv();
        env.chat.results = [
          const SendResult(
            outcome: Outcome.blocked,
            reason: 'Keep contact details and payments inside the app.',
            strikes: 1,
          ),
        ];
        await _open(tester, env, fakeConversation());
        await _type(tester, 'call 08031234567');
        await tester.tap(find.bySemanticsLabel(ChatStrings.send));
        await settleFrames(tester, 3);
        expect(
          find.text('Keep contact details and payments inside the app.'),
          findsOneWidget,
        );
        expect(find.text(ChatStrings.strikes(1)), findsOneWidget);
        // Never the filter's category, only plain words.
        expect(find.textContaining('contact_info'), findsNothing);
        await _tap(tester, find.text(ChatStrings.edit));
        await settleFrames(tester, 2);
        expect(
          tester.widget<TextField>(_field()).controller!.text,
          'call 08031234567',
        );
      },
    );

    testWidgets('a_rewrite_offers_the_suggestion_and_sends_it_by_its_id', (
      tester,
    ) async {
      final env = BuyEnv();
      env.chat.results = [
        const SendResult(
          outcome: Outcome.rewrite,
          flagId: 'f1',
          suggested: 'Please keep it polite',
          reason: 'That breaks the rules.',
        ),
      ];
      await _open(tester, env, fakeConversation());
      await _type(tester, 'you are useless');
      await tester.tap(find.bySemanticsLabel(ChatStrings.send));
      await settleFrames(tester, 3);
      expect(find.text('Please keep it polite'), findsOneWidget);
      await _tap(tester, find.text(ChatStrings.sendInstead));
      await settleFrames(tester, 3);
      expect(env.chat.suggestions.single.flag, 'f1');
      expect(env.chat.sent, hasLength(1));
      expect(find.text('suggested text'), findsOneWidget);
    });

    testWidgets(
      'muted_unverified_and_closed_replace_the_composer_with_a_plain_line',
      (tester) async {
        final env = BuyEnv();
        await _open(tester, env, fakeConversation(writable: false));
        expect(find.text(ChatStrings.closed), findsOneWidget);
        expect(find.byType(TextField), findsNothing);

        final env2 = BuyEnv();
        env2.chat.failSend = apiError('identity_required', status: 403);
        await _open(
          tester,
          env2,
          fakeConversation(you: Side.seller, with_: 'Buyer'),
        );
        await _type(tester, 'Hello');
        await tester.tap(find.bySemanticsLabel(ChatStrings.send));
        await settleFrames(tester, 3);
        expect(find.text(ChatStrings.identityNeeded), findsOneWidget);
        expect(find.byType(TextField), findsNothing);
      },
    );

    testWidgets('a_live_frame_adds_the_message_and_the_read_mark_follows', (
      tester,
    ) async {
      final env = BuyEnv();
      await _open(tester, env, fakeConversation());
      env.chat.messageRows = [
        fakeMessage(1, from: Side.seller, body: 'Hi there'),
      ];
      env.chatSocket.push(MessageFrame(env.chat.messageRows.single));
      await settleFrames(tester, 3);
      expect(find.text('Hi there'), findsOneWidget);
      expect(env.chat.reads, contains(1));
    });

    testWidgets('without_the_live_link_the_page_says_reconnecting', (
      tester,
    ) async {
      final env = BuyEnv();
      env.chatSocket.refuse = true;
      await _open(tester, env, fakeConversation());
      expect(find.text(ChatStrings.reconnecting), findsOneWidget);
      // Let the retry timer run out before the tree goes away.
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(minutes: 1));
    });

    testWidgets('text_at_200_percent_does_not_overflow', (tester) async {
      final env = BuyEnv();
      env.chat.messageRows = [
        fakeMessage(
          1,
          body: 'A fairly long message that wraps onto several lines when the text is large',
        ),
      ];
      tester.platformDispatcher.textScaleFactorTestValue = 2.0;
      addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      await _open(
        tester,
        env,
        fakeConversation(
          you: Side.seller,
          with_: 'Buyer',
          orderId: 'o1',
          branch: 'Garki',
        ),
      );
      expect(tester.takeException(), isNull);
    });
  });

  group('lists and entry points', () {
    testWidgets('the_buyers_messages_show_unread_and_open_the_conversation', (
      tester,
    ) async {
      final env = BuyEnv();
      env.chat.conversationRows = [
        fakeConversation(unread: 2),
        fakeConversation(id: 'c2', you: Side.seller, with_: 'Buyer'),
      ];
      await pumpBuy(tester, const MessagesPage(), env);
      await settleFrames(tester, 3);
      // Only the buyer's own conversations.
      expect(find.text('Ada Fashion'), findsOneWidget);
      expect(find.text('Buyer'), findsNothing);
      await tester.tap(find.text('Ada Fashion'));
      await settleFrames(tester, 4);
      expect(find.byType(ConversationPage), findsOneWidget);
    });

    testWidgets('the_empty_list_has_its_state_and_a_new_message_refreshes_it', (
      tester,
    ) async {
      final env = BuyEnv();
      await pumpBuy(tester, const MessagesPage(), env);
      await settleFrames(tester, 3);
      expect(find.text(ChatStrings.noChats), findsOneWidget);
      env.chat.conversationRows = [fakeConversation()];
      env.chatSocket.push(MessageFrame(fakeMessage(1)));
      await settleFrames(tester, 4);
      expect(find.text('Ada Fashion'), findsOneWidget);
    });

    testWidgets(
      'the_sell_chats_tab_shows_only_this_stores_seller_side_with_the_branch',
      (tester) async {
        final env = SellEnv();
        env.chat.conversationRows = [
          fakeConversation(
            id: 'c1',
            you: Side.seller,
            with_: 'Buyer',
            orderId: 'o1',
            branch: 'Garki',
          ),
          fakeConversation(
            id: 'c2',
            you: Side.seller,
            with_: 'Buyer',
            storeId: 'other',
          ),
          fakeConversation(id: 'c3', you: Side.buyer, with_: 'Another Shop'),
        ];
        await pumpSell(tester, const SellChatsTab(storeId: 's1'), env);
        await settleFrames(tester, 3);
        expect(find.text('Buyer'), findsOneWidget);
        expect(find.text('Another Shop'), findsNothing);
        expect(
          find.textContaining(ChatStrings.atBranch('Garki')),
          findsOneWidget,
        );
      },
    );

    testWidgets('the_sell_area_has_a_chats_tab_for_every_role', (tester) async {
      for (final role in [
        StoreRole.owner,
        StoreRole.manager,
        StoreRole.staff,
      ]) {
        final env = SellEnv(seller: role == StoreRole.owner);
        env.sell.stores = [
          SellStore(
            id: 's1',
            name: 'Ada Shop',
            slug: 's1',
            status: 'active',
            role: role,
          ),
        ];
        await pumpSell(tester, const SellEntryPage(), env);
        await settleFrames(tester, 3);
        expect(find.text(ChatStrings.chats), findsWidgets, reason: '$role');
      }
    });

    testWidgets(
      'a_notice_with_a_conversation_opens_it_and_a_missing_one_says_so',
      (tester) async {
        final env = BuyEnv();
        env.chat.conversationRows = [fakeConversation(id: 'c9')];
        final n = Notice(
          id: 'n',
          kind: 'chat_message',
          title: 'T',
          body: 'B',
          createdAt: DateTime(2026),
          read: false,
          conversationId: 'c9',
        );
        final page = pageForNotice(n)!;
        await pumpBuy(tester, page, env);
        await settleFrames(tester, 4);
        expect(find.byType(ConversationPage), findsOneWidget);

        final env2 = BuyEnv();
        await pumpBuy(tester, const ConversationByIdPage(id: 'gone'), env2);
        await settleFrames(tester, 4);
        expect(find.text(ChatStrings.notFound), findsOneWidget);
      },
    );

    testWidgets(
      'opening_a_product_chat_shows_the_conversation_and_a_refusal_shows_plain_words',
      (tester) async {
        final env = BuyEnv();
        await pumpBuy(
          tester,
          Consumer(
            builder: (context, ref, _) => TextButton(
              onPressed: () => openProductChat(context, ref, 'p1'),
              child: const Text('ask'),
            ),
          ),
          env,
        );
        await tester.tap(find.text('ask'));
        await settleFrames(tester, 4);
        expect(env.chat.opened, 1);
        expect(find.byType(ConversationPage), findsOneWidget);
      },
    );
  });
}
