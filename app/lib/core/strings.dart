/// Every user-visible string in the app. Widgets take their copy from here or
/// from parameters, never from literals.
abstract final class Strings {
  // Brand.
  static const String wordmark = 'MARKET';

  // The four states (loading, empty, error, offline).
  static const String loading = 'Loading';
  static const String emptyTitle = 'Nothing here yet';
  static const String genericError =
      'Something went wrong. Check your connection and try again.';
  static const String offlineTitle = "You're offline";
  static const String offlineBody =
      'Check your connection. We will show your latest data when you are back.';

  // Actions.
  static const String retry = 'Try again';
  static const String back = 'Back';

  // Code entry.
  static String codeFieldLabel(int length) =>
      'Verification code, $length digits';
}
