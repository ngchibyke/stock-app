import 'package:market/features/auth/auth_repository.dart';

/// A scriptable [AuthRepository] that records what it was asked.
class FakeAuthRepository implements AuthRepository {
  Future<CodeChallenge> Function(String phone, String password, String name)?
  onSignUp;
  Future<String> Function(String verificationId, String code)? onVerify;
  Future<String> Function(String phone, String password)? onSignIn;

  final List<String> signInPhones = [];
  final List<String> signInPasswords = [];
  final List<String> verifiedIds = [];
  final List<String> verifiedCodes = [];

  @override
  Future<CodeChallenge> signUp({
    required String phone,
    required String password,
    required String displayName,
  }) => onSignUp!(phone, password, displayName);

  @override
  Future<String> verifyPhone({
    required String verificationId,
    required String code,
  }) {
    verifiedIds.add(verificationId);
    verifiedCodes.add(code);
    return onVerify!(verificationId, code);
  }

  @override
  Future<String> signIn({required String phone, required String password}) {
    signInPhones.add(phone);
    signInPasswords.add(password);
    return onSignIn!(phone, password);
  }

  @override
  Future<CodeChallenge> requestPasswordReset(String phone) =>
      throw UnimplementedError();

  @override
  Future<void> confirmPasswordReset({
    required String verificationId,
    required String code,
    required String newPassword,
  }) => throw UnimplementedError();
}
