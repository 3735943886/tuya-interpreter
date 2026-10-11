//! Host-owned inputs and effects. Serialization preserves the JSON C ABI.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Input {
    Connected,
    Disconnected,
    Message {
        channel: Channel,
        json: Value,
    },
    Command {
        prop: String,
        #[serde(default)]
        value: Value,
    },
    Timer {
        name: String,
    },
    Describe {
        #[serde(default)]
        seed: Vec<Input>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Active,
    Passive,
    State,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Output {
    Descriptor {
        desc: Value,
    },
    Value {
        prop: String,
        value: Value,
    },
    Absent {
        prop: String,
    },
    Event {
        prop: String,
        kind: String,
    },
    SendMessage {
        channel: SendChannel,
        json: Value,
    },
    Reject {
        prop: String,
        code: RejectCode,
        reason: String,
    },
    SetTimer {
        name: String,
        after: f64,
    },
    CancelTimer {
        name: String,
    },
    SettingsChanged {
        block: Value,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendChannel {
    Set,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    UnknownProperty,
    ReadOnly,
    RequiresUnmet,
    InvalidValue,
    OutOfRange,
    BadStep,
    Unsupported,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DpKind {
    Integer,
    Boolean,
    Enum,
    Bitmap,
    Json,
    Raw,
    String,
}

impl DpKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Integer => "Integer",
            Self::Boolean => "Boolean",
            Self::Enum => "Enum",
            Self::Bitmap => "Bitmap",
            Self::Json => "Json",
            Self::Raw => "Raw",
            Self::String => "String",
        }
    }
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "Integer" | "value" => Some(Self::Integer),
            "Boolean" | "bool" => Some(Self::Boolean),
            "Enum" | "enum" => Some(Self::Enum),
            "Bitmap" | "bitmap" => Some(Self::Bitmap),
            "Json" | "json" => Some(Self::Json),
            "Raw" | "raw" => Some(Self::Raw),
            "String" | "string" => Some(Self::String),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateSource {
    Control,
    #[default]
    Inferred,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionState {
    Open,
    Closed,
    Opening,
    Closing,
    Stopped,
}

impl MotionState {
    pub(crate) fn moving(self) -> bool {
        matches!(self, Self::Opening | Self::Closing)
    }
    pub(crate) fn resting(position: f64) -> Self {
        if position <= 0.0 {
            Self::Closed
        } else if position >= 100.0 {
            Self::Open
        } else {
            Self::Stopped
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SlotKind {
    Event,
    Delta,
}

impl SlotKind {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "event" => Some(Self::Event),
            "delta" => Some(Self::Delta),
            _ => None,
        }
    }
}
