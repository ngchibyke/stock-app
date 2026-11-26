import 'package:flutter/services.dart';

/// Keeps the window out of screenshots and the recent-apps picture while a secret
/// (a handover code) is on screen. Android only; elsewhere, and in tests, it does
/// nothing.
abstract final class SecureScreen {
  static const MethodChannel _channel = MethodChannel(
    'ng.market.market/secure',
  );

  static Future<void> set(bool on) async {
    try {
      await _channel.invokeMethod<void>(on ? 'on' : 'off');
    } on MissingPluginException {
      // Not Android, or a test.
    } on PlatformException {
      // The window could not be changed; the code still shows.
    }
  }
}
