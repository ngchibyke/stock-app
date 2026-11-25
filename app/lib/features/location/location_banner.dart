import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import 'location_check.dart';
import 'location_prefs.dart';

/// Asks once whether we may check the phone's rough location, and tells a
/// phone outside the FCT that we are coming soon. Ordering is untouched:
/// the delivery-address rule decides what can be delivered.
class LocationNotice extends ConsumerStatefulWidget {
  const LocationNotice({super.key});

  @override
  ConsumerState<LocationNotice> createState() => _LocationNoticeState();
}

class _LocationNoticeState extends ConsumerState<LocationNotice> {
  bool _show = false;

  bool _started = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // Waits until the market itself is on top: a store or product opened
    // over it (a shared link on first launch) is not interrupted.
    final onTop = ModalRoute.of(context)?.isCurrent ?? true;
    if (!_started && onTop) {
      _started = true;
      WidgetsBinding.instance.addPostFrameCallback((_) => _start());
    }
  }

  Future<void> _start() async {
    if (!mounted) return;
    // Read before any wait: the market may be gone by the time one ends.
    final check = ref.read(locationCheckProvider);
    final now = ref.read(locationClockProvider);
    final prefs = await ref.read(locationPrefsProvider.future);
    if (!mounted) return;
    if (!prefs.asked) {
      final allow = await showModalBottomSheet<bool>(
        context: context,
        isScrollControlled: true,
        builder: (_) => const _AskSheet(),
      );
      await prefs.markAsked();
      if (allow != true || !await check.requestPermission()) return;
      if (!mounted) return;
    }
    if (prefs.closedOn(now())) return;
    final area = await check.currentArea();
    if (mounted && area == Area.outside) setState(() => _show = true);
  }

  Future<void> _close() async {
    setState(() => _show = false);
    final now = ref.read(locationClockProvider);
    final prefs = await ref.read(locationPrefsProvider.future);
    await prefs.closeToday(now());
  }

  @override
  Widget build(BuildContext context) {
    if (!_show) return const SizedBox.shrink();
    final tokens = context.tokens;
    return Align(
      alignment: Alignment.bottomCenter,
      child: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(Tokens.s3),
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: tokens.card,
              borderRadius: BorderRadius.circular(Tokens.radius),
              border: Border.all(color: tokens.line),
            ),
            child: Row(
              children: [
                const SizedBox(width: Tokens.s4),
                Icon(Icons.location_on_outlined, color: tokens.brand),
                const SizedBox(width: Tokens.s3),
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.symmetric(vertical: Tokens.s3),
                    child: Text(
                      Strings.locationComingSoon,
                      style: TextStyle(color: tokens.ink),
                    ),
                  ),
                ),
                Pressable(
                  radius: Tokens.minTap / 2,
                  semanticLabel: Strings.locationClose,
                  onTap: _close,
                  child: SizedBox.square(
                    dimension: Tokens.minTap,
                    child: Icon(Icons.close, color: tokens.mute),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _AskSheet extends StatelessWidget {
  const _AskSheet();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // Scrolls at large text sizes instead of overflowing.
    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(Tokens.s5),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(Strings.locationAskTitle, style: theme.textTheme.titleLarge),
            const SizedBox(height: Tokens.s2),
            Text(Strings.locationAskBody, style: theme.textTheme.bodyLarge),
            const SizedBox(height: Tokens.s5),
            FilledButton(
              onPressed: () => Navigator.of(context).pop(true),
              child: const Text(Strings.locationAllow),
            ),
            const SizedBox(height: Tokens.s2),
            TextButton(
              onPressed: () => Navigator.of(context).pop(false),
              child: const Text(Strings.locationNotNow),
            ),
          ],
        ),
      ),
    );
  }
}
