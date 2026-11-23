import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../theme/tokens.dart';

/// Anything tappable that is not a stock Material button.
///
/// It answers the finger at once: a slight press-in (0.97) and a faint tint
/// over 90 ms, both dropped with reduced motion. [haptic] adds a light tap
/// for primary actions. Disabled when [onTap] is null.
class Pressable extends StatefulWidget {
  const Pressable({
    super.key,
    required this.onTap,
    required this.child,
    this.semanticLabel,
    this.haptic = false,
    this.radius = Tokens.radius,
  });

  final VoidCallback? onTap;
  final Widget child;

  /// Read instead of the child's text, for icon-only controls.
  final String? semanticLabel;
  final bool haptic;
  final double radius;

  @override
  State<Pressable> createState() => _PressableState();
}

class _PressableState extends State<Pressable> {
  bool _down = false;

  void _setDown(bool down) {
    if (_down != down && mounted) setState(() => _down = down);
  }

  void _tap() {
    if (widget.haptic) HapticFeedback.lightImpact();
    widget.onTap?.call();
  }

  @override
  Widget build(BuildContext context) {
    final enabled = widget.onTap != null;
    final still = MediaQuery.maybeDisableAnimationsOf(context) ?? false;
    final pressed = _down && enabled;
    final duration = still ? Duration.zero : const Duration(milliseconds: 90);
    return MergeSemantics(
      child: Semantics(
        button: true,
        enabled: enabled,
        label: widget.semanticLabel,
        excludeSemantics: widget.semanticLabel != null,
        onTap: enabled ? _tap : null,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          excludeFromSemantics: true,
          onTapDown: enabled ? (_) => _setDown(true) : null,
          onTapUp: enabled ? (_) => _setDown(false) : null,
          onTapCancel: () => _setDown(false),
          onTap: enabled ? _tap : null,
          child: AnimatedScale(
            scale: pressed && !still ? 0.97 : 1,
            duration: duration,
            curve: Curves.easeOut,
            child: AnimatedContainer(
              duration: duration,
              foregroundDecoration: BoxDecoration(
                color: context.tokens.ink.withValues(alpha: pressed ? 0.06 : 0),
                borderRadius: BorderRadius.circular(widget.radius),
              ),
              child: widget.child,
            ),
          ),
        ),
      ),
    );
  }
}
