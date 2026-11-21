import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_riverpod/misc.dart' show Override;
import 'package:material_ui/material_ui.dart';

import 'core/config/app_config.dart';
import 'core/config/app_gate.dart';
import 'core/session/session_controller.dart';
import 'core/strings.dart';
import 'core/theme/theme.dart';
import 'core/theme/tokens.dart';
import 'router.dart';

/// No automatic retry for any provider: a failure is shown to the person (with
/// the screen's own retry), not hidden behind silent retries.
Duration? noRetry(int retryCount, Object error) => null;

/// The root of the app: the Riverpod scope with its wiring, and [MarketApp].
///
/// [overrides] are added to the session hooks that the API client needs
/// (without those it throws on purpose). `main.dart` adds the production-only
/// providers; tests add fakes.
class MarketRoot extends StatelessWidget {
  const MarketRoot({super.key, this.overrides = const []});

  final List<Override> overrides;

  @override
  Widget build(BuildContext context) {
    return ProviderScope(
      retry: noRetry,
      overrides: [...sessionOverrides, ...overrides],
      child: const MarketApp(),
    );
  }
}

class MarketApp extends ConsumerWidget {
  const MarketApp({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Read once here so the config is refetched whenever the app returns to the
    // foreground.
    ref.watch(appForegroundRefresherProvider);
    final gate = ref.watch(gateProvider);
    final router = ref.watch(routerProvider);
    final config = ref.watch(appConfigProvider).value;

    return MaterialApp.router(
      title: Strings.wordmark,
      theme: lightTheme(),
      darkTheme: darkTheme(),
      routerConfig: router,
      builder: (context, child) {
        if (gate == Gate.open) return child ?? const SizedBox.shrink();
        // Nothing of the app is built behind the notice.
        return GateNotice(
          gate: gate,
          maintenanceMessage: config?.maintenanceMessage,
        );
      },
    );
  }
}

/// A full-screen notice for a gate that is not open: the app is too old, or
/// the service is in maintenance. It is only text; there is no way into the app
/// from here, and it goes away by itself when the gate opens.
class GateNotice extends StatelessWidget {
  const GateNotice({super.key, required this.gate, this.maintenanceMessage});

  final Gate gate;
  final String? maintenanceMessage;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final message = maintenanceMessage?.trim();
    final (title, body) = switch (gate) {
      Gate.updateRequired => (
        Strings.updateRequiredTitle,
        Strings.updateRequiredBody,
      ),
      Gate.maintenance || Gate.open => (
        Strings.maintenanceTitle,
        message == null || message.isEmpty
            ? Strings.maintenanceFallback
            : message,
      ),
    };
    return Scaffold(
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(Tokens.s5),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  Strings.wordmark,
                  style: theme.textTheme.titleMedium?.copyWith(
                    color: context.tokens.gold,
                    letterSpacing: 3,
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: Tokens.s4),
                Semantics(
                  header: true,
                  child: Text(
                    title,
                    textAlign: TextAlign.center,
                    style: theme.textTheme.titleLarge,
                  ),
                ),
                const SizedBox(height: Tokens.s3),
                Semantics(
                  liveRegion: true,
                  child: Text(
                    body,
                    textAlign: TextAlign.center,
                    style: theme.textTheme.bodyLarge,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
