import 'package:flutter/services.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../addresses/address.dart';
import '../market/data/market_source.dart';
import 'sell_models.dart';
import 'sell_providers.dart';
import 'sell_repository.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';

/// A phone field's limit, as the sign-up and address forms: digits, at most 11.
final TextInputFormatter _phoneDigits = FilteringTextInputFormatter.allow(
  RegExp(r'[0-9+]'),
);

/// Open a store: name, what is sold, where in the market, the pickup address
/// (which becomes the first branch), delivery.
class CreateStorePage extends ConsumerStatefulWidget {
  const CreateStorePage({super.key, required this.onCreated});

  final void Function(SellStore store) onCreated;

  @override
  ConsumerState<CreateStorePage> createState() => _CreateStorePageState();
}

class _CreateStorePageState extends ConsumerState<CreateStorePage>
    with BusyAction {
  final _name = TextEditingController();
  final _about = TextEditingController();
  final _address = TextEditingController();
  final _phone = TextEditingController();
  final _fee = TextEditingController();
  String? _district;
  AreaCouncil _council = AreaCouncil.amac;
  bool _courier = true;
  bool _self = false;

  @override
  void dispose() {
    for (final c in [_name, _about, _address, _phone, _fee]) {
      c.dispose();
    }
    super.dispose();
  }

  bool get _ready =>
      _name.text.trim().isNotEmpty &&
      _address.text.trim().isNotEmpty &&
      _phone.text.trim().length >= 10 &&
      _district != null &&
      (_courier || _self) &&
      (!_self || _fee.text.trim().isEmpty || nairaToKobo(_fee.text) != null);

  Future<void> _submit() async {
    final district = _district;
    if (!_ready || district == null) return;
    SellStore? made;
    final ok = await run(() async {
      made = await ref
          .read(sellRepositoryProvider)
          .createStore(
            NewStore(
              name: _name.text.trim(),
              description: _about.text.trim(),
              pickupAddress: _address.text.trim(),
              district: district,
              areaCouncil: _council.wire,
              businessPhone: _phone.text.trim(),
              courier: _courier,
              selfDelivery: _self,
              selfDeliveryFeeKobo: _self ? (nairaToKobo(_fee.text) ?? 0) : null,
            ),
          );
    });
    final store = made;
    if (ok && store != null && mounted) {
      ref.invalidate(myStoresProvider);
      widget.onCreated(store);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final market = ref.watch(marketProvider);
    final districts = market.value?.layout.districts ?? const [];
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellAppBar(context, SellStrings.createStoreTitle),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(Tokens.s5),
          children: [
            AppTextField(
              label: SellStrings.storeName,
              controller: _name,
              maxLength: 60,
              capitalization: TextCapitalization.words,
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: Tokens.s3),
            AppTextField(
              label: SellStrings.storeDescription,
              controller: _about,
              maxLines: 3,
              maxLength: 500,
              capitalization: TextCapitalization.sentences,
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: Tokens.s4),
            Text(SellStrings.districtTitle, style: theme.textTheme.titleMedium),
            const SizedBox(height: Tokens.s2),
            Wrap(
              spacing: Tokens.s2,
              runSpacing: Tokens.s2,
              children: [
                for (final d in districts)
                  ChoiceChip(
                    label: Text(d.name),
                    selected: _district == d.id,
                    onSelected: (_) => setState(() => _district = d.id),
                  ),
              ],
            ),
            const SizedBox(height: Tokens.s4),
            AppTextField(
              label: SellStrings.pickupAddress,
              controller: _address,
              maxLines: 2,
              maxLength: 300,
              helperText: SellStrings.pickupNote,
              capitalization: TextCapitalization.words,
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: Tokens.s3),
            DropdownButtonFormField<AreaCouncil>(
              initialValue: _council,
              decoration: const InputDecoration(
                labelText: SellStrings.areaCouncil,
              ),
              items: [
                for (final c in AreaCouncil.values)
                  DropdownMenuItem(value: c, child: Text(c.label)),
              ],
              onChanged: (c) => setState(() => _council = c ?? _council),
            ),
            const SizedBox(height: Tokens.s3),
            AppTextField(
              label: SellStrings.businessPhone,
              controller: _phone,
              keyboardType: TextInputType.phone,
              maxLength: 14,
              formatters: [_phoneDigits],
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: Tokens.s4),
            Text(SellStrings.deliveryTitle, style: theme.textTheme.titleMedium),
            SwitchListTile(
              contentPadding: EdgeInsets.zero,
              title: const Text(SellStrings.courierDelivery),
              value: _courier,
              onChanged: (v) => setState(() => _courier = v),
            ),
            SwitchListTile(
              contentPadding: EdgeInsets.zero,
              title: const Text(SellStrings.selfDelivery),
              value: _self,
              onChanged: (v) => setState(() => _self = v),
            ),
            if (_self)
              AppTextField(
                label: SellStrings.selfDeliveryFee,
                controller: _fee,
                keyboardType: const TextInputType.numberWithOptions(
                  decimal: true,
                ),
                onChanged: (_) => setState(() {}),
              ),
            if (error != null) ...[
              const SizedBox(height: Tokens.s3),
              ErrorLine(error!),
            ],
            const SizedBox(height: Tokens.s4),
            LoadingButton(
              label: SellStrings.createStore,
              loading: busy,
              onPressed: _ready ? _submit : null,
            ),
          ],
        ),
      ),
    );
  }
}

/// The banks the payout account can be at, by CBN code. The server accepts any
/// valid code; this short list covers the common banks (the owner confirms it).
const Map<String, String> payoutBanks = {
  '044': 'Access Bank',
  '058': 'GTBank',
  '057': 'Zenith Bank',
  '011': 'First Bank',
  '033': 'UBA',
  '070': 'Fidelity Bank',
  '232': 'Sterling Bank',
  '035': 'Wema Bank',
  '221': 'Stanbic IBTC',
  '050': 'Ecobank',
  '214': 'FCMB',
  '032': 'Union Bank',
  '215': 'Unity Bank',
  '076': 'Polaris Bank',
  '301': 'Jaiz Bank',
};

/// The store's payout account: bank, account number, and its state.
class PayoutPage extends ConsumerStatefulWidget {
  const PayoutPage({super.key, required this.store});

  final SellStore store;

  @override
  ConsumerState<PayoutPage> createState() => _PayoutPageState();
}

class _PayoutPageState extends ConsumerState<PayoutPage> with BusyAction {
  final _number = TextEditingController();
  final _password = TextEditingController();
  String _bank = payoutBanks.keys.first;
  PayoutAccount? _account;
  bool _loaded = false;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final a = await ref
          .read(sellRepositoryProvider)
          .payoutAccount(widget.store.id);
      if (mounted) {
        setState(() {
          _account = a;
          _loaded = true;
        });
      }
    } on Object catch (e) {
      if (mounted) {
        setState(() {
          error = sellError(e);
          _loaded = true;
        });
      }
    }
  }

  @override
  void dispose() {
    // The number and password are cleared as the page goes.
    _number.clear();
    _password.clear();
    _number.dispose();
    _password.dispose();
    super.dispose();
  }

  bool get _ready =>
      _number.text.length == 10 &&
      (_account == null || _password.text.isNotEmpty);

  Future<void> _save() async {
    if (!_ready) return;
    final number = _number.text;
    final password = _password.text;
    PayoutAccount? saved;
    final ok = await run(() async {
      saved = await ref
          .read(sellRepositoryProvider)
          .setPayoutAccount(
            widget.store.id,
            bankCode: _bank,
            accountNumber: number,
            password: _account == null ? null : password,
          );
    });
    if (ok && mounted) {
      _number.clear();
      _password.clear();
      setState(() => _account = saved);
      ref.invalidate(myStoresProvider);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final a = _account;
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: sellAppBar(context, SellStrings.payoutTitle),
      body: SafeArea(
        child: !_loaded
            ? const Center(child: CircularProgressIndicator())
            : ListView(
                padding: const EdgeInsets.all(Tokens.s5),
                children: [
                  if (a != null) ...[
                    Text(
                      SellStrings.payoutLine(a.last4, a.accountName),
                      style: theme.textTheme.titleMedium,
                    ),
                    const SizedBox(height: Tokens.s2),
                    StatusPill(
                      label: a.verified
                          ? SellStrings.payoutVerified
                          : SellStrings.payoutPending,
                      good: a.verified,
                    ),
                    if (!a.verified && a.pendingReason == 'name_mismatch') ...[
                      const SizedBox(height: Tokens.s2),
                      Text(
                        SellStrings.payoutNameMismatch,
                        style: theme.textTheme.bodyMedium,
                      ),
                    ],
                    if (!widget.store.isActive) ...[
                      const SizedBox(height: Tokens.s3),
                      Text(
                        SellStrings.payoutWaitingBoth,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: tokens.mute,
                        ),
                      ),
                    ],
                    const SizedBox(height: Tokens.s5),
                  ],
                  DropdownButtonFormField<String>(
                    initialValue: _bank,
                    decoration: const InputDecoration(
                      labelText: SellStrings.bank,
                    ),
                    items: [
                      for (final e in payoutBanks.entries)
                        DropdownMenuItem(value: e.key, child: Text(e.value)),
                    ],
                    onChanged: (b) => setState(() => _bank = b ?? _bank),
                  ),
                  const SizedBox(height: Tokens.s3),
                  AppTextField(
                    label: SellStrings.accountNumber,
                    controller: _number,
                    keyboardType: TextInputType.number,
                    maxLength: 10,
                    formatters: [FilteringTextInputFormatter.digitsOnly],
                    onChanged: (_) => setState(() {}),
                  ),
                  if (a != null) ...[
                    const SizedBox(height: Tokens.s3),
                    AppTextField(
                      label: SellStrings.password,
                      controller: _password,
                      obscure: true,
                      helperText: SellStrings.passwordNeeded,
                      onChanged: (_) => setState(() {}),
                    ),
                  ],
                  if (error != null) ...[
                    const SizedBox(height: Tokens.s3),
                    ErrorLine(error!),
                  ],
                  const SizedBox(height: Tokens.s4),
                  LoadingButton(
                    label: SellStrings.savePayout,
                    loading: busy,
                    onPressed: _ready ? _save : null,
                  ),
                ],
              ),
      ),
    );
  }
}
