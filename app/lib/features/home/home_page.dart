import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/paths.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';

/// The Market tab until the market itself is built: it says so, and points to
/// the one thing that is ready, the address book.
class HomePage extends StatelessWidget {
  const HomePage({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SafeArea(
      bottom: false,
      child: ListView(
        padding: const EdgeInsets.all(Tokens.s5),
        children: [
          const SizedBox(height: Tokens.s5),
          Semantics(
            header: true,
            child: Text(Strings.home, style: theme.textTheme.titleLarge),
          ),
          const SizedBox(height: Tokens.s3),
          Text(Strings.marketOpeningSoon, style: theme.textTheme.titleMedium),
          const SizedBox(height: Tokens.s2),
          Text(
            Strings.marketOpeningBody,
            style: theme.textTheme.bodyMedium?.copyWith(
              color: context.tokens.mute,
            ),
          ),
          const SizedBox(height: Tokens.s5),
          OutlinedButton(
            style: OutlinedButton.styleFrom(
              minimumSize: const Size(Tokens.minTap, Tokens.minTap),
            ),
            onPressed: () => context.push(addressesPath),
            child: const Text(Strings.addresses),
          ),
        ],
      ),
    );
  }
}
