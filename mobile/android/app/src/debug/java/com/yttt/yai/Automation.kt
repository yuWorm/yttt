package com.yttt.yai

/**
 * Debug builds only: automation for emulator runs, from the launch extras
 * (`adb shell am start -e …`).
 * - `pair`: a pairing code; with a Host paired already, adds one more, as
 *   "添加 Host" does.
 * - `qrImage`: a picture in the app's files to pair from, read as the image
 *   picker reads one, when there is no `pair`.
 * - `type`: a line to type into the terminal opened first, after taking control.
 * - `relayOnly` (boolean): keep off direct paths, through the Host's relay.
 */
fun launchHooks(
    extra: (String) -> String?,
    flag: (String) -> Boolean,
    decodeImage: (String) -> String?,
): LaunchHooks = LaunchHooks(
    pair = extra("pair") ?: extra("qrImage")?.let(decodeImage),
    type = extra("type"),
    relayOnly = flag("relayOnly"),
)
