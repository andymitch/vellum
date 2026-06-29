# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# Rust (JNI) calls MainActivity's @JvmStatic methods (setDarkMode,
# getDynamicColors, ...) by name. R8 can't see JNI references, so in the
# minified release build it strips/renames them — they then throw
# NoSuchMethodError at the JNI call, silently breaking system-bar contrast and
# the Material You theme on installed releases (debug builds aren't minified, so
# this is invisible in `tauri android dev`). Keep the class and its members.
-keep class com.andymitch.vellum.MainActivity { *; }
