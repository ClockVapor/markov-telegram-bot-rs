use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use substring::Substring;

use ReadError::{IoError, SerdeError};

#[derive(Serialize, Deserialize, Debug)]
pub struct ChatExport {
    // TODO: Each import should have a tracked checksum, so we can avoid importing the same file more than once.
    pub id: i64,
    #[serde(rename = "type")]
    pub type_field: ChatType,
    pub messages: Vec<Message>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ChatType {
    PrivateGroup,
    PrivateSupergroup,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Message {
    pub from_id: Option<String>,
    #[serde(rename = "text")]
    pub contents: MessageContents,
}

impl Display for Message {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.contents.to_string())
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum MessageContents {
    PlainText(String),
    Pieces(Vec<TextPiece>),
}

impl Display for MessageContents {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageContents::PlainText(text) => f.write_str(text),
            MessageContents::Pieces(pieces) => f.write_str(
                &pieces
                    .iter()
                    .map(|piece| piece.to_string())
                    .collect::<Vec<String>>()
                    .join(" "),
            ),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum TextPiece {
    PlainText(String),
    Entity(Entity),
}

impl Display for TextPiece {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TextPiece::PlainText(text) => f.write_str(text),
            TextPiece::Entity(entity) => f.write_str(&entity.text),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Entity {
    #[serde(rename = "type")]
    pub type_field: String,
    pub text: String,
}

#[derive(Debug)]
pub enum ReadError {
    IoError(std::io::Error),
    SerdeError(serde_json::Error),
}

pub fn read_chat_export(file_path: &str) -> Result<ChatExport, ReadError> {
    let contents = match std::fs::read_to_string(file_path) {
        Ok(v) => v,
        Err(e) => return Err(IoError(e)),
    };
    let mut export: ChatExport = match serde_json::from_str(contents.as_str()) {
        Ok(v) => v,
        Err(e) => return Err(SerdeError(e)),
    };

    // Fix the chat ID field to match what the API gives (no idea why this needs to be done)
    match export.type_field {
        ChatType::PrivateGroup => {
            export.id = -export.id;
        }
        ChatType::PrivateSupergroup => {
            let mut new_id_str = "-100".to_string();
            new_id_str.push_str(export.id.to_string().as_str());
            export.id = new_id_str.parse().unwrap();
        }
    }

    // Only keep messages whose "from_id" starts with "user" or "channel"
    export.messages.retain(|message| match &message.from_id {
        None => false,
        Some(id) => id.starts_with("user") || id.starts_with("channel"),
    });
    for message in &mut export.messages {
        match &message.from_id {
            None => {}
            Some(id) => {
                // Remove the prefix from the user ID
                if id.starts_with("user") {
                    message.from_id = Some(id.substring(4, id.len()).to_string());
                } else if id.starts_with("channel") {
                    message.from_id = Some(id.substring(7, id.len()).to_string());
                } else {
                    panic!(
                        "Expected prefix \"user\" or \"channel\" on \"from_id\" value: {}",
                        id
                    );
                }
            }
        }
    }

    Ok(export)
}
