// Spike 7 (ADR 0012): a SwiftUI view in an NSHostingView, in a window that the
// Rust main loop makes. Rust pushes a view model as JSON, and the view sends
// intents back through a C callback.

import AppKit
import SwiftUI

struct Row: Decodable, Identifiable {
    let id: String
    let label: String
    let detail: String
}

struct Snapshot: Decodable {
    let seq: UInt64
    let title: String
    let rows: [Row]
}

@MainActor @Observable final class Model {
    var snapshot = Snapshot(seq: 0, title: "Telescreen", rows: [])
}

public typealias IntentCallback = @convention(c) (UnsafePointer<CChar>) -> Void

@MainActor var callback: IntentCallback?
@MainActor let model = Model()

@MainActor func send(_ json: String) {
    json.withCString { callback?($0) }
}

struct MenuView: View {
    let model: Model

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(model.snapshot.title).font(.headline)
                Spacer()
                Text("#\(model.snapshot.seq)").font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
            ForEach(model.snapshot.rows) { row in
                HStack {
                    VStack(alignment: .leading) {
                        Text(row.label)
                        Text(row.detail).font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Switch & control") {
                        send(#"{"intent":"switch_and_control","row":"\#(row.id)"}"#)
                    }
                    .buttonStyle(.glass)
                }
                .padding(10)
                .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            }
        }
        .padding(14)
        .frame(width: 330)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        // Rust owns the window: tell it the size that SwiftUI wants.
        .onGeometryChange(for: CGSize.self) { $0.size } action: { size in
            send(#"{"intent":"size","w":\#(size.width),"h":\#(size.height)}"#)
        }
        // Tell Rust each time SwiftUI applied a new snapshot, to measure the round trip.
        .onChange(of: model.snapshot.seq) { _, seq in
            send(#"{"intent":"applied","seq":\#(seq)}"#)
        }
    }
}

@_cdecl("ts_make_view")
public func makeView(_ cb: IntentCallback) -> UnsafeMutableRawPointer {
    let view: NSView = MainActor.assumeIsolated {
        callback = cb
        let view = NSHostingView(rootView: MenuView(model: model))
        view.sizingOptions = [.intrinsicContentSize]
        return view
    }
    return Unmanaged.passRetained(view).toOpaque()
}

/// Any thread. The decode runs on the caller thread; the model changes on the
/// main thread.
@_cdecl("ts_push_model")
public func pushModel(_ bytes: UnsafePointer<UInt8>, _ len: Int) {
    let data = Data(bytes: bytes, count: len)
    guard let snapshot = try? JSONDecoder().decode(Snapshot.self, from: data) else {
        print("swift: bad snapshot")
        return
    }
    if Thread.isMainThread {
        MainActor.assumeIsolated { model.snapshot = snapshot }
    } else {
        DispatchQueue.main.async { model.snapshot = snapshot }
    }
}

/// Any thread.
@_cdecl("ts_quit")
public func quit() {
    DispatchQueue.main.async { NSApp.terminate(nil) }
}
