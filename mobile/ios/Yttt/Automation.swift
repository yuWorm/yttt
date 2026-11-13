import Foundation

/// Debug builds only: automation for simulator runs, from the environment
/// (`SIMCTL_CHILD_<name>`). A Release build compiles none of it, not even the
/// names, so every field stays empty there.
struct LaunchHooks: Equatable {
    /// Pair with this code, or the one in this QR code image; with a Host
    /// paired already, adds one more, as "添加 Host" does.
    var pairCode: String?
    var qrImage: String?
    /// Switch to the paired Host whose id ends so, or forget the current one,
    /// as the Host menu does.
    var switchTo: String?
    var forget = false
    /// Open this terminal, then take control and type this line.
    var open: String?
    var type: String?
    /// Open this project (and tab, pane) once the overview has it; show the
    /// left drawer, the right drawer, or this file in it, for screenshots.
    var project: String?
    var tab: String?
    var pane: String?
    var drawer = false
    var files = false
    var edit: String?
    /// Keep off direct paths: everything goes through the Host's relay.
    var relayOnly = false

    init(environment: [String: String]) {
        #if DEBUG
        func value(_ name: String) -> String? { environment["YTTT_DEBUG_" + name] }
        func flag(_ name: String) -> Bool { value(name) == "1" }
        pairCode = value("PAIR_CODE")
        qrImage = value("QR_IMAGE")
        switchTo = value("SWITCH")
        forget = flag("FORGET")
        open = value("OPEN")
        type = value("TYPE")
        project = value("PROJECT")
        tab = value("TAB")
        pane = value("PANE")
        drawer = flag("DRAWER")
        files = flag("FILES")
        edit = value("EDIT")
        relayOnly = flag("RELAY_ONLY")
        #endif
    }

    static let current = LaunchHooks(environment: ProcessInfo.processInfo.environment)
}
