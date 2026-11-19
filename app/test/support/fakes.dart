import 'package:market_app/core/api/api_error.dart';
import 'package:market_app/features/auth/auth_models.dart';
import 'package:market_app/features/auth/auth_repository.dart';

const testMe = Me(
  id: 'u1',
  displayName: 'Amaka Obi',
  phone: '+2348031112222',
  role: Role.user,
  isSeller: false,
);

/// A scriptable [AuthRepository]. Every call is recorded in [calls].
class FakeAuthRepository implements AuthRepository {
  FakeAuthRepository({this.stored});

  Me? stored;
  final List<String> calls = [];
  Object? signUpError;
  Object? verifyError;
  Object? signInError;
  bool restoreFails = false;

  /// What the next sign-in does when the account still needs its code.
  String unverifiedId = 'v-2';
  bool needsCode = false;

  @override
  Future<Me?> restore() async {
    calls.add('restore');
    if (restoreFails) throw const ApiError(0, 'network', 'offline');
    return stored;
  }

  @override
  Future<String> signUp({
    required String phone,
    required String password,
    required String displayName,
  }) async {
    calls.add('signUp $phone $displayName');
    if (signUpError != null) throw signUpError!;
    return 'v-1';
  }

  @override
  Future<Me> verify({
    required String verificationId,
    required String code,
  }) async {
    calls.add('verify $verificationId $code');
    if (verifyError != null) throw verifyError!;
    return stored = testMe;
  }

  @override
  Future<Me> signIn({required String phone, required String password}) async {
    calls.add('signIn $phone');
    if (signInError != null) throw signInError!;
    if (needsCode) throw PhoneNotVerified(unverifiedId);
    return stored = testMe;
  }

  @override
  Future<void> signOut() async {
    calls.add('signOut');
    stored = null;
  }
}
