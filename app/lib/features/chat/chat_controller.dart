import 'dart:async';
import 'dart:math';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../../core/api/api_error.dart';
import 'chat_models.dart';
import 'chat_repository.dart';

/// A random version 4 UUID: the server wants one as the `client_message_id`.
String newClientMessageId([Random? random]) {
  final r = random ?? Random.secure();
  final b = List<int>.generate(16, (_) => r.nextInt(256));
  b[6] = (b[6] & 0x0F) | 0x40;
  b[8] = (b[8] & 0x3F) | 0x80;
  String h(int from, int to) =>
      [for (var i = from; i < to; i++) b[i].toRadixString(16).padLeft(2, '0')]
          .join();
  return '${h(0, 4)}-${h(4, 6)}-${h(6, 8)}-${h(8, 10)}-${h(10, 16)}';
}

enum LocalStatus {
  /// On its way to the server.
  sending,

  /// Not sent: the network failed or the server asked to slow down. Retrying sends
  /// the same message.
  failed,

  /// The server is checking it.
  held,

  /// Stopped. The reason is in plain words; the sender may edit and send again.
  blocked,

  /// Stopped, with a rewritten text the sender may send instead.
  rewrite,
}

/// One of the person's own messages the other side has not seen. The text is personal:
/// it lives in memory only and is never printed.
class LocalMessage {
  LocalMessage({
    required this.clientId,
    required this.text,
    required this.status,
    required this.createdAt,
    this.reason,
    this.flagId,
    this.suggested,
    this.slowDown = false,
  });

  final String clientId;
  final String text;
  final LocalStatus status;
  final DateTime createdAt;
  final String? reason;
  final String? flagId;
  final String? suggested;
  final bool slowDown;

  LocalMessage copyWith({
    String? text,
    LocalStatus? status,
    String? reason,
    String? flagId,
    String? suggested,
    bool? slowDown,
  }) => LocalMessage(
    clientId: clientId,
    text: text ?? this.text,
    status: status ?? this.status,
    createdAt: createdAt,
    reason: reason ?? this.reason,
    flagId: flagId ?? this.flagId,
    suggested: suggested ?? this.suggested,
    slowDown: slowDown ?? this.slowDown,
  );

  @override
  String toString() => 'LocalMessage($status)';
}

class ConvState {
  const ConvState({
    this.messages = const [],
    this.locals = const [],
    this.mutedUntil,
    this.strikes,
    this.identityRequired = false,
    this.closed = false,
  });

  final List<ChatMessage> messages;
  final List<LocalMessage> locals;
  final DateTime? mutedUntil;
  final int? strikes;

  /// A manager or staff member who has not passed the identity check.
  final bool identityRequired;

  /// The conversation can no longer be written to (the store closed, or an account went).
  final bool closed;

  int get lastSeq => messages.isEmpty ? 0 : messages.last.seq;

  bool mutedAt(DateTime now) => mutedUntil != null && mutedUntil!.isAfter(now);

  ConvState copyWith({
    List<ChatMessage>? messages,
    List<LocalMessage>? locals,
    DateTime? mutedUntil,
    bool clearMuted = false,
    int? strikes,
    bool? identityRequired,
    bool? closed,
  }) => ConvState(
    messages: messages ?? this.messages,
    locals: locals ?? this.locals,
    mutedUntil: clearMuted ? null : (mutedUntil ?? this.mutedUntil),
    strikes: strikes ?? this.strikes,
    identityRequired: identityRequired ?? this.identityRequired,
    closed: closed ?? this.closed,
  );
}

/// Merges delivered messages in by `seq`: a message already held is not added twice,
/// and the list stays in order whatever order frames and history arrive in.
List<ChatMessage> mergeMessages(
  List<ChatMessage> have,
  Iterable<ChatMessage> incoming,
) {
  final bySeq = {for (final m in have) m.seq: m};
  for (final m in incoming) {
    bySeq[m.seq] = m;
  }
  return [for (final k in (bySeq.keys.toList()..sort())) bySeq[k]!];
}

/// How often the open conversation asks for news while the live link is down.
final chatPollEveryProvider = Provider<Duration>(
  (ref) => const Duration(seconds: 5),
);

/// The conversation on screen: history, live frames, the person's own held and
/// stopped messages, and sending. The server's outcome is the only word on what
/// was sent.
class ConversationController extends AsyncNotifier<ConvState> {
  ConversationController(this.conversationId);

  final String conversationId;
  int _readSent = 0;
  Timer? _poll;

  ChatRepository get _repo => ref.read(chatRepositoryProvider);

  @override
  Future<ConvState> build() async {
    ref.onDispose(() => _poll?.cancel());
    ref.listen(chatFramesProvider, (_, next) {
      final frame = next.value;
      if (frame != null && frame.conversationId == conversationId) {
        _onFrame(frame);
      }
    });
    // While the live link is down, ask for news now and then.
    ref.listen(chatConnectedProvider, (_, up) => _arm(up));
    _arm(ref.read(chatConnectedProvider));
    final first = await _repo.history(conversationId);
    var messages = mergeMessages(const [], first.messages);
    // The history comes in pages of 100, oldest first.
    var page = first;
    for (var i = 0; i < 9 && page.messages.length >= 100; i++) {
      page = await _repo.history(conversationId, after: messages.last.seq);
      messages = mergeMessages(messages, page.messages);
    }
    return ConvState(
      messages: messages,
      locals: _fromPending(const [], first.pending),
    );
  }

  void _arm(bool connected) {
    _poll?.cancel();
    if (connected) return;
    _poll = Timer.periodic(ref.read(chatPollEveryProvider), (_) => catchUp());
  }

  /// The server's own held and stopped messages, keeping the screen's local ones that
  /// the server does not know yet (still sending, or failed).
  List<LocalMessage> _fromPending(
    List<LocalMessage> locals,
    List<OwnPending> pending,
  ) {
    final known = {for (final p in pending) p.clientMessageId};
    final out = <LocalMessage>[
      for (final l in locals)
        if (!known.contains(l.clientId) &&
            (l.status == LocalStatus.sending || l.status == LocalStatus.failed))
          l,
    ];
    for (final p in pending) {
      final prior = locals
          .where((l) => l.clientId == p.clientMessageId)
          .firstOrNull;
      final text = p.body ?? prior?.text ?? '';
      if (p.status == 'held') {
        out.add(
          LocalMessage(
            clientId: p.clientMessageId,
            text: text,
            status: LocalStatus.held,
            createdAt: p.createdAt,
          ),
        );
      } else {
        out.add(
          LocalMessage(
            clientId: p.clientMessageId,
            text: text,
            status: p.suggested != null
                ? LocalStatus.rewrite
                : LocalStatus.blocked,
            createdAt: p.createdAt,
            reason: p.reason,
            flagId: p.flagId,
            suggested: p.suggested,
          ),
        );
      }
    }
    out.sort((a, b) => a.createdAt.compareTo(b.createdAt));
    return out;
  }

  void _onFrame(ChatFrame frame) {
    final s = state.value;
    if (s == null) return;
    switch (frame) {
      case MessageFrame(:final message):
        state = AsyncData(
          s.copyWith(messages: mergeMessages(s.messages, [message])),
        );
        // One of ours that was being checked has now gone through, or a reply came.
        unawaited(catchUp());
      case ModerationFrame():
        unawaited(catchUp());
    }
  }

  /// Fetches what came after the last message, and the person's own pending ones.
  Future<void> catchUp() async {
    final s = state.value;
    if (s == null) return;
    try {
      final h = await _repo.history(conversationId, after: s.lastSeq);
      final now = state.value ?? s;
      state = AsyncData(
        now.copyWith(
          messages: mergeMessages(now.messages, h.messages),
          locals: _fromPending(now.locals, h.pending),
        ),
      );
    } on Object {
      // Try again on the next frame or tick.
    }
  }

  /// Tells the server how far the person has read, once per new message.
  Future<void> markRead() async {
    final s = state.value;
    if (s == null || s.lastSeq <= _readSent) {
      return;
    }
    _readSent = s.lastSeq;
    try {
      await _repo.markRead(conversationId, s.lastSeq);
    } on Object {
      _readSent = 0;
    }
  }

  LocalMessage _local(String text, {String? id}) => LocalMessage(
    clientId: id ?? newClientMessageId(),
    text: text,
    status: LocalStatus.sending,
    createdAt: DateTime.now(),
  );

  /// Sends [text] as a new message.
  Future<void> send(String text) => _run(_local(text.trim()), isNew: true);

  /// Sends a failed message again: the same message, the same id.
  Future<void> retry(String clientId) {
    final l = state.value?.locals
        .where((x) => x.clientId == clientId)
        .firstOrNull;
    return l == null
        ? Future.value()
        : _run(
            l.copyWith(status: LocalStatus.sending, slowDown: false),
            isNew: false,
          );
  }

  /// Sends the rewrite the server offered, by its id.
  Future<void> useSuggestion(String clientId) async {
    final l = state.value?.locals
        .where((x) => x.clientId == clientId)
        .firstOrNull;
    final flag = l?.flagId;
    if (l == null || flag == null) return;
    // A fresh message id: this is a new send of the stored text.
    final next = _local(l.suggested ?? '');
    _replace(l.clientId, next);
    await _finish(
      next,
      () => _repo.sendSuggestion(
        conversationId,
        flag,
        clientMessageId: next.clientId,
      ),
    );
  }

  /// Drops a stopped message from the screen (the person edits it in the composer).
  void dismiss(String clientId) {
    final s = state.value;
    if (s == null) return;
    state = AsyncData(
      s.copyWith(
        locals: [
          for (final l in s.locals)
            if (l.clientId != clientId) l,
        ],
      ),
    );
  }

  Future<void> _run(LocalMessage l, {required bool isNew}) async {
    if (l.text.isEmpty) return;
    if (isNew) {
      _add(l);
    } else {
      _replace(l.clientId, l);
    }
    await _finish(
      l,
      () =>
          _repo.send(conversationId, clientMessageId: l.clientId, body: l.text),
    );
  }

  void _add(LocalMessage l) {
    final s = state.value;
    if (s != null) state = AsyncData(s.copyWith(locals: [...s.locals, l]));
  }

  void _replace(String clientId, LocalMessage next) {
    final s = state.value;
    if (s == null) return;
    state = AsyncData(
      s.copyWith(
        locals: [for (final l in s.locals) l.clientId == clientId ? next : l],
      ),
    );
  }

  Future<void> _finish(
    LocalMessage l,
    Future<SendResult> Function() call,
  ) async {
    try {
      final r = await call();
      final s = state.value;
      if (s == null) return;
      switch (r.outcome) {
        case Outcome.delivered:
          final m = r.message;
          state = AsyncData(
            s.copyWith(
              messages: m == null ? s.messages : mergeMessages(s.messages, [m]),
              locals: [
                for (final x in s.locals)
                  if (x.clientId != l.clientId) x,
              ],
              strikes: r.strikes,
            ),
          );
        case Outcome.held:
          _replace(l.clientId, l.copyWith(status: LocalStatus.held));
        case Outcome.blocked:
          _replace(
            l.clientId,
            l.copyWith(status: LocalStatus.blocked, reason: r.reason),
          );
          _note(r);
        case Outcome.rewrite:
          _replace(
            l.clientId,
            l.copyWith(
              status: LocalStatus.rewrite,
              reason: r.reason,
              flagId: r.flagId,
              suggested: r.suggested,
            ),
          );
          _note(r);
      }
    } on ApiError catch (e) {
      _failed(l, e);
    } on Object {
      _replace(l.clientId, l.copyWith(status: LocalStatus.failed));
    }
  }

  void _note(SendResult r) {
    final s = state.value;
    if (s == null) return;
    state = AsyncData(s.copyWith(strikes: r.strikes, mutedUntil: r.mutedUntil));
  }

  void _failed(LocalMessage l, ApiError e) {
    final s = state.value;
    if (s == null) return;
    final without = [
      for (final x in s.locals)
        if (x.clientId != l.clientId) x,
    ];
    if (e.status == 403 && e.code == 'muted') {
      state = AsyncData(s.copyWith(locals: without, mutedUntil: e.mutedUntil));
    } else if (e.status == 403 && e.code == 'identity_required') {
      state = AsyncData(s.copyWith(locals: without, identityRequired: true));
    } else if (e.code == 'chat_closed') {
      state = AsyncData(s.copyWith(locals: without, closed: true));
    } else if (e.code == 'chat_rate_limited') {
      _replace(
        l.clientId,
        l.copyWith(status: LocalStatus.failed, slowDown: true),
      );
    } else {
      _replace(l.clientId, l.copyWith(status: LocalStatus.failed));
    }
  }
}

final conversationProvider = AsyncNotifierProvider.autoDispose
    .family<ConversationController, ConvState, String>(
      (id) => ConversationController(id),
    );
