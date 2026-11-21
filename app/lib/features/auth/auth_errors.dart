import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';

/// Plain words for a failed auth call. The person never sees a server code or
/// the server's own wording; every message is from [Strings].
String authErrorMessage(Object error) {
  if (error is! ApiError) return Strings.genericError;
  if (error.isOffline) return Strings.offlineMessage;
  if (error.isTimeout) return Strings.timeoutMessage;
  switch (error.code) {
    case 'otp_throttled':
    case 'too_many_attempts':
      return Strings.throttled(error.retryAfterSecs);
    case 'otp_unavailable':
      return Strings.otpUnavailable;
    case 'otp_invalid':
      return Strings.codeWrong;
    case 'otp_expired':
      return Strings.codeExpired;
    case 'otp_locked':
      return Strings.codeLocked;
    case 'phone_taken':
      return Strings.phoneTaken;
    case 'invalid_phone':
      return Strings.phoneInvalid;
    case 'validation_failed':
      return Strings.checkDetails;
  }
  if (error.isServerDown) return Strings.serverDownMessage;
  return Strings.genericError;
}

/// Whether repeating the same request could work: the network or the service
/// failed, as opposed to the person's input being refused.
bool authErrorIsRetryable(Object error) {
  if (error is! ApiError) return true;
  return error.isOffline || error.isTimeout || error.isServerDown;
}

/// An error line in the error-text colour, announced to screen readers, with
/// an optional retry. It is always text, never colour alone.
class AuthErrorBanner extends StatelessWidget {
  const AuthErrorBanner({super.key, required this.message, this.onRetry});

  final String message;
  final VoidCallback? onRetry;

  @override
  Widget build(BuildContext context) {
    final colors = context.tokens;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Semantics(
          liveRegion: true,
          child: Text(
            message,
            style: Theme.of(context).textTheme.bodyMedium?.copyWith(
              color: colors.errorText,
              fontWeight: FontWeight.w600,
            ),
          ),
        ),
        if (onRetry != null)
          TextButton(
            style: TextButton.styleFrom(minimumSize: const Size(48, 48)),
            onPressed: onRetry,
            child: const Text(Strings.retry),
          ),
      ],
    );
  }
}
