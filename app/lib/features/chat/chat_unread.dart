import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/session/session_controller.dart';
import 'chat_models.dart';
import 'chat_repository.dart';

/// How many messages the person has not read in their own (buyer) conversations, for
/// the Account row. A failure counts as none: the row still opens the list.
final chatUnreadProvider = FutureProvider.autoDispose<int>((ref) async {
  final session = ref.watch(sessionProvider).value;
  if (session is! SessionSignedIn) return 0;
  try {
    final page = await ref.watch(chatRepositoryProvider).conversations();
    return page.items
        .where((c) => c.you == Side.buyer)
        .fold<int>(0, (a, c) => a + c.unread);
  } on Object {
    return 0;
  }
});
