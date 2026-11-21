import 'package:flutter/widgets.dart';
import 'package:package_info_plus/package_info_plus.dart';

import 'app.dart';
import 'core/config/app_gate.dart';
import 'core/session/session_store.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(
    MarketRoot(
      overrides: [
        // Production-only providers; tests use plain overrides instead.
        sessionStoreProvider.overrideWithValue(const SecureSessionStore()),
        packageVersionProvider.overrideWith(
          (ref) async => (await PackageInfo.fromPlatform()).version,
        ),
      ],
    ),
  );
}
