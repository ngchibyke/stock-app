import 'dart:async';
import 'dart:math';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/format/money.dart';
import '../../core/paths.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import '../../core/ui/states.dart';
import '../addresses/address.dart';
import '../addresses/address_repository.dart';
import '../cart/cart_repository.dart';
import 'buy_models.dart';
import 'buy_repository.dart';
import 'buy_strings.dart';
import 'inbox.dart';
import 'orders_pages.dart';
import 'payment_page.dart';

/// A random key for one checkout attempt: 32 hex characters, no personal data.
String newIdempotencyKey() {
  final rng = Random.secure();
  return List.generate(
    16,
    (_) => rng.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ).join();
}

/// The methods every group of every store can use: one kind of delivery for the
/// whole checkout, as the server takes one method.
List<BuyMethod> commonMethods(List<StoreDelivery> stores) {
  final groups = [for (final s in stores) ...s.groups];
  if (groups.isEmpty) return const [];
  return [
    for (final m in BuyMethod.values)
      if (groups.every((g) => g.options.any((o) => o.method == m))) m,
  ];
}

/// For [method], the option chosen in each group (the cheapest of that kind).
Map<String, DeliveryOption> choose(
  List<StoreDelivery> stores,
  BuyMethod method,
) {
  final out = <String, DeliveryOption>{};
  for (final s in stores) {
    for (final g in s.groups) {
      final matches = g.options.where((o) => o.method == method).toList()
        ..sort((a, b) => a.feeKobo.compareTo(b.feeKobo));
      if (matches.isNotEmpty) out[g.groupId] = matches.first;
    }
  }
  return out;
}

enum _Step { address, options, review }

/// Checkout: the address, the delivery options of each part of the order, and the
/// review before paying.
class CheckoutPage extends ConsumerStatefulWidget {
  const CheckoutPage({super.key});

  @override
  ConsumerState<CheckoutPage> createState() => _CheckoutPageState();
}

class _CheckoutPageState extends ConsumerState<CheckoutPage> {
  _Step _step = _Step.address;
  Address? _address;
  List<StoreDelivery> _stores = const [];
  BuyMethod? _method;
  bool _busy = false;
  String? _error;
  // One key per attempt: a retry of the same choices is the same checkout.
  String _key = newIdempotencyKey();
  String _keyFor = '';

  Future<void> _loadOptions(Address a) async {
    setState(() {
      _address = a;
      _step = _Step.options;
      _busy = true;
      _error = null;
    });
    try {
      final stores = await ref.read(checkoutRepositoryProvider).options(a.id);
      if (!mounted) return;
      final methods = commonMethods(stores);
      setState(() {
        _stores = stores;
        _method = methods.isEmpty ? null : methods.first;
      });
    } on Object catch (e) {
      if (mounted) setState(() => _error = buyError(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  bool get _allReady =>
      _stores.isNotEmpty && _stores.every((s) => s.ready) && _method != null;

  Future<void> _pay() async {
    final address = _address;
    final method = _method;
    if (address == null || method == null || _busy) return;
    final picked = choose(_stores, method);
    final groups = {for (final e in picked.entries) e.key: e.value.id};
    // New choices need a new key; the same choices keep theirs.
    final fingerprint =
        '${address.id}|${method.name}|${(groups.entries.toList()..sort((a, b) => a.key.compareTo(b.key))).map((e) => '${e.key}:${e.value}').join(',')}';
    if (fingerprint != _keyFor) {
      _key = newIdempotencyKey();
      _keyFor = fingerprint;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final created = await ref
          .read(checkoutRepositoryProvider)
          .create(
            method: method.name,
            addressId: address.id,
            groups: groups,
            key: _key,
          );
      if (!mounted) return;
      await Navigator.of(context).push<void>(
        MaterialPageRoute(
          builder: (_) => PaymentPage(
            checkout: created,
            onFinished: (paid) {
              ref.invalidate(cartProvider);
              Navigator.of(context).popUntil((r) => r.isFirst);
              if (paid) {
                unawaited(maybeAskForPush(context, ref));
                unawaited(
                  Navigator.of(context).push<void>(
                    MaterialPageRoute(builder: (_) => const OrdersPage()),
                  ),
                );
              }
            },
          ),
        ),
      );
    } on ApiError catch (e) {
      if (e.code == 'allocation_changed' && _address != null) {
        // Nothing was reserved: show what changed, with fresh options.
        await _loadOptions(_address!);
        if (mounted) setState(() => _error = BuyStrings.optionsChanged);
      } else if (mounted) {
        setState(() => _error = buyError(e));
      }
    } on Object catch (e) {
      if (mounted) setState(() => _error = buyError(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final title = switch (_step) {
      _Step.address => BuyStrings.chooseAddress,
      _Step.options => BuyStrings.deliveryTitle,
      _Step.review => BuyStrings.reviewTitle,
    };
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: AppBar(
        title: Text(title),
        backgroundColor: tokens.bg,
        surfaceTintColor: const Color(0x00000000),
      ),
      body: SafeArea(
        child: switch (_step) {
          _Step.address => _addressStep(),
          _Step.options => _optionsStep(),
          _Step.review => _reviewStep(),
        },
      ),
    );
  }

  Widget _addressStep() {
    final addresses = ref.watch(addressesProvider);
    final theme = Theme.of(context);
    return addresses.when(
      loading: () => const Center(child: CircularProgressIndicator()),
      error: (e, _) => ErrorState(
        message: buyError(e),
        onRetry: () => ref.invalidate(addressesProvider),
      ),
      data: (list) => ListView(
        padding: const EdgeInsets.all(Tokens.s5),
        children: [
          if (list.isEmpty)
            Text(BuyStrings.noAddresses, style: theme.textTheme.bodyLarge),
          for (final a in list)
            Padding(
              padding: const EdgeInsets.only(bottom: Tokens.s3),
              child: _Card(
                onTap: () => _loadOptions(a),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(a.recipientName, style: theme.textTheme.titleMedium),
                    Text(a.addressLine, style: theme.textTheme.bodyMedium),
                    if (a.areaCouncil != null)
                      Text(
                        a.areaCouncil!.label,
                        style: theme.textTheme.bodySmall,
                      ),
                  ],
                ),
              ),
            ),
          OutlinedButton(
            onPressed: () async {
              await context.push(newAddressPath);
              ref.invalidate(addressesProvider);
            },
            child: const Text(BuyStrings.addAddress),
          ),
        ],
      ),
    );
  }

  Widget _optionsStep() {
    final theme = Theme.of(context);
    if (_busy && _stores.isEmpty) {
      return Semantics(
        label: BuyStrings.loadingOptions,
        child: const Center(child: CircularProgressIndicator()),
      );
    }
    if (_error != null && _stores.isEmpty) {
      return ErrorState(
        message: _error!,
        onRetry: _address == null ? null : () => _loadOptions(_address!),
      );
    }
    final methods = commonMethods(_stores);
    final split = _stores.any((s) => s.groups.length > 1);
    return ListView(
      padding: const EdgeInsets.all(Tokens.s5),
      children: [
        if (_error != null) ...[
          ErrorLine(_error!),
          const SizedBox(height: Tokens.s3),
        ],
        for (final s in _stores) ...[
          Text(s.storeName, style: theme.textTheme.titleMedium),
          const SizedBox(height: Tokens.s2),
          for (final line in s.short)
            Text(
              BuyStrings.onlyAvailable(line.available),
              style: theme.textTheme.bodyMedium?.copyWith(
                color: context.tokens.errorText,
              ),
            ),
          for (final g in s.groups)
            Padding(
              padding: const EdgeInsets.only(bottom: Tokens.s2),
              child: Text(
                g.unavailable != null
                    ? BuyStrings.nothingToSend
                    : BuyStrings.sentFrom(g.branchName),
                style: theme.textTheme.bodyMedium,
              ),
            ),
          const SizedBox(height: Tokens.s3),
        ],
        if (split)
          Padding(
            padding: const EdgeInsets.only(bottom: Tokens.s3),
            child: Text(
              BuyStrings.splitNote,
              style: theme.textTheme.bodyMedium,
            ),
          ),
        if (_stores.any((s) => s.short.isNotEmpty))
          Padding(
            padding: const EdgeInsets.only(bottom: Tokens.s3),
            child: Text(
              BuyStrings.changeCart,
              style: theme.textTheme.bodyLarge,
            ),
          )
        else if (methods.isEmpty)
          Padding(
            padding: const EdgeInsets.only(bottom: Tokens.s3),
            child: Text(
              BuyStrings.methodMixed,
              style: theme.textTheme.bodyLarge,
            ),
          ),
        for (final m in methods)
          Padding(
            padding: const EdgeInsets.only(bottom: Tokens.s2),
            child: _Card(
              selected: _method == m,
              onTap: () => setState(() => _method = m),
              child: _methodRow(m),
            ),
          ),
        if (_method == BuyMethod.collect)
          Text(BuyStrings.collectNote, style: theme.textTheme.bodyMedium),
        const SizedBox(height: Tokens.s4),
        LoadingButton(
          label: BuyStrings.reviewTitle,
          onPressed: _allReady
              ? () => setState(() => _step = _Step.review)
              : null,
        ),
      ],
    );
  }

  Widget _methodRow(BuyMethod m) {
    final theme = Theme.of(context);
    final picked = choose(_stores, m);
    final fee = picked.values.fold<int>(0, (a, o) => a + o.feeKobo);
    final dates =
        picked.values
            .map((o) => o.promisedDate)
            .where((d) => d.isNotEmpty)
            .toList()
          ..sort();
    final label = switch (m) {
      BuyMethod.courier => BuyStrings.courier,
      BuyMethod.self => BuyStrings.selfDelivery,
      BuyMethod.collect => BuyStrings.collect,
    };
    return Row(
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(label, style: theme.textTheme.titleMedium),
              if (dates.isNotEmpty)
                Text(
                  BuyStrings.promised(dates.last),
                  style: theme.textTheme.bodyMedium,
                ),
            ],
          ),
        ),
        Text(
          fee == 0 ? BuyStrings.free : formatNaira(fee),
          style: theme.textTheme.titleMedium,
        ),
      ],
    );
  }

  Widget _reviewStep() {
    final theme = Theme.of(context);
    final cart = ref.watch(cartProvider).value;
    final picked = _method == null
        ? const <String, DeliveryOption>{}
        : choose(_stores, _method!);
    final fee = picked.values.fold<int>(0, (a, o) => a + o.feeKobo);
    final goods = cart?.totalKobo ?? 0;
    return ListView(
      padding: const EdgeInsets.all(Tokens.s5),
      children: [
        if (_error != null) ...[
          ErrorLine(_error!),
          const SizedBox(height: Tokens.s3),
        ],
        _row(BuyStrings.itemsTotal, formatNaira(goods)),
        _row(
          BuyStrings.delivery,
          fee == 0 ? BuyStrings.free : formatNaira(fee),
        ),
        const Divider(),
        _row(BuyStrings.total, formatNaira(goods + fee), bold: true),
        if (_method == BuyMethod.collect) ...[
          const SizedBox(height: Tokens.s3),
          Text(BuyStrings.collectNote, style: theme.textTheme.bodyMedium),
        ],
        const SizedBox(height: Tokens.s5),
        LoadingButton(label: BuyStrings.pay, loading: _busy, onPressed: _pay),
      ],
    );
  }

  Widget _row(String k, String v, {bool bold = false}) => Padding(
    padding: const EdgeInsets.symmetric(vertical: Tokens.s1),
    child: Row(
      children: [
        Expanded(
          child: Text(
            k,
            style: bold ? Theme.of(context).textTheme.titleMedium : null,
          ),
        ),
        Text(v, style: bold ? Theme.of(context).textTheme.titleLarge : null),
      ],
    ),
  );
}

class _Card extends StatelessWidget {
  const _Card({
    required this.child,
    required this.onTap,
    this.selected = false,
  });

  final Widget child;
  final VoidCallback onTap;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Pressable(
      onTap: onTap,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: tokens.card,
          borderRadius: BorderRadius.circular(Tokens.radius),
          border: Border.all(
            color: selected ? tokens.accent : tokens.line,
            width: selected ? 2 : 1,
          ),
        ),
        child: Padding(padding: const EdgeInsets.all(Tokens.s4), child: child),
      ),
    );
  }
}

class ErrorLine extends StatelessWidget {
  const ErrorLine(this.message, {super.key});

  final String message;

  @override
  Widget build(BuildContext context) => Semantics(
    liveRegion: true,
    child: Text(
      message,
      style: Theme.of(context).textTheme.bodyMedium
          ?.copyWith(color: context.tokens.errorText),
    ),
  );
}
