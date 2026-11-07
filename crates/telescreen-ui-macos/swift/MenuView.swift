// The menu (spec UI-9). It draws the MenuScreen of the core and sends intents.
// It makes no decision (ADR 0012). The layout follows the canvas mockups
// "macOS · menu, no session", "controlling a target", and "controlled by Linux".

import SwiftUI

private let corner: CGFloat = 22
/// Smoked glass: a neutral black tint (user, 2026-11-07; the mockup color
/// has a blue cast).
private let glassTint = Color.black.opacity(0.4)
/// Text levels on the glass. The system levels (`.secondary`, `.tertiary`)
/// are too dim on the dark glass (user, 2026-11-07); these follow the gray
/// levels of the mockup.
let menuSecondary = Color.primary.opacity(0.75)
let menuTertiary = Color.primary.opacity(0.6)
/// The transparent margin for the shadow (mockup: 0 18px 50px, 45 % black).
/// The system shadow follows the rectangle of the panel, not the glass, so
/// SwiftUI draws it. src/mac.rs has the same values (SHADOW_*).
private let shadowTop: CGFloat = 32
private let shadowSide: CGFloat = 50
private let shadowBottom: CGFloat = 68

struct MenuView: View {
    let model: MenuModel
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        MenuContent(screen: model.screen)
            .padding(.top, 14)
            .padding(.horizontal, 12)
            .padding(.bottom, 10)
            .frame(width: 330)
            .background {
                // UI-16: a fill that Telescreen draws becomes opaque.
                if reduceTransparency {
                    RoundedRectangle(cornerRadius: corner, style: .continuous)
                        .fill(Color(nsColor: .windowBackgroundColor))
                }
            }
            .glassEffect(.regular.tint(glassTint),
                         in: RoundedRectangle(cornerRadius: corner, style: .continuous))
            .background {
                RoundedRectangle(cornerRadius: corner, style: .continuous)
                    .fill(.black.opacity(0.001))
                    .shadow(color: .black.opacity(0.45), radius: 25, y: 18)
            }
            .padding(EdgeInsets(top: shadowTop, leading: shadowSide,
                                bottom: shadowBottom, trailing: shadowSide))
            .onGeometryChange(for: CGSize.self) { $0.size } action: { size in
                send(["ui": "menu_size", "width": size.width, "height": size.height])
            }
    }
}

/// The content of the menu panel.
struct MenuContent: View {
    let screen: MenuScreen

    var body: some View {
        VStack(alignment: .leading, spacing: screen.sessions.isEmpty ? 2 : 4) {
            HStack {
                Text("Telescreen").font(.system(size: 15, weight: .bold))
                Spacer()
                Text(screen.computer).font(.system(size: 12)).foregroundStyle(menuSecondary)
            }
            .padding(.top, 2)
            .padding(.horizontal, 6)
            .padding(.bottom, 8)

            if screen.sessions.isEmpty {
                MenuDivider().padding(.bottom, 2)
            } else {
                ForEach(screen.sessions) { session in
                    SessionCard(session: session, screen: screen)
                }
                // The clipboard switch shows only with a session (mockups).
                if let clipboard = screen.clipboard {
                    HStack {
                        Text("Sync text clipboard").font(.system(size: 13))
                        Spacer()
                        ClipboardSwitch(on: clipboard)
                    }
                    .padding(.top, 10)
                    .padding(.horizontal, 8)
                    .padding(.bottom, 6)
                }
                MenuDivider()
            }

            ForEach(Array(screen.displays.enumerated()), id: \.element.id) { index, display in
                DisplaySection(display: display, screen: screen, first: index == 0)
            }

            let others = otherControlRequests(screen)
            if !others.isEmpty {
                MenuDivider()
                ForEach(others) { row in
                    ControlRequestButton(row: row)
                }
            }

            MenuDivider()
            MenuRow(title: "Telescreen Settings…") { sendUi("open_settings") }
            MenuRow(title: "Quit Telescreen") { sendUi("quit") }
        }
    }
}

/// A peer that can control this computer but is not a candidate of a shown
/// device has its own row.
@MainActor func otherControlRequests(_ screen: MenuScreen) -> [ControlRequestRow] {
    let shown = Set(screen.displays.flatMap { display in
        display.devices.flatMap { device in
            device.candidates.map { $0.target + "/" + device.device }
        }
    })
    return screen.controlRequests.filter { !shown.contains($0.id) }
}

struct MenuDivider: View {
    var body: some View {
        Rectangle().fill(.primary.opacity(0.12)).frame(height: 1)
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
    }
}

struct ClipboardSwitch: View {
    let on: Bool

    var body: some View {
        Toggle("Sync text clipboard", isOn: Binding(
            get: { on },
            set: { sendIntent(["kind": "set_clipboard", "on": $0]) }
        ))
        .labelsHidden()
        .toggleStyle(.switch)
        .controlSize(.small)
    }
}

final class Hover: ObservableObject {
    @Published var on = false
}

private struct MenuRow: View {
    let title: String
    let action: () -> Void
    // The SwiftUI macros (`@State`) are not available to this swiftc build.
    @StateObject private var hover = Hover()

    var body: some View {
        Button(action: action) {
            Text(title).font(.system(size: 14))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 8)
                .padding(.vertical, 7)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background {
            if hover.on {
                RoundedRectangle(cornerRadius: 8, style: .continuous).fill(.primary.opacity(0.10))
            }
        }
        .onHover { hover.on = $0 }
    }
}

func phaseText(_ phase: Tagged?) -> String {
    switch phase?.kind {
    case "requesting": return "requesting"
    case "handing_off": return "switching the display"
    case "verifying": return "verifying"
    case "waiting_for_ack": return "press \(phase?.key ?? "Return") in the portal"
    case "forwarding": return "forwarding"
    case "ended": return "ended: \(phase?.reason ?? "")"
    default: return ""
    }
}

/// The chip of a video input state, or nil for "not switched".
private func videoInputChip(_ value: String?) -> (String, Color?)? {
    switch value {
    case "confirmed": return ("Display confirmed", .green)
    case "unverified": return ("Switched, not verified", nil)
    case "unknown": return ("Display state unknown", .orange)
    default: return nil
    }
}

/// The label of a device id from the displays of the menu.
func deviceLabel(_ id: String, in screen: MenuScreen) -> String {
    for display in screen.displays {
        if let device = display.devices.first(where: { $0.device == id }) {
            return device.label
        }
    }
    return id
}

private func deviceState(_ id: String, in screen: MenuScreen) -> String? {
    for display in screen.displays {
        if let device = display.devices.first(where: { $0.device == id }) {
            return device.state
        }
    }
    return nil
}

private struct Chip: View {
    let text: String
    let tint: Color?

    var body: some View {
        Text(text).font(.system(size: 11))
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .foregroundStyle(tint ?? .primary.opacity(0.85))
            .background((tint ?? .primary).opacity(tint == nil ? 0.12 : 0.22), in: Capsule())
    }
}

/// A button of a session card: equal height, 10 pt corners (mockups).
private struct CardButtonStyle: ButtonStyle {
    var fill: Color?
    var text: Color = .white
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13, weight: fill == nil ? .regular : .semibold))
            .foregroundStyle(fill == nil ? .primary : text)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(
                fill ?? .primary.opacity(0.14),
                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
            )
            .opacity(configuration.isPressed ? 0.75 : isEnabled ? 1 : 0.45)
            .contentShape(Rectangle())
    }
}

struct SessionCard: View {
    let session: MenuSession
    let screen: MenuScreen

    private var source: Bool { session.role.kind == "source" }
    private var tint: Color { source ? .accentColor : .orange }
    /// The dark text on orange of the mockup.
    private let onOrange = Color(red: 0x2a / 255, green: 0x1a / 255, blue: 0)

    private var subtitle: String {
        let device = deviceLabel(session.device, in: screen)
        let state = source ? phaseText(session.role.phase) : (session.role.state ?? "").lowercased()
        return state.isEmpty ? device : "\(device) · \(state)"
    }

    private var forwarding: Bool {
        source ? session.role.phase?.kind == "forwarding"
            : session.role.state?.lowercased() == "forwarding"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Circle().fill(tint).frame(width: 36, height: 36)
                    .overlay {
                        Image(systemName: source ? "display" : "display.and.arrow.down")
                            .font(.system(size: 17))
                            .foregroundStyle(source ? .white : onOrange)
                    }
                VStack(alignment: .leading, spacing: 0) {
                    Text(source ? "Controlling \(session.peerLabel)"
                                : "Controlled by \(session.peerLabel)")
                        .font(.system(size: 14, weight: .semibold))
                    Text(subtitle).font(.system(size: 12)).foregroundStyle(menuSecondary)
                }
                Spacer(minLength: 0)
            }
            let video = videoInputChip(
                source ? session.role.videoInput : deviceState(session.device, in: screen))
            if video != nil || forwarding {
                HStack(spacing: 6) {
                    if let (text, color) = video { Chip(text: text, tint: color) }
                    if forwarding {
                        Chip(text: source ? "Keys go to \(session.peerLabel)"
                                          : "Input from \(session.peerLabel)", tint: nil)
                    }
                }
            }
            if source {
                HStack(spacing: 8) {
                    let portal = session.id.portal ?? 0
                    Button("Hand back") {
                        sendIntent(["kind": "source_hand_back", "portal": portal])
                    }
                    .buttonStyle(CardButtonStyle(fill: .accentColor))
                    Button("End session") { sendIntent(["kind": "end_session", "portal": portal]) }
                        .buttonStyle(CardButtonStyle())
                }
                Text("Hand back shortcut ⌃⌥⌘⎋ · works when no window here is visible")
                    .font(.system(size: 11)).foregroundStyle(menuSecondary)
            } else {
                let id = session.id.session ?? 0
                Button("Hand back to \(session.peerLabel)") {
                    sendIntent(["kind": "hand_back", "session": id])
                }
                .buttonStyle(CardButtonStyle(fill: .orange, text: onOrange))
                .disabled(session.role.handBack != nil)
                .help(session.role.handBack ?? "")
                HStack(spacing: 8) {
                    Button("Control from here") {
                        sendIntent(["kind": "control_from_here", "session": id])
                    }
                    .buttonStyle(CardButtonStyle())
                    Button("Disconnect") { sendIntent(["kind": "stop_session", "session": id]) }
                        .buttonStyle(CardButtonStyle())
                }
                if let cause = session.role.handBack {
                    Text(cause).font(.system(size: 11)).foregroundStyle(menuSecondary)
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(source ? 0.22 : 0.18),
                    in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .strokeBorder(tint.opacity(0.45), lineWidth: 1)
        }
    }
}

/// The text after the display name: "in this session", or "shows this Mac".
func displayStateText(_ display: MenuDisplay, screen: MenuScreen) -> String? {
    let ids = Set(display.devices.map(\.device))
    if screen.sessions.contains(where: { ids.contains($0.device) }) {
        return "in this session"
    }
    if display.devices.contains(where: { $0.state == "confirmed" }) {
        return "shows this Mac"
    }
    return nil
}

private struct DisplaySection: View {
    let display: MenuDisplay
    let screen: MenuScreen
    let first: Bool

    private var title: String {
        guard let state = displayStateText(display, screen: screen) else { return display.name }
        return "\(display.name) · \(state)"
    }

    private func note(_ text: String) -> some View {
        Text(text).font(.system(size: 13)).foregroundStyle(menuTertiary)
            .padding(.top, 4)
            .padding(.horizontal, 8)
            .padding(.bottom, 8)
    }

    var body: some View {
        let candidates = display.devices.flatMap { device in
            device.candidates.map { (device, $0) }
        }
        VStack(alignment: .leading, spacing: 2) {
            if display.hasCandidates {
                HStack(spacing: 6) {
                    Text(title).font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(menuSecondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    RefreshButton(display: display, size: 24, corner: 7)
                }
                .padding(.top, first ? 2 : 6)
                .padding(.leading, 8)
                .padding(.trailing, 4)
            } else {
                Text(title).font(.system(size: 12, weight: .semibold)).foregroundStyle(menuSecondary)
                    .padding(.top, first ? 6 : 10)
                    .padding(.horizontal, 8)
                    .padding(.bottom, candidates.isEmpty ? 2 : 4)
            }
            switch display.matching.kind {
            case "evaluating":
                note("Checking the display…")
            case "failed":
                note(display.matching.error ?? "portal-match failed")
            default:
                if display.devices.isEmpty {
                    note("No handoff routes in the config")
                }
                ForEach(display.devices) { device in
                    if device.candidates.isEmpty {
                        note(display.devices.count > 1
                             ? "\(device.label): No handoff routes in the config"
                             : "No handoff routes in the config")
                    }
                    ForEach(device.candidates) { candidate in
                        CandidateRow(
                            display: display, device: device, candidate: candidate,
                            request: screen.controlRequests.first {
                                $0.peer == candidate.target && $0.device == device.device
                            },
                            style: .menu)
                    }
                }
            }
        }
    }
}

/// "Refresh" of a display (NET-10): dials its offline candidates. It always
/// shows; it is dim and disabled when nothing is offline.
struct RefreshButton: View {
    let display: MenuDisplay
    let size: CGFloat
    let corner: CGFloat
    @StateObject private var hover = Hover()

    var body: some View {
        Button {
            sendIntent(["kind": "refresh", "display": display.token])
        } label: {
            Image(systemName: "arrow.clockwise")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Color.primary.opacity(0.85))
                .frame(width: size, height: size)
                .background {
                    if hover.on && display.canRefresh {
                        RoundedRectangle(cornerRadius: corner, style: .continuous)
                            .fill(.primary.opacity(0.12))
                    }
                }
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!display.canRefresh)
        .opacity(display.canRefresh ? 1 : 0.35)
        .onHover { hover.on = $0 }
        .help("Refresh")
        .accessibilityLabel("Refresh \(display.name)")
    }
}

/// The actions of a candidate (ACT-1) and "Let <peer> control this display".
@MainActor struct CandidateActions {
    let display: MenuDisplay
    let device: MenuDevice
    let candidate: MenuCandidate
    let request: ControlRequestRow?

    struct Action {
        let title: String
        let kind: String
        let unavailable: String?
    }

    var actions: [Action] {
        [
            Action(title: "Switch & control", kind: "switch_and_control",
                   unavailable: candidate.switchAndControl),
            Action(title: "Switch display only", kind: "switch_display_only",
                   unavailable: candidate.switchDisplayOnly),
            Action(title: "Control only", kind: "control_only", unavailable: candidate.controlOnly),
        ]
    }

    /// The first available action (UI-18).
    var primary: Action? { actions.first { $0.unavailable == nil } }

    var online: Bool { candidate.status == "online" }

    var subtitle: String {
        let status = candidate.status.capitalized
        if let request, request.waiting { return "Waiting for \(request.peerLabel)…" }
        if let primary { return "\(status) · \(primary.title)" }
        return [status, candidate.switchAndControl].compactMap { $0 }.joined(separator: " · ")
    }

    /// The menu has more than one available item: the row shows "⋯".
    var hasMore: Bool {
        let available = actions.filter { $0.unavailable == nil }.count
        return available + (request.map { $0.waiting ? 0 : 1 } ?? 0) > 1
    }

    func run(_ action: Action) {
        sendIntent(["kind": action.kind, "display": display.token, "device": device.device,
                    "target": candidate.target])
    }

    @ViewBuilder var menuItems: some View {
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
}

/// One candidate row: a click runs the first available action, and "⋯" holds
/// all actions (UI-18). The menu and the main window have different sizes.
struct CandidateRow: View {
    enum Style { case menu, window }

    let display: MenuDisplay
    let device: MenuDevice
    let candidate: MenuCandidate
    let request: ControlRequestRow?
    let style: Style
    @StateObject private var hover = Hover()

    private func subtitleStyle(online: Bool) -> Color {
        switch (style, online) {
        case (.menu, true): return menuSecondary
        case (.menu, false): return menuTertiary
        case (.window, true): return Color.secondary
        case (.window, false): return Color(nsColor: .tertiaryLabelColor)
        }
    }

    private var model: CandidateActions {
        CandidateActions(display: display, device: device, candidate: candidate, request: request)
    }

    var body: some View {
        let model = self.model
        HStack(spacing: style == .menu ? 4 : 0) {
            Button {
                if let primary = model.primary { model.run(primary) }
            } label: {
                HStack(spacing: 10) {
                    if style == .menu {
                        Circle()
                            .fill(model.online ? Color.accentColor : .primary.opacity(0.14))
                            .frame(width: 30, height: 30)
                            .overlay {
                                Image(systemName: "display").font(.system(size: 14))
                                    .foregroundStyle(model.online ? Color.white : Color.secondary)
                            }
                    }
                    VStack(alignment: .leading, spacing: style == .menu ? 0 : 2) {
                        Text(candidate.label)
                            .font(.system(size: style == .menu ? 14 : 13))
                            .foregroundStyle(model.online ? Color.primary : style == .menu ? menuSecondary : Color.secondary)
                        Text(model.subtitle).font(.system(size: 12))
                            .foregroundStyle(subtitleStyle(online: model.online))
                    }
                    Spacer(minLength: 0)
                }
                .padding(.vertical, style == .menu ? 6 : 0)
                .padding(.leading, style == .menu ? 8 : 14)
                .padding(.trailing, model.hasMore ? 0 : (style == .menu ? 8 : 14))
                .frame(minHeight: style == .menu ? nil : 48)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(model.primary == nil)
            if model.hasMore {
                let size: CGFloat = style == .menu ? 32 : 44
                Menu {
                    model.menuItems
                } label: {
                    Image(systemName: "ellipsis")
                        .font(.system(size: 13, weight: .semibold))
                        .frame(width: size, height: size)
                        .background {
                            if hover.on && style == .menu {
                                RoundedRectangle(cornerRadius: 8, style: .continuous)
                                    .fill(.primary.opacity(0.18))
                            }
                        }
                        .contentShape(Rectangle())
                }
                .menuStyle(.button)
                .buttonStyle(.plain)
                .menuIndicator(.hidden)
                .fixedSize()
                .padding(.trailing, style == .menu ? 6 : 2)
                .accessibilityLabel("More actions for \(candidate.label)")
            }
        }
        .background {
            if hover.on {
                if style == .menu {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(.primary.opacity(0.10))
                } else {
                    Rectangle().fill(.primary.opacity(0.05))
                }
            }
        }
        .onHover { hover.on = $0 }
        .contextMenu { if model.hasMore { model.menuItems } }
    }
}

struct ControlRequestButton: View {
    let row: ControlRequestRow

    private var title: String {
        if row.waiting { return "Waiting for \(row.peerLabel)…" }
        return "Let \(row.peerLabel) control \(row.deviceLabel)"
    }

    var body: some View {
        Button {
            sendIntent(["kind": "let_control", "peer": row.peer, "device": row.device])
        } label: {
            Text(title).font(.system(size: 14))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 8)
                .padding(.vertical, 7)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(row.waiting)
    }
}
