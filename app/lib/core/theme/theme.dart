import 'package:material_ui/material_ui.dart';

import 'tokens.dart';

/// Light theme; the app follows the system setting between this and
/// [darkTheme].
ThemeData lightTheme() => _build(Brightness.light, Tokens.light);

ThemeData darkTheme() => _build(Brightness.dark, Tokens.dark);

ThemeData _build(Brightness brightness, TokenColors c) {
  final scheme = ColorScheme(
    brightness: brightness,
    primary: c.accent,
    onPrimary: const Color(0xFFFFFFFF),
    secondary: c.gold,
    onSecondary: Tokens.light.brand,
    error: c.bad,
    onError: const Color(0xFFFFFFFF),
    surface: c.card,
    onSurface: c.ink,
    outline: c.line,
    outlineVariant: c.line,
  );

  // Platform system font (San Francisco / Roboto): no family is set. Heavy
  // weights for headlines and big numerals.
  final text = const TextTheme(
    displayLarge: TextStyle(
      fontSize: Tokens.displaySize,
      fontWeight: FontWeight.w800,
      height: 1.15,
    ),
    displayMedium: TextStyle(
      fontSize: Tokens.displaySize,
      fontWeight: FontWeight.w800,
      height: 1.15,
    ),
    headlineMedium: TextStyle(
      fontSize: Tokens.titleSize,
      fontWeight: FontWeight.w800,
      height: 1.2,
    ),
    titleLarge: TextStyle(
      fontSize: Tokens.titleSize,
      fontWeight: FontWeight.w800,
      height: 1.2,
    ),
    titleMedium: TextStyle(
      fontSize: Tokens.bodySize,
      fontWeight: FontWeight.w700,
    ),
    bodyLarge: TextStyle(fontSize: Tokens.bodySize),
    bodyMedium: TextStyle(fontSize: Tokens.bodySize),
    bodySmall: TextStyle(fontSize: Tokens.captionSize),
    labelLarge: TextStyle(
      fontSize: Tokens.bodySize,
      fontWeight: FontWeight.w700,
    ),
    labelSmall: TextStyle(fontSize: Tokens.captionSize),
  ).apply(bodyColor: c.ink, displayColor: c.ink);

  const pill = StadiumBorder();

  OutlineInputBorder border(Color color, double width) => OutlineInputBorder(
    borderRadius: BorderRadius.circular(Tokens.radius),
    borderSide: BorderSide(color: color, width: width),
  );

  return ThemeData(
    useMaterial3: true,
    brightness: brightness,
    colorScheme: scheme,
    scaffoldBackgroundColor: c.bg,
    canvasColor: c.bg,
    textTheme: text,
    extensions: <ThemeExtension<dynamic>>[c],
    dividerColor: c.line,
    // Primary: a full-width pill in the accent colour. The label is 19 px and
    // heavy so it counts as large text against the accent fill.
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        backgroundColor: c.accent,
        foregroundColor: const Color(0xFFFFFFFF),
        minimumSize: const Size(double.infinity, 54),
        shape: pill,
        padding: const EdgeInsets.symmetric(
          horizontal: Tokens.s5,
          vertical: Tokens.s3,
        ),
        textStyle: const TextStyle(fontSize: 19, fontWeight: FontWeight.w800),
      ),
    ),
    // Secondary: a pill with a line border.
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: c.ink,
        minimumSize: const Size(double.infinity, 54),
        shape: pill,
        side: BorderSide(color: c.line, width: 1.5),
        padding: const EdgeInsets.symmetric(
          horizontal: Tokens.s5,
          vertical: Tokens.s3,
        ),
        textStyle: const TextStyle(fontSize: 19, fontWeight: FontWeight.w800),
      ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: c.brand,
        minimumSize: const Size(Tokens.minTap, Tokens.minTap),
        textStyle: const TextStyle(
          fontSize: Tokens.bodySize,
          fontWeight: FontWeight.w700,
        ),
      ),
    ),
    iconButtonTheme: IconButtonThemeData(
      style: IconButton.styleFrom(
        minimumSize: const Size(Tokens.minTap, Tokens.minTap),
        tapTargetSize: MaterialTapTargetSize.padded,
      ),
    ),
    // Fields: a line border that turns accent on focus and bad on error.
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: c.card,
      contentPadding: const EdgeInsets.symmetric(
        horizontal: Tokens.s4,
        vertical: Tokens.s4,
      ),
      labelStyle: TextStyle(color: c.mute),
      floatingLabelStyle: TextStyle(color: c.mute),
      errorStyle: TextStyle(
        color: c.bad,
        fontSize: Tokens.captionSize,
        fontWeight: FontWeight.w600,
      ),
      errorMaxLines: 3,
      border: border(c.line, 1.5),
      enabledBorder: border(c.line, 1.5),
      focusedBorder: border(c.accent, 2),
      errorBorder: border(c.bad, 1.5),
      focusedErrorBorder: border(c.bad, 2),
      disabledBorder: border(c.line, 1.5),
    ),
    cardTheme: CardThemeData(
      color: c.card,
      elevation: 0,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(Tokens.radius + 4),
        side: BorderSide(color: c.line),
      ),
    ),
    progressIndicatorTheme: ProgressIndicatorThemeData(color: c.accent),
    snackBarTheme: SnackBarThemeData(
      behavior: SnackBarBehavior.floating,
      backgroundColor: c.brand2,
      contentTextStyle: const TextStyle(color: Color(0xFFFFFFFF)),
    ),
  );
}
