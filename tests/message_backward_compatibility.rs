use serde_cbor::de::from_slice;
use serde_cbor::ser::to_vec;
use serde_derive::{Deserialize, Serialize};

use pueue_lib::network::message::{
    GroupMessage as OriginalGroupMessage, Message as OriginalMessage,
};

/// This is the main message enum. \
/// Everything that's communicated in Pueue can be serialized as this enum.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(from="OriginalMessage")]
pub enum Message {
    Switch(SwitchMessage),
    Clean(CleanMessage),
    Group(GroupMessage),
}

impl From<OriginalMessage> for Message {
    fn from(o_msg: OriginalMessage) -> Self {
        match o_msg {
            OriginalMessage::Switch(m) => Message::Switch(m.into()),
            OriginalMessage::Clean(m) => Message::Clean(m.into()),
            OriginalMessage::Group(m) => Message::Group(m.into()),
            _ => todo!()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SwitchMessage {
    pub task_id_1: usize,
    pub task_id_2: usize,
    pub some_new_field: usize,
}

impl From<pueue_lib::network::message::SwitchMessage> for SwitchMessage {
    fn from(sw: pueue_lib::network::message::SwitchMessage) -> Self {
        SwitchMessage { task_id_1: sw.task_id_1,
                        task_id_2: sw.task_id_2,
                        some_new_field: 0
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CleanMessage {}
impl From<pueue_lib::network::message::CleanMessage> for CleanMessage {
    fn from(_o_msg: pueue_lib::network::message::CleanMessage) -> Self {
        // Do some conversion here
        CleanMessage {}
    }
}

impl From<OriginalGroupMessage> for GroupMessage {
    fn from(o_msg: OriginalGroupMessage) -> Self {
        match o_msg {
            OriginalGroupMessage::Add(s) => GroupMessage::Add(s),
            OriginalGroupMessage::Remove(s) => GroupMessage::Remove(s),
            OriginalGroupMessage::List => GroupMessage::List{something: "".to_string()},
        }
    }
}

impl From<GroupMessage> for OriginalGroupMessage {
    fn from(group_msg: GroupMessage) -> Self {
        match group_msg {
            GroupMessage::Add(s) => OriginalGroupMessage::Add(s),
            GroupMessage::Remove(s) => OriginalGroupMessage::Remove(s),
            GroupMessage::List{something: _} => OriginalGroupMessage::List,
        }
    }
}


#[derive(PartialEq, Clone, Debug, Deserialize, Serialize)]
pub enum GroupMessage {
    Add(String),
    Remove(String),
    List { something: String },
}

#[test]
/// Make sure we can deserialize old messages as long as we have default values set.
fn test_deserialize_old_message() {
    let message = Message::Clean(CleanMessage {});
    let payload_bytes = to_vec(&message).unwrap();

    let message: OriginalMessage = from_slice(&payload_bytes).unwrap();
    if let OriginalMessage::Clean(message) = message {
        // The serialized message didn't have the `successful_only` property yet.
        // Instead the default `false` should be used.
        assert!(!message.successful_only);
    } else {
        panic!("It must be a clean message");
    }
}

#[test]
/// Make sure we can deserialize new messages, even if new values exist.
fn test_deserialize_new_message() {
    let message = Message::Switch(SwitchMessage {
        task_id_1: 0,
        task_id_2: 1,
        some_new_field: 2,
    });
    let payload_bytes = to_vec(&message).unwrap();

    let message: OriginalMessage = from_slice(&payload_bytes).unwrap();
    // The serialized message did have an additional field. The deserialization works anyway.
    assert!(matches!(message, OriginalMessage::Switch(_)));
}

#[test]
/// Make sure we can deserialize enums, if they changed from a simple variant to a complex
/// struct-like variant.
fn test_deserialize_changed_enum_variant() {
    let message = OriginalMessage::Group(OriginalGroupMessage::List);
    let payload_bytes = to_vec(&message).unwrap();

    let message : Message = from_slice(&payload_bytes).unwrap();
    // The serialized message did have an additional field. The deserialization works anyway.
    assert!(matches!(message, Message::Group(_)));
}
