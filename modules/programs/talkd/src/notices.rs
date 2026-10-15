//! Which user-visible errors have been shown. Each distinct one is sent once
//! until the mic recovers; one whose notification did not get through is
//! sent again the next time it happens.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    Sending,
    Delivered,
}

#[derive(Debug, Default)]
pub struct Notices {
    shown: BTreeMap<String, Shown>,
}

impl Notices {
    /// `message` happened: the notification to send now, if any.
    pub fn raise(&mut self, message: &str) -> Option<String> {
        if self.shown.contains_key(message) {
            return None;
        }
        self.shown.insert(message.to_owned(), Shown::Sending);
        Some(message.to_owned())
    }

    /// How sending `message` went. Undelivered, it is forgotten, so the next
    /// occurrence tries again.
    pub fn sent(&mut self, message: &str, delivered: bool) {
        match self.shown.get_mut(message) {
            Some(shown @ Shown::Sending) if delivered => *shown = Shown::Delivered,
            Some(Shown::Sending) => {
                self.shown.remove(message);
            }
            // recovered meanwhile, or already settled
            _ => {}
        }
    }

    /// The mic is back to normal: every error may be shown again.
    pub fn recover(&mut self) {
        self.shown.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_distinct_error_is_sent_once_until_recovery() {
        let mut notices = Notices::default();
        assert_eq!(notices.raise("a").as_deref(), Some("a"));
        assert_eq!(notices.raise("b").as_deref(), Some("b"));
        assert_eq!(notices.raise("a"), None, "on its way");
        notices.sent("a", true);
        notices.sent("b", true);
        assert_eq!(notices.raise("a"), None, "A/B/A: still once");
        assert_eq!(notices.raise("b"), None);
        notices.recover();
        assert_eq!(notices.raise("a").as_deref(), Some("a"));
    }

    #[test]
    fn only_a_delivered_notification_counts() {
        let mut notices = Notices::default();
        assert!(notices.raise("a").is_some());
        notices.sent("a", false); // mako not running
        assert_eq!(notices.raise("a").as_deref(), Some("a"), "tried again");
        notices.sent("a", true);
        assert_eq!(notices.raise("a"), None);
    }

    #[test]
    fn a_result_arriving_after_recovery_changes_nothing() {
        let mut notices = Notices::default();
        assert!(notices.raise("a").is_some());
        notices.recover();
        notices.sent("a", true);
        assert_eq!(notices.raise("a").as_deref(), Some("a"));
    }
}
