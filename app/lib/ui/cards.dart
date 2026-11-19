import 'package:material_ui/material_ui.dart';

import 'motion.dart';

/// A card that lifts and squashes a little when pressed.
class PressableCard extends StatefulWidget {
  const PressableCard({
    super.key,
    required this.child,
    this.onTap,
    this.padding = const EdgeInsets.all(16),
    this.semanticLabel,
  });
  final Widget child;
  final VoidCallback? onTap;
  final EdgeInsetsGeometry padding;
  final String? semanticLabel;

  @override
  State<PressableCard> createState() => _PressableCardState();
}

class _PressableCardState extends State<PressableCard> {
  bool _down = false;

  @override
  Widget build(BuildContext context) {
    final reduce = Motion.reduced(context);
    final card = Card(
      margin: EdgeInsets.zero,
      child: Padding(padding: widget.padding, child: widget.child),
    );
    if (widget.onTap == null) return card;
    return Semantics(
      button: true,
      label: widget.semanticLabel,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapDown: (_) => setState(() => _down = true),
        onTapCancel: () => setState(() => _down = false),
        onTapUp: (_) => setState(() => _down = false),
        onTap: widget.onTap,
        child: AnimatedScale(
          scale: _down && !reduce ? 0.975 : 1,
          duration: _down ? Motion.instant : Motion.fast,
          curve: _down ? Curves.easeOut : Curves.easeOutBack,
          child: card,
        ),
      ),
    );
  }
}
