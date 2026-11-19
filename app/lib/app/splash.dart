import 'package:flutter_animate/flutter_animate.dart';
import 'package:material_ui/material_ui.dart';

import '../ui/app_icon.dart';

/// Shown for the moment it takes to check a stored session.
class SplashScreen extends StatelessWidget {
  const SplashScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      body: Center(
        child:
            Container(
                  width: 84,
                  height: 84,
                  decoration: BoxDecoration(
                    gradient: LinearGradient(
                      colors: [scheme.primary, scheme.tertiary],
                    ),
                    borderRadius: BorderRadius.circular(26),
                  ),
                  child: const Center(
                    child: AppIcon(
                      AppIcons.store,
                      size: 44,
                      color: Colors.white,
                    ),
                  ),
                )
                .animate(onPlay: (c) => c.repeat(reverse: true))
                .scale(
                  begin: const Offset(0.92, 0.92),
                  end: const Offset(1.06, 1.06),
                  duration: 900.ms,
                  curve: Curves.easeInOut,
                ),
      ),
    );
  }
}
