use std::collections::HashMap;

use clap::{App, Arg};
use futures::StreamExt;
use mongodb::{Client, Database};
use mongodb::bson::doc;
use mongodb::options::{ClientOptions, ReplaceOptions};
use rand::prelude::IteratorRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};
use telegram_bot::{Api, CanReplySendMessage, ChatId, Integer, Message, MessageEntity, MessageEntityKind, MessageKind, MessageText, Update, UpdateKind, User};

#[tokio::main]
async fn main() -> Result<(), String> {
    let args = App::new("markov-telegram-bot-rs")
        .arg(Arg::with_name("TELEGRAM_BOT_TOKEN")
            .short("t")
            .long("token")
            .help("Telegram bot token given by @BotFather")
            .required(true)
            .takes_value(true))
        .get_matches();

    let token = args.value_of("TELEGRAM_BOT_TOKEN").unwrap();
    let api = Api::new(token);

    let mut stream = api.stream();
    while let Some(update) = stream.next().await {
        match update {
            Ok(update) => {
                handle_update(&api, &update).await;
            }

            Err(error) => {
                println!("Failed to fetch update: {:?}", error);
            }
        }
    }
    Ok(())
}

async fn handle_update(api: &Api, update: &Update) {
    if let UpdateKind::Message(message) = &update.kind {
        handle_message(api, message).await;
    }
}

async fn handle_message(api: &Api, message: &Message) {
    if let Err(e) = remember_message_sender(message).await {
        println!("Failed to remember user {:?}: {:?}", message.from, e);
    }

    if let Some(text) = &message.text() {
        // Check for bot commands
        if let MessageKind::Text { ref entities, .. } = message.kind {
            if let Some(MessageEntity { kind: MessageEntityKind::BotCommand, ref offset, ref length }) = entities.get(0) {
                match text.get((offset + 1) as usize..(offset + length) as usize) {
                    Some(command_text) => {
                        if command_text == "msg" || command_text.starts_with("msg@") {
                            let reply_text = if let Some(entity) = entities.get(1) {
                                if let Some(user_mention) = get_user_mention(text, entity) {
                                    let seed = if let Some(rest) = text.get((entity.offset + entity.length) as usize..) {
                                        let rest_parts: Vec<&str> = rest.split_whitespace().collect();
                                        if rest_parts.len() == 1 {
                                            Ok(Some(rest_parts.get(0).unwrap().to_string()))
                                        } else if rest_parts.len() > 1 {
                                            Err("<up to one seed word can be provided>".to_owned())
                                        } else { Ok(None) }
                                    } else { Ok(None) };
                                    match seed {
                                        Err(e) => Some(e),
                                        Ok(seed) => {
                                            println!("Got /msg for {:?}", user_mention);
                                            match do_msg_command(&message.chat.id(), &user_mention, seed).await {
                                                Ok(Some(text)) => Some(text),
                                                Ok(None) => Some("<no data>".to_owned()),
                                                Err(MsgCommandError::MarkovChainError(MarkovChainError::NoSuchSeed)) =>
                                                    Some("<no such seed>".to_owned()),
                                                Err(e) => {
                                                    println!("An error occurred executing /msg command: {:?}", e);
                                                    Some("<an error occurred>".to_owned())
                                                }
                                            }
                                        }
                                    }
                                } else { None }
                            } else { None }.unwrap_or("<expected a user mention>".to_owned());
                            if let Err(e) = api.send(message.text_reply(reply_text)).await {
                                println!("Failed to send reply: {:?}", e);
                            }
                            return;
                        }
                    }

                    _ => {}
                }
            }
        }

        // If message was not handled by some bot command, add it to the sending user's markov chain
        if let Err(e) = add_to_markov_chain(message).await {
            println!("Failed to add message to markov chain: {:?}", e);
        };
    }
}

async fn remember_message_sender(message: &Message) -> Result<(), mongodb::error::Error> {
    if let Some(username) = &message.from.username {
        let username = username.to_lowercase();
        let db = connect_to_db().await?;
        let user_infos = db.collection_with_type::<UserInfo>("user_infos");
        let mut replace_options = ReplaceOptions::default();
        replace_options.upsert = Some(true);
        user_infos.replace_one(doc! {"username": &username},
                               UserInfo { username: username.clone(), user_id: message.from.id.to_string() },
                               replace_options).await?;
        println!("Remembered username {} has user_id {}", &username, message.from.id.to_string());
    }
    Ok(())
}

async fn do_msg_command<'a>(chat_id: &ChatId, target_user_mention: &UserMention<'a>, seed: Option<String>) -> Result<Option<String>, MsgCommandError> {
    match target_user_mention.user_id().await {
        Err(e) => Err(MsgCommandError::DbError(e)),
        Ok(None) => Ok(None),
        Ok(Some(user_id)) => {
            match read_chat_data(chat_id).await {
                Err(e) => Err(MsgCommandError::DbError(e)),
                Ok(None) => Ok(None),
                Ok(Some(chat_data)) => {
                    match chat_data.chat_data.get(&user_id) {
                        None => Ok(None),
                        Some(markov_chain) => {
                            match markov_chain.generate(seed) {
                                Err(e) => Err(MsgCommandError::MarkovChainError(e)),
                                Ok(words) => {
                                    Ok(Some(words.join(" ")))
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
enum MsgCommandError {
    MarkovChainError(MarkovChainError),
    DbError(mongodb::error::Error),
}

async fn add_to_markov_chain(message: &Message) -> Result<(), mongodb::error::Error> {
    match &message.text() {
        Some(text) => {
            let chat_id = message.chat.id();
            let chat_id_raw: Integer = chat_id.into();
            let chat_id_str = chat_id_raw.to_string();
            let mut chat_data = read_chat_data(&chat_id)
                .await?
                .or_else(|| Some(ChatData { chat_id: chat_id_str, chat_data: HashMap::new() }))
                .unwrap();
            let sender_id_raw: Integer = message.from.id.into();
            let sender_id_str = sender_id_raw.to_string();
            chat_data.add_message(&sender_id_str, text);
            write_chat_data(chat_data).await
        }

        _ => Ok(()),
    }
}

async fn read_chat_data(chat_id: &ChatId) -> Result<Option<ChatData>, mongodb::error::Error> {
    let chat_id_raw: Integer = (*chat_id).into();
    let chat_id_str = chat_id_raw.to_string();
    let db = connect_to_db().await?;
    let collection = db.collection_with_type::<ChatData>("chats");
    let option = collection.find_one(doc! {"chat_id": chat_id_str}, None).await?;
    println!("Read chat data {:?}", option);
    Ok(option)
}

async fn write_chat_data(chat_data: ChatData) -> Result<(), mongodb::error::Error> {
    let db = connect_to_db().await?;
    let collection = db.collection_with_type::<ChatData>("chats");
    let mut replace_options = ReplaceOptions::default();
    replace_options.upsert = Some(true);
    let msg = format!("Wrote chat data {:?}", chat_data);
    collection.replace_one(doc! {"chat_id": chat_data.chat_id.clone()},
                           chat_data,
                           Some(replace_options)).await?;
    println!("{}", msg);
    Ok(())
}

async fn connect_to_db() -> Result<Database, mongodb::error::Error> {
    let mut client_options = ClientOptions::parse("mongodb://localhost:27017").await?;
    client_options.app_name = Some("markov-telegram-bot-rs".to_owned());
    let client = Client::with_options(client_options)?;
    Ok(client.database("markov"))
}

/// Given a message's text and a `MessageEntity` within it, returns a `UserMention` if one is
/// present.
fn get_user_mention<'a>(text: &String, entity: &'a MessageEntity) -> Option<UserMention<'a>> {
    match &entity.kind {
        MessageEntityKind::Mention => {
            let username = text.get((entity.offset + 1) as usize..
                (entity.offset + entity.length) as usize
            )?.to_owned();
            Some(UserMention::AtMention(username))
        }

        MessageEntityKind::TextMention(user) => {
            Some(UserMention::TextMention(user))
        }

        _ => None,
    }
}

// HashMap of userId to that user's markov chain in the group.
#[derive(Serialize, Deserialize, Debug)]
struct ChatData {
    chat_id: String,
    chat_data: HashMap<String, MarkovChain>,
}

impl ChatData {
    fn add_message(&mut self, user_id: &String, text: &String) {
        if self.chat_data.contains_key(user_id) {
            let markov_chain = self.chat_data.get_mut(user_id).unwrap();
            markov_chain.add_message(text);
        } else {
            let mut markov_chain = MarkovChain { user_id: user_id.clone(), markov_chain: HashMap::new() };
            markov_chain.add_message(text);
            self.chat_data.insert(user_id.clone(), markov_chain);
        }
    }
}

// HashMap of word (#1) to HashMap of following word (#2) to number of times #2 followed #1.
#[derive(Serialize, Deserialize, Debug)]
struct MarkovChain {
    user_id: String,
    markov_chain: HashMap<String, HashMap<String, i32>>,
}

impl MarkovChain {
    fn generate(&self, seed: Option<String>) -> Result<Vec<String>, MarkovChainError> {
        if self.markov_chain.is_empty() {
            return Err(MarkovChainError::Empty);
        }
        let mut word = match seed {
            // Use the given seed word
            Some(word) => {
                if !self.markov_chain.contains_key(&word) {
                    return Err(MarkovChainError::NoSuchSeed);
                }
                word
            }
            // Pick a random starting seed word
            None => {
                match self.markov_chain.get("") {
                    None => return Err(MarkovChainError::Empty),
                    Some(word_map) => word_map.keys().choose(&mut rand::thread_rng()).unwrap().clone(),
                }
            }
        };

        let mut result: Vec<String> = vec![];
        while word != "" {
            result.push(word.clone());
            match self.markov_chain.get(&word) {
                None => { // Should never happen based on how we build the markov chains
                    println!("Expected word {} to be in the markov chain but it wasn't", word);
                    return Err(MarkovChainError::InternalError);
                }
                Some(word_map) => {
                    let mut cumulative_distribution: Vec<(i32, &String)> = vec![];
                    let mut n = 0;
                    for (following_word, count) in word_map {
                        n += count;
                        cumulative_distribution.push((n, following_word));
                    }
                    let random = rand::thread_rng().gen_range(0..n);
                    let mut next_word: Option<String> = None;
                    for (cumulative_value, following_word) in cumulative_distribution {
                        if random < cumulative_value {
                            next_word = Some(following_word.clone());
                            break;
                        }
                    }
                    if next_word == None { // Should never happen
                        println!("Failed to pick next word in cumulative distribution");
                        return Err(MarkovChainError::InternalError);
                    }
                    word = next_word.unwrap();
                }
            }
        }

        Ok(result)
    }

    /// Adds each word pair in the given String (separated by whitespace) to the markov chain.
    fn add_message(&mut self, text: &String) {
        let mut words = text.split_whitespace().peekable();
        if let Some(_) = words.peek() {
            let mut last_word = "";
            for word in words {
                self.add_word_pair(&last_word.to_owned(), &word.to_owned());
                last_word = word;
            }
            self.add_word_pair(&last_word.to_owned(), &"".to_owned());
        }
    }

    /// Adds a pair of words to the markov chain.
    fn add_word_pair(&mut self, first: &String, second: &String) {
        match self.markov_chain.get_mut(first) {
            Some(word_map) => match word_map.get(second) {
                Some(count) => {
                    let new_count = count + 1;
                    word_map.insert(second.clone(), new_count);
                }
                None => {
                    word_map.insert(second.clone(), 1);
                }
            },
            None => {
                let mut word_map = HashMap::new();
                word_map.insert(second.clone(), 1);
                self.markov_chain.insert(first.clone(), word_map);
            }
        }
    }
}

#[derive(Debug)]
enum MarkovChainError {
    Empty,
    NoSuchSeed,
    InternalError,
}

#[derive(Serialize, Deserialize, Debug)]
struct UserInfo {
    username: String,
    user_id: String,
}

#[derive(Debug)]
enum UserMention<'a> {
    /// A mention of the form @username. The contained String will not include the leading @.
    AtMention(String),

    /// A text mention that is a link to a user that does not have a username.
    TextMention(&'a User),
}

impl<'a> UserMention<'a> {
    async fn user_id(&self) -> Result<Option<String>, mongodb::error::Error> {
        match self {
            UserMention::AtMention(username) => {
                let username = username.to_lowercase();
                let db = connect_to_db().await?;
                let user_infos = db.collection_with_type::<UserInfo>("user_infos");
                let user_info = user_infos.find_one(doc! {"username": &username}, None).await?;
                println!("Read user info for username {}: {:?}", &username, user_info);
                Ok(user_info.map(|o| o.user_id))
            }
            UserMention::TextMention(user) => Ok(Some(user.id.to_string())),
        }
    }
}
