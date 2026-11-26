import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/format/money.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import '../../core/ui/states.dart';
import '../sell/photos/camera_page.dart';
import '../sell/photos/photo_prep.dart';
import '../sell/photos/photo_section.dart';
import '../sell/photos/photo_source.dart';
import '../sell/photos/photo_upload.dart';
import 'buy_models.dart';
import 'buy_repository.dart';
import 'buy_strings.dart';
import 'checkout_page.dart' show ErrorLine;

/// The most photos a report or a review takes (the server's limit).
const int maxEvidencePhotos = 5;

/// Photos for a report or a review: taken or chosen, cut square, shrunk, stripped of
/// tags and uploaded as they are added.
mixin PhotoPicking<T extends ConsumerStatefulWidget> on ConsumerState<T> {
  final List<PhotoEntry> photos = [];

  String get photoPurpose;

  Future<void> addPhoto(Future<dynamic> Function() pick) async {
    final raw = await pick();
    if (raw == null || !mounted) return;
    final entry = PhotoEntry.fresh(raw);
    setState(() => photos.add(entry));
    unawaited(uploadPhoto(entry));
  }

  Future<void> uploadPhoto(PhotoEntry entry) async {
    final raw = entry.bytes;
    if (raw == null) return;
    setState(() => entry.status = PhotoStatus.uploading);
    try {
      final prepared = await preparePhoto(raw);
      if (prepared == null) throw const FormatException('not a picture');
      entry.key = await ref
          .read(photoUploaderProvider)
          .upload(prepared, purpose: photoPurpose);
      if (mounted) setState(() => entry.status = PhotoStatus.ready);
    } on Object {
      if (mounted) setState(() => entry.status = PhotoStatus.failed);
    }
  }

  bool get uploading => photos.any((p) => p.status == PhotoStatus.uploading);
  List<String> get photoKeys => [
    for (final p in photos)
      if (p.key != null) p.key!,
  ];

  Widget photoSection() => PhotoSection(
    entries: photos,
    max: maxEvidencePhotos,
    onGuide: () => showPhotoGuide(context),
    onCamera: () => addPhoto(
      () => ref
          .read(photoSourceProvider)
          .fromCamera(context, shotIndex: photos.length),
    ),
    onGallery: () => addPhoto(ref.read(photoSourceProvider).fromGallery),
    onRemove: (i) => setState(() => photos.removeAt(i)),
    onRetry: (i) => uploadPhoto(photos[i]),
  );
}

/// Report a problem with an order.
class ReportPage extends ConsumerStatefulWidget {
  const ReportPage({super.key, required this.orderId});

  final String orderId;

  @override
  ConsumerState<ReportPage> createState() => _ReportPageState();
}

class _ReportPageState extends ConsumerState<ReportPage> with PhotoPicking {
  final _text = TextEditingController();
  DisputeReason _reason = DisputeReason.notReceived;
  bool _busy = false;
  String? _error;

  @override
  String get photoPurpose => 'dispute';

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    if (_busy || uploading || _text.text.trim().isEmpty) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final d = await ref
          .read(buyOrdersRepositoryProvider)
          .openDispute(
            widget.orderId,
            reason: _reason,
            body: _text.text.trim(),
            photoKeys: photoKeys,
          );
      if (!mounted) return;
      await Navigator.of(context).pushReplacement<void, void>(
        MaterialPageRoute(builder: (_) => DisputeStatusPage(disputeId: d.id)),
      );
    } on Object catch (e) {
      if (mounted) setState(() => _error = buyError(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: AppBar(
        title: const Text(BuyStrings.reportTitle),
        backgroundColor: context.tokens.bg,
        surfaceTintColor: const Color(0x00000000),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(Tokens.s5),
          children: [
            DropdownButtonFormField<DisputeReason>(
              isExpanded: true,
              initialValue: _reason,
              decoration: const InputDecoration(
                labelText: BuyStrings.reasonLabel,
              ),
              items: [
                for (final r in DisputeReason.values)
                  DropdownMenuItem(
                    value: r,
                    child: Text(BuyStrings.reason(r.wire)),
                  ),
              ],
              onChanged: (r) => setState(() => _reason = r ?? _reason),
            ),
            const SizedBox(height: Tokens.s3),
            AppTextField(
              label: BuyStrings.statementLabel,
              controller: _text,
              maxLines: 5,
              maxLength: 2000,
              capitalization: TextCapitalization.sentences,
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: Tokens.s3),
            Text(BuyStrings.reportPhotos, style: theme.textTheme.titleSmall),
            photoSection(),
            if (_error != null) ...[
              const SizedBox(height: Tokens.s3),
              ErrorLine(_error!),
            ],
            const SizedBox(height: Tokens.s4),
            LoadingButton(
              label: BuyStrings.sendReport,
              loading: _busy,
              onPressed: _text.text.trim().isEmpty || uploading ? null : _send,
            ),
          ],
        ),
      ),
    );
  }
}

final buyDisputeProvider = FutureProvider.autoDispose
    .family<BuyDispute, String>(
      (ref, id) => ref.watch(buyOrdersRepositoryProvider).dispute(id),
    );

/// Where a report stands, and withdrawing it while it is open.
class DisputeStatusPage extends ConsumerStatefulWidget {
  const DisputeStatusPage({super.key, required this.disputeId});

  final String disputeId;

  @override
  ConsumerState<DisputeStatusPage> createState() => _DisputeStatusPageState();
}

class _DisputeStatusPageState extends ConsumerState<DisputeStatusPage> {
  bool _busy = false;
  String? _error;

  Future<void> _withdraw() async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text(BuyStrings.withdraw),
        content: const Text(BuyStrings.withdrawBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text(BuyStrings.keep),
          ),
          FilledButton(
            style: FilledButton.styleFrom(
              minimumSize: const Size(0, Tokens.minTap),
            ),
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text(BuyStrings.confirm),
          ),
        ],
      ),
    );
    if (sure != true || _busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await ref.read(buyOrdersRepositoryProvider).withdraw(widget.disputeId);
      ref.invalidate(buyDisputeProvider(widget.disputeId));
    } on Object catch (e) {
      if (mounted) setState(() => _error = buyError(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final d = ref.watch(buyDisputeProvider(widget.disputeId));
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: AppBar(
        title: const Text(BuyStrings.reportStatusTitle),
        backgroundColor: context.tokens.bg,
        surfaceTintColor: const Color(0x00000000),
      ),
      body: SafeArea(
        child: d.when(
          loading: () => const Center(child: CircularProgressIndicator()),
          error: (e, _) => ErrorState(
            message: buyError(e),
            onRetry: () => ref.invalidate(buyDisputeProvider(widget.disputeId)),
          ),
          data: (x) => ListView(
            padding: const EdgeInsets.all(Tokens.s5),
            children: [
              Text(switch (x.status) {
                'awaiting_seller' => BuyStrings.awaitingSeller,
                'awaiting_return' => BuyStrings.awaitingReturn,
                'awaiting_decision' => BuyStrings.awaitingDecision,
                'withdrawn' => BuyStrings.withdrawn,
                _ => BuyStrings.decided,
              }, style: theme.textTheme.titleMedium),
              if (x.sellerAnswer != null) ...[
                const SizedBox(height: Tokens.s3),
                Text(
                  BuyStrings.sellerAnswered,
                  style: theme.textTheme.labelLarge,
                ),
                Text(x.sellerAnswer!, style: theme.textTheme.bodyLarge),
              ],
              if (x.refundKobo != null) ...[
                const SizedBox(height: Tokens.s3),
                Text(
                  BuyStrings.refunded(formatNaira(x.refundKobo!)),
                  style: theme.textTheme.bodyLarge,
                ),
              ],
              if (_error != null) ...[
                const SizedBox(height: Tokens.s3),
                ErrorLine(_error!),
              ],
              if (x.awaitingSeller) ...[
                const SizedBox(height: Tokens.s5),
                OutlinedButton(
                  onPressed: _busy ? null : _withdraw,
                  child: const Text(BuyStrings.withdraw),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// Stars and words about an order, with optional photos.
class ReviewPage extends ConsumerStatefulWidget {
  const ReviewPage({super.key, required this.orderId});

  final String orderId;

  @override
  ConsumerState<ReviewPage> createState() => _ReviewPageState();
}

class _ReviewPageState extends ConsumerState<ReviewPage> with PhotoPicking {
  final _text = TextEditingController();
  int _stars = 0;
  bool _busy = false;
  String? _error;

  @override
  String get photoPurpose => 'review';

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    if (_stars == 0 || _busy || uploading) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await ref
          .read(buyOrdersRepositoryProvider)
          .review(
            widget.orderId,
            stars: _stars,
            body: _text.text.trim().isEmpty ? null : _text.text.trim(),
            photoKeys: photoKeys,
          );
      if (!mounted) return;
      ScaffoldMessenger.of(context)
          .showSnackBar(const SnackBar(content: Text(BuyStrings.reviewSent)));
      Navigator.of(context).pop();
    } on Object catch (e) {
      if (mounted) setState(() => _error = buyError(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: AppBar(
        title: const Text(BuyStrings.reviewScreen),
        backgroundColor: tokens.bg,
        surfaceTintColor: const Color(0x00000000),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(Tokens.s5),
          children: [
            Text(BuyStrings.starsLabel, style: theme.textTheme.titleMedium),
            const SizedBox(height: Tokens.s2),
            Row(
              children: [
                for (var i = 1; i <= 5; i++)
                  Pressable(
                    haptic: true,
                    semanticLabel: '$i',
                    onTap: () => setState(() => _stars = i),
                    child: SizedBox(
                      width: Tokens.minTap,
                      height: Tokens.minTap,
                      child: Icon(
                        i <= _stars ? Icons.star : Icons.star_border,
                        size: 32,
                        color: tokens.gold,
                      ),
                    ),
                  ),
              ],
            ),
            const SizedBox(height: Tokens.s3),
            AppTextField(
              label: BuyStrings.reviewText,
              controller: _text,
              maxLines: 4,
              maxLength: 1000,
              capitalization: TextCapitalization.sentences,
            ),
            const SizedBox(height: Tokens.s3),
            photoSection(),
            if (_error != null) ...[
              const SizedBox(height: Tokens.s3),
              ErrorLine(_error!),
            ],
            const SizedBox(height: Tokens.s4),
            LoadingButton(
              label: BuyStrings.sendReview,
              loading: _busy,
              onPressed: _stars == 0 || uploading ? null : _send,
            ),
          ],
        ),
      ),
    );
  }
}
