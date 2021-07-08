use std::collections::HashMap;

use clap::{App, Arg};
use futures::StreamExt;
use mongodb::{Client, Database};
use mongodb::bson::doc;
use mongodb::options::ClientOptions;
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
                match parse_update(&update) {
                    Some(BotAction::Msg(message, user_mention)) => {
                        if let Err(error) = api.send(message.text_reply("Foo")).await {
                            println!("Failed to send /msg reply: {}", error);
                        }
                    }

                    Some(BotAction::MsgAll(message)) => {
                        if let Err(error) = api.send(message.text_reply("Foo")).await {
                            println!("Failed to send /msgall reply: {}", error);
                        }
                    }

                    Some(BotAction::AddToMarkovChain(message)) => {
                        if let Err(error) = add_to_markov_chain(message).await {
                            println!("Failed to add message to markov chain: {:?}", error);
                        }
                    }

                    None => {}
                }
            }

            Err(error) => {
                println!("Failed to fetch update: {:?}", error);
            }
        }
    }
    Ok(())
    //
    // // Open a connection to the mini-redis address.
    // let mut client = mini_redis::client::connect("127.0.0.1:6379").await?;
    //
    // // Set the key "hello" with value "world"
    // let r = client.set("hello", "world".into()).await?;
    // println!("set value on server; result={:?}", r);
    //
    // // Get key "hello"
    // let result = client.get("hello").await?;
    // println!("got value from the server; result={:?}", result);
    //
    // Ok(())
}

fn parse_update(update: &Update) -> Option<BotAction> {
    match &update.kind {
        UpdateKind::Message(message) => parse_message(message),
        _ => None,
    }
}

fn parse_message(message: &Message) -> Option<BotAction> {
    match message.kind {
        MessageKind::Text { data: ref text, ref entities } =>
            parse_text(message, text, entities),

        _ => None,
    }.or_else(|| Some(BotAction::AddToMarkovChain(message)))
}

fn parse_text<'a>(message: &'a Message, text: &String, entities: &'a Vec<MessageEntity>) -> Option<BotAction<'a>> {
    if let Some(MessageEntity { kind: MessageEntityKind::BotCommand, ref offset, ref length }) = entities.get(0) {
        if let Some(command_string) = text.get((offset + 1) as usize..(offset + length) as usize) {
            match command_string {
                "msg" => {
                    if let Some(entity) = entities.get(1) {
                        if let Some(mention) = get_user_mention(text, &entity) {
                            println!("got /msg for {:?}", mention);
                            return Some(BotAction::Msg(message, mention));
                        }
                    }
                }

                "msgall" => {
                    println!("got /msgall");
                    return Some(BotAction::MsgAll(message));
                }

                _ => {}
            }
        }
    }

    None
}

async fn add_to_markov_chain(message: &Message) -> Result<(), mongodb::error::Error> {
    match &message.text() {
        Some(text) => {
            let chat_id = message.chat.id();
            let mut chat_data = read_chat_data(chat_id)
                .await?
                .or_else(|| Some(ChatData { chat_id: chat_id.into(), chat_data: HashMap::new() }))
                .unwrap();
            chat_data.add_message(&message.from.id.into(), text);
            write_chat_data(chat_data).await
        }

        _ => Ok(()),
    }
}

async fn read_chat_data(chat_id: ChatId) -> Result<Option<ChatData>, mongodb::error::Error> {
    let db = connect_to_db().await?;
    let collection = db.collection_with_type::<ChatData>("chats");
    let chat_id_raw: Integer = chat_id.into();
    let option = collection.find_one(doc! {"chat_id": chat_id_raw}, None).await?;
    println!("Read chat data {:?}", option);
    Ok(option)
}

async fn write_chat_data(chat_data: ChatData) -> Result<(), mongodb::error::Error> {
    let db = connect_to_db().await?;
    let collection = db.collection_with_type::<ChatData>("chats");
    collection.insert_one(chat_data, None).await?;
    Ok(())
}

async fn connect_to_db() -> Result<Database, mongodb::error::Error> {
    let mut client_options = ClientOptions::parse("mongodb://localhost:27017").await?;
    client_options.app_name = Some("markov-telegram-bot-rs".to_string());
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
            )?.to_string();
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
    chat_id: Integer,
    chat_data: HashMap<Integer, MarkovChain>,
}

impl ChatData {
    fn add_message(&mut self, user_id: &Integer, text: &String) {
        if self.chat_data.contains_key(user_id) {
            let markov_chain = self.chat_data.get_mut(user_id).unwrap();
            markov_chain.add_message(text);
        } else {
            let mut markov_chain = MarkovChain { user_id: *user_id, markov_chain: HashMap::new() };
            markov_chain.add_message(text);
            self.chat_data.insert(user_id.clone(), markov_chain);
        }
    }
}

// HashMap of word (#1) to HashMap of following word (#2) to number of times #2 followed #1.
#[derive(Serialize, Deserialize, Debug)]
struct MarkovChain {
    user_id: Integer,
    markov_chain: HashMap<String, HashMap<String, u32>>,
}

impl MarkovChain {
    fn add_message(&mut self, text: &String) {
        let mut words = text.split_whitespace().peekable();
        if let Some(_) = words.peek() {
            let mut last_word = "";
            for word in words {
                match self.markov_chain.get_mut(last_word) {
                    Some(word_map) => {
                        match word_map.get(word) {
                            Some(count) => {
                                let new_count = count + 1;
                                word_map.insert(word.to_owned(), new_count);
                            }
                            None => {
                                word_map.insert(word.to_owned(), 1);
                            }
                        }
                    }
                    None => {
                        let mut word_map = HashMap::new();
                        word_map.insert(word.to_owned(), 1);
                        self.markov_chain.insert(last_word.to_owned(), word_map);
                    }
                }
                last_word = word;
            }
        }
    }
}

#[derive(Debug)]
enum BotAction<'a> {
    AddToMarkovChain(&'a Message),
    Msg(&'a Message, UserMention<'a>),
    MsgAll(&'a Message),
}

#[derive(Debug)]
enum UserMention<'a> {
    /// A mention of the form @username.
    AtMention(String),

    /// A text mention that is a link to a user, without the leading @.
    TextMention(&'a User),
}
