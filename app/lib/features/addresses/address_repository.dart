import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/providers.dart';
import '../../core/session/session_controller.dart';
import 'address.dart';

abstract class AddressRepository {
  /// The person's addresses, newest first.
  Future<List<Address>> list();

  /// Throws an [ApiError]: `validation_failed`, `invalid_phone`,
  /// `outside_service_area`, `invalid_postcode` or `too_many_addresses` (422).
  Future<Address> create(NewAddress a);

  Future<void> remove(String id);
}

ApiError _badResponse() => ApiError(
  status: 0,
  code: 'bad_response',
  message: 'The server sent an unexpected response.',
);

/// [AddressRepository] over the server API. Addresses are personal data: this
/// class keeps nothing, writes nothing to disk and logs nothing.
class ApiAddressRepository implements AddressRepository {
  ApiAddressRepository(this._api);

  final ApiClient _api;

  @override
  Future<List<Address>> list() async {
    final json = await _api.get('/me/addresses');
    if (json is! List) throw _badResponse();
    // A bad row must not blank the whole list: rows that are not objects, or
    // have no usable id, are skipped; the rest are kept.
    return [
      for (final row in json)
        if (row is Map<String, dynamic>)
          if (Address.fromJson(row) case final a when a.id.isNotEmpty) a,
    ];
  }

  @override
  Future<Address> create(NewAddress a) async {
    final json = await _api.post('/me/addresses', body: a.toJson());
    if (json is! Map<String, dynamic>) throw _badResponse();
    final created = Address.fromJson(json);
    if (created.id.isEmpty) throw _badResponse();
    return created;
  }

  @override
  Future<void> remove(String id) async {
    await _api.delete('/me/addresses/${Uri.encodeComponent(id)}');
  }
}

final addressRepositoryProvider = Provider<AddressRepository>(
  (ref) => ApiAddressRepository(ref.watch(apiClientProvider)),
);

/// The person's addresses, held in memory only.
///
/// The provider is auto-disposed, so the list goes when nobody is looking at
/// it, and it is rebuilt (so emptied) whenever the person signed in changes:
/// a signed-out person's addresses are never kept. Nothing here touches disk.
class AddressesNotifier extends AsyncNotifier<List<Address>> {
  @override
  Future<List<Address>> build() async {
    final userId = ref.watch(
      sessionProvider.select((s) {
        final state = s.value;
        return state is SessionSignedIn ? state.me.id : null;
      }),
    );
    if (userId == null) return const [];
    return ref.read(addressRepositoryProvider).list();
  }

  /// Saves [a] and puts the result at the top of the held list, without a
  /// reload. Throws the [ApiError] on failure, leaving the list as it was.
  Future<Address> create(NewAddress a) async {
    // Held open across the call, so a form opened with no list on screen is
    // not disposed while it waits.
    final link = ref.keepAlive();
    try {
      final created = await ref.read(addressRepositoryProvider).create(a);
      final current = state.value;
      if (current != null) {
        state = AsyncData([created, ...current]);
      } else {
        ref.invalidateSelf();
      }
      return created;
    } finally {
      link.close();
    }
  }

  /// Deletes one address and drops its row. Throws the [ApiError] on failure,
  /// keeping the row. A 404 means it is already gone, so the row goes too.
  Future<void> remove(String id) async {
    final link = ref.keepAlive();
    try {
      try {
        await ref.read(addressRepositoryProvider).remove(id);
      } on ApiError catch (e) {
        if (e.status != 404) rethrow;
      }
      final current = state.value;
      if (current != null) {
        state = AsyncData([
          for (final a in current)
            if (a.id != id) a,
        ]);
      }
    } finally {
      link.close();
    }
  }
}

final addressesProvider =
    AsyncNotifierProvider.autoDispose<AddressesNotifier, List<Address>>(
      AddressesNotifier.new,
    );
