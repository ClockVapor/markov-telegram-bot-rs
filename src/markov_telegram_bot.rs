use std::cmp::Ordering;
use std::collections::hash_map::Entry::Vacant;
use std::collections::HashMap;

use futures::StreamExt;
use log::{debug, error};
use mongodb::bson::doc;
use mongodb::options::{ClientOptions, ReplaceOptions};
use mongodb::{Client, Database};
use serde::{Deserialize, Serialize};
use telegram_bot::{
    Api, CanReplySendMessage, ChatId, Integer, Message, MessageEntity, MessageEntityKind,
    MessageId, MessageKind, MessageOrChannelPost, MessageText, ToMessageId, Update, UpdateKind,
    User, UserId,
};
use MessageEntityKind::{BotCommand, Mention, TextMention};

use crate::{MarkovChain, MarkovChainError};

/// Virtual "user ID" for Markov chain of all users in a chat.
const ALL: &str = "all";

const CHATS_COLLECTION_NAME: &str = "chats";
const CHAT_ID_KEY: &str = "chat_id";

const USER_INFOS_COLLECTION_NAME: &str = "user_infos";
const USERNAME_KEY: &str = "username";

static YES_STRINGS: [&str; 5] = ["y", "yes", "ye", "ya", "yeah"];

type DbError = mongodb::error::Error;

#[derive(Default)]
pub struct MarkovTelegramBot {
    prompts: HashMap<ChatId, HashMap<UserId, Prompt>>,
}

impl MarkovTelegramBot {
    pub fn new() -> Self {
        Default::default()
    }

    pub async fn run(&mut self, bot_token: &str) -> Result<(), String> {
        let api = Api::new(bot_token);
        let mut stream = api.stream();
        while let Some(update) = stream.next().await {
            match update {
                Ok(update) => {
                    self.handle_update(&api, &update).await;
                }

                Err(error) => {
                    error!("Failed to fetch update: {:?}", error);
                }
            }
        }
        Ok(())
    }

    async fn handle_update(&mut self, api: &Api, update: &Update) {
        if let UpdateKind::Message(message) = &update.kind {
            self.handle_message(api, message).await;
        }
    }

    async fn handle_message(&mut self, api: &Api, message: &Message) {
        if let Err(e) = remember_message_sender(message).await {
            error!("Failed to remember user {:?}: {:?}", message.from, e);
        }

        if let Some(text) = &message.text() {
            // Check if the message is a reply to a prompt
            if let Some(prompt) = self.original_prompt_for(message) {
                match prompt.handle_response(message).await {
                    Err(e) => {
                        error!("Failed to handle prompt response: {:?}", e);
                    }
                    Ok(reply_text) => {
                        try_reply(api, message, reply_text).await;
                    }
                }
                return;
            }

            // Check for bot commands
            if let MessageKind::Text { ref entities, .. } = message.kind {
                if let Some(
                    command @ MessageEntity {
                        kind: BotCommand, ..
                    },
                ) = entities.get(0)
                {
                    if let Some(command_text) = text.get(
                        (command.offset + 1) as usize..(command.offset + command.length) as usize,
                    ) {
                        if command_text == "msg" || command_text.starts_with("msg@") {
                            handle_msg_command_message(
                                api,
                                message,
                                text,
                                command,
                                entities.as_slice(),
                            )
                            .await;
                        } else if command_text == "deletemydata"
                            || command_text.starts_with("deletemydata@")
                        {
                            self.handle_delete_my_data_command_message(api, message)
                                .await;
                        }
                    }
                    return; // Don't add bot command messages to the Markov chain
                }
            }

            // If message was not handled by some bot command, add it to the sending user's markov chain
            if let Err(e) = add_to_markov_chain(message).await {
                error!("Failed to add message to Markov chain: {:?}", e);
            };
        }
    }

    /// Handles a message with a /deletemydata command.
    async fn handle_delete_my_data_command_message(&mut self, api: &Api, message: &Message) {
        let ask_message_id = match try_reply(
            api,
            message,
            "Are you sure you want to delete your Markov chain data in this group?".to_string(),
        )
        .await
        {
            None => {
                return;
            }
            Some(result) => result.to_message_id(),
        };

        let chat_prompts = if let Vacant(e) = self.prompts.entry(message.chat.id()) {
            e.insert(HashMap::new());
            self.prompts.get_mut(&message.chat.id()).unwrap()
        } else {
            self.prompts.get_mut(&message.chat.id()).unwrap()
        };

        let prompt = Prompt {
            message_id: ask_message_id,
            kind: PromptKind::DeleteMyData,
        };
        chat_prompts.insert(message.from.id, prompt);
    }

    /// Gets the prompt that a message is replying to, if one exists.
    fn original_prompt_for(&self, message: &Message) -> Option<&Prompt> {
        if let Some(reply_to_message) = &message.reply_to_message {
            if let Some(prompts) = self.prompts.get(&message.chat.id()) {
                if let Some(prompt) = prompts.get(&message.from.id) {
                    if prompt.message_id == reply_to_message.to_message_id() {
                        return Some(prompt);
                    }
                }
            }
        }
        None
    }
}

struct Prompt {
    /// Message ID for the message from the bot that initiated this prompt.
    message_id: MessageId,
    kind: PromptKind,
}

impl Prompt {
    async fn handle_response(&self, response: &Message) -> Result<String, DbError> {
        self.kind.handle_response(response).await
    }
}

enum PromptKind {
    DeleteMyData,
}

impl PromptKind {
    async fn handle_response(&self, response: &Message) -> Result<String, DbError> {
        Ok(match self {
            PromptKind::DeleteMyData => {
                if YES_STRINGS
                    .contains(&response.text().unwrap_or_default().to_lowercase().as_str())
                {
                    if let Some(mut chat_data) = read_chat_data(&response.chat.id()).await? {
                        let user_id = response.from.id.to_string();
                        if chat_data.data.contains_key(user_id.as_str()) {
                            chat_data.data.remove(user_id.as_str());
                            write_chat_data(chat_data).await?;
                            Some(
                                "Your Markov chain data in this group has been deleted."
                                    .to_string(),
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                    .unwrap_or_else(|| "No data found.".to_string())
                } else {
                    "Okay, I won't delete your Markov chain data in this group then.".to_string()
                }
            }
        })
    }
}

async fn handle_msg_command_message(
    api: &Api,
    message: &Message,
    text: &str,
    command: &MessageEntity,
    entities: &[MessageEntity],
) {
    let (source, mention_entity) = match entities.get(1) {
        Some(entity) => {
            if let Some(user_mention) = get_user_mention(text, entity) {
                (Source::SingleUser(user_mention), Some(entity))
            } else {
                (Source::AllUsers, None)
            }
        }
        None => (Source::AllUsers, None),
    };
    let reply_text = {
        let seed = match source {
            Source::SingleUser(_) => {
                get_seed(text.get(
                    (mention_entity.unwrap().offset + mention_entity.unwrap().length) as usize..,
                ))
            }
            Source::AllUsers => get_seed(text.get((command.offset + command.length) as usize..)),
        };
        match seed {
            Err(e) => e,
            Ok(seed) => {
                debug!("Got /msg for {:?}", source);
                match do_msg_command(&message.chat.id(), &source, seed).await {
                    Ok(Some(text)) => text,
                    Ok(None) | Err(MsgCommandError::MarkovChainError(MarkovChainError::Empty)) => {
                        "<no data>".to_string()
                    }
                    Err(MsgCommandError::MarkovChainError(MarkovChainError::NoSuchSeed)) => {
                        "<no such seed>".to_string()
                    }
                    Err(e) => {
                        error!("An error occurred executing /msg command: {:?}", e);
                        "<an error occurred>".to_string()
                    }
                }
            }
        }
    };
    try_reply(api, message, reply_text).await;
}

/// Parses up to one seed value from the given optional string. Err is returned if more than one seed value is given.
fn get_seed(text: Option<&str>) -> Result<Option<String>, String> {
    match text {
        Some(text) => {
            let parts: Vec<&str> = text.split_whitespace().collect();
            match parts.len().cmp(&1_usize) {
                Ordering::Equal => Ok(Some(parts.get(0).unwrap().to_string())),
                Ordering::Greater => Err("<up to one seed word can be provided>".to_string()),
                Ordering::Less => Ok(None),
            }
        }
        None => Ok(None),
    }
}

/// Stores the message sender's username and user ID so that their username can be associated with their user ID.
async fn remember_message_sender(message: &Message) -> Result<(), DbError> {
    if let Some(username) = &message.from.username {
        let username = username.to_lowercase();
        let db = connect_to_db().await?;
        let user_infos = db.collection_with_type::<UserInfo>(USER_INFOS_COLLECTION_NAME);
        let replace_options = {
            let mut replace_options = ReplaceOptions::default();
            replace_options.upsert = Some(true); // Insert new document if an existing one isn't found
            replace_options
        };
        user_infos
            .replace_one(
                doc! {USERNAME_KEY: &username},
                UserInfo {
                    username: username.clone(),
                    user_id: message.from.id.to_string(),
                },
                replace_options,
            )
            .await?;
        debug!(
            "Remembered username {} has user_id {}",
            &username,
            message.from.id.to_string()
        );
    }
    Ok(())
}

async fn do_msg_command<'a>(
    chat_id: &ChatId,
    source: &Source<'a>,
    seed: Option<String>,
) -> Result<Option<String>, MsgCommandError> {
    let user_id = match source {
        Source::SingleUser(target_user_mention) => target_user_mention.user_id().await,
        Source::AllUsers => Ok(Some(ALL.to_string())),
    };
    match user_id {
        Err(e) => Err(MsgCommandError::DbError(e)),
        Ok(None) => Ok(None),
        Ok(Some(user_id)) => match read_chat_data(chat_id).await {
            Err(e) => Err(MsgCommandError::DbError(e)),
            Ok(None) => Ok(None),
            Ok(Some(chat_data)) => match chat_data.data.get(&user_id) {
                None => Ok(None),
                Some(markov_chain) => match markov_chain.generate(seed) {
                    Err(e) => Err(MsgCommandError::MarkovChainError(e)),
                    Ok(words) => Ok(Some(words.join(" "))),
                },
            },
        },
    }
}

#[derive(Debug)]
enum MsgCommandError {
    MarkovChainError(MarkovChainError),
    DbError(DbError),
}

async fn add_to_markov_chain(message: &Message) -> Result<(), DbError> {
    let text = match &message.kind {
        MessageKind::Text { data, .. } => Some(data),
        MessageKind::Photo { caption, .. } => caption.as_ref(),
        MessageKind::Video { caption, .. } => caption.as_ref(),
        MessageKind::Document { caption, .. } => caption.as_ref(),
        _ => None,
    };
    match text {
        Some(text) => {
            let chat_id = message.chat.id();
            let chat_id_raw: Integer = chat_id.into();
            let chat_id_str = chat_id_raw.to_string();
            let mut chat_data = read_chat_data(&chat_id).await?.unwrap_or_else(|| ChatData {
                chat_id: chat_id_str,
                data: HashMap::new(),
            });
            let sender_id_raw: Integer = message.from.id.into();
            let sender_id_str = sender_id_raw.to_string();
            chat_data.add_message(&sender_id_str, text); // Add to the specific user's Markov chain
            chat_data.add_message(&ALL.to_string(), text); // Also add to the "all users" Markov chain
            write_chat_data(chat_data).await
        }

        _ => Ok(()),
    }
}

async fn read_chat_data(chat_id: &ChatId) -> Result<Option<ChatData>, DbError> {
    let chat_id_raw: Integer = (*chat_id).into();
    let chat_id_str = chat_id_raw.to_string();
    let db = connect_to_db().await?;
    let collection = db.collection_with_type::<ChatData>(CHATS_COLLECTION_NAME);
    let result = collection
        .find_one(doc! {CHAT_ID_KEY: chat_id_str.clone()}, None)
        .await;
    match result {
        Ok(chat_data) => {
            debug!("Read chat data for chat {}: {:?}", chat_id_str, chat_data);
            Ok(chat_data)
        }
        Err(e) => {
            error!("Failed to read chat data for chat {}: {:?}", chat_id_str, e);
            Err(e)
        }
    }
}

async fn write_chat_data(chat_data: ChatData) -> Result<(), DbError> {
    let db = connect_to_db().await?;
    let collection = db.collection_with_type::<ChatData>(CHATS_COLLECTION_NAME);
    let replace_options = {
        let mut replace_options = ReplaceOptions::default();
        replace_options.upsert = Some(true); // Insert new document if an existing one isn't found
        replace_options
    };
    let chat_id = chat_data.chat_id.clone();
    let result = collection
        .replace_one(
            doc! {CHAT_ID_KEY: chat_data.chat_id.clone()},
            chat_data,
            Some(replace_options),
        )
        .await;
    match result {
        Ok(_) => {
            debug!("{}", format!("Wrote chat data for chat {}", chat_id));
            Ok(())
        }
        Err(e) => {
            error!("Failed to write chat data for chat {}: {:?}", chat_id, e);
            Err(e)
        }
    }
}

async fn connect_to_db() -> Result<Database, DbError> {
    let mut client_options =
        ClientOptions::parse("mongodb://localhost:27017/?connectTimeoutMS=3000").await?;
    client_options.app_name = Some("markov-telegram-bot-rs".to_string());
    let client = Client::with_options(client_options)?;
    Ok(client.database("markov"))
}

/// Given a message's text and a `MessageEntity` within it, returns a `UserMention` if one is present.
fn get_user_mention<'a>(text: &str, entity: &'a MessageEntity) -> Option<UserMention<'a>> {
    match &entity.kind {
        Mention => {
            let username = text
                .get((entity.offset + 1) as usize..(entity.offset + entity.length) as usize)?
                .to_string();
            Some(UserMention::AtMention(username))
        }

        TextMention(user) => Some(UserMention::TextMention(user)),

        _ => None,
    }
}

async fn try_reply(api: &Api, message: &Message, text: String) -> Option<MessageOrChannelPost> {
    match api.send(message.text_reply(text)).await {
        Err(e) => {
            error!("Failed to send reply: {:?}", e);
            None
        }
        Ok(result) => Some(result),
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ChatData {
    /// ID of the Telegram chat that this data belongs to.
    chat_id: String,

    /// HashMap from a Telegram user's ID to their Markov chain.
    data: HashMap<String, MarkovChain>,
}

impl ChatData {
    /// Adds a Telegram message to a user's Markov chain.
    fn add_message(&mut self, user_id: &str, text: &str) {
        if self.data.contains_key(user_id) {
            let markov_chain = self.data.get_mut(user_id).unwrap();
            markov_chain.add_message(text);
        } else {
            let mut markov_chain = MarkovChain {
                user_id: user_id.to_string(),
                data: HashMap::new(),
            };
            markov_chain.add_message(text);
            self.data.insert(user_id.to_string(), markov_chain);
        }
    }
}

/// Data structure used to map a Telegram user's username to their user ID, as the Telegram bot API has no way to
/// fetch this relationship.
#[derive(Serialize, Deserialize, Debug)]
struct UserInfo {
    username: String,
    user_id: String,
}

#[derive(Debug)]
enum Source<'a> {
    SingleUser(UserMention<'a>),
    AllUsers,
}

#[derive(Debug)]
enum UserMention<'a> {
    /// A mention of the form @username. The contained String will not include the leading @.
    AtMention(String),

    /// A text mention that is a link to a user that does not have a username.
    TextMention(&'a User),
}

impl<'a> UserMention<'a> {
    /// If the mention is a TextMention, simply returns the linked user's ID.
    /// If the mention is an AtMention, fetches the user ID that maps to the username from the database.
    async fn user_id(&self) -> Result<Option<String>, DbError> {
        match self {
            UserMention::AtMention(username) => {
                let username = username.to_lowercase();
                let db = connect_to_db().await?;
                let user_infos = db.collection_with_type::<UserInfo>(USER_INFOS_COLLECTION_NAME);
                let user_info = user_infos
                    .find_one(doc! {USERNAME_KEY: &username}, None)
                    .await?;
                debug!("Read user info for username {}: {:?}", &username, user_info);
                Ok(user_info.map(|o| o.user_id))
            }
            UserMention::TextMention(user) => Ok(Some(user.id.to_string())),
        }
    }
}
