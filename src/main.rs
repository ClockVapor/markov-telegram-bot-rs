use std::thread::park_timeout;

use clap::{App, Arg};
use futures::StreamExt;
use mini_redis;
use telegram_bot::*;

#[tokio::main]
async fn main() -> Result<(), Error> {
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
                        add_to_markov_chain(message);
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

fn add_to_markov_chain(message: &Message) -> Option<Error> {
    match &message.text() {
        Some(text) => {
            if !text.chars().all(|c| c.is_whitespace()) {
                for word in text.split_whitespace() {}
            }
            todo!()
        }

        _ => None,
    }
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
