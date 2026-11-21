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

  // Sign-up form.
  static const String signUpTitle = 'Create your account';
  static const String signUpSubtitle =
      'Buy and sell with people you can trust.';
  static const String phoneLabel = 'Phone number';
  static const String displayNameLabel = 'Your name';
  static const String passwordLabel = 'Password';
  static const String passwordHint = 'At least 10 characters.';
  static const String signUpAction = 'Create account';
  static const String signInInstead = 'Already have an account? Sign in';
  static const String phoneInvalid = 'Enter a Nigerian phone number.';
  static const String nameRequired = 'Enter your name.';
  static const String nameTooLong = 'Use 80 characters or fewer for your name.';
  static const String passwordTooShort = 'Use at least 10 characters.';
  static const String passwordTooLong = 'Use 128 characters or fewer.';

  // Sign-in form.
  static const String signInTitle = 'Welcome back';
  static const String signInSubtitle = 'Sign in to your account.';
  static const String signInAction = 'Sign in';
  static const String createAccountInstead = 'New here? Create an account';
  static const String passwordRequired = 'Enter your password.';
  // One message for a wrong password and for an unknown phone: the screen
  // never says which of the two it was.
  static const String invalidCredentials =
      'That phone number and password do not match. Check them and try again.';

  // The app shell.
  static const String home = 'Home';
  static const String signOut = 'Sign out';

  // Full-screen notices (the gate).
  static const String updateRequiredTitle = 'Please update the app';
  static const String updateRequiredBody =
      'This version is too old to keep working. '
      'Open your app store and install the latest version.';
  static const String maintenanceTitle = 'Back soon';
  static const String maintenanceFallback =
      'We are making improvements. Please try again in a little while.';

  // Code page.
  static const String verifyTitle = 'Enter your code';
  static String verifySubtitle(String phone) =>
      'We sent a 6-digit code to $phone.';
  static const String verifyAction = 'Verify';
  static const String codeIncomplete = 'Enter all 6 digits.';
  static const String codeWrong = 'That code is not right. Check it and try.';
  static const String codeExpired =
      'That code has expired. Ask for a new code.';
  static const String codeLocked =
      'Too many wrong codes. Ask for a new code to try again.';
  static const String resend = 'Send a new code';
  static String resendIn(int secs) => 'Send a new code in ${secs}s';
  static const String resentConfirmation = 'We sent you a new code.';
  static const String signInAgain =
      'For your safety we cannot send a new code from here. '
      'Please sign in again.';
  static const String resendPasswordRejected =
      'We could not send a new code. Go back and create your account again.';

  // Plain messages for failed calls.
  static const String phoneTaken = 'That phone number already has an account.';
  static const String otpUnavailable =
      'We cannot send codes right now. Please try again in a few minutes.';
  static const String offlineMessage =
      "You're offline. Check your connection and try again.";
  static const String timeoutMessage =
      'That took too long. Check your connection and try again.';
  static const String serverDownMessage =
      'Our service is having trouble. Please try again in a few minutes.';
  static const String throttledGeneric =
      'Too many tries. Please wait a little and try again.';
  static const String checkDetails =
      'Please check what you entered and try again.';

  /// "Please wait 1 minute 30 seconds before trying again." Computed from the
  /// server's `retry_after_secs`; a missing or zero wait reads generically.
  static String throttled(int? secs) {
    if (secs == null || secs <= 0) return throttledGeneric;
    final minutes = secs ~/ 60;
    final rest = secs % 60;
    final parts = <String>[
      if (minutes > 0) '$minutes minute${minutes == 1 ? '' : 's'}',
      if (rest > 0) '$rest second${rest == 1 ? '' : 's'}',
    ];
    return 'Please wait ${parts.join(' ')} before trying again.';
  }
}
