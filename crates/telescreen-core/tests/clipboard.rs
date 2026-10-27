//! CLIP-1 to CLIP-7: the clipboard sync rules of one peer.

use telescreen_core::clipboard::{
    ClipboardChange, ClipboardItem, ClipboardSync, MAX_TEXT, Outgoing,
};

fn text(text: &str) -> ClipboardChange {
    ClipboardChange::Text(ClipboardItem {
        text: text.into(),
        concealed: false,
    })
}

fn secret(text: &str) -> ClipboardChange {
    ClipboardChange::Text(ClipboardItem {
        text: text.into(),
        concealed: true,
    })
}

fn send(text: &str, concealed: bool) -> Vec<Outgoing> {
    vec![Outgoing::Send {
        text: text.into(),
        concealed,
    }]
}

#[test]
fn clip1_the_switch_starts_from_the_config() {
    assert!(!ClipboardSync::new(false).enabled());
    assert!(ClipboardSync::new(true).enabled());
}

#[test]
fn clip2_a_local_change_is_sent_only_while_the_switch_is_on() {
    let mut c = ClipboardSync::new(true);
    assert_eq!(c.local_change(&text("a")), send("a", false));
    c.set_enabled(false);
    assert_eq!(c.local_change(&text("b")), []);
    c.set_enabled(true);
    assert_eq!(c.local_change(&text("c")), send("c", false));
}

#[test]
fn clip2_a_received_text_is_applied_only_while_the_switch_is_on() {
    let mut c = ClipboardSync::new(false);
    assert!(!c.received("a"));
    assert!(!c.received_clear());
    c.set_enabled(true);
    assert!(c.received("a"));
    assert!(c.received_clear());
}

#[test]
fn clip4_ac33_the_applied_text_is_not_sent_back() {
    let mut c = ClipboardSync::new(true);
    assert!(c.received("from peer"));
    // The platform reports the change that the app made itself.
    assert_eq!(c.local_change(&text("from peer")), []);
    // A later copy of the same text by the user goes out again.
    assert_eq!(c.local_change(&text("from peer")), send("from peer", false));
}

#[test]
fn clip4_a_different_text_after_apply_is_sent() {
    let mut c = ClipboardSync::new(true);
    c.received("from peer");
    assert_eq!(c.local_change(&text("new copy")), send("new copy", false));
}

#[test]
fn clip6_large_text_is_not_sent() {
    let mut c = ClipboardSync::new(true);
    let big = "x".repeat(MAX_TEXT + 1);
    assert_eq!(
        c.local_change(&text(&big)),
        [Outgoing::TooLarge {
            bytes: MAX_TEXT + 1
        }]
    );
    let max = "x".repeat(MAX_TEXT);
    assert_eq!(c.local_change(&text(&max)), send(&max, false));
    assert!(!c.received(&big), "a large received text is not applied");
}

#[test]
fn clip6_empty_text_is_not_sent() {
    let mut c = ClipboardSync::new(true);
    assert_eq!(c.local_change(&text("")), []);
    assert_eq!(c.local_change(&ClipboardChange::Cleared), []);
}

#[test]
fn clip7_ac34_a_concealed_text_is_sent_with_the_mark() {
    let mut c = ClipboardSync::new(true);
    assert_eq!(c.local_change(&secret("hunter2")), send("hunter2", true));
}

#[test]
fn clip7_ac34_a_clear_follows_the_sent_concealed_text() {
    let mut c = ClipboardSync::new(true);
    c.local_change(&secret("hunter2"));
    assert_eq!(c.local_change(&ClipboardChange::Cleared), [Outgoing::Clear]);
    assert_eq!(c.local_change(&ClipboardChange::Cleared), [], "only once");
}

#[test]
fn clip7_an_empty_text_also_clears() {
    let mut c = ClipboardSync::new(true);
    c.local_change(&secret("hunter2"));
    assert_eq!(c.local_change(&text("")), [Outgoing::Clear]);
}

#[test]
fn clip7_a_large_text_clears_the_sent_concealed_text() {
    let mut c = ClipboardSync::new(true);
    c.local_change(&secret("hunter2"));
    let big = "x".repeat(MAX_TEXT + 1);
    assert_eq!(
        c.local_change(&text(&big)),
        [
            Outgoing::Clear,
            Outgoing::TooLarge {
                bytes: MAX_TEXT + 1
            }
        ]
    );
}

#[test]
fn clip7_no_clear_for_a_text_without_the_mark() {
    let mut c = ClipboardSync::new(true);
    c.local_change(&text("a"));
    assert_eq!(c.local_change(&ClipboardChange::Cleared), []);
}

#[test]
fn clip7_a_new_text_replaces_the_concealed_text_with_no_clear() {
    let mut c = ClipboardSync::new(true);
    c.local_change(&secret("hunter2"));
    assert_eq!(c.local_change(&text("a")), send("a", false));
    assert_eq!(c.local_change(&ClipboardChange::Cleared), []);
}

#[test]
fn clip7_a_received_text_replaces_the_sent_concealed_text() {
    let mut c = ClipboardSync::new(true);
    c.local_change(&secret("hunter2"));
    assert!(c.received("from peer"));
    assert_eq!(c.local_change(&text("from peer")), []);
    assert_eq!(c.local_change(&ClipboardChange::Cleared), []);
}

#[test]
fn clip7_a_received_clear_is_not_sent_back() {
    let mut c = ClipboardSync::new(true);
    assert!(c.received("hunter2"));
    assert_eq!(c.local_change(&secret("hunter2")), []);
    assert!(c.received_clear());
    // The platform reports the clear that the app made itself.
    assert_eq!(c.local_change(&ClipboardChange::Cleared), []);
}

#[test]
fn clip5_reset_forgets_the_sent_concealed_text() {
    let mut c = ClipboardSync::new(true);
    c.local_change(&secret("hunter2"));
    c.reset();
    assert_eq!(c.local_change(&ClipboardChange::Cleared), []);
    c.local_change(&secret("hunter2"));
    c.set_enabled(false);
    c.set_enabled(true);
    assert_eq!(c.local_change(&ClipboardChange::Cleared), []);
}
