import 'package:flutter/widgets.dart';

/// Motion tokens. Personality: quick and tactile. Short travel, snappy
/// springs, nothing over [slow] except hero moments (the tracking screen, an
/// order placed). Every animated widget reads these, so the feel changes in one
/// place.
abstract final class Motion {
  static const instant = Duration(milliseconds: 100);
  static const fast = Duration(milliseconds: 180);
  static const base = Duration(milliseconds: 280);
  static const slow = Duration(milliseconds: 420);
  static const hero = Duration(milliseconds: 700);

  static const Curve out = Curves.easeOutCubic;
  static const Curve inOut = Curves.easeInOutCubic;
  static const Curve spring = Curves.easeOutBack;

  /// A press: down fast, back with a little overshoot.
  static const pressScale = 0.96;

  static SpringDescription get snappy =>
      const SpringDescription(mass: 1, stiffness: 420, damping: 28);
  static SpringDescription get bouncy =>
      const SpringDescription(mass: 1, stiffness: 260, damping: 14);

  /// People who ask their system for less motion get none of the movement.
  static bool reduced(BuildContext context) =>
      MediaQuery.maybeDisableAnimationsOf(context) ?? false;
}
