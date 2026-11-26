import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../../core/api/api_error.dart';
import '../../../core/theme/tokens.dart';
import '../../../core/ui/states.dart';
import '../sell_models.dart';
import '../sell_repository.dart';
import '../sell_strings.dart';
import '../sell_widgets.dart';
import 'identity_notice.dart';
import 'nin_input.dart';
import 'selfie_capture.dart';

enum _Stage {
  loading,
  notice,
  nin,
  selfie,
  waiting,
  passed,
  review,
  refused,
  revoked,
  unavailable,
}

/// The identity check: notice, NIN, selfie, waiting and the result. The NIN lives
/// in the field only: it is cleared when the screen goes, when the app is
/// paused and once it has been sent.
class IdentityPage extends ConsumerStatefulWidget {
  const IdentityPage({super.key, required this.onPassed});

  /// Called when the person taps Done after passing.
  final VoidCallback onPassed;

  @override
  ConsumerState<IdentityPage> createState() => _IdentityPageState();
}

class _IdentityPageState extends ConsumerState<IdentityPage>
    with BusyAction, WidgetsBindingObserver {
  _Stage _stage = _Stage.loading;
  bool _agreed = false;
  final TextEditingController _nin = TextEditingController();
  IdentityStart? _started;
  Timer? _poll;
  int _polls = 0;
  String? _note;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(_load());
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _poll?.cancel();
    _nin.clear();
    _nin.dispose();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // The NIN is not kept while the app is in the background.
    if (state != AppLifecycleState.resumed) _nin.clear();
  }

  _Stage _stageFor(IdentityState s) => switch (s) {
    IdentityState.passed => _Stage.passed,
    IdentityState.review => _Stage.review,
    IdentityState.revoked => _Stage.revoked,
    IdentityState.inProgress => _Stage.waiting,
    IdentityState.failed || IdentityState.unverified => _Stage.notice,
  };

  Future<void> _load() async {
    try {
      final status = await ref.read(sellRepositoryProvider).identityStatus();
      if (!mounted) return;
      setState(() => _stage = _stageFor(status.state));
      if (_stage == _Stage.waiting) _startPolling();
    } on Object catch (e) {
      if (!mounted) return;
      setState(() {
        _stage = _Stage.unavailable;
        _note = sellError(e);
      });
    }
  }

  Future<void> _sendNin() async {
    if (!isCompleteNin(_nin.text)) return;
    final nin = _nin.text;
    final ok = await run(() async {
      _started = await ref.read(sellRepositoryProvider).startIdentity(nin);
    });
    // Sent (or refused): the number is not kept either way.
    _nin.clear();
    if (!mounted) return;
    if (ok) {
      setState(() => _stage = _Stage.selfie);
      return;
    }
    // Refusals that end the attempt get their own screen; the rest stay as a line.
    final code = _lastCode;
    if (code == 'identity_not_accepted') {
      setState(() => _stage = _Stage.refused);
    } else if (code == 'identity_unavailable') {
      setState(() => _stage = _Stage.unavailable);
    } else if (code == 'identity_in_review') {
      setState(() => _stage = _Stage.review);
    } else if (code == 'identity_already_verified') {
      setState(() => _stage = _Stage.passed);
    } else if (code == 'identity_in_progress') {
      setState(() => _stage = _Stage.waiting);
      _startPolling();
    }
  }

  String? get _lastCode => _errorCode;
  String? _errorCode;

  @override
  Future<bool> run(Future<void> Function() action) async {
    _errorCode = null;
    return super.run(() async {
      try {
        await action();
      } on ApiError catch (e) {
        _errorCode = e.code;
        rethrow;
      }
    });
  }

  Future<void> _takeSelfie() async {
    final started = _started;
    if (started == null) return;
    final result = await run(() async {
      final r = await ref
          .read(selfieCaptureProvider)
          .capture(started.sessionToken);
      if (r == SelfieResult.cancelled) {
        throw _SelfieNote(SellStrings.selfieCancelled);
      }
      if (r == SelfieResult.failed) {
        throw _SelfieNote(SellStrings.selfieFailed);
      }
    });
    if (!mounted) return;
    if (result) {
      setState(() => _stage = _Stage.waiting);
      _startPolling();
    }
  }

  void _startPolling() {
    final cfg = ref.read(identityPollProvider);
    _poll?.cancel();
    _polls = 0;
    _poll = Timer.periodic(cfg.every, (t) async {
      _polls++;
      try {
        final status = await ref.read(sellRepositoryProvider).identityStatus();
        if (!mounted) return;
        final next = _stageFor(status.state);
        if (status.state == IdentityState.failed) {
          t.cancel();
          setState(() => _stage = _Stage.refused);
        } else if (next != _Stage.waiting && next != _Stage.notice) {
          t.cancel();
          setState(() => _stage = next);
        }
      } on Object {
        // Try again on the next tick.
      }
      if (_polls >= cfg.times) t.cancel();
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    Widget body;
    switch (_stage) {
      case _Stage.loading:
        body = const Center(child: CircularProgressIndicator());
      case _Stage.notice:
        body = _list([
          Text(SellStrings.noticeHeading, style: theme.textTheme.titleLarge),
          const SizedBox(height: Tokens.s3),
          for (final p in IdentityNotice.points) ...[
            Text(p, style: theme.textTheme.bodyLarge),
            const SizedBox(height: Tokens.s3),
          ],
          CheckboxListTile(
            contentPadding: EdgeInsets.zero,
            value: _agreed,
            onChanged: (v) => setState(() => _agreed = v ?? false),
            title: const Text(SellStrings.agree),
            controlAffinity: ListTileControlAffinity.leading,
          ),
          const SizedBox(height: Tokens.s4),
          LoadingButton(
            label: SellStrings.identityContinue,
            onPressed: _agreed
                ? () => setState(() => _stage = _Stage.nin)
                : null,
          ),
        ]);
      case _Stage.nin:
        body = _list([
          TextField(
            controller: _nin,
            keyboardType: TextInputType.number,
            inputFormatters: [ninFormatter],
            maxLength: ninLength,
            obscureText: false,
            enableSuggestions: false,
            autocorrect: false,
            autofillHints: null,
            enableIMEPersonalizedLearning: false,
            decoration: const InputDecoration(
              labelText: SellStrings.ninLabel,
              helperText: SellStrings.ninHelp,
              counterText: '',
            ),
            onChanged: (_) => setState(() {}),
          ),
          if (error != null) ...[
            const SizedBox(height: Tokens.s3),
            ErrorLine(error!),
          ],
          const SizedBox(height: Tokens.s4),
          LoadingButton(
            label: SellStrings.identityContinue,
            loading: busy,
            onPressed: isCompleteNin(_nin.text) ? _sendNin : null,
          ),
        ]);
      case _Stage.selfie:
        body = _list([
          Text(SellStrings.selfieHint, style: theme.textTheme.bodyLarge),
          if (error != null) ...[
            const SizedBox(height: Tokens.s3),
            ErrorLine(error!),
          ],
          const SizedBox(height: Tokens.s4),
          LoadingButton(
            label: SellStrings.takeSelfie,
            loading: busy,
            onPressed: _takeSelfie,
          ),
        ]);
      case _Stage.waiting:
        body = _message(
          SellStrings.waiting,
          SellStrings.waitingBody,
          progress: true,
        );
      case _Stage.passed:
        body = _message(
          SellStrings.passedTitle,
          SellStrings.passedBody,
          action: SellStrings.done,
          onAction: widget.onPassed,
        );
      case _Stage.review:
        body = _message(
          SellStrings.reviewTitle,
          SellStrings.reviewBody,
          action: SellStrings.done,
          onAction: widget.onPassed,
        );
      case _Stage.refused:
        body = _message(SellStrings.refusedTitle, SellStrings.refusedBody);
      case _Stage.revoked:
        body = _message(SellStrings.revokedTitle, SellStrings.revokedBody);
      case _Stage.unavailable:
        body = _message(
          SellStrings.unavailableTitle,
          _note ?? SellStrings.unavailableBody,
          action: SellStrings.tryAgain,
          onAction: () {
            setState(() => _stage = _Stage.loading);
            unawaited(_load());
          },
        );
    }
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: sellAppBar(context, SellStrings.identityTitle),
      body: SafeArea(child: body),
    );
  }

  Widget _list(List<Widget> children) =>
      ListView(padding: const EdgeInsets.all(Tokens.s5), children: children);

  Widget _message(
    String title,
    String body, {
    bool progress = false,
    String? action,
    VoidCallback? onAction,
  }) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.all(Tokens.s5),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (progress) ...[
            const Center(child: CircularProgressIndicator()),
            const SizedBox(height: Tokens.s5),
          ],
          Semantics(
            header: true,
            child: Text(
              title,
              style: theme.textTheme.titleLarge,
              textAlign: TextAlign.center,
            ),
          ),
          const SizedBox(height: Tokens.s3),
          Text(
            body,
            style: theme.textTheme.bodyLarge,
            textAlign: TextAlign.center,
          ),
          if (action != null) ...[
            const SizedBox(height: Tokens.s5),
            LoadingButton(label: action, onPressed: onAction),
          ],
        ],
      ),
    );
  }
}

/// A line to show when the selfie did not finish; carried as an error so the
/// shared busy helper shows it.
class _SelfieNote extends ApiError {
  _SelfieNote(String message)
    : super(status: 0, code: 'selfie', message: message);
}
