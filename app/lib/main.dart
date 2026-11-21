import 'package:flutter/widgets.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'app.dart';
import 'core/config/app_gate.dart';
import 'core/session/session_store.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  const store = SecureSessionStore();
  // Before the session is restored, so a token from an earlier install is
  // never used.
  await clearSessionLeftByEarlierInstall(
    await SharedPreferences.getInstance(),
    store,
  );
  runApp(
    MarketRoot(
      overrides: [
        // Production-only providers; tests use plain overrides instead.
        sessionStoreProvider.overrideWithValue(store),
        packageVersionProvider.overrideWith(
          (ref) async => (await PackageInfo.fromPlatform()).version,
        ),
      ],
    ),
  );
}
