// The main window (spec UI-13): the menu content, the computers and their
// trust state, the device states, discovery, and the notices. The layout
// follows the canvas mockup "macOS · main window": one 14 pt inset for the
// group titles, the row text, and the row dividers.

import AppKit
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

private let inset: CGFloat = 14

struct MainView: View {
    let model: MainModel

    var body: some View {
        let screen = model.screen
        VStack(spacing: 0) {
            // Two layers (mockup Version 75): the title bar and the header in one
            // lighter layer, then the content a step darker.
            Header(screen: screen)
                .padding(.top, 14)
                .padding(.horizontal, 28 + inset)
                .padding(.bottom, 22)
                .frame(maxWidth: .infinity)
                .background(.primary.opacity(0.05), ignoresSafeAreaEdges: .top)
                .overlay(alignment: .bottom) {
                    Rectangle().fill(.primary.opacity(0.07)).frame(height: 1)
                }
            ScrollView {
                Grid(alignment: .topLeading, horizontalSpacing: 28) {
                    GridRow {
                        LeftColumn(screen: screen)
                            .gridColumnAlignment(.leading)
                            .containerRelativeFrame(.horizontal) { width, _ in
                                // The left column is 1.2 times the right column.
                                (width - 2 * 28 - 28) * 1.2 / 2.2
                            }
                        RightColumn(screen: screen)
                    }
                }
                .padding(.top, 22)
                .padding(.horizontal, 28)
                .padding(.bottom, 24)
            }
        }
        .frame(minWidth: 820, minHeight: 520, alignment: .top)
    }
}

private struct Header: View {
    let screen: MainScreen

    private var state: String {
        guard let session = screen.menu.sessions.first else { return "No session" }
        return session.role.kind == "source" ? "Controlling \(session.peerLabel)"
            : "Controlled by \(session.peerLabel)"
    }

    var body: some View {
        HStack(spacing: 16) {
            // The icon file has the transparent margin of the Apple grid (the
            // body is 824 of 1024). The body is 72 pt, as in the mockup.
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 72 * 1024 / 824, height: 72 * 1024 / 824)
                .frame(width: 72, height: 72)
                .accessibilityLabel("Telescreen app icon")
            VStack(alignment: .leading, spacing: 2) {
                Text("Telescreen").font(.system(size: 24, weight: .bold)).kerning(-0.3)
                Text("\(screen.menu.computer) · \(state)")
                    .font(.system(size: 13)).foregroundStyle(.secondary)
            }
            Spacer()
            HStack(spacing: 8) {
                // ADR 0014: the pairing flow comes later. Until then, the
                // button shows where it goes.
                Button {
                } label: {
                    Label("Add computer…", systemImage: "plus")
                        .labelStyle(.titleAndIcon)
                        .padding(.horizontal, 12)
                }
                .buttonStyle(TitleButtonStyle())
                .disabled(true)
                .help("Pairing is not available yet. Use telescreen trust.")
                Button {
                    sendUi("open_settings")
                } label: {
                    Image(systemName: "gearshape").frame(width: 28)
                }
                .buttonStyle(TitleButtonStyle())
                .keyboardShortcut(",", modifiers: .command)
                .help("Settings")
                .accessibilityLabel("Settings")
            }
        }
    }
}

/// A 28 pt high gray button with 7 pt corners (mockup title row).
private struct TitleButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13))
            .frame(height: 28)
            .background(.primary.opacity(configuration.isPressed ? 0.18 : 0.10),
                        in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .opacity(isEnabled ? 1 : 0.5)
            .contentShape(Rectangle())
    }
}

/// A group: a title with the 14 pt inset over a card with rows.
private struct Card<Trailing: View, Content: View>: View {
    let title: String
    @ViewBuilder let trailing: Trailing
    @ViewBuilder let content: Content

    init(_ title: String, @ViewBuilder trailing: () -> Trailing = { EmptyView() },
         @ViewBuilder content: () -> Content) {
        self.title = title
        self.trailing = trailing()
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(title).font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary)
                Spacer()
                trailing
            }
            .padding(.horizontal, inset)
            VStack(alignment: .leading, spacing: 0) { content }
                .frame(maxWidth: .infinity, alignment: .leading)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .background(.primary.opacity(0.05),
                            in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(.primary.opacity(0.07), lineWidth: 1)
                }
        }
    }
}

private struct RowDivider: View {
    var body: some View {
        Rectangle().fill(.primary.opacity(0.07)).frame(height: 1).padding(.horizontal, inset)
    }
}

/// A row of one line, 40 pt high.
private struct Line<Trailing: View>: View {
    let title: String
    @ViewBuilder let trailing: Trailing

    var body: some View {
        HStack {
            Text(title).font(.system(size: 13))
            Spacer()
            trailing
        }
        .padding(.horizontal, inset)
        .frame(minHeight: 40)
    }
}

/// A row of two lines, 48 pt high.
private struct TwoLines<Trailing: View>: View {
    let title: String
    let subtitle: String
    @ViewBuilder let trailing: Trailing

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 13))
                Text(subtitle).font(.system(size: 12)).foregroundStyle(.tertiary)
            }
            Spacer()
            trailing
        }
        .padding(.horizontal, inset)
        .padding(.vertical, 6)
        .frame(minHeight: 48)
    }
}

// ----- The left column: what the user does (UI-9 with no footer) -----

private struct LeftColumn: View {
    let screen: MainScreen

    var body: some View {
        let menu = screen.menu
        VStack(alignment: .leading, spacing: 20) {
            ForEach(menu.sessions) { session in
                SessionCard(session: session, screen: menu)
            }
            ForEach(menu.displays) { display in
                DisplayGroup(display: display, screen: menu)
            }
            let others = otherControlRequests(menu)
            if !others.isEmpty {
                Card("Other displays") {
                    ForEach(Array(others.enumerated()), id: \.element.id) { index, row in
                        if index > 0 { RowDivider() }
                        Button {
                            sendIntent(["kind": "let_control", "peer": row.peer, "device": row.device])
                        } label: {
                            Line(title: row.waiting ? "Waiting for \(row.peerLabel)…"
                                 : "Let \(row.peerLabel) control \(row.deviceLabel)") { EmptyView() }
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(row.waiting)
                    }
                }
            }
            if !screen.notices.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Notices").font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.secondary).padding(.horizontal, inset)
                    ForEach(Array(screen.notices.enumerated().reversed()), id: \.offset) { _, text in
                        Text(text).font(.system(size: 12)).lineSpacing(3).textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, inset)
                            .padding(.vertical, 10)
                            .background(Color.orange.opacity(0.10),
                                        in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                            .overlay {
                                RoundedRectangle(cornerRadius: 10, style: .continuous)
                                    .strokeBorder(Color.orange.opacity(0.35), lineWidth: 1)
                            }
                    }
                }
            }
        }
    }
}

private struct DisplayGroup: View {
    let display: MenuDisplay
    let screen: MenuScreen

    private func note(_ text: String) -> some View {
        Text(text).font(.system(size: 12)).foregroundStyle(.tertiary)
            .padding(.horizontal, inset)
            .frame(maxWidth: .infinity, minHeight: 40, alignment: .leading)
    }

    var body: some View {
        let state = displayStateText(display, screen: screen).map { $0.prefix(1).uppercased() + $0.dropFirst() }
        Card(display.name) {
            if let state {
                Text(state).font(.system(size: 12)).foregroundStyle(.tertiary)
            }
        } content: {
            switch display.matching.kind {
            case "evaluating":
                note("Checking the display…")
            case "failed":
                note(display.matching.error ?? "portal-match failed")
            default:
                let rows = display.devices.flatMap { device in
                    device.candidates.map { (device, $0) }
                }
                if rows.isEmpty {
                    note("No handoff routes in the config")
                }
                ForEach(Array(rows.enumerated()), id: \.element.1.id) { index, row in
                    if index > 0 { RowDivider() }
                    CandidateRow(
                        display: display, device: row.0, candidate: row.1,
                        request: screen.controlRequests.first {
                            $0.peer == row.1.target && $0.device == row.0.device
                        },
                        style: .window)
                }
            }
        }
    }
}

// ----- The right column: the state and the options of this computer -----

private struct RightColumn: View {
    let screen: MainScreen

    private func discoveryText(_ seconds: UInt64?) -> String {
        guard let seconds else { return "Other computers cannot find this Mac" }
        return "Other computers can find this Mac · \(seconds / 60):\(String(format: "%02d", seconds % 60)) left"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            if screen.menu.clipboard != nil || screen.discovery != nil {
                Card("This Mac") {
                    if let clipboard = screen.menu.clipboard {
                        Line(title: "Sync text clipboard") {
                            ClipboardSwitch(on: clipboard).tint(.green)
                        }
                    }
                    if let discovery = screen.discovery {
                        if screen.menu.clipboard != nil { RowDivider() }
                        TwoLines(title: "Discoverable",
                                 subtitle: discoveryText(discovery.remainingSeconds)) {
                            Toggle("Discoverable", isOn: Binding(
                                get: { discovery.remainingSeconds != nil },
                                set: { on in
                                    sendIntent(["kind": on ? "allow_discovery" : "stop_discovery"])
                                }
                            ))
                            .labelsHidden()
                            .toggleStyle(.switch)
                            .controlSize(.small)
                            .tint(.green)
                        }
                    }
                }
            }
            Card("Computers") {
                ForEach(Array(screen.computers.enumerated()), id: \.element.id) { index, computer in
                    if index > 0 { RowDivider() }
                    ComputerLine(computer: computer)
                }
            }
            if !screen.devices.isEmpty {
                Card("Devices") {
                    ForEach(Array(screen.devices.enumerated()), id: \.element.id) { index, device in
                        if index > 0 { RowDivider() }
                        Line(title: device.alias) {
                            HStack(spacing: 10) {
                                Text(stateText(device.state)).font(.system(size: 12))
                                    .foregroundStyle(.secondary)
                                if device.state == "unknown" {
                                    Button("It is back") {
                                        sendIntent(["kind": "device_is_back", "device": device.device])
                                    }
                                    .controlSize(.small)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

private struct ComputerLine: View {
    let computer: ComputerRow

    private var status: (String, Color) {
        if computer.thisComputer { return ("This computer", .secondary) }
        if !computer.trusted { return ("Not trusted", .orange) }
        return computer.connected ? ("Trusted · online", .green) : ("Trusted · offline", .secondary)
    }

    var body: some View {
        let (text, color) = status
        if !computer.thisComputer && !computer.trusted {
            // UI-13: no trust in the window; a later pairing flow replaces it.
            VStack(alignment: .leading, spacing: 2) {
                HStack {
                    Text(computer.label).font(.system(size: 13))
                    Spacer()
                    Text(text).font(.system(size: 12)).foregroundStyle(color)
                }
                Text("Add its fingerprint with \(Text("telescreen trust").font(.system(size: 11, design: .monospaced)))")
                    .font(.system(size: 11)).foregroundStyle(.tertiary).textSelection(.enabled)
            }
            .padding(.horizontal, inset)
            .padding(.vertical, 6)
            .frame(minHeight: 48)
        } else {
            Line(title: computer.label) {
                Text(text).font(.system(size: 12)).foregroundStyle(color)
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
