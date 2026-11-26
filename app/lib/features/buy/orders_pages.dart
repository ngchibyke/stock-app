import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/format/money.dart';
import '../../core/links/external_link.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/secure_screen.dart';
import '../../core/ui/states.dart';
import 'buy_models.dart';
import 'buy_repository.dart';
import 'buy_strings.dart';
import 'checkout_page.dart' show ErrorLine;
import 'dispute_pages.dart';

Future<T?> _push<T>(BuildContext context, Widget page) =>
    Navigator.of(context).push<T>(MaterialPageRoute<T>(builder: (_) => page));

/// The buyer's orders, newest first, paged.
class OrdersPage extends ConsumerStatefulWidget {
  const OrdersPage({super.key});

  @override
  ConsumerState<OrdersPage> createState() => _OrdersPageState();
}

class _OrdersPageState extends ConsumerState<OrdersPage> {
  final List<BuyOrder> _orders = [];
  String? _cursor;
  bool _loading = true;
  bool _more = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    unawaited(_load(reset: true));
  }

  Future<void> _load({bool reset = false}) async {
    if (reset) {
      setState(() {
        _loading = true;
        _error = null;
        _orders.clear();
        _cursor = null;
      });
    } else {
      setState(() => _more = true);
    }
    try {
      final page = await ref
          .read(buyOrdersRepositoryProvider)
          .orders(before: reset ? null : _cursor);
      if (!mounted) return;
      setState(() {
        _orders.addAll(page.items);
        _cursor = page.nextCursor;
      });
    } on Object catch (e) {
      if (mounted) setState(() => _error = buyError(e));
    } finally {
      if (mounted) {
        setState(() {
          _loading = false;
          _more = false;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final theme = Theme.of(context);
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: AppBar(
        title: const Text(BuyStrings.ordersTitle),
        backgroundColor: tokens.bg,
        surfaceTintColor: const Color(0x00000000),
      ),
      body: _loading
          ? const Padding(
              padding: EdgeInsets.all(Tokens.s5),
              child: Column(
                children: [
                  Skeleton(height: 72),
                  SizedBox(height: Tokens.s3),
                  Skeleton(height: 72),
                ],
              ),
            )
          : _error != null && _orders.isEmpty
          ? ErrorState(message: _error!, onRetry: () => _load(reset: true))
          : RefreshIndicator(
              onRefresh: () => _load(reset: true),
              child: _orders.isEmpty
                  ? ListView(
                      children: const [
                        SizedBox(height: Tokens.s6),
                        EmptyState(
                          title: BuyStrings.noOrders,
                          body: BuyStrings.noOrdersBody,
                        ),
                      ],
                    )
                  : ListView.separated(
                      itemCount: _orders.length + (_cursor != null ? 1 : 0),
                      separatorBuilder: (_, _) => const Divider(height: 1),
                      itemBuilder: (context, i) {
                        if (i == _orders.length) {
                          return Padding(
                            padding: const EdgeInsets.all(Tokens.s4),
                            child: OutlinedButton(
                              onPressed: _more ? null : () => _load(),
                              child: const Text(BuyStrings.loadMore),
                            ),
                          );
                        }
                        final o = _orders[i];
                        return ListTile(
                          minTileHeight: Tokens.minTap,
                          title: Text(
                            o.lines.isEmpty
                                ? formatNaira(o.totalKobo)
                                : '${o.lines.first.name}${o.lines.length > 1 ? ' +${o.lines.length - 1}' : ''}',
                          ),
                          subtitle: Text(
                            '${BuyStrings.statusLabel(o.status)}  ·  ${formatNaira(o.totalKobo)}',
                            style: theme.textTheme.bodyMedium?.copyWith(
                              color: tokens.mute,
                            ),
                          ),
                          trailing: const Icon(Icons.chevron_right),
                          onTap: () async {
                            await _push<void>(
                              context,
                              BuyOrderPage(orderId: o.id),
                            );
                            unawaited(_load(reset: true));
                          },
                        );
                      },
                    ),
            ),
    );
  }
}

/// One order: where it is, the handover code or pickup point, and what the buyer can do.
class BuyOrderPage extends ConsumerStatefulWidget {
  const BuyOrderPage({super.key, required this.orderId});

  final String orderId;

  @override
  ConsumerState<BuyOrderPage> createState() => _BuyOrderPageState();
}

class _BuyOrderPageState extends ConsumerState<BuyOrderPage> {
  BuyOrder? _order;
  Tracking? _tracking;
  String? _error;
  String? _actionError;
  bool _busy = false;
  bool _secure = false;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void dispose() {
    if (_secure) unawaited(SecureScreen.set(false));
    super.dispose();
  }

  Future<void> _load() async {
    final repo = ref.read(buyOrdersRepositoryProvider);
    try {
      final order = await repo.order(widget.orderId);
      Tracking? tracking;
      try {
        tracking = await repo.tracking(widget.orderId);
      } on Object {
        // The order still shows without its steps.
      }
      if (!mounted) return;
      setState(() {
        _order = order;
        _tracking = tracking;
        _error = null;
      });
      final show = tracking?.handoverCode != null;
      if (show != _secure) {
        _secure = show;
        unawaited(SecureScreen.set(show));
      }
    } on Object catch (e) {
      if (mounted) setState(() => _error = buyError(e));
    }
  }

  Future<void> _act(
    String title,
    String body,
    Future<void> Function() run,
  ) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(title),
        content: Text(body),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text(BuyStrings.keep),
          ),
          FilledButton(
            style: FilledButton.styleFrom(
              minimumSize: const Size(0, Tokens.minTap),
            ),
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text(BuyStrings.confirm),
          ),
        ],
      ),
    );
    if (sure != true || _busy) return;
    setState(() {
      _busy = true;
      _actionError = null;
    });
    try {
      await run();
      await _load();
    } on Object catch (e) {
      if (mounted) setState(() => _actionError = buyError(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final o = _order;
    final t = _tracking;
    final repo = ref.read(buyOrdersRepositoryProvider);
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: AppBar(
        title: const Text(BuyStrings.orderTitle),
        backgroundColor: tokens.bg,
        surfaceTintColor: const Color(0x00000000),
      ),
      body: SafeArea(
        child: o == null
            ? (_error != null
                  ? ErrorState(message: _error!, onRetry: _load)
                  : const Center(child: CircularProgressIndicator()))
            : RefreshIndicator(
                onRefresh: _load,
                child: ListView(
                  padding: const EdgeInsets.all(Tokens.s5),
                  children: [
                    Text(
                      BuyStrings.statusLabel(o.status),
                      style: theme.textTheme.titleLarge,
                    ),
                    const SizedBox(height: Tokens.s3),
                    for (final l in o.lines)
                      Padding(
                        padding: const EdgeInsets.only(bottom: Tokens.s1),
                        child: Row(
                          children: [
                            Expanded(child: Text('${l.quantity} × ${l.name}')),
                            Text(formatNaira(l.unitPriceKobo * l.quantity)),
                          ],
                        ),
                      ),
                    const Divider(),
                    Row(
                      children: [
                        const Expanded(child: Text(BuyStrings.total)),
                        Text(
                          formatNaira(o.totalKobo),
                          style: theme.textTheme.titleMedium,
                        ),
                      ],
                    ),
                    if (o.refundedKobo != null)
                      Text(
                        BuyStrings.refunded(formatNaira(o.refundedKobo!)),
                        style: theme.textTheme.bodyMedium,
                      ),
                    if (t != null) ...[
                      const SizedBox(height: Tokens.s5),
                      Text(
                        BuyStrings.trackingTitle,
                        style: theme.textTheme.titleMedium,
                      ),
                      const SizedBox(height: Tokens.s2),
                      if (t.promisedDate != null && o.isOpenForDelivery)
                        Text(
                          BuyStrings.promised(t.promisedDate!),
                          style: theme.textTheme.bodyMedium,
                        ),
                      if (t.delayNote != null)
                        Text(t.delayNote!, style: theme.textTheme.bodyMedium),
                      for (final s in t.steps)
                        Padding(
                          padding: const EdgeInsets.symmetric(
                            vertical: Tokens.s1,
                          ),
                          child: Row(
                            children: [
                              Icon(
                                s.at != null
                                    ? Icons.check_circle
                                    : (s.current
                                          ? Icons.radio_button_checked
                                          : Icons.radio_button_unchecked),
                                size: 20,
                                color: s.at != null ? tokens.good : tokens.mute,
                              ),
                              const SizedBox(width: Tokens.s2),
                              Expanded(
                                child: Text(
                                  s.label,
                                  style: s.current
                                      ? theme.textTheme.titleSmall
                                      : theme.textTheme.bodyMedium,
                                ),
                              ),
                            ],
                          ),
                        ),
                      if (t.trackingUrl != null)
                        TextButton(
                          onPressed: () => ref.read(externalLinkOpenerProvider)(
                            Uri.parse(t.trackingUrl!),
                          ),
                          child: const Text(BuyStrings.openTracking),
                        ),
                      if (t.pickupBranch != null) ...[
                        const SizedBox(height: Tokens.s4),
                        Text(
                          BuyStrings.pickupTitle,
                          style: theme.textTheme.titleMedium,
                        ),
                        Text(t.pickupBranch!, style: theme.textTheme.bodyLarge),
                        if (t.pickupAddress != null)
                          Text(
                            t.pickupAddress!,
                            style: theme.textTheme.bodyMedium,
                          ),
                        if (t.pickupPhone != null)
                          Text(
                            t.pickupPhone!,
                            style: theme.textTheme.bodyMedium,
                          ),
                      ],
                      if (t.handoverCode != null) ...[
                        const SizedBox(height: Tokens.s4),
                        Text(
                          BuyStrings.handoverTitle,
                          style: theme.textTheme.titleMedium,
                        ),
                        Semantics(
                          label: BuyStrings.handoverTitle,
                          child: Text(
                            t.handoverCode!,
                            style: theme.textTheme.displaySmall?.copyWith(
                              letterSpacing: 6,
                              fontWeight: FontWeight.w700,
                            ),
                          ),
                        ),
                        Text(
                          t.handoverNote ?? BuyStrings.handoverHint,
                          style: theme.textTheme.bodyMedium,
                        ),
                      ],
                    ],
                    if (_actionError != null) ...[
                      const SizedBox(height: Tokens.s3),
                      ErrorLine(_actionError!),
                    ],
                    const SizedBox(height: Tokens.s5),
                    if (o.canConfirm)
                      LoadingButton(
                        label: BuyStrings.received,
                        loading: _busy,
                        onPressed: () => _act(
                          BuyStrings.received,
                          BuyStrings.receivedBody,
                          () => repo.received(o.id),
                        ),
                      ),
                    if (o.canCancel) ...[
                      const SizedBox(height: Tokens.s2),
                      OutlinedButton(
                        onPressed: _busy
                            ? null
                            : () => _act(
                                BuyStrings.cancel,
                                BuyStrings.cancelBody,
                                () => repo.cancel(o.id),
                              ),
                        child: const Text(BuyStrings.cancel),
                      ),
                    ],
                    if (o.canReport) ...[
                      const SizedBox(height: Tokens.s2),
                      OutlinedButton(
                        onPressed: () async {
                          await _push<void>(context, ReportPage(orderId: o.id));
                          unawaited(_load());
                        },
                        child: const Text(BuyStrings.reportProblem),
                      ),
                    ],
                    if (o.disputeId != null) ...[
                      const SizedBox(height: Tokens.s2),
                      OutlinedButton(
                        onPressed: () => _push<void>(
                          context,
                          DisputeStatusPage(disputeId: o.disputeId!),
                        ),
                        child: const Text(BuyStrings.seeReport),
                      ),
                    ],
                    if (o.canReview) ...[
                      const SizedBox(height: Tokens.s2),
                      OutlinedButton(
                        onPressed: () =>
                            _push<void>(context, ReviewPage(orderId: o.id)),
                        child: const Text(BuyStrings.leaveReview),
                      ),
                    ],
                  ],
                ),
              ),
      ),
    );
  }
}
