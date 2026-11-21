import '../../core/api/api_error.dart';
import '../../core/strings.dart';

/// Plain words for a call that failed for a reason outside the person's input:
/// no network, too slow, or the service is having trouble. Anything else reads
/// as [fallback]. The server's own wording is never shown.
String addressErrorMessage(
  Object error, {
  String fallback = Strings.genericError,
}) {
  if (error is! ApiError) return fallback;
  if (error.isOffline) return Strings.offlineMessage;
  if (error.isTimeout) return Strings.timeoutMessage;
  if (error.isServerDown) return Strings.serverDownMessage;
  return fallback;
}
