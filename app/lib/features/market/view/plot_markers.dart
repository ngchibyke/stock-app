import 'package:flutter/semantics.dart';
import 'package:material_ui/material_ui.dart';

import '../../../core/strings.dart';
import '../../../core/theme/tokens.dart';
import '../camera/market_camera.dart';
import '../model/market.dart';

/// Names and ratings show from this zoom.
const labelZoom = 1.6;

/// Half the side of a marker's touch area (48 dp whatever the zoom).
const _hitHalf = Tokens.minTap / 2;

/// One plot's marker, placed on screen.
class MarkerSpot {
  const MarkerSpot({
    required this.plot,
    required this.placement,
    required this.screen,
    required this.label,
  });

  final Plot plot;
  final Placement? placement;
  final Offset screen;
  final String label;

  bool get isOpen => placement != null;
}

/// The markers on screen, in reading order: district, then name.
List<MarkerSpot> markerSpots(MarketSnapshot snap, MarketCamera camera) {
  final view = Offset.zero & camera.viewport;
  final districtOrder = {
    for (var i = 0; i < snap.layout.districts.length; i++)
      snap.layout.districts[i].id: i,
  };
  final spots = <MarkerSpot>[
    for (final plot in snap.layout.plots)
      if (camera.worldToScreen(plot.rect.center) case final s
          when view.inflate(_hitHalf).contains(s))
        _spot(snap, plot, s),
  ];
  String sortName(MarkerSpot m) =>
      m.placement?.store.name.toLowerCase() ?? '\u{FFFF}${m.plot.id}';
  spots.sort((a, b) {
    final d = (districtOrder[a.plot.districtId] ?? 999).compareTo(
      districtOrder[b.plot.districtId] ?? 999,
    );
    return d != 0 ? d : sortName(a).compareTo(sortName(b));
  });
  return spots;
}

MarkerSpot _spot(MarketSnapshot snap, Plot plot, Offset screen) {
  final placement = snap.placementFor(plot.id);
  final store = placement?.store;
  final label = store == null
      ? Strings.comingSoonLabel(
          snap.layout.district(plot.districtId)?.name ?? '',
        )
      : Strings.openStoreLabel(
          store.name,
          store.ratingAverage,
          store.ratingCount,
        );
  return MarkerSpot(
    plot: plot,
    placement: placement,
    screen: screen,
    label: label,
  );
}

/// The marker whose 48 dp touch area holds [point]; the nearest centre wins
/// where areas overlap.
MarkerSpot? markerAt(List<MarkerSpot> spots, Offset point) {
  MarkerSpot? best;
  var bestDistance = double.infinity;
  for (final m in spots) {
    final d = point - m.screen;
    if (d.dx.abs() > _hitHalf || d.dy.abs() > _hitHalf) continue;
    if (d.distanceSquared < bestDistance) {
      best = m;
      bestDistance = d.distanceSquared;
    }
  }
  return best;
}

/// Draws the markers and gives each one to screen readers as a button.
/// Touches are resolved by the view (see [markerAt]), not here.
class PlotMarkers extends StatefulWidget {
  const PlotMarkers({
    super.key,
    required this.spots,
    required this.zoom,
    required this.pressedPlotId,
    required this.onActivate,
  });

  final List<MarkerSpot> spots;
  final double zoom;
  final String? pressedPlotId;

  /// A screen reader's double tap on a marker.
  final ValueChanged<MarkerSpot> onActivate;

  @override
  State<PlotMarkers> createState() => _PlotMarkersState();
}

class _PlotMarkersState extends State<PlotMarkers>
    with SingleTickerProviderStateMixin {
  late final AnimationController _pulse = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1600),
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _pulse.stop();
      _pulse.value = 0.5;
    } else if (!_pulse.isAnimating) {
      _pulse.repeat(reverse: true);
    }
  }

  @override
  void dispose() {
    _pulse.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    const width = 168.0;
    final showLabels = widget.zoom >= labelZoom;
    return Stack(
      clipBehavior: Clip.none,
      children: [
        for (var i = 0; i < widget.spots.length; i++)
          Positioned(
            key: ValueKey(widget.spots[i].plot.id),
            left: widget.spots[i].screen.dx - width / 2,
            top: widget.spots[i].screen.dy - _hitHalf,
            width: width,
            child: Semantics(
              container: true,
              button: widget.spots[i].isOpen,
              label: widget.spots[i].label,
              sortKey: OrdinalSortKey(i.toDouble()),
              onTap: widget.spots[i].isOpen
                  ? () => widget.onActivate(widget.spots[i])
                  : null,
              excludeSemantics: true,
              child: _Marker(
                spot: widget.spots[i],
                showLabel: showLabels,
                pressed: widget.pressedPlotId == widget.spots[i].plot.id,
                pulse: _pulse,
              ),
            ),
          ),
      ],
    );
  }
}

class _Marker extends StatelessWidget {
  const _Marker({
    required this.spot,
    required this.showLabel,
    required this.pressed,
    required this.pulse,
  });

  final MarkerSpot spot;
  final bool showLabel;
  final bool pressed;
  final Animation<double> pulse;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final open = spot.isOpen;

    final dot = open
        ? AnimatedBuilder(
            animation: pulse,
            builder: (context, _) => Container(
              width: 16,
              height: 16,
              decoration: BoxDecoration(
                color: tokens.gold,
                shape: BoxShape.circle,
                border: Border.all(color: const Color(0xFFFFFFFF), width: 2),
                boxShadow: [
                  BoxShadow(
                    color: tokens.gold.withValues(
                      alpha: 0.35 + 0.3 * pulse.value,
                    ),
                    blurRadius: 8 + 10 * pulse.value,
                    spreadRadius: 1 + 3 * pulse.value,
                  ),
                ],
              ),
            ),
          )
        : Container(
            width: 10,
            height: 10,
            decoration: BoxDecoration(
              color: tokens.mute.withValues(alpha: 0.7),
              shape: BoxShape.circle,
              border: Border.all(color: const Color(0xCCFFFFFF), width: 1.5),
            ),
          );

    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
          height: Tokens.minTap,
          child: Center(
            child: AnimatedScale(
              scale: pressed ? 1.3 : 1,
              duration: reduceMotion
                  ? Duration.zero
                  : const Duration(milliseconds: 90),
              curve: Curves.easeOut,
              child: dot,
            ),
          ),
        ),
        if (showLabel) open ? _StoreTag(spot: spot) : const _SoonTag(),
      ],
    );
  }
}

class _StoreTag extends StatelessWidget {
  const _StoreTag({required this.spot});

  final MarkerSpot spot;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final store = spot.placement!.store;
    final style = Theme.of(context).textTheme.labelMedium
        ?.copyWith(color: Tokens.onHeader, fontWeight: FontWeight.w600);
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: Tokens.s2,
        vertical: Tokens.s1,
      ),
      decoration: BoxDecoration(
        color: Tokens.headerFrom.withValues(alpha: 0.88),
        borderRadius: BorderRadius.circular(Tokens.s2),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Flexible(
            child: Text(
              store.name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: style,
            ),
          ),
          if (store.ratingAverage != null && store.ratingCount > 0) ...[
            const SizedBox(width: Tokens.s1),
            Icon(Icons.star_rounded, size: 13, color: tokens.gold),
            Text(Strings.rating(store.ratingAverage!), style: style),
          ],
        ],
      ),
    );
  }
}

class _SoonTag extends StatelessWidget {
  const _SoonTag();

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: Tokens.s2, vertical: 2),
      decoration: BoxDecoration(
        color: const Color(0xD9FFFFFF),
        borderRadius: BorderRadius.circular(Tokens.s2),
      ),
      child: Text(
        Strings.comingSoon,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: Theme.of(context).textTheme.labelSmall
            ?.copyWith(color: tokens.mute),
      ),
    );
  }
}
