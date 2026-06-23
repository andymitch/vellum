package com.andymitch.notes

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

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
    initAndroidContext(applicationContext)
    watchNetworkChanges()
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
