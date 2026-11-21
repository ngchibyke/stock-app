/// The password just typed on sign-up, held in memory only so the code page
/// can ask for a resend (a sign-in on the unverified account). It is never
/// stored, logged or serialised. Whoever receives it clears it as soon as it
/// is no longer needed: the code page does so after a successful verification
/// and when it goes away.
class PendingCredential {
  PendingCredential(this.password);

  String? password;

  void clear() => password = null;

  // Never includes the password.
  @override
  String toString() => 'PendingCredential';
}
