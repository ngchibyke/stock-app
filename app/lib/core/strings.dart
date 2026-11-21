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
  static const String home = 'Welcome';
  static const String signOut = 'Sign out';
  static const String tabMarket = 'Market';
  static const String tabAccount = 'Account';
  static const String marketOpeningSoon = 'The market is opening soon.';
  static const String marketOpeningBody =
      'Browse and buy from sellers near you, right here. '
      'In the meantime, keep your delivery addresses up to date.';
  static const String addresses = 'Addresses';

  // The account page.
  static const String terms = 'Terms of use';
  static const String privacy = 'Privacy policy';
  static const String contactSupport = 'Contact support';
  static const String linkOpenFailed =
      'We could not open that link. Please try again later.';
  static String appVersion(String version) => 'Version $version';

  /// The person's role as it reads on the account page; an unknown role is
  /// shown as the server sent it.
  static String roleName(String role) => switch (role) {
    'buyer' => 'Buyer',
    'seller' => 'Seller',
    'admin' => 'Admin',
    _ => role,
  };

  // The address book.
  static const String cancel = 'Cancel';
  static const String delete = 'Delete';
  static const String addAddress = 'Add an address';
  static const String addressesEmptyTitle = 'No addresses yet';
  static const String addressesEmptyBody =
      'Add the place you want your orders delivered to.';
  static const String unknownCouncil = 'Area council not listed';
  static String landmarkLine(String landmark) => 'Landmark: $landmark';
  static String postcodeLine(String postcode) => 'Postcode: $postcode';
  static String deleteAddressFor(String name) => name.trim().isEmpty
      ? 'Delete this address'
      : 'Delete the address for $name';
  static const String deleteConfirmTitle = 'Delete this address?';
  static const String deleteConfirmBody =
      'It will be removed from your address book.';
  static const String deleteFailed =
      'We could not delete that address. Please try again.';

  // The new address form.
  static const String newAddressTitle = 'New address';
  static const String saveAddress = 'Save address';
  static const String recipientNameLabel = 'Recipient name';
  static const String addressLineLabel = 'Street address';
  static const String landmarkLabel = 'Landmark (optional)';
  static const String councilLabel = 'Area council';
  static const String postcodeLabel = 'Postcode (optional)';
  static const String postcodeHint =
      'The new 11-character code that starts with FC. '
      'Leave it blank if you do not have one.';
  static const String recipientRequired = "Enter the recipient's name.";
  static const String recipientTooLong =
      'Use 100 characters or fewer for the name.';
  static const String addressLineRequired = 'Enter the street address.';
  static const String addressLineTooLong =
      'Use 300 characters or fewer for the address.';
  static const String landmarkTooLong =
      'Use 200 characters or fewer for the landmark.';
  static const String councilRequired = 'Choose your area council.';
  static const String postcodeInvalid =
      'That postcode is not right. Enter the 11-character code that starts '
      'with FC, or leave it blank.';
  static const String outsideServiceArea =
      'We do not deliver there yet. Coming to your city soon.';
  static const String tooManyAddresses =
      'You have saved as many addresses as we can keep. '
      'Delete one you no longer use, then try again.';
  static const String recipientInvalid = 'Please check the recipient name.';
  static const String addressLineInvalid = 'Please check the street address.';
  static const String landmarkInvalid = 'Please check the landmark.';

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

  // Password recovery.
  static const String forgotLink = 'Forgot your password?';
  static const String forgotTitle = 'Reset your password';
  static const String forgotSubtitle =
      'Enter your phone number and we will text you a code.';
  static const String forgotAction = 'Send code';
  static const String backToSignIn = 'Back to sign in';
  // Said for every phone number, with or without an account.
  static const String resetSentNotice =
      'If that number has an account, we have sent a code.';
  static const String resetTitle = 'Choose a new password';
  static const String newPasswordLabel = 'New password';
  static const String confirmPasswordLabel = 'Confirm new password';
  static const String passwordsDoNotMatch = 'The two passwords do not match.';
  static const String resetAction = 'Change password';
  // One message for a wrong, expired, locked or unknown code.
  static const String resetCodeInvalid =
      'That code is not right, or it has expired. Check it, or ask for a new code.';
  static const String resetPasswordRejected =
      'That password cannot be used. Choose another one with 10 to 128 '
      'characters.';
  static const String passwordChanged =
      'Your password was changed. Sign in with your new password.';

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
