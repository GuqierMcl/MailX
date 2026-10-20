import AppKit
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 4,
      let pid = Int32(CommandLine.arguments[1]),
      let x = Double(CommandLine.arguments[2]),
      let y = Double(CommandLine.arguments[3]) else {
    fputs("Expected pid x y\n", stderr)
    exit(2)
}
guard CGPreflightPostEventAccess() else {
    fputs("Native pointer input requires Accessibility permission; no synthetic fallback was used.\n", stderr)
    exit(3)
}
NSRunningApplication(processIdentifier: pid)?.activate(options: [.activateAllWindows])
Thread.sleep(forTimeInterval: 0.25)
let position = CGPoint(x: x, y: y)
CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: position, mouseButton: .left)?.post(tap: .cghidEventTap)
CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: position, mouseButton: .left)?.post(tap: .cghidEventTap)
CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: position, mouseButton: .left)?.post(tap: .cghidEventTap)
