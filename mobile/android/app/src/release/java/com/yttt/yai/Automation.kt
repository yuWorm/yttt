package com.yttt.yai

/** Release builds read no launch hooks; see the debug source set's. */
@Suppress("UNUSED_PARAMETER")
fun launchHooks(
    extra: (String) -> String?,
    flag: (String) -> Boolean,
    decodeImage: (String) -> String?,
): LaunchHooks = LaunchHooks()
