package com.andymitch.vellum

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.os.Build
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat

class MainActivity : TauriActivity() {
  // Implemented in Rust (libnotes_lib.so). Hands the JNI VM + app context to
  // ndk_context so iroh's network monitor doesn't panic on Android.
  private external fun initAndroidContext(context: Context)

  // Tells iroh the network changed (Android doesn't surface this natively), so
  // it re-probes and migrates connections on wifi <-> cellular handoff.
  private external fun notifyNetworkChange()

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    instance = this
    initAndroidContext(applicationContext)
    watchNetworkChanges()
  }

  override fun onDestroy() {
    if (instance === this) instance = null
    super.onDestroy()
  }

  // Set system-bar icon contrast (light icons for dark UI, dark for light).
  // Bars are transparent and draw behind the web header, so contrast must
  // follow the in-app (web) theme, which Android can't see — JS pushes it here.
  private fun applySystemBarAppearance(lightIcons: Boolean) {
    runOnUiThread {
      val c = WindowCompat.getInsetsController(window, window.decorView)
      c.isAppearanceLightStatusBars = lightIcons
      c.isAppearanceLightNavigationBars = lightIcons
    }
  }

  companion object {
    @Volatile private var instance: MainActivity? = null

    // Called from Rust (set_dark_mode command) via JNI. `dark` = web theme is
    // dark -> use light (white) status-bar icons.
    @JvmStatic
    fun setDarkMode(dark: Boolean) {
      instance?.applySystemBarAppearance(!dark)
    }

    // Called from Rust (get_material_you command) via JNI. Returns the device's
    // Material You (Monet) tonal palette as a JSON object of #RRGGBB hex strings,
    // or null on pre-Android-12 (API < 31) where dynamic colors don't exist.
    // The frontend maps these tones to its theme variables for a "Dynamic" theme.
    @JvmStatic
    fun getDynamicColors(): String? {
      val ctx = instance ?: return null
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return null
      fun hex(id: Int): String = String.format("#%06X", 0xFFFFFF and ContextCompat.getColor(ctx, id))
      return buildString {
        append("{")
        val parts = listOf(
          "a1_100" to android.R.color.system_accent1_100,
          "a1_200" to android.R.color.system_accent1_200,
          "a1_300" to android.R.color.system_accent1_300,
          "a1_500" to android.R.color.system_accent1_500,
          "a1_600" to android.R.color.system_accent1_600,
          "a2_500" to android.R.color.system_accent2_500,
          "a3_500" to android.R.color.system_accent3_500,
          "n1_10" to android.R.color.system_neutral1_10,
          "n1_50" to android.R.color.system_neutral1_50,
          "n1_100" to android.R.color.system_neutral1_100,
          "n1_500" to android.R.color.system_neutral1_500,
          "n1_800" to android.R.color.system_neutral1_800,
          "n1_900" to android.R.color.system_neutral1_900,
          "n2_100" to android.R.color.system_neutral2_100,
          "n2_300" to android.R.color.system_neutral2_300,
          "n2_700" to android.R.color.system_neutral2_700,
        )
        parts.forEachIndexed { i, (k, id) ->
          if (i > 0) append(",")
          append("\"").append(k).append("\":\"").append(hex(id)).append("\"")
        }
        append("}")
      }
    }
  }

  // Notify iroh on default-network changes so it re-probes/migrates (wifi<->cellular).
  // We deliberately do NOT bindProcessToNetwork: pinning the process to a network
  // leaves DNS pointed at the old network's resolver after a wifi->cellular
  // switch, so relay hostnames fail to resolve. ACCESS_NETWORK_STATE (in the
  // manifest) is what iroh needs for its socket/network association.
  private fun watchNetworkChanges() {
    val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    cm.registerDefaultNetworkCallback(
      object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
          notifyNetworkChange()
        }

        override fun onLost(network: Network) {
          notifyNetworkChange()
        }
      },
    )
  }
}
