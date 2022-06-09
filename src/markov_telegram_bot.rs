use std::cmp::Ordering;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::collections::HashMap;
use std::num::ParseIntError;
use std::sync::Mutex;

use frankenstein::MessageEntityType::TextMention;
use frankenstein::{
    AsyncApi, AsyncTelegramApi, ChatId, ChatMember, GetChatAdministratorsParamsBuilder,
    GetChatMemberParams, GetUpdatesParamsBuilder, Message, MessageEntity, MessageEntityType,
    SendMessageParamsBuilder, User,
};
use lazy_static::lazy_static;
use log::{debug, error, info};
use mongodb::bson::doc;
use mongodb::options::{ClientOptions, ReplaceOptions};
use mongodb::{Client, Collection, Database};
use serde::{Deserialize, Serialize};
use substring::Substring;
use MessageEntityType::Mention;

use crate::import::{MessageContents, TextPiece};
use crate::{import, read_chat_export, MarkovChainError, TripletMarkovChain, LengthRequirement, ComparisonOperator};

/// Virtual "user ID" for Markov chain of all users in a chat.
const ALL: &str = "all";

/// MongoDB collection name for chat data.
const CHATS_COLLECTION_NAME: &str = "chats";
/// Key for chat ID within the "chats" collection.
const CHAT_ID_KEY: &str = "chat_id";

/// MongoDB collection name for user info.
const USER_INFOS_COLLECTION_NAME: &str = "user_infos";
/// Key for username within the "user_infos" collection.
const USERNAME_KEY: &str = "username";

type DbError = mongodb::error::Error;

/// Affirmative responses to questions asked by the bot.
static YES_STRINGS: [&str; 7] = ["y", "yes", "ye", "ya", "yeah", "yea", "yah"];

lazy_static! {
    static ref COMPARISON_OPERATORS: HashMap<&'static str, ComparisonOperator> = HashMap::from([
        (">", ComparisonOperator::GreaterThan),
        (">=", ComparisonOperator::GreaterThanOrEqualTo),
        ("<", ComparisonOperator::LessThan),
        ("<=", ComparisonOperator::LessThanOrEqualTo),
        ("=", ComparisonOperator::EqualTo),
    ]);
}

lazy_static! {
    /// Map of prompts that the bot is asking users. First key is chat ID, second key is user ID within that chat.
    static ref PROMPTS: Mutex<HashMap<i64, HashMap<u64, Prompt>>> = Mutex::new(HashMap::new());
}

pub async fn run(
    bot_token: String,
    db_url: String,
    import_file_path: Option<&str>,
) -> Result<(), String> {
    let api = AsyncApi::new(bot_token.as_str());

    if let Some(file_path) = import_file_path {
        if let Err(e) = import_chat(&api, &db_url, file_path).await {
            error!("Failed to import: {:?}", e);
            return Err("Failed to import".to_string());
        }
        return Ok(());
    }

    let mut update_params_builder = GetUpdatesParamsBuilder::default();
    update_params_builder.allowed_updates(vec!["message".to_string()]);
    let mut update_params = update_params_builder.build().unwrap();

    info!("Bot started");
    loop {
        match api.get_updates(&update_params).await {
            Ok(response) => {
                for update in response.result {
                    if let Some(message) = update.message {
                        let api_clone = api.clone();
                        let db_url_clone = db_url.clone();
                        tokio::spawn(async move {
                            handle_message(api_clone, db_url_clone.as_str(), message).await;
                        });
                        update_params = update_params_builder
                            .offset(update.update_id + 1)
                            .build()
                            .unwrap();
                    }
                }
            }
            Err(e) => {
                error!("Failed to get updates: {:?}", e);
            }
        }
    }
}

async fn handle_message(api: AsyncApi, db_url: &str, message: Message) {
    if message.from.is_none() {
        return;
    }

    // Don't care if this doesn't work here
    match remember_message_sender(db_url, &message).await {
        _ => {}
    };

    if message.text.is_none() {
        return;
    }

    let text = message.text.as_ref().unwrap();
    // Check if the message is a reply to a prompt
    if let Some(prompt) = original_prompt_for(&message) {
        match prompt.handle_response(db_url, &message).await {
            Err(e) => {
                error!("Failed to handle prompt response: {:?}", e);
                try_reply(&api, &message, "<an error occurred>".to_string()).await;
            }
            Ok(reply_text) => {
                remove_prompt(message.chat.id, &message.from.as_ref().unwrap().id);
                try_reply(&api, &message, reply_text).await;
            }
        }
        return;
    }

    // Check for bot commands
    if let Some(entities) = &message.entities {
        if let Some(
            command @ MessageEntity {
                type_field: MessageEntityType::BotCommand,
                ..
            },
        ) = entities.get(0)
        {
            if let Some(command_text) =
                text.get((command.offset + 1) as usize..(command.offset + command.length) as usize)
            {
                if command_text == "msg" || command_text.starts_with("msg@") {
                    handle_msg_command_message(db_url, &api, &message, text, entities.as_slice())
                        .await;
                } else if command_text == "deletemydata"
                    || command_text.starts_with("deletemydata@")
                {
                    handle_delete_my_data_command_message(&api, &message).await;
                } else if command_text == "deleteusermydata"
                    || command_text.starts_with("deleteuserdata@")
                {
                    handle_delete_user_data_command_message(
                        &api,
                        db_url,
                        &message,
                        text,
                        entities.as_slice(),
                    )
                    .await;
                }
            }
            return; // Don't add bot command messages to the Markov chain
        }
    }

    // If message was not handled by some bot command, add it to the sending user's Markov chain
    if let Err(e) = add_to_markov_chain(db_url, &message).await {
        error!("Failed to add message to Markov chain: {:?}", e);
    };
}

/// Handles a message with a /deletemydata command.
async fn handle_delete_my_data_command_message(api: &AsyncApi, message: &Message) {
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
        Some(result) => result.message_id,
    };

    let prompt = Prompt {
        message_id: ask_message_id,
        kind: PromptKind::DeleteMyData,
    };
    add_prompt(message.chat.id, message.from.as_ref().unwrap().id, prompt);
}

/// Handles a message with a /deleteuserdata command.
async fn handle_delete_user_data_command_message(
    api: &AsyncApi,
    db_url: &str,
    message: &Message,
    text: &str,
    entities: &[MessageEntity],
) {
    let params = GetChatAdministratorsParamsBuilder::default()
        .chat_id(ChatId::Integer(message.chat.id))
        .build()
        .unwrap();
    let admins = api.get_chat_administrators(&params).await;
    match admins {
        Err(e) => {
            error!("Failed to fetch chat admins: {:?}", e);
            try_reply(api, message, "<an error occurred>".to_string()).await;
        }
        Ok(admins) => {
            if !admins.result.iter().any(|chat_member| match chat_member {
                ChatMember::Owner(m) => m.user.id == message.from.as_ref().unwrap().id,
                ChatMember::Administrator(m) => m.user.id == message.from.as_ref().unwrap().id,
                _ => false,
            }) {
                try_reply(api, message, "You aren't an admin.".to_string()).await;
                return;
            }
            let user_id = match entities.get(1) {
                Some(entity) => match get_user_mention(text, entity) {
                    Some(user_mention) => user_mention.user_id(db_url).await,
                    None => Ok(None),
                },
                None => Ok(None),
            };
            match user_id {
                Err(_) => {
                    try_reply(api, message, "<an error occurred>".to_string()).await;
                }
                Ok(None) => {
                    try_reply(api, message, "<expected a user mention>".to_string()).await;
                }
                Ok(Some(user_id)) => {
                    let ask_message_id = match try_reply(api, message,
                                                         "Are you sure you want to delete that user's Markov chain data in this group?".to_string()).await {
                        None => {
                            try_reply(api, message, "<an error occurred>".to_string()).await;
                            return;
                        }
                        Some(result) => result.message_id,
                    };
                    let prompt = Prompt {
                        message_id: ask_message_id,
                        kind: PromptKind::DeleteUserData(user_id),
                    };
                    add_prompt(message.chat.id, message.from.as_ref().unwrap().id, prompt);
                }
            }
        }
    }
}

fn add_prompt(chat_id: i64, user_id: u64, prompt: Prompt) {
    let mut prompts = PROMPTS.lock().unwrap();
    if let Vacant(e) = prompts.entry(chat_id) {
        e.insert(HashMap::new());
    }
    prompts.get_mut(&chat_id).unwrap().insert(user_id, prompt);
}

fn remove_prompt(chat_id: i64, user_id: &u64) {
    let mut prompts = PROMPTS.lock().unwrap();
    if let Occupied(mut e) = prompts.entry(chat_id) {
        e.get_mut().remove(user_id);
    }
}

/// Gets the prompt that a message is replying to, if one exists.
fn original_prompt_for(message: &Message) -> Option<Prompt> {
    if let Some(reply_to_message) = &message.reply_to_message {
        if let Some(prompts) = PROMPTS.lock().unwrap().get(&message.chat.id) {
            if let Some(user) = &message.from {
                if let Some(prompt) = prompts.get(&user.id) {
                    if prompt.message_id == reply_to_message.message_id {
                        return Some(prompt.clone());
                    }
                }
            }
        }
    }
    None
}

#[derive(Clone)]
struct Prompt {
    /// Message ID for the message from the bot that initiated this prompt.
    message_id: i32,
    kind: PromptKind,
}

impl Prompt {
    async fn handle_response(&self, db_url: &str, response: &Message) -> Result<String, DbError> {
        self.kind.handle_response(db_url, response).await
    }
}

#[derive(Clone)]
enum PromptKind {
    DeleteMyData,
    DeleteUserData(String),
}

impl PromptKind {
    async fn handle_response(&self, db_url: &str, response: &Message) -> Result<String, DbError> {
        Ok(match self {
            PromptKind::DeleteMyData => {
                if YES_STRINGS.contains(
                    &response
                        .text
                        .as_ref()
                        .unwrap_or(&"".to_string())
                        .to_lowercase()
                        .as_str(),
                ) {
                    if let Some(mut chat_data) = read_chat_data(db_url, &response.chat.id).await? {
                        let user_id = response.from.as_ref().unwrap().id.to_string();

                        // Remove the user's Markov chain from the "all" Markov chain
                        if chat_data.data.contains_key(&user_id) {
                            let markov_chain_clone = chat_data.data.get(&user_id).unwrap().clone();
                            if let Some(all_markov_chain) = chat_data.data.get_mut(ALL) {
                                all_markov_chain.remove_markov_chain(&markov_chain_clone);
                            }
                        }

                        // Delete the user's Markov chain
                        match chat_data.data.entry(user_id) {
                            Occupied(entry) => {
                                entry.remove();
                                write_chat_data(db_url, chat_data).await?;
                                Some(
                                    "Your Markov chain data in this group has been deleted."
                                        .to_string(),
                                )
                            }
                            Vacant(_) => None,
                        }
                    } else {
                        None
                    }
                    .unwrap_or_else(|| "No data found.".to_string())
                } else {
                    "Okay, I won't delete your Markov chain data in this group then.".to_string()
                }
            }

            PromptKind::DeleteUserData(user_id) => {
                if YES_STRINGS.contains(
                    &response
                        .text
                        .as_ref()
                        .unwrap_or(&"".to_string())
                        .to_lowercase()
                        .as_str(),
                ) {
                    if let Some(mut chat_data) = read_chat_data(db_url, &response.chat.id).await? {
                        // Remove the user's Markov chain from the "all" Markov chain
                        if chat_data.data.contains_key(user_id) {
                            let markov_chain_clone = chat_data.data.get(user_id).unwrap().clone();
                            if let Some(all_markov_chain) = chat_data.data.get_mut(ALL) {
                                all_markov_chain.remove_markov_chain(&markov_chain_clone);
                            }
                        }

                        // Delete the user's Markov chain
                        match chat_data.data.entry(user_id.to_string()) {
                            Occupied(entry) => {
                                entry.remove();
                                write_chat_data(db_url, chat_data).await?;
                                Some(
                                    "Their Markov chain data in this group has been deleted."
                                        .to_string(),
                                )
                            }
                            Vacant(_) => None,
                        }
                    } else {
                        None
                    }
                    .unwrap_or_else(|| "No data found.".to_string())
                } else {
                    "Okay, I won't delete their Markov chain data in this group then.".to_string()
                }
            }
        })
    }
}

async fn handle_msg_command_message(
    db_url: &str,
    api: &AsyncApi,
    message: &Message,
    text: &str,
    entities: &[MessageEntity],
) {
    let reply_text = match parse_msg_command_params(text, entities) {
        Err(e) => match e {
            MsgCommandParamsError::TooManySeeds => "<up to one seed word can be provided>".to_string(),
            MsgCommandParamsError::ParseIntError(s) => format!("<invalid integer in length requirement \"{}\">", s).to_string(),
        },
        Ok(params) => {
            debug!("Got /msg for {:?} in chat {}", params.source, message.chat.id);
             match do_msg_command(db_url, &message.chat.id, &params).await {
                Ok(Some(text)) => text,
                Ok(None) | Err(MsgCommandError::MarkovChainError(MarkovChainError::Empty)) => {
                    "<no data>".to_string()
                }
                Err(MsgCommandError::MarkovChainError(MarkovChainError::NoSuchSeed)) => {
                    "<no such seed>".to_string()
                }
                Err(MsgCommandError::MarkovChainError(
                        MarkovChainError::LengthRequirementInvalid,
                    )) => "<invalid length requirement>".to_string(),
                Err(MsgCommandError::MarkovChainError(
                        MarkovChainError::CannotMeetLengthRequirement,
                    )) => "<could not meet length requirement>".to_string(),
                Err(e) => {
                    error!("An error occurred executing /msg command: {:?}", e);
                    "<an error occurred>".to_string()
                }
            }
        }
    };
    try_reply(api, message, reply_text).await;
}

fn parse_msg_command_params<'a>(text: &str, entities: &'a [MessageEntity]) -> Result<MsgCommandParams<'a>, MsgCommandParamsError> {
    // /msg [mention] [seed] [length requirement]
    let command_entity = entities.get(0).unwrap();
    let (source, remaining_text) = match entities.get(1) {
        Some(entity) => {
            if let Some(user_mention) = get_user_mention(text, entity) {
                let remaining_text = text
                    .substring((entity.offset + entity.length) as usize, text.len())
                    .trim();
                (Source::SingleUser(user_mention), remaining_text)
            } else {
                let remaining_text = text
                    .substring(
                        (command_entity.offset + command_entity.length) as usize,
                        text.len(),
                    )
                    .trim();
                (Source::AllUsers, remaining_text)
            }
        }
        None => {
            let remaining_text = text
                .substring(
                    (command_entity.offset + command_entity.length) as usize,
                    text.len(),
                )
                .trim();
            (Source::AllUsers, remaining_text)
        }
    };

    let parts: Vec<&str> = remaining_text.split_whitespace().collect();
    let (seed, length_requirement) = match parts.len() {
        // Neither
        0 => (None, None),

        // Seed or length requirement
        1 => {
            match parse_length_requirement(parts[0]) {
                Err(_) => {
                    return Err(MsgCommandParamsError::ParseIntError(parts[0].to_string()));
                }
                Ok(None) => (Some(parts[0].to_string()), None),
                Ok(Some(length_requirement)) => (None, Some(length_requirement)),
            }
        }

        // Seed and length requirement
        2 => {
            match parse_length_requirement(parts[1]) {
                Err(_) => {
                    return Err(MsgCommandParamsError::ParseIntError(parts[1].to_string()));
                }
                Ok(None) => {
                    return Err(MsgCommandParamsError::TooManySeeds);
                }
                Ok(Some(length_requirement)) => (Some(parts[0].to_string()), Some(length_requirement)),
            }
        }

        // Too many arguments
        _ => {
            return Err(MsgCommandParamsError::TooManySeeds);
        }
    };

    Ok(MsgCommandParams {
        source,
        seed,
        length_requirement,
    })
}

fn parse_length_requirement(s: &str) -> Result<Option<LengthRequirement>, ParseIntError> {
    for (prefix, comparison_operator) in COMPARISON_OPERATORS.iter() {
        if s.starts_with(prefix) {
            let value = s.substring(prefix.len(), s.len()).parse::<i32>()?;
            return Ok(Some(LengthRequirement {
                value,
                comparison_operator: (*comparison_operator).clone(),
            }));
        }
    }
    Ok(None)
}

struct MsgCommandParams<'a> {
    source: Source<'a>,
    seed: Option<String>,
    length_requirement: Option<LengthRequirement>,
}

/// Parses up to one seed value from the given string. [`Err`] is returned if more than one seed value is given.
fn get_seed(text: &str) -> Result<Option<String>, String> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    match parts.len().cmp(&1_usize) {
        Ordering::Equal => Ok(Some(parts.get(0).unwrap().to_string())),
        Ordering::Greater => Err("<up to one seed word can be provided>".to_string()),
        Ordering::Less => Ok(None),
    }
}

/// Stores the message sender's username and user ID so that their username can be associated with their user ID.
async fn remember_message_sender(db_url: &str, message: &Message) -> Result<(), DbError> {
    if let Some(username) = &message.from.as_ref().unwrap().username {
        let username = username.to_lowercase();
        let db = connect_to_db(db_url).await?;
        let user_infos: Collection<UserInfo> = db.collection(USER_INFOS_COLLECTION_NAME);
        let replace_options = {
            let mut replace_options = ReplaceOptions::default();
            replace_options.upsert = Some(true); // Insert new document if an existing one isn't found
            replace_options
        };
        let result = user_infos
            .replace_one(
                doc! {USERNAME_KEY: &username},
                UserInfo {
                    username: username.clone(),
                    user_id: message.from.as_ref().unwrap().id.to_string(),
                },
                replace_options,
            )
            .await;
        if let Err(e) = result {
            error!(
                "Failed to remember username {} has user_id {}",
                &username,
                message.from.as_ref().unwrap().id
            );
            return Err(e);
        }
        debug!(
            "Remembered username {} has user_id {}",
            &username,
            message.from.as_ref().unwrap().id
        );
    }
    Ok(())
}

async fn do_msg_command<'a>(
    db_url: &str,
    chat_id: &i64,
    params: &MsgCommandParams<'a>,
) -> Result<Option<String>, MsgCommandError> {
    let user_id = match &params.source {
        Source::SingleUser(target_user_mention) => target_user_mention.user_id(db_url).await,
        Source::AllUsers => Ok(Some(ALL.to_string())),
    };
    match user_id {
        Err(e) => Err(MsgCommandError::DbError(e)),
        Ok(None) => Ok(None),
        Ok(Some(user_id)) => match read_chat_data(db_url, chat_id).await {
            Err(e) => Err(MsgCommandError::DbError(e)),
            Ok(None) => Ok(None),
            Ok(Some(chat_data)) => match chat_data.data.get(&user_id.to_string()) {
                None => Ok(None),
                Some(markov_chain) => match markov_chain.generate(params.seed.as_ref(),
                                                                  params.length_requirement.as_ref()) {
                    Err(e) => Err(MsgCommandError::MarkovChainError(e)),
                    Ok(words) => Ok(Some(words.join(" "))),
                },
            },
        },
    }
}

/// Attempts to reply to a message with some text, and returns the sent message if successful.
async fn try_reply(api: &AsyncApi, reply_to_message: &Message, text: String) -> Option<Message> {
    let params = SendMessageParamsBuilder::default()
        .chat_id(ChatId::Integer(reply_to_message.chat.id))
        .reply_to_message_id(reply_to_message.message_id)
        .text(text)
        .build()
        .unwrap();
    match api.send_message(&params).await {
        Err(e) => {
            error!("Failed to send reply: {:?}", e);
            None
        }
        Ok(result) => Some(result.result),
    }
}

/// Adds a message to its sender's Markov chain and the "all users" Markov chain in the chat.
async fn add_to_markov_chain(db_url: &str, message: &Message) -> Result<(), DbError> {
    let text = message.text.as_ref().or(message.caption.as_ref());
    match text {
        Some(text) => {
            let mut chat_data = read_chat_data(db_url, &message.chat.id)
                .await?
                .unwrap_or_else(|| ChatData {
                    chat_id: message.chat.id,
                    data: HashMap::new(),
                });
            let sender_id_str = message.from.as_ref().unwrap().id.to_string();
            chat_data.add_message(sender_id_str, text); // Add to the specific user's Markov chain
            chat_data.add_message(ALL.to_string(), text); // Also add to the "all users" Markov chain
            write_chat_data(db_url, chat_data).await
        }

        _ => Ok(()),
    }
}

/// Imports a Telegram chat export JSON file into the Markov chains for that chat. Messages sent by bots and messages
/// starting with a bot command are not included in the import.
async fn import_chat(api: &AsyncApi, db_url: &str, file_path: &str) -> Result<(), ImportError> {
    info!("Reading chat export file {}", file_path);
    let chat_export = match read_chat_export(file_path) {
        Ok(v) => v,
        Err(e) => return Err(ImportError::ReadError(e)),
    };

    info!("Successfully read chat export file; now importing it");
    let mut chat_data = match read_chat_data(db_url, &chat_export.id).await {
        Ok(option) => option.unwrap_or_else(|| ChatData {
            chat_id: chat_export.id,
            data: HashMap::new(),
        }),
        Err(e) => return Err(ImportError::DbError(e)),
    };

    info!(
        "There are {} total messages in the chat export, but they may not all be imported",
        chat_export.messages.len()
    );
    let mut num_messages_imported: i64 = 0;
    let mut users_cache = HashMap::<u64, Option<User>>::new();
    for message in &chat_export.messages {
        if let Some(from_id_str) = &message.from_id {
            let from_id = from_id_str.parse::<u64>().unwrap();

            // Fetch the user info from Telegram if we haven't yet
            if let Vacant(entry) = users_cache.entry(from_id.clone()) {
                let params = GetChatMemberParams {
                    chat_id: ChatId::Integer(chat_data.chat_id),
                    user_id: from_id.clone(),
                };
                match api.get_chat_member(&params).await {
                    Err(e) => {
                        error!("Failed to fetch user with ID {}: {:?}", from_id, e);
                        entry.insert(None);
                    }
                    Ok(response) => {
                        let user = match response.result {
                            ChatMember::Owner(chat_member) => chat_member.user,
                            ChatMember::Administrator(chat_member) => chat_member.user,
                            ChatMember::Member(chat_member) => chat_member.user,
                            ChatMember::Restricted(chat_member) => chat_member.user,
                            ChatMember::Left(chat_member) => chat_member.user,
                            ChatMember::Banned(chat_member) => chat_member.user,
                        };
                        entry.insert(Some(user));
                    }
                }
            }

            if let Some(Some(user)) = users_cache.get(&from_id) {
                if !user.is_bot {
                    // Ignore messages that start with a bot command
                    let include = match &message.contents {
                        MessageContents::PlainText(_) => true,
                        MessageContents::Pieces(pieces) => {
                            if !pieces.is_empty() {
                                if let TextPiece::Entity(entity) = &pieces[0] {
                                    entity.type_field != "bot_command".to_string()
                                } else {
                                    true
                                }
                            } else {
                                false
                            }
                        }
                    };
                    if include {
                        let text = message.to_string();
                        chat_data.add_message(from_id_str.clone(), text.as_str());
                        chat_data.add_message(ALL.to_string(), text.as_str());
                        num_messages_imported += 1;
                    }
                }
            }
        }
    }

    let chat_id = chat_data.chat_id.clone();
    if let Err(e) = write_chat_data(db_url, chat_data).await {
        return Err(ImportError::DbError(e));
    }

    info!(
        "Successfully imported {}/{} messages into chat {}",
        num_messages_imported,
        chat_export.messages.len(),
        chat_id
    );
    Ok(())
}

#[derive(Debug)]
pub enum ImportError {
    ReadError(import::ReadError),
    DbError(DbError),
}

/// Reads the Markov chain data stored for a Telegram chat from the database.
async fn read_chat_data(db_url: &str, chat_id: &i64) -> Result<Option<ChatData>, DbError> {
    let db = connect_to_db(db_url).await?;
    let collection = db.collection(CHATS_COLLECTION_NAME);
    let result = collection
        .find_one(doc! {CHAT_ID_KEY: chat_id.clone()}, None)
        .await;
    match result {
        Ok(chat_data) => {
            debug!("Read chat data for chat {}", chat_id);
            Ok(chat_data)
        }
        Err(e) => {
            error!("Failed to read chat data for chat {}: {:?}", chat_id, e);
            Err(e)
        }
    }
}

/// Writes Markov chain data for a Telegram chat to the database.
async fn write_chat_data(db_url: &str, chat_data: ChatData) -> Result<(), DbError> {
    let db = connect_to_db(db_url).await?;
    let collection: Collection<ChatData> = db.collection(CHATS_COLLECTION_NAME);
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

/// Connects to a MongoDB database.
async fn connect_to_db(db_url: &str) -> Result<Database, DbError> {
    let mut client_options = match ClientOptions::parse(db_url).await {
        Ok(client_options) => client_options,
        Err(e) => {
            error!("Failed to connect to database: {:?}", e);
            return Err(e);
        }
    };
    client_options.app_name = Some("markov-telegram-bot-rs".to_string());
    let client = match Client::with_options(client_options) {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to build database client: {:?}", e);
            return Err(e);
        }
    };
    Ok(client.database("markov"))
}

/// Given a message's text and a [MessageEntity] within it, returns a [UserMention] if one is present.
fn get_user_mention<'a>(text: &str, entity: &'a MessageEntity) -> Option<UserMention<'a>> {
    match &entity.type_field {
        Mention => {
            let username = text
                .get((entity.offset + 1) as usize..(entity.offset + entity.length) as usize)?
                .to_string();
            Some(UserMention::AtMention(username))
        }

        TextMention => Some(UserMention::TextMention(&entity.user.as_ref().unwrap())),

        _ => None,
    }
}

/// Data structure containing Markov chains for a Telegram chat.
#[derive(Serialize, Deserialize, Debug)]
struct ChatData {
    /// ID of the Telegram chat that this data belongs to.
    chat_id: i64,

    /// HashMap from a Telegram user's ID to their Markov chain.
    data: HashMap<String, TripletMarkovChain>,
}

impl ChatData {
    /// Adds a Telegram message to a user's Markov chain.
    fn add_message(&mut self, user_id: String, text: &str) {
        match self.data.entry(user_id.clone()) {
            Occupied(mut entry) => {
                entry.get_mut().add_message(text);
            }
            Vacant(entry) => {
                let mut markov_chain = TripletMarkovChain::default();
                markov_chain.add_message(text);
                entry.insert(markov_chain);
            }
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

/// Enum designating the source of generating a new message (i.e. which user's Markov chain, or the
/// "all users" Markov chain).
#[derive(Debug)]
enum Source<'a> {
    SingleUser(UserMention<'a>),
    AllUsers,
}

/// Enum for the two types of Telegram user mentions.
#[derive(Debug)]
enum UserMention<'a> {
    /// A mention of the form @username. The contained [String] will not include the leading @.
    AtMention(String),

    /// A text mention that is a link to a user who does not have a username.
    TextMention(&'a User),
}

impl<'a> UserMention<'a> {
    /// If the mention is a [UserMention::TextMention], simply returns the linked user's ID.
    /// If the mention is an [UserMention::AtMention], fetches the user ID that maps to the username from the database.
    async fn user_id(&self, db_url: &str) -> Result<Option<String>, DbError> {
        match self {
            UserMention::AtMention(username) => {
                let username = username.to_lowercase();
                let db = connect_to_db(db_url).await?;
                let user_infos: Collection<UserInfo> = db.collection(USER_INFOS_COLLECTION_NAME);
                let user_info = match user_infos
                    .find_one(doc! {USERNAME_KEY: &username}, None)
                    .await
                {
                    Ok(user_info) => user_info,
                    Err(e) => {
                        error!(
                            "Failed to fetch user ID for username \"{}\": {:?}",
                            username, e
                        );
                        return Err(e);
                    }
                };
                debug!("Read user info for username {}: {:?}", &username, user_info);
                Ok(user_info.map(|o| o.user_id))
            }

            UserMention::TextMention(user) => Ok(Some(user.id.to_string())),
        }
    }
}

#[derive(Debug)]
enum MsgCommandParamsError {
    TooManySeeds,
    ParseIntError(String),
}

#[derive(Debug)]
enum MsgCommandError {
    DbError(DbError),
    MarkovChainError(MarkovChainError),
    TooManySeeds,
}
