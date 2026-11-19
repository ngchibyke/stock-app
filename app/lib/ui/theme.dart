import 'package:material_ui/material_ui.dart';

/// Light and dark themes from one seed. Screens read colours from the theme,
/// never hard-code them, so dark mode and contrast stay correct everywhere.
abstract final class AppTheme {
  static const seed = Color(0xFF4F46E5);

  static ThemeData light() => _build(Brightness.light);
  static ThemeData dark() => _build(Brightness.dark);

  static ThemeData _build(Brightness brightness) {
    // The seed's own colour as the primary, so the app matches the dashboard's indigo; the
    // rest of the scheme is still derived from it.
    final light = brightness == Brightness.light;
    final scheme = ColorScheme.fromSeed(seedColor: seed, brightness: brightness)
        .copyWith(
          primary: light ? seed : const Color(0xFF8B83FF),
          onPrimary: light ? Colors.white : const Color(0xFF14123A),
          tertiary: const Color(0xFF7C3AED),
        );
    final base = ThemeData(
      colorScheme: scheme,
      useMaterial3: true,
      brightness: brightness,
    );
    return base.copyWith(
      scaffoldBackgroundColor: brightness == Brightness.light
          ? const Color(0xFFF5F6F9)
          : const Color(0xFF0D1020),
      appBarTheme: AppBarTheme(
        backgroundColor: Colors.transparent,
        elevation: 0,
        scrolledUnderElevation: 0,
        foregroundColor: scheme.onSurface,
        centerTitle: false,
        titleTextStyle: base.textTheme.titleLarge?.copyWith(
          fontWeight: FontWeight.w700,
          color: scheme.onSurface,
        ),
      ),
      cardTheme: CardThemeData(
        elevation: 0,
        color: scheme.surface,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(18),
          side: BorderSide(color: scheme.outlineVariant),
        ),
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: scheme.surface,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(14),
          borderSide: BorderSide(color: scheme.outlineVariant),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(14),
          borderSide: BorderSide(color: scheme.outlineVariant),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(14),
          borderSide: BorderSide(color: scheme.primary, width: 2),
        ),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 16,
          vertical: 16,
        ),
      ),
      snackBarTheme: SnackBarThemeData(
        behavior: SnackBarBehavior.floating,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(14)),
      ),
    );
  }
}

/// Semantic colours that are not in Material's scheme.
extension StatusColors on ColorScheme {
  Color get good => brightness == Brightness.light
      ? const Color(0xFF0F9D6B)
      : const Color(0xFF3FD39A);
  Color get warn => brightness == Brightness.light
      ? const Color(0xFFB7791F)
      : const Color(0xFFF0B95A);
  Color get bad => brightness == Brightness.light
      ? const Color(0xFFD4383F)
      : const Color(0xFFFF7A80);
}
