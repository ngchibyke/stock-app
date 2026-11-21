import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import 'core/config/app_gate.dart';
import 'core/session/session_controller.dart';
import 'core/strings.dart';
import 'core/theme/tokens.dart';
import 'core/ui/states.dart';
import 'features/auth/auth_errors.dart';
import 'features/auth/pending_verification.dart';
import 'features/auth/sign_in_page.dart';
import 'features/auth/sign_up_page.dart';
import 'features/auth/verify_page.dart';

const String splashPath = '/splash';
const String blockedPath = '/blocked';
const String signInPath = '/sign-in';
const String signUpPath = '/sign-up';
const String verifyPath = '/verify';
const String homePath = '/home';

/// Pages a signed-out person may open.
const Set<String> _signedOutPaths = {
  signInPath,
  signUpPath,
  verifyPath,
  '/forgot',
  '/reset',
};

/// Pages (and everything below them) that need a signed-in person. Later tasks
/// add `/account` and `/addresses` routes under these.
const List<String> _signedInPrefixes = [homePath, '/account', '/addresses'];

bool _isUnder(String path, String prefix) =>
    path == prefix || path.startsWith('$prefix/');

/// Where the router should send [location], or null to stay. Pure.
///
/// A gate that is not open overrides everything (the notice has no navigation
/// behind it). While the session loads, or failed to load, the person stays on
/// the splash page (which carries the retry and sign-out for a failure). Then a
/// signed-out person may open only the sign-in pages and a signed-in person
/// only the app. Anything unknown goes home, or to sign-in.
String? redirectFor({
  required AsyncValue<SessionState> session,
  required Gate gate,
  required String location,
}) {
  final path = Uri.parse(location).path;

  if (gate != Gate.open) return path == blockedPath ? null : blockedPath;

  final state = session.value;
  if (state == null) return path == splashPath ? null : splashPath;

  switch (state) {
    case SessionSignedOut():
      return _signedOutPaths.contains(path) ? null : signInPath;
    case SessionSignedIn():
      final known = _signedInPrefixes.any((p) => _isUnder(path, p));
      return known ? null : homePath;
  }
}

/// Rebuilds the router's redirect when the session or the gate changes.
class _RouterRefresh extends ChangeNotifier {
  _RouterRefresh(ProviderContainer container) {
    container.listen(sessionProvider, (_, _) => notifyListeners());
    container.listen(gateProvider, (_, _) => notifyListeners());
  }
}

GoRouter buildRouter(ProviderContainer container) {
  final pending = pendingVerificationProvider;
  return GoRouter(
    initialLocation: homePath,
    refreshListenable: _RouterRefresh(container),
    redirect: (context, state) => redirectFor(
      session: container.read(sessionProvider),
      gate: container.read(gateProvider),
      location: state.uri.toString(),
    ),
    routes: [
      GoRoute(
        path: splashPath,
        builder: (context, state) => const SessionGatePage(),
      ),
      // The gate notice itself is drawn over the app by MarketApp; this route
      // only gives the redirect somewhere to put the person.
      GoRoute(
        path: blockedPath,
        builder: (context, state) => const SizedBox.shrink(),
      ),
      GoRoute(
        path: signInPath,
        builder: (context, state) => SignInPage(
          onUnverified: (challenge, phone, credential) {
            container
                .read(pending.notifier)
                .set(
                  PendingVerification(
                    challenge: challenge,
                    phone: phone,
                    credential: credential,
                  ),
                );
            context.push(verifyPath);
          },
          onSignUp: () => context.go(signUpPath),
        ),
      ),
      GoRoute(
        path: signUpPath,
        builder: (context, state) => SignUpPage(
          onCodeSent: (challenge, phone, credential) {
            container
                .read(pending.notifier)
                .set(
                  PendingVerification(
                    challenge: challenge,
                    phone: phone,
                    credential: credential,
                  ),
                );
            context.push(verifyPath);
          },
          onSignIn: () => context.go(signInPath),
        ),
      ),
      GoRoute(
        path: verifyPath,
        // Nothing is carried in the path: with no hand-off in memory (the app
        // was restarted, or someone typed the address) there is no code to
        // enter, so the person signs in again and gets a fresh one.
        redirect: (context, state) =>
            container.read(pending) == null ? signInPath : null,
        builder: (context, state) {
          final held = container.read(pending);
          if (held == null) return const SizedBox.shrink();
          return VerifyPage(
            challenge: held.challenge,
            phone: held.phone,
            credential: held.credential,
            onVerified: () {
              container.read(pending.notifier).clear();
              context.go(homePath);
            },
          );
        },
      ),
      // Replaced by the shell in a later task.
      GoRoute(
        path: homePath,
        builder: (context, state) =>
            const Scaffold(body: Center(child: Text(Strings.home))),
      ),
    ],
  );
}

final routerProvider = Provider<GoRouter>((ref) {
  final router = buildRouter(ref.container);
  ref.onDispose(router.dispose);
  return router;
});

/// What the person sees before the session is known: a labelled placeholder
/// while it restores, and, if the restore failed (offline, a server error), a
/// plain message with a retry and a way out. Without the sign-out a session
/// the server keeps answering badly for would be a dead end.
class SessionGatePage extends ConsumerWidget {
  const SessionGatePage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final session = ref.watch(sessionProvider);
    if (session.hasError && !session.isLoading) {
      return Scaffold(
        body: SafeArea(
          child: Column(
            children: [
              Expanded(
                child: ErrorState(
                  message: authErrorMessage(session.error!),
                  onRetry: () => ref.invalidate(sessionProvider),
                ),
              ),
              Padding(
                padding: const EdgeInsets.only(bottom: Tokens.s4),
                child: TextButton(
                  style: TextButton.styleFrom(minimumSize: const Size(48, 48)),
                  onPressed: () => ref.read(sessionProvider.notifier).signOut(),
                  child: const Text(Strings.signOut),
                ),
              ),
            ],
          ),
        ),
      );
    }
    return const _Splash();
  }
}

class _Splash extends StatelessWidget {
  const _Splash();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      body: Semantics(
        label: Strings.loading,
        liveRegion: true,
        container: true,
        excludeSemantics: true,
        child: Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                Strings.wordmark,
                style: theme.textTheme.titleMedium?.copyWith(
                  color: context.tokens.gold,
                  letterSpacing: 3,
                  fontWeight: FontWeight.w800,
                ),
              ),
              const SizedBox(height: Tokens.s4),
              const Skeleton(width: 120),
            ],
          ),
        ),
      ),
    );
  }
}
