import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/format/phone.dart';
import '../../core/paths.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import 'address.dart';
import 'address_errors.dart';
import 'address_repository.dart';

/// The person's saved delivery addresses. They are held in memory only and
/// dropped when this page goes away or the person signs out.
class AddressListPage extends ConsumerStatefulWidget {
  const AddressListPage({super.key});

  @override
  ConsumerState<AddressListPage> createState() => _AddressListPageState();
}

class _AddressListPageState extends ConsumerState<AddressListPage> {
  // Ids being deleted, so a row cannot be deleted twice at once.
  final Set<String> _deleting = {};

  Future<void> _confirmDelete(Address address) async {
    if (_deleting.contains(address.id)) return;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text(Strings.deleteConfirmTitle),
        content: const Text(Strings.deleteConfirmBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text(Strings.cancel),
          ),
          TextButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text(Strings.delete),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;

    setState(() => _deleting.add(address.id));
    final messenger = ScaffoldMessenger.of(context);
    try {
      await ref.read(addressesProvider.notifier).remove(address.id);
    } catch (e) {
      if (!mounted) return;
      messenger.hideCurrentSnackBar();
      messenger.showSnackBar(
        SnackBar(
          content: Text(addressErrorMessage(e, fallback: Strings.deleteFailed)),
        ),
      );
    } finally {
      if (mounted) setState(() => _deleting.remove(address.id));
    }
  }

  @override
  Widget build(BuildContext context) {
    final addresses = ref.watch(addressesProvider);
    return Scaffold(
      appBar: AppBar(
        title: Semantics(header: true, child: const Text(Strings.addresses)),
      ),
      body: SafeArea(
        child: addresses.when(
          loading: () => const _Loading(),
          error: (error, _) => ErrorState(
            message: addressErrorMessage(error),
            onRetry: () => ref.invalidate(addressesProvider),
          ),
          data: (list) {
            if (list.isEmpty) {
              return EmptyState(
                title: Strings.addressesEmptyTitle,
                body: Strings.addressesEmptyBody,
                actionLabel: Strings.addAddress,
                onAction: () => context.push(newAddressPath),
              );
            }
            return Column(
              children: [
                Expanded(
                  child: ListView.separated(
                    padding: const EdgeInsets.all(Tokens.s4),
                    itemCount: list.length,
                    separatorBuilder: (_, _) =>
                        const SizedBox(height: Tokens.s3),
                    itemBuilder: (context, i) => _AddressCard(
                      address: list[i],
                      busy: _deleting.contains(list[i].id),
                      onDelete: () => _confirmDelete(list[i]),
                    ),
                  ),
                ),
                Padding(
                  padding: const EdgeInsets.fromLTRB(
                    Tokens.s4,
                    Tokens.s2,
                    Tokens.s4,
                    Tokens.s4,
                  ),
                  child: FilledButton(
                    onPressed: () => context.push(newAddressPath),
                    child: const Text(
                      Strings.addAddress,
                      textAlign: TextAlign.center,
                    ),
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
  }
}

/// Rows shaped like address cards, announced once as loading.
class _Loading extends StatelessWidget {
  const _Loading();

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: Strings.loading,
      liveRegion: true,
      container: true,
      excludeSemantics: true,
      child: ListView(
        physics: const NeverScrollableScrollPhysics(),
        padding: const EdgeInsets.all(Tokens.s4),
        children: [
          for (var i = 0; i < 3; i++) ...[
            const _SkeletonCard(),
            const SizedBox(height: Tokens.s3),
          ],
        ],
      ),
    );
  }
}

class _SkeletonCard extends StatelessWidget {
  const _SkeletonCard();

  @override
  Widget build(BuildContext context) {
    final colors = context.tokens;
    return DecoratedBox(
      decoration: BoxDecoration(
        color: colors.card,
        borderRadius: BorderRadius.circular(Tokens.radius + 4),
        border: Border.all(color: colors.line),
      ),
      child: const Padding(
        padding: EdgeInsets.all(Tokens.s4),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Skeleton(width: 140),
            SizedBox(height: Tokens.s3),
            Skeleton(),
            SizedBox(height: Tokens.s2),
            Skeleton(width: 200),
          ],
        ),
      ),
    );
  }
}

/// One address. Every line wraps, so a long name or street never clips.
class _AddressCard extends StatelessWidget {
  const _AddressCard({
    required this.address,
    required this.busy,
    required this.onDelete,
  });

  final Address address;
  final bool busy;
  final VoidCallback onDelete;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = context.tokens;
    final landmark = address.landmark;
    final postcode = address.postcode;
    final muted = theme.textTheme.bodyMedium?.copyWith(color: colors.mute);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: colors.card,
        borderRadius: BorderRadius.circular(Tokens.radius + 4),
        border: Border.all(color: colors.line),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Tokens.s4,
          Tokens.s3,
          Tokens.s2,
          Tokens.s3,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (address.recipientName.isNotEmpty)
                    Text(
                      address.recipientName,
                      style: theme.textTheme.titleMedium,
                    ),
                  Text(address.councilLabel, style: muted),
                  const SizedBox(height: Tokens.s2),
                  if (address.addressLine.isNotEmpty)
                    Text(
                      address.addressLine,
                      style: theme.textTheme.bodyMedium,
                    ),
                  if (landmark != null)
                    Text(Strings.landmarkLine(landmark), style: muted),
                  if (postcode != null)
                    Text(Strings.postcodeLine(postcode), style: muted),
                  if (address.phone.isNotEmpty) ...[
                    const SizedBox(height: Tokens.s2),
                    Text(displayPhone(address.phone), style: muted),
                  ],
                ],
              ),
            ),
            IconButton(
              tooltip: Strings.deleteAddressFor(address.recipientName),
              onPressed: busy ? null : onDelete,
              icon: const Icon(Icons.delete_outline),
            ),
          ],
        ),
      ),
    );
  }
}
