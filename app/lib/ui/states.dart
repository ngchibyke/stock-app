import 'package:material_ui/material_ui.dart';
import 'package:flutter_animate/flutter_animate.dart';

import '../core/api/api_error.dart';
import 'app_icon.dart';
import 'buttons.dart';

/// Placeholder lines while something loads.
class Skeleton extends StatelessWidget {
  const Skeleton({super.key, this.lines = 3});
  final int lines;

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme.surfaceContainerHighest;
    return ExcludeSemantics(
      child:
          Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  for (var i = 0; i < lines; i++)
                    Padding(
                      padding: const EdgeInsets.symmetric(vertical: 6),
                      child: FractionallySizedBox(
                        widthFactor: 0.9 - i * 0.14,
                        child: Container(
                          height: 14,
                          decoration: BoxDecoration(
                            color: c,
                            borderRadius: BorderRadius.circular(7),
                          ),
                        ),
                      ),
                    ),
                ],
              )
              .animate(onPlay: (c) => c.repeat())
              .shimmer(duration: 1200.ms, color: Colors.white24),
    );
  }
}

/// Nothing here yet: a bobbing icon and a sentence.
class EmptyState extends StatelessWidget {
  const EmptyState({
    super.key,
    required this.icon,
    required this.title,
    this.hint,
  });
  final IconSpec icon;
  final String title;
  final String? hint;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
                  width: 72,
                  height: 72,
                  decoration: BoxDecoration(
                    color: scheme.primaryContainer,
                    borderRadius: BorderRadius.circular(22),
                  ),
                  child: Icon(
                    icon.data,
                    size: 34,
                    color: scheme.onPrimaryContainer,
                  ),
                )
                .animate(onPlay: (c) => c.repeat(reverse: true))
                .moveY(
                  begin: 0,
                  end: -6,
                  duration: 1600.ms,
                  curve: Curves.easeInOut,
                ),
            const SizedBox(height: 16),
            Text(
              title,
              style: Theme.of(context).textTheme.titleMedium
                  ?.copyWith(fontWeight: FontWeight.w700),
              textAlign: TextAlign.center,
            ),
            if (hint != null) ...[
              const SizedBox(height: 6),
              Text(
                hint!,
                style: TextStyle(color: scheme.onSurfaceVariant),
                textAlign: TextAlign.center,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// The server's own message, with a retry.
class ErrorState extends StatelessWidget {
  const ErrorState({super.key, required this.error, this.onRetry});
  final Object error;
  final VoidCallback? onRetry;

  @override
  Widget build(BuildContext context) {
    final message = error is ApiError
        ? (error as ApiError).message
        : 'Something went wrong. Try again.';
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              AppIcons.warn.data,
              size: 40,
              color: Theme.of(context).colorScheme.error,
            ),
            const SizedBox(height: 12),
            Text(message, textAlign: TextAlign.center),
            if (onRetry != null) ...[
              const SizedBox(height: 16),
              AppButton(
                label: 'Try again',
                icon: AppIcons.refresh,
                style: AppButtonStyle.secondary,
                onPressed: onRetry,
              ),
            ],
          ],
        ),
      ),
    );
  }
}
