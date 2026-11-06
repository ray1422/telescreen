// The main window (spec UI-13): the menu content, the computers and their
// trust state, the device states, discovery, and the notices.

import SwiftUI

struct MainScreen: Decodable, Equatable {
    var menu = MenuScreen()
    var fingerprint = ""
    var computers: [ComputerRow] = []
    var devices: [DeviceRow] = []
    var discovery: DiscoveryRow?
    var notices: [String] = []
}

struct ComputerRow: Decodable, Equatable, Identifiable {
    let id: String
    let label: String
    let thisComputer: Bool
    let trusted: Bool
    let connected: Bool
}

struct DeviceRow: Decodable, Equatable, Identifiable {
    let alias: String
    let device: String
    let state: String
    var id: String { device }
}

struct DiscoveryRow: Decodable, Equatable {
    let remainingSeconds: UInt64?
}

@MainActor @Observable final class MainModel {
    var screen = MainScreen()
}

@MainActor let mainModel = MainModel()

struct MainView: View {
    let model: MainModel

    var body: some View {
        let screen = model.screen
        HStack(alignment: .top, spacing: 20) {
            ScrollView {
                MenuContent(screen: screen.menu, showsFooter: false)
                    .padding(16)
            }
            .frame(width: 380)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Section("Computers") {
                        ForEach(screen.computers) { computer in
                            ComputerLine(computer: computer)
                        }
                    }
                    Section("Devices") {
                        ForEach(screen.devices) { device in
                            HStack {
                                Text(device.alias)
                                Spacer()
                                Text(stateText(device.state)).foregroundStyle(.secondary)
                                if device.state == "unknown" {
                                    Button("It is back") {
                                        sendIntent(["kind": "device_is_back", "device": device.device])
                                    }
                                }
                            }
                        }
                    }
                    if let discovery = screen.discovery {
                        Section("Discovery") {
                            HStack {
                                if let seconds = discovery.remainingSeconds {
                                    Text("Discoverable for \(seconds / 60):\(String(format: "%02d", seconds % 60))")
                                    Spacer()
                                    Button("Stop") { sendIntent(["kind": "stop_discovery"]) }
                                } else {
                                    Text("Not discoverable")
                                    Spacer()
                                    Button("Allow discovery") { sendIntent(["kind": "allow_discovery"]) }
                                }
                            }
                        }
                    }
                    if !screen.notices.isEmpty {
                        Section("Notices") {
                            ForEach(Array(screen.notices.enumerated().reversed()), id: \.offset) { _, text in
                                Text(text).font(.callout).textSelection(.enabled)
                            }
                        }
                    }
                }
                .padding(16)
            }
        }
        .frame(minWidth: 820, minHeight: 520)
    }
}

private struct Section<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.headline)
            VStack(alignment: .leading, spacing: 8) { content }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.5),
                            in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
    }
}

private struct ComputerLine: View {
    let computer: ComputerRow

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(computer.label)
                Spacer()
                if computer.thisComputer {
                    Text("This computer").foregroundStyle(.secondary)
                } else if !computer.trusted {
                    Text("Not trusted").foregroundStyle(.orange)
                } else {
                    Text(computer.connected ? "Trusted · online" : "Trusted · offline")
                        .foregroundStyle(computer.connected ? .green : .secondary)
                }
            }
            if !computer.thisComputer && !computer.trusted {
                // UI-13: no trust in the window; a later pairing flow replaces it.
                Text("Add its fingerprint with telescreen trust \(computer.id) <fingerprint>")
                    .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
            }
        }
    }
}

private func stateText(_ state: String) -> String {
    switch state {
    case "confirmed": return "Confirmed"
    case "unverified": return "Switched, not verified"
    case "unknown": return "Unknown"
    default: return "Not on this computer"
    }
}

/// Returns a +1 retained NSHostingView with the main window content.
@_cdecl("ts_make_main_view")
public func makeMainView() -> UnsafeMutableRawPointer {
    let view: NSView = MainActor.assumeIsolated {
        NSHostingView(rootView: MainView(model: mainModel))
    }
    return Unmanaged.passRetained(view).toOpaque()
}

/// A full MainScreen as JSON (UI-18). Returns false if the JSON does not decode.
@_cdecl("ts_push_main")
public func pushMain(_ bytes: UnsafePointer<UInt8>, _ len: Int) -> Bool {
    let data = Data(bytes: bytes, count: len)
    do {
        let screen = try decoder.decode(MainScreen.self, from: data)
        MainActor.assumeIsolated { mainModel.screen = screen }
        return true
    } catch {
        print("telescreen: cannot decode the main window: \(error)")
        return false
    }
}
