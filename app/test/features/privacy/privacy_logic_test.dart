import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:market/core/api/api_client.dart';
import 'package:market/core/api/api_error.dart';
import 'package:market/core/api/providers.dart';
import 'package:market/core/config/app_gate.dart';
import 'package:market/core/paths.dart';
import 'package:market/core/session/session_controller.dart';
import 'package:market/core/session/session_store.dart';
import 'package:market/features/privacy/privacy_models.dart';
import 'package:market/features/privacy/privacy_repository.dart';
import 'package:market/features/privacy/privacy_strings.dart';
import 'package:market/router.dart';

import '../../support/fakes.dart';

const _me = {
  'id': 'u1',
  'display_name': 'Ada',
  'phone': '+2348012345678',
  'role': 'buyer',
  'is_seller': false,
};

const _pending = {
  'error': {'code': 'deletion_pending', 'message': 'x'},
};

ApiClient _client(FakeAdapter a) => ApiClient(
  baseUrl: 'https://api.test',
  readToken: () async => 'tok',
  onUnauthorized: () {},
  adapter: a,
);

ProviderContainer _container(SessionStore store, FakeAdapter adapter) {
  final c = ProviderContainer(
    overrides: [
      sessionStoreProvider.overrideWithValue(store),
      ...sessionOverrides,
      apiClientProvider.overrideWith(
        (ref) => ApiClient(
          baseUrl: 'https://api.test',
          readToken: () => ref.read(accessTokenReaderProvider)(),
          onUnauthorized: () => ref.read(unauthorizedHandlerProvider)(),
          adapter: adapter,
        ),
      ),
    ],
  );
  addTearDown(c.dispose);
  return c;
}

void main() {
  group('repository', () {
    test('the_password_goes_in_the_body_only_and_never_in_the_path', () async {
      final a = FakeAdapter()
        ..reply(200, {
          'state': 'pending',
          'requested_at': '2026-11-26T10:00:00Z',
          'due_at': '2026-12-26T10:00:00Z',
        })
        ..reply(204)
        ..reply(202, {
          'state': 'pending',
          'requested_at': '2026-11-26T10:00:00Z',
        });
      final repo = ApiPrivacyRepository(_client(a));
      final d = await repo.requestDeletion('hunter-hunter-1');
      expect(d.pending, isTrue);
      expect(d.dueAt, DateTime.utc(2026, 12, 26, 10));
      await repo.cancelDeletion();
      final e = await repo.requestExport('hunter-hunter-1');
      expect(e.state, ExportState.pending);
      expect(a.requests[0].path, '/me/deletion');
      expect(a.requests[0].body, {'password': 'hunter-hunter-1'});
      expect(a.requests[1].path, '/me/deletion');
      expect(a.requests[2].path, '/me/export');
      for (final r in a.requests) {
        expect(r.path, isNot(contains('hunter')));
        expect(r.query, isEmpty);
      }
    });

    test('statuses_read_every_state_and_ignore_what_they_do_not_know', () {
      expect(
        ExportStatus.fromJson({
          'state': 'ready',
          'expires_at': '2026-12-03T00:00:00Z',
        }).state,
        ExportState.ready,
      );
      expect(
        ExportStatus.fromJson({'state': 'expired'}).state,
        ExportState.expired,
      );
      expect(
        ExportStatus.fromJson({'state': 'failed'}).state,
        ExportState.failed,
      );
      expect(ExportStatus.fromJson({'state': 'weird'}).state, ExportState.none);
      expect(DeletionStatus.fromJson({'state': 'none'}).pending, isFalse);
    });

    test('the_file_comes_back_as_readable_text', () async {
      final a = FakeAdapter()
        ..reply(200, {
          'profile': {'display_name': 'Ada'},
        });
      final text = await ApiPrivacyRepository(_client(a)).exportFile();
      expect(text, contains('"display_name": "Ada"'));
      expect(a.requests.single.path, '/me/export/file');
    });

    test('a_blocked_deletion_carries_its_reasons_as_codes', () {
      final e = ApiError.fromResponse(409, {
        'error': {
          'code': 'deletion_blocked',
          'message': 'x',
          'reasons': ['open_order', 'owns_store', 7],
        },
      });
      expect(e.reasons, ['open_order', 'owns_store']);
      expect(
        ApiError.fromResponse(409, {
          'error': {'code': 'c', 'message': 'x'},
        }).reasons,
        isEmpty,
      );
    });

    test(
      'every_blocker_code_has_its_own_words_and_unknown_ones_a_fallback',
      () {
        final words = {
          for (final c in [
            'open_order',
            'open_dispute',
            'owns_store',
            'money_in_flight',
          ])
            PrivacyStrings.blocker(c),
        };
        expect(words, hasLength(4));
        expect(PrivacyStrings.blocker('something_new'), isNotEmpty);
        expect(
          words.contains(PrivacyStrings.blocker('something_new')),
          isFalse,
        );
        expect(PrivacyStrings.day(DateTime(2026, 1, 5)), '5 Jan 2026');
      },
    );
  });

  group('session', () {
    test(
      'a_restored_session_with_a_pending_deletion_lands_on_the_waiting_state',
      () async {
        final a = FakeAdapter()
          ..reply(403, _pending)
          ..reply(200, {'state': 'pending', 'due_at': '2026-12-26T10:00:00Z'});
        final c = _container(MemorySessionStore('tok'), a);
        final state = await c.read(sessionProvider.future);
        expect(state, isA<SessionDeleting>());
        expect(
          (state as SessionDeleting).dueAt,
          DateTime.utc(2026, 12, 26, 10),
        );
        expect(a.requests.map((r) => r.path), ['/me', '/me/deletion']);
      },
    );

    test(
      'signing_in_with_a_pending_deletion_keeps_the_token_and_waits',
      () async {
        final store = MemorySessionStore();
        final a = FakeAdapter()
          ..reply(403, _pending)
          ..reply(200, {'state': 'pending', 'due_at': '2026-12-26T10:00:00Z'});
        final c = _container(store, a);
        await c.read(sessionProvider.future);
        await c.read(sessionProvider.notifier).signInWithToken('new-token');
        expect(c.read(sessionProvider).value, isA<SessionDeleting>());
        expect(
          store.token,
          'new-token',
          reason: 'they can still change their mind',
        );
      },
    );

    test('keeping_the_account_loads_who_they_are_again', () async {
      final a = FakeAdapter()
        ..reply(403, _pending)
        ..reply(200, {'state': 'pending', 'due_at': '2026-12-26T10:00:00Z'})
        ..reply(200, _me);
      final c = _container(MemorySessionStore('tok'), a);
      await c.read(sessionProvider.future);
      await c.read(sessionProvider.notifier).keepAccount();
      expect(c.read(sessionProvider).value, isA<SessionSignedIn>());
    });

    test('other_403s_are_not_mistaken_for_a_pending_deletion', () async {
      final a = FakeAdapter()
        ..reply(403, {
          'error': {'code': 'forbidden', 'message': 'x'},
        });
      final c = _container(MemorySessionStore('tok'), a);
      await expectLater(
        c.read(sessionProvider.future),
        throwsA(isA<ApiError>()),
      );
    });
  });

  group('router', () {
    test('a_waiting_account_can_only_be_on_the_waiting_page', () {
      const waiting = AsyncData<SessionState>(SessionDeleting(null));
      for (final path in ['/home', '/cart', '/account', '/sell', '/anything']) {
        expect(
          redirectFor(session: waiting, gate: Gate.open, location: path),
          deletionPendingPath,
          reason: path,
        );
      }
      expect(
        redirectFor(
          session: waiting,
          gate: Gate.open,
          location: deletionPendingPath,
        ),
        isNull,
      );
    });
  });
}
