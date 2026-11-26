import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../listing_models.dart';
import '../sell_repository.dart';

/// Sends the bytes of one ticket to the object store. The app's own API client
/// is not used: the bearer token never goes to another host.
abstract class ObjectPutter {
  Future<void> put(UploadTicket ticket, Uint8List bytes);
}

class DioObjectPutter implements ObjectPutter {
  DioObjectPutter([Dio? dio]) : _dio = dio ?? Dio();

  final Dio _dio;

  @override
  Future<void> put(UploadTicket ticket, Uint8List bytes) async {
    final uri = Uri.tryParse(ticket.url);
    if (uri == null || uri.scheme != 'https' && !_localHttp(uri)) {
      throw ArgumentError('upload address is not https');
    }
    await _dio.put<void>(
      ticket.url,
      data: Stream.fromIterable([bytes]),
      options: Options(
        headers: {...ticket.headers, Headers.contentLengthHeader: bytes.length},
        // The signed headers carry the content type; nothing is added.
        contentType:
            ticket.headers['Content-Type'] ?? ticket.headers['content-type'],
        sendTimeout: const Duration(seconds: 60),
        receiveTimeout: const Duration(seconds: 30),
      ),
    );
  }

  bool _localHttp(Uri uri) =>
      uri.scheme == 'http' &&
      const {'10.0.2.2', 'localhost', '127.0.0.1'}.contains(uri.host);
}

final objectPutterProvider = Provider<ObjectPutter>((ref) => DioObjectPutter());

/// Uploads a prepared photo: ask for a ticket, put the bytes, finalize, and answer
/// the key to save on the product.
class PhotoUploader {
  PhotoUploader(this._work, this._putter);

  final WorkRepository _work;
  final ObjectPutter _putter;

  Future<String> upload(Uint8List jpeg, {String purpose = 'product'}) async {
    final ticket = await _work.newUpload(
      purpose: purpose,
      contentType: 'image/jpeg',
      bytes: jpeg.length,
    );
    await _putter.put(ticket, jpeg);
    return _work.finishUpload(ticket.uploadId);
  }
}

final photoUploaderProvider = Provider<PhotoUploader>(
  (ref) => PhotoUploader(
    ref.watch(workRepositoryProvider),
    ref.watch(objectPutterProvider),
  ),
);
