import 'package:material_ui/material_ui.dart';

import 'app_icon.dart';
import 'motion.dart';

enum AppButtonStyle { primary, secondary, ghost, danger }

/// A button that squashes under the thumb, shows a spinner while it works, and
/// a check when it is done. While loading it ignores taps.
class AppButton extends StatefulWidget {
  const AppButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.icon,
    this.style = AppButtonStyle.primary,
    this.loading = false,
    this.done = false,
    this.expand = false,
  });

  final String label;
  final VoidCallback? onPressed;
  final IconSpec? icon;
  final AppButtonStyle style;
  final bool loading;
  final bool done;
  final bool expand;

  @override
  State<AppButton> createState() => _AppButtonState();
}

class _AppButtonState extends State<AppButton> {
  bool _down = false;
  int _presses = 0;

  bool get _enabled => widget.onPressed != null && !widget.loading;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final (bg, fg, border) = switch (widget.style) {
      AppButtonStyle.primary => (
        scheme.primary,
        scheme.onPrimary,
        Colors.transparent,
      ),
      AppButtonStyle.secondary => (
        scheme.surface,
        scheme.onSurface,
        scheme.outlineVariant,
      ),
      AppButtonStyle.ghost => (
        Colors.transparent,
        scheme.primary,
        Colors.transparent,
      ),
      AppButtonStyle.danger => (
        scheme.error,
        scheme.onError,
        Colors.transparent,
      ),
    };
    final reduce = Motion.reduced(context);
    final lead = widget.loading
        ? SizedBox(
            key: const ValueKey('spinner'),
            width: 18,
            height: 18,
            child: CircularProgressIndicator(strokeWidth: 2.4, color: fg),
          )
        : widget.done
        ? AppIcon(
            AppIcons.check,
            key: const ValueKey('done'),
            size: 20,
            color: fg,
            trigger: 1,
          )
        : widget.icon != null
        ? AppIcon(
            widget.icon!,
            key: const ValueKey('icon'),
            size: 20,
            color: fg,
            trigger: _presses,
          )
        : null;

    return Semantics(
      button: true,
      enabled: _enabled,
      label: widget.label,
      excludeSemantics: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapDown: _enabled ? (_) => setState(() => _down = true) : null,
        onTapCancel: _enabled ? () => setState(() => _down = false) : null,
        onTapUp: _enabled ? (_) => setState(() => _down = false) : null,
        onTap: _enabled
            ? () {
                setState(() => _presses++);
                widget.onPressed!();
              }
            : null,
        child: AnimatedScale(
          scale: _down && !reduce ? Motion.pressScale : 1,
          duration: _down ? Motion.instant : Motion.fast,
          curve: _down ? Curves.easeOut : Curves.easeOutBack,
          child: AnimatedOpacity(
            opacity: widget.onPressed == null && !widget.loading ? 0.5 : 1,
            duration: Motion.fast,
            child: Container(
              constraints: const BoxConstraints(minHeight: 52, minWidth: 52),
              width: widget.expand ? double.infinity : null,
              padding: const EdgeInsets.symmetric(horizontal: 20),
              decoration: BoxDecoration(
                color: bg,
                borderRadius: BorderRadius.circular(16),
                border: Border.all(color: border),
                boxShadow: widget.style == AppButtonStyle.primary && _enabled
                    ? [
                        BoxShadow(
                          color: scheme.primary.withValues(alpha: 0.28),
                          blurRadius: 14,
                          offset: const Offset(0, 6),
                        ),
                      ]
                    : null,
              ),
              child: Row(
                mainAxisSize: widget.expand
                    ? MainAxisSize.max
                    : MainAxisSize.min,
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  AnimatedSize(
                    duration: Motion.fast,
                    curve: Motion.out,
                    child: AnimatedSwitcher(
                      duration: Motion.fast,
                      transitionBuilder: (c, a) => ScaleTransition(
                        scale: a,
                        child: FadeTransition(opacity: a, child: c),
                      ),
                      child: lead == null
                          ? const SizedBox.shrink(key: ValueKey('none'))
                          : Padding(
                              padding: const EdgeInsets.only(right: 10),
                              child: lead,
                            ),
                    ),
                  ),
                  Flexible(
                    child: Text(
                      widget.label,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        color: fg,
                        fontWeight: FontWeight.w700,
                        fontSize: 15,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A round icon-only button with a 48 dp target and a label for screen readers.
class AppIconButton extends StatelessWidget {
  const AppIconButton({
    super.key,
    required this.icon,
    required this.label,
    required this.onPressed,
    this.color,
  });
  final IconSpec icon;
  final String label;
  final VoidCallback? onPressed;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      button: true,
      label: label,
      excludeSemantics: true,
      child: InkResponse(
        onTap: onPressed,
        radius: 26,
        child: SizedBox(
          width: 48,
          height: 48,
          child: Center(child: AppIcon(icon, color: color)),
        ),
      ),
    );
  }
}
