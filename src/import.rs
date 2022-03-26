use serde::{Deserialize, Serialize};
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

impl Message {
    pub fn to_string(&self) -> String {
        self.contents.to_string()
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum MessageContents {
    PlainText(String),
    Pieces(Vec<TextPiece>),
}

impl MessageContents {
    pub fn to_string(&self) -> String {
        match self {
            MessageContents::PlainText(text) => text.clone(),
            MessageContents::Pieces(pieces) => pieces
                .iter()
                .map(|piece| piece.to_string())
                .collect::<Vec<String>>()
                .join(" "),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum TextPiece {
    PlainText(String),
    Entity(Entity),
}

impl TextPiece {
    pub fn to_string(&self) -> String {
        match self {
            TextPiece::PlainText(text) => text.clone(),
            TextPiece::Entity(entity) => entity.text.clone(),
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

    for message in &mut export.messages {
        match &message.from_id {
            None => {}
            Some(id) => {
                // Remove the "user" prefix from the user ID
                message.from_id = Some(id.substring(4, id.len()).to_string());
            }
        }
    }

    Ok(export)
}
