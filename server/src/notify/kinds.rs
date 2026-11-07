//! The fourteen kinds of notice and their fixed copy. Notices store no text:
//! the title and body come from the kind when they are read or pushed.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    OrderPaid,
    OrderPickedUp,
    OrderOutForDelivery,
    OrderDelivered,
    ConfirmReminder,
    ReleaseTomorrow,
    RefundStarted,
    FundsReleased,
    ChatMessage,
    DisputeOpened,
    ReturnRequested,
    DisputeDecided,
    ModerationWarning,
    ModerationMuted,
}

impl Kind {
    pub const ALL: [Kind; 14] = [
        Kind::OrderPaid,
        Kind::OrderPickedUp,
        Kind::OrderOutForDelivery,
        Kind::OrderDelivered,
        Kind::ConfirmReminder,
        Kind::ReleaseTomorrow,
        Kind::RefundStarted,
        Kind::FundsReleased,
        Kind::ChatMessage,
        Kind::DisputeOpened,
        Kind::ReturnRequested,
        Kind::DisputeDecided,
        Kind::ModerationWarning,
        Kind::ModerationMuted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::OrderPaid => "order_paid",
            Kind::OrderPickedUp => "order_picked_up",
            Kind::OrderOutForDelivery => "order_out_for_delivery",
            Kind::OrderDelivered => "order_delivered",
            Kind::ConfirmReminder => "confirm_reminder",
            Kind::ReleaseTomorrow => "release_tomorrow",
            Kind::RefundStarted => "refund_started",
            Kind::FundsReleased => "funds_released",
            Kind::ChatMessage => "chat_message",
            Kind::DisputeOpened => "dispute_opened",
            Kind::ReturnRequested => "return_requested",
            Kind::DisputeDecided => "dispute_decided",
            Kind::ModerationWarning => "moderation_warning",
            Kind::ModerationMuted => "moderation_muted",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::OrderPaid => "Paid \u{2014} please ship",
            Kind::OrderPickedUp => "Order picked up",
            Kind::OrderOutForDelivery => "Out for delivery",
            Kind::OrderDelivered => "Delivered",
            Kind::ConfirmReminder => "Did your order arrive?",
            Kind::ReleaseTomorrow => "Payment releases tomorrow",
            Kind::RefundStarted => "Refund started",
            Kind::FundsReleased => "Payment released",
            Kind::ChatMessage => "New message",
            Kind::DisputeOpened => "A buyer reported a problem",
            Kind::ReturnRequested => "Return requested",
            Kind::DisputeDecided => "Report decided",
            Kind::ModerationWarning => "Message not sent",
            Kind::ModerationMuted => "Posting paused",
        }
    }

    pub fn body(self) -> &'static str {
        match self {
            Kind::OrderPaid => "A buyer has paid. Prepare the order for delivery within 48 hours.",
            Kind::OrderPickedUp => "Your order is on its way.",
            Kind::OrderOutForDelivery => {
                "Your order is out for delivery. Open the app to track it."
            }
            Kind::OrderDelivered => {
                "Your order was marked delivered. Confirm in the app once you hold it, or report a problem."
            }
            Kind::ConfirmReminder => "Confirm in the app once you hold it, or report a problem.",
            Kind::ReleaseTomorrow => {
                "The seller will be paid tomorrow. If something is wrong with your order, report it before then."
            }
            Kind::RefundStarted => "Your refund has started. Banks can take up to 10 working days.",
            Kind::FundsReleased => "An order's payment was released. Your payout is on its way.",
            Kind::ChatMessage => "You have a new message. Open the app to read it.",
            Kind::DisputeOpened => {
                "Respond within 48 hours. The order's payment is on hold until it is settled."
            }
            Kind::ReturnRequested => {
                "The market asked for the item to be returned before deciding. Open the app for details."
            }
            Kind::DisputeDecided => {
                "The market has decided on the reported order. Open the app for details."
            }
            Kind::ModerationWarning => {
                "A recent message broke the market's rules. Repeated breaks lead to a mute."
            }
            Kind::ModerationMuted => "You can't post for a while because of repeated rule breaks.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_have_the_exact_copy() {
        let table: [(&str, &str, &str); 14] = [
            (
                "order_paid",
                "Paid \u{2014} please ship",
                "A buyer has paid. Prepare the order for delivery within 48 hours.",
            ),
            (
                "order_picked_up",
                "Order picked up",
                "Your order is on its way.",
            ),
            (
                "order_out_for_delivery",
                "Out for delivery",
                "Your order is out for delivery. Open the app to track it.",
            ),
            (
                "order_delivered",
                "Delivered",
                "Your order was marked delivered. Confirm in the app once you hold it, or report a problem.",
            ),
            (
                "confirm_reminder",
                "Did your order arrive?",
                "Confirm in the app once you hold it, or report a problem.",
            ),
            (
                "release_tomorrow",
                "Payment releases tomorrow",
                "The seller will be paid tomorrow. If something is wrong with your order, report it before then.",
            ),
            (
                "refund_started",
                "Refund started",
                "Your refund has started. Banks can take up to 10 working days.",
            ),
            (
                "funds_released",
                "Payment released",
                "An order's payment was released. Your payout is on its way.",
            ),
            (
                "chat_message",
                "New message",
                "You have a new message. Open the app to read it.",
            ),
            (
                "dispute_opened",
                "A buyer reported a problem",
                "Respond within 48 hours. The order's payment is on hold until it is settled.",
            ),
            (
                "return_requested",
                "Return requested",
                "The market asked for the item to be returned before deciding. Open the app for details.",
            ),
            (
                "dispute_decided",
                "Report decided",
                "The market has decided on the reported order. Open the app for details.",
            ),
            (
                "moderation_warning",
                "Message not sent",
                "A recent message broke the market's rules. Repeated breaks lead to a mute.",
            ),
            (
                "moderation_muted",
                "Posting paused",
                "You can't post for a while because of repeated rule breaks.",
            ),
        ];
        for (kind, title, body) in table {
            let k = Kind::parse(kind).unwrap_or_else(|| panic!("unknown kind {kind}"));
            assert_eq!(k.as_str(), kind);
            assert_eq!(k.title(), title);
            assert_eq!(k.body(), body);
        }
        assert_eq!(Kind::ALL.len(), 14);
    }
}
