package com.andymitch.vellum

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
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
