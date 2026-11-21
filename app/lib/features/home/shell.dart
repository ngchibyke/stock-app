import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/strings.dart';
import '../../core/theme/tokens.dart';

/// The signed-in frame: the page of the open tab and a bottom navigation with
/// Market and Account. Each tab keeps its own state (scroll position, pages
/// pushed inside it) while another tab is open.
class Shell extends StatelessWidget {
  const Shell({super.key, required this.navigationShell});

  final StatefulNavigationShell navigationShell;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final textTheme = Theme.of(context).textTheme;

    WidgetStateProperty<Color?> colour() => WidgetStateProperty.resolveWith(
      (states) =>
          states.contains(WidgetState.selected) ? tokens.brand : tokens.mute,
    );

    return Scaffold(
      body: navigationShell,
      // The bar pads itself for the system gesture area at the bottom.
      bottomNavigationBar: NavigationBarTheme(
        data: NavigationBarThemeData(
          backgroundColor: tokens.card,
          surfaceTintColor: const Color(0x00000000),
          indicatorColor: tokens.brand.withValues(alpha: 0.12),
          labelBehavior: NavigationDestinationLabelBehavior.alwaysShow,
          iconTheme: WidgetStateProperty.resolveWith(
            (states) => IconThemeData(
              color: states.contains(WidgetState.selected)
                  ? tokens.brand
                  : tokens.mute,
            ),
          ),
          labelTextStyle: WidgetStateProperty.resolveWith(
            (states) => textTheme.labelMedium?.copyWith(
              color: colour().resolve(states),
              fontWeight: states.contains(WidgetState.selected)
                  ? FontWeight.w700
                  : FontWeight.w500,
            ),
          ),
        ),
        child: NavigationBar(
          selectedIndex: navigationShell.currentIndex,
          onDestinationSelected: (index) => navigationShell.goBranch(
            index,
            // Tapping the open tab again returns it to its first page.
            initialLocation: index == navigationShell.currentIndex,
          ),
          destinations: const [
            NavigationDestination(
              icon: Icon(Icons.storefront_outlined),
              selectedIcon: Icon(Icons.storefront),
              label: Strings.tabMarket,
            ),
            NavigationDestination(
              icon: Icon(Icons.person_outline),
              selectedIcon: Icon(Icons.person),
              label: Strings.tabAccount,
            ),
          ],
        ),
      ),
    );
  }
}
