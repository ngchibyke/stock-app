import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import 'app/router.dart';
import 'app/theme_mode.dart';
import 'features/auth/auth_controller.dart';
import 'ui/theme.dart';

void main() {
  runApp(const ProviderScope(child: MarketApp()));
}

class MarketApp extends ConsumerStatefulWidget {
  const MarketApp({super.key});

  @override
  ConsumerState<MarketApp> createState() => _MarketAppState();
}

class _MarketAppState extends ConsumerState<MarketApp> {
  @override
  void initState() {
    super.initState();
    // Check a stored session once, after the first frame.
    Future.microtask(() => ref.read(authControllerProvider.notifier).start());
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp.router(
      title: 'Market',
      debugShowCheckedModeBanner: false,
      theme: AppTheme.light(),
      darkTheme: AppTheme.dark(),
      themeMode: ref.watch(themeModeProvider),
      routerConfig: ref.watch(routerProvider),
    );
  }
}
