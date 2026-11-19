import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';
import 'package:material_ui/material_ui.dart';

import '../features/account/account_screen.dart';
import '../features/auth/auth_controller.dart';
import '../features/auth/auth_models.dart';
import '../features/auth/auth_screens.dart';
import 'shell.dart';
import 'splash.dart';

const _publicPaths = {'/welcome', '/sign-up', '/sign-in', '/otp'};

/// Where an unauthenticated visit should go, and where a signed-in one should
/// not stay. Pure, so the rules are easy to test: returns null for "stay".
String? redirectFor({required AuthStatus status, required Uri uri}) {
  final path = uri.path;
  switch (status) {
    case AuthStatus.starting:
      return path == '/splash' ? null : '/splash';
    case AuthStatus.signedOut:
      if (_publicPaths.contains(path)) return null;
      // Remember a deep link (an order, a chat) so signing in lands on it.
      final from = path == '/splash' || path == '/' ? null : uri.toString();
      return from == null
          ? '/welcome'
          : Uri(path: '/welcome', queryParameters: {'from': from}).toString();
    case AuthStatus.signedIn:
      if (path == '/splash' || _publicPaths.contains(path)) {
        final from = uri.queryParameters['from'];
        return from != null && from.startsWith('/') ? from : '/';
      }
      return null;
  }
}

final routerProvider = Provider<GoRouter>((ref) {
  // go_router re-runs its redirects whenever this notifier fires.
  final refresh = ValueNotifier<int>(0);
  ref.listen<AuthState>(authControllerProvider, (_, _) => refresh.value++);
  ref.onDispose(refresh.dispose);

  final router = GoRouter(
    initialLocation: '/splash',
    refreshListenable: refresh,
    redirect: (context, state) => redirectFor(
      status: ref.read(authControllerProvider).status,
      uri: state.uri,
    ),
    routes: [
      GoRoute(path: '/splash', builder: (_, _) => const SplashScreen()),
      GoRoute(path: '/welcome', builder: (_, _) => const WelcomeScreen()),
      GoRoute(path: '/sign-up', builder: (_, _) => const SignUpScreen()),
      GoRoute(path: '/sign-in', builder: (_, _) => const SignInScreen()),
      GoRoute(
        path: '/otp',
        // The flow's own state travels in `extra`; without it (a stale link) start over.
        redirect: (_, state) =>
            state.extra is PendingVerification ? null : '/welcome',
        builder: (_, state) =>
            OtpScreen(pending: state.extra! as PendingVerification),
      ),
      ShellRoute(
        builder: (context, state, child) =>
            AppShell(location: state.uri.path, child: child),
        routes: [GoRoute(path: '/', builder: (_, _) => const AccountScreen())],
      ),
    ],
  );
  ref.onDispose(router.dispose);
  return router;
});
