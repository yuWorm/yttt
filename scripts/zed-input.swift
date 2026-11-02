import AppKit
import CoreGraphics
// usage: drv <pid> activate | key <code> [cmd,shift,ctrl,alt] | text <s> | move x y | click x y | rclick x y
let a = CommandLine.arguments
let pid = pid_t(a[1])!
func flags(_ s: String) -> CGEventFlags { var f = CGEventFlags(); for p in s.split(separator: ",") { switch p { case "cmd": f.insert(.maskCommand); case "shift": f.insert(.maskShift); case "ctrl": f.insert(.maskControl); case "alt": f.insert(.maskAlternate); default: break } }; return f }
let src = CGEventSource(stateID: .hidSystemState)
func key(_ c: CGKeyCode, _ f: CGEventFlags) { for down in [true,false] { let e = CGEvent(keyboardEventSource: src, virtualKey: c, keyDown: down)!; e.flags = f; e.post(tap: .cghidEventTap); usleep(20000) } }
func mouse(_ t: CGEventType, _ x: Double, _ y: Double, _ b: CGMouseButton = .left) { CGEvent(mouseEventSource: src, mouseType: t, mouseCursorPosition: CGPoint(x:x,y:y), mouseButton: b)!.post(tap: .cghidEventTap) }
switch a[2] {
case "activate": _ = NSRunningApplication(processIdentifier: pid)?.activate(options: [.activateAllWindows])
case "key": key(CGKeyCode(a[3])!, a.count > 4 ? flags(a[4]) : CGEventFlags())
case "text": for ch in a[3].utf16 { var c = ch; for down in [true,false] { let e = CGEvent(keyboardEventSource: src, virtualKey: 0, keyDown: down)!; e.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c); e.post(tap: .cghidEventTap); usleep(15000) } }
case "move": mouse(.mouseMoved, Double(a[3])!, Double(a[4])!)
case "click": let x=Double(a[3])!, y=Double(a[4])!; mouse(.mouseMoved,x,y); usleep(80000); mouse(.leftMouseDown,x,y); usleep(40000); mouse(.leftMouseUp,x,y)
case "rclick": let x=Double(a[3])!, y=Double(a[4])!; mouse(.mouseMoved,x,y); usleep(80000); mouse(.rightMouseDown,x,y,.right); usleep(40000); mouse(.rightMouseUp,x,y,.right)
default: break }
