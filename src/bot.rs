use std:: {
    sync::Arc,
    fmt::Write,
    time::{Duration, Instant, SystemTime}
};
use chrono::{DateTime, Utc, Timelike, TimeZone, Datelike};
use chrono_tz::Europe::Moscow;
use prost_types::Timestamp;

use std::fmt::{Debug, Display, Formatter};
use tonic:: {
    transport::Channel,
    codegen::InterceptedService,
};
use crate::DefaultInterceptor;
use crate::tcs::{CancelOrderRequest, GetOrderBookRequest, GetOrderBookResponse, GetOrdersRequest,
                 GetOrderStateRequest, OrderDirection, PositionsRequest, PostOrderRequest,
                 Quotation, CandleInterval, GetCandlesRequest, GetCandlesResponse,
                 market_data_service_client::MarketDataServiceClient,
                 operations_service_client::OperationsServiceClient,
                 orders_service_client::OrdersServiceClient, OrderType};
use tokio::sync::mpsc::Receiver;
use serde::{
    {Deserialize, Deserializer, Serialize, Serializer},
    de::Error,
    ser::SerializeStruct
};
use serde_json::{Value};

use crate::tg::{send_message, RequestType};


impl Serialize for Quotation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer {
        let mut ser = serializer.serialize_struct("Quotation", 2)?;
        ser.serialize_field("units", &self.units)?;
        ser.serialize_field("nano", &self.nano)?;
        ser.end()

    }
}
impl<'de> Deserialize<'de> for Quotation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de> {
        let value = Value::deserialize(deserializer)?;
        if let (Some(units), Some(nano)) = (value.get("units"), value.get("nano")) {
            Ok(Quotation {
                units: units.as_i64().unwrap(),
                nano: nano.as_i64().unwrap() as i32,
            })
        } else {
            Err(Error::custom("invalid Quotation"))
        }
    }
}
impl std::ops::Add for Quotation {
    type Output = Quotation;
    fn add(self, rhs: Self) -> Self::Output {
        let mut nano_sum = self.nano + rhs.nano;
        let mut units_sum = self.units + rhs.units;
        if nano_sum > 99_9999999 {
            nano_sum -= 100_0000000;
            units_sum += 1;
        }
        Quotation { units: units_sum, nano: nano_sum }
    }
}
impl std::ops::Sub for Quotation {
    type Output = Quotation;
    fn sub(self, rhs: Self) -> Self::Output {
        let mut nano_sub = self.nano - rhs.nano;
        let mut units_sub = self.units - rhs.units;
        if nano_sub < 0 {
            nano_sub = 100_0000000 + nano_sub;
            units_sub -= 1;
        }
        Quotation { units: units_sub, nano: nano_sub }
    }
}
impl std::ops::Mul for Quotation {
    type Output = Quotation;
    fn mul(self, rhs: Self) -> Self::Output {
        const MAX: i64 = 99_9999999;
        const NUM_LENGTH: i32 = 9;
        let first_mul = {
            let uu = self.units * rhs.units;
            let un = self.units * rhs.nano as i64;
            Quotation { units: uu + un / 100_0000000, nano: (un % 100_0000000) as i32 }
        };
        let second_mul = {
            let nu = self.nano as i64 * rhs.units;
            let shift = {
                let mut n = self.nano;
                let mut res = 0;
                while n != 0 {
                    if n % 10 != 0 {
                        break;
                    }
                    res += 1;
                    n /= 10;
                }
                let shift1 = NUM_LENGTH - res;
                let mut n = rhs.nano;
                let mut res = 0;
                while n != 0 {
                    if n % 10 != 0 {
                        break;
                    }
                    res += 1;
                    n /= 10;
                }
                let shift2 = NUM_LENGTH - res;
                shift1 + shift2 - 1
            };
            let mut nn = self.nano as i64 * rhs.nano as i64;
            let signs = {
                let mut n = nn;
                let mut res = 0;
                let mut is_count = false;
                while n != 0 {
                    if n % 10 != 0 {
                        is_count = true;
                    }
                    if is_count {
                        res += 1;
                    }
                    n /= 10;
                }
                res - 1
            };
            while nn > MAX {
                nn /= 10;
            }
            let shift = shift - signs;
            for _ in 0..shift {
                nn /= 10;
            }
            let nano = nu + nn;
            let units = nano / 100_0000000;
            Quotation { units, nano: (nano % 100_0000000) as i32 }
        };
        first_mul + second_mul
    }
}
impl Display for Quotation {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut nano = self.nano;
        while nano % 10 == 0 {
            nano /= 10;
        }
        write!(f, "{}.{}", self.units, nano)
    }
}


#[derive(Serialize, Deserialize)]
pub struct Statistics {
    pub bot_start_date: String,
    pub trade_secs: u32,
    pub turnover: u32,
    pub profit: ProfitStat,
    pub trades: Vec<TradeStat>,
    pub trades_count: u16
}
#[derive(Serialize, Deserialize)]
pub struct DayStat {
    pub date: String,
    pub turnover: u32,
    pub profit: ProfitStat,
    pub trades: Vec<TradeStat>,
    pub trades_count: u16
}
#[derive(Serialize, Deserialize, Clone)]
pub struct ProfitStat {
    pub net: Quotation,
    pub after_fees: Quotation,
    pub after_tax: Quotation
}
#[derive(Serialize, Deserialize)]
pub struct TradeStat {
    pub time_in: String,
    pub time_out: String,
    pub price_in: (i64, i32),
    pub price_out: (i64, i32),
    pub direction: bool,
    pub turnover: u32,
    pub profit: ProfitStat,
}
impl Display for ProfitStat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.net.units == 0 && self.net.nano == 0 {
            write!(f, "0")
        } else {
            write!(f, "{} -> {} -> {}", self.net, self.after_fees, self.after_tax)
        }
    }
}
impl Display for DayStat {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Сегодня: {}\nСделок: {}, Прибыль: {}", self.date, self.trades_count, self.profit)
    }
}

pub enum AnalysisType {
    OrderBook(i32),
    Candle(CandleInterval)
}

pub struct Settings {
    pub account_id: String,

    pub ticker: String,
    pub uid: String,
    pub figi: String,

    pub class_code: String,
    pub trading_time: Vec<(u32, u32, u32, u32)>,
    pub data_type: AnalysisType,

    pub fee_rate: Quotation,
    pub tax_rate: Quotation,
}

pub trait Strategy {
    fn analyze_ob(&mut self, orders: &GetOrderBookResponse, state: &State) -> Action {
        panic!("Analyze order book not implemented")
    }
    fn analyze_c(&mut self, candles: &GetCandlesResponse, state: &State) -> Action {
        panic!("Analyze candles not implemented")
    }
    fn get_take_profit(&mut self, p: Quotation, l: i64) -> (Quotation, i64, OrderDirection);
    fn get_settings(&self) -> Settings;
}


#[derive(PartialEq, Debug)]
pub enum Action {
    Close(Quotation, i64, OrderDirection),
    Open(Quotation, i64, OrderDirection),
    Hold,
}

#[derive(PartialEq, Debug)]
enum PosState {
    WaitOpen,
    WaitClose,
    PartialOpen,
    PartialClose,
    Hold,
}

#[derive(Debug)]
pub struct Position {
    state: PosState,
    pub price_in: Quotation,
    pub lots: i64,
    pub direction: OrderDirection,
    price_out: Quotation,
    id: (String, String) // open_id, close_id
}



#[derive(Debug)]
pub enum State {
    InPosition(Position),
    Seeking(Quotation),
    Sleeping(Instant, Duration)
}

pub struct Bot {
    state: Option<State>,

    order_id: (u32, String), // for generating `order_id`
    money: Quotation,

    order_client: OrdersServiceClient<InterceptedService<Channel, DefaultInterceptor>>,
    market_client: MarketDataServiceClient<InterceptedService<Channel, DefaultInterceptor>>,
    operation_client: OperationsServiceClient<InterceptedService<Channel, DefaultInterceptor>>,

    settings: Settings,
    strategy: Box<dyn Strategy>,

    tg_bot: Arc<teloxide::prelude::Bot>,
}

struct Client {
    order: OrdersServiceClient<InterceptedService<Channel, DefaultInterceptor>>,
    market: MarketDataServiceClient<InterceptedService<Channel, DefaultInterceptor>>,
    operation: OperationsServiceClient<InterceptedService<Channel, DefaultInterceptor>>,
}

impl<'a> Bot {
    const ADD_INFO_PATH: &'a str = ".\\add_info\\";

    pub fn new(channel: Channel,
               inter: DefaultInterceptor,
               money: Quotation,
               strategy: Box<dyn Strategy>,
               tg_bot: teloxide::prelude::Bot) -> Bot {
        let time_crutch = || {
            let mut set = strategy.get_settings();
            for (fh, fm, th, tm)
            in (&mut set.trading_time).into_iter() {
                if *th < *fh {
                    *th += 24;
                }
                *fh = fh.checked_sub(1).unwrap_or(23);
                *fm = fm.checked_sub(1).unwrap_or(59);
                *th = th.checked_sub(1).unwrap_or(23);
                *tm = tm.checked_sub(1).unwrap_or(59);
            }
            set
        };
        Bot {
            state: None,
            order_id: Bot::get_order_id(), // 1_0000 for Debug, 1_000000 for Release
            money,
            order_client: OrdersServiceClient::with_interceptor(
                channel.clone(), inter.clone()),
            market_client: MarketDataServiceClient::with_interceptor(
                channel.clone(), inter.clone()),
            operation_client: OperationsServiceClient::with_interceptor(
                channel.clone(), inter.clone()),
            settings: time_crutch(),
            strategy,
            tg_bot: Arc::new(tg_bot),
        }
    }

    pub fn update_tg_bot_ref(&mut self, tg_bot: teloxide::prelude::Bot) {
        self.tg_bot = Arc::new(tg_bot);
    }

    fn get_order_id() -> (u32, String) {
        use std::str::FromStr;
        let contents = std::fs::read_to_string(Self::ADD_INFO_PATH.to_owned() + "oid.txt")
            .expect("Didn't find file oid.txt");
        (u32::from_str(contents.as_str()).unwrap(), contents)
    }
    fn update_order_id(order_id: &String) {
        std::fs::write(Self::ADD_INFO_PATH.to_owned() + "oid.txt", &order_id)
            .unwrap_or_else(|err| {
                panic!("Can't write `orders_id` to `oid.txt`. Current id: {}\n Error: {}",
                       &order_id, err);
            });
    }

    async fn get_money(&mut self)
        -> Result<Quotation, Box<dyn std::error::Error>> {
        let req = PositionsRequest {
            account_id: self.settings.account_id.clone(),
        };
        let response = self.operation_client.get_positions(req).await?;
        let money = &response.get_ref().money;
        for val in money {
            if val.currency.eq("rub") {
                return Ok(Quotation { units: val.units, nano: val.nano });
            }
        }
        panic!("Can't take RUB money!")
    }

    fn get_sleep_time(&self) -> Duration {
        let (h, m) = {
            let datetime: DateTime<Utc> = SystemTime::now().into();
            let moscow_datetime = datetime.with_timezone(&Moscow);
            (moscow_datetime.hour(), moscow_datetime.minute())
        };

        for &(fh, fm, ..) in &self.settings.trading_time {
            if fh > h || fh == h && fm > m {
                return Duration::new((fh - h) as u64 * 3600 + (fm - m) as u64 * 60, 1);
            }
        }
        let (fh, fm, ..) = self.settings.trading_time[0];
        let w = (fh + 23 - h) as u64 * 3600 + (fm + 59 - m) as u64 * 60;
        Duration::new(w, 1)
    }

    async fn get_from_tg(&self, rx: &mut Receiver<RequestType>) {
        while let Ok(val) = rx.try_recv() {
            match val {
                RequestType::StateRequest => {
                    let mut ans = if let None = &self.state {
                        send_message(self.tg_bot.clone(), "Состояние неизвестно".to_string()).await;
                        continue;
                    } else {
                        String::new()
                    };
                    match self.state.as_ref().unwrap() {
                        State::InPosition(pos) => {
                            ans.push_str(match pos.state {
                                PosState::WaitClose => {
                                    format!(
                                        "Открытые позиции:\
                                        \nвход {} лотов по цене {} RUB\
                                        \nожидаем выход по цене {} RUB",
                                        pos.lots, pos.price_in, pos.price_out)
                                },
                                PosState::PartialClose => {
                                    format!(
                                        "Открытые позиции:\
                                        \nвход {} лотов по цене {} RUB\
                                        \nожидаем выход по цене {} RUB, частично закрыто\
                                        (пока не могу узнать сколько точно :))",
                                        pos.lots, pos.price_in, pos.price_out)
                                },
                                PosState::WaitOpen => {
                                    format!(
                                        "Ожидаем открытия позиции:\
                                        \nвход {} лотов по цене {} RUB",
                                        pos.lots, pos.price_in)
                                },
                                PosState::PartialOpen => {
                                    format!(
                                        "Открытые позиции:\
                                        \nвход {} лотов (из x лотов, хз сколько :)) по цене {} RUB",
                                        pos.lots, pos.price_in)
                                },
                                PosState::Hold => {
                                    format!(
                                        "Открытые позиции:\
                                        \nвход {} лотов по цене {} RUB\
                                        \nзаявка на закрытие ещё не выставлена",
                                        pos.lots, pos.price_in)
                                },
                            }.as_str());
                        },
                        State::Seeking(money) => {
                            ans.push_str(
                                format!(
                                    "Ищем точку входа, портфель: {} RUB",
                                    money).as_str());
                        },
                        State::Sleeping(i, d) => {
                            ans.push_str(
                                format!(
                                    "Спим {} минут, уже проспали {} минут",
                                    d.as_secs() / 60,
                                    Instant::now().duration_since(*i).as_secs() / 60).as_str());
                        }
                    }
                    send_message(self.tg_bot.clone(), ans).await
                },
                RequestType::StatRequest => {
                    let file = std::fs::read_to_string(
                        Self::ADD_INFO_PATH.to_owned() + "today.json").unwrap();
                    let stat: DayStat = serde_json::from_str(file.as_str()).unwrap();
                    send_message(self.tg_bot.clone(), stat.to_string()).await
                },
                RequestType::StopRequest => {
                    if let State::InPosition(pos) = &self.state.as_ref().unwrap() {
                        send_message(self.tg_bot.clone(),
                                     format!("Есть открытые позиции: {} лотов по {} RUB",
                                             pos.lots, pos.price_in)).await;
                    }
                    // по хорошему дать бы ещё диалог, если есть открытые позиции
                    send_message(self.tg_bot.clone(), "Закрываемся".to_string()).await;
                    std::process::exit(0);
                },
                RequestType::Something => {
                    send_message(self.tg_bot.clone(), "WTF?!".to_string()).await
                }
            }
        }
    }

    pub async fn handler(mut self, mut rx: Receiver<RequestType>)
        -> Result<(), Box<dyn std::error::Error>>  {
        const SLEEP_INTERVAL_TIME: Duration = Duration::new(60, 0);
        const PAUSE_TIME: Duration = Duration::new(0, 500_000_000);
        const LIMIT: Duration = Duration::new(60, 1);

        let req_ob = GetOrderBookRequest {
            figi: self.settings.figi.clone(),
            depth: 10,
            instrument_id: self.settings.uid.clone(),
        };
        let req_c = {
            if let AnalysisType::Candle(interval) = self.settings.data_type {
                let now = {
                    let now: DateTime<Utc> = SystemTime::now().into();
                    now.with_timezone(&Moscow)
                };

                let from: SystemTime = Utc.with_ymd_and_hms(
                    now.year(), now.month(), now.day(),
                    self.settings.trading_time[0].0 + 1, 0, 0).unwrap().into();
                let to: SystemTime = Utc.with_ymd_and_hms(
                    now.year(), now.month(), now.day(),
                    self.settings.trading_time.last().unwrap().0 + 1, 0, 0).unwrap().into();
                Some(GetCandlesRequest {
                    figi: self.settings.figi.clone(),
                    interval: interval.into(),
                    from: Some(Timestamp::from(from)),
                    to: Some(Timestamp::from(to)),
                    instrument_id: self.settings.uid.clone(),
                })
            } else {
                None
            }
        };

        let mut time = Instant::now();
        self.state = Some(State::Seeking(self.get_money().await?));
        send_message(self.tg_bot.clone(), "Мы начали!".to_string()).await;

        loop {
            self.get_from_tg(&mut rx).await;
            let result = match self.settings.data_type {
                AnalysisType::OrderBook(depth) => {
                    let response = {
                        let response = self.market_client.get_order_book(req_ob.clone()).await;
                        if let Err(_) = &response {
                            println!("So fast or network error!");
                            std::thread::sleep(LIMIT - time.elapsed());
                            time = Instant::now();
                            continue;
                        }
                        response.unwrap()
                    };
                    if response.get_ref().orderbook_ts.is_none() {
                        if let Some(State::Seeking(..) | State::InPosition(..)) = self.state.as_ref() {
                            let d = self.get_sleep_time();
                            self.state = Some(State::Sleeping(Instant::now(), d.clone()));
                            send_message(self.tg_bot.clone(),
                                         format!("Идём спать на {} минут!", d.as_secs() / 60)).await;
                        }
                        None
                    } else {
                        Some(self.strategy.analyze_ob(response.get_ref(),
                                                      self.state.as_ref().unwrap()))
                    }
                },
                AnalysisType::Candle(_) => {
                    let response = {
                        let response = self.market_client.get_candles(
                            req_c.as_ref().unwrap().clone()).await;
                        if let Err(_) = &response {
                            println!("So fast or network error!");
                            std::thread::sleep(LIMIT - time.elapsed());
                            time = Instant::now();
                            continue;
                        }
                        response.unwrap()
                    };
                    if response.get_ref().candles.is_empty() {
                        if let Some(State::Seeking(..) | State::InPosition(..)) = self.state.as_ref() {
                            let d = self.get_sleep_time();
                            self.state = Some(State::Sleeping(Instant::now(), d.clone()));
                            send_message(self.tg_bot.clone(),
                                         format!("Идём спать на {} минут!", d.as_secs() / 60)).await;
                        }
                        None
                    } else {
                        Some(self.strategy.analyze_c(response.get_ref(),
                                                self.state.as_ref().unwrap()))
                    }
                },
            };

            self.state = Some(match self.state.take().unwrap() {
                state @ State::Seeking(..) => {
                    match result.unwrap() {
                        Action::Open(p, l, d)
                        | Action::Close(p, l, d) => {
                            let order_id = self.place_order(l, p.clone(), d).await?;
                            State::InPosition(Position {
                                state: PosState::WaitOpen,
                                price_in: p,
                                lots: l,
                                direction: d,
                                price_out: Quotation::default(),
                                id: (order_id, String::new())
                            })
                        },
                        Action::Hold => state
                    }
                },
                State::InPosition(pos) => {
                    let state = self.update_position_state(pos).await?;
                    //let result = self.strategy.analyze_ob(response.get_ref(), &state);
                    // Вопрос пока
                    // match result {
                    //     Action::Open(p, l, d)
                    //     | Action::Close(p, l, d) => {
                    //         let order_id = self.place_order(l, p.clone(), d).await?;
                    //         State::InPosition(Position {
                    //             state: PosState::WaitOpen,
                    //             price_in: p,
                    //             lots: l,
                    //             direction: d,
                    //             price_out: Quotation::default(),
                    //             id: order_id
                    //         })
                    //     },
                    //     Action::Hold => state
                    // }
                    state
                },
                State::Sleeping(i, d) => {
                    if i.elapsed() < d {
                        std::thread::sleep(SLEEP_INTERVAL_TIME);
                        State::Sleeping(i, d)
                    } else {
                        send_message(self.tg_bot.clone(), "Проснись и пой!".to_string()).await;
                        State::Seeking(self.money.clone())
                    }
                }
            });

            std::thread::sleep(PAUSE_TIME);
        }
    }

    async fn place_order(&mut self, lots: i64, price: Quotation, direction: OrderDirection)
        -> Result<String, Box<dyn std::error::Error>> {
        self.order_id = {
            let next = self.order_id.0 + 1;
            let next = (next, next.to_string());
            Self::update_order_id(&next.1);
            next
        };
        let req = PostOrderRequest {
            figi: self.settings.figi.clone(),
            quantity: lots,
            price: Some(price),
            direction: i32::from(direction),
            account_id: self.settings.account_id.clone(),
            order_type: OrderType::Limit.into(),
            order_id: self.order_id.1.clone(),
            instrument_id: self.settings.uid.clone(),
        };
        return match self.order_client.post_order(req).await {
            Ok(response) => Ok(response.get_ref().order_id.clone()),
            Err(err) => Err(Box::new(err))
        };
    }

    async fn get_order_time(&mut self, order_id: String)
        -> Result<String, Box<dyn std::error::Error>> {
        let req = GetOrderStateRequest {
            account_id: self.settings.account_id.clone(),
            order_id
        };
        let response = self.order_client.get_order_state(req).await?;

        if let Some(time) = response.get_ref().order_date.clone() {
            let dt = {
                let dt: DateTime<Utc> = Utc.timestamp_nanos(time.nanos as i64);
                dt.with_timezone(&Moscow)
            };
            Ok(dt.format("%H:%M").to_string())
        } else {
            Ok("time undetermined".to_string())
        }
    }

    async fn write_stat(&mut self,
                  Position {
                      price_in: p_in,
                      price_out: p_out,
                      lots: l,
                      direction: d,
                      id: (open_id, close_id),
                      ..}: Position) {
        let mut file = std::fs::read_to_string(
            Self::ADD_INFO_PATH.to_owned() + "today.json").unwrap();
        let mut stat: DayStat = serde_json::from_str(file.as_str()).unwrap();

        let (profit, turnover) = {
            let p = if d == OrderDirection::Buy {
                p_out.clone() - p_in.clone()
            } else {
                p_in.clone() - p_out.clone()
            };
            let p = Quotation { units: l, nano: 0 } * p;
            let p_after_fees = p.clone() * self.settings.fee_rate.clone();
            let p_after_tax = p_after_fees.clone() * self.settings.tax_rate.clone();
            let t = (p_in.clone() + p_out.clone()) * Quotation { units: l, nano: 0 };
            (
                ProfitStat { net: p, after_fees: p_after_fees, after_tax: p_after_tax },
                t.units as u32
            )
        };
        let trade = TradeStat {
            price_in: (p_in.units, p_in.nano),
            price_out: (p_out.units, p_out.nano),
            direction: d == OrderDirection::Buy,
            profit: profit.clone(),
            turnover,
            time_in: self.get_order_time(open_id).await.unwrap(),
            time_out: self.get_order_time(close_id).await.unwrap(),
        };
        stat.trades_count += 1;
        stat.trades.push(trade);
        stat.turnover += turnover;
        stat.profit = ProfitStat {
            net: stat.profit.net + profit.net,
            after_fees: stat.profit.after_fees + profit.after_fees,
            after_tax: stat.profit.after_tax + profit.after_tax,
        };
        file.write_str(serde_json::to_string(&stat).unwrap().as_str()).unwrap();
    }

    async fn update_position_state(&mut self, mut pos: Position)
        -> Result<State, Box<dyn std::error::Error>> {
        let order_id = {
            if pos.state == PosState::WaitClose || pos.state == PosState::PartialClose {
                pos.id.1.clone()
            } else {
                pos.id.0.clone()
            }
        };
        let req = GetOrderStateRequest {
            account_id: self.settings.account_id.clone(),
            order_id,
        };
        let response = self.order_client.get_order_state(req.clone()).await?;
        let response = response.get_ref();

        return match response.execution_report_status {
            1 => {
                let price = {
                    let mv = response.average_position_price.as_ref().unwrap();
                    Quotation { units: mv.units, nano: mv.nano }
                };
                if pos.state == PosState::WaitClose || pos.state == PosState::PartialClose {
                    self.write_stat(pos).await;
                    Ok(State::Seeking(self.get_money().await?))
                } else {
                    pos.state = PosState::Hold;
                    let (p, l, d) =
                        self.strategy.get_take_profit(price, response.lots_executed);
                    pos.id.1 = self.place_order(l, p.clone(), d).await?;
                    pos.state = PosState::WaitClose;
                    pos.price_out = p;
                    Ok(State::InPosition(pos))
                }
            }, // Исполнена
            5 => {
                pos.state = PosState::PartialOpen;
                pos.price_in = {
                    let mv = response.average_position_price.as_ref().unwrap();
                    Quotation { units: mv.units, nano: mv.nano }
                };
                pos.lots = response.lots_executed;
                // Попозже добавлю, что бот будет закрывать ещё не полностью исполненные
                //  позиции, и будет изменять заявку закрытия по мере исполнения открытия
                Ok(State::InPosition(pos))
            }, // Частично исполнена
            4 => {
                Ok(State::InPosition(pos))
            }, // Новая (Только размещена и 0 исполнено)
            2 | 3 | _ => {
                self.solve_problems().await?;
                Ok(State::Seeking(self.money.clone()))
            } // Отклонена | Отменена пользователем (?) | none
        }
    }

    async fn cancel_order(&mut self, order_id: String) -> Result<(), Box<dyn std::error::Error>> {
        if let Err(err) = self.order_client.cancel_order(
            CancelOrderRequest {
                order_id,
                account_id: self.settings.account_id.clone()
            }).await {
            send_message(self.tg_bot.clone(), format!("Can't cancel order: {}", err)).await;
            panic!("Can't cancel order: {}", err)
        }
        Ok(())
    }

    async fn portfolio_control(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let req = PositionsRequest {
            account_id: self.settings.account_id.clone(),
        };
        let positions = {
            let response = self.operation_client.get_positions(req).await?;
            let pos = &response.get_ref().securities;
            pos[0].balance + pos[0].blocked
        };

        return match self.state.as_ref().unwrap() {
            State::InPosition(pos) => Ok(pos.lots == positions),
            State::Seeking(..) | State::Sleeping(_, _)
            => Ok(positions == 0),
        }
    }
    async fn orders_control(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let req = GetOrdersRequest {
            account_id: self.settings.account_id.clone()
        };
        let response = self.order_client.get_orders(req).await?;
        let orders = &response.get_ref().orders;
        match self.state.as_ref().unwrap() {
            State::Sleeping(..) | State::Seeking(_) => {
                if orders.is_empty() {
                    Ok(true)
                } else {
                    for order in orders {
                        self.cancel_order(order.order_id.clone()).await?;
                    }
                    Ok(false)
                }
            },
            State::InPosition(pos) => {
                match (&pos.state, orders.is_empty()) {
                    (PosState::Hold, false) | (_, true) => {
                        for order in orders {
                            self.cancel_order(order.order_id.clone()).await?;
                        }
                        Ok(false)
                    },
                    _ => Ok(true)

                }
            }
        }
    }

    async fn solve_problems (&mut self) -> Result<(), Box<dyn std::error::Error>> {
        match (self.portfolio_control().await, self.orders_control().await) {
            (Ok(false), Ok(false)) => {
                // Надо чёт с портфелем делать
                send_message(self.tg_bot.clone(),
                             "Unexpected orders and portfolio state".to_string()).await;
                panic!("Unexpected order and portfolio state")
            },
            (Ok(true), Ok(false)) => {
                send_message(self.tg_bot.clone(),
                             "Unexpected orders".to_string()).await;
                panic!("Unexpected orders")
            },
            (Ok(false), Ok(true)) => {
                send_message(self.tg_bot.clone(),
                             "Unexpected portfolio state".to_string()).await;
                panic!("Unexpected portfolio state")
                // Тип продать что-то или хз
            }
            (Err(er), Ok(_)) | (Ok(_), Err(er))
            => return Err(er),
            _ =>  {
                send_message(self.tg_bot.clone(),
                             "Unexpected in `fn solve_problems`".to_string()).await;
                panic!("Unexpected in `fn solve_problems`");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_write() {
        let x = Bot::get_order_id();
        assert_eq!(x.1, x.0.to_string());
        Bot::update_order_id(&(x.0 + 1).to_string());
        let y = Bot::get_order_id();
        assert_eq!(y.1, y.0.to_string());
        assert_eq!(x.0 + 1, y.0);
        Bot::update_order_id(&(x.0).to_string());
    }
    #[test]
    fn sub_quotation() {
        let x = Quotation { units: 4, nano: 22_0000000 };
        let y = Quotation { units: 2, nano: 12_0000000 };
        assert_eq!(Quotation { units: 2, nano: 10_0000000 }, x - y);
        let x = Quotation { units: 3, nano: 22_0000000 };
        let y = Quotation { units: 5, nano: 22_0000000 };
        assert_eq!(Quotation { units: -2, nano: 00_0000000 }, x - y);
        let x = Quotation { units: 4, nano: 22_0000000 };
        let y = Quotation { units: 3, nano: 33_0000000 };
        assert_eq!(Quotation { units: 0, nano: 89_0000000 }, x - y);
    }
    #[test]
    fn sum_quotation() {
        let x = Quotation { units: 3, nano: 22_0000000 };
        let y = Quotation { units: 2, nano: 12_0000000 };
        assert_eq!(Quotation { units: 5, nano: 34_0000000 }, x + y);
        let x = Quotation { units: 3, nano: 82_0000000 };
        let y = Quotation { units: 5, nano: 43_0000000 };
        assert_eq!(Quotation { units: 9, nano: 25_0000000 }, x + y);
        let x = Quotation { units: 4, nano: 22_0000000 };
        let y = Quotation { units: 3, nano: 78_0000000 };
        assert_eq!(Quotation { units: 8, nano: 00_0000000 }, x + y);
    }
    #[test]
    fn mul_quotation() {
        let x = Quotation { units: 3, nano: 02_0000000 };
        let y = Quotation { units: 2, nano: 01_0000000 };
        assert_eq!(Quotation { units: 6, nano: 07_0200000 }, x * y);
        let x = Quotation { units: 3, nano: 20_0000000 };
        let y = Quotation { units: 2, nano: 01_0000000 };
        assert_eq!(Quotation { units: 6, nano: 43_2000000 }, x * y);
        let x = Quotation { units: 3, nano: 22_0000000 };
        let y = Quotation { units: 2, nano: 12_0000000 };
        assert_eq!(Quotation { units: 6, nano: 82_6400000 }, x * y);
        let x = Quotation { units: 12, nano: 22_0000000 };
        let y = Quotation { units: 66, nano: 78_0000000 };
        assert_eq!(Quotation { units: 816, nano: 05_1600000 }, x * y);
        let x = Quotation { units: 999999, nano: 99_9999999 };
        let y = Quotation { units: 999999, nano: 99_9999999 };
        x * y;
        let x = Quotation { units: 20, nano: 14_2112222 };
        let y = Quotation { units: 0, nano: 14_0412543 };
        assert_eq!(Quotation { units: 2, nano: 82_8205198 }, x * y);
    }
    #[test]
    fn ser_deser_test() {
        let x = Quotation { units: 2, nano: 10_0000000 };
        let ss = serde_json::to_string(&x).unwrap();
        let y: Quotation = serde_json::from_str(ss.as_str()).unwrap();
        assert_eq!(x, y);
    }
}