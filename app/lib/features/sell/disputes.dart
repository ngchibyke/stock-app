import 'dart:async';

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/format/money.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import 'order_models.dart';
import 'photos/camera_page.dart';
import 'photos/photo_prep.dart';
import 'photos/photo_section.dart';
import 'photos/photo_source.dart';
import 'photos/photo_upload.dart';
import 'sell_models.dart';
import 'sell_repository.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';

final disputeProvider = FutureProvider.autoDispose.family<SellDispute, String>(
  (ref, id) => ref.watch(workRepositoryProvider).dispute(id),
);

/// A buyer's photo, fetched with the session on demand: the link is short-lived and
/// never kept.
class _EvidencePhoto extends ConsumerWidget {
  const _EvidencePhoto({required this.disputeId, required this.photoKey});

  final String disputeId;
  final String photoKey;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final link = ref.watch(_photoLinkProvider((disputeId, photoKey)));
    final url = link.value;
    return SizedBox(
      width: 96,
      height: 96,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(Tokens.radius),
        child: url == null
            ? ColoredBox(color: context.tokens.line)
            : CachedNetworkImage(
                imageUrl: url,
                fit: BoxFit.cover,
                memCacheWidth: 200,
                cacheKey: photoKey,
              ),
      ),
    );
  }
}

final _photoLinkProvider = FutureProvider.autoDispose
    .family<String, (String, String)>(
      (ref, k) => ref.watch(workRepositoryProvider).disputePhotoUrl(k.$1, k.$2),
    );

/// The orders that have a reported problem, found in the store's recent orders.
class DisputesPage extends ConsumerStatefulWidget {
  const DisputesPage({super.key, required this.store});

  final SellStore store;

  @override
  ConsumerState<DisputesPage> createState() => _DisputesPageState();
}

class _DisputesPageState extends ConsumerState<DisputesPage> {
  List<SellOrder>? _found;
  String? _error;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    setState(() {
      _found = null;
      _error = null;
    });
    try {
      final repo = ref.read(workRepositoryProvider);
      final found = <SellOrder>[];
      String? cursor;
      // The four most recent pages of orders are enough to find what is open.
      for (var i = 0; i < 4; i++) {
        final page = await repo.orders(widget.store.id, before: cursor);
        found.addAll(page.items.where((o) => o.disputeId != null));
        cursor = page.nextCursor;
        if (cursor == null) break;
      }
      if (mounted) setState(() => _found = found);
    } on Object catch (e) {
      if (mounted) setState(() => _error = sellError(e));
    }
  }

  @override
  Widget build(BuildContext context) {
    final found = _found;
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellAppBar(context, SellStrings.disputesTitle),
      body: _error != null
          ? ErrorState(message: _error!, onRetry: _load)
          : found == null
          ? const Center(child: CircularProgressIndicator())
          : found.isEmpty
          ? const EmptyState(title: SellStrings.noDisputes)
          : ListView.separated(
              itemCount: found.length,
              separatorBuilder: (_, _) => const Divider(height: 1),
              itemBuilder: (context, i) {
                final o = found[i];
                return SellRow(
                  title: o.lines.isEmpty
                      ? formatNaira(o.totalKobo)
                      : o.lines.first.name,
                  subtitle: formatNaira(o.totalKobo),
                  onTap: () => pushPage<void>(
                    context,
                    DisputePage(store: widget.store, disputeId: o.disputeId!),
                  ),
                );
              },
            ),
    );
  }
}

/// One reported problem: the buyer's side, and the seller's answer.
class DisputePage extends ConsumerStatefulWidget {
  const DisputePage({super.key, required this.store, required this.disputeId});

  final SellStore store;
  final String disputeId;

  @override
  ConsumerState<DisputePage> createState() => _DisputePageState();
}

class _DisputePageState extends ConsumerState<DisputePage> with BusyAction {
  final _answer = TextEditingController();
  final List<PhotoEntry> _photos = [];

  @override
  void dispose() {
    _answer.dispose();
    super.dispose();
  }

  Future<void> _addPhoto(Future<dynamic> Function() pick) async {
    final raw = await pick();
    if (raw == null || !mounted) return;
    final entry = PhotoEntry.fresh(raw);
    setState(() => _photos.add(entry));
    unawaited(_upload(entry));
  }

  Future<void> _upload(PhotoEntry entry) async {
    final raw = entry.bytes;
    if (raw == null) return;
    setState(() => entry.status = PhotoStatus.uploading);
    try {
      final prepared = await preparePhoto(raw);
      if (prepared == null) throw const FormatException('not a picture');
      entry.key = await ref
          .read(photoUploaderProvider)
          .upload(prepared, purpose: 'dispute');
      if (mounted) setState(() => entry.status = PhotoStatus.ready);
    } on Object {
      if (mounted) setState(() => entry.status = PhotoStatus.failed);
    }
  }

  Future<void> _send() async {
    final text = _answer.text.trim();
    if (text.isEmpty || _photos.any((p) => p.status == PhotoStatus.uploading)) {
      return;
    }
    final ok = await run(() async {
      await ref.read(workRepositoryProvider).respondToDispute(
        widget.disputeId,
        text,
        [
          for (final p in _photos)
            if (p.key != null) p.key!,
        ],
      );
    });
    if (ok) ref.invalidate(disputeProvider(widget.disputeId));
  }

  Future<void> _return() async {
    final ok = await run(() async {
      await ref.read(workRepositoryProvider).returnReceived(widget.disputeId);
    });
    if (ok) ref.invalidate(disputeProvider(widget.disputeId));
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final dispute = ref.watch(disputeProvider(widget.disputeId));
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: sellAppBar(context, SellStrings.disputesTitle),
      body: SafeArea(
        child: AsyncBody<SellDispute>(
          value: dispute,
          onRetry: () => ref.invalidate(disputeProvider(widget.disputeId)),
          builder: (d) => ListView(
            padding: const EdgeInsets.all(Tokens.s5),
            children: [
              Text(SellStrings.buyerSaid, style: theme.textTheme.titleMedium),
              const SizedBox(height: Tokens.s2),
              Text(d.buyer.body, style: theme.textTheme.bodyLarge),
              if (d.buyer.photoKeys.isNotEmpty) ...[
                const SizedBox(height: Tokens.s3),
                Wrap(
                  spacing: Tokens.s2,
                  runSpacing: Tokens.s2,
                  children: [
                    for (final k in d.buyer.photoKeys)
                      _EvidencePhoto(disputeId: d.id, photoKey: k),
                  ],
                ),
              ],
              const SizedBox(height: Tokens.s5),
              if (d.seller != null) ...[
                Text(
                  SellStrings.yourAnswer,
                  style: theme.textTheme.titleMedium,
                ),
                const SizedBox(height: Tokens.s2),
                Text(d.seller!.body, style: theme.textTheme.bodyLarge),
              ] else if (d.canRespond && widget.store.role.isManagerUp) ...[
                Text(
                  '${SellStrings.answerBy} ${'${d.respondBy.toLocal()}'.split('.').first}',
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: tokens.mute,
                  ),
                ),
                const SizedBox(height: Tokens.s2),
                AppTextField(
                  label: SellStrings.answerLabel,
                  controller: _answer,
                  maxLines: 5,
                  maxLength: 2000,
                  capitalization: TextCapitalization.sentences,
                  onChanged: (_) => setState(() {}),
                ),
                const SizedBox(height: Tokens.s3),
                PhotoSection(
                  entries: _photos,
                  onGuide: () => showPhotoGuide(context),
                  onCamera: () => _addPhoto(
                    () => ref
                        .read(photoSourceProvider)
                        .fromCamera(context, shotIndex: _photos.length),
                  ),
                  onGallery: () =>
                      _addPhoto(ref.read(photoSourceProvider).fromGallery),
                  onRemove: (i) => setState(() => _photos.removeAt(i)),
                  onRetry: (i) => _upload(_photos[i]),
                ),
                if (error != null) ...[
                  const SizedBox(height: Tokens.s3),
                  ErrorLine(error!),
                ],
                const SizedBox(height: Tokens.s4),
                LoadingButton(
                  label: SellStrings.sendAnswer,
                  loading: busy,
                  onPressed: _answer.text.trim().isEmpty ? null : _send,
                ),
              ],
              if (d.returnRequested && widget.store.role.isManagerUp) ...[
                const SizedBox(height: Tokens.s4),
                OutlinedButton(
                  onPressed: busy ? null : _return,
                  child: const Text(SellStrings.returnReceived),
                ),
              ],
              if (error != null && d.seller != null) ...[
                const SizedBox(height: Tokens.s3),
                ErrorLine(error!),
              ],
            ],
          ),
        ),
      ),
    );
  }
}
