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

/// Ask for a password-reset code.
///
/// Any well-formed phone is answered the same way, so a good answer always
/// goes on to the reset page with the returned challenge ([onCodeSent]), and
/// nothing here says whether the number has an account.
class ForgotPage extends ConsumerStatefulWidget {
  const ForgotPage({super.key, required this.onCodeSent, required this.onBack});

  final void Function(CodeChallenge challenge, String phone) onCodeSent;
  final VoidCallback onBack;

  @override
  ConsumerState<ForgotPage> createState() => _ForgotPageState();
}

class _ForgotPageState extends ConsumerState<ForgotPage> {
  final _phone = TextEditingController();

  String? _phoneError;
  String? _formError;
  bool _retryable = false;
  bool _busy = false;

  @override
  void dispose() {
    _phone.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    if (_busy) return;
    final phone = _phone.text.trim();
    if (!looksLikeNigerianPhone(phone)) {
      setState(() {
        _phoneError = Strings.phoneInvalid;
        _formError = null;
      });
      return;
    }
    setState(() {
      _busy = true;
      _phoneError = null;
      _formError = null;
    });
    try {
      final challenge = await ref
          .read(authRepositoryProvider)
          .requestPasswordReset(phone);
      if (!mounted) return;
      setState(() => _busy = false);
      widget.onCodeSent(challenge, phone);
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _retryable = authErrorIsRetryable(e);
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
      title: Strings.forgotTitle,
      subtitle: Strings.forgotSubtitle,
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
          const SizedBox(height: Tokens.s5),
          LoadingButton(
            label: Strings.forgotAction,
            loading: _busy,
            onPressed: _submit,
          ),
          const SizedBox(height: Tokens.s3),
          TextButton(
            style: TextButton.styleFrom(minimumSize: const Size(48, 48)),
            onPressed: widget.onBack,
            child: const Text(Strings.backToSignIn),
          ),
        ],
      ),
    );
  }
}
