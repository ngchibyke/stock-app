import 'dart:async';

import 'package:flutter/services.dart' show LengthLimitingTextInputFormatter;
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/theme/tokens.dart';
import '../../core/ui/pressable.dart';
import '../../core/ui/states.dart';
import '../sell/sell_models.dart';
import '../sell/sell_providers.dart';
import '../sell/sell_shell.dart' show confirmIdentity;
import '../sell/sell_strings.dart';
import 'chat_controller.dart';
import 'chat_models.dart';
import 'chat_repository.dart';
import 'chat_strings.dart';

/// The server's cap on one message.
const int maxChatChars = 1000;

Future<T?> _push<T>(BuildContext context, Widget page) =>
    Navigator.of(context).push<T>(MaterialPageRoute<T>(builder: (_) => page));

String _clock(DateTime t) {
  final l = t.toLocal();
  return '${l.hour.toString().padLeft(2, '0')}:${l.minute.toString().padLeft(2, '0')}';
}

/// The time, with the day when that is not today.
String _until(DateTime t, DateTime now) {
  final l = t.toLocal();
  final today = l.year == now.year && l.month == now.month && l.day == now.day;
  return today ? _clock(t) : ChatStrings.timeOnDay(_clock(t), l.day, l.month);
}

/// Opens the chat about a product (as a buyer) and shows it.
Future<void> openProductChat(
  BuildContext context,
  WidgetRef ref,
  String productId,
) async {
  try {
    final c = await ref.read(chatRepositoryProvider).openProduct(productId);
    if (context.mounted) {
      await _push<void>(context, ConversationPage(conversation: c));
    }
  } on Object catch (e) {
    if (context.mounted) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text(chatError(e))));
    }
  }
}

/// Opens the chat about a paid order and shows it.
Future<void> openOrderChat(
  BuildContext context,
  WidgetRef ref,
  String orderId,
) async {
  try {
    final c = await ref.read(chatRepositoryProvider).openOrder(orderId);
    if (context.mounted) {
      await _push<void>(context, ConversationPage(conversation: c));
    }
  } on Object catch (e) {
    if (context.mounted) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text(chatError(e))));
    }
  }
}

/// A conversation, found by its id (a tapped notice carries only that).
class ConversationByIdPage extends ConsumerStatefulWidget {
  const ConversationByIdPage({super.key, required this.id});

  final String id;

  @override
  ConsumerState<ConversationByIdPage> createState() => _ByIdState();
}

class _ByIdState extends ConsumerState<ConversationByIdPage> {
  Conversation? _found;
  bool _missing = false;

  @override
  void initState() {
    super.initState();
    unawaited(_find());
  }

  Future<void> _find() async {
    try {
      String? cursor;
      for (var i = 0; i < 4; i++) {
        final page = await ref
            .read(chatRepositoryProvider)
            .conversations(before: cursor);
        final hit = page.items.where((c) => c.id == widget.id).firstOrNull;
        if (hit != null) {
          if (mounted) setState(() => _found = hit);
          return;
        }
        cursor = page.nextCursor;
        if (cursor == null) break;
      }
    } on Object {
      // Falls through to "not available".
    }
    if (mounted) setState(() => _missing = true);
  }

  @override
  Widget build(BuildContext context) {
    final c = _found;
    if (c != null) return ConversationPage(conversation: c);
    return Scaffold(
      backgroundColor: context.tokens.bg,
      appBar: AppBar(
        backgroundColor: context.tokens.bg,
        surfaceTintColor: const Color(0x00000000),
      ),
      body: _missing
          ? const ErrorState(message: ChatStrings.notFound)
          : const Center(child: CircularProgressIndicator()),
    );
  }
}

/// The conversation: history, live messages, the person's own held and stopped ones,
/// and the composer.
class ConversationPage extends ConsumerStatefulWidget {
  const ConversationPage({super.key, required this.conversation});

  final Conversation conversation;

  @override
  ConsumerState<ConversationPage> createState() => _ConversationPageState();
}

class _ConversationPageState extends ConsumerState<ConversationPage>
    with WidgetsBindingObserver {
  final _text = TextEditingController();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _text.dispose();
    super.dispose();
  }

  String get _id => widget.conversation.id;

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    // Back in front: pick up anything that arrived while the socket was away.
    if (state == AppLifecycleState.resumed) {
      unawaited(ref.read(conversationProvider(_id).notifier).catchUp());
    }
  }

  void _send() {
    final text = _text.text.trim();
    if (text.isEmpty) return;
    _text.clear();
    setState(() {});
    unawaited(ref.read(conversationProvider(_id).notifier).send(text));
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.conversation;
    final tokens = context.tokens;
    final theme = Theme.of(context);
    final conv = ref.watch(conversationProvider(_id));
    final connected = ref.watch(chatConnectedProvider);
    ref.listen(conversationProvider(_id), (prev, next) {
      final s = next.value;
      if (s != null && s.lastSeq != (prev?.value?.lastSeq ?? -1)) {
        unawaited(ref.read(conversationProvider(_id).notifier).markRead());
      }
    });
    final about = c.aboutOrder
        ? ChatStrings.aboutOrder
        : ChatStrings.aboutProduct;
    return Scaffold(
      backgroundColor: tokens.bg,
      appBar: AppBar(
        backgroundColor: tokens.bg,
        surfaceTintColor: const Color(0x00000000),
        title: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(c.with_, style: theme.textTheme.titleMedium),
            Text(
              [
                about,
                if (c.branchName != null) ChatStrings.atBranch(c.branchName!),
              ].join('  ·  '),
              style: theme.textTheme.bodySmall?.copyWith(color: tokens.mute),
            ),
          ],
        ),
      ),
      body: SafeArea(
        child: conv.when(
          loading: () => const Center(child: CircularProgressIndicator()),
          error: (e, _) => ErrorState(
            message: chatError(e),
            onRetry: () => ref.invalidate(conversationProvider(_id)),
          ),
          data: (s) => Column(
            children: [
              if (!connected)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: Tokens.s1),
                  child: Text(
                    ChatStrings.reconnecting,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: tokens.mute,
                    ),
                  ),
                ),
              Expanded(child: _list(s)),
              _composer(s),
            ],
          ),
        ),
      ),
    );
  }

  Widget _local(LocalMessage l) => _LocalBubble(
    local: l,
    onRetry: () =>
        ref.read(conversationProvider(_id).notifier).retry(l.clientId),
    onEdit: () {
      _text.text = l.text;
      setState(() {});
      ref.read(conversationProvider(_id).notifier).dismiss(l.clientId);
    },
    onUseSuggestion: () =>
        ref.read(conversationProvider(_id).notifier).useSuggestion(l.clientId),
  );

  Widget _list(ConvState s) {
    final c = widget.conversation;
    // A stopped message loaded again has no text left: it is history, and goes where
    // it happened. Everything else of one's own is still in hand, at the bottom.
    final past = [
      for (final l in s.locals)
        if (l.text.isEmpty) l,
    ];
    var next = 0;
    final items = <Widget>[];
    for (final m in s.messages) {
      while (next < past.length && past[next].createdAt.isBefore(m.sentAt)) {
        items.add(_local(past[next++]));
      }
      items.add(_Bubble(message: m, mine: m.from == c.you));
    }
    items.addAll(past.skip(next).map(_local));
    items.addAll(s.locals.where((l) => l.text.isNotEmpty).map(_local));
    // Newest at the bottom, and the list sticks there.
    return ListView(
      reverse: true,
      padding: const EdgeInsets.all(Tokens.s4),
      children: items.reversed.toList(),
    );
  }

  /// A manager or staff member who has not passed the identity check, as far as the
  /// app can tell. The server refuses the send either way.
  bool _unverifiedMember() {
    final c = widget.conversation;
    if (c.you != Side.seller) return false;
    final store = ref
        .watch(myStoresProvider)
        .value
        ?.where((s) => s.id == c.storeId)
        .firstOrNull;
    if (store == null || store.role.isOwner) return false;
    final status = ref.watch(identityStatusProvider).value;
    return status != null && status.state != IdentityState.passed;
  }

  Widget _composer(ConvState s) {
    final tokens = context.tokens;
    final theme = Theme.of(context);
    final now = DateTime.now();
    final unverified = _unverifiedMember();
    String? blocked;
    var needsIdentity = false;
    if (!widget.conversation.writable || s.closed) {
      blocked = ChatStrings.closed;
    } else if (s.identityRequired || unverified) {
      blocked = ChatStrings.identityNeeded;
      needsIdentity = true;
    } else if (s.mutedAt(now)) {
      blocked = ChatStrings.mutedUntil(_until(s.mutedUntil!, now));
    }
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tokens.card,
        border: Border(top: BorderSide(color: tokens.line)),
      ),
      child: Padding(
        padding: const EdgeInsets.all(Tokens.s3),
        child: blocked != null
            ? Semantics(
                liveRegion: true,
                child: Padding(
                  padding: const EdgeInsets.all(Tokens.s2),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(blocked, style: theme.textTheme.bodyMedium),
                      if (needsIdentity)
                        TextButton(
                          onPressed: () async {
                            await confirmIdentity(context, ref);
                            if (mounted) {
                              ref.invalidate(conversationProvider(_id));
                            }
                          },
                          child: const Text(SellStrings.identityBannerAction),
                        ),
                    ],
                  ),
                ),
              )
            : Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (s.strikes != null && s.strikes! > 0)
                    Padding(
                      padding: const EdgeInsets.only(bottom: Tokens.s1),
                      child: Text(
                        ChatStrings.strikes(s.strikes!),
                        style: theme.textTheme.bodySmall?.copyWith(
                          color: tokens.errorText,
                        ),
                      ),
                    ),
                  Text(
                    ChatStrings.keepInApp,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: tokens.mute,
                    ),
                  ),
                  const SizedBox(height: Tokens.s1),
                  Row(
                    crossAxisAlignment: CrossAxisAlignment.end,
                    children: [
                      Expanded(
                        child: TextField(
                          controller: _text,
                          minLines: 1,
                          maxLines: 4,
                          textCapitalization: TextCapitalization.sentences,
                          inputFormatters: [
                            LengthLimitingTextInputFormatter(maxChatChars),
                          ],
                          decoration: const InputDecoration(
                            hintText: ChatStrings.composerHint,
                          ),
                          onChanged: (_) => setState(() {}),
                        ),
                      ),
                      const SizedBox(width: Tokens.s2),
                      Pressable(
                        haptic: true,
                        semanticLabel: ChatStrings.send,
                        onTap: _text.text.trim().isEmpty ? null : _send,
                        child: SizedBox(
                          width: Tokens.minTap,
                          height: Tokens.minTap,
                          child: Icon(
                            Icons.send,
                            color: _text.text.trim().isEmpty
                                ? tokens.mute
                                : tokens.accent,
                          ),
                        ),
                      ),
                    ],
                  ),
                ],
              ),
      ),
    );
  }
}

class _Bubble extends StatelessWidget {
  const _Bubble({required this.message, required this.mine});

  final ChatMessage message;
  final bool mine;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final theme = Theme.of(context);
    return Align(
      alignment: mine ? Alignment.centerRight : Alignment.centerLeft,
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: MediaQuery.sizeOf(context).width * 0.78,
        ),
        child: Padding(
          padding: const EdgeInsets.only(bottom: Tokens.s2),
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: mine ? tokens.accent.withValues(alpha: 0.14) : tokens.card,
              border: Border.all(color: tokens.line),
              borderRadius: BorderRadius.circular(Tokens.radius),
            ),
            child: Padding(
              padding: const EdgeInsets.all(Tokens.s3),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (message.by != null)
                    Text(
                      ChatStrings.writtenBy(message.by!),
                      style: theme.textTheme.labelSmall?.copyWith(
                        color: tokens.mute,
                      ),
                    ),
                  Text(message.body, style: theme.textTheme.bodyLarge),
                  Align(
                    alignment: Alignment.centerRight,
                    child: Text(
                      _clock(message.sentAt),
                      style: theme.textTheme.labelSmall?.copyWith(
                        color: tokens.mute,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _LocalBubble extends StatelessWidget {
  const _LocalBubble({
    required this.local,
    required this.onRetry,
    required this.onEdit,
    required this.onUseSuggestion,
  });

  final LocalMessage local;
  final VoidCallback onRetry;
  final VoidCallback onEdit;
  final VoidCallback onUseSuggestion;

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final theme = Theme.of(context);
    final note = switch (local.status) {
      LocalStatus.sending => ChatStrings.sending,
      LocalStatus.held => ChatStrings.checking,
      LocalStatus.failed =>
        local.slowDown ? ChatStrings.slowDown : ChatStrings.notSent,
      LocalStatus.blocked || LocalStatus.rewrite => local.reason ?? '',
    };
    final bad =
        local.status == LocalStatus.failed ||
        local.status == LocalStatus.blocked ||
        local.status == LocalStatus.rewrite;
    return Align(
      alignment: Alignment.centerRight,
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: MediaQuery.sizeOf(context).width * 0.78,
        ),
        child: Padding(
          padding: const EdgeInsets.only(bottom: Tokens.s2),
          child: Pressable(
            onTap: local.status == LocalStatus.failed ? onRetry : null,
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: tokens.accent.withValues(alpha: 0.07),
                border: Border.all(color: bad ? tokens.errorText : tokens.line),
                borderRadius: BorderRadius.circular(Tokens.radius),
              ),
              child: Padding(
                padding: const EdgeInsets.all(Tokens.s3),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    if (local.text.isNotEmpty) ...[
                      Text(
                        local.text,
                        style: theme.textTheme.bodyLarge?.copyWith(
                          color: tokens.mute,
                        ),
                      ),
                      const SizedBox(height: Tokens.s1),
                    ],
                    Semantics(
                      liveRegion: true,
                      child: Text(
                        note,
                        style: theme.textTheme.bodySmall?.copyWith(
                          color: bad ? tokens.errorText : tokens.mute,
                        ),
                      ),
                    ),
                    if (local.status == LocalStatus.rewrite &&
                        local.suggested != null) ...[
                      const SizedBox(height: Tokens.s2),
                      Text(
                        ChatStrings.suggestionTitle,
                        style: theme.textTheme.labelMedium,
                      ),
                      Text(local.suggested!, style: theme.textTheme.bodyLarge),
                      const SizedBox(height: Tokens.s2),
                      Wrap(
                        spacing: Tokens.s2,
                        children: [
                          FilledButton(
                            style: FilledButton.styleFrom(
                              minimumSize: const Size(0, Tokens.minTap),
                            ),
                            onPressed: onUseSuggestion,
                            child: const Text(ChatStrings.sendInstead),
                          ),
                          if (local.text.isNotEmpty)
                            TextButton(
                              onPressed: onEdit,
                              child: const Text(ChatStrings.edit),
                            ),
                        ],
                      ),
                    ],
                    if (local.status == LocalStatus.blocked &&
                        local.text.isNotEmpty)
                      TextButton(
                        onPressed: onEdit,
                        child: const Text(ChatStrings.edit),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The list of conversations, newest first, paged. [storeId] limits it to one store
/// (the Sell area); [side] to the buyer's own or the store's side.
class ConversationList extends ConsumerStatefulWidget {
  const ConversationList({super.key, this.storeId, this.side, this.banner});

  final String? storeId;
  final Side? side;
  final Widget? banner;

  @override
  ConsumerState<ConversationList> createState() => _ConversationListState();
}

class _ConversationListState extends ConsumerState<ConversationList> {
  final List<Conversation> _items = [];
  String? _cursor;
  bool _loading = true;
  bool _more = false;
  String? _error;

  @override
  void initState() {
    super.initState();
    unawaited(_load(reset: true));
  }

  bool _keep(Conversation c) =>
      (widget.storeId == null || c.storeId == widget.storeId) &&
      (widget.side == null || c.you == widget.side);

  Future<void> _load({bool reset = false}) async {
    if (reset) {
      setState(() {
        _loading = _items.isEmpty;
        _error = null;
      });
    } else {
      setState(() => _more = true);
    }
    try {
      final page = await ref
          .read(chatRepositoryProvider)
          .conversations(before: reset ? null : _cursor);
      if (!mounted) return;
      setState(() {
        if (reset) _items.clear();
        _items.addAll(page.items.where(_keep));
        _cursor = page.nextCursor;
      });
    } on Object catch (e) {
      if (mounted) setState(() => _error = chatError(e));
    } finally {
      if (mounted) {
        setState(() {
          _loading = false;
          _more = false;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final tokens = context.tokens;
    final theme = Theme.of(context);
    // A new message anywhere refreshes the list.
    ref.listen(chatFramesProvider, (_, next) {
      if (next.value is MessageFrame) unawaited(_load(reset: true));
    });
    if (_loading) {
      return const Padding(
        padding: EdgeInsets.all(Tokens.s5),
        child: Skeleton(height: 72),
      );
    }
    if (_error != null && _items.isEmpty) {
      return ErrorState(message: _error!, onRetry: () => _load(reset: true));
    }
    return Column(
      children: [
        ?widget.banner,
        Expanded(
          child: RefreshIndicator(
            onRefresh: () => _load(reset: true),
            child: _items.isEmpty
                ? ListView(
                    children: const [
                      SizedBox(height: Tokens.s6),
                      EmptyState(
                        title: ChatStrings.noChats,
                        body: ChatStrings.noChatsBody,
                      ),
                    ],
                  )
                : ListView.separated(
                    itemCount: _items.length + (_cursor != null ? 1 : 0),
                    separatorBuilder: (_, _) => const Divider(height: 1),
                    itemBuilder: (context, i) {
                      if (i == _items.length) {
                        return Padding(
                          padding: const EdgeInsets.all(Tokens.s4),
                          child: OutlinedButton(
                            onPressed: _more ? null : () => _load(),
                            child: const Text(ChatStrings.loadMore),
                          ),
                        );
                      }
                      final c = _items[i];
                      return ListTile(
                        minTileHeight: Tokens.minTap + 8,
                        leading: c.unread > 0
                            ? Icon(Icons.circle, size: 10, color: tokens.accent)
                            : const SizedBox(width: 10),
                        title: Text(
                          c.with_,
                          style: c.unread > 0
                              ? theme.textTheme.titleSmall
                              : null,
                        ),
                        subtitle: Text(
                          [
                            c.aboutOrder
                                ? ChatStrings.aboutOrder
                                : ChatStrings.aboutProduct,
                            if (c.branchName != null)
                              ChatStrings.atBranch(c.branchName!),
                          ].join('  ·  '),
                        ),
                        trailing: Text(
                          _clock(c.lastMessageAt),
                          style: theme.textTheme.labelSmall,
                        ),
                        onTap: () async {
                          await _push<void>(
                            context,
                            ConversationPage(conversation: c),
                          );
                          unawaited(_load(reset: true));
                        },
                      );
                    },
                  ),
          ),
        ),
      ],
    );
  }
}

/// The buyer's Messages page.
class MessagesPage extends StatelessWidget {
  const MessagesPage({super.key});

  @override
  Widget build(BuildContext context) => Scaffold(
    backgroundColor: context.tokens.bg,
    appBar: AppBar(
      title: const Text(ChatStrings.messages),
      backgroundColor: context.tokens.bg,
      surfaceTintColor: const Color(0x00000000),
    ),
    body: const ConversationList(side: Side.buyer),
  );
}

/// The Chats tab of the Sell area: the conversations of the chosen store the person
/// may see.
class SellChatsTab extends StatelessWidget {
  const SellChatsTab({super.key, required this.storeId, this.banner});

  final String storeId;
  final Widget? banner;

  @override
  Widget build(BuildContext context) => Scaffold(
    backgroundColor: context.tokens.bg,
    appBar: AppBar(
      title: const Text(ChatStrings.chats),
      backgroundColor: context.tokens.bg,
      surfaceTintColor: const Color(0x00000000),
    ),
    body: ConversationList(storeId: storeId, side: Side.seller, banner: banner),
  );
}
