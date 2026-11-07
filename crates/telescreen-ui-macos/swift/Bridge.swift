// The C ABI between the Rust main loop and SwiftUI (ADR 0012, spike 7).
// Rust makes the windows; Swift makes the hosting views. Rust calls these
// functions on the main thread only.

import AppKit
import SwiftUI

public typealias EventCallback = @convention(c) (UnsafePointer<CChar>) -> Void

@MainActor var eventCallback: EventCallback?

/// Sends one JSON object to Rust: {"intent": {...}} or {"ui": "<command>", ...}.
@MainActor func send(_ object: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: object),
          let text = String(data: data, encoding: .utf8) else { return }
    text.withCString { eventCallback?($0) }
}

@MainActor func sendIntent(_ intent: [String: Any]) {
    send(["intent": intent])
}

@MainActor func sendUi(_ command: String) {
    send(["ui": command])
}

@MainActor @Observable final class MenuModel {
    var screen = MenuScreen()
}

@MainActor let menuModel = MenuModel()

@_cdecl("ts_set_event_callback")
public func setEventCallback(_ callback: EventCallback) {
    MainActor.assumeIsolated { eventCallback = callback }
}

/// Returns a +1 retained NSHostingView with the menu.
@_cdecl("ts_make_menu_view")
public func makeMenuView() -> UnsafeMutableRawPointer {
    let view: NSView = MainActor.assumeIsolated {
        let view = NSHostingView(rootView: MenuView(model: menuModel))
        view.sizingOptions = [.intrinsicContentSize]
        return view
    }
    return Unmanaged.passRetained(view).toOpaque()
}

/// A full MenuScreen as JSON (UI-18). Returns false if the JSON does not decode.
@_cdecl("ts_push_menu")
public func pushMenu(_ bytes: UnsafePointer<UInt8>, _ len: Int) -> Bool {
    let data = Data(bytes: bytes, count: len)
    do {
        let screen = try decoder.decode(MenuScreen.self, from: data)
        MainActor.assumeIsolated { menuModel.screen = screen }
        return true
    } catch {
        print("telescreen: cannot decode the menu: \(error)")
        return false
    }
}
