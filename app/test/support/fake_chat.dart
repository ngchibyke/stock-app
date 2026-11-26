import 'dart:async';

import 'package:market/features/chat/chat_models.dart';
import 'package:market/features/chat/chat_repository.dart';

export 'fake_sell.dart' show apiError;

Conversation fakeConversation({
  String id = 'c1',
  Side you = Side.buyer,
  String with_ = 'Ada Fashion',
  String storeId = 's1',
  String? orderId,
  String? productId = 'p1',
  String? branch,
  bool writable = true,
  int unread = 0,
}) => Conversation(
  id: id,
  with_: with_,
  you: you,
  storeId: storeId,
  storeName: 'Ada Fashion',
  orderId: orderId,
  productId: orderId == null ? productId : null,
  branchName: branch,
  writable: writable,
  unread: unread,
  lastMessageAt: DateTime(2026, 11, 26, 10),
);

ChatMessage fakeMessage(
  int seq, {
  Side from = Side.seller,
  String body = 'Hello',
  String? by,
  String conversation = 'c1',
}) => ChatMessage(
  id: 'm$seq',
  conversationId: conversation,
  seq: seq,
  from: from,
  body: body,
  by: by,
  sentAt: DateTime(2026, 11, 26, 10, seq),
);

class FakeChatRepository implements ChatRepository {
  List<Conversation> conversationRows = [];
  List<ChatMessage> messageRows = [];
  List<OwnPending> pendingRows = [];
  Object? failSend;
  Object? failHistory;

  /// What `send` answers, in order; the last repeats. Empty means "delivered".
  List<SendResult> results = [];

  final List<({String conv, String client, String body})> sent = [];
  final List<({String conv, String flag, String client})> suggestions = [];
  final List<int> reads = [];
  final List<int> historyAfter = [];
  int opened = 0;
  int historyCalls = 0;

  @override
  Future<Conversation> openProduct(String productId) async {
    opened++;
    return conversationRows.isNotEmpty
        ? conversationRows.first
        : fakeConversation();
  }

  @override
  Future<Conversation> openOrder(String orderId) async {
    opened++;
    return fakeConversation(orderId: orderId);
  }

  @override
  Future<ConversationPage> conversations({String? before}) async =>
      ConversationPage(items: [...conversationRows]);

  @override
  Future<History> history(String conversationId, {int after = 0}) async {
    historyCalls++;
    historyAfter.add(after);
    if (failHistory != null) throw failHistory!;
    return History(
      messages: [
        for (final m in messageRows)
          if (m.seq > after) m,
      ],
      pending: [...pendingRows],
    );
  }

  SendResult _next() {
    if (results.isEmpty) return const SendResult(outcome: Outcome.delivered);
    return results.length > 1 ? results.removeAt(0) : results.first;
  }

  @override
  Future<SendResult> send(
    String conversationId, {
    required String clientMessageId,
    required String body,
  }) async {
    sent.add((conv: conversationId, client: clientMessageId, body: body));
    if (failSend != null) {
      final e = failSend!;
      failSend = null;
      throw e;
    }
    final r = _next();
    if (r.outcome == Outcome.delivered && r.message == null) {
      final m = fakeMessage(
        messageRows.length + 1,
        from: Side.buyer,
        body: body,
      );
      messageRows = [...messageRows, m];
      return SendResult(outcome: Outcome.delivered, message: m);
    }
    return r;
  }

  @override
  Future<SendResult> sendSuggestion(
    String conversationId,
    String flagId, {
    required String clientMessageId,
  }) async {
    suggestions.add((
      conv: conversationId,
      flag: flagId,
      client: clientMessageId,
    ));
    final m = fakeMessage(
      messageRows.length + 1,
      from: Side.buyer,
      body: 'suggested text',
    );
    messageRows = [...messageRows, m];
    return SendResult(outcome: Outcome.delivered, message: m);
  }

  @override
  Future<void> markRead(String conversationId, int seq) async {
    reads.add(seq);
  }
}

/// A scriptable live socket: tests push frames and drop the connection.
class FakeChatSocket implements ChatSocket {
  final List<StreamController<ChatFrame>> _open = [];
  int connects = 0;
  bool refuse = false;

  void push(ChatFrame frame) {
    for (final c in _open) {
      if (!c.isClosed) c.add(frame);
    }
  }

  void drop() {
    for (final c in _open) {
      if (!c.isClosed) c.close();
    }
    _open.clear();
  }

  @override
  Stream<ChatFrame> frames({void Function()? onOpen}) async* {
    connects++;
    if (refuse) throw StateError('no connection');
    final c = StreamController<ChatFrame>();
    _open.add(c);
    onOpen?.call();
    yield* c.stream;
  }
}
