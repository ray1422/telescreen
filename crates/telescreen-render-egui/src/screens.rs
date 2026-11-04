//! The layout of each screen, as functions of an `egui::Ui` and a view model.
//! They have no window or GPU code, so tests can run them on a headless context.

use telescreen_core::session::VideoInput;
use telescreen_view::{
    ControlScreen, Intent, Matching, PortalScreen, PromptScreen, SessionPhase, SetupIntent,
    SetupScreen, VideoInputText,
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
        if let Some(mut on) = screen.clipboard
            && ui.checkbox(&mut on, "Sync text clipboard").changed()
        {
            intents.push(Intent::SetClipboard { on });
        }
        for display in &screen.displays {
            if display.has_portal {
                ui.label(format!("{} (portal)", display.name));
            } else {
                ui.label(&display.name);
            }
        }
        if screen.displays.iter().any(|d| d.has_portal) {
            if ui.button("Close portal").clicked() {
                intents.push(Intent::ClosePortal);
            }
        } else if ui.button("Open portal on this display").clicked() {
            intents.push(Intent::OpenPortal);
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

pub(crate) fn portal(ui: &mut egui::Ui, screen: &PortalScreen) -> Vec<Intent> {
    let mut intents = Vec::new();
    ui.scope(|ui| {
        ui.style_mut().override_font_id = Some(egui::FontId::proportional(28.0));
        ui.spacing_mut().interact_size.y = 56.0;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(egui::RichText::new(&screen.display).size(36.0));
            match &screen.matching {
                Matching::Evaluating => {
                    ui.label("Checking this display…");
                }
                Matching::NoMatch => {
                    ui.label("No device in the config matches this display.");
                }
                Matching::Ambiguous(choices) => {
                    ui.label("More than one device matches. Select one:");
                    for choice in choices {
                        ui.push_id(&choice.alias, |ui| {
                            if ui.button(&choice.label).clicked() {
                                intents.push(Intent::SelectDevice {
                                    alias: choice.alias.clone(),
                                });
                            }
                        });
                    }
                }
                Matching::Matched(choice) => {
                    ui.label(&choice.label);
                }
                Matching::Failed(error) => {
                    ui.label(error);
                }
            }
            for candidate in &screen.candidates {
                ui.push_id(&candidate.target, |ui| {
                    if ui
                        .add_enabled(
                            candidate.unavailable.is_none(),
                            egui::Button::new(&candidate.label),
                        )
                        .clicked()
                    {
                        intents.push(Intent::StartHandoff {
                            target: candidate.target.clone(),
                        });
                    }
                    if let Some(reason) = &candidate.unavailable {
                        ui.label(reason);
                    }
                });
            }
            if let Some(session) = &screen.session {
                ui.label(format!("Target: {}", session.target));
                match &session.phase {
                    SessionPhase::Requesting => {
                        ui.label("Requesting a session…");
                    }
                    SessionPhase::HandingOff => {
                        ui.label("Handing off…");
                    }
                    SessionPhase::Verifying => {
                        ui.label("The target verifies the display.");
                    }
                    SessionPhase::WaitingForAck { key } => {
                        ui.label(
                            egui::RichText::new(format!(
                                "Press {key} or click here to take control of {}.",
                                session.target
                            ))
                            .size(48.0),
                        );
                    }
                    SessionPhase::Forwarding => {
                        ui.label("Forwarding input.");
                    }
                    SessionPhase::Ended { reason } => {
                        ui.label(format!("Session ended: {reason}."));
                    }
                }
                ui.label(format!("Video input: {}", session.video_input.text()));
                if !matches!(session.phase, SessionPhase::Ended { .. })
                    && ui.button("End session").clicked()
                {
                    intents.push(Intent::EndSession);
                }
            }
            if ui.button("Close portal").clicked() {
                intents.push(Intent::ClosePortal);
            }
            if let Some(notice) = &screen.notice {
                ui.label(notice);
            }
        });
    });
    intents
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
        CandidateRow, DeviceChoice, DeviceRow, DisplayRow, IncomingRow, SessionRow,
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
                    DisplayRow {
                        token: "built-in".into(),
                        name: "Display".into(),
                        has_portal: false,
                    },
                    DisplayRow {
                        token: "shared".into(),
                        name: "Display".into(),
                        has_portal: true,
                    },
                ],
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
