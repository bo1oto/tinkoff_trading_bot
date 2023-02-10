use tonic::{
    Status,
    service::Interceptor,
    transport::{Channel, ClientTlsConfig}
};
use tcs::{
    {GetAccountsRequest, InstrumentRequest, GetTradingStatusRequest, GetOrderStateRequest,
     PositionsRequest, PostOrderRequest, Quotation, TradingSchedulesRequest, CandleInterval,
     GetCandlesRequest
    },
    orders_service_client::OrdersServiceClient,
    market_data_service_client::MarketDataServiceClient,
    users_service_client::UsersServiceClient,
    instruments_service_client::InstrumentsServiceClient,
    operations_service_client::OperationsServiceClient

};
use crate::bot::{Bot, Statistics, DayStat, ProfitStat};
use tokio::join;


pub mod tcs;
mod bot;
pub mod tg;
pub mod strategies;


#[derive(Debug)]
pub struct DefaultInterceptor {
    token: String,
}
impl Clone for DefaultInterceptor {
    fn clone(&self) -> DefaultInterceptor {
        DefaultInterceptor { token: self.token.clone() }
    }
}
impl Interceptor for DefaultInterceptor {
    fn call(&mut self, request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        let mut req = request;
        req.metadata_mut().append(
            "authorization",
            format!("bearer {}", self.token).parse().unwrap(),
        );
        req.metadata_mut().append(
            "x-tracking-id",
            uuid::Uuid::new_v4().to_string().parse().unwrap(),
        );
        req.metadata_mut()
            .append("x-app-name", "bo1oto".parse().unwrap());

        Ok(req)
    }
}

static mut ACCOUNT_ID: String = String::new();


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe { ACCOUNT_ID = std::env::var("T_ACCOUNT_ID").unwrap() }
    create_env();
    let inter = DefaultInterceptor { token: std::env::var("TOKEN_BOT").unwrap() };
    let channel = Channel::from_static("https://invest-public-api.tinkoff.ru:443/")
        .tls_config(ClientTlsConfig::new())?
        .connect()
        .await?;
    //get_account(channel.clone()).await?;
    //get_status(channel.clone()).await?;
    //get_instrument(channel, inter).await?;
    //test_get_order(channel.clone()).await?;
    //test_order(channel.clone()).await?;
    //get_schedule(channel.clone()).await?;
    //test_candles(channel.clone()).await?;
    //return Ok(());

    loop {
        let (mut disp, rx, tg_bot) = tg::start().await;
        let scalp = strategies::scalp::Scalp::new();

        let bot = Bot::new(channel.clone(),
                               inter.clone(),
                               get_money(channel.clone(), inter.clone()).await?,
                               Box::new(scalp),
                               tg_bot
        );
        let f = || async move {
            disp.dispatch().await;
        };
        let tg_t = tokio::spawn(f());

        let result = join!(bot.handler(rx), tg_t);

        if let Err(err) = result.0 {
            panic!("Something going wrong...\n{}", err)
        } else if let Ok(_) = result.1 {
            break;
        } else {
            println!("Something going wrong in tg_bot, but we will return soon!");
        }
    }

    Ok(())
}

fn create_env() {
    use std::{fs, time};
    use chrono::{DateTime, Local};

    let default_json = {
        let today = {
            let now: DateTime<Local> = time::SystemTime::now().into();
            now.date_naive().to_string()
        };
        let stat = Statistics {
            bot_start_date: today,
            trade_secs: 0,
            turnover: 0,
            profit: ProfitStat {
                net: Quotation { units: 0, nano: 0 },
                after_fees: Quotation { units: 0, nano: 0 },
                after_tax: Quotation { units: 0, nano: 0 }
            },
            trades: Vec::new(),
            trades_count: 0
        };
        serde_json::to_string(&stat).unwrap()
    };
    let default_json_today = {
        let today = {
            let now: DateTime<Local> = time::SystemTime::now().into();
            now.date_naive().to_string()
        };
        let stat =  DayStat {
            date: today,
            turnover: 0,
            profit: ProfitStat {
                net: Quotation { units: 0, nano: 0 },
                after_fees: Quotation { units: 0, nano: 0 },
                after_tax: Quotation { units: 0, nano: 0 }
            },
            trades: Vec::new(),
            trades_count: 0
        };
        serde_json::to_string(&stat).unwrap()
    };
    match fs::read_dir(".\\add_info\\") {
        Err(_) => {
            fs::create_dir(".\\add_info\\").unwrap();
            if cfg!(debug_assertions) {
                fs::write(".\\add_info\\oid.txt", "10000").unwrap();
            } else {
                fs::write(".\\add_info\\oid.txt", "1000000").unwrap();
            }
            fs::write(".\\add_info\\stat.json", default_json).unwrap();
            fs::write(".\\add_info\\today.json", default_json_today).unwrap();
        }
        Ok(_) => {
            if let Err(_) = fs::read(".\\add_info\\oid.txt") {
                if cfg!(debug_assertions) {
                    fs::write(".\\add_info\\oid.txt", "10000").unwrap();
                } else {
                    fs::write(".\\add_info\\oid.txt", "1000000").unwrap();
                }
            }
            if let Err(_) = fs::read(".\\add_info\\stat.json") {
                fs::write(".\\add_info\\stat.json", default_json).unwrap();
            }
            if let Err(_) = fs::read(".\\add_info\\today.json") {
                fs::write(".\\add_info\\today.json", default_json_today).unwrap();
            }
        }
    }
}

pub async fn get_money(channel: Channel, inter: DefaultInterceptor)
                       -> Result<Quotation, Box<dyn std::error::Error>> {
    let req = PositionsRequest {
        account_id: unsafe { ACCOUNT_ID.clone() },
    };
    let mut client =
        OperationsServiceClient::with_interceptor(channel, inter);
    let response = client.get_positions(req).await?;
    let money = &response.get_ref().money;
    for val in money {
        if val.currency.eq("rub") {
            return Ok(Quotation { units: val.units, nano: val.nano });
        }
    }
    panic!("Can't take RUB money!")
}

#[cfg(debug_assertions)]
#[allow(dead_code)]
async fn get_schedule(channel: Channel) -> Result<(), Box<dyn std::error::Error>> {
    use prost_types::Timestamp;
    use std::time::SystemTime;
    use chrono::{TimeZone, Utc};

    let inter = DefaultInterceptor {
        token: String::from(std::env::var("TOKEN_BOT").unwrap())
    };
    let mut client =
        InstrumentsServiceClient::with_interceptor(channel, inter);
    let x: SystemTime = Utc.with_ymd_and_hms(2023, 2, 1, 9, 0, 0).unwrap().into();
    let from = Timestamp::from(x);
    let x: SystemTime = Utc.with_ymd_and_hms(2023, 2, 2, 9, 0, 0).unwrap().into();
    let to = Timestamp::from(x);
    let req = TradingSchedulesRequest {
        from: Some(from),
        to: Some(to),
        exchange: "".to_string(),
    };

    let response = client.trading_schedules(req).await?;
    println!("{:#?}", response.into_inner());
    Ok(())
}
#[cfg(debug_assertions)]
#[allow(dead_code)]
async fn get_instrument(channel: Channel) -> Result<(), Box<dyn std::error::Error>> {
    let inter = DefaultInterceptor {
        token: String::from(std::env::var("TOKEN_BOT").unwrap())
    };
    let mut client =
        InstrumentsServiceClient::with_interceptor(channel, inter);
    let req = InstrumentRequest {
        //instrument_status: i32::from(InstrumentType::Share),
        id_type: 2,
        class_code: "TQBR".to_string(),
        id: "IRAO".to_string(),
    };

    let response = client.get_instrument_by(req).await?;
    println!("{:#?}", response.into_inner());
    Ok(())
}
#[cfg(debug_assertions)]
#[allow(dead_code)]
async fn get_status(channel: Channel) -> Result<(), Box<dyn std::error::Error>> {
    let inter = DefaultInterceptor {
        token: String::from(std::env::var("TOKEN_BOT").unwrap())
    };
    let mut client =
        MarketDataServiceClient::with_interceptor(channel, inter);
    let req = GetTradingStatusRequest {
        figi: "BBG000000001".to_string(),// IRAO figi: "BBG004S68473"
        instrument_id: "e2d0dbac-d354-4c36-a5ed-e5aae42ffc76".to_string(),// IRAO uid: "427f9bcc-2cab-4561-bf94-942d4261fbb7"
    };

    let response = client.get_trading_status(req).await?;
    println!("{:#?}", response.into_inner());
    Ok(())
}
#[cfg(debug_assertions)]
#[allow(dead_code)]
async fn get_account(channel: Channel) -> Result<(), Box<dyn std::error::Error>> {
    let inter = DefaultInterceptor {
        token: String::from(std::env::var("TOKEN_BOT").unwrap())
    };
    let mut client =
        UsersServiceClient::with_interceptor(channel, inter);
    let req = GetAccountsRequest { };

    let response = client.get_accounts(req).await?;
    println!("{:#?}", response.into_inner());
    Ok(())
}
#[cfg(debug_assertions)]
#[allow(dead_code)]
async fn test_get_order(channel: Channel) -> Result<(), Box<dyn std::error::Error>> {
    let inter = DefaultInterceptor {
        token: String::from(std::env::var("TOKEN_BOT").unwrap())
    };
    let req = GetOrderStateRequest {
        account_id: unsafe { ACCOUNT_ID.clone() },
        order_id: "1000012".to_string()
    };
    let mut client = OrdersServiceClient::with_interceptor(
        channel, inter);
    let response = client.get_order_state(req).await?;
    println!("{:?}", response.into_inner());
    Ok(())
}
#[cfg(debug_assertions)]
#[allow(dead_code)]
async fn test_order(channel: Channel) -> Result<(), Box<dyn std::error::Error>> {
    let inter = DefaultInterceptor {
        token: String::from(std::env::var("TOKEN_BOT").unwrap())
    };
    let req = PostOrderRequest {
        figi: "BBG000000001".to_string(),
        quantity: 1,
        price: Some(Quotation { units: 5, nano: 94_0000000 }),
        direction: 1,
        account_id: unsafe { ACCOUNT_ID.clone() },
        order_type: 1,// LIMIT
        order_id: "7".to_string(),//"34177515807"
        instrument_id: "e2d0dbac-d354-4c36-a5ed-e5aae42ffc76".to_string(),
    };
    let mut client = OrdersServiceClient::with_interceptor(
        channel, inter);
    let response = client.post_order(req).await?;
    let order_id = response.get_ref().order_id.clone();
    println!("{:?}", response.into_inner());
    std::thread::sleep(std::time::Duration::new(5, 0));
    let req = GetOrderStateRequest {
        account_id: unsafe { ACCOUNT_ID.clone() },
        order_id: order_id,
    };
    let response = client.get_order_state(req).await?;
    println!("{:?}", response.into_inner());
    Ok(())
}
#[cfg(debug_assertions)]
#[allow(dead_code)]
async fn test_candles(channel: Channel) -> Result<(), Box<dyn std::error::Error>> {
    use prost_types::Timestamp;
    use std::time::SystemTime;
    use chrono::{TimeZone, Utc};

    let inter = DefaultInterceptor {
        token: String::from(std::env::var("TOKEN_BOT").unwrap())
    };
    let x: SystemTime = Utc.with_ymd_and_hms(2023, 2, 9, 9, 0, 0).unwrap().into();
    let from = Timestamp::from(x);
    let x: SystemTime = Utc.with_ymd_and_hms(2023, 2, 9, 23, 0, 0).unwrap().into();
    let to = Timestamp::from(x);
    let req = GetCandlesRequest {
        figi: "BBG000000001".to_string(),
        interval: CandleInterval::CandleInterval5Min.into(),
        from: Some(from),
        to: Some(to),
        instrument_id: "e2d0dbac-d354-4c36-a5ed-e5aae42ffc76".to_string(),
    };
    let mut client = MarketDataServiceClient::with_interceptor(
        channel, inter);
    let response = client.get_candles(req).await?;
    println!("{:#?}", response.into_inner());
    Ok(())
}


