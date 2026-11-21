import 'package:material_ui/material_ui.dart';

import '../strings.dart';
import '../theme/tokens.dart';

/// The entry-screen layout: a brand gradient header (wordmark, title,
/// subtitle) with the `bg` sheet rising from the bottom. The whole screen
/// scrolls, and the Scaffold lifts it above the keyboard, so a focused field
/// is never hidden. Used by sign-in, sign-up, code, forgot and reset screens.
class BrandHeaderScaffold extends StatelessWidget {
  const BrandHeaderScaffold({
    super.key,
    required this.title,
    this.subtitle,
    required this.child,
    this.showBack = false,
  });

  final String title;
  final String? subtitle;
  final Widget child;
  final bool showBack;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = context.tokens;

    return Scaffold(
      backgroundColor: Tokens.headerFrom,
      resizeToAvoidBottomInset: true,
      body: CustomScrollView(
        slivers: [
          SliverFillRemaining(
            hasScrollBody: false,
            // The gradient sits behind the whole column, so the rounded top
            // corners of the sheet show the brand colour, not a seam.
            child: DecoratedBox(
              decoration: const BoxDecoration(
                gradient: LinearGradient(
                  begin: Alignment.topLeft,
                  end: Alignment.bottomRight,
                  colors: [Tokens.headerFrom, Tokens.headerTo],
                ),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SafeArea(
                    bottom: false,
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(
                        Tokens.s5,
                        Tokens.s2,
                        Tokens.s5,
                        Tokens.s5,
                      ),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          if (showBack)
                            Semantics(
                              button: true,
                              label: Strings.back,
                              excludeSemantics: true,
                              onTap: () => Navigator.maybePop(context),
                              child: IconButton(
                                tooltip: Strings.back,
                                onPressed: () => Navigator.maybePop(context),
                                icon: const Icon(
                                  Icons.arrow_back,
                                  color: Tokens.onHeader,
                                ),
                              ),
                            )
                          else
                            const SizedBox(height: Tokens.s4),
                          const SizedBox(height: Tokens.s2),
                          Text(
                            Strings.wordmark,
                            style: theme.textTheme.titleMedium?.copyWith(
                              color: colors.gold,
                              letterSpacing: 3,
                              fontWeight: FontWeight.w800,
                            ),
                          ),
                          const SizedBox(height: Tokens.s3),
                          Semantics(
                            header: true,
                            child: Text(
                              title,
                              style: theme.textTheme.displayLarge?.copyWith(
                                color: Tokens.onHeader,
                              ),
                            ),
                          ),
                          if (subtitle != null) ...[
                            const SizedBox(height: Tokens.s2),
                            Text(
                              subtitle!,
                              style: theme.textTheme.bodyLarge?.copyWith(
                                color: const Color(0xFFD7DAF0),
                              ),
                            ),
                          ],
                        ],
                      ),
                    ),
                  ),
                  Expanded(
                    child: Container(
                      decoration: BoxDecoration(
                        color: colors.bg,
                        borderRadius: const BorderRadius.vertical(
                          top: Radius.circular(Tokens.sheetRadius),
                        ),
                      ),
                      padding: const EdgeInsets.fromLTRB(
                        Tokens.s5,
                        Tokens.s5,
                        Tokens.s5,
                        Tokens.s6,
                      ),
                      child: child,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
