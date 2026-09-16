//! Agent-to-agent messages: the small coordination talk ("take task X",
//! "I released server.rs") that used to go through whatever chat the
//! agents' clients happened to share. Tirith carries it so any MCP client
//! can take part. A message is delivered by piggybacking on the
//! recipient's next tool call, whatever it is; nothing polls. See
//! ADR-0020.
//!
//! Messages are runtime state: they live in `.tirith/runtime/messages.jsonl`,
//! are never committed, and are dropped after [`RETENTION`]. Delivery
//! marks live only in memory for the daemon's lifetime, like a brief's.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{AgentId, MessageId, Page, PrefixError, RepoPath, resolve_prefix};

/// The most characters a message may carry.
pub const MAX_TEXT: usize = 1000;
/// How many undelivered messages one result carries.
pub const INBOX_LIMIT: usize = 5;
/// Messages older than this are dropped when the daemon loads them.
pub const RETENTION: Duration = Duration::hours(24);
/// A broadcast reaches the agents seen within this window before it.
pub const BROADCAST_WINDOW: Duration = Duration::hours(1);
/// The recipient name that means every agent seen recently.
pub const EVERYONE: &str = "*";

/// One message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Message {
    /// Unique identifier.
    pub id: MessageId,
    /// Who sent it.
    pub from: AgentId,
    /// The agent name it was sent to, or [`EVERYONE`].
    pub to: String,
    /// The text, at most [`MAX_TEXT`] characters.
    pub text: String,
    /// The message this answers, if any.
    pub reply_to: Option<MessageId>,
    /// Paths the message is about, if any.
    pub paths: Vec<RepoPath>,
    /// When it was sent.
    pub at: DateTime<Utc>,
    /// For a broadcast, the agents it was addressed to: everyone seen in
    /// the hour before it was sent, minus the sender. Empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audience: Vec<AgentId>,
}

impl Message {
    /// Whether this went to everyone.
    pub fn is_broadcast(&self) -> bool {
        self.to == EVERYONE
    }

    /// Whether `agent` is a recipient.
    pub fn addressed_to(&self, agent: &AgentId) -> bool {
        if self.is_broadcast() {
            self.audience.contains(agent)
        } else {
            self.to == agent.as_str()
        }
    }

    /// Whether `agent` sent or received it.
    pub fn involves(&self, agent: &AgentId) -> bool {
        &self.from == agent || self.addressed_to(agent)
    }
}

/// What to send, for [`MessageBoard::send`]. Build it with
/// [`NewMessage::new`] and the `with_*` setters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewMessage {
    /// See [`Message::to`].
    pub to: String,
    /// See [`Message::text`].
    pub text: String,
    /// See [`Message::reply_to`].
    pub reply_to: Option<MessageId>,
    /// See [`Message::paths`].
    pub paths: Vec<RepoPath>,
}

impl NewMessage {
    /// A message to `to` (an agent name or [`EVERYONE`]) saying `text`.
    ///
    /// ```
    /// use tirith::messages::NewMessage;
    ///
    /// let message = NewMessage::new("bob", "I released src/server.rs");
    /// assert_eq!(message.to, "bob");
    /// ```
    pub fn new(to: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            to: to.into(),
            text: text.into(),
            ..Self::default()
        }
    }

    /// Marks this as an answer to `id`.
    #[must_use]
    pub fn with_reply_to(mut self, id: MessageId) -> Self {
        self.reply_to = Some(id);
        self
    }

    /// Sets the paths the message is about.
    #[must_use]
    pub fn with_paths(mut self, paths: Vec<RepoPath>) -> Self {
        self.paths = paths;
        self
    }
}

/// Filters for [`MessageBoard::list`]. All optional, combined with AND;
/// the listing is always limited to messages the caller sent or received.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageFilter {
    /// Only the conversation with this agent.
    pub with: Option<AgentId>,
    /// Only messages sent at or after this instant.
    pub since: Option<DateTime<Utc>>,
    /// Only messages to the caller that it has not yet received.
    pub unread: bool,
}

/// Why a message was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MessageError {
    /// The text was empty.
    #[error("message text must not be empty")]
    EmptyText,
    /// The text was too long.
    #[error("message text is {len} characters; the maximum is {max}")]
    TooLong {
        /// The offending length.
        len: usize,
        /// The maximum.
        max: usize,
    },
    /// The recipient was not an agent name or `*`.
    #[error("recipient must be an agent name or `*`: {0}")]
    BadRecipient(String),
    /// `reply_to` names no message.
    #[error("no message with id {0}")]
    NotFound(MessageId),
}

/// What one result delivers: the newest undelivered messages and how
/// many more are waiting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inbox {
    /// Newest first, at most [`INBOX_LIMIT`].
    pub messages: Vec<Message>,
    /// Undelivered messages beyond these.
    pub more: usize,
}

/// All messages the daemon holds, plus who has received what.
#[derive(Debug, Default)]
pub struct MessageBoard {
    messages: Vec<Message>,
    delivered: BTreeMap<AgentId, BTreeSet<MessageId>>,
}

impl MessageBoard {
    /// Rebuilds a board from persisted messages, dropping those older
    /// than [`RETENTION`] at `now`. Nothing counts as delivered yet.
    pub fn from_messages(messages: Vec<Message>, now: DateTime<Utc>) -> Self {
        let keep_after = now - RETENTION;
        Self {
            messages: messages
                .into_iter()
                .filter(|m| m.at >= keep_after)
                .collect(),
            delivered: BTreeMap::new(),
        }
    }

    /// All messages in send order.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Sends a message. `recent` is every agent seen within
    /// [`BROADCAST_WINDOW`], the audience of a broadcast; it is ignored for
    /// a direct message.
    pub fn send(
        &mut self,
        from: AgentId,
        new: NewMessage,
        recent: impl IntoIterator<Item = AgentId>,
        now: DateTime<Utc>,
    ) -> Result<&Message, MessageError> {
        let text = new.text.trim().to_owned();
        if text.is_empty() {
            return Err(MessageError::EmptyText);
        }
        let len = text.chars().count();
        if len > MAX_TEXT {
            return Err(MessageError::TooLong { len, max: MAX_TEXT });
        }
        let to = new.to.trim();
        if to.is_empty()
            || to.contains(['\n', '\r'])
            || (to != EVERYONE && AgentId::new(to).is_err())
        {
            return Err(MessageError::BadRecipient(to.to_owned()));
        }
        if let Some(reply_to) = new.reply_to
            && !self.messages.iter().any(|m| m.id == reply_to)
        {
            return Err(MessageError::NotFound(reply_to));
        }
        let audience = if to == EVERYONE {
            recent.into_iter().filter(|a| *a != from).collect()
        } else {
            Vec::new()
        };
        self.messages.push(Message {
            id: MessageId::new(),
            from,
            to: to.to_owned(),
            text,
            reply_to: new.reply_to,
            paths: new.paths,
            at: now,
            audience,
        });
        Ok(self
            .messages
            .last()
            .unwrap_or_else(|| unreachable!("just pushed")))
    }

    /// Whether `agent` still has to receive `message`.
    fn undelivered(&self, agent: &AgentId, message: &Message) -> bool {
        message.addressed_to(agent)
            && self
                .delivered
                .get(agent)
                .is_none_or(|seen| !seen.contains(&message.id))
    }

    /// Messages `agent` sent or received, matching `filter`, in send order.
    pub fn list(&self, agent: &AgentId, filter: &MessageFilter) -> Vec<&Message> {
        self.messages
            .iter()
            .filter(|m| m.involves(agent))
            .filter(|m| {
                filter
                    .with
                    .as_ref()
                    .is_none_or(|other| &m.from == other || m.addressed_to(other))
            })
            .filter(|m| filter.since.is_none_or(|s| m.at >= s))
            .filter(|m| !filter.unread || self.undelivered(agent, m))
            .collect()
    }

    /// Newest `limit` messages for `agent` matching `filter` and older
    /// than the `(at, id)` cursor `before`. See [`Page`].
    pub fn list_page(
        &self,
        agent: &AgentId,
        filter: &MessageFilter,
        before: Option<&(DateTime<Utc>, MessageId)>,
        limit: usize,
    ) -> Page<&Message> {
        Page::newest_first(self.list(agent, filter), |m| (m.at, m.id), before, limit)
    }

    /// The newest [`INBOX_LIMIT`] messages `agent` has not received yet,
    /// marked delivered as they leave, plus how many more are waiting.
    /// Empty when nothing is waiting, which is the common case and costs
    /// nothing on the wire.
    pub fn take_inbox(&mut self, agent: &AgentId) -> Inbox {
        let mut waiting: Vec<&Message> = self
            .messages
            .iter()
            .filter(|m| self.undelivered(agent, m))
            .collect();
        waiting.sort_by(|a, b| (b.at, b.id).cmp(&(a.at, a.id)));
        let more = waiting.len().saturating_sub(INBOX_LIMIT);
        let messages: Vec<Message> = waiting.into_iter().take(INBOX_LIMIT).cloned().collect();
        if !messages.is_empty() {
            let seen = self.delivered.entry(agent.clone()).or_default();
            seen.extend(messages.iter().map(|m| m.id));
        }
        Inbox { messages, more }
    }

    /// Resolves a full id or a unique prefix to a message id.
    pub fn resolve_id(&self, raw: &str) -> Result<MessageId, PrefixError> {
        resolve_prefix("message", self.messages.iter().map(|m| m.id), raw)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).unwrap()
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 16, 3, 0, 0).unwrap()
    }

    #[test]
    fn a_direct_message_is_delivered_once_to_its_recipient_only() {
        let mut board = MessageBoard::default();
        board
            .send(
                agent("alice"),
                NewMessage::new("bob", "take task X"),
                [],
                t0(),
            )
            .unwrap();
        assert!(board.take_inbox(&agent("carol")).messages.is_empty());
        assert!(board.take_inbox(&agent("alice")).messages.is_empty());
        let inbox = board.take_inbox(&agent("bob"));
        assert_eq!(inbox.messages.len(), 1);
        assert_eq!(inbox.messages[0].text, "take task X");
        assert_eq!(inbox.more, 0);
        assert!(board.take_inbox(&agent("bob")).messages.is_empty(), "once");
        // History still shows it for both parties, and unread is now false.
        assert_eq!(
            board.list(&agent("bob"), &MessageFilter::default()).len(),
            1
        );
        assert_eq!(
            board.list(&agent("alice"), &MessageFilter::default()).len(),
            1
        );
        assert!(
            board
                .list(
                    &agent("bob"),
                    &MessageFilter {
                        unread: true,
                        ..Default::default()
                    }
                )
                .is_empty()
        );
    }

    #[test]
    fn a_broadcast_reaches_the_agents_seen_recently_but_not_the_sender() {
        let mut board = MessageBoard::default();
        board
            .send(
                agent("head"),
                NewMessage::new(EVERYONE, "server.rs is free"),
                [agent("head"), agent("alice"), agent("bob")],
                t0(),
            )
            .unwrap();
        assert_eq!(board.take_inbox(&agent("alice")).messages.len(), 1);
        assert_eq!(board.take_inbox(&agent("bob")).messages.len(), 1);
        assert!(board.take_inbox(&agent("head")).messages.is_empty());
        assert!(
            board.take_inbox(&agent("late")).messages.is_empty(),
            "not seen before the send"
        );
    }

    #[test]
    fn the_inbox_carries_the_newest_five_and_counts_the_rest() {
        let mut board = MessageBoard::default();
        for i in 0..7 {
            board
                .send(
                    agent("alice"),
                    NewMessage::new("bob", format!("m{i}")),
                    [],
                    t0() + Duration::seconds(i),
                )
                .unwrap();
        }
        let first = board.take_inbox(&agent("bob"));
        assert_eq!(first.messages.len(), INBOX_LIMIT);
        assert_eq!(first.messages[0].text, "m6", "newest first");
        assert_eq!(first.more, 2);
        let second = board.take_inbox(&agent("bob"));
        assert_eq!(second.messages.len(), 2);
        assert_eq!(second.more, 0);
    }

    #[test]
    fn listing_pages_and_filters_by_conversation() {
        let mut board = MessageBoard::default();
        for i in 0..3 {
            board
                .send(
                    agent("alice"),
                    NewMessage::new("bob", format!("a{i}")),
                    [],
                    t0() + Duration::seconds(i),
                )
                .unwrap();
        }
        board
            .send(
                agent("carol"),
                NewMessage::new("bob", "c"),
                [],
                t0() + Duration::seconds(10),
            )
            .unwrap();
        let with_alice = MessageFilter {
            with: Some(agent("alice")),
            ..Default::default()
        };
        let page = board.list_page(&agent("bob"), &with_alice, None, 2);
        assert_eq!(page.total, 3);
        assert_eq!(page.items[0].text, "a2");
        let cursor = page.next_before(|m| (m.at, m.id)).unwrap();
        let rest = board.list_page(&agent("bob"), &with_alice, Some(&cursor), 2);
        assert_eq!(rest.items.len(), 1);
        assert_eq!(rest.items[0].text, "a0");
        assert!(board.list(&agent("carol"), &with_alice).is_empty());
    }

    #[test]
    fn empty_too_long_and_bad_recipients_are_refused_and_old_messages_expire() {
        let mut board = MessageBoard::default();
        assert_eq!(
            board
                .send(agent("a"), NewMessage::new("b", "   "), [], t0())
                .unwrap_err(),
            MessageError::EmptyText
        );
        let long = "x".repeat(MAX_TEXT + 1);
        assert!(matches!(
            board
                .send(agent("a"), NewMessage::new("b", long), [], t0())
                .unwrap_err(),
            MessageError::TooLong { len: 1001, .. }
        ));
        assert!(matches!(
            board
                .send(agent("a"), NewMessage::new("  ", "hi"), [], t0())
                .unwrap_err(),
            MessageError::BadRecipient(_)
        ));
        let sent = board
            .send(agent("a"), NewMessage::new("b", "hi"), [], t0())
            .unwrap()
            .clone();
        assert!(matches!(
            board
                .send(
                    agent("b"),
                    NewMessage::new("a", "?").with_reply_to(MessageId::new()),
                    [],
                    t0()
                )
                .unwrap_err(),
            MessageError::NotFound(_)
        ));
        board
            .send(
                agent("b"),
                NewMessage::new("a", "ok").with_reply_to(sent.id),
                [],
                t0(),
            )
            .unwrap();
        let later = t0() + RETENTION + Duration::seconds(1);
        let reloaded = MessageBoard::from_messages(board.messages().to_vec(), later);
        assert!(reloaded.messages().is_empty(), "24 h retention");
        let fresh =
            MessageBoard::from_messages(board.messages().to_vec(), t0() + Duration::hours(1));
        assert_eq!(fresh.messages().len(), 2);
    }
}
