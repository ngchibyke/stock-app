import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'api_client.dart';

const apiBaseUrl = String.fromEnvironment(
  'API_BASE_URL',
  defaultValue: 'http://10.0.2.2:8080',
);

/// Reads the current access token. The session layer overrides this (see
/// `sessionOverrides`); reading it without an override is a wiring bug.
final accessTokenReaderProvider = Provider<Future<String?> Function()>(
  (ref) => throw UnimplementedError(
    'accessTokenReaderProvider must be overridden (add sessionOverrides to '
    'the ProviderScope).',
  ),
);

/// Called once when an authenticated call is rejected with 401. The session
/// layer overrides this to sign the user out; reading it without an override
/// is a wiring bug.
final unauthorizedHandlerProvider = Provider<void Function()>(
  (ref) => throw UnimplementedError(
    'unauthorizedHandlerProvider must be overridden (add sessionOverrides to '
    'the ProviderScope).',
  ),
);

final apiClientProvider = Provider<ApiClient>(
  (ref) => ApiClient(
    baseUrl: apiBaseUrl,
    readToken: () => ref.read(accessTokenReaderProvider)(),
    onUnauthorized: () => ref.read(unauthorizedHandlerProvider)(),
  ),
);
