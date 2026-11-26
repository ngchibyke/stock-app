import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/api_error.dart';
import '../../core/session/session_controller.dart';
import 'sell_models.dart';
import 'sell_repository.dart';

/// The person signed in right now, or null. The selling providers watch this so
/// nothing of one person's stores or identity outlives their session.
final _userIdProvider = Provider<String?>((ref) {
  final state = ref.watch(sessionProvider).value;
  return state is SessionSignedIn ? state.me.id : null;
});

/// The stores the person owns or works at, from the server.
final myStoresProvider = FutureProvider.autoDispose<List<SellStore>>((
  ref,
) async {
  if (ref.watch(_userIdProvider) == null) return const [];
  try {
    return await ref.watch(sellRepositoryProvider).myStores();
  } on ApiError catch (e) {
    // Someone who is neither a seller nor on a team has no stores, which is not an error.
    if (e.status == 403 && e.code == 'not_a_seller') return const [];
    rethrow;
  }
});

final identityStatusProvider = FutureProvider.autoDispose<IdentityStatus>((
  ref,
) async {
  if (ref.watch(_userIdProvider) == null) {
    return const IdentityStatus(state: IdentityState.unverified);
  }
  return ref.watch(sellRepositoryProvider).identityStatus();
});

final myInvitesProvider = FutureProvider.autoDispose<List<MyInvite>>((
  ref,
) async {
  if (ref.watch(_userIdProvider) == null) return const [];
  return ref.watch(sellRepositoryProvider).myInvites();
});

/// Which store the Sell area is showing. Null means the first one.
class SelectedStore extends Notifier<String?> {
  @override
  String? build() {
    ref.watch(_userIdProvider);
    return null;
  }

  void choose(String id) => state = id;
}

final selectedStoreIdProvider = NotifierProvider<SelectedStore, String?>(
  SelectedStore.new,
);

/// The chosen store, checked against the person's live list on every refresh:
/// a store they were removed from stops being theirs at once.
final selectedStoreProvider = Provider.autoDispose<SellStore?>((ref) {
  final stores = ref.watch(myStoresProvider).value ?? const <SellStore>[];
  if (stores.isEmpty) return null;
  final chosen = ref.watch(selectedStoreIdProvider);
  for (final s in stores) {
    if (s.id == chosen) return s;
  }
  return stores.first;
});
