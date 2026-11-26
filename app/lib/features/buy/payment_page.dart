import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';
import 'package:webview_flutter/webview_flutter.dart';

import '../../core/config/app_config.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import 'buy_models.dart';
import 'buy_repository.dart';
import 'buy_strings.dart';
import 'payment_rules.dart';

/// Shows the provider's payment page. A seam, so tests replace the platform
/// browser: [onReturn] is called once when the provider sends the buyer back.
typedef PaymentViewBuilder = Widget Function(
  BuildContext context,
  String url,
  String? returnUrl,
  VoidCallback onReturn,
);

Widget _platformView(
  BuildContext context,
  String url,
  String? returnUrl,
  VoidCallback onReturn,
) => _WebPayment(url: url, returnUrl: returnUrl, onReturn: onReturn);

final paymentViewProvider = Provider<PaymentViewBuilder>(
  (ref) => _platformView,
);

/// How often the result is asked for, and how many times.
final paymentPollProvider = Provider<({Duration every, int times})>(
  (ref) => (every: const Duration(seconds: 3), times: 40),
);

class _WebPayment extends StatefulWidget {
  const _WebPayment({
    required this.url,
    required this.returnUrl,
    required this.onReturn,
  });

  final String url;
  final String? returnUrl;
  final VoidCallback onReturn;

  @override
  State<_WebPayment> createState() => _WebPaymentState();
}

class _WebPaymentState extends State<_WebPayment> {
  late final WebViewController _controller;
  bool _returned = false;
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    _controller = WebViewController()
      ..setJavaScriptMode(JavaScriptMode.unrestricted)
      ..setNavigationDelegate(
        NavigationDelegate(
          onNavigationRequest: (req) {
            if (isReturnUrl(req.url, widget.returnUrl)) {
              _done();
              return NavigationDecision.prevent;
            }
            return mayNavigate(req.url, widget.returnUrl)
                ? NavigationDecision.navigate
                : NavigationDecision.prevent;
          },
          onPageFinished: (_) {
            if (mounted) setState(() => _loading = false);
          },
        ),
      )
      ..loadRequest(Uri.parse(widget.url));
  }

  void _done() {
    if (_returned) return;
    _returned = true;
    widget.onReturn();
  }

  @override
  void dispose() {
    // Nothing of the payment page is kept: cookies and storage go with the view.
    unawaited(_controller.clearCache());
    unawaited(_controller.clearLocalStorage());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Stack(
    children: [
      WebViewWidget(controller: _controller),
      if (_loading) const Center(child: CircularProgressIndicator()),
    ],
  );
}

enum _Stage { paying, checking, paid, pending, failed }

/// The payment: the provider's page, then the server's word on it. The app only
/// ever shows "paid" when `GET /checkout/{id}` says so.
class PaymentPage extends ConsumerStatefulWidget {
  const PaymentPage({
    super.key,
    required this.checkout,
    required this.onFinished,
  });

  final CreatedCheckout checkout;

  /// Called when the buyer leaves with a result: [paid] is whether the server
  /// confirmed it.
  final void Function(bool paid) onFinished;

  @override
  ConsumerState<PaymentPage> createState() => _PaymentPageState();
}

class _PaymentPageState extends ConsumerState<PaymentPage> {
  _Stage _stage = _Stage.paying;
  Timer? _timer;
  int _polls = 0;

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  void _check() {
    if (_stage != _Stage.paying) return;
    setState(() => _stage = _Stage.checking);
    final cfg = ref.read(paymentPollProvider);
    _polls = 0;
    _timer?.cancel();
    unawaited(_poll());
    _timer = Timer.periodic(cfg.every, (_) => _poll());
  }

  Future<void> _poll() async {
    final cfg = ref.read(paymentPollProvider);
    _polls++;
    try {
      final s = await ref
          .read(checkoutRepositoryProvider)
          .status(widget.checkout.checkoutId);
      if (!mounted) return;
      if (s.paid) {
        _timer?.cancel();
        setState(() => _stage = _Stage.paid);
        return;
      }
      if (!s.pending) {
        _timer?.cancel();
        setState(() => _stage = _Stage.failed);
        return;
      }
    } on Object {
      // Try again on the next tick.
    }
    if (_polls >= cfg.times && mounted) {
      _timer?.cancel();
      setState(() => _stage = _Stage.pending);
    }
  }

  @override
  Widget build(BuildContext context) {
    final cfg = ref.watch(appConfigProvider).value ?? AppConfig.empty;
    final Widget body;
    switch (_stage) {
      case _Stage.paying:
        body = mayStartPayment(widget.checkout.paymentUrl)
            ? ref.watch(paymentViewProvider)(
                context,
                widget.checkout.paymentUrl,
                cfg.paymentReturnUrl,
                _check,
              )
            : const ErrorState(message: BuyStrings.paymentBlocked);
      case _Stage.checking:
        body = _message(
          BuyStrings.checking,
          BuyStrings.checkingBody,
          progress: true,
        );
      case _Stage.paid:
        body = _message(
          BuyStrings.paidTitle,
          BuyStrings.paidBody,
          action: BuyStrings.viewOrders,
          onAction: () => widget.onFinished(true),
        );
      case _Stage.pending:
        body = _message(
          BuyStrings.pendingTitle,
          BuyStrings.pendingBody,
          action: BuyStrings.viewOrders,
          onAction: () => widget.onFinished(true),
        );
      case _Stage.failed:
        body = _message(
          BuyStrings.failedTitle,
          BuyStrings.failedBody,
          action: BuyStrings.backToCart,
          onAction: () => widget.onFinished(false),
        );
    }
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: AppBar(
        title: const Text(BuyStrings.paymentTitle),
        backgroundColor: context.tokens.bg,
        surfaceTintColor: const Color(0x00000000),
        automaticallyImplyLeading: false,
        actions: [
          if (_stage == _Stage.paying)
            TextButton(
              onPressed: _check,
              child: const Text(BuyStrings.closePayment),
            ),
        ],
      ),
      body: SafeArea(child: body),
    );
  }

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
