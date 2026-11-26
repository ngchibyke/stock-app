package ng.market.market

import android.view.WindowManager
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        // While a handover code is on screen the window is kept out of screenshots and
        // the recent-apps picture.
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ng.market.market/secure")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "on" -> window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
                    "off" -> window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
                    else -> {
                        result.notImplemented()
                        return@setMethodCallHandler
                    }
                }
                result.success(null)
            }
    }
}
