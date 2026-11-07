//! The layout of each screen, as functions of an `egui::Ui` and a view model.
//! They have no window or GPU code, so tests can run them on a headless context.

use telescreen_core::session::VideoInput;
use telescreen_view::{
    ControlScreen, DisplayMatching, Intent, Matching, PortalScreen, PromptScreen, SessionPhase,
    SetupIntent, SetupScreen, VideoInputText,
};

pub(crate) fn setup(ui: &mut egui::Ui, screen: &SetupScreen) -> Vec<SetupIntent> {
    let mut intents = Vec::new();
    egui::ScrollArea::vertical().show(ui, |ui| {
        if let Some(problem) = &screen.problem {
            ui.heading("Telescreen");
            ui.add(egui::Label::new(problem).selectable(true));
            if ui.button("Check again").clicked() {
                intents.push(SetupIntent::CheckAgain);
            }
        } else {
            ui.heading("Which computer is this?");
            if let Some(notice) = &screen.notice {
                ui.label(notice);
            }
            ui.add(egui::Label::new(format!("Config: {}", screen.config_path)).selectable(true));
            for computer in &screen.computers {
                let text = match &computer.label {
                    Some(label) => format!("{label} ({})", computer.id),
                    None => computer.id.to_string(),
                };
                if ui.button(text).clicked() {
                    intents.push(SetupIntent::Choose {
                        computer: computer.id.clone(),
                    });
                }
            }
        }
    });
    intents
}

pub(crate) fn control(ui: &mut egui::Ui, screen: &ControlScreen) -> Vec<Intent> {
    let mut intents = Vec::new();
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Not a choice of computer: the label of this peer.
        ui.heading(format!("This computer: {}", screen.computer));
        ui.add(egui::Label::new(&screen.fingerprint).selectable(true));
        if let Some(discovery) = screen.discovery {
            ui.horizontal(|ui| {
                if let Some(seconds) = discovery.remaining_seconds {
                    ui.label(format!(
                        "Discoverable for {}:{:02}",
                        seconds / 60,
                        seconds % 60
                    ));
                    if ui.button("Stop").clicked() {
                        intents.push(Intent::StopDiscovery);
                    }
                } else if ui.button("Allow discovery").clicked() {
                    intents.push(Intent::AllowDiscovery);
                }
            });
        }
        if let Some(mut on) = screen.clipboard
            && ui.checkbox(&mut on, "Sync text clipboard").changed()
        {
            intents.push(Intent::SetClipboard { on });
        }
        if let Some(session) = &screen.session {
            ui.horizontal(|ui| {
                ui.label(format!("Controlling {}", session.target));
                ui.label(phase_text(&session.phase));
                if !matches!(session.phase, SessionPhase::Ended { .. }) {
                    if ui.button("Hand back").clicked() {
                        intents.push(Intent::SourceHandBack { portal: None });
                    }
                    if ui.button("End session").clicked() {
                        intents.push(Intent::EndSession { portal: None });
                    }
                }
            });
        }
        // ACT-1, ACT-5: each action names the display, the device, and the target.
        for display in &screen.displays {
            ui.push_id(&display.token, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(&display.name);
                    // NET-10: dial the offline candidates of this display.
                    let has_candidates = display.devices.iter().any(|d| !d.candidates.is_empty());
                    if has_candidates
                        && ui
                            .add_enabled(display.can_refresh, egui::Button::new("Refresh"))
                            .clicked()
                    {
                        intents.push(Intent::Refresh {
                            display: display.token.clone(),
                        });
                    }
                });
                match &display.matching {
                    DisplayMatching::Evaluating => {
                        ui.label("Checking this display…");
                    }
                    DisplayMatching::Failed { error } => {
                        ui.label(error);
                    }
                    DisplayMatching::Done if display.devices.is_empty() => {
                        ui.label("No device in the config matches this display.");
                    }
                    DisplayMatching::Done => {}
                }
                for device in &display.devices {
                    ui.push_id(&device.device, |ui| {
                        ui.label(&device.label);
                        if device.candidates.is_empty() {
                            ui.label("No handoff routes in the config");
                        }
                        for candidate in &device.candidates {
                            ui.push_id(&candidate.target, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(format!("{} ({})", candidate.label, candidate.status));
                                    let actions = [
                                        ("Switch & control", &candidate.switch_and_control),
                                        ("Switch display only", &candidate.switch_display_only),
                                        ("Control only", &candidate.control_only),
                                    ];
                                    for (index, (label, unavailable)) in
                                        actions.into_iter().enumerate()
                                    {
                                        let button = ui
                                            .add_enabled(
                                                unavailable.is_none(),
                                                egui::Button::new(label),
                                            )
                                            .on_disabled_hover_text(
                                                unavailable.as_deref().unwrap_or(""),
                                            );
                                        if button.clicked() {
                                            let (display, device, target) = (
                                                display.token.clone(),
                                                device.device.clone(),
                                                candidate.target.clone(),
                                            );
                                            intents.push(match index {
                                                0 => Intent::SwitchAndControl {
                                                    display,
                                                    device,
                                                    target,
                                                },
                                                1 => Intent::SwitchDisplayOnly {
                                                    display,
                                                    device,
                                                    target,
                                                },
                                                _ => Intent::ControlOnly {
                                                    display,
                                                    device,
                                                    target,
                                                },
                                            });
                                        }
                                    }
                                });
                            });
                        }
                    });
                }
            });
        }
        ui.heading("Devices");
        for device in &screen.devices {
            ui.push_id(&device.device, |ui| {
                ui.horizontal(|ui| {
                    ui.label(&device.alias);
                    ui.label(device.state.text());
                    if device.state == VideoInput::Unknown && ui.button("It is back").clicked() {
                        intents.push(Intent::DeviceIsBack {
                            device: device.device.clone(),
                        });
                    }
                });
            });
        }
        for incoming in &screen.incoming {
            ui.push_id(incoming.session, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Controlled by {}", incoming.source));
                    ui.label(&incoming.state);
                    if ui
                        .add_enabled(incoming.hand_back.is_none(), egui::Button::new("Hand back"))
                        .clicked()
                    {
                        intents.push(Intent::HandBack {
                            session: incoming.session,
                        });
                    }
                    if let Some(cause) = &incoming.hand_back {
                        ui.label(cause);
                    }
                    if ui.button("Control from here").clicked() {
                        intents.push(Intent::ControlFromHere {
                            session: incoming.session,
                        });
                    }
                    if ui.button("Disconnect").clicked() {
                        intents.push(Intent::StopSession {
                            session: incoming.session,
                        });
                    }
                });
            });
        }
        for notice in &screen.notices {
            ui.label(notice);
        }
    });
    intents
}

/// ACT-5: the portal shows no candidates and at most one status line.
pub(crate) fn portal(ui: &mut egui::Ui, screen: &PortalScreen) -> Vec<Intent> {
    let line = match (&screen.session, &screen.notice) {
        (Some(session), _) => match &session.phase {
            SessionPhase::WaitingForAck { key } => format!(
                "Press {key} or click here to take control of {}.",
                session.target
            ),
            phase => phase_text(phase),
        },
        (None, Some(notice)) => notice.clone(),
        (None, None) => match &screen.matching {
            Matching::Failed(error) => error.clone(),
            _ => "Starting…".into(),
        },
    };
    ui.add_space((ui.available_height() / 2.0 - 32.0).max(0.0));
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(line).size(48.0));
    });
    Vec::new()
}

fn phase_text(phase: &SessionPhase) -> String {
    match phase {
        SessionPhase::Requesting => "Requesting a session…".into(),
        SessionPhase::HandingOff => "Handing off…".into(),
        SessionPhase::Verifying => "The target verifies the display.".into(),
        SessionPhase::WaitingForAck { key } => format!("Press {key} in the portal."),
        SessionPhase::Forwarding => "Forwarding input.".into(),
        SessionPhase::Ended { reason } => format!("Session ended: {reason}."),
    }
}

pub(crate) fn prompt(ui: &mut egui::Ui, screen: &PromptScreen) {
    let width = ui.available_width();
    let color = ui.visuals().text_color();
    let title = screen.title().map(|title| {
        ui.painter()
            .layout(title, egui::FontId::proportional(48.0), color, width)
    });
    let text = screen.text.as_ref().map(|text| {
        ui.painter()
            .layout(text.clone(), egui::FontId::proportional(28.0), color, width)
    });
    let height = title.as_ref().map_or(0.0, |title| title.size().y)
        + text.as_ref().map_or(0.0, |text| text.size().y)
        + if title.is_some() && text.is_some() {
            ui.spacing().item_spacing.y
        } else {
            0.0
        };
    ui.add_space(((ui.available_height() - height) / 2.0).max(0.0));
    ui.vertical_centered(|ui| {
        if let Some(title) = title {
            ui.label(title);
        }
        if let Some(text) = text {
            ui.label(text);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use telescreen_core::{ids::SessionId, session::VideoInput};
    use telescreen_view::{
        CandidateRow, DeviceChoice, DeviceRow, DiscoveryRow, IncomingRow, MenuCandidate,
        MenuDevice, MenuDisplay, SessionRow,
    };

    fn assert_idle_frame(
        context: &egui::Context,
        mut screen: impl FnMut(&mut egui::Ui) -> Vec<Intent>,
    ) {
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                assert!(screen(ui).is_empty());
            });
        });
        output.drop_without_applying_deltas();
    }

    #[test]
    fn setup_variants_have_no_intents_without_input() {
        let context = egui::Context::default();
        let screen = SetupScreen {
            config_path: "/config.yaml".into(),
            notice: Some("Select this computer again.".into()),
            computers: vec![
                telescreen_view::ComputerChoice {
                    id: "mac".into(),
                    label: Some("Laptop".into()),
                },
                telescreen_view::ComputerChoice {
                    id: "linux".into(),
                    label: None,
                },
            ],
            ..Default::default()
        };
        for screen in [
            screen,
            SetupScreen {
                problem: Some("There is no config at /config.yaml.".into()),
                ..Default::default()
            },
        ] {
            assert_idle_frame(&context, |ui| {
                assert!(setup(ui, &screen).is_empty());
                Vec::new()
            });
        }
    }

    #[test]
    fn control_variants_have_no_intents_without_input() {
        let context = egui::Context::default();
        let screens = [
            ControlScreen::default(),
            ControlScreen {
                computer: "This peer".into(),
                fingerprint: "ab:cd:ef:01".into(),
                displays: vec![
                    MenuDisplay {
                        token: "built-in".into(),
                        name: "Display".into(),
                        matching: DisplayMatching::Done,
                        devices: Vec::new(),
                        can_refresh: false,
                    },
                    MenuDisplay {
                        token: "evaluating".into(),
                        name: "Display".into(),
                        matching: DisplayMatching::Evaluating,
                        devices: Vec::new(),
                        can_refresh: false,
                    },
                    MenuDisplay {
                        token: "failed".into(),
                        name: "Display".into(),
                        matching: DisplayMatching::Failed {
                            error: "no EDID".into(),
                        },
                        devices: Vec::new(),
                        can_refresh: false,
                    },
                    MenuDisplay {
                        token: "shared".into(),
                        name: "Display".into(),
                        matching: DisplayMatching::Done,
                        can_refresh: true,
                        devices: vec![
                            MenuDevice {
                                device: "no-route".into(),
                                alias: "Other".into(),
                                label: "Other".into(),
                                state: VideoInput::Unchanged,
                                candidates: Vec::new(),
                            },
                            MenuDevice {
                                device: "shared".into(),
                                alias: "Monitor".into(),
                                label: "Monitor".into(),
                                state: VideoInput::Confirmed,
                                candidates: vec![MenuCandidate {
                                    target: "pc1".into(),
                                    label: "Target peer".into(),
                                    status: "offline".into(),
                                    switch_and_control: Some("Target peer is offline".into()),
                                    switch_display_only: None,
                                    control_only: Some("Target peer is offline".into()),
                                }],
                            },
                        ],
                    },
                ],
                session: Some(SessionRow {
                    target: "Target peer".into(),
                    phase: SessionPhase::Forwarding,
                    video_input: VideoInput::Confirmed,
                }),
                devices: vec![
                    DeviceRow {
                        alias: "Monitor".into(),
                        device: "shared".into(),
                        state: VideoInput::Unchanged,
                    },
                    DeviceRow {
                        alias: "Hub".into(),
                        device: "hub".into(),
                        state: VideoInput::Unknown,
                    },
                ],
                incoming: vec![
                    IncomingRow {
                        session: SessionId(1),
                        source: "Source peer".into(),
                        state: "Forwarding".into(),
                        hand_back: None,
                    },
                    IncomingRow {
                        session: SessionId(2),
                        source: "Source peer".into(),
                        state: "Paused".into(),
                        hand_back: Some("No route back to Source peer in this config".into()),
                    },
                ],
                notices: vec!["Older notice.".into(), "Newer notice.".into()],
                clipboard: Some(true),
                discovery: Some(DiscoveryRow {
                    remaining_seconds: Some(90),
                }),
            },
            ControlScreen {
                discovery: Some(DiscoveryRow {
                    remaining_seconds: None,
                }),
                ..ControlScreen::default()
            },
        ];
        for screen in screens {
            assert_idle_frame(&context, |ui| control(ui, &screen));
        }
    }

    #[test]
    fn portal_matching_variants_have_no_intents_without_input() {
        let context = egui::Context::default();
        let choice = DeviceChoice {
            alias: "shared".into(),
            label: "Shared display".into(),
        };
        let matching = [
            Matching::Evaluating,
            Matching::NoMatch,
            Matching::Ambiguous(Vec::new()),
            Matching::Ambiguous(vec![
                choice.clone(),
                DeviceChoice {
                    alias: "other".into(),
                    label: choice.label.clone(),
                },
            ]),
            Matching::Matched(choice),
            Matching::Failed("Could not read the display.".into()),
        ];
        assert_idle_frame(&context, |ui| portal(ui, &PortalScreen::default()));
        for matching in matching {
            let screen = PortalScreen {
                display: "Shared display".into(),
                matching,
                candidates: vec![
                    CandidateRow {
                        target: "available".into(),
                        label: "Target peer".into(),
                        unavailable: None,
                    },
                    CandidateRow {
                        target: "unavailable".into(),
                        label: "Target peer".into(),
                        unavailable: Some("This peer is offline.".into()),
                    },
                ],
                show_refresh: true,
                can_refresh: true,
                notice: Some("A display changed.".into()),
                ..Default::default()
            };
            assert_idle_frame(&context, |ui| portal(ui, &screen));
        }
    }

    #[test]
    fn portal_session_variants_have_no_intents_without_input() {
        let context = egui::Context::default();
        let phases = [
            SessionPhase::Requesting,
            SessionPhase::HandingOff,
            SessionPhase::Verifying,
            SessionPhase::WaitingForAck {
                key: "Return".into(),
            },
            SessionPhase::Forwarding,
            SessionPhase::Ended {
                reason: "The target disconnected".into(),
            },
        ];
        for phase in phases {
            for video_input in [
                VideoInput::Unchanged,
                VideoInput::Unverified,
                VideoInput::Confirmed,
                VideoInput::Unknown,
            ] {
                let screen = PortalScreen {
                    display: "Shared display".into(),
                    session: Some(SessionRow {
                        target: "Target peer".into(),
                        phase: phase.clone(),
                        video_input,
                    }),
                    ..Default::default()
                };
                assert_idle_frame(&context, |ui| portal(ui, &screen));
            }
        }
    }

    #[test]
    fn prompt_variants_render_without_input() {
        let context = egui::Context::default();
        for text in [
            None,
            Some(String::new()),
            Some("**Copy**: Control+C\nPaste: Control+V".into()),
        ] {
            for ack_wanted in [false, true] {
                // `None`: the acknowledgement is on this computer (a hand back).
                for source in [Some("Source peer".to_string()), None] {
                    let screen = PromptScreen {
                        source,
                        confirm_key: "Return".into(),
                        text: text.clone(),
                        ack_wanted,
                        portal_hidden: false,
                        blank: false,
                    };
                    assert_idle_frame(&context, |ui| {
                        prompt(ui, &screen);
                        Vec::new()
                    });
                }
            }
        }
    }
}
