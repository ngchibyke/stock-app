import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../features/auth/auth_controller.dart';
import '../features/auth/auth_repository.dart';
import 'api/api_client.dart';
import 'api/session.dart';

/// The server the app talks to. Set at build time:
/// `flutter run --dart-define=API_BASE_URL=https://api.example.com`.
/// The default is the host machine as seen from the Android emulator.
const apiBaseUrl = String.fromEnvironment(
  'API_BASE_URL',
  defaultValue: 'http://10.0.2.2:8080',
);

final sessionStoreProvider = Provider<SessionStore>(
  (ref) => SecureSessionStore(),
);

final apiClientProvider = Provider<ApiClient>((ref) {
  final client = ApiClient(
    baseUrl: apiBaseUrl,
    session: ref.watch(sessionStoreProvider),
  );
  // A dead session signs the person out wherever they are.
  client.onUnauthorized = () =>
      ref.read(authControllerProvider.notifier).sessionEnded();
  return client;
});

final authRepositoryProvider = Provider<AuthRepository>(
  (ref) => ApiAuthRepository(
    ref.watch(apiClientProvider),
    ref.watch(sessionStoreProvider),
  ),
);
