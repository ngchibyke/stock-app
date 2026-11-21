import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import 'core/config/app_gate.dart';
import 'core/session/session_controller.dart';
import 'core/strings.dart';
import 'core/theme/tokens.dart';
import 'core/ui/states.dart';
import 'features/auth/auth_errors.dart';
import 'features/auth/forgot_page.dart';
import 'features/auth/pending_reset.dart';
import 'features/auth/pending_verification.dart';
import 'features/auth/reset_page.dart';
import 'features/auth/sign_in_page.dart';
import 'features/auth/sign_up_page.dart';
import 'features/auth/verify_page.dart';

const String splashPath = '/splash';
const String blockedPath = '/blocked';
const String signInPath = '/sign-in';
const String signUpPath = '/sign-up';
const String verifyPath = '/verify';
const String forgotPath = '/forgot';
const String resetPath = '/reset';
const String homePath = '/home';

/// Pages a signed-out person may open.
const Set<String> _signedOutPaths = {
  signInPath,
  signUpPath,
  verifyPath,
  forgotPath,
  resetPath,
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

/// Whether a code page could be built right now: a hand-off is held and its
/// password has not been wiped.
bool _hasLiveHandOff(ProviderContainer container) {
  final password = container
      .read(pendingVerificationProvider)
      ?.credential
      .password;
  return password != null && password.isNotEmpty;
}

/// Whether the reset page could be built right now: a hand-off is held.
bool _hasResetHandOff(ProviderContainer container) =>
    container.read(pendingResetProvider) != null;

GoRouter buildRouter(ProviderContainer container) {
  final pending = pendingVerificationProvider;
  return GoRouter(
    initialLocation: homePath,
    refreshListenable: _RouterRefresh(container),
    redirect: (context, state) {
      final location = state.uri.toString();
      final to = redirectFor(
        session: container.read(sessionProvider),
        gate: container.read(gateProvider),
        location: location,
      );
      if (to != null) return to;
      // Checked here as well as on the route: a code page that was pushed is
      // not re-checked by its own route redirect when the router refreshes.
      if (Uri.parse(location).path == verifyPath &&
          !_hasLiveHandOff(container)) {
        return signInPath;
      }
      // The same for the reset page: no hand-off, nothing to enter a code for.
      if (Uri.parse(location).path == resetPath &&
          !_hasResetHandOff(container)) {
        return forgotPath;
      }
      return null;
    },
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
          onForgot: () => context.go(forgotPath),
        ),
      ),
      GoRoute(
        path: forgotPath,
        builder: (context, state) => ForgotPage(
          onCodeSent: (challenge, phone) {
            container
                .read(pendingResetProvider.notifier)
                .set(PendingReset(challenge: challenge, phone: phone));
            context.push(resetPath);
          },
          onBack: () => context.go(signInPath),
        ),
      ),
      GoRoute(
        path: resetPath,
        // Nothing is carried in the path: with no hand-off in memory the
        // person asks for a code again.
        redirect: (context, state) =>
            _hasResetHandOff(container) ? null : forgotPath,
        builder: (context, state) {
          final held = container.read(pendingResetProvider);
          // A pushed page is not re-checked by the redirects, so one that is
          // rebuilt without a hand-off sends the person back to ask again.
          if (held == null) return const _GoTo(forgotPath);
          return _ClearResetOnLeave(
            container: container,
            held: held,
            child: ResetPage(
              challenge: held.challenge,
              phone: held.phone,
              onReset: () {
                // No session: the server ended them all. The person signs in
                // again, with a one-time message saying so.
                container.read(passwordChangedNoticeProvider.notifier).show();
                container.read(pendingResetProvider.notifier).clear();
                context.go(signInPath);
              },
            ),
          );
        },
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
        // A wiped credential counts as no hand-off too (the code page wipes it
        // when it goes away), so the page is only built with a live one.
        redirect: (context, state) =>
            _hasLiveHandOff(container) ? null : signInPath,
        builder: (context, state) {
          final held = container.read(pending);
          // A pushed code page is not re-checked by the redirects, so one that
          // is rebuilt without a live hand-off sends the person to sign-in.
          if (held == null || !_hasLiveHandOff(container)) {
            return const _GoTo(signInPath);
          }
          return _ClearHolderOnLeave(
            container: container,
            child: VerifyPage(
              challenge: held.challenge,
              phone: held.phone,
              credential: held.credential,
              onVerified: () {
                container.read(pending.notifier).clear();
                context.go(homePath);
              },
            ),
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

/// Replaces whatever is stacked with the page at [path], after the frame.
class _GoTo extends StatefulWidget {
  const _GoTo(this.path);

  final String path;

  @override
  State<_GoTo> createState() => _GoToState();
}

class _GoToState extends State<_GoTo> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) context.go(widget.path);
    });
  }

  @override
  Widget build(BuildContext context) => const SizedBox.shrink();
}

/// Drops the hand-off when the code page goes away (back, or a move to another
/// page), so stale phone and id are not kept. A gate notice also removes the
/// page; the route's redirect then sends the person to sign-in on return.
class _ClearHolderOnLeave extends StatefulWidget {
  const _ClearHolderOnLeave({required this.container, required this.child});

  final ProviderContainer container;
  final Widget child;

  @override
  State<_ClearHolderOnLeave> createState() => _ClearHolderOnLeaveState();
}

class _ClearHolderOnLeaveState extends State<_ClearHolderOnLeave> {
  @override
  void dispose() {
    final container = widget.container;
    // A provider may not change during a widget life-cycle, so this waits for
    // the frame to end. By then the whole app may be gone; that is fine.
    scheduleMicrotask(() {
      try {
        container.read(pendingVerificationProvider.notifier).clear();
      } on StateError {
        // The container was disposed first: nothing is held any more.
      }
    });
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.child;
}

/// Drops the reset hand-off (the phone) when the reset page goes away. It
/// clears only the hand-off the page was built for: a newer one, set before
/// this page's microtask runs, is left alone.
class _ClearResetOnLeave extends StatefulWidget {
  const _ClearResetOnLeave({
    required this.container,
    required this.held,
    required this.child,
  });

  final ProviderContainer container;
  final PendingReset held;
  final Widget child;

  @override
  State<_ClearResetOnLeave> createState() => _ClearResetOnLeaveState();
}

class _ClearResetOnLeaveState extends State<_ClearResetOnLeave> {
  @override
  void dispose() {
    final container = widget.container;
    final held = widget.held;
    // A provider may not change during a widget life-cycle, so this waits for
    // the frame to end. By then the whole app may be gone; that is fine.
    scheduleMicrotask(() {
      try {
        container.read(pendingResetProvider.notifier).clearIfCurrent(held);
      } on StateError {
        // The container was disposed first: nothing is held any more.
      }
    });
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => widget.child;
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
