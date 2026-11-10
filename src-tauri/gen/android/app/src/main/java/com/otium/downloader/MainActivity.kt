package com.otium.downloader

import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import org.json.JSONObject
import java.util.Locale
import kotlin.math.max
import kotlin.math.roundToInt

class MainActivity : TauriActivity() {
  @Volatile private var insetTopPx: Float = 0f
  @Volatile private var insetBottomPx: Float = 0f
  @Volatile private var insetLeftPx: Float = 0f
  @Volatile private var insetRightPx: Float = 0f
  @Volatile private var insetImePx: Float = 0f
  @Volatile private var pendingSharedText: String? = null
  private var attachedWebView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.dark(Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.dark(Color.TRANSPARENT)
    )
    super.onCreate(savedInstanceState)
    captureIntentUrl(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    captureIntentUrl(intent)
    dispatchPendingShareToWeb()
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    attachedWebView = webView
    webView.setBackgroundColor(Color.parseColor("#0A0C10"))
    webView.addJavascriptInterface(ArgonNativeBridge(), "ArgonNative")

    ViewCompat.setOnApplyWindowInsetsListener(webView) { v, windowInsets ->
      val sys = windowInsets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val ime = windowInsets.getInsets(WindowInsetsCompat.Type.ime())
      val density = max(1f, v.resources.displayMetrics.density)

      insetTopPx = sys.top / density
      insetBottomPx = sys.bottom / density
      insetLeftPx = sys.left / density
      insetRightPx = sys.right / density
      insetImePx = max(0f, (ime.bottom - sys.bottom) / density)

      pushInsetsToWeb(webView)
      windowInsets
    }
    ViewCompat.requestApplyInsets(webView)
  }

  private fun captureIntentUrl(intent: Intent?) {
    if (intent == null) return
    val text = when (intent.action) {
      Intent.ACTION_SEND -> intent.getStringExtra(Intent.EXTRA_TEXT)
      Intent.ACTION_VIEW -> intent.dataString
      else -> null
    }?.trim()
    if (!text.isNullOrEmpty() && (text.startsWith("http://", ignoreCase = true) || text.startsWith("https://", ignoreCase = true) || text.contains("http://") || text.contains("https://"))) {
      pendingSharedText = text
    }
  }

  private fun insetsJson(): String {
    return String.format(
      Locale.US,
      "{\"top\":%d,\"bottom\":%d,\"left\":%d,\"right\":%d,\"ime\":%d}",
      insetTopPx.roundToInt(),
      insetBottomPx.roundToInt(),
      insetLeftPx.roundToInt(),
      insetRightPx.roundToInt(),
      insetImePx.roundToInt()
    )
  }

  private fun pushInsetsToWeb(webView: WebView) {
    val json = insetsJson()
    val js = """
      (function(){
        var d = $json;
        window.__ARGON_INSETS__ = d;
        var s = document.documentElement && document.documentElement.style;
        if (s) {
          s.setProperty('--native-sat', d.top + 'px');
          s.setProperty('--native-sab', d.bottom + 'px');
          s.setProperty('--native-sal', d.left + 'px');
          s.setProperty('--native-sar', d.right + 'px');
          s.setProperty('--native-ime', d.ime + 'px');
        }
        window.dispatchEvent(new CustomEvent('argon-insets', { detail: d }));
      })();
    """.trimIndent()
    webView.post { webView.evaluateJavascript(js, null) }
  }

  private fun dispatchPendingShareToWeb() {
    val webView = attachedWebView ?: return
    val text = pendingSharedText ?: return
    val quoted = JSONObject.quote(text)
    val js = "window.dispatchEvent(new CustomEvent('argon-share-url', { detail: $quoted }));"
    webView.post { webView.evaluateJavascript(js, null) }
  }

  inner class ArgonNativeBridge {
    @JavascriptInterface
    fun getInsetsJson(): String = insetsJson()

    @JavascriptInterface
    fun consumeSharedUrl(): String {
      val value = pendingSharedText ?: ""
      pendingSharedText = null
      return value
    }

    @JavascriptInterface
    fun setSystemBarTheme(light: Boolean) {
      runOnUiThread {
        val controller = WindowInsetsControllerCompat(window, window.decorView)
        controller.isAppearanceLightStatusBars = light
        controller.isAppearanceLightNavigationBars = light
        val bg = if (light) "#F8FAFC" else "#0A0C10"
        attachedWebView?.setBackgroundColor(Color.parseColor(bg))
      }
    }
  }
}
