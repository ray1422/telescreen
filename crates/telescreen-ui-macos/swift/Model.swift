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
    var loginHint: LoginHint?
}

struct LoginHint: Decodable, Equatable {
    let display: String
    let device: String
    let target: String
    let targetLabel: String
    let command: String
    let state: String
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
        let keysOnly: Bool
        let switchBack: Bool

        private enum CodingKeys: String, CodingKey {
            case kind, phase, videoInput, state, handBack, keysOnly, switchBack
        }

        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            kind = try values.decode(String.self, forKey: .kind)
            phase = try values.decodeIfPresent(Tagged.self, forKey: .phase)
            videoInput = try values.decodeIfPresent(String.self, forKey: .videoInput)
            state = try values.decodeIfPresent(String.self, forKey: .state)
            handBack = try values.decodeIfPresent(String.self, forKey: .handBack)
            keysOnly = try values.decodeIfPresent(Bool.self, forKey: .keysOnly) ?? false
            switchBack = try values.decodeIfPresent(Bool.self, forKey: .switchBack) ?? false
        }
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
    let switchBack: Bool
    var id: String { device }

    private enum CodingKeys: String, CodingKey {
        case device, alias, label, state, candidates, switchBack
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        device = try values.decode(String.self, forKey: .device)
        alias = try values.decode(String.self, forKey: .alias)
        label = try values.decode(String.self, forKey: .label)
        state = try values.decode(String.self, forKey: .state)
        candidates = try values.decode([MenuCandidate].self, forKey: .candidates)
        switchBack = try values.decodeIfPresent(Bool.self, forKey: .switchBack) ?? false
    }
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
    let stage: String?
    let logIn: Tagged
    var id: String { target }

    private enum CodingKeys: String, CodingKey {
        case target, label, status, switchAndControl, switchDisplayOnly, controlOnly
        case primary, wake, stage, logIn
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        target = try values.decode(String.self, forKey: .target)
        label = try values.decode(String.self, forKey: .label)
        status = try values.decode(String.self, forKey: .status)
        switchAndControl = try values.decodeIfPresent(String.self, forKey: .switchAndControl)
        switchDisplayOnly = try values.decodeIfPresent(String.self, forKey: .switchDisplayOnly)
        controlOnly = try values.decodeIfPresent(String.self, forKey: .controlOnly)
        primary = try values.decodeIfPresent(String.self, forKey: .primary)
        wake = try values.decode(Tagged.self, forKey: .wake)
        stage = try values.decodeIfPresent(String.self, forKey: .stage)
        logIn = try values.decodeIfPresent(Tagged.self, forKey: .logIn) ?? Tagged(kind: "hidden")
    }
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
