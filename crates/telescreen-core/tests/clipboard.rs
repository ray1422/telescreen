//! CLIP-1 to CLIP-7: the clipboard sync rules of one peer.

use telescreen_core::clipboard::{ClipboardItem, ClipboardSync, MAX_TEXT, Outgoing};

fn item(text: &str) -> ClipboardItem {
    ClipboardItem {
        text: text.into(),
        concealed: false,
    }
}

#[test]
fn clip1_the_switch_starts_from_the_config() {
    assert!(!ClipboardSync::new(false).enabled());
    assert!(ClipboardSync::new(true).enabled());
}

#[test]
fn clip2_a_local_change_is_sent_only_while_the_switch_is_on() {
    let mut c = ClipboardSync::new(true);
    assert_eq!(c.local_change(&item("a")), Some(Outgoing::Send("a".into())));
    c.set_enabled(false);
    assert_eq!(c.local_change(&item("b")), None);
    c.set_enabled(true);
    assert_eq!(c.local_change(&item("c")), Some(Outgoing::Send("c".into())));
}

#[test]
fn clip2_a_received_text_is_applied_only_while_the_switch_is_on() {
    let mut c = ClipboardSync::new(false);
    assert!(!c.received("a"));
    c.set_enabled(true);
    assert!(c.received("a"));
}

#[test]
fn clip4_ac33_the_applied_text_is_not_sent_back() {
    let mut c = ClipboardSync::new(true);
    assert!(c.received("from peer"));
    // The platform reports the change that the app made itself.
    assert_eq!(c.local_change(&item("from peer")), None);
    // A later copy of the same text by the user goes out again.
    assert_eq!(
        c.local_change(&item("from peer")),
        Some(Outgoing::Send("from peer".into()))
    );
}

#[test]
fn clip4_a_different_text_after_apply_is_sent() {
    let mut c = ClipboardSync::new(true);
    c.received("from peer");
    assert_eq!(
        c.local_change(&item("new copy")),
        Some(Outgoing::Send("new copy".into()))
    );
}

#[test]
fn clip6_large_text_is_not_sent() {
    let mut c = ClipboardSync::new(true);
    let big = "x".repeat(MAX_TEXT + 1);
    assert_eq!(
        c.local_change(&item(&big)),
        Some(Outgoing::TooLarge {
            bytes: MAX_TEXT + 1
        })
    );
    let max = "x".repeat(MAX_TEXT);
    assert!(matches!(
        c.local_change(&item(&max)),
        Some(Outgoing::Send(_))
    ));
    assert!(!c.received(&big), "a large received text is not applied");
}

#[test]
fn clip7_ac34_concealed_items_are_not_sent() {
    let mut c = ClipboardSync::new(true);
    let secret = ClipboardItem {
        text: "hunter2".into(),
        concealed: true,
    };
    assert_eq!(c.local_change(&secret), None);
}

#[test]
fn clip6_empty_text_is_not_sent() {
    let mut c = ClipboardSync::new(true);
    assert_eq!(c.local_change(&item("")), None);
}
