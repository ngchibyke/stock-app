import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_animate/flutter_animate.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../ui/app_icon.dart';
import '../../ui/buttons.dart';
import '../../ui/motion.dart';
import 'auth_controller.dart';
import 'auth_models.dart';
import 'phone_format.dart';
import 'widgets/otp_boxes.dart';

String _message(Object e) =>
    e is ApiError ? e.message : 'Something went wrong. Try again.';

/// A form error that slides in under the fields.
class _FormError extends StatelessWidget {
  const _FormError(this.text);
  final String? text;

  @override
  Widget build(BuildContext context) {
    return AnimatedSize(
      duration: Motion.base,
      curve: Motion.out,
      child: text == null
          ? const SizedBox(width: double.infinity)
          : Container(
                  width: double.infinity,
                  margin: const EdgeInsets.only(top: 14),
                  padding: const EdgeInsets.all(12),
                  decoration: BoxDecoration(
                    color: Theme.of(context).colorScheme.errorContainer,
                    borderRadius: BorderRadius.circular(12),
                  ),
                  child: Row(
                    children: [
                      AppIcon(
                        AppIcons.warn,
                        size: 20,
                        color: Theme.of(context).colorScheme.onErrorContainer,
                        active: true,
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Text(
                          text!,
                          style: TextStyle(
                            color: Theme.of(context)
                                .colorScheme
                                .onErrorContainer,
                          ),
                          key: const Key('form-error'),
                        ),
                      ),
                    ],
                  ),
                )
                .animate()
                .fadeIn(duration: 200.ms)
                .slideY(begin: -0.1, end: 0, duration: 240.ms),
    );
  }
}

class WelcomeScreen extends StatelessWidget {
  const WelcomeScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Spacer(),
              Container(
                    width: 76,
                    height: 76,
                    decoration: BoxDecoration(
                      gradient: LinearGradient(
                        colors: [scheme.primary, scheme.tertiary],
                      ),
                      borderRadius: BorderRadius.circular(24),
                    ),
                    child: const Center(
                      child: AppIcon(
                        AppIcons.store,
                        size: 40,
                        color: Colors.white,
                      ),
                    ),
                  )
                  .animate()
                  .scale(duration: 600.ms, curve: Curves.easeOutBack)
                  .then()
                  .shimmer(duration: 1400.ms, color: Colors.white54),
              const SizedBox(height: 28),
              Text(
                    'Shop Abuja,\nsafely.',
                    style: Theme.of(context).textTheme.displaySmall
                        ?.copyWith(fontWeight: FontWeight.w800, height: 1.05),
                  )
                  .animate()
                  .fadeIn(delay: 150.ms, duration: 400.ms)
                  .slideY(
                    begin: 0.15,
                    end: 0,
                    duration: 400.ms,
                    curve: Curves.easeOutCubic,
                  ),
              const SizedBox(height: 12),
              Text(
                'Your money is held until your order arrives. Chat with sellers, track every step, and get help if something is wrong.',
                style: TextStyle(
                  color: scheme.onSurfaceVariant,
                  fontSize: 16,
                  height: 1.4,
                ),
              ).animate().fadeIn(delay: 300.ms, duration: 400.ms),
              const Spacer(),
              AppButton(
                label: 'Create an account',
                expand: true,
                onPressed: () => context.push('/sign-up'),
              ),
              const SizedBox(height: 12),
              AppButton(
                label: 'I already have an account',
                style: AppButtonStyle.secondary,
                expand: true,
                onPressed: () => context.push('/sign-in'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class SignUpScreen extends ConsumerStatefulWidget {
  const SignUpScreen({super.key});

  @override
  ConsumerState<SignUpScreen> createState() => _SignUpScreenState();
}

class _SignUpScreenState extends ConsumerState<SignUpScreen> {
  final _name = TextEditingController();
  final _phone = TextEditingController();
  final _password = TextEditingController();
  bool _show = false;
  bool _busy = false;
  String? _error;

  bool get _ready =>
      _name.text.trim().isNotEmpty &&
      phoneForServer(_phone.text).length >= 10 &&
      _password.text.length >= 10;

  Future<void> _submit() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final pending = await ref
          .read(authControllerProvider.notifier)
          .signUp(
            phone: phoneForServer(_phone.text),
            password: _password.text,
            displayName: _name.text.trim(),
          );
      if (mounted) context.push('/otp', extra: pending);
    } catch (e) {
      if (mounted) setState(() => _error = _message(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  void dispose() {
    _name.dispose();
    _phone.dispose();
    _password.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        leading: AppIconButton(
          icon: AppIcons.back,
          label: 'Back',
          onPressed: () => context.pop(),
        ),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(24, 8, 24, 24),
          children: [
            Text(
              'Create your account',
              style: Theme.of(context).textTheme.headlineMedium
                  ?.copyWith(fontWeight: FontWeight.w800),
            ),
            const SizedBox(height: 6),
            Text(
              'We will text you a code to check your number.',
              style: TextStyle(
                color: Theme.of(context).colorScheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: 24),
            TextField(
              controller: _name,
              textCapitalization: TextCapitalization.words,
              autofillHints: const [AutofillHints.name],
              decoration: const InputDecoration(labelText: 'Your name'),
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: 14),
            TextField(
              controller: _phone,
              keyboardType: TextInputType.phone,
              autofillHints: const [AutofillHints.telephoneNumber],
              decoration: const InputDecoration(
                labelText: 'Phone number',
                hintText: '0803 123 4567',
              ),
              inputFormatters: [_PhoneFormatter()],
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: 14),
            TextField(
              controller: _password,
              obscureText: !_show,
              autofillHints: const [AutofillHints.newPassword],
              decoration: InputDecoration(
                labelText: 'Password',
                helperText: 'At least 10 characters',
                suffixIcon: AppIconButton(
                  icon: _show ? AppIcons.eyeOff : AppIcons.eye,
                  label: _show ? 'Hide password' : 'Show password',
                  onPressed: () => setState(() => _show = !_show),
                ),
              ),
              onChanged: (_) => setState(() {}),
            ),
            _FormError(_error),
            const SizedBox(height: 24),
            AppButton(
              label: 'Continue',
              expand: true,
              loading: _busy,
              onPressed: _ready ? _submit : null,
            ),
          ],
        ),
      ),
    );
  }
}

class _PhoneFormatter extends TextInputFormatter {
  @override
  TextEditingValue formatEditUpdate(
    TextEditingValue old,
    TextEditingValue next,
  ) {
    final text = formatPhoneInput(next.text);
    return TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
  }
}

class SignInScreen extends ConsumerStatefulWidget {
  const SignInScreen({super.key});

  @override
  ConsumerState<SignInScreen> createState() => _SignInScreenState();
}

class _SignInScreenState extends ConsumerState<SignInScreen> {
  final _phone = TextEditingController();
  final _password = TextEditingController();
  bool _show = false;
  bool _busy = false;
  String? _error;
  int _shake = 0;

  bool get _ready =>
      phoneForServer(_phone.text).length >= 10 && _password.text.isNotEmpty;

  Future<void> _submit() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final pending = await ref
          .read(authControllerProvider.notifier)
          .signIn(phone: phoneForServer(_phone.text), password: _password.text);
      if (pending != null && mounted) context.push('/otp', extra: pending);
    } catch (e) {
      if (mounted) {
        setState(() {
          _error = e is ApiError && e.code == 'invalid_credentials'
              ? 'That phone number and password do not match.'
              : _message(e);
          _shake++;
        });
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  void dispose() {
    _phone.dispose();
    _password.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final form = Column(
      children: [
        TextField(
          controller: _phone,
          keyboardType: TextInputType.phone,
          autofillHints: const [AutofillHints.telephoneNumber],
          decoration: const InputDecoration(
            labelText: 'Phone number',
            hintText: '0803 123 4567',
          ),
          inputFormatters: [_PhoneFormatter()],
          onChanged: (_) => setState(() {}),
        ),
        const SizedBox(height: 14),
        TextField(
          controller: _password,
          obscureText: !_show,
          autofillHints: const [AutofillHints.password],
          decoration: InputDecoration(
            labelText: 'Password',
            suffixIcon: AppIconButton(
              icon: _show ? AppIcons.eyeOff : AppIcons.eye,
              label: _show ? 'Hide password' : 'Show password',
              onPressed: () => setState(() => _show = !_show),
            ),
          ),
          onChanged: (_) => setState(() {}),
          onSubmitted: (_) => _ready && !_busy ? _submit() : null,
        ),
      ],
    );
    return Scaffold(
      appBar: AppBar(
        leading: AppIconButton(
          icon: AppIcons.back,
          label: 'Back',
          onPressed: () => context.pop(),
        ),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(24, 8, 24, 24),
          children: [
            Text(
              'Welcome back',
              style: Theme.of(context).textTheme.headlineMedium
                  ?.copyWith(fontWeight: FontWeight.w800),
            ),
            const SizedBox(height: 24),
            Motion.reduced(context) || _shake == 0
                ? form
                : form
                      .animate(key: ValueKey(_shake))
                      .shakeX(hz: 5, amount: 8, duration: 420.ms),
            _FormError(_error),
            const SizedBox(height: 24),
            AppButton(
              label: 'Sign in',
              expand: true,
              loading: _busy,
              onPressed: _ready ? _submit : null,
            ),
          ],
        ),
      ),
    );
  }
}

/// The code screen. A wrong code shakes the boxes; the resend button drains a
/// 30-second ring before it can be pressed again.
class OtpScreen extends ConsumerStatefulWidget {
  const OtpScreen({super.key, required this.pending});
  final PendingVerification pending;

  @override
  ConsumerState<OtpScreen> createState() => _OtpScreenState();
}

class _OtpScreenState extends ConsumerState<OtpScreen> {
  static const _cooldown = 30;
  late PendingVerification _pending = widget.pending;
  bool _busy = false;
  String? _error;
  int _shake = 0;
  int _left = _cooldown;
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    _startTimer();
  }

  void _startTimer() {
    _timer?.cancel();
    setState(() => _left = _cooldown);
    _timer = Timer.periodic(const Duration(seconds: 1), (t) {
      if (_left <= 1) {
        t.cancel();
      }
      if (mounted) setState(() => _left = _left > 0 ? _left - 1 : 0);
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  Future<void> _verify(String code) async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await ref.read(authControllerProvider.notifier).verify(_pending, code);
      // The router sees the signed-in state and moves on.
    } catch (e) {
      if (mounted) {
        HapticFeedback.mediumImpact();
        setState(() {
          _error = _message(e);
          _shake++;
        });
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _resend() async {
    setState(() => _error = null);
    try {
      final next = await ref
          .read(authControllerProvider.notifier)
          .resend(_pending);
      if (!mounted) return;
      setState(() => _pending = next);
      _startTimer();
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(const SnackBar(content: Text('We sent you a new code.')));
    } catch (e) {
      if (mounted) setState(() => _error = _message(e));
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      appBar: AppBar(
        leading: AppIconButton(
          icon: AppIcons.back,
          label: 'Back',
          onPressed: () => context.pop(),
        ),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(24, 8, 24, 24),
          children: [
            Text(
              'Enter the code',
              style: Theme.of(context).textTheme.headlineMedium
                  ?.copyWith(fontWeight: FontWeight.w800),
            ),
            const SizedBox(height: 6),
            Text(
              'We texted a 6-digit code to ${formatPhoneInput(_pending.phone)}.',
              style: TextStyle(color: scheme.onSurfaceVariant),
            ),
            const SizedBox(height: 28),
            OtpBoxes(onCompleted: _verify, shakeKey: _shake, enabled: !_busy),
            _FormError(_error),
            const SizedBox(height: 28),
            Center(
              child: _left > 0
                  ? Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        SizedBox(
                          width: 18,
                          height: 18,
                          child: CircularProgressIndicator(
                            value: _left / _cooldown,
                            strokeWidth: 2.4,
                          ),
                        ),
                        const SizedBox(width: 10),
                        Text(
                          'Send a new code in ${_left}s',
                          style: TextStyle(color: scheme.onSurfaceVariant),
                        ),
                      ],
                    )
                  : AppButton(
                      label: 'Send a new code',
                      icon: AppIcons.refresh,
                      style: AppButtonStyle.ghost,
                      onPressed: _resend,
                    ),
            ),
            if (_busy)
              const Padding(
                padding: EdgeInsets.only(top: 20),
                child: Center(child: CircularProgressIndicator()),
              ),
          ],
        ),
      ),
    );
  }
}
