//! Which user-visible errors have been shown. Each distinct one is sent once
//! until its mic recovers; one whose notification did not get through is
//! sent again the next time it happens. Errors about a dormant restore have
//! that restore as their subject and recover with it alone; the rest recover
//! with the active cycle. Two restores' errors never pass for one, even when
//! their mics share a description and so a message.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    Sending,
    Delivered,
}

/// One error about one subject: a dormant restore's node.name, or None.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Notice {
    pub subject: Option<String>,
    pub message: String,
}

impl Notice {
    /// The notification body: the message as text, since mako reads body
    /// markup and a mic's description is whatever the device says.
    pub fn body(&self) -> String {
        let mut body = String::with_capacity(self.message.len());
        for c in self.message.chars() {
            match c {
                '&' => body.push_str("&amp;"),
                '<' => body.push_str("&lt;"),
                '>' => body.push_str("&gt;"),
                c => body.push(c),
            }
        }
        body
    }
}

#[derive(Debug, Default)]
pub struct Notices {
    shown: BTreeMap<Notice, Shown>,
}

impl Notices {
    /// `message` about `subject` happened: the notification to send now, if any.
    pub fn raise(&mut self, subject: Option<&str>, message: &str) -> Option<Notice> {
        let notice = Notice {
            subject: subject.map(str::to_owned),
            message: message.to_owned(),
        };
        if self.shown.contains_key(&notice) {
            return None;
        }
        self.shown.insert(notice.clone(), Shown::Sending);
        Some(notice)
    }

    /// How sending `notice` went. Undelivered, it is forgotten, so the next
    /// occurrence tries again.
    pub fn sent(&mut self, notice: &Notice, delivered: bool) {
        match self.shown.get_mut(notice) {
            Some(shown @ Shown::Sending) if delivered => *shown = Shown::Delivered,
            Some(Shown::Sending) => {
                self.shown.remove(notice);
            }
            // recovered meanwhile, or already settled
            _ => {}
        }
    }

    /// The mic is back to normal: the active cycle's (None) or a dormant
    /// restore's. Its errors may be shown again.
    pub fn recover(&mut self, subject: Option<&str>) {
        self.shown
            .retain(|notice, _| notice.subject.as_deref() != subject);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(notice: Option<Notice>) -> Option<String> {
        notice.map(|notice| notice.message)
    }

    fn notice(message: &str) -> Notice {
        Notice {
            subject: None,
            message: message.to_owned(),
        }
    }

    #[test]
    fn each_distinct_error_is_sent_once_until_recovery() {
        let mut notices = Notices::default();
        assert_eq!(message(notices.raise(None, "a")).as_deref(), Some("a"));
        assert_eq!(message(notices.raise(None, "b")).as_deref(), Some("b"));
        assert_eq!(notices.raise(None, "a"), None, "on its way");
        notices.sent(&notice("a"), true);
        notices.sent(&notice("b"), true);
        assert_eq!(notices.raise(None, "a"), None, "A/B/A: still once");
        assert_eq!(notices.raise(None, "b"), None);
        notices.recover(None);
        assert_eq!(message(notices.raise(None, "a")).as_deref(), Some("a"));
    }

    #[test]
    fn only_a_delivered_notification_counts() {
        let mut notices = Notices::default();
        assert!(notices.raise(None, "a").is_some());
        notices.sent(&notice("a"), false); // mako not running
        assert_eq!(
            message(notices.raise(None, "a")).as_deref(),
            Some("a"),
            "tried again"
        );
        notices.sent(&notice("a"), true);
        assert_eq!(notices.raise(None, "a"), None);
    }

    #[test]
    fn a_result_arriving_after_recovery_changes_nothing() {
        let mut notices = Notices::default();
        assert!(notices.raise(None, "a").is_some());
        notices.recover(None);
        notices.sent(&notice("a"), true);
        assert_eq!(message(notices.raise(None, "a")).as_deref(), Some("a"));
    }

    #[test]
    fn a_restore_recovering_lets_only_its_own_errors_be_shown_again() {
        let mut notices = Notices::default();
        assert!(notices.raise(Some("airpods"), "airpods gone").is_some());
        assert!(notices.raise(Some("headset"), "headset gone").is_some());
        assert!(notices.raise(None, "cycle").is_some());
        notices.recover(Some("airpods"));
        assert!(notices.raise(Some("airpods"), "airpods gone").is_some());
        assert_eq!(notices.raise(Some("headset"), "headset gone"), None);
        assert_eq!(notices.raise(None, "cycle"), None);
        notices.recover(None);
        assert!(notices.raise(None, "cycle").is_some());
        assert_eq!(notices.raise(Some("headset"), "headset gone"), None);
    }

    #[test]
    fn two_mics_with_one_description_are_two_notices() {
        let mut notices = Notices::default();
        let gone = "Headset, which talkd flipped, is gone";
        let left = notices.raise(Some("bluez_input.left"), gone).unwrap();
        let right = notices.raise(Some("bluez_input.right"), gone);
        assert!(right.is_some(), "not taken for the first one's");
        notices.sent(&left, true);
        notices.sent(&right.unwrap(), true);
        notices.recover(Some("bluez_input.left"));
        assert!(notices.raise(Some("bluez_input.left"), gone).is_some());
        assert_eq!(notices.raise(Some("bluez_input.right"), gone), None);
    }

    #[test]
    fn a_body_shows_markup_as_text() {
        assert_eq!(
            notice("<b>Nic's</b> & AirPods is gone").body(),
            "&lt;b&gt;Nic's&lt;/b&gt; &amp; AirPods is gone"
        );
    }
}
