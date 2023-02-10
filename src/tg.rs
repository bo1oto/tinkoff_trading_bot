use std::sync::Arc;
use tokio::sync::{
    Mutex,
    mpsc::{Sender, Receiver, channel}
};
use teloxide::{
    prelude::*, RequestError,
    utils::command::BotCommands,
    dispatching::DefaultKey,
    dptree::deps
};

static mut MY_CHAT_ID: ChatId = ChatId(0);

#[derive(Debug)]
pub enum RequestType {
    Something = 3,
    StateRequest = 2,
    StatRequest = 1,
    StopRequest = 0,
}


pub async fn start() -> (Dispatcher<Bot, RequestError, DefaultKey>, Receiver<RequestType>, Bot) {
    use std::str::FromStr;

    unsafe {
        MY_CHAT_ID = ChatId(i64::from_str(std::env::var("TG_CHAT_ID").unwrap().as_str()).unwrap())
    }
    pretty_env_logger::init();

    let (tx, rx): (Sender<RequestType>, Receiver<RequestType>) = channel(10);

    let dmap = deps![Arc::new(Mutex::new(tx))];

    let bot = Bot::from_env();
    let command_handler = teloxide::filter_command::<Command, _>().endpoint(answer);
    let message_handler = Update::filter_message()
        .branch(command_handler)
        .branch(dptree::endpoint(get_message));
    (
        Dispatcher::builder(bot.clone(), message_handler)
            .dependencies(dmap)
            .build(),
        rx,
        bot
    )
}

pub async fn send_message(bot: Arc<Bot>, message: String) {
    let _ = bot.send_message(unsafe { MY_CHAT_ID }, message).await;
}

async fn get_message(_: Bot, _: Message, tx: Arc<Mutex<Sender<RequestType>>>)
    -> ResponseResult<()> {
    tx.lock().await.send(RequestType::Something).await.unwrap();
    Ok(())
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Поддерживаемые команды:")]
pub enum Command {
    #[command(description = "описание команд")]
    Help,
    #[command(description = "портфель, ордера, дневная статистика")]
    State,
    #[command(description = "статистика торговли")]
    Stat,
    #[command(description = "остановить работу бота")]
    Stop,
}

pub async fn answer(bot: Bot, msg: Message, cmd: Command, tx: Arc<Mutex<Sender<RequestType>>>)
    -> ResponseResult<()> {
    if msg.chat.id != unsafe { MY_CHAT_ID } {
        return Ok(())
    }
    match cmd {
        Command::Help => bot.send_message(msg.chat.id, Command::descriptions().to_string()).await?,
        Command::State => {
            tx.lock().await.send(RequestType::StateRequest).await.unwrap();
            bot.send_message(msg.chat.id, format!("Запрашиваю состояние портфеля")).await?
        },
        Command::Stat => {
            tx.lock().await.send(RequestType::StatRequest).await.unwrap();
            bot.send_message(msg.chat.id, format!("Запрашиваю статистику")).await?
        }
        Command::Stop => {
            tx.lock().await.send(RequestType::StopRequest).await.unwrap();
            bot.send_message(msg.chat.id, format!("Запрашиваю остановку")).await?
        }
    };
    Ok(())
}