import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../strings.dart';
import '../theme/tokens.dart';

/// A grey placeholder block shown while content loads. It pulses gently, and
/// stands still when the system asks for reduced motion.
///
/// It is hidden from screen readers; the screen that shows a group of
/// skeletons should announce [Strings.loading] once.
class Skeleton extends StatefulWidget {
  const Skeleton({super.key, this.height, this.width});

  final double? height;
  final double? width;

  @override
  State<Skeleton> createState() => _SkeletonState();
}

class _SkeletonState extends State<Skeleton>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1100),
  );
  late final Animation<double> _opacity = Tween<double>(
    begin: 0.45,
    end: 1,
  ).animate(CurvedAnimation(parent: _controller, curve: Curves.easeInOut));

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _controller.stop();
    } else if (!_controller.isAnimating) {
      _controller.repeat(reverse: true);
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final block = Container(
      height: widget.height ?? Tokens.s4,
      width: widget.width,
      decoration: BoxDecoration(
        color: context.tokens.line,
        borderRadius: BorderRadius.circular(Tokens.s2),
      ),
    );
    return ExcludeSemantics(
      child: MediaQuery.disableAnimationsOf(context)
          ? block
          : FadeTransition(opacity: _opacity, child: block),
    );
  }
}

/// A centred message for an empty list, with an optional action.
class EmptyState extends StatelessWidget {
  const EmptyState({
    super.key,
    required this.title,
    this.body,
    this.actionLabel,
    this.onAction,
  });

  final String title;
  final String? body;
  final String? actionLabel;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final hasAction = actionLabel != null && onAction != null;
    return _CentredScroll(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Semantics(
            header: true,
            child: Text(
              title,
              textAlign: TextAlign.center,
              style: theme.textTheme.titleLarge,
            ),
          ),
          if (body != null) ...[
            const SizedBox(height: Tokens.s2),
            Text(
              body!,
              textAlign: TextAlign.center,
              style: theme.textTheme.bodyMedium?.copyWith(
                color: context.tokens.mute,
              ),
            ),
          ],
          if (hasAction) ...[
            const SizedBox(height: Tokens.s5),
            FilledButton(onPressed: onAction, child: Text(actionLabel!)),
          ],
        ],
      ),
    );
  }
}

/// A centred error message with an optional retry. The message is always
/// text, never colour alone.
class ErrorState extends StatelessWidget {
  const ErrorState({super.key, required this.message, this.onRetry});

  final String message;
  final VoidCallback? onRetry;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return _CentredScroll(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ExcludeSemantics(
            child: Icon(
              Icons.error_outline,
              size: 40,
              color: context.tokens.mute,
            ),
          ),
          const SizedBox(height: Tokens.s3),
          Semantics(
            liveRegion: true,
            child: Text(
              message,
              textAlign: TextAlign.center,
              style: theme.textTheme.bodyLarge,
            ),
          ),
          if (onRetry != null) ...[
            const SizedBox(height: Tokens.s5),
            Semantics(
              button: true,
              label: Strings.retry,
              excludeSemantics: true,
              onTap: onRetry,
              child: OutlinedButton(
                onPressed: onRetry,
                child: const Text(Strings.retry),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

/// A full-width primary button that shows progress and ignores taps while
/// [loading].
class LoadingButton extends StatelessWidget {
  const LoadingButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.loading = false,
  });

  final String label;
  final VoidCallback? onPressed;
  final bool loading;

  @override
  Widget build(BuildContext context) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final enabled = onPressed != null && !loading;

    final Widget content;
    if (!loading) {
      content = Text(label, textAlign: TextAlign.center);
    } else if (reduceMotion) {
      content = const Text(Strings.loading, textAlign: TextAlign.center);
    } else {
      content = Stack(
        alignment: Alignment.center,
        children: [
          // Keeps the button the same size as its idle state.
          Opacity(opacity: 0, child: Text(label, textAlign: TextAlign.center)),
          const SizedBox(
            width: 24,
            height: 24,
            child: CircularProgressIndicator(
              strokeWidth: 3,
              color: Color(0xFFFFFFFF),
            ),
          ),
        ],
      );
    }

    return Semantics(
      button: true,
      enabled: enabled,
      label: loading ? '$label, ${Strings.loading}' : label,
      excludeSemantics: true,
      onTap: enabled ? onPressed : null,
      child: FilledButton(
        // A no-op while loading keeps the button looking active but inert.
        onPressed: onPressed == null ? null : (loading ? () {} : onPressed),
        child: content,
      ),
    );
  }
}

/// A text field with the app's look: a floating label, a `line` border that
/// turns accent on focus and shows [errorText] as text on error.
class AppTextField extends StatelessWidget {
  const AppTextField({
    super.key,
    required this.label,
    this.controller,
    this.errorText,
    this.obscure = false,
    this.keyboardType,
    this.action,
    this.maxLength,
    this.formatters,
    this.onChanged,
    this.onSubmitted,
  });

  final String label;
  final TextEditingController? controller;
  final String? errorText;
  final bool obscure;
  final TextInputType? keyboardType;
  final TextInputAction? action;
  final int? maxLength;
  final List<TextInputFormatter>? formatters;
  final ValueChanged<String>? onChanged;
  final ValueChanged<String>? onSubmitted;

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: controller,
      obscureText: obscure,
      keyboardType: keyboardType,
      textInputAction: action,
      maxLength: maxLength,
      // The length is enforced, but no "0/10" counter is shown.
      buildCounter: (
        _, {
        required currentLength,
        required isFocused,
        maxLength,
      }) => null,
      inputFormatters: formatters,
      onChanged: onChanged,
      onSubmitted: onSubmitted,
      decoration: InputDecoration(labelText: label, errorText: errorText),
    );
  }
}

/// Centres its child, and scrolls when the content is taller than the space
/// (large text, small screens).
class _CentredScroll extends StatelessWidget {
  const _CentredScroll({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        // In a column with no height limit there is nothing to centre in or
        // scroll; just pad.
        if (!constraints.hasBoundedHeight) {
          return Padding(
            padding: const EdgeInsets.all(Tokens.s5),
            child: Center(child: child),
          );
        }
        return SingleChildScrollView(
          padding: const EdgeInsets.all(Tokens.s5),
          child: ConstrainedBox(
            constraints: BoxConstraints(
              minHeight: (constraints.maxHeight - 2 * Tokens.s5).clamp(
                0,
                double.infinity,
              ),
            ),
            child: Center(child: child),
          ),
        );
      },
    );
  }
}
