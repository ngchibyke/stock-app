import '../../core/api/api_error.dart';
import '../../core/strings.dart';

/// Every word on the privacy screens. Plain, and never the server's own text.
abstract final class PrivacyStrings {
  static const String title = 'Privacy and data';
  static const String rowLabel = 'Privacy and data';

  static const String downloadTitle = 'Download my data';
  static const String downloadBody =
      'You get one file with your profile, addresses, orders, messages, reviews, '
      'disputes and notices. It is ready in a few minutes and stays for 7 days.';
  static const String downloadAsk = 'Ask for my data';
  static const String downloadPending =
      'We are putting your file together. Check back in a few minutes.';
  static const String downloadReady = 'Your file is ready';
  static const String downloadSave = 'Save or share the file';
  static const String downloadFailed =
      'We could not make your file. Ask again in a moment.';
  static const String downloadExpired =
      'Your last file has expired. You can ask for a new one.';
  static String downloadUntil(String when) => 'Available until $when';
  static const String downloadLimited =
      'You can ask for one file a day. Try again tomorrow.';
  static const String refresh = 'Check again';

  static const String deleteTitle = 'Delete my account';
  static const String deleteIntro =
      'This removes your name, phone number, saved addresses, photos and '
      'notices. You have 30 days to change your mind: sign in and tap '
      '"Keep my account".';
  static const String deleteKeptTitle = 'What we keep';
  static const String deleteKept =
      'Orders and payment records stay, without your name or address, for as '
      'long as the law needs. Your messages stay for the other person, shown '
      'as "Deleted user", until their retention period ends.';
  static const String deleteBlockedTitle = 'Finish these first';
  static const String passwordLabel = 'Your password';
  static const String deleteButton = 'Delete my account';
  static const String wrongPassword = 'That password is not right.';
  static const String deleteDone = 'Your account is scheduled for deletion.';

  static String blocker(String code) => switch (code) {
    'open_order' =>
      'An order is still open. Wait for it to finish or cancel it.',
    'open_dispute' => 'A report or return is still open.',
    'owns_store' => 'You own a store. Close it first.',
    'money_in_flight' =>
      'A payout or refund is still on its way. Try again when it lands.',
    _ => 'Something is still open on your account.',
  };

  static const String pendingTitle = 'Your account will be deleted';
  static String pendingBody(String date) =>
      'It will be deleted on $date. Until then you cannot use the app. '
      'If you change your mind, keep your account.';
  static const String pendingBodyNoDate =
      'Until it is deleted you cannot use the app. If you change your mind, '
      'keep your account.';
  static const String keepAccount = 'Keep my account';
  static const String keepFailed =
      'We could not keep your account just now. Try again.';

  /// Plain words for a failed call. The server's own wording is not shown.
  static String error(Object e) {
    if (e is ApiError) {
      if (e.code == 'invalid_credentials') return wrongPassword;
      if (e.code == 'export_rate_limited') return downloadLimited;
      if (e.isOffline) return Strings.offlineMessage;
      if (e.isTimeout) return Strings.timeoutMessage;
      if (e.isServerDown) return Strings.serverDownMessage;
    }
    return Strings.genericError;
  }

  static const List<String> _months = [
    'Jan',
    'Feb',
    'Mar',
    'Apr',
    'May',
    'Jun',
    'Jul',
    'Aug',
    'Sep',
    'Oct',
    'Nov',
    'Dec',
  ];

  /// "27 Nov 2026".
  static String day(DateTime d) {
    final l = d.toLocal();
    return '${l.day} ${_months[l.month - 1]} ${l.year}';
  }
}
