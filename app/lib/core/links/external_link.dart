import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:url_launcher/url_launcher.dart';

/// Opens [Uri] outside the app (the browser, the mail app) and says whether it
/// could. Widgets read this instead of calling `url_launcher` themselves, so a
/// test can replace it and no platform channel is used.
typedef ExternalLinkOpener = Future<bool> Function(Uri uri);

final externalLinkOpenerProvider = Provider<ExternalLinkOpener>(
  (ref) =>
      (uri) => launchUrl(uri, mode: LaunchMode.externalApplication),
);
