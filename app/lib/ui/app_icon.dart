import 'dart:math' as math;

import 'package:material_ui/material_ui.dart';

import 'motion.dart';

/// How an icon moves when it is pressed, or when it becomes selected.
enum IconBehaviour { spin, wiggle, bounce, pulse, slide, turn, tilt, pop }

class IconSpec {
  const IconSpec(this.data, this.behaviour);
  final IconData data;
  final IconBehaviour behaviour;
}

/// The app's icons. Each has its own behaviour, so a press feels like the
/// thing it is: the gear turns, the bell rings, the truck drives off.
abstract final class AppIcons {
  static const home = IconSpec(Icons.home_rounded, IconBehaviour.bounce);
  static const search = IconSpec(Icons.search_rounded, IconBehaviour.tilt);
  static const orders = IconSpec(
    Icons.inventory_2_rounded,
    IconBehaviour.slide,
  );
  static const chat = IconSpec(Icons.chat_bubble_rounded, IconBehaviour.pop);
  static const account = IconSpec(Icons.person_rounded, IconBehaviour.pulse);
  static const store = IconSpec(Icons.storefront_rounded, IconBehaviour.bounce);
  static const cart = IconSpec(
    Icons.shopping_bag_rounded,
    IconBehaviour.wiggle,
  );
  static const heart = IconSpec(Icons.favorite_rounded, IconBehaviour.pulse);
  static const bell = IconSpec(
    Icons.notifications_rounded,
    IconBehaviour.wiggle,
  );
  static const truck = IconSpec(
    Icons.local_shipping_rounded,
    IconBehaviour.slide,
  );
  static const check = IconSpec(Icons.check_rounded, IconBehaviour.pop);
  static const close = IconSpec(Icons.close_rounded, IconBehaviour.spin);
  static const add = IconSpec(Icons.add_rounded, IconBehaviour.spin);
  static const remove = IconSpec(Icons.remove_rounded, IconBehaviour.pop);
  static const refresh = IconSpec(Icons.refresh_rounded, IconBehaviour.turn);
  static const settings = IconSpec(Icons.settings_rounded, IconBehaviour.spin);
  static const logout = IconSpec(Icons.logout_rounded, IconBehaviour.slide);
  static const eye = IconSpec(Icons.visibility_rounded, IconBehaviour.pulse);
  static const eyeOff = IconSpec(
    Icons.visibility_off_rounded,
    IconBehaviour.pulse,
  );
  static const phone = IconSpec(Icons.call_rounded, IconBehaviour.wiggle);
  static const lock = IconSpec(Icons.lock_rounded, IconBehaviour.pop);
  static const camera = IconSpec(Icons.photo_camera_rounded, IconBehaviour.pop);
  static const star = IconSpec(Icons.star_rounded, IconBehaviour.pop);
  static const warn = IconSpec(
    Icons.warning_amber_rounded,
    IconBehaviour.wiggle,
  );
  static const info = IconSpec(Icons.info_rounded, IconBehaviour.pop);
  static const back = IconSpec(Icons.arrow_back_rounded, IconBehaviour.slide);
  static const send = IconSpec(Icons.send_rounded, IconBehaviour.slide);
  static const flag = IconSpec(Icons.flag_rounded, IconBehaviour.wiggle);
  static const dispute = IconSpec(Icons.balance_rounded, IconBehaviour.tilt);
  static const clock = IconSpec(Icons.schedule_rounded, IconBehaviour.turn);
  static const pin = IconSpec(Icons.place_rounded, IconBehaviour.bounce);

  /// Every icon, for tests and a gallery.
  static const all = <String, IconSpec>{
    'home': home,
    'search': search,
    'orders': orders,
    'chat': chat,
    'account': account,
    'store': store,
    'cart': cart,
    'heart': heart,
    'bell': bell,
    'truck': truck,
    'check': check,
    'close': close,
    'add': add,
    'remove': remove,
    'refresh': refresh,
    'settings': settings,
    'logout': logout,
    'eye': eye,
    'eyeOff': eyeOff,
    'phone': phone,
    'lock': lock,
    'camera': camera,
    'star': star,
    'warn': warn,
    'info': info,
    'back': back,
    'send': send,
    'flag': flag,
    'dispute': dispute,
    'clock': clock,
    'pin': pin,
  };
}

/// The motion of one behaviour at progress [t] in 0..1: (rotation in radians,
/// scale, dx, dy in logical pixels). Pure, so it is easy to test.
({double rotate, double scale, double dx, double dy}) behaviourAt(
  IconBehaviour b,
  double t,
) {
  final eased = Curves.easeOutCubic.transform(t);
  final bell = math.sin(t * math.pi);
  switch (b) {
    case IconBehaviour.spin:
      return (
        rotate: Curves.easeOutBack.transform(t) * math.pi / 2,
        scale: 1,
        dx: 0,
        dy: 0,
      );
    case IconBehaviour.wiggle:
      return (
        rotate: math.sin(t * math.pi * 4) * (1 - t) * 0.4,
        scale: 1,
        dx: 0,
        dy: 0,
      );
    case IconBehaviour.bounce:
      return (rotate: 0, scale: 1, dx: 0, dy: -bell * 4);
    case IconBehaviour.pulse:
      return (rotate: 0, scale: 1 + bell * 0.24, dx: 0, dy: 0);
    case IconBehaviour.slide:
      return (rotate: 0, scale: 1, dx: bell * 4, dy: 0);
    case IconBehaviour.turn:
      return (rotate: eased * math.pi, scale: 1, dx: 0, dy: 0);
    case IconBehaviour.tilt:
      return (
        rotate: math.sin(t * math.pi * 2) * (1 - t) * 0.3,
        scale: 1,
        dx: 0,
        dy: -bell * 1.5,
      );
    case IconBehaviour.pop:
      return (rotate: 0, scale: 1 + bell * 0.32, dx: 0, dy: 0);
  }
}

/// An icon that answers a touch. It plays its behaviour when pressed, when
/// [trigger] changes (a parent button counts presses), and when [active] turns
/// true (a tab becoming selected). With "reduce motion" on it never moves.
class AppIcon extends StatefulWidget {
  const AppIcon(
    this.spec, {
    super.key,
    this.size = 24,
    this.color,
    this.active = false,
    this.trigger = 0,
    this.semanticLabel,
  });

  final IconSpec spec;
  final double size;
  final Color? color;
  final bool active;

  /// Change this number to play the animation from outside.
  final int trigger;
  final String? semanticLabel;

  @override
  State<AppIcon> createState() => _AppIconState();
}

class _AppIconState extends State<AppIcon> with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 560),
  );

  void _play() {
    if (Motion.reduced(context)) return;
    _c.forward(from: 0);
  }

  @override
  void didUpdateWidget(AppIcon old) {
    super.didUpdateWidget(old);
    if (widget.trigger != old.trigger || (widget.active && !old.active))
      _play();
  }

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Listener(
      behavior: HitTestBehavior.translucent,
      onPointerDown: (_) => _play(),
      child: AnimatedBuilder(
        animation: _c,
        builder: (context, child) {
          final m = behaviourAt(widget.spec.behaviour, _c.value);
          return Transform.translate(
            offset: Offset(m.dx, m.dy),
            child: Transform.rotate(
              angle: m.rotate,
              child: Transform.scale(scale: m.scale, child: child),
            ),
          );
        },
        child: Icon(
          widget.spec.data,
          size: widget.size,
          color: widget.color,
          semanticLabel: widget.semanticLabel,
        ),
      ),
    );
  }
}
