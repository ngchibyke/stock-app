import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/providers.dart';
import '../../core/session/session_controller.dart';
import 'cart.dart';

abstract class CartRepository {
  Future<Cart> get();

  /// Sets one line: 0 removes it. Answers with the whole cart.
  Future<Cart> set(String variantId, int quantity);
}

ApiError _badResponse() => ApiError(
  status: 0,
  code: 'bad_response',
  message: 'The server sent an unexpected response.',
);

Cart _parse(Object? json) {
  try {
    return Cart.fromJson(json);
  } on FormatException {
    throw _badResponse();
  }
}

/// [CartRepository] over the server API. Sends ids and quantities only.
class ApiCartRepository implements CartRepository {
  ApiCartRepository(this._api);

  final ApiClient _api;

  @override
  Future<Cart> get() async => _parse(await _api.get('/cart'));

  @override
  Future<Cart> set(String variantId, int quantity) async => _parse(
    await _api.put(
      '/cart/items',
      body: {'variant_id': variantId, 'quantity': quantity},
    ),
  );
}

final cartRepositoryProvider = Provider<CartRepository>(
  (ref) => ApiCartRepository(ref.watch(apiClientProvider)),
);

/// The cart of whoever is signed in. Lives in memory only (nothing on disk)
/// and is rebuilt empty when the person signs out or someone else signs in.
class CartNotifier extends AsyncNotifier<Cart> {
  /// Changes run one after another, so each one starts from the last answer.
  Future<void> _queue = Future.value();

  String? _signedInUserId() {
    final state = ref.read(sessionProvider).value;
    return state is SessionSignedIn ? state.me.id : null;
  }

  @override
  Future<Cart> build() async {
    final userId = ref.watch(
      sessionProvider.select((s) {
        final state = s.value;
        return state is SessionSignedIn ? state.me.id : null;
      }),
    );
    if (userId == null) return Cart.empty;
    final loaded = await ref.read(cartRepositoryProvider).get();
    // A slow load for someone who has since signed out is dropped.
    return _signedInUserId() == userId ? loaded : Cart.empty;
  }

  /// One more of [variantId].
  Future<void> add(String variantId) =>
      _run(() => (state.value?.quantityOf(variantId) ?? 0) + 1, variantId);

  /// Sets [variantId] to [quantity]; 0 removes it. Throws the [ApiError] on
  /// failure, leaving the cart as it was.
  Future<void> setQuantity(String variantId, int quantity) =>
      _run(() => quantity, variantId);

  Future<void> refresh() async {
    ref.invalidateSelf();
    await future;
  }

  Future<void> _run(int Function() quantity, String variantId) {
    final done = _queue.then((_) async {
      final userId = _signedInUserId();
      if (userId == null) return;
      final cart = await ref
          .read(cartRepositoryProvider)
          .set(variantId, quantity());
      if (_signedInUserId() == userId) state = AsyncData(cart);
    });
    _queue = done.catchError((Object _) {});
    return done;
  }
}

final cartProvider = AsyncNotifierProvider<CartNotifier, Cart>(
  CartNotifier.new,
);
