use super::{schema::*, DbConnection};
use crate::{
    account::Account,
    contact::{Contact, ContactError},
    storage::StorageError,
    ContentCodec, Save, TextCodec,
};
use diesel::prelude::*;
use prost::{DecodeError, Message};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use xmtp_cryptography::hash::sha256_bytes;
use xmtp_proto::xmtp::{message_api::v1::Envelope, message_contents::EncodedContent};

#[derive(Insertable, Selectable, Identifiable, Queryable, PartialEq, Debug, Clone)]
#[diesel(table_name = users)]
#[diesel(belongs_to(StoredConversation))]
#[diesel(primary_key(user_address))]
pub struct StoredUser {
    pub user_address: String,
    pub created_at: i64,
    pub last_refreshed: i64,
}

pub enum ConversationState {
    Uninitialized = 0,
    Invited = 10,
    InviteReceived = 20,
}

#[derive(Insertable, Identifiable, Selectable, Queryable, PartialEq, Debug, Clone)]
#[diesel(table_name = conversations)]
#[diesel(primary_key(convo_id))]
pub struct StoredConversation {
    pub convo_id: String,
    pub peer_address: String, // links to users table
    pub created_at: i64,
    pub convo_state: i32, // ConversationState
}

pub enum MessageState {
    Unprocessed = 0,
    LocallyCommitted = 10,
    Received = 20,
}

/// Placeholder type for messages returned from the Store.
#[derive(Queryable, Debug)]
pub struct StoredMessage {
    pub id: i32,
    pub created_at: i64,
    pub sent_at_ns: i64,
    pub convo_id: String,
    pub addr_from: String,
    pub content: Vec<u8>,
    pub state: i32,
}

impl StoredMessage {
    pub fn get_text(&self) -> Result<String, DecodeError> {
        let content = EncodedContent::decode(self.content.as_slice())?;
        let fallback = String::from(content.fallback());
        match TextCodec::decode(content) {
            Ok(t) => Ok(t),
            Err(_) => Ok(fallback),
        }
    }
}

/// Placeholder type for messages being inserted into the store. This type is the same as
/// DecryptedMessage expect it does not have an `id` feild. The field is generated by the
/// store when it is inserted.
#[derive(Insertable, Clone, PartialEq, Debug)]
#[diesel(table_name = messages)]
pub struct NewStoredMessage {
    pub created_at: i64,
    pub sent_at_ns: i64,
    pub convo_id: String,
    pub addr_from: String,
    pub content: Vec<u8>,
    pub state: i32,
}

impl NewStoredMessage {
    pub fn new(
        convo_id: String,
        addr_from: String,
        content: Vec<u8>,
        state: i32,
        sent_at_ns: i64,
    ) -> Self {
        Self {
            created_at: now(),
            convo_id,
            sent_at_ns,
            addr_from,
            content,
            state,
        }
    }
}

impl PartialEq<StoredMessage> for NewStoredMessage {
    fn eq(&self, other: &StoredMessage) -> bool {
        self.created_at == other.created_at
            && self.sent_at_ns == other.sent_at_ns
            && self.convo_id == other.convo_id
            && self.addr_from == other.addr_from
            && self.content == other.content
    }
}

pub enum OutboundPayloadState {
    Pending = 0,
    ServerAcknowledged = 10,
}

#[derive(Insertable, Identifiable, Queryable, PartialEq, Debug)]
#[diesel(table_name = outbound_payloads)]
#[diesel(primary_key(created_at_ns))]
pub struct StoredOutboundPayload {
    pub payload_id: String,
    pub created_at_ns: i64,
    pub content_topic: String,
    pub payload: Vec<u8>,
    pub outbound_payload_state: i32,
    pub locked_until_ns: i64,
}

impl StoredOutboundPayload {
    pub fn new(
        created_at_ns: i64,
        content_topic: String,
        payload: Vec<u8>,
        outbound_payload_state: i32,
        locked_until_ns: i64,
    ) -> Self {
        let payload_id = hex::encode(sha256_bytes(
            &(format!("{created_at_ns}:{content_topic}").encode_to_vec()),
        ));
        Self {
            payload_id,
            created_at_ns,
            content_topic,
            payload,
            outbound_payload_state,
            locked_until_ns,
        }
    }
}

pub fn now() -> i64 {
    let start = SystemTime::now();
    start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos() as i64
}

#[derive(Insertable, Identifiable, Queryable, Clone, PartialEq, Debug, QueryableByName)]
#[diesel(table_name = sessions)]
#[diesel(primary_key(session_id))]
pub struct StoredSession {
    pub session_id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub peer_installation_id: String,
    pub vmac_session_data: Vec<u8>,
    pub user_address: String,
}

impl StoredSession {
    pub fn new(
        session_id: String,
        peer_installation_id: String,
        vmac_session_data: Vec<u8>,
        user_address: String,
    ) -> Self {
        let now = now();
        Self {
            session_id,
            peer_installation_id,
            created_at: now,
            updated_at: now,
            vmac_session_data,
            user_address,
        }
    }
}

impl Save<DbConnection> for StoredSession {
    fn save(&self, into: &mut DbConnection) -> Result<(), StorageError> {
        diesel::update(sessions::table)
            .filter(sessions::session_id.eq(self.session_id.clone()))
            .set((
                sessions::vmac_session_data.eq(&self.vmac_session_data),
                sessions::peer_installation_id.eq(&self.peer_installation_id),
                sessions::updated_at.eq(now()),
            ))
            .execute(into)?;

        Ok(())
    }
}

#[derive(Queryable, Debug)]
pub struct StoredAccount {
    pub id: i32,
    pub created_at: i64,
    pub serialized_key: Vec<u8>,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = accounts)]
pub struct NewStoredAccount {
    pub created_at: i64,
    pub serialized_key: Vec<u8>,
}
impl TryFrom<&Account> for NewStoredAccount {
    type Error = StorageError;
    fn try_from(account: &Account) -> Result<Self, StorageError> {
        Ok(Self {
            created_at: now(),
            serialized_key: serde_json::to_vec(account).map_err(|e| {
                StorageError::Store(format!(
                    "could not initialize model:NewStoredAccount -- {}",
                    e
                ))
            })?,
        })
    }
}

#[derive(
    Queryable, Selectable, Associations, Insertable, Debug, PartialEq, Identifiable, Clone,
)]
#[diesel(belongs_to(StoredUser, foreign_key = user_address))]
#[diesel(primary_key(installation_id))]
#[diesel(table_name = installations)]
pub struct StoredInstallation {
    pub installation_id: String,
    pub user_address: String,
    pub first_seen_ns: i64,
    pub contact: Vec<u8>,
    pub expires_at_ns: Option<i64>,
}

impl StoredInstallation {
    pub fn new(contact: &Contact) -> Result<Self, ContactError> {
        let contact_bytes: Vec<u8> = contact.try_into()?;

        Ok(Self {
            installation_id: contact.installation_id(),
            user_address: contact.wallet_address.clone(),
            first_seen_ns: now(),
            contact: contact_bytes,
            expires_at_ns: None,
        })
    }

    pub fn get_contact(&self) -> Result<Contact, ContactError> {
        Contact::from_bytes(self.contact.clone(), self.user_address.clone())
    }
}

pub enum RefreshJobKind {
    Invite,
    Message,
}

impl fmt::Display for RefreshJobKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RefreshJobKind::Invite => write!(f, "invite"),
            RefreshJobKind::Message => write!(f, "message"),
        }
    }
}

#[derive(Insertable, Identifiable, Queryable, Clone, PartialEq, Debug)]
#[diesel(table_name = refresh_jobs)]
pub struct RefreshJob {
    pub id: String,
    pub last_run: i64,
}

impl Save<DbConnection> for RefreshJob {
    fn save(&self, into: &mut DbConnection) -> Result<(), StorageError> {
        diesel::update(refresh_jobs::table)
            .set(refresh_jobs::last_run.eq(&self.last_run))
            .execute(into)?;

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum InboundInviteStatus {
    Pending = 0,
    Processed = 1,
    DecryptionFailure = 2,
    Invalid = 3,
}

#[derive(Clone, Debug)]
pub enum InboundMessageStatus {
    Pending = 0,
    Processed = 1,
    DecryptionFailure = 2,
    Invalid = 3,
}

#[derive(Insertable, Identifiable, Queryable, Clone, PartialEq, Debug)]
#[diesel(table_name = inbound_invites)]
pub struct InboundInvite {
    pub id: String,
    pub sent_at_ns: i64,
    pub payload: Vec<u8>,
    pub topic: String,
    pub status: i16,
}

impl From<Envelope> for InboundInvite {
    fn from(envelope: Envelope) -> Self {
        let payload = envelope.message;
        let topic = envelope.content_topic;
        let sent_at_ns: i64 = envelope.timestamp_ns.try_into().unwrap();
        let id =
            hex::encode(sha256_bytes(&[payload.as_slice(), topic.as_bytes()].concat()).as_slice());

        Self {
            id,
            sent_at_ns,
            payload,
            topic,
            status: InboundInviteStatus::Pending as i16,
        }
    }
}

#[derive(Insertable, Identifiable, Queryable, Clone, PartialEq, Debug)]
#[diesel(table_name = inbound_messages)]
pub struct InboundMessage {
    pub id: String,
    pub sent_at_ns: i64,
    pub payload: Vec<u8>,
    pub topic: String,
    pub status: i16,
}

impl From<Envelope> for InboundMessage {
    fn from(envelope: Envelope) -> Self {
        let payload = envelope.message;
        let topic = envelope.content_topic;
        let sent_at_ns: i64 = envelope.timestamp_ns.try_into().unwrap();
        let id =
            hex::encode(sha256_bytes(&[payload.as_slice(), topic.as_bytes()].concat()).as_slice());

        Self {
            id,
            sent_at_ns,
            payload,
            topic,
            status: InboundMessageStatus::Pending as i16,
        }
    }
}
