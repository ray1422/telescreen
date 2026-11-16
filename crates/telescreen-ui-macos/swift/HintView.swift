// The login hint (STG-7). Rust owns its window; this view only sends intents.
import AppKit
import SwiftUI

struct HintView: View {
    let model: MenuModel

    var body: some View {
        if let hint = model.screen.loginHint {
            VStack(alignment: .leading, spacing: 14) {
                Text("Control the login screen of \(hint.targetLabel)")
                    .font(.system(size: 15, weight: .bold))
                Text("Run this command on \(hint.targetLabel), for example through SSH.")
                    .font(.system(size: 13))
                HStack(spacing: 10) {
                    Text(hint.command)
                        .font(.system(size: 12, design: .monospaced))
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    Button("Copy") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(hint.command, forType: .string)
                    }
                }
                .padding(10)
                .background(.primary.opacity(0.05),
                            in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                Text("The command also makes \(hint.targetLabel) discoverable for 10 minutes.")
                    .font(.system(size: 12)).foregroundStyle(.secondary)
                Divider()
                switch hint.state {
                case "waiting":
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Waiting for \(hint.targetLabel)…")
                        Spacer(minLength: 0)
                        closeButton("Cancel")
                    }
                case "ready":
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(spacing: 6) {
                            Circle().fill(.green).frame(width: 6, height: 6)
                            Text("\(hint.targetLabel) is connected.")
                        }
                        Text("The display switches to \(hint.targetLabel). Your keys then go to its login screen.")
                            .font(.system(size: 12)).foregroundStyle(.secondary)
                        HStack(spacing: 6) {
                            Chip(text: "Keyboard only", tint: nil)
                            Chip(text: "Display not verified", tint: nil)
                        }
                        HStack(spacing: 8) {
                            Spacer()
                            closeButton("Cancel")
                            Button("Switch & log in") {
                                sendIntent(["kind": "switch_and_log_in", "display": hint.display,
                                            "device": hint.device, "target": hint.target])
                            }
                            .keyboardShortcut(.defaultAction)
                        }
                    }
                case "logged_in":
                    HStack(spacing: 8) {
                        Text("\(hint.targetLabel) is logged in. Use \"Switch & control\".")
                        Spacer(minLength: 0)
                        closeButton("Close")
                    }
                default:
                    EmptyView()
                }
            }
            .font(.system(size: 13))
            .padding(20)
            .frame(width: 400)
            .fixedSize(horizontal: false, vertical: true)
            .onGeometryChange(for: CGSize.self) { $0.size } action: { size in
                send(["ui": "hint_size", "width": size.width, "height": size.height])
            }
        }
    }

    private func closeButton(_ title: String) -> some View {
        Button(title) { sendIntent(["kind": "close_login_hint"]) }
    }
}
