import '../../core/api/api_error.dart';
import '../../core/strings.dart';

abstract final class ChatStrings {
  static const String messages = 'Messages';
  static const String chats = 'Chats';
  static const String noChats = 'No messages yet';
  static const String noChatsBody =
      'Questions about products and orders show up here.';
  static const String askSeller = 'Ask the seller';
  static const String messageSeller = 'Message the seller';
  static const String loadMore = 'Load more';
  static const String composerHint = 'Write a message';
  static const String send = 'Send';
  static const String keepInApp =
      'Keep contact details and payments in the app.';
  static const String sending = 'Sending';
  static const String notSent = 'Not sent. Tap to try again.';
  static const String slowDown =
      'Slow down for a moment, then tap to try again.';
  static const String checking = 'Checking';
  static const String edit = 'Edit';
  static const String sendInstead = 'Send this instead';
  static const String suggestionTitle = 'We suggest this wording';
  static const String closed =
      'This conversation is closed. You can still read it.';
  static const String reconnecting = 'Reconnecting';
  static const String identityNeeded =
      'Confirm your identity to answer buyers.';
  static String mutedUntil(String when) => 'You cannot post until $when.';
  static String strikes(int n) => n == 1
      ? 'That is a warning. Repeated breaks lead to a pause.'
      : 'That is $n warnings. More lead to a pause.';
  static const String aboutProduct = 'About a product';
  static const String aboutOrder = 'About an order';
  static String atBranch(String name) => 'Branch: $name';
  static String writtenBy(String name) => 'by $name';
  static const String notFound = 'That conversation is not available.';
}

/// Plain words for a failed call in the chat screens.
String chatError(Object error) {
  if (error is! ApiError) return Strings.genericError;
  if (error.isOffline) return Strings.offlineMessage;
  if (error.isTimeout) return Strings.timeoutMessage;
  switch (error.code) {
    case 'chat_rate_limited':
      return ChatStrings.slowDown;
    case 'own_store_item':
      return 'You cannot message your own store.';
    case 'chat_closed':
      return ChatStrings.closed;
    case 'identity_required':
      return ChatStrings.identityNeeded;
  }
  if (error.status == 404) return ChatStrings.notFound;
  if (error.isServerDown) return Strings.serverDownMessage;
  return Strings.genericError;
}
