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

  Future<CodeChallenge> Function(String phone)? onRequestReset;
  Future<void> Function(String verificationId, String code, String newPassword)?
  onConfirmReset;

  final List<String> resetPhones = [];
  final List<String> resetIds = [];
  final List<String> resetCodes = [];
  final List<String> resetPasswords = [];

  @override
  Future<CodeChallenge> requestPasswordReset(String phone) {
    resetPhones.add(phone);
    return onRequestReset!(phone);
  }

  @override
  Future<void> confirmPasswordReset({
    required String verificationId,
    required String code,
    required String newPassword,
  }) {
    resetIds.add(verificationId);
    resetCodes.add(code);
    resetPasswords.add(newPassword);
    return onConfirmReset!(verificationId, code, newPassword);
  }
}
