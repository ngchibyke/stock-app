/// The notice shown before the identity check.
///
/// FOR LEGAL REVIEW: this wording is a draft and is not legal advice. It lives in
/// one file so that it can be reviewed and changed without touching any screen.
/// Replace [provider] with the chosen identity partner's name at go-live.
abstract final class IdentityNotice {
  static const String provider = 'our identity partner';

  static const List<String> points = [
    'We ask this to stop scams and fake stores, and to keep buyers and sellers safe.',
    'Your NIN and a selfie are sent to $provider, who check them against NIMC records. We never see the selfie.',
    'We keep the result, your verified name and a fingerprint of your NIN that cannot be turned back into the number. We do not keep the NIN itself.',
    'If your account is closed for fraud, we keep the fingerprint so the same NIN cannot open another account. In any other case it is deleted with your account.',
    'Questions or complaints: contact support from the Account page.',
  ];
}
