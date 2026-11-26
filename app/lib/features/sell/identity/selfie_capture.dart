import 'package:flutter_riverpod/flutter_riverpod.dart';

enum SelfieResult { done, cancelled, failed }

/// Takes the selfie for the identity check. The provider's SDK implements this;
/// it sends the picture straight to the provider, so the app never holds an
/// image. [sessionToken] is for the SDK only: never shown or logged.
abstract class SelfieCapture {
  Future<SelfieResult> capture(String sessionToken);
}

/// Used until a provider is chosen: there is no SDK to run, so the capture
/// cannot be done.
class UnavailableSelfieCapture implements SelfieCapture {
  const UnavailableSelfieCapture();

  @override
  Future<SelfieResult> capture(String sessionToken) async =>
      SelfieResult.failed;
}

/// A stand-in for development against the server's fake provider: it completes
/// at once. Switched on with `--dart-define=IDENTITY_FAKE=true`.
class FakeSelfieCapture implements SelfieCapture {
  const FakeSelfieCapture();

  @override
  Future<SelfieResult> capture(String sessionToken) async => SelfieResult.done;
}

const bool _identityFake = bool.fromEnvironment('IDENTITY_FAKE');

final selfieCaptureProvider = Provider<SelfieCapture>(
  (ref) => _identityFake
      ? const FakeSelfieCapture()
      : const UnavailableSelfieCapture(),
);

/// How often the waiting screen asks for the result, and how many times.
final identityPollProvider = Provider<({Duration every, int times})>(
  (ref) => (every: const Duration(seconds: 3), times: 40),
);
