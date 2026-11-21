import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../strings.dart';
import '../theme/tokens.dart';

/// A row of digit boxes backed by one hidden text field, so paste and the
/// system one-time-code autofill work. Accepts only digits, strips pasted
/// spaces and ignores input beyond [length].
class CodeBoxes extends StatefulWidget {
  const CodeBoxes({
    super.key,
    this.length = 6,
    required this.onChanged,
    this.onCompleted,
    this.errorText,
  });

  final int length;
  final ValueChanged<String> onChanged;
  final ValueChanged<String>? onCompleted;

  /// Shown as text under the boxes; the boxes also take the error border, but
  /// colour is never the only signal.
  final String? errorText;

  static const double boxWidth = 46;
  static const double boxHeight = 54;
  static const double _gap = Tokens.s2;

  @override
  State<CodeBoxes> createState() => _CodeBoxesState();
}

class _CodeBoxesState extends State<CodeBoxes> {
  final TextEditingController _controller = TextEditingController();
  final FocusNode _focus = FocusNode();
  String _last = '';

  @override
  void dispose() {
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _changed(String value) {
    final wasComplete = _last.length == widget.length;
    _last = value;
    widget.onChanged(value);
    if (value.length == widget.length && !wasComplete) {
      widget.onCompleted?.call(value);
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.tokens;
    final theme = Theme.of(context);
    final hasError = widget.errorText != null;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Semantics(
          label: Strings.codeFieldLabel(widget.length),
          textField: true,
          child: ExcludeSemantics(
            excluding: false,
            child: LayoutBuilder(
              builder: (context, constraints) {
                // Boxes shrink on narrow screens rather than overflow.
                final gaps = CodeBoxes._gap * (widget.length - 1);
                final fit = (constraints.maxWidth - gaps) / widget.length;
                final width = fit.isFinite
                    ? fit.clamp(0.0, CodeBoxes.boxWidth).toDouble()
                    : CodeBoxes.boxWidth;
                return SizedBox(
                  height: CodeBoxes.boxHeight,
                  child: Stack(
                    children: [
                      ExcludeSemantics(
                        child: ListenableBuilder(
                          listenable: Listenable.merge([_controller, _focus]),
                          builder: (context, _) {
                            final text = _controller.text;
                            final active = _focus.hasFocus
                                ? text.length.clamp(0, widget.length - 1)
                                : -1;
                            return Row(
                              mainAxisAlignment: MainAxisAlignment.center,
                              children: [
                                for (var i = 0; i < widget.length; i++) ...[
                                  if (i > 0)
                                    const SizedBox(width: CodeBoxes._gap),
                                  Container(
                                    key: ValueKey('code-box-$i'),
                                    width: width,
                                    height: CodeBoxes.boxHeight,
                                    alignment: Alignment.center,
                                    decoration: BoxDecoration(
                                      color: colors.card,
                                      borderRadius: BorderRadius.circular(
                                        Tokens.radius,
                                      ),
                                      border: Border.all(
                                        color: hasError
                                            ? colors.bad
                                            : i == active
                                            ? colors.accent
                                            : colors.line,
                                        width: i == active || hasError
                                            ? 2
                                            : 1.5,
                                      ),
                                    ),
                                    child: FittedBox(
                                      fit: BoxFit.scaleDown,
                                      child: Text(
                                        i < text.length ? text[i] : '',
                                        style: theme.textTheme.titleLarge,
                                      ),
                                    ),
                                  ),
                                ],
                              ],
                            );
                          },
                        ),
                      ),
                      // The one real field: it covers the row, so a tap on
                      // any box focuses it. Its text and cursor are hidden.
                      Positioned.fill(
                        child: TextField(
                          controller: _controller,
                          focusNode: _focus,
                          autofillHints: const [AutofillHints.oneTimeCode],
                          keyboardType: TextInputType.number,
                          textInputAction: TextInputAction.done,
                          enableInteractiveSelection: false,
                          showCursor: false,
                          cursorColor: const Color(0x00000000),
                          style: const TextStyle(
                            color: Color(0x00000000),
                            fontSize: 1,
                          ),
                          decoration: const InputDecoration(
                            border: InputBorder.none,
                            enabledBorder: InputBorder.none,
                            focusedBorder: InputBorder.none,
                            filled: false,
                            counterText: '',
                            contentPadding: EdgeInsets.zero,
                          ),
                          inputFormatters: [
                            // Pasted spaces and anything that is not a digit
                            // go; then the length cap ignores the overflow.
                            FilteringTextInputFormatter.digitsOnly,
                            LengthLimitingTextInputFormatter(widget.length),
                          ],
                          onChanged: _changed,
                        ),
                      ),
                    ],
                  ),
                );
              },
            ),
          ),
        ),
        if (hasError) ...[
          const SizedBox(height: Tokens.s2),
          Semantics(
            liveRegion: true,
            child: Text(
              widget.errorText!,
              style: theme.textTheme.bodySmall?.copyWith(
                color: colors.bad,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        ],
      ],
    );
  }
}
