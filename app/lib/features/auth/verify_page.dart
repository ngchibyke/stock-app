import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/format/phone.dart';
import '../../core/session/session_controller.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/brand_header.dart';
import '../../core/ui/code_boxes.dart';
import '../../core/ui/states.dart';
import 'auth_errors.dart';
import 'auth_repository.dart';
import 'countdown.dart';

const int _codeLength = 6;

/// Enter the code that was texted. A correct code signs the person in and
/// calls [onVerified].
///
/// [password] is the one just typed on the sign-up page, kept in this page's
/// memory only so a resend can be asked for (a sign-in on the unverified
/// account). It is cleared when the page goes away and is never stored,
/// logged, or carried in a URL.
class VerifyPage extends ConsumerStatefulWidget {
  const VerifyPage({
    super.key,
    required this.challenge,
    required this.phone,
    required this.password,
    required this.onVerified,
    @visibleForTesting this.clock,
    @visibleForTesting this.tickSource,
  });

  final CodeChallenge challenge;
  final String phone;
  final String password;
  final VoidCallback onVerified;
  final Clock? clock;
  final Stream<void>? tickSource;

  @override
  ConsumerState<VerifyPage> createState() => _VerifyPageState();
}

class _VerifyPageState extends ConsumerState<VerifyPage>
    with WidgetsBindingObserver {
  late String _verificationId = widget.challenge.verificationId;
  String _password = '';
  late final Countdown _countdown;
  StreamSubscription<Duration>? _ticks;

  String _code = '';
  String? _codeError;
  String? _formError;
  String? _notice;
  VoidCallback? _retry;
  bool _busy = false;
  bool _resending = false;

  // A token the server already gave us whose session load failed. A retry
  // repeats only that load: the code is spent and must not be sent again.
  String? _pendingToken;

  @override
  void initState() {
    super.initState();
    _password = widget.password;
    _countdown = Countdown(
      Duration(seconds: widget.challenge.resendAfterSecs),
      clock: widget.clock,
      tickSource: widget.tickSource,
    );
    _ticks = _countdown.ticks.listen((_) {
      if (mounted) setState(() {});
    });
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // No ticks arrive while the app is in the background; the deadline is
    // still right, so just look at it again.
    if (state == AppLifecycleState.resumed && mounted) setState(() {});
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    unawaited(_ticks?.cancel());
    _password = '';
    _pendingToken = null;
    super.dispose();
  }

  void _codeChanged(String value) {
    setState(() {
      _code = value;
      _codeError = null;
    });
  }

  Future<void> _verify() async {
    if (_busy || _resending) return;
    final spent = _pendingToken;
    if (spent == null && _code.length != _codeLength) {
      setState(() => _codeError = Strings.codeIncomplete);
      return;
    }
    final repo = ref.read(authRepositoryProvider);
    final session = ref.read(sessionProvider.notifier);
    setState(() {
      _busy = true;
      _codeError = null;
      _formError = null;
      _notice = null;
      _retry = null;
    });
    try {
      final token =
          spent ??
          await repo.verifyPhone(verificationId: _verificationId, code: _code);
      _pendingToken = token;
      await session.signInWithToken(token);
      _pendingToken = null;
      if (!mounted) return;
      setState(() => _busy = false);
      widget.onVerified();
    } catch (e) {
      if (!mounted) return;
      final retryable = authErrorIsRetryable(e);
      if (!retryable) _pendingToken = null;
      final message = authErrorMessage(e);
      setState(() {
        _busy = false;
        if (e is ApiError && e.code.startsWith('otp_') && spent == null) {
          _codeError = message;
        } else {
          _formError = message;
          _retry = retryable ? _verify : null;
        }
      });
    }
  }

  Future<void> _resend() async {
    if (_busy || _resending || !_countdown.done) return;
    final repo = ref.read(authRepositoryProvider);
    setState(() {
      _resending = true;
      _codeError = null;
      _formError = null;
      _notice = null;
      _retry = null;
    });
    try {
      final token = await repo.signIn(phone: widget.phone, password: _password);
      // The account was verified in the meantime: just sign in.
      if (!mounted) return;
      setState(() => _resending = false);
      _pendingToken = token;
      await _verify();
    } on PhoneUnverified catch (unverified) {
      if (!mounted) return;
      final next = unverified.challenge;
      _countdown.restart(Duration(seconds: next.resendAfterSecs));
      setState(() {
        _resending = false;
        _verificationId = next.verificationId;
        _notice = Strings.resentConfirmation;
      });
    } catch (e) {
      if (!mounted) return;
      if (e is ApiError && (e.retryAfterSecs ?? 0) > 0) {
        _countdown.restart(Duration(seconds: e.retryAfterSecs!));
      }
      final rejected = e is ApiError && e.code == 'invalid_credentials';
      setState(() {
        _resending = false;
        _formError = rejected
            ? Strings.resendPasswordRejected
            : authErrorMessage(e);
        _retry = authErrorIsRetryable(e) ? _resend : null;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = context.tokens;
    final done = _countdown.done;
    final formError = _formError;
    final notice = _notice;

    return BrandHeaderScaffold(
      showBack: true,
      title: Strings.verifyTitle,
      subtitle: Strings.verifySubtitle(displayPhone(widget.phone)),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          CodeBoxes(
            length: _codeLength,
            errorText: _codeError,
            onChanged: _codeChanged,
            onCompleted: (_) => _verify(),
          ),
          if (formError != null) ...[
            const SizedBox(height: Tokens.s3),
            AuthErrorBanner(
              message: formError,
              onRetry: _busy || _resending ? null : _retry,
            ),
          ],
          if (notice != null) ...[
            const SizedBox(height: Tokens.s3),
            Semantics(
              liveRegion: true,
              child: Text(
                notice,
                style: theme.textTheme.bodyMedium?.copyWith(color: colors.mute),
              ),
            ),
          ],
          const SizedBox(height: Tokens.s5),
          LoadingButton(
            label: Strings.verifyAction,
            loading: _busy,
            onPressed: _verify,
          ),
          const SizedBox(height: Tokens.s3),
          TextButton(
            key: const ValueKey('resend-button'),
            style: TextButton.styleFrom(minimumSize: const Size(48, 48)),
            onPressed: done && !_busy && !_resending ? _resend : null,
            child: Text(
              done
                  ? Strings.resend
                  : Strings.resendIn(_countdown.remainingSecs),
              textAlign: TextAlign.center,
            ),
          ),
        ],
      ),
    );
  }
}
