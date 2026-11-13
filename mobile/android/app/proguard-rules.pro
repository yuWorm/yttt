# The Rust core is reached through JNA, which finds the generated bindings'
# library interface, structures and callbacks by reflection and by name.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { *; }
-keep class uniffi.yttt_mobile_core.** { *; }
# JNA's desktop-only paths reference AWT, which Android does not have.
-dontwarn java.awt.**
