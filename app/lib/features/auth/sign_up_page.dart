import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/format/phone.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/brand_header.dart';
import '../../core/ui/states.dart';
import 'auth_errors.dart';
import 'auth_repository.dart';
import 'pending_credential.dart';

const int _passwordMin = 10;
const int _passwordMax = 128;
const int _nameMax = 80;

/// Create an account. On success the server has sent a code; [onCodeSent]
/// gets the challenge, the phone as typed and the password so the code page
/// can ask for a resend. The password lives only in this page's field and is
/// cleared when the page goes away; it is never stored, logged or put in a
/// URL.
class SignUpPage extends ConsumerStatefulWidget {
  const SignUpPage({
    super.key,
    required this.onCodeSent,
    required this.onSignIn,
  });

  final void Function(
    CodeChallenge challenge,
    String phone,
    PendingCredential credential,
  )
  onCodeSent;
  final VoidCallback onSignIn;

  @override
  ConsumerState<SignUpPage> createState() => _SignUpPageState();
}

class _SignUpPageState extends ConsumerState<SignUpPage> {
  final _phone = TextEditingController();
  final _name = TextEditingController();
  final _password = TextEditingController();

  String? _phoneError;
  String? _nameError;
  String? _passwordError;
  String? _formError;
  bool _retryable = false;
  bool _busy = false;

  @override
  void dispose() {
    _password.clear();
    _password.dispose();
    _phone.dispose();
    _name.dispose();
    super.dispose();
  }

  bool _validate() {
    final phone = _phone.text.trim();
    final name = _name.text.trim();
    final password = _password.text; // Never trimmed.

    final phoneError = looksLikeNigerianPhone(phone)
        ? null
        : Strings.phoneInvalid;
    final nameError = name.isEmpty
        ? Strings.nameRequired
        : name.length > _nameMax
        ? Strings.nameTooLong
        : null;
    final passwordError = password.length < _passwordMin
        ? Strings.passwordTooShort
        : password.length > _passwordMax
        ? Strings.passwordTooLong
        : null;

    setState(() {
      _phoneError = phoneError;
      _nameError = nameError;
      _passwordError = passwordError;
      _formError = null;
    });
    return phoneError == null && nameError == null && passwordError == null;
  }

  Future<void> _submit() async {
    if (_busy) return;
    if (!_validate()) return;
    final phone = _phone.text.trim();
    final password = _password.text;
    setState(() {
      _busy = true;
      _formError = null;
    });
    try {
      final challenge = await ref
          .read(authRepositoryProvider)
          .signUp(
            phone: phone,
            password: password,
            displayName: _name.text.trim(),
          );
      if (!mounted) return;
      setState(() => _busy = false);
      widget.onCodeSent(challenge, phone, PendingCredential(password));
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _retryable = authErrorIsRetryable(e);
        // Our own words for every code, phone_taken included: the server's
        // message is generic ("Conflict with existing data").
        if (e is ApiError && e.code == 'invalid_phone') {
          _phoneError = Strings.phoneInvalid;
        } else {
          _formError = authErrorMessage(e);
        }
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final error = _formError;
    return BrandHeaderScaffold(
      title: Strings.signUpTitle,
      subtitle: Strings.signUpSubtitle,
      child: AutofillGroup(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
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
              label: Strings.displayNameLabel,
              controller: _name,
              errorText: _nameError,
              keyboardType: TextInputType.name,
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
            const SizedBox(height: Tokens.s2),
            Text(
              Strings.passwordHint,
              style: Theme.of(context).textTheme.bodySmall
                  ?.copyWith(color: context.tokens.mute),
            ),
            if (error != null) ...[
              const SizedBox(height: Tokens.s3),
              AuthErrorBanner(
                message: error,
                onRetry: _retryable && !_busy ? _submit : null,
              ),
            ],
            const SizedBox(height: Tokens.s5),
            LoadingButton(
              label: Strings.signUpAction,
              loading: _busy,
              onPressed: _submit,
            ),
            const SizedBox(height: Tokens.s3),
            TextButton(
              style: TextButton.styleFrom(minimumSize: const Size(48, 48)),
              onPressed: widget.onSignIn,
              child: const Text(Strings.signInInstead),
            ),
          ],
        ),
      ),
    );
  }
}
