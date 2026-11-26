import 'dart:convert';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:share_plus/share_plus.dart';

import '../../core/api/api_client.dart';
import '../../core/api/api_error.dart';
import '../../core/api/providers.dart';
import 'privacy_models.dart';

ApiError _bad() => ApiError(
  status: 0,
  code: 'bad_response',
  message: 'The server sent an unexpected response.',
);

Map<String, dynamic> _obj(Object? j) =>
    j is Map<String, dynamic> ? j : throw _bad();

abstract class PrivacyRepository {
  Future<DeletionStatus> deletionStatus();

  /// Starts the cooling-off. Throws an [ApiError]: 401 `invalid_credentials` for a wrong
  /// password, 409 `deletion_blocked` with [ApiError.reasons].
  Future<DeletionStatus> requestDeletion(String password);

  Future<void> cancelDeletion();

  Future<ExportStatus> exportStatus();

  /// 401 `invalid_credentials`, 429 `export_rate_limited`.
  Future<ExportStatus> requestExport(String password);

  /// The export, as text, while it is ready.
  Future<String> exportFile();
}

/// [PrivacyRepository] over the server API. The export holds personal data: it is returned
/// to the caller and kept nowhere here.
class ApiPrivacyRepository implements PrivacyRepository {
  ApiPrivacyRepository(this._api);

  final ApiClient _api;

  @override
  Future<DeletionStatus> deletionStatus() async =>
      DeletionStatus.fromJson(_obj(await _api.get('/me/deletion')));

  @override
  Future<DeletionStatus> requestDeletion(String password) async =>
      DeletionStatus.fromJson(
        _obj(await _api.post('/me/deletion', body: {'password': password})),
      );

  @override
  Future<void> cancelDeletion() async {
    await _api.delete('/me/deletion');
  }

  @override
  Future<ExportStatus> exportStatus() async =>
      ExportStatus.fromJson(_obj(await _api.get('/me/export')));

  @override
  Future<ExportStatus> requestExport(String password) async =>
      ExportStatus.fromJson(
        _obj(await _api.post('/me/export', body: {'password': password})),
      );

  @override
  Future<String> exportFile() async =>
      const JsonEncoder.withIndent('  ')
          .convert(await _api.get('/me/export/file'));
}

final privacyRepositoryProvider = Provider<PrivacyRepository>(
  (ref) => ApiPrivacyRepository(ref.watch(apiClientProvider)),
);

/// Hands the export to the system's share sheet (save to Files, Drive, and so on). The
/// file is never kept in the app.
abstract class FileSharer {
  Future<void> shareText(String fileName, String content);
}

class SystemFileSharer implements FileSharer {
  @override
  Future<void> shareText(String fileName, String content) async {
    await SharePlus.instance.share(
      ShareParams(
        files: [
          XFile.fromData(utf8.encode(content), mimeType: 'application/json'),
        ],
        fileNameOverrides: [fileName],
      ),
    );
  }
}

final fileSharerProvider = Provider<FileSharer>((ref) => SystemFileSharer());

final exportStatusProvider = FutureProvider.autoDispose<ExportStatus>(
  (ref) => ref.watch(privacyRepositoryProvider).exportStatus(),
);
