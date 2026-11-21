import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/brand_header.dart';
import '../../core/ui/code_boxes.dart';
import '../../core/ui/states.dart';
import 'auth_errors.dart';
import 'auth_repository.dart';
import 'countdown.dart';

const int _codeLength = 6;
const int _passwordMin = 10;
const int _passwordMax = 128;

/// Enter the texted code and a new password. A good answer calls [onReset];
/// it does not sign anyone in (the server has ended every session).
///
/// The new password lives only in this page's two fields. It is read at the
/// moment of sending, never trimmed, logged, stored or put in a URL, and the
/// fields are wiped when the page goes away.
class ResetPage extends ConsumerStatefulWidget {
  const ResetPage({
    super.key,
    required this.challenge,
    required this.phone,
    required this.onReset,
    @visibleForTesting this.clock,
    @visibleForTesting this.tickSource,
  });

  final CodeChallenge challenge;
  final String phone;
  final VoidCallback onReset;
  final Clock? clock;
  final Stream<void>? tickSource;

  @override
  ConsumerState<ResetPage> createState() => _ResetPageState();
}

class _ResetPageState extends ConsumerState<ResetPage>
    with WidgetsBindingObserver {
  late String _verificationId = widget.challenge.verificationId;
  late final Countdown _countdown;
  StreamSubscription<Duration>? _ticks;

  final _password = TextEditingController();
  final _confirm = TextEditingController();

  String _code = '';
  String? _codeError;
  String? _passwordError;
  String? _confirmError;
  String? _formError;
  String? _notice;
  VoidCallback? _retry;
  bool _busy = false;
  bool _resending = false;

  // A full code was typed while a resend was in flight: submit it as soon as
  // the resend ends instead of dropping it.
  bool _submitAfterResend = false;

  @override
  void initState() {
    super.initState();
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
    // No ticks arrive in the background; the deadline is still right.
    if (state == AppLifecycleState.resumed && mounted) setState(() {});
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    unawaited(_ticks?.cancel());
    _password.clear();
    _confirm.clear();
    _password.dispose();
    _confirm.dispose();
    super.dispose();
  }

  void _codeChanged(String value) {
    setState(() {
      _code = value;
      _codeError = null;
    });
  }

  /// A completed code: sent now if the passwords are already good, queued
  /// while a resend is in flight. Otherwise the person finishes the passwords
  /// and taps the button; nothing is shouted at them meanwhile.
  void _codeCompleted() {
    if (_resending) {
      _submitAfterResend = true;
      return;
    }
    unawaited(_submit(quiet: true));
  }

  void _afterResend() {
    if (!_submitAfterResend) return;
    _submitAfterResend = false;
    if (_code.length == _codeLength) unawaited(_submit(quiet: true));
  }

  String? _passwordProblem(String password) => password.length < _passwordMin
      ? Strings.passwordTooShort
      : password.length > _passwordMax
      ? Strings.passwordTooLong
      : null;

  /// Checks the form. With [quiet] a problem only stops the send; otherwise it
  /// is shown.
  bool _validate({required bool quiet}) {
    final password = _password.text; // Never trimmed.
    final codeError = _code.length == _codeLength
        ? null
        : Strings.codeIncomplete;
    final passwordError = _passwordProblem(password);
    final confirmError = passwordError == null && _confirm.text != password
        ? Strings.passwordsDoNotMatch
        : null;
    final ok =
        codeError == null && passwordError == null && confirmError == null;
    if (!quiet) {
      setState(() {
        _codeError = codeError;
        _passwordError = passwordError;
        _confirmError = confirmError;
        _formError = null;
        _retry = null;
      });
    }
    return ok;
  }

  Future<void> _submit({bool quiet = false}) async {
    if (_busy || _resending) return;
    if (!_validate(quiet: quiet)) return;
    final repo = ref.read(authRepositoryProvider);
    setState(() {
      _busy = true;
      _codeError = null;
      _passwordError = null;
      _confirmError = null;
      _formError = null;
      _notice = null;
      _retry = null;
    });
    try {
      // Read only now, at the moment of sending.
      await repo.confirmPasswordReset(
        verificationId: _verificationId,
        code: _code,
        newPassword: _password.text,
      );
      if (!mounted) return;
      _password.clear();
      _confirm.clear();
      setState(() => _busy = false);
      widget.onReset();
    } catch (e) {
      if (!mounted) return;
      final message = authErrorMessage(e);
      setState(() {
        _busy = false;
        if (e is ApiError && e.code == 'code_invalid') {
          _codeError = Strings.resetCodeInvalid;
        } else if (e is ApiError && e.code == 'validation_failed') {
          // The code is not used up by this: keep it so it can be retried.
          _passwordError = Strings.resetPasswordRejected;
        } else {
          _formError = message;
          _retry = authErrorIsRetryable(e) ? _submit : null;
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
      final next = await repo.requestPasswordReset(widget.phone);
      if (!mounted) return;
      _countdown.restart(Duration(seconds: next.resendAfterSecs));
      setState(() {
        _resending = false;
        _verificationId = next.verificationId;
        _notice = Strings.resentConfirmation;
      });
      _afterResend();
    } catch (e) {
      if (!mounted) return;
      if (e is ApiError && (e.retryAfterSecs ?? 0) > 0) {
        _countdown.restart(Duration(seconds: e.retryAfterSecs!));
      }
      setState(() {
        _resending = false;
        _formError = authErrorMessage(e);
        _retry = authErrorIsRetryable(e) ? _resend : null;
      });
      _afterResend();
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
      title: Strings.resetTitle,
      subtitle: Strings.resetSentNotice,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          CodeBoxes(
            length: _codeLength,
            errorText: _codeError,
            onChanged: _codeChanged,
            onCompleted: (_) => _codeCompleted(),
          ),
          const SizedBox(height: Tokens.s4),
          AppTextField(
            label: Strings.newPasswordLabel,
            controller: _password,
            errorText: _passwordError,
            obscure: true,
            action: TextInputAction.next,
          ),
          const SizedBox(height: Tokens.s2),
          Text(
            Strings.passwordHint,
            style: theme.textTheme.bodySmall?.copyWith(color: colors.mute),
          ),
          const SizedBox(height: Tokens.s4),
          AppTextField(
            label: Strings.confirmPasswordLabel,
            controller: _confirm,
            errorText: _confirmError,
            obscure: true,
            action: TextInputAction.done,
            onSubmitted: (_) => _submit(),
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
            label: Strings.resetAction,
            loading: _busy,
            onPressed: _submit,
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
