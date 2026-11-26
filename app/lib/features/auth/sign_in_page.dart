import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/format/phone.dart';
import '../../core/session/session_controller.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/brand_header.dart';
import '../../core/ui/states.dart';
import 'auth_errors.dart';
import 'auth_repository.dart';
import 'pending_credential.dart';
import 'pending_reset.dart';

/// Sign in with a phone and a password.
///
/// A good answer signs the person in (the router then leaves this page). An
/// account whose phone is not verified yet answers with a fresh code, and
/// [onUnverified] gets the challenge, the phone as typed and the password so
/// the code page can ask for a resend; the password is held in memory only and
/// never goes into a URL.
///
/// A wrong password and an unknown phone read the same: this page never says
/// which.
class SignInPage extends ConsumerStatefulWidget {
  const SignInPage({
    super.key,
    required this.onUnverified,
    required this.onSignUp,
    this.onForgot,
  });

  final void Function(
    CodeChallenge challenge,
    String phone,
    PendingCredential credential,
  )
  onUnverified;
  final VoidCallback onSignUp;

  /// Opens password recovery. The link is shown only when this is given.
  final VoidCallback? onForgot;

  @override
  ConsumerState<SignInPage> createState() => _SignInPageState();
}

class _SignInPageState extends ConsumerState<SignInPage> {
  final _phone = TextEditingController();
  final _password = TextEditingController();

  String? _phoneError;
  String? _passwordError;
  String? _formError;
  bool _retryable = false;
  bool _busy = false;

  // True when the person has just changed their password (shown once).
  bool _passwordChanged = false;

  @override
  void initState() {
    super.initState();
    _passwordChanged = ref.read(passwordChangedNoticeProvider);
    if (_passwordChanged) {
      // A provider may not change while the tree is being built.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) ref.read(passwordChangedNoticeProvider.notifier).consume();
      });
    }
  }

  @override
  void dispose() {
    _password.clear();
    _password.dispose();
    _phone.dispose();
    super.dispose();
  }

  bool _validate() {
    final phoneError = looksLikeNigerianPhone(_phone.text.trim())
        ? null
        : Strings.phoneInvalid;
    // Never trimmed: spaces may be part of the password.
    final passwordError = _password.text.isEmpty
        ? Strings.passwordRequired
        : null;
    setState(() {
      _phoneError = phoneError;
      _passwordError = passwordError;
      _formError = null;
    });
    return phoneError == null && passwordError == null;
  }

  Future<void> _submit() async {
    if (_busy) return;
    if (!_validate()) return;
    final phone = _phone.text.trim();
    final password = _password.text;
    final repo = ref.read(authRepositoryProvider);
    final session = ref.read(sessionProvider.notifier);
    setState(() {
      _busy = true;
      _formError = null;
    });
    try {
      final token = await repo.signIn(phone: phone, password: password);
      await session.signInWithToken(token);
      if (!mounted) return;
      setState(() => _busy = false);
    } on PhoneUnverified catch (unverified) {
      if (!mounted) return;
      setState(() => _busy = false);
      widget.onUnverified(
        unverified.challenge,
        phone,
        PendingCredential(password),
      );
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _retryable = authErrorIsRetryable(e);
        _formError = e is ApiError && e.code == 'invalid_credentials'
            ? Strings.invalidCredentials
            : authErrorMessage(e);
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final error = _formError;
    return BrandHeaderScaffold(
      title: Strings.signInTitle,
      subtitle: Strings.signInSubtitle,
      child: AutofillGroup(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (_passwordChanged) ...[
              Semantics(
                liveRegion: true,
                child: Text(
                  Strings.passwordChanged,
                  style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                    color: context.tokens.ink,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
              const SizedBox(height: Tokens.s4),
            ],
            AppTextField(
              label: Strings.phoneLabel,
              controller: _phone,
              errorText: _phoneError,
              keyboardType: TextInputType.phone,
              formatters: const [PhoneInputFormatter()],
              action: TextInputAction.next,
            ),
            const SizedBox(height: Tokens.s4),
            AppTextField(
              label: Strings.passwordLabel,
              controller: _password,
              errorText: _passwordError,
              obscure: true,
              action: TextInputAction.done,
              onSubmitted: (_) => _submit(),
            ),
            if (error != null) ...[
              const SizedBox(height: Tokens.s3),
              AuthErrorBanner(
                message: error,
                onRetry: _retryable && !_busy ? _submit : null,
              ),
            ],
            if (widget.onForgot != null)
              Align(
                alignment: AlignmentDirectional.centerStart,
                child: TextButton(
                  style: TextButton.styleFrom(minimumSize: const Size(48, 48)),
                  onPressed: widget.onForgot,
                  child: const Text(Strings.forgotLink),
                ),
              ),
            const SizedBox(height: Tokens.s5),
            LoadingButton(
              label: Strings.signInAction,
              loading: _busy,
              onPressed: _submit,
            ),
            const SizedBox(height: Tokens.s3),
            TextButton(
              style: TextButton.styleFrom(minimumSize: const Size(48, 48)),
              onPressed: widget.onSignUp,
              child: const Text(Strings.createAccountInstead),
            ),
          ],
        ),
      ),
    );
  }
}
