package dev.yttt.spike

/**
 * What a debug build's launch asked for, for emulator runs: pair with this
 * code, type this line into the opened terminal, keep off direct paths. A
 * release build reads none of it (see `launchHooks` in the debug and release
 * source sets).
 */
data class LaunchHooks(
    val pair: String? = null,
    val type: String? = null,
    val relayOnly: Boolean = false,
)
