// Spike 8: a Control Center button for Telescreen (UI-10, plan step f2).
// The button runs OpenMenuIntent. Its App Intents metadata is written by hand
// in Metadata.appintents, because the tool that makes it is only in Xcode.

import AppIntents
import AppKit
import SwiftUI
import WidgetKit

struct OpenMenuIntent: AppIntent {
    static let title: LocalizedStringResource = "Open Telescreen Menu"
    static let isDiscoverable = false

    func perform() async throws -> some IntentResult {
        NSWorkspace.shared.open(URL(string: "telescreen://menu")!)
        return .result()
    }
}

struct TelescreenControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "dev.telescreen.Telescreen.Control.menu") {
            ControlWidgetButton(action: OpenMenuIntent()) {
                Label("Telescreen", systemImage: "display")
            }
        }
        .displayName("Telescreen")
        .description("Opens the Telescreen menu.")
    }
}

@main
struct TelescreenControls: WidgetBundle {
    var body: some Widget {
        TelescreenControl()
    }
}
