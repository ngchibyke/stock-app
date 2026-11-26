import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/api/api_error.dart';
import '../../core/session/session_controller.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import 'privacy_models.dart';
import 'privacy_repository.dart';
import 'privacy_strings.dart';

/// Account, Privacy and data: take a copy of your data, or delete the account.
class PrivacyPage extends ConsumerWidget {
  const PrivacyPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Scaffold(
      appBar: AppBar(
        title: Semantics(header: true, child: const Text(PrivacyStrings.title)),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(Tokens.s4),
          children: [
            const _DownloadSection(),
            const SizedBox(height: Tokens.s6),
            Semantics(
              header: true,
              child: Text(
                PrivacyStrings.deleteTitle,
                style: Theme.of(context).textTheme.titleMedium,
              ),
            ),
            const SizedBox(height: Tokens.s2),
            OutlinedButton(
              style: OutlinedButton.styleFrom(
                minimumSize: const Size(0, Tokens.minTap),
              ),
              onPressed: () => Navigator.of(context).push<void>(
                MaterialPageRoute<void>(
                  builder: (_) => const DeleteAccountPage(),
                ),
              ),
              child: const Text(PrivacyStrings.deleteTitle),
            ),
          ],
        ),
      ),
    );
  }
}

class _DownloadSection extends ConsumerStatefulWidget {
  const _DownloadSection();

  @override
  ConsumerState<_DownloadSection> createState() => _DownloadSectionState();
}

class _DownloadSectionState extends ConsumerState<_DownloadSection> {
  final _password = TextEditingController();
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _password.dispose();
    super.dispose();
  }

  Future<void> _ask() async {
    if (_busy || _password.text.isEmpty) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await ref.read(privacyRepositoryProvider).requestExport(_password.text);
      _password.clear();
      ref.invalidate(exportStatusProvider);
    } on Object catch (e) {
      if (mounted) setState(() => _error = PrivacyStrings.error(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _share() async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final text = await ref.read(privacyRepositoryProvider).exportFile();
      await ref.read(fileSharerProvider).shareText('my-data.json', text);
    } on Object catch (e) {
      if (mounted) setState(() => _error = PrivacyStrings.error(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final status = ref.watch(exportStatusProvider);
    final tokens = Theme.of(context).extension<Tokens>();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Semantics(
          header: true,
          child: Text(
            PrivacyStrings.downloadTitle,
            style: Theme.of(context).textTheme.titleMedium,
          ),
        ),
        const SizedBox(height: Tokens.s2),
        status.when(
          loading: () => const Padding(
            padding: EdgeInsets.all(Tokens.s4),
            child: Center(child: CircularProgressIndicator()),
          ),
          error: (e, _) => ErrorState(
            message: PrivacyStrings.error(e),
            onRetry: () => ref.invalidate(exportStatusProvider),
          ),
          data: (s) => _body(context, s, tokens),
        ),
        if (_error != null) ...[
          const SizedBox(height: Tokens.s2),
          Semantics(
            liveRegion: true,
            child: Text(
              _error!,
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ),
        ],
      ],
    );
  }

  Widget _body(BuildContext context, ExportStatus s, Tokens? tokens) {
    switch (s.state) {
      case ExportState.pending:
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text(PrivacyStrings.downloadPending),
            const SizedBox(height: Tokens.s2),
            OutlinedButton(
              style: OutlinedButton.styleFrom(
                minimumSize: const Size(0, Tokens.minTap),
              ),
              onPressed: () => ref.invalidate(exportStatusProvider),
              child: const Text(PrivacyStrings.refresh),
            ),
          ],
        );
      case ExportState.ready:
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Text(PrivacyStrings.downloadReady),
            if (s.expiresAt != null)
              Text(
                PrivacyStrings.downloadUntil(PrivacyStrings.day(s.expiresAt!)),
              ),
            const SizedBox(height: Tokens.s2),
            LoadingButton(
              label: PrivacyStrings.downloadSave,
              loading: _busy,
              onPressed: _share,
            ),
          ],
        );
      case ExportState.none:
      case ExportState.failed:
      case ExportState.expired:
        final note = switch (s.state) {
          ExportState.failed => PrivacyStrings.downloadFailed,
          ExportState.expired => PrivacyStrings.downloadExpired,
          _ => PrivacyStrings.downloadBody,
        };
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(note),
            const SizedBox(height: Tokens.s2),
            TextField(
              controller: _password,
              obscureText: true,
              enableSuggestions: false,
              autocorrect: false,
              decoration: const InputDecoration(
                labelText: PrivacyStrings.passwordLabel,
              ),
              onSubmitted: (_) => _ask(),
            ),
            const SizedBox(height: Tokens.s2),
            LoadingButton(
              label: PrivacyStrings.downloadAsk,
              loading: _busy,
              onPressed: _ask,
            ),
          ],
        );
    }
  }
}

/// Asks for deletion: what goes, what stays, what is still open, then the password.
class DeleteAccountPage extends ConsumerStatefulWidget {
  const DeleteAccountPage({super.key});

  @override
  ConsumerState<DeleteAccountPage> createState() => _DeleteAccountPageState();
}

class _DeleteAccountPageState extends ConsumerState<DeleteAccountPage> {
  final _password = TextEditingController();
  bool _busy = false;
  String? _error;
  List<String> _blockers = const [];

  @override
  void dispose() {
    _password.dispose();
    super.dispose();
  }

  Future<void> _delete() async {
    if (_busy || _password.text.isEmpty) return;
    setState(() {
      _busy = true;
      _error = null;
      _blockers = const [];
    });
    try {
      final status = await ref
          .read(privacyRepositoryProvider)
          .requestDeletion(_password.text);
      if (!mounted) return;
      // The router moves to the waiting screen when the session says so.
      ref.read(sessionProvider.notifier).markDeleting(status.dueAt);
      Navigator.of(context).popUntil((r) => r.isFirst);
    } on ApiError catch (e) {
      if (!mounted) return;
      setState(() {
        if (e.code == 'deletion_blocked') {
          _blockers = e.reasons;
        } else {
          _error = PrivacyStrings.error(e);
        }
      });
    } on Object catch (e) {
      if (mounted) setState(() => _error = PrivacyStrings.error(e));
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(
        title: Semantics(
          header: true,
          child: const Text(PrivacyStrings.deleteTitle),
        ),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.all(Tokens.s4),
          children: [
            const Text(PrivacyStrings.deleteIntro),
            const SizedBox(height: Tokens.s4),
            Semantics(
              header: true,
              child: Text(
                PrivacyStrings.deleteKeptTitle,
                style: theme.textTheme.titleMedium,
              ),
            ),
            const SizedBox(height: Tokens.s1),
            const Text(PrivacyStrings.deleteKept),
            if (_blockers.isNotEmpty) ...[
              const SizedBox(height: Tokens.s4),
              Semantics(
                header: true,
                liveRegion: true,
                child: Text(
                  PrivacyStrings.deleteBlockedTitle,
                  style: theme.textTheme.titleMedium,
                ),
              ),
              for (final code in _blockers)
                Padding(
                  padding: const EdgeInsets.only(top: Tokens.s1),
                  child: Text(PrivacyStrings.blocker(code)),
                ),
            ],
            const SizedBox(height: Tokens.s4),
            TextField(
              controller: _password,
              obscureText: true,
              enableSuggestions: false,
              autocorrect: false,
              decoration: const InputDecoration(
                labelText: PrivacyStrings.passwordLabel,
              ),
              onSubmitted: (_) => _delete(),
            ),
            if (_error != null) ...[
              const SizedBox(height: Tokens.s2),
              Semantics(
                liveRegion: true,
                child: Text(
                  _error!,
                  style: TextStyle(color: theme.colorScheme.error),
                ),
              ),
            ],
            const SizedBox(height: Tokens.s4),
            LoadingButton(
              label: PrivacyStrings.deleteButton,
              loading: _busy,
              onPressed: _delete,
            ),
          ],
        ),
      ),
    );
  }
}

/// The only screen while a deletion waits out its cooling-off.
class DeletionPendingPage extends ConsumerStatefulWidget {
  const DeletionPendingPage({super.key});

  @override
  ConsumerState<DeletionPendingPage> createState() =>
      _DeletionPendingPageState();
}

class _DeletionPendingPageState extends ConsumerState<DeletionPendingPage> {
  bool _busy = false;
  String? _error;

  Future<void> _keep() async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await ref.read(privacyRepositoryProvider).cancelDeletion();
      await ref.read(sessionProvider.notifier).keepAccount();
    } on Object {
      if (mounted) setState(() => _error = PrivacyStrings.keepFailed);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final session = ref.watch(sessionProvider).value;
    final due = session is SessionDeleting ? session.dueAt : null;
    final theme = Theme.of(context);
    return Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(Tokens.s4),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Semantics(
                header: true,
                child: Text(
                  PrivacyStrings.pendingTitle,
                  style: theme.textTheme.headlineSmall,
                ),
              ),
              const SizedBox(height: Tokens.s3),
              Text(
                due == null
                    ? PrivacyStrings.pendingBodyNoDate
                    : PrivacyStrings.pendingBody(PrivacyStrings.day(due)),
              ),
              if (_error != null) ...[
                const SizedBox(height: Tokens.s2),
                Semantics(
                  liveRegion: true,
                  child: Text(
                    _error!,
                    style: TextStyle(color: theme.colorScheme.error),
                  ),
                ),
              ],
              const SizedBox(height: Tokens.s4),
              LoadingButton(
                label: PrivacyStrings.keepAccount,
                loading: _busy,
                onPressed: _keep,
              ),
              const SizedBox(height: Tokens.s2),
              TextButton(
                style: TextButton.styleFrom(
                  minimumSize: const Size(0, Tokens.minTap),
                ),
                onPressed: _busy
                    ? null
                    : () => ref.read(sessionProvider.notifier).signOut(),
                child: const Text(Strings.signOut),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
