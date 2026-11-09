// The view models of telescreen-view as JSON (spec UI-18). The Rust tests in
// crates/telescreen-view/tests/json.rs fix this shape.

import Foundation

struct MenuScreen: Decodable, Equatable {
    var computer: String = ""
    var glyph: String = "idle"
    var sessions: [MenuSession] = []
    var displays: [MenuDisplay] = []
    var controlRequests: [ControlRequestRow] = []
    var clipboard: Bool?
}

struct Tagged: Decodable, Equatable {
    let kind: String
    var session: UInt64?
    /// The portal of a source session row (FLOW-25).
    var portal: UInt64?
    var key: String?
    var reason: String?
    var error: String?
}

struct MenuSession: Decodable, Equatable, Identifiable {
    let id: Tagged
    let peer: String
    let peerLabel: String
    let device: String
    let role: Role

    struct Role: Decodable, Equatable {
        let kind: String
        var phase: Tagged?
        var videoInput: String?
        var state: String?
        var handBack: String?
    }
}

extension Tagged: Hashable {}

struct MenuDisplay: Decodable, Equatable, Identifiable {
    let token: String
    let name: String
    let matching: Tagged
    let devices: [MenuDevice]
    let canRefresh: Bool
    var id: String { token }
    /// The display has candidates, so it shows "Refresh" (NET-10, mockup 81).
    var hasCandidates: Bool { devices.contains { !$0.candidates.isEmpty } }
}

struct MenuDevice: Decodable, Equatable, Identifiable {
    let device: String
    let alias: String
    let label: String
    let state: String
    let candidates: [MenuCandidate]
    var id: String { device }
}

struct MenuCandidate: Decodable, Equatable, Identifiable {
    let target: String
    let label: String
    let status: String
    let switchAndControl: String?
    let switchDisplayOnly: String?
    let controlOnly: String?
    let primary: String?
    let wake: Tagged
    var id: String { target }
}

struct ControlRequestRow: Decodable, Equatable, Identifiable {
    let peer: String
    let peerLabel: String
    let device: String
    let deviceLabel: String
    let waiting: Bool
    var id: String { peer + "/" + device }
}

let decoder: JSONDecoder = {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return decoder
}()
