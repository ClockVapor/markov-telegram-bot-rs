use clap::{App, Arg};

use markov_chain::*;
use markov_telegram_bot::MarkovTelegramBot;

mod markov_chain;
mod markov_telegram_bot;

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::init();

    let args = App::new("markov-telegram-bot-rs")
        .arg(
            Arg::with_name("TELEGRAM_BOT_TOKEN")
                .short("t")
                .long("token")
                .help("Telegram bot token given by @BotFather")
                .required(true)
                .takes_value(true),
        )
        .get_matches();

    let bot_token = args.value_of("TELEGRAM_BOT_TOKEN").unwrap();
    MarkovTelegramBot::new().run(bot_token).await
}
