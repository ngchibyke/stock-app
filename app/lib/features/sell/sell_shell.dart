import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/session/me.dart';
import '../../core/session/session_controller.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../chat/chat_pages.dart';
import '../chat/chat_strings.dart';
import 'become_page.dart';
import 'identity/identity_page.dart';
import 'orders.dart';
import 'products.dart';
import 'sell_models.dart';
import 'sell_providers.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';
import 'store_forms.dart';
import 'store_tab.dart';
import 'team_branches.dart';

/// Where the selling area starts. It works out what the person still has to do
/// (an access code, the identity check, their first store) and shows that, or the
/// area itself once they have a store.
class SellEntryPage extends ConsumerWidget {
  const SellEntryPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final session = ref.watch(sessionProvider).value;
    final me = session is SessionSignedIn ? session.me : null;
    final stores = ref.watch(myStoresProvider);
    if (me == null) return const SizedBox.shrink();

    final list = stores.value;
    if (list == null) {
      return Scaffold(
        appBar: sellAppBar(context, SellStrings.sellArea),
        body: stores.hasError
            ? ErrorState(
                message: sellError(stores.error!),
                onRetry: () => ref.invalidate(myStoresProvider),
              )
            : const Center(child: CircularProgressIndicator()),
      );
    }
    if (list.isNotEmpty) return const SellShell();
    if (!me.isSeller) {
      return BecomeSellerPage(
        onRedeemed: () => ref.invalidate(myStoresProvider),
      );
    }
    // A seller with no store: confirm identity, then open one and set the payout.
    final identity = ref.watch(identityStatusProvider);
    final passed = identity.value?.state == IdentityState.passed;
    if (!passed) {
      return identity.when(
        data: (_) => IdentityPage(
          onPassed: () => ref.invalidate(identityStatusProvider),
        ),
        loading: () =>
            const Scaffold(body: Center(child: CircularProgressIndicator())),
        error: (e, _) => Scaffold(
          appBar: sellAppBar(context, SellStrings.sellArea),
          body: ErrorState(
            message: sellError(e),
            onRetry: () => ref.invalidate(identityStatusProvider),
          ),
        ),
      );
    }
    return CreateStorePage(
      onCreated: (store) async {
        await pushPage<void>(context, PayoutPage(store: store));
        ref.invalidate(myStoresProvider);
      },
    );
  }
}

/// Shows the identity check, then reads again what depends on it.
Future<void> confirmIdentity(BuildContext context, WidgetRef ref) async {
  await pushPage<void>(
    context,
    IdentityPage(onPassed: () => Navigator.of(context).pop()),
  );
  ref.invalidate(identityStatusProvider);
  ref.invalidate(myStoresProvider);
}

/// A line asking the person to confirm their identity, when they have not.
class IdentityBanner extends ConsumerWidget {
  const IdentityBanner({super.key, required this.owner});

  final bool owner;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final status = ref.watch(identityStatusProvider).value;
    if (status == null || status.state == IdentityState.passed) {
      return const SizedBox.shrink();
    }
    final due = status.requiredBy;
    final text = owner && due != null
        ? SellStrings.graceBanner('${due.toLocal()}'.split(' ').first)
        : owner
        ? SellStrings.identityTitle
        : SellStrings.identityBannerMember;
    final tokens = context.tokens;
    return Material(
      color: tokens.accent.withValues(alpha: 0.12),
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Tokens.s5,
          vertical: Tokens.s2,
        ),
        child: Row(
          children: [
            Expanded(
              child: Text(text, style: Theme.of(context).textTheme.bodyMedium),
            ),
            TextButton(
              onPressed: () => confirmIdentity(context, ref),
              child: const Text(SellStrings.identityBannerAction),
            ),
          ],
        ),
      ),
    );
  }
}

/// The selling area: Orders, Products, Store and (for owners and managers) Team, for
/// the chosen store.
class SellShell extends ConsumerStatefulWidget {
  const SellShell({super.key});

  @override
  ConsumerState<SellShell> createState() => _SellShellState();
}

class _SellShellState extends ConsumerState<SellShell> {
  int _index = 0;

  @override
  Widget build(BuildContext context) {
    final store = ref.watch(selectedStoreProvider);
    if (store == null) return const SizedBox.shrink();
    final tokens = context.tokens;
    final team = store.role.isManagerUp;
    final tabs = <Widget>[
      OrdersTab(
        key: ValueKey('orders-${store.id}'),
        store: store,
        banner: IdentityBanner(owner: store.role.isOwner),
      ),
      SellChatsTab(
        key: ValueKey('chats-${store.id}'),
        storeId: store.id,
        banner: IdentityBanner(owner: store.role.isOwner),
      ),
      ProductsTab(key: ValueKey('products-${store.id}'), store: store),
      StoreTab(key: ValueKey('store-${store.id}'), store: store),
      if (team) TeamTab(key: ValueKey('team-${store.id}'), store: store),
    ];
    final index = _index >= tabs.length ? 0 : _index;
    return Scaffold(
      body: IndexedStack(index: index, children: tabs),
      bottomNavigationBar: NavigationBarTheme(
        data: NavigationBarThemeData(
          backgroundColor: tokens.card,
          surfaceTintColor: const Color(0x00000000),
          indicatorColor: tokens.brand.withValues(alpha: 0.12),
          labelBehavior: NavigationDestinationLabelBehavior.alwaysShow,
        ),
        child: NavigationBar(
          selectedIndex: index,
          onDestinationSelected: (i) => setState(() => _index = i),
          destinations: [
            const NavigationDestination(
              icon: Icon(Icons.receipt_long_outlined),
              selectedIcon: Icon(Icons.receipt_long),
              label: SellStrings.tabOrders,
            ),
            const NavigationDestination(
              icon: Icon(Icons.chat_bubble_outline),
              selectedIcon: Icon(Icons.chat_bubble),
              label: ChatStrings.chats,
            ),
            const NavigationDestination(
              icon: Icon(Icons.inventory_2_outlined),
              selectedIcon: Icon(Icons.inventory_2),
              label: SellStrings.tabProducts,
            ),
            const NavigationDestination(
              icon: Icon(Icons.storefront_outlined),
              selectedIcon: Icon(Icons.storefront),
              label: SellStrings.tabStore,
            ),
            if (team)
              const NavigationDestination(
                icon: Icon(Icons.group_outlined),
                selectedIcon: Icon(Icons.group),
                label: SellStrings.tabTeam,
              ),
          ],
        ),
      ),
    );
  }
}

/// Used by Account to decide which entry to show.
bool hasSellerSide(Me me, List<SellStore>? stores) =>
    me.isSeller || (stores?.isNotEmpty ?? false);
