// The Control Center button (spec UI-10, docs/spikes.md spike 8). It is a
// WidgetKit control extension in Telescreen.app. A press opens
// telescreen://menu, and the app opens its menu.
//
// scripts/make-macos-control.sh makes its App Intents metadata with the Xcode
// tool. Without Xcode it copies fallback/Metadata.appintents: change that
// copy together with OpenMenuIntent.

import AppIntents
import AppKit
import SwiftUI
import WidgetKit

struct OpenMenuIntent: AppIntent {
    static let title: LocalizedStringResource = "Open Telescreen Menu"
    static let isDiscoverable = false

    func perform() async throws -> some IntentResult {
        // The menu is a panel that does not activate the app (UI-10). An
        // active app brings its main window to the front, and the menu then
        // loses the key window and closes.
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = false
        _ = try await NSWorkspace.shared.open(
            URL(string: "telescreen://menu")!, configuration: configuration)
        return .result()
    }
}

struct TelescreenControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "dev.telescreen.Telescreen.Control.menu") {
            // UI-17: the custom SF Symbol needs actool (Xcode). Without it,
            // the button shows a system symbol.
            ControlWidgetButton(action: OpenMenuIntent()) {
                #if TELESCREEN_SYMBOL
                    Label("Telescreen", image: "telescreen.display")
                #else
                    Label("Telescreen", systemImage: "display")
                #endif
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
