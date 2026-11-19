import 'package:flutter/services.dart';
import 'package:flutter_animate/flutter_animate.dart';
import 'package:material_ui/material_ui.dart';

import '../../../ui/motion.dart';

/// Six boxes for the code. One hidden text field underneath takes the typing,
/// a paste, and the SMS autofill; the boxes just draw what it holds. Each digit
/// pops in, the active box glows, and [shakeKey] changing shakes the row (a wrong code).
class OtpBoxes extends StatefulWidget {
  const OtpBoxes({
    super.key,
    required this.onCompleted,
    this.length = 6,
    this.shakeKey = 0,
    this.enabled = true,
  });

  final void Function(String code) onCompleted;
  final int length;
  final int shakeKey;
  final bool enabled;

  @override
  State<OtpBoxes> createState() => _OtpBoxesState();
}

class _OtpBoxesState extends State<OtpBoxes> {
  final _controller = TextEditingController();
  final _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _controller.addListener(() => setState(() {}));
    _focus.addListener(() => setState(() {}));
  }

  @override
  void didUpdateWidget(OtpBoxes old) {
    super.didUpdateWidget(old);
    if (widget.shakeKey != old.shakeKey) {
      // A wrong code: clear it and keep the keyboard up for the next try.
      _controller.clear();
      _focus.requestFocus();
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _changed(String v) {
    if (v.length == widget.length) widget.onCompleted(v);
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = _controller.text;
    final reduce = Motion.reduced(context);
    final row = Semantics(
      label:
          'Verification code, ${text.length} of ${widget.length} digits entered',
      textField: true,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => _focus.requestFocus(),
        child: Stack(
          children: [
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                for (var i = 0; i < widget.length; i++)
                  _Box(
                    char: i < text.length ? text[i] : '',
                    active:
                        _focus.hasFocus &&
                        i ==
                            (text.length < widget.length
                                ? text.length
                                : widget.length - 1),
                    scheme: scheme,
                  ),
              ],
            ),
            // The real field: invisible, but it is what the keyboard and autofill talk to.
            Positioned.fill(
              child: Opacity(
                opacity: 0,
                child: TextField(
                  controller: _controller,
                  focusNode: _focus,
                  enabled: widget.enabled,
                  autofocus: true,
                  keyboardType: TextInputType.number,
                  autofillHints: const [AutofillHints.oneTimeCode],
                  inputFormatters: [
                    FilteringTextInputFormatter.digitsOnly,
                    LengthLimitingTextInputFormatter(widget.length),
                  ],
                  onChanged: _changed,
                  enableInteractiveSelection: false,
                  showCursor: false,
                ),
              ),
            ),
          ],
        ),
      ),
    );
    if (reduce || widget.shakeKey == 0) return row;
    return row
        .animate(key: ValueKey(widget.shakeKey))
        .shakeX(hz: 5, amount: 8, duration: 420.ms);
  }
}

class _Box extends StatelessWidget {
  const _Box({required this.char, required this.active, required this.scheme});
  final String char;
  final bool active;
  final ColorScheme scheme;

  @override
  Widget build(BuildContext context) {
    return AnimatedContainer(
      duration: Motion.fast,
      curve: Motion.out,
      width: 46,
      height: 58,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: scheme.surface,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(
          color: active ? scheme.primary : scheme.outlineVariant,
          width: active ? 2 : 1,
        ),
        boxShadow: active
            ? [
                BoxShadow(
                  color: scheme.primary.withValues(alpha: 0.25),
                  blurRadius: 12,
                ),
              ]
            : null,
      ),
      child: AnimatedSwitcher(
        duration: Motion.fast,
        transitionBuilder: (c, a) => ScaleTransition(
          scale: CurvedAnimation(parent: a, curve: Curves.easeOutBack),
          child: c,
        ),
        child: Text(
          char,
          key: ValueKey(char),
          style: Theme.of(context).textTheme.headlineSmall
              ?.copyWith(fontWeight: FontWeight.w700),
        ),
      ),
    );
  }
}
