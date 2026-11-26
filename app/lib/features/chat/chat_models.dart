import 'package:flutter/foundation.dart';

import '../../core/json_read.dart';

/// Which side of a conversation a message or a person is on.
enum Side {
  buyer,
  seller;

  static Side? fromWire(Object? v) => switch (v) {
    'buyer' => buyer,
    'seller' => seller,
    _ => null,
  };
}

@immutable
class Conversation {
  const Conversation({
    required this.id,
    required this.with_,
    required this.you,
    required this.storeId,
    required this.storeName,
    required this.writable,
    required this.unread,
    required this.lastMessageAt,
    this.productId,
    this.orderId,
    this.branchName,
  });

  static Conversation? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final you = Side.fromWire(j['you']);
    final last = DateTime.tryParse(readString(j['last_message_at']) ?? '');
    if (id == null || you == null || last == null) return null;
    return Conversation(
      id: id,
      with_: readString(j['with']) ?? '',
      you: you,
      storeId: readString(j['store_id']) ?? '',
      storeName: readString(j['store_name']) ?? '',
      productId: readString(j['product_id']),
      orderId: readString(j['order_id']),
      branchName: readString(j['branch_name']),
      writable: j['writable'] == true,
      unread: readInt(j['unread']) ?? 0,
      lastMessageAt: last,
    );
  }

  final String id;

  /// The other side's label: the store's name, or "Buyer".
  final String with_;
  final Side you;
  final String storeId;
  final String storeName;
  final String? productId;
  final String? orderId;
  final String? branchName;
  final bool writable;
  final int unread;
  final DateTime lastMessageAt;

  bool get aboutOrder => orderId != null;
}

class ConversationPage {
  const ConversationPage({required this.items, this.nextCursor});

  factory ConversationPage.fromJson(Map<String, dynamic> j) => ConversationPage(
    items: [
      for (final c in readObjects(j['conversations']))
        ?Conversation.fromJson(c),
    ],
    nextCursor: readString(j['next_cursor']),
  );

  final List<Conversation> items;
  final String? nextCursor;
}

/// A delivered message. The text is personal: it is held in memory only and never
/// printed.
@immutable
class ChatMessage {
  const ChatMessage({
    required this.id,
    required this.conversationId,
    required this.seq,
    required this.from,
    required this.body,
    required this.sentAt,
    this.by,
  });

  static ChatMessage? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final seq = readInt(j['seq']);
    final from = Side.fromWire(j['from']);
    final at = DateTime.tryParse(readString(j['sent_at']) ?? '');
    if (id == null || seq == null || from == null || at == null) return null;
    return ChatMessage(
      id: id,
      conversationId: readString(j['conversation_id']) ?? '',
      seq: seq,
      from: from,
      body: j['body'] is String ? j['body'] as String : '',
      by: readString(j['by']),
      sentAt: at,
    );
  }

  final String id;
  final String conversationId;
  final int seq;
  final Side from;
  final String body;

  /// Which member of the store wrote it, told to the store's team only.
  final String? by;
  final DateTime sentAt;

  @override
  String toString() => 'ChatMessage($seq)';
}

/// What the server decided about a sent message.
enum Outcome { delivered, held, blocked, rewrite }

@immutable
class SendResult {
  const SendResult({
    required this.outcome,
    this.message,
    this.flagId,
    this.suggested,
    this.reason,
    this.strikes,
    this.mutedUntil,
  });

  static SendResult? fromJson(Map<String, dynamic> j) {
    final outcome = switch (j['outcome']) {
      'delivered' => Outcome.delivered,
      'held' => Outcome.held,
      'blocked' => Outcome.blocked,
      'rewrite' => Outcome.rewrite,
      _ => null,
    };
    if (outcome == null) return null;
    final m = j['message'];
    return SendResult(
      outcome: outcome,
      message: m is Map<String, dynamic> ? ChatMessage.fromJson(m) : null,
      flagId: readString(j['flag_id']),
      suggested: readString(j['suggested']),
      reason: readString(j['reason']),
      strikes: readInt(j['strikes']),
      mutedUntil: DateTime.tryParse(readString(j['muted_until']) ?? ''),
    );
  }

  final Outcome outcome;
  final ChatMessage? message;
  final String? flagId;
  final String? suggested;
  final String? reason;
  final int? strikes;
  final DateTime? mutedUntil;
}

/// One of the sender's own messages the other side has not seen.
@immutable
class OwnPending {
  const OwnPending({
    required this.id,
    required this.clientMessageId,
    required this.status,
    required this.createdAt,
    this.body,
    this.reason,
    this.flagId,
    this.suggested,
  });

  static OwnPending? fromJson(Map<String, dynamic> j) {
    final id = readString(j['id']);
    final client = readString(j['client_message_id']);
    final at = DateTime.tryParse(readString(j['created_at']) ?? '');
    if (id == null || client == null || at == null) return null;
    return OwnPending(
      id: id,
      clientMessageId: client,
      status: readString(j['status']) ?? 'held',
      createdAt: at,
      body: readString(j['body']),
      reason: readString(j['reason']),
      flagId: readString(j['flag_id']),
      suggested: readString(j['suggested']),
    );
  }

  final String id;
  final String clientMessageId;

  /// `held` (being checked) or `withheld` (stopped).
  final String status;
  final DateTime createdAt;
  final String? body;
  final String? reason;
  final String? flagId;
  final String? suggested;
}

class History {
  const History({required this.messages, required this.pending});

  factory History.fromJson(Map<String, dynamic> j) => History(
    messages: [
      for (final m in readObjects(j['messages'])) ?ChatMessage.fromJson(m),
    ],
    pending: [
      for (final p in readObjects(j['pending'])) ?OwnPending.fromJson(p),
    ],
  );

  final List<ChatMessage> messages;
  final List<OwnPending> pending;
}

/// A frame pushed over the socket.
sealed class ChatFrame {
  const ChatFrame();

  /// Null for anything this app does not know.
  static ChatFrame? fromJson(Object? json) {
    if (json is! Map<String, dynamic>) return null;
    switch (json['type']) {
      case 'message':
        final m = ChatMessage.fromJson(json);
        return m == null ? null : MessageFrame(m);
      case 'moderation':
        final c = readString(json['conversation_id']);
        final o = json['outcome'];
        final r = o is Map<String, dynamic> ? SendResult.fromJson(o) : null;
        return c == null || r == null
            ? null
            : ModerationFrame(c, readString(json['message_id']), r);
    }
    return null;
  }

  String get conversationId;
}

class MessageFrame extends ChatFrame {
  const MessageFrame(this.message);

  final ChatMessage message;

  @override
  String get conversationId => message.conversationId;
}

class ModerationFrame extends ChatFrame {
  const ModerationFrame(this.conversationId, this.messageId, this.result);

  @override
  final String conversationId;
  final String? messageId;
  final SendResult result;
}
