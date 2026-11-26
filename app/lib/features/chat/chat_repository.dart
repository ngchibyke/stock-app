import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:web_socket_channel/io.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/providers.dart';
import 'chat_models.dart';

ApiError _bad() => ApiError(
  status: 0,
  code: 'bad_response',
  message: 'The server sent an unexpected response.',
);

Map<String, dynamic> _obj(Object? j) =>
    j is Map<String, dynamic> ? j : throw _bad();

String _id(String s) => Uri.encodeComponent(s);

abstract class ChatRepository {
  /// Opens (or finds) the conversation about a product, as a buyer.
  Future<Conversation> openProduct(String productId);

  /// Opens (or finds) the conversation about a paid order.
  Future<Conversation> openOrder(String orderId);

  Future<ConversationPage> conversations({String? before});

  /// Delivered messages after [after], oldest first, and the caller's own pending ones.
  Future<History> history(String conversationId, {int after = 0});

  /// [clientMessageId] makes a retry the same message.
  Future<SendResult> send(
    String conversationId, {
    required String clientMessageId,
    required String body,
  });

  /// Sends the text the server stored for [flagId]; no text comes from the app.
  Future<SendResult> sendSuggestion(
    String conversationId,
    String flagId, {
    required String clientMessageId,
  });

  Future<void> markRead(String conversationId, int seq);
}

class ApiChatRepository implements ChatRepository {
  ApiChatRepository(this._api);

  final ApiClient _api;

  Conversation _conv(Object? j) =>
      Conversation.fromJson(_obj(j)) ?? (throw _bad());

  @override
  Future<Conversation> openProduct(String productId) async =>
      _conv(await _api.post('/chats', body: {'product_id': productId}));

  @override
  Future<Conversation> openOrder(String orderId) async =>
      _conv(await _api.post('/chats', body: {'order_id': orderId}));

  @override
  Future<ConversationPage> conversations({String? before}) async =>
      ConversationPage.fromJson(
        _obj(await _api.get('/chats', query: {'limit': 30, 'before': ?before})),
      );

  @override
  Future<History> history(String conversationId, {int after = 0}) async =>
      History.fromJson(
        _obj(
          await _api.get(
            '/chats/${_id(conversationId)}/messages',
            query: {'after': after, 'limit': 100},
          ),
        ),
      );

  SendResult _result(Object? j) =>
      SendResult.fromJson(_obj(j)) ?? (throw _bad());

  @override
  Future<SendResult> send(
    String conversationId, {
    required String clientMessageId,
    required String body,
  }) async => _result(
    await _api.post(
      '/chats/${_id(conversationId)}/messages',
      body: {'client_message_id': clientMessageId, 'body': body},
    ),
  );

  @override
  Future<SendResult> sendSuggestion(
    String conversationId,
    String flagId, {
    required String clientMessageId,
  }) async => _result(
    await _api.post(
      '/chats/${_id(conversationId)}/suggestions/${_id(flagId)}/send',
      body: {'client_message_id': clientMessageId},
    ),
  );

  @override
  Future<void> markRead(String conversationId, int seq) async {
    await _api.post('/chats/${_id(conversationId)}/read', body: {'seq': seq});
  }
}

final chatRepositoryProvider = Provider<ChatRepository>(
  (ref) => ApiChatRepository(ref.watch(apiClientProvider)),
);

/// A connection to the server's live socket. It only pushes: sending goes through
/// [ChatRepository].
abstract class ChatSocket {
  /// Frames until the connection ends. Throws or completes when it drops. [onOpen] is
  /// called once the connection is up, even before any frame arrives.
  Stream<ChatFrame> frames({void Function()? onOpen});
}

class WebChatSocket implements ChatSocket {
  WebChatSocket({required this.baseUrl, required this.readToken});

  final String baseUrl;
  final Future<String?> Function() readToken;

  @override
  Stream<ChatFrame> frames({void Function()? onOpen}) {
    IOWebSocketChannel? channel;
    StreamSubscription<Object?>? inner;
    late final StreamController<ChatFrame> out;

    Future<void> close() async {
      final i = inner;
      inner = null;
      await i?.cancel();
      final c = channel;
      channel = null;
      if (c != null) unawaited(c.sink.close());
    }

    Future<void> run() async {
      final token = await readToken();
      if (token == null || out.isClosed) {
        if (!out.isClosed) await out.close();
        return;
      }
      final base = Uri.parse(baseUrl);
      final uri = base.replace(
        scheme: base.scheme == 'https' ? 'wss' : 'ws',
        path: '/chats/ws',
      );
      // The bearer header, never the URL: a token in a URL ends up in logs.
      final c = IOWebSocketChannel.connect(
        uri,
        headers: {'Authorization': 'Bearer $token'},
        pingInterval: const Duration(seconds: 25),
      );
      channel = c;
      try {
        await c.ready;
      } on Object catch (e, st) {
        if (!out.isClosed) out.addError(e, st);
        await close();
        if (!out.isClosed) await out.close();
        return;
      }
      if (out.isClosed) return close();
      onOpen?.call();
      inner = c.stream.listen(
        (raw) {
          if (raw is! String) return;
          Object? json;
          try {
            json = jsonDecode(raw);
          } on FormatException {
            return;
          }
          final frame = ChatFrame.fromJson(json);
          if (frame != null && !out.isClosed) out.add(frame);
        },
        onError: (Object e, StackTrace st) {
          if (!out.isClosed) out.addError(e, st);
        },
        onDone: () async {
          await close();
          if (!out.isClosed) await out.close();
        },
      );
    }

    out = StreamController<ChatFrame>(
      onListen: () => unawaited(run()),
      // Cancelling closes the connection now, not at the next frame.
      onCancel: close,
    );
    return out.stream;
  }
}

final chatSocketProvider = Provider<ChatSocket>(
  (ref) => WebChatSocket(
    baseUrl: apiBaseUrl,
    readToken: () => ref.read(accessTokenReaderProvider)(),
  ),
);

/// How long to wait before the next reconnect, with jitter: 1, 2, 4 ... up to 30 s.
Duration reconnectDelay(int attempt, [Random? random]) {
  final base = min(30, 1 << min(attempt, 5));
  final jitter = (random ?? Random()).nextDouble() * 0.5 + 0.75;
  return Duration(milliseconds: (base * 1000 * jitter).round());
}

/// How the live link is doing.
final chatConnectedProvider = NotifierProvider<_Connected, bool>(
  _Connected.new,
);

class _Connected extends Notifier<bool> {
  @override
  bool build() => false;

  void set(bool v) {
    // A socket loop can outlive the container by a moment.
    if (ref.mounted && state != v) state = v;
  }
}

/// Live frames, reconnecting with backoff while something is watching. When nothing
/// watches (the app has no chat screen open) the socket is closed.
final chatFramesProvider = StreamProvider.autoDispose<ChatFrame>((ref) async* {
  final socket = ref.watch(chatSocketProvider);
  final connected = ref.read(chatConnectedProvider.notifier);
  ref.onDispose(() => Future.microtask(() => connected.set(false)));
  var attempt = 0;
  while (ref.mounted) {
    try {
      await for (final frame in socket.frames(
        onOpen: () {
          attempt = 0;
          connected.set(true);
        },
      )) {
        yield frame;
      }
    } on Object {
      // Dropped: reconnect below.
    }
    connected.set(false);
    await Future<void>.delayed(reconnectDelay(attempt++));
  }
});
