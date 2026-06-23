package com.andymitch.notes

import android.content.Context
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
  }
}
