import 'dart:async';
import 'dart:io' show Platform;

import 'package:firebase_core/firebase_core.dart';
import 'package:firebase_messaging/firebase_messaging.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../core/session/session_controller.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../chat/chat_pages.dart';
import 'buy_models.dart';
import 'buy_repository.dart';
import 'buy_strings.dart';
import 'dispute_pages.dart';
import 'orders_pages.dart';

/// The person's notices, newest first, with the unread count. Rebuilt for whoever is
/// signed in; nothing is kept for a signed-out person.
final inboxProvider = FutureProvider.autoDispose<Inbox>((ref) async {
  final session = ref.watch(sessionProvider).value;
  if (session is! SessionSignedIn) return const Inbox(items: [], unread: 0);
  return ref.watch(inboxRepositoryProvider).inbox();
});

/// Where a notice leads, from its ids alone (the text is the server's fixed copy).
/// A chat notice opens the conversation, a dispute notice the report, an order notice
/// the order; anything else has no page.
Widget? pageForNotice(Notice n) {
  if (n.conversationId != null) {
    return ConversationByIdPage(id: n.conversationId!);
  }
  if (n.disputeId != null) return DisputeStatusPage(disputeId: n.disputeId!);
  if (n.orderId != null) return BuyOrderPage(orderId: n.orderId!);
  return null;
}

class InboxPage extends ConsumerWidget {
  const InboxPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final inbox = ref.watch(inboxProvider);
    final tokens = context.tokens;
    final theme = Theme.of(context);
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: AppBar(
        title: const Text(BuyStrings.inboxTitle),
        backgroundColor: tokens.bg,
        surfaceTintColor: const Color(0x00000000),
        actions: [
          if ((inbox.value?.unread ?? 0) > 0)
            TextButton(
              onPressed: () async {
                await ref.read(inboxRepositoryProvider).markRead(all: true);
                ref.invalidate(inboxProvider);
              },
              child: const Text(BuyStrings.markAllRead),
            ),
        ],
      ),
      body: inbox.when(
        loading: () => const Padding(
          padding: EdgeInsets.all(Tokens.s5),
          child: Skeleton(height: 72),
        ),
        error: (e, _) => ErrorState(
          message: buyError(e),
          onRetry: () => ref.invalidate(inboxProvider),
        ),
        data: (box) => box.items.isEmpty
            ? const EmptyState(title: BuyStrings.noNotices)
            : RefreshIndicator(
                onRefresh: () => ref.refresh(inboxProvider.future),
                child: ListView.separated(
                  itemCount: box.items.length,
                  separatorBuilder: (_, _) => const Divider(height: 1),
                  itemBuilder: (context, i) {
                    final n = box.items[i];
                    return ListTile(
                      minTileHeight: Tokens.minTap,
                      title: Text(
                        n.title,
                        style: n.read ? null : theme.textTheme.titleSmall,
                      ),
                      subtitle: Text(n.body),
                      leading: n.read
                          ? null
                          : Icon(Icons.circle, size: 10, color: tokens.accent),
                      onTap: () async {
                        if (!n.read) {
                          unawaited(
                            ref
                                .read(inboxRepositoryProvider)
                                .markRead(ids: [n.id])
                                .then((_) => ref.invalidate(inboxProvider)),
                          );
                        }
                        final page = pageForNotice(n);
                        if (page != null) {
                          await Navigator.of(
                            context,
                          ).push<void>(MaterialPageRoute(builder: (_) => page));
                        }
                      },
                    );
                  },
                ),
              ),
      ),
    );
  }
}

/// Turns on push for this device. Answers the device token, or null when the person
/// said no or push is not set up. The token goes to the server only.
abstract class PushService {
  Future<String?> enable();
}

/// Firebase push. It starts only when the buyer agrees, and fails quietly where the
/// app has no Firebase files (development, tests).
class FcmPushService implements PushService {
  const FcmPushService();

  @override
  Future<String?> enable() async {
    try {
      await Firebase.initializeApp();
      final messaging = FirebaseMessaging.instance;
      final settings = await messaging.requestPermission();
      if (settings.authorizationStatus == AuthorizationStatus.denied) {
        return null;
      }
      await messaging.register();
      return await messaging.onRegistered.first.timeout(
        const Duration(seconds: 10),
      );
    } on Object {
      return null;
    }
  }
}

final pushServiceProvider = Provider<PushService>(
  (ref) => const FcmPushService(),
);

const _askedKey = 'push_asked.v1';

/// After the first paid order, once: explain what push sends, and if the buyer agrees,
/// ask the system and register the device with the server.
Future<void> maybeAskForPush(BuildContext context, WidgetRef ref) async {
  final prefs = await SharedPreferences.getInstance();
  if (prefs.getBool(_askedKey) ?? false) return;
  await prefs.setBool(_askedKey, true);
  if (!context.mounted) return;
  final yes = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text(BuyStrings.pushTitle),
      content: const Text(BuyStrings.pushBody),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(false),
          child: const Text(BuyStrings.pushNo),
        ),
        FilledButton(
          style: FilledButton.styleFrom(
            minimumSize: const Size(0, Tokens.minTap),
          ),
          onPressed: () => Navigator.of(context).pop(true),
          child: const Text(BuyStrings.pushYes),
        ),
      ],
    ),
  );
  if (yes != true) return;
  final token = await ref.read(pushServiceProvider).enable();
  if (token == null) return;
  try {
    await ref
        .read(inboxRepositoryProvider)
        .registerPush(token, Platform.isIOS ? 'ios' : 'android');
  } on Object {
    // Registered next time; the inbox still works.
  }
}
