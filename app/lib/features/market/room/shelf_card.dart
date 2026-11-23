import 'dart:math' as math;

import 'package:material_ui/material_ui.dart';

import '../../../core/format/money.dart';
import '../../../core/strings.dart';
import '../../../core/theme/tokens.dart';
import '../../../core/ui/pressable.dart';
import '../../catalog/catalog_models.dart';
import '../model/room.dart';

/// How far the nearest card drifts at a full look, in dp.
const shelfDrift = 6.0;

/// A product standing on a shelf: its photo turned to face along the shelf,
/// lit by the room, with its starting price.
///
/// The card is drawn at [size] but always takes at least 48 dp each way, so a
/// small far-away card is still easy to hit.
class ShelfCard extends StatelessWidget {
  const ShelfCard({
    super.key,
    required this.product,
    required this.slot,
    required this.type,
    required this.size,
    required this.look,
    required this.onTap,
  });

  final ProductCard product;
  final ShelfSlot slot;
  final StoreType type;

  /// The drawn card, from the slot and the picture's size on screen.
  final Size size;

  /// The room's view offset, -1 to 1.
  final Offset look;
  final VoidCallback onTap;

  /// Nearer cards move more, so they sit at their depth in the room.
  Offset get drift => look * shelfDrift * slot.depth;

  @override
  Widget build(BuildContext context) {
    final price = formatNaira(product.priceFromKobo);
    return Pressable(
      semanticLabel: Strings.shelfCardLabel(product.name, price),
      onTap: onTap,
      radius: Tokens.s2,
      child: SizedBox(
        width: math.max(size.width, Tokens.minTap),
        height: math.max(size.height, Tokens.minTap),
        child: Center(
          child: Transform.translate(
            offset: drift,
            child: Transform(
              alignment: Alignment.center,
              transform: Matrix4.identity()
                ..setEntry(3, 2, 0.0015)
                ..rotateY(slot.tiltY),
              child: SizedBox.fromSize(
                size: size,
                child: _Card(product: product, type: type, price: price),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _Card extends StatelessWidget {
  const _Card({required this.product, required this.type, required this.price});

  final ProductCard product;
  final StoreType type;
  final String price;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final url = product.photoUrl;
    final tile = ColoredBox(
      color: tokens.line,
      child: Center(
        child: Icon(Icons.shopping_bag_outlined, color: tokens.mute, size: 20),
      ),
    );
    return DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(Tokens.s2),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: type.shadowOpacity),
            blurRadius: 10,
            offset: const Offset(0, 5),
          ),
        ],
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(Tokens.s2),
        child: ColoredBox(
          color: tokens.card,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(
                child: Stack(
                  fit: StackFit.expand,
                  children: [
                    if (url == null)
                      tile
                    else
                      Image.network(
                        url,
                        key: const Key('shelf-photo'),
                        fit: BoxFit.cover,
                        excludeFromSemantics: true,
                        errorBuilder: (_, _, _) => tile,
                        frameBuilder: (context, child, frame, sync) => sync
                            ? child
                            : AnimatedOpacity(
                                opacity: frame == null ? 0 : 1,
                                duration: const Duration(milliseconds: 150),
                                child: child,
                              ),
                      ),
                    // The room's light falls on the photo.
                    ColoredBox(color: type.lightTint.withValues(alpha: 0.12)),
                  ],
                ),
              ),
              Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Tokens.s1 + 2,
                  vertical: Tokens.s1,
                ),
                child: FittedBox(
                  fit: BoxFit.scaleDown,
                  alignment: Alignment.centerLeft,
                  child: Text(
                    Strings.fromPrice(price),
                    maxLines: 1,
                    style: TextStyle(
                      color: tokens.ink,
                      fontSize: 11,
                      fontWeight: FontWeight.w700,
                      height: 1.1,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
