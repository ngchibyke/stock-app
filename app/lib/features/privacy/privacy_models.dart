import '../../core/json_read.dart';

/// A request to delete the account: waiting out its cooling-off until [dueAt].
class DeletionStatus {
  const DeletionStatus({required this.pending, this.requestedAt, this.dueAt});

  static DeletionStatus fromJson(Map<String, dynamic> j) => DeletionStatus(
    pending: j['state'] == 'pending',
    requestedAt: DateTime.tryParse(readString(j['requested_at']) ?? ''),
    dueAt: DateTime.tryParse(readString(j['due_at']) ?? ''),
  );

  final bool pending;
  final DateTime? requestedAt;
  final DateTime? dueAt;
}

enum ExportState { none, pending, ready, failed, expired }

/// Where a data export is: asked for, being built, ready to take, or gone.
class ExportStatus {
  const ExportStatus({required this.state, this.requestedAt, this.expiresAt});

  static ExportStatus fromJson(Map<String, dynamic> j) => ExportStatus(
    state: switch (j['state']) {
      'pending' => ExportState.pending,
      'ready' => ExportState.ready,
      'failed' => ExportState.failed,
      'expired' => ExportState.expired,
      _ => ExportState.none,
    },
    requestedAt: DateTime.tryParse(readString(j['requested_at']) ?? ''),
    expiresAt: DateTime.tryParse(readString(j['expires_at']) ?? ''),
  );

  final ExportState state;
  final DateTime? requestedAt;
  final DateTime? expiresAt;
}
