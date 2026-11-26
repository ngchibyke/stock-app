// Privacy wire check: the app's real privacy repository, session handling and
// sign-in against a REAL local server. A person asks for their data and gets it, asks
// to delete the account, is gated while the cooling-off runs, changes their mind, asks
// again, and after the date the server's own job erases them and frees their number.
//
// The jobs run once a minute, so this takes about two minutes. Skipped unless
// WIRE_BASE_URL and WIRE_OTP_LOG are set (see "Wire check" in README.md). Also needs
// WIRE_DATABASE_URL (the scratch database): the check moves the deletion date into the
// past there, which stands in for 30 days passing, and changes nothing else.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/features/auth/auth_repository.dart';
import 'package:market/features/privacy/privacy_models.dart';
import 'package:market/features/privacy/privacy_repository.dart';

import 'wire_support.dart';

final _databaseUrl = Platform.environment['WIRE_DATABASE_URL'] ?? '';

Future<void> _psql(String sql) async {
  final r = await Process.run('psql', [
    _databaseUrl,
    '-v',
    'ON_ERROR_STOP=1',
    '-c',
    sql,
  ]);
  expect(r.exitCode, 0, reason: '${r.stderr}');
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() => HttpOverrides.global = null);

  test(
    'a_person_takes_their_data_asks_to_leave_changes_their_mind_and_is_then_erased',
    () async {
      if (!isLoopbackBaseUrl(wireBaseUrl)) {
        fail('the wire check only talks to a loopback server');
      }
      if (_databaseUrl.isEmpty) {
        fail('set WIRE_DATABASE_URL to the scratch database');
      }
      String? token;
      final api = ApiClient(
        baseUrl: wireBaseUrl,
        readToken: () async => token,
        onUnauthorized: () {},
      );
      final auth = ApiAuthRepository(api);
      final phone = nextPhone();
      final password = freshPassword();
      late CodeChallenge challenge;
      final code = await codeAfter(() async {
        challenge = await auth.signUp(
          phone: phone,
          password: password,
          displayName: 'Wire Leaver',
        );
      });
      token = await auth.verifyPhone(
        verificationId: challenge.verificationId,
        code: code,
      );
      final repo = ApiPrivacyRepository(api);
      expect((await repo.deletionStatus()).pending, isFalse);

      // A wrong password changes nothing.
      await expectLater(
        repo.requestDeletion('not-the-password-1'),
        throwsA(
          isA<ApiError>().having((e) => e.code, 'code', 'invalid_credentials'),
        ),
      );
      expect((await repo.deletionStatus()).pending, isFalse);

      // Asking starts the cooling-off. The account is gated: any other route says why.
      final asked = await repo.requestDeletion(password);
      expect(asked.pending, isTrue);
      expect(
        asked.dueAt!.difference(DateTime.now()).inDays,
        inInclusiveRange(28, 30),
      );
      await expectLater(
        api.get('/me'),
        throwsA(
          isA<ApiError>()
              .having((e) => e.status, 'status', 403)
              .having((e) => e.code, 'code', 'deletion_pending'),
        ),
      );
      // Changing your mind restores everything.
      await repo.cancelDeletion();
      expect((await repo.deletionStatus()).pending, isFalse);
      expect((await api.get('/me') as Map)['display_name'], 'Wire Leaver');

      // The data export: asked for, built by the server's job, taken, and one a day.
      expect((await repo.exportStatus()).state, ExportState.none);
      expect((await repo.requestExport(password)).state, ExportState.pending);
      await expectLater(
        repo.exportFile(),
        throwsA(isA<ApiError>().having((e) => e.status, 'status', 404)),
      );
      var status = await repo.exportStatus();
      final deadline = DateTime.now().add(const Duration(seconds: 100));
      while (status.state == ExportState.pending &&
          DateTime.now().isBefore(deadline)) {
        await Future<void>.delayed(const Duration(seconds: 3));
        status = await repo.exportStatus();
      }
      expect(
        status.state,
        ExportState.ready,
        reason: 'the jobs run each minute',
      );
      expect(
        status.expiresAt!.difference(DateTime.now()).inDays,
        inInclusiveRange(6, 7),
      );
      final file = await repo.exportFile();
      expect(file, contains('"display_name": "Wire Leaver"'));
      expect(file, isNot(contains(password)));
      // Asking again while one is ready is the same file, not a second one.
      expect((await repo.requestExport(password)).state, ExportState.ready);

      // Asking again, and the date passes: the server's job erases the account.
      await repo.requestDeletion(password);
      await _psql(
        "UPDATE deletion_requests SET due_at = now() - interval '1 minute' "
        "WHERE status = 'pending'",
      );
      final erased = DateTime.now().add(const Duration(seconds: 100));
      var gone = false;
      while (!gone && DateTime.now().isBefore(erased)) {
        await Future<void>.delayed(const Duration(seconds: 3));
        try {
          await api.get('/me/deletion');
        } on ApiError catch (e) {
          // The session ended with the account.
          gone = e.status == 401;
        }
      }
      expect(gone, isTrue, reason: 'the jobs run each minute');

      // Signing in no longer works, and the number is free to sign up again.
      await expectLater(
        auth.signIn(phone: phone, password: password),
        throwsA(isA<ApiError>().having((e) => e.status, 'status', 401)),
      );
      late CodeChallenge again;
      await codeAfter(() async {
        again = await auth.signUp(
          phone: phone,
          password: freshPassword(),
          displayName: 'Someone New',
        );
      });
      expect(again.verificationId, isNotEmpty);
    },
    skip: wireSkip,
    timeout: const Timeout(Duration(minutes: 5)),
  );
}
