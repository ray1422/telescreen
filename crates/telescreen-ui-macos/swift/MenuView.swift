// The menu (spec UI-9). It draws the MenuScreen of the core and sends intents.
// It makes no decision (ADR 0012).

import SwiftUI

private let corner: CGFloat = 22

struct MenuView: View {
    let model: MenuModel
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        MenuContent(screen: model.screen)
            .padding(12)
            .frame(width: 330)
            .background {
                // UI-16: a fill that Telescreen draws becomes opaque.
                if reduceTransparency {
                    RoundedRectangle(cornerRadius: corner, style: .continuous)
                        .fill(Color(nsColor: .windowBackgroundColor))
                }
            }
            .glassEffect(.regular, in: RoundedRectangle(cornerRadius: corner, style: .continuous))
            .onGeometryChange(for: CGSize.self) { $0.size } action: { size in
                send(["ui": "menu_size", "width": size.width, "height": size.height])
            }
    }
}

/// The content of the menu. The main window shows it too (UI-13).
struct MenuContent: View {
    let screen: MenuScreen
    var showsFooter = true

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Telescreen").font(.headline)
                Spacer()
                Text(screen.computer).font(.caption).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 8)
            .padding(.bottom, 4)

            ForEach(screen.sessions) { session in
                SessionCard(session: session)
            }

            ForEach(screen.displays) { display in
                DisplaySection(display: display, requests: screen.controlRequests)
            }

            // A peer that can control this computer but is not a candidate of a
            // shown device has its own row.
            let shown = Set(screen.displays.flatMap { display in
                display.devices.flatMap { device in
                    device.candidates.map { $0.target + "/" + device.device }
                }
            })
            let others = screen.controlRequests.filter { !shown.contains($0.id) }
            if !others.isEmpty {
                Divider().padding(.vertical, 2)
                ForEach(others) { row in
                    ControlRequestButton(row: row, withDevice: true)
                }
            }

            Divider().padding(.vertical, 2)
            if let clipboard = screen.clipboard {
                Toggle(isOn: Binding(
                    get: { clipboard },
                    set: { sendIntent(["kind": "set_clipboard", "on": $0]) }
                )) {
                    Text("Sync text clipboard")
                }
                .toggleStyle(.switch)
                .controlSize(.small)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
            }
            if showsFooter {
                MenuRow(title: "Telescreen Settings…") { sendUi("open_settings") }
                MenuRow(title: "Quit Telescreen") { sendUi("quit") }
            }
        }
    }
}

private struct MenuRow: View {
    let title: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title).frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .contentShape(Rectangle())
    }
}

private func phaseText(_ phase: Tagged?) -> String {
    switch phase?.kind {
    case "requesting": return "Requesting"
    case "handing_off": return "Switching the display"
    case "verifying": return "Verifying"
    case "waiting_for_ack": return "Press \(phase?.key ?? "Return") here to confirm"
    case "forwarding": return "Forwarding"
    case "ended": return "Ended: \(phase?.reason ?? "")"
    default: return ""
    }
}

private func videoInputText(_ value: String?) -> String {
    switch value {
    case "confirmed": return "Display confirmed"
    case "unverified": return "Switched, not verified"
    case "unknown": return "Display state unknown"
    default: return "Not switched"
    }
}

private struct SessionCard: View {
    let session: MenuSession

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if session.role.kind == "source" {
                Text("Controlling \(session.peerLabel)").font(.subheadline.weight(.semibold))
                Text("\(phaseText(session.role.phase)) · \(videoInputText(session.role.videoInput))")
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("Hand back") { sendIntent(["kind": "source_hand_back"]) }
                        .buttonStyle(.glassProminent)
                    Button("End session") { sendIntent(["kind": "end_session"]) }
                        .buttonStyle(.glass)
                }
            } else {
                let id = session.id.session ?? 0
                Text("Controlled by \(session.peerLabel)").font(.subheadline.weight(.semibold))
                Text(session.role.state ?? "").font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("Hand back") { sendIntent(["kind": "hand_back", "session": id]) }
                        .buttonStyle(.glassProminent)
                        .disabled(session.role.handBack != nil)
                        .help(session.role.handBack ?? "")
                    Button("Control from here") {
                        sendIntent(["kind": "control_from_here", "session": id])
                    }
                    .buttonStyle(.glass)
                    Button("Disconnect") { sendIntent(["kind": "stop_session", "session": id]) }
                        .buttonStyle(.glass)
                }
                if let cause = session.role.handBack {
                    Text(cause).font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            Color.accentColor.opacity(0.18),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
    }
}

private struct DisplaySection: View {
    let display: MenuDisplay
    let requests: [ControlRequestRow]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(display.name).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                .padding(.horizontal, 8)
                .padding(.top, 6)
            switch display.matching.kind {
            case "evaluating":
                Text("Checking the display…").font(.callout).foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
            case "failed":
                Text(display.matching.error ?? "portal-match failed").font(.callout)
                    .foregroundStyle(.secondary).padding(.horizontal, 8)
            default:
                if display.devices.isEmpty {
                    Text("No handoff routes in the config").font(.callout)
                        .foregroundStyle(.secondary).padding(.horizontal, 8)
                }
                ForEach(display.devices) { device in
                    if device.candidates.isEmpty {
                        Text("No handoff routes in the config").font(.callout)
                            .foregroundStyle(.secondary).padding(.horizontal, 8)
                    }
                    ForEach(device.candidates) { candidate in
                        CandidateCard(
                            display: display, device: device, candidate: candidate,
                            request: requests.first {
                                $0.peer == candidate.target && $0.device == device.device
                            })
                    }
                }
            }
        }
    }
}

private final class Hover: ObservableObject {
    @Published var on = false
}

private struct CandidateCard: View {
    let display: MenuDisplay
    let device: MenuDevice
    let candidate: MenuCandidate
    /// "Let <peer> control this display" in the "⋯" menu of that peer (UI-18).
    let request: ControlRequestRow?
    // The SwiftUI macros (`@State`) are not available to this swiftc build.
    @StateObject private var hover = Hover()

    private struct Action {
        let title: String
        let kind: String
        let unavailable: String?
    }

    private var actions: [Action] {
        [
            Action(title: "Switch & control", kind: "switch_and_control",
                   unavailable: candidate.switchAndControl),
            Action(title: "Switch display only", kind: "switch_display_only",
                   unavailable: candidate.switchDisplayOnly),
            Action(title: "Control only", kind: "control_only", unavailable: candidate.controlOnly),
        ]
    }

    /// The first available action (UI-18).
    private var primary: Action? { actions.first { $0.unavailable == nil } }

    private var subtitle: String {
        let status = candidate.status.capitalized
        if let request, request.waiting { return "Waiting for \(request.peerLabel)…" }
        if let primary { return "\(status) · \(primary.title)" }
        return [status, candidate.switchAndControl].compactMap { $0 }.joined(separator: " · ")
    }

    /// The menu has more than one available item: the row shows "⋯".
    private var hasMore: Bool {
        let available = actions.filter { $0.unavailable == nil }.count
        return available + (request.map { $0.waiting ? 0 : 1 } ?? 0) > 1
    }

    private func run(_ action: Action) {
        sendIntent(["kind": action.kind, "display": display.token, "device": device.device,
                    "target": candidate.target])
    }

    @ViewBuilder private var menuItems: some View {
        ForEach(actions, id: \.kind) { action in
            Button {
                run(action)
            } label: {
                if let reason = action.unavailable {
                    Text(action.title)
                    Text(reason)
                } else {
                    Text(action.title)
                }
            }
            .disabled(action.unavailable != nil)
        }
        if let request {
            Divider()
            Button("Let \(request.peerLabel) control this display") {
                sendIntent(["kind": "let_control", "peer": request.peer, "device": request.device])
            }
            .disabled(request.waiting)
        }
    }

    var body: some View {
        HStack(spacing: 4) {
            Button {
                if let primary { run(primary) }
            } label: {
                VStack(alignment: .leading, spacing: 1) {
                    Text(candidate.label)
                    Text(subtitle).font(.caption).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(primary == nil)
            if hasMore {
                Menu {
                    menuItems
                } label: {
                    Image(systemName: "ellipsis")
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .menuStyle(.button)
                .buttonStyle(.plain)
                .menuIndicator(.hidden)
                .fixedSize()
                .opacity(hover.on ? 1 : 0.5)
                .accessibilityLabel("More actions for \(candidate.label)")
            }
        }
        .padding(.leading, 8)
        .background {
            if hover.on {
                RoundedRectangle(cornerRadius: 8, style: .continuous).fill(.primary.opacity(0.08))
            }
        }
        .onHover { hover.on = $0 }
        .contextMenu { if hasMore { menuItems } }
    }
}

private struct ControlRequestButton: View {
    let row: ControlRequestRow
    /// A row outside a candidate card names the device.
    let withDevice: Bool

    private var title: String {
        if row.waiting { return "Waiting for \(row.peerLabel)…" }
        let target = withDevice ? row.deviceLabel : "this display"
        return "Let \(row.peerLabel) control \(target)"
    }

    var body: some View {
        Button {
            sendIntent(["kind": "let_control", "peer": row.peer, "device": row.device])
        } label: {
            Text(title).frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(.plain)
        .disabled(row.waiting)
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
    }
}
