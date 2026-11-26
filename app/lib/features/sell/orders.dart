import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/format/money.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import 'disputes.dart';
import 'order_models.dart';
import 'sell_models.dart';
import 'sell_repository.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';

enum OrderFilter { toShip, onTheWay, done }

bool _matches(SellOrder o, OrderFilter f) => switch (f) {
  OrderFilter.toShip => o.status == 'paid',
  OrderFilter.onTheWay => const {
    'picked_up',
    'out_for_delivery',
    'delivered',
  }.contains(o.status),
  OrderFilter.done => !o.isOpen,
};

/// The Orders tab: the store's orders in three groups, newest first, paged.
class OrdersTab extends ConsumerStatefulWidget {
  const OrdersTab({super.key, required this.store, this.banner});

  final SellStore store;

  /// A line above the list (the identity prompt), when there is one.
  final Widget? banner;

  @override
  ConsumerState<OrdersTab> createState() => _OrdersTabState();
}

class _OrdersTabState extends ConsumerState<OrdersTab> {
  final List<SellOrder> _orders = [];
  String? _cursor;
  bool _loading = true;
  bool _more = false;
  String? _error;
  OrderFilter _filter = OrderFilter.toShip;

  @override
  void initState() {
    super.initState();
    unawaited(_load(reset: true));
  }

  @override
  void didUpdateWidget(OrdersTab old) {
    super.didUpdateWidget(old);
    // A different store starts a new list.
    if (old.store.id != widget.store.id) unawaited(_load(reset: true));
  }

  Future<void> _load({bool reset = false}) async {
    final storeId = widget.store.id;
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
          .read(workRepositoryProvider)
          .orders(storeId, before: reset ? null : _cursor);
      if (!mounted || storeId != widget.store.id) return;
      setState(() {
        _orders.addAll(page.items);
        _cursor = page.nextCursor;
      });
    } on Object catch (e) {
      if (mounted) setState(() => _error = sellError(e));
    } finally {
      if (mounted) {
        setState(() {
          _loading = false;
          _more = false;
        });
      }
    }
  }

  Future<void> _open(SellOrder o) async {
    final changed = await pushPage<SellOrder>(
      context,
      OrderPage(store: widget.store, order: o),
    );
    if (changed != null && mounted) {
      setState(() {
        final i = _orders.indexWhere((x) => x.id == changed.id);
        if (i >= 0) _orders[i] = changed;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final shown = [
      for (final o in _orders)
        if (_matches(o, _filter)) o,
    ];
    final tokens = context.tokens;
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: sellRootAppBar(context, SellStrings.tabOrders),
      body: Column(
        children: [
          ?widget.banner,
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Tokens.s5,
              Tokens.s2,
              Tokens.s5,
              Tokens.s2,
            ),
            child: SegmentedButton<OrderFilter>(
              showSelectedIcon: false,
              segments: const [
                ButtonSegment(
                  value: OrderFilter.toShip,
                  label: Text(SellStrings.ordersToShip),
                ),
                ButtonSegment(
                  value: OrderFilter.onTheWay,
                  label: Text(SellStrings.ordersOnTheWay),
                ),
                ButtonSegment(
                  value: OrderFilter.done,
                  label: Text(SellStrings.ordersDone),
                ),
              ],
              selected: {_filter},
              onSelectionChanged: (s) => setState(() => _filter = s.first),
            ),
          ),
          Expanded(
            child: _loading
                ? const AsyncBody<int>(
                    value: AsyncLoading(),
                    builder: _never,
                    onRetry: _noop,
                  )
                : _error != null && _orders.isEmpty
                ? ErrorState(
                    message: _error!,
                    onRetry: () => _load(reset: true),
                  )
                : RefreshIndicator(
                    onRefresh: () => _load(reset: true),
                    child: shown.isEmpty && _cursor == null
                        ? ListView(
                            children: const [
                              SizedBox(height: Tokens.s6),
                              EmptyState(
                                title: SellStrings.noOrders,
                                body: SellStrings.noOrdersBody,
                              ),
                            ],
                          )
                        : ListView.separated(
                            itemCount: shown.length + (_cursor != null ? 1 : 0),
                            separatorBuilder: (_, _) =>
                                const Divider(height: 1),
                            itemBuilder: (context, i) {
                              if (i == shown.length) {
                                return Padding(
                                  padding: const EdgeInsets.all(Tokens.s4),
                                  child: OutlinedButton(
                                    onPressed: _more ? null : () => _load(),
                                    child: const Text(SellStrings.loadMore),
                                  ),
                                );
                              }
                              final o = shown[i];
                              return SellRow(
                                title: o.lines.isEmpty
                                    ? formatNaira(o.totalKobo)
                                    : '${o.lines.first.name}${o.lines.length > 1 ? ' +${o.lines.length - 1}' : ''}',
                                subtitle: [
                                  formatNaira(o.totalKobo),
                                  if (o.method != null)
                                    SellStrings.methodLabel(o.method!.name),
                                  ?o.branchName,
                                ].join('  ·  '),
                                trailing: o.held
                                    ? const StatusPill(
                                        label: SellStrings.disputeOpen,
                                      )
                                    : null,
                                onTap: () => _open(o),
                              );
                            },
                          ),
                  ),
          ),
        ],
      ),
    );
  }
}

Widget _never(int _) => const SizedBox.shrink();
void _noop() {}

String _stepLabel(OrderStep s, DeliveryMethod? m) => switch (s) {
  OrderStep.ready => SellStrings.stepReady,
  OrderStep.pickedUp => SellStrings.stepPickedUp,
  OrderStep.outForDelivery => SellStrings.stepOut,
  OrderStep.delivered =>
    m == DeliveryMethod.collect
        ? SellStrings.stepCollected
        : SellStrings.stepDelivered,
};

/// One order: what was bought, how it goes out, and the one step to take now.
class OrderPage extends ConsumerStatefulWidget {
  const OrderPage({super.key, required this.store, required this.order});

  final SellStore store;
  final SellOrder order;

  @override
  ConsumerState<OrderPage> createState() => _OrderPageState();
}

class _OrderPageState extends ConsumerState<OrderPage> with BusyAction {
  late SellOrder _order = widget.order;

  Future<void> _step(OrderStep step) async {
    String? code;
    if (SellOrder.needsCode(step)) {
      code = await _askCode();
      if (code == null) return;
    }
    await run(() async {
      final next = await ref
          .read(workRepositoryProvider)
          .orderStep(_order.id, step, handoverCode: code);
      if (mounted) setState(() => _order = next);
    });
  }

  Future<String?> _askCode() =>
      showDialog<String>(context: context, builder: (_) => const _CodeDialog());

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final o = _order;
    final step = o.nextStep();
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) Navigator.of(context).pop(_order);
      },
      child: Scaffold(
        backgroundColor: tokens.bg,
        appBar: sellAppBar(context, SellStrings.tabOrders),
        body: SafeArea(
          child: ListView(
            padding: const EdgeInsets.all(Tokens.s5),
            children: [
              for (final l in o.lines)
                Padding(
                  padding: const EdgeInsets.only(bottom: Tokens.s2),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          '${l.quantity} × ${l.name}${l.label.isEmpty ? '' : ' (${l.label})'}',
                          style: theme.textTheme.bodyLarge,
                        ),
                      ),
                      Text(formatNaira(l.unitPriceKobo * l.quantity)),
                    ],
                  ),
                ),
              const Divider(),
              _kv(
                SellStrings.payout,
                o.expectedPayoutKobo == null
                    ? ''
                    : formatNaira(o.expectedPayoutKobo!),
              ),
              if (o.method != null)
                _kv('', SellStrings.methodLabel(o.method!.name)),
              if (o.branchName != null)
                _kv(SellStrings.branchLabel, o.branchName!),
              if (o.buyerName != null) _kv('', o.buyerName!),
              if (o.shipBy != null && o.status == 'paid')
                _kv(
                  SellStrings.shipBy,
                  '${o.shipBy!.toLocal()}'.split('.').first,
                ),
              for (final e in o.doneBy.entries)
                _kv(e.key.replaceAll('_', ' '), e.value),
              const SizedBox(height: Tokens.s3),
              if (o.held)
                Text(SellStrings.orderHeld, style: theme.textTheme.bodyLarge),
              if (o.disputeId != null) ...[
                const SizedBox(height: Tokens.s2),
                OutlinedButton(
                  onPressed: () => pushPage<void>(
                    context,
                    DisputePage(store: widget.store, disputeId: o.disputeId!),
                  ),
                  child: const Text(SellStrings.disputeOpen),
                ),
              ],
              if (error != null) ...[
                const SizedBox(height: Tokens.s3),
                ErrorLine(error!),
              ],
              if (step != null) ...[
                const SizedBox(height: Tokens.s4),
                LoadingButton(
                  label: _stepLabel(step, o.method),
                  loading: busy,
                  onPressed: () => _step(step),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }

  Widget _kv(String k, String v) => Padding(
    padding: const EdgeInsets.symmetric(vertical: Tokens.s1),
    child: Row(
      children: [
        if (k.isNotEmpty) Expanded(child: Text(k)) else const Spacer(),
        Text(v),
      ],
    ),
  );
}

class _CodeDialog extends StatefulWidget {
  const _CodeDialog();

  @override
  State<_CodeDialog> createState() => _CodeDialogState();
}

class _CodeDialogState extends State<_CodeDialog> {
  final _code = TextEditingController();

  @override
  void dispose() {
    _code.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text(SellStrings.codeEntry),
    content: AppTextField(
      label: SellStrings.codeEntry,
      controller: _code,
      keyboardType: TextInputType.number,
      maxLength: 6,
      helperText: SellStrings.codeEntryHelp,
      formatters: [FilteringTextInputFormatter.digitsOnly],
      onChanged: (_) => setState(() {}),
    ),
    actions: [
      TextButton(
        onPressed: () => Navigator.of(context).pop(),
        child: const Text(SellStrings.cancel),
      ),
      FilledButton(
        onPressed: _code.text.length == 6
            ? () => Navigator.of(context).pop(_code.text)
            : null,
        child: const Text(SellStrings.done),
      ),
    ],
  );
}

/// Whether [e] means the handover code is wrong or locked (the order page shows the
/// server's message in plain words either way).
bool isCodeError(Object e) =>
    e is ApiError &&
    (e.code == 'handover_code_wrong' || e.code == 'handover_locked');
