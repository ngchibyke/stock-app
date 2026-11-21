import 'package:material_ui/material_ui.dart';

/// A status chip's pair of colours: the fill and the text on it.
@immutable
class ChipColors {
  const ChipColors({required this.background, required this.foreground});

  final Color background;
  final Color foreground;
}

/// The colour roles of one brightness ("Abuja Dusk"). Read them from the
/// theme with `context.tokens`, never hard-code a colour in a widget.
@immutable
class TokenColors extends ThemeExtension<TokenColors> {
  const TokenColors({
    required this.brand,
    required this.brand2,
    required this.accent,
    required this.gold,
    required this.bg,
    required this.card,
    required this.ink,
    required this.mute,
    required this.line,
    required this.good,
    required this.bad,
    required this.paid,
    required this.onTheWay,
    required this.delivered,
    required this.reportOpen,
  });

  /// Brand navy; on dark it is lightened so text and icons stay readable.
  final Color brand;
  final Color brand2;

  /// Primary buttons and focus.
  final Color accent;
  final Color gold;
  final Color bg;
  final Color card;
  final Color ink;
  final Color mute;
  final Color line;
  final Color good;
  final Color bad;

  final ChipColors paid;
  final ChipColors onTheWay;
  final ChipColors delivered;
  final ChipColors reportOpen;

  @override
  TokenColors copyWith({
    Color? brand,
    Color? brand2,
    Color? accent,
    Color? gold,
    Color? bg,
    Color? card,
    Color? ink,
    Color? mute,
    Color? line,
    Color? good,
    Color? bad,
    ChipColors? paid,
    ChipColors? onTheWay,
    ChipColors? delivered,
    ChipColors? reportOpen,
  }) {
    return TokenColors(
      brand: brand ?? this.brand,
      brand2: brand2 ?? this.brand2,
      accent: accent ?? this.accent,
      gold: gold ?? this.gold,
      bg: bg ?? this.bg,
      card: card ?? this.card,
      ink: ink ?? this.ink,
      mute: mute ?? this.mute,
      line: line ?? this.line,
      good: good ?? this.good,
      bad: bad ?? this.bad,
      paid: paid ?? this.paid,
      onTheWay: onTheWay ?? this.onTheWay,
      delivered: delivered ?? this.delivered,
      reportOpen: reportOpen ?? this.reportOpen,
    );
  }

  @override
  TokenColors lerp(ThemeExtension<TokenColors>? other, double t) {
    if (other is! TokenColors) return this;
    Color mix(Color a, Color b) => Color.lerp(a, b, t)!;
    ChipColors mixChip(ChipColors a, ChipColors b) => ChipColors(
      background: mix(a.background, b.background),
      foreground: mix(a.foreground, b.foreground),
    );
    return TokenColors(
      brand: mix(brand, other.brand),
      brand2: mix(brand2, other.brand2),
      accent: mix(accent, other.accent),
      gold: mix(gold, other.gold),
      bg: mix(bg, other.bg),
      card: mix(card, other.card),
      ink: mix(ink, other.ink),
      mute: mix(mute, other.mute),
      line: mix(line, other.line),
      good: mix(good, other.good),
      bad: mix(bad, other.bad),
      paid: mixChip(paid, other.paid),
      onTheWay: mixChip(onTheWay, other.onTheWay),
      delivered: mixChip(delivered, other.delivered),
      reportOpen: mixChip(reportOpen, other.reportOpen),
    );
  }
}

/// Design tokens: spacing, shape and the colour roles for light and dark.
abstract final class Tokens {
  // Spacing scale (dp).
  static const double s1 = 4;
  static const double s2 = 8;
  static const double s3 = 12;
  static const double s4 = 16;
  static const double s5 = 24;
  static const double s6 = 32;

  /// Corner radius of cards and fields.
  static const double radius = 14;

  /// Top corner radius of the sheet that rises over a brand header.
  static const double sheetRadius = 24;

  /// Smallest tap target (dp), both ways.
  static const double minTap = 48;

  // Type scale (logical pixels).
  static const double displaySize = 32;
  static const double titleSize = 22;
  static const double bodySize = 16;
  static const double captionSize = 13;

  /// The header gradient is the same navy in both brightnesses.
  static const Color headerFrom = Color(0xFF14193D);
  static const Color headerTo = Color(0xFF232B63);

  /// Text and icons on the header gradient.
  static const Color onHeader = Color(0xFFFFFFFF);

  static const TokenColors light = TokenColors(
    brand: Color(0xFF14193D),
    brand2: Color(0xFF232B63),
    accent: Color(0xFFE2662D),
    gold: Color(0xFFF5B83D),
    bg: Color(0xFFFBF7F2),
    card: Color(0xFFFFFFFF),
    ink: Color(0xFF14193D),
    mute: Color(0xFF6B7089),
    line: Color(0xFFECE6DE),
    good: Color(0xFF1E9E6A),
    bad: Color(0xFFD4383F),
    paid: ChipColors(
      background: Color(0xFFE7EAFB),
      foreground: Color(0xFF2B3AA8),
    ),
    onTheWay: ChipColors(
      background: Color(0xFFFFE9DC),
      foreground: Color(0xFFB44A14),
    ),
    delivered: ChipColors(
      background: Color(0xFFDDF5EA),
      foreground: Color(0xFF12805A),
    ),
    reportOpen: ChipColors(
      background: Color(0xFFFDE2E3),
      foreground: Color(0xFFB3262E),
    ),
  );

  /// Derived from the light palette: `bg`, `card`, `ink`, `mute` and `line`
  /// are the owner's values; `bad` and the chip colours are lightened here so
  /// they keep their contrast on the dark surfaces.
  static const TokenColors dark = TokenColors(
    brand: Color(0xFF8F98F0),
    brand2: Color(0xFF232B63),
    accent: Color(0xFFE2662D),
    gold: Color(0xFFF5B83D),
    bg: Color(0xFF0E1127),
    card: Color(0xFF171B3A),
    ink: Color(0xFFF3F0EA),
    mute: Color(0xFF9AA0BD),
    line: Color(0xFF2A2F55),
    good: Color(0xFF3CC48C),
    bad: Color(0xFFFF7B80),
    paid: ChipColors(
      background: Color(0xFF1F2660),
      foreground: Color(0xFFB5BCFF),
    ),
    onTheWay: ChipColors(
      background: Color(0xFF4A2410),
      foreground: Color(0xFFFFB48A),
    ),
    delivered: ChipColors(
      background: Color(0xFF0F3D2E),
      foreground: Color(0xFF6FDDB0),
    ),
    reportOpen: ChipColors(
      background: Color(0xFF4A1A1D),
      foreground: Color(0xFFFF9EA3),
    ),
  );
}

extension TokensContext on BuildContext {
  /// The colour roles of the current theme.
  TokenColors get tokens => Theme.of(this).extension<TokenColors>()!;
}
