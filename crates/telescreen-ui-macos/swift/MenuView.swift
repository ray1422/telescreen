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
            .padding(.horizontal, 6)
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
                .padding(.horizontal, 6)
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

private struct CandidateCard: View {
    let display: MenuDisplay
    let device: MenuDevice
    let candidate: MenuCandidate
    /// "Let <peer> control this display" in the card of that peer (UI-9).
    let request: ControlRequestRow?

    private func intent(_ kind: String) -> [String: Any] {
        ["kind": kind, "display": display.token, "device": device.device, "target": candidate.target]
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                VStack(alignment: .leading, spacing: 1) {
                    Text(candidate.label)
                    Text(candidate.status.capitalized).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
            }
            .padding(.bottom, 4)
            ActionButton(title: "Switch & control", unavailable: candidate.switchAndControl,
                         prominent: true) { sendIntent(intent("switch_and_control")) }
            ActionButton(title: "Switch display only", unavailable: candidate.switchDisplayOnly,
                         prominent: false) { sendIntent(intent("switch_display_only")) }
            ActionButton(title: "Control only", unavailable: candidate.controlOnly,
                         prominent: false) { sendIntent(intent("control_only")) }
            if let request {
                ControlRequestButton(row: request, withDevice: false)
            }
        }
        .padding(10)
        .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

private struct ActionButton: View {
    let title: String
    let unavailable: String?
    let prominent: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack {
                Text(title)
                Spacer()
                if let reason = unavailable {
                    Text(reason).font(.caption).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background {
            if prominent && unavailable == nil {
                RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color.accentColor)
            }
        }
        .foregroundStyle(prominent && unavailable == nil ? Color.white : Color.primary)
        .disabled(unavailable != nil)
        .opacity(unavailable == nil ? 1 : 0.55)
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
