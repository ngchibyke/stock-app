import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../../core/config/app_config.dart';
import '../../core/config/app_gate.dart';
import '../../core/format/phone.dart';
import '../../core/links/external_link.dart';
import '../../core/paths.dart';
import '../../core/session/me.dart';
import '../../core/session/session_controller.dart';
import '../../core/strings.dart';
import '../../core/theme/tokens.dart';
import '../../core/ui/states.dart';
import '../auth/auth_errors.dart';
import '../sell/sell_providers.dart';
import '../sell/sell_shell.dart';
import '../sell/sell_strings.dart';
import '../sell/sell_widgets.dart';
import '../sell/team_branches.dart';

/// Who is signed in, the app version, the links to the terms, privacy page
/// and support, the address book and the way out.
class AccountPage extends ConsumerStatefulWidget {
  const AccountPage({super.key});

  @override
  ConsumerState<AccountPage> createState() => _AccountPageState();
}

class _AccountPageState extends ConsumerState<AccountPage> {
  // Set at once on the first tap (not on the next frame), so a second tap in
  // the same moment cannot start a second sign-out.
  bool _signingOut = false;

  Future<void> _signOut() async {
    if (_signingOut) return;
    setState(() => _signingOut = true);
    try {
      await ref.read(sessionProvider.notifier).signOut();
    } finally {
      if (mounted) setState(() => _signingOut = false);
    }
  }

  Future<void> _open(Uri? uri) async {
    var opened = false;
    if (uri != null) {
      try {
        opened = await ref.read(externalLinkOpenerProvider)(uri);
      } on Object {
        // No app to open it, or the platform refused: told below.
      }
    }
    if (!opened && mounted) {
      ScaffoldMessenger.of(context)
          .showSnackBar(const SnackBar(content: Text(Strings.linkOpenFailed)));
    }
  }

  /// Only absolute https links are opened.
  Uri? _webLink(String url) {
    final uri = Uri.tryParse(url);
    if (uri == null || uri.scheme != 'https' || uri.host.isEmpty) return null;
    return uri;
  }

  /// The address goes in as the path, so characters like `?` and `&` are
  /// escaped and cannot add headers (a copy, a subject, a body).
  Uri _mailLink(String address) => Uri(scheme: 'mailto', path: address);

  @override
  Widget build(BuildContext context) {
    final session = ref.watch(sessionProvider);
    final state = session.value;

    final Widget body;
    if (state is SessionSignedIn) {
      body = _data(context, state.me);
    } else if (state is SessionSignedOut) {
      // The router is about to move on; there is nothing to show.
      body = const SizedBox.shrink();
    } else if (session.isLoading) {
      body = const _Loading();
    } else {
      body = Column(
        children: [
          Expanded(
            child: ErrorState(
              message: authErrorMessage(session.error!),
              onRetry: () => ref.invalidate(sessionProvider),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Tokens.s5,
              0,
              Tokens.s5,
              Tokens.s4,
            ),
            child: _signOutButton(),
          ),
        ],
      );
    }
    return SafeArea(bottom: false, child: body);
  }

  Widget _signOutButton() => LoadingButton(
    label: Strings.signOut,
    loading: _signingOut,
    onPressed: _signOut,
  );

  Widget _data(BuildContext context, Me me) {
    final theme = Theme.of(context);
    final tokens = context.tokens;
    final config = ref.watch(appConfigProvider).value ?? AppConfig.empty;
    final version = ref.watch(packageVersionProvider).value;
    final termsUrl = config.termsUrl;
    final privacyUrl = config.privacyUrl;
    final supportEmail = config.supportEmail;

    return ListView(
      padding: const EdgeInsets.all(Tokens.s5),
      children: [
        const SizedBox(height: Tokens.s4),
        Semantics(
          header: true,
          child: Text(me.displayName, style: theme.textTheme.titleLarge),
        ),
        const SizedBox(height: Tokens.s1),
        Text(
          displayPhone(me.phone),
          style: theme.textTheme.bodyLarge?.copyWith(color: tokens.mute),
        ),
        const SizedBox(height: Tokens.s1),
        Text(
          Strings.roleName(me.role),
          style: theme.textTheme.bodyMedium?.copyWith(color: tokens.mute),
        ),
        if (version != null) ...[
          const SizedBox(height: Tokens.s1),
          Text(
            Strings.appVersion(version),
            style: theme.textTheme.bodySmall?.copyWith(color: tokens.mute),
          ),
        ],
        const SizedBox(height: Tokens.s5),
        _LinkRow(
          label: hasSellerSide(me, ref.watch(myStoresProvider).value)
              ? SellStrings.myStores
              : SellStrings.sellEntry,
          icon: Icons.chevron_right,
          onTap: () => context.push(sellPath),
        ),
        if (ref.watch(myInvitesProvider).value?.isNotEmpty ?? false)
          _LinkRow(
            label: SellStrings.invitesTitle,
            icon: Icons.chevron_right,
            onTap: () => pushPage<void>(context, const InvitesPage()),
          ),
        _LinkRow(
          label: Strings.addresses,
          icon: Icons.chevron_right,
          onTap: () => context.push(addressesPath),
        ),
        if (termsUrl != null)
          _LinkRow(
            label: Strings.terms,
            icon: Icons.open_in_new,
            onTap: () => _open(_webLink(termsUrl)),
          ),
        if (privacyUrl != null)
          _LinkRow(
            label: Strings.privacy,
            icon: Icons.open_in_new,
            onTap: () => _open(_webLink(privacyUrl)),
          ),
        if (supportEmail != null)
          _LinkRow(
            label: Strings.contactSupport,
            icon: Icons.mail_outline,
            onTap: () => _open(_mailLink(supportEmail)),
          ),
        const SizedBox(height: Tokens.s5),
        _signOutButton(),
      ],
    );
  }
}

/// A tappable row at least 48dp tall; its text wraps rather than clips.
class _LinkRow extends StatelessWidget {
  const _LinkRow({
    required this.label,
    required this.icon,
    required this.onTap,
  });

  final String label;
  final IconData icon;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      button: true,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(Tokens.radius),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: Tokens.minTap),
          child: Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: Tokens.s2,
              vertical: Tokens.s2,
            ),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    label,
                    style: Theme.of(context).textTheme.bodyLarge,
                  ),
                ),
                ExcludeSemantics(child: Icon(icon, color: context.tokens.mute)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Placeholder blocks while the session is restored, announced once.
class _Loading extends StatelessWidget {
  const _Loading();

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: Strings.loading,
      liveRegion: true,
      container: true,
      excludeSemantics: true,
      child: const Padding(
        padding: EdgeInsets.all(Tokens.s5),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(height: Tokens.s4),
            Skeleton(height: Tokens.s5, width: 160),
            SizedBox(height: Tokens.s3),
            Skeleton(width: 120),
            SizedBox(height: Tokens.s2),
            Skeleton(width: 80),
          ],
        ),
      ),
    );
  }
}
