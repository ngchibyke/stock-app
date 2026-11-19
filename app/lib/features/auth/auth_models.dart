enum Role { user, support, owner }

class Me {
  const Me({
    required this.id,
    required this.displayName,
    required this.phone,
    required this.role,
    required this.isSeller,
  });

  factory Me.fromJson(Map<String, dynamic> j) => Me(
    id: j['id'] as String,
    displayName: j['display_name'] as String,
    phone: j['phone'] as String,
    role: Role.values.firstWhere(
      (r) => r.name == j['role'],
      orElse: () => Role.user,
    ),
    isSeller: j['is_seller'] as bool? ?? false,
  );

  final String id;
  final String displayName;
  final String phone;
  final Role role;
  final bool isSeller;

  String get firstName => displayName.trim().split(RegExp(r'\s+')).first;
}

/// Where a sign-up or sign-in stands when the phone still needs its code.
class PendingVerification {
  const PendingVerification({
    required this.verificationId,
    required this.phone,
    required this.password,
  });
  final String verificationId;
  final String phone;

  /// Kept in memory for this flow only, so a resend can sign in again (the server
  /// re-issues the code to a correct password on an unverified account).
  final String password;

  PendingVerification withId(String id) =>
      PendingVerification(verificationId: id, phone: phone, password: password);
}
