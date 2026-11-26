import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import 'order_models.dart';
import 'sell_models.dart';
import 'sell_repository.dart';
import 'sell_strings.dart';
import 'sell_widgets.dart';

final reviewsProvider = FutureProvider.autoDispose
    .family<List<SellReview>, String>(
      (ref, slug) => ref.watch(workRepositoryProvider).reviews(slug),
    );

/// The store's reviews, with a reply box for owners and managers.
class ReviewsPage extends ConsumerWidget {
  const ReviewsPage({super.key, required this.store});

  final SellStore store;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reviews = ref.watch(reviewsProvider(store.slug));
    final theme = Theme.of(context);
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: sellAppBar(context, SellStrings.reviewsTitle),
      body: AsyncBody<List<SellReview>>(
        value: reviews,
        onRetry: () => ref.invalidate(reviewsProvider(store.slug)),
        builder: (items) => items.isEmpty
            ? const EmptyState(title: SellStrings.noReviews)
            : ListView.separated(
                padding: const EdgeInsets.all(Tokens.s5),
                itemCount: items.length,
                separatorBuilder: (_, _) => const Divider(height: Tokens.s5),
                itemBuilder: (context, i) {
                  final r = items[i];
                  return Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Semantics(
                        label: '${r.stars} of 5',
                        child: Text(
                          '★' * r.stars + '☆' * (5 - r.stars),
                          style: theme.textTheme.titleMedium,
                        ),
                      ),
                      if (r.body != null) ...[
                        const SizedBox(height: Tokens.s1),
                        Text(r.body!, style: theme.textTheme.bodyLarge),
                      ],
                      if (r.reply != null) ...[
                        const SizedBox(height: Tokens.s2),
                        Text(
                          SellStrings.replied,
                          style: theme.textTheme.labelLarge,
                        ),
                        Text(r.reply!, style: theme.textTheme.bodyMedium),
                      ] else if (store.role.isManagerUp)
                        Align(
                          alignment: Alignment.centerLeft,
                          child: TextButton(
                            onPressed: () async {
                              final sent = await showModalBottomSheet<bool>(
                                context: context,
                                isScrollControlled: true,
                                showDragHandle: true,
                                builder: (_) => _ReplySheet(review: r),
                              );
                              if (sent == true) {
                                ref.invalidate(reviewsProvider(store.slug));
                              }
                            },
                            child: const Text(SellStrings.reply),
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

class _ReplySheet extends ConsumerStatefulWidget {
  const _ReplySheet({required this.review});

  final SellReview review;

  @override
  ConsumerState<_ReplySheet> createState() => _ReplySheetState();
}

class _ReplySheetState extends ConsumerState<_ReplySheet> with BusyAction {
  final _text = TextEditingController();

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Padding(
    padding: EdgeInsets.fromLTRB(
      Tokens.s5,
      0,
      Tokens.s5,
      Tokens.s5 + MediaQuery.viewInsetsOf(context).bottom,
    ),
    child: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        AppTextField(
          label: SellStrings.replyLabel,
          controller: _text,
          maxLines: 4,
          maxLength: 1000,
          capitalization: TextCapitalization.sentences,
          onChanged: (_) => setState(() {}),
        ),
        if (error != null) ...[
          const SizedBox(height: Tokens.s2),
          ErrorLine(error!),
        ],
        const SizedBox(height: Tokens.s3),
        LoadingButton(
          label: SellStrings.sendReply,
          loading: busy,
          onPressed: _text.text.trim().isEmpty
              ? null
              : () async {
                  final ok = await run(
                    () => ref
                        .read(workRepositoryProvider)
                        .replyToReview(widget.review.id, _text.text.trim()),
                  );
                  if (ok && context.mounted) Navigator.of(context).pop(true);
                },
        ),
      ],
    ),
  );
}
