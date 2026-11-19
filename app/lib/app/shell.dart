import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../ui/app_icon.dart';

class AppTab {
  const AppTab({required this.path, required this.label, required this.icon});
  final String path;
  final String label;
  final IconSpec icon;
}

/// The bottom tabs. Screens add themselves here as they are built; with one tab
/// there is nothing to switch between, so the bar is hidden.
const kTabs = <AppTab>[
  AppTab(path: '/', label: 'Account', icon: AppIcons.account),
];

/// The frame around the signed-in screens: a bottom bar whose selected icon
/// plays its animation when it is chosen.
class AppShell extends StatelessWidget {
  const AppShell({super.key, required this.location, required this.child});
  final String location;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final index = kTabs.indexWhere(
      (t) => t.path == '/' ? location == '/' : location.startsWith(t.path),
    );
    return Scaffold(
      body: child,
      bottomNavigationBar: kTabs.length < 2
          ? null
          : NavigationBar(
              selectedIndex: index < 0 ? 0 : index,
              onDestinationSelected: (i) => context.go(kTabs[i].path),
              destinations: [
                for (var i = 0; i < kTabs.length; i++)
                  NavigationDestination(
                    icon: AppIcon(kTabs[i].icon),
                    selectedIcon: AppIcon(kTabs[i].icon, active: true),
                    label: kTabs[i].label,
                  ),
              ],
            ),
    );
  }
}
