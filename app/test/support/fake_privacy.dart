import 'package:market/core/api/api_error.dart';
import 'package:market/features/privacy/privacy_models.dart';
import 'package:market/features/privacy/privacy_repository.dart';

/// Deletion and export state held in memory. Set [deleteError] or [exportError] to make
/// the next call fail; [file] is what the export holds once it is ready.
class FakePrivacyRepository implements PrivacyRepository {
  DeletionStatus deletion = const DeletionStatus(pending: false);
  ExportStatus export = const ExportStatus(state: ExportState.none);
  Object? deleteError;
  Object? exportError;
  String file = '{"profile":{"display_name":"Ada"}}';

  final List<String> passwords = [];
  int cancelled = 0;
  int fileReads = 0;

  @override
  Future<DeletionStatus> deletionStatus() async => deletion;

  @override
  Future<DeletionStatus> requestDeletion(String password) async {
    passwords.add(password);
    final e = deleteError;
    if (e != null) throw e;
    deletion = DeletionStatus(
      pending: true,
      requestedAt: DateTime(2026, 11, 26),
      dueAt: DateTime(2026, 12, 26),
    );
    return deletion;
  }

  @override
  Future<void> cancelDeletion() async {
    final e = deleteError;
    if (e != null) throw e;
    cancelled++;
    deletion = const DeletionStatus(pending: false);
  }

  @override
  Future<ExportStatus> exportStatus() async => export;

  @override
  Future<ExportStatus> requestExport(String password) async {
    passwords.add(password);
    final e = exportError;
    if (e != null) throw e;
    export = ExportStatus(
      state: ExportState.pending,
      requestedAt: DateTime(2026, 11, 26),
    );
    return export;
  }

  @override
  Future<String> exportFile() async {
    fileReads++;
    return file;
  }
}

class FakeFileSharer implements FileSharer {
  final List<(String, String)> shared = [];

  @override
  Future<void> shareText(String fileName, String content) async {
    shared.add((fileName, content));
  }
}

ApiError blocked(List<String> reasons) => ApiError(
  status: 409,
  code: 'deletion_blocked',
  message: 'x',
  reasons: reasons,
);
