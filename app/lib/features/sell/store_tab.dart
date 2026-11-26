import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/paths.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import 'disputes.dart';
import 'reviews.dart';
import 'sell_models.dart';
import 'sell_providers.dart';
import 'sell_repository.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';
import 'store_forms.dart';
import 'team_branches.dart';

/// The Store tab: the store's state and the doors to its settings, payout,
/// branches, reviews and reported problems, and into its room.
class StoreTab extends ConsumerWidget {
  const StoreTab({super.key, required this.store});

  final SellStore store;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final owner = store.role.isOwner;
    final managerUp = store.role.isManagerUp;
    final status = switch (store.status) {
      'active' => SellStrings.storeStatusActive,
      'suspended' => SellStrings.storeStatusSuspended,
      _ => SellStrings.storeStatusPending,
    };
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: sellRootAppBar(context, SellStrings.tabStore),
      body: ListView(
        children: [
          Padding(
            padding: const EdgeInsets.all(Tokens.s5),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(store.name, style: theme.textTheme.titleLarge),
                const SizedBox(height: Tokens.s2),
                StatusPill(label: status, good: store.isActive),
                if (store.status == 'pending') ...[
                  const SizedBox(height: Tokens.s2),
                  Text(
                    SellStrings.payoutWaitingBoth,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: tokens.mute,
                    ),
                  ),
                ],
              ],
            ),
          ),
          if (store.isActive)
            SellRow(
              title: SellStrings.walkIn,
              onTap: () => context.push(storeRoomPath(store.slug)),
            ),
          if (managerUp)
            SellRow(
              title: SellStrings.storeDetails,
              onTap: () async {
                await pushPage<void>(context, StoreDetailsPage(store: store));
                ref.invalidate(myStoresProvider);
              },
            ),
          if (owner)
            SellRow(
              title: SellStrings.payoutRow,
              onTap: () => pushPage<void>(context, PayoutPage(store: store)),
            ),
          if (managerUp)
            SellRow(
              title: SellStrings.branchesRow,
              onTap: () => pushPage<void>(context, BranchesPage(store: store)),
            ),
          if (managerUp)
            SellRow(
              title: SellStrings.reviewsRow,
              onTap: () => pushPage<void>(context, ReviewsPage(store: store)),
            ),
          if (managerUp)
            SellRow(
              title: SellStrings.disputesRow,
              onTap: () => pushPage<void>(context, DisputesPage(store: store)),
            ),
        ],
      ),
    );
  }
}

/// Edit the store's name, description and delivery methods. Pickup details are
/// edited under Branches.
class StoreDetailsPage extends ConsumerStatefulWidget {
  const StoreDetailsPage({super.key, required this.store});

  final SellStore store;

  @override
  ConsumerState<StoreDetailsPage> createState() => _StoreDetailsPageState();
}

class _StoreDetailsPageState extends ConsumerState<StoreDetailsPage>
    with BusyAction {
  late final _name = TextEditingController(text: widget.store.name);
  late final _about = TextEditingController(text: widget.store.description);
  late bool _courier = widget.store.deliveryCourier;
  late bool _self = widget.store.deliverySelf;

  @override
  void dispose() {
    _name.dispose();
    _about.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    final owner = widget.store.role.isOwner;
    final ok = await run(
      () => ref
          .read(sellRepositoryProvider)
          .setStoreDetails(
            widget.store.id,
            name: _name.text.trim(),
            description: _about.text.trim(),
            courier: owner ? _courier : null,
            self: owner ? _self : null,
          ),
    );
    if (ok && mounted) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    backgroundColor: context.tokens.bg,
    appBar: sellAppBar(context, SellStrings.storeDetails),
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
          ),
          if (widget.store.role.isOwner) ...[
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
          ],
          if (error != null) ...[
            const SizedBox(height: Tokens.s3),
            ErrorLine(error!),
          ],
          const SizedBox(height: Tokens.s4),
          LoadingButton(
            label: SellStrings.saveChanges,
            loading: busy,
            onPressed: _name.text.trim().isEmpty || (!_courier && !_self)
                ? null
                : _save,
          ),
        ],
      ),
    ),
  );
}
