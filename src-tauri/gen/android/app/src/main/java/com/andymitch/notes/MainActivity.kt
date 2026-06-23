package com.andymitch.notes

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkRequest
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  // Implemented in Rust (libnotes_lib.so). Hands the JNI VM + app context to
  // ndk_context so iroh's network monitor doesn't panic on Android.
  private external fun initAndroidContext(context: Context)

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    initAndroidContext(applicationContext)
    bindProcessToDefaultNetwork()
  }

  // Bind the whole process to the active network. Without this, iroh's UDP
  // sockets aren't associated with the cellular network: IPv6 sends fail with
  // EPERM and the IPv4 CLAT path fails with EIO (GSO), so the relay never
  // connects off-wifi. Keep the binding current as networks change.
  private fun bindProcessToDefaultNetwork() {
    val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    cm.activeNetwork?.let { cm.bindProcessToNetwork(it) }
    cm.registerDefaultNetworkCallback(
      object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
          cm.bindProcessToNetwork(network)
        }
      },
    )
  }
}
