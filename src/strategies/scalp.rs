use crate::bot::{Action, Strategy, State, Settings, AnalysisType};
use crate::tcs::{GetOrderBookResponse, Quotation, OrderDirection};

enum Signal {
    BuyAsk,
    BuyBid,
    Hold,
}

pub struct Scalp {
    c_bid_q_vec: Vec<i64>,
    c_bid_q_avg: i64,
    c_ask_q_vec: Vec<i64>,
    c_ask_q_avg: i64,
}

impl Scalp {
    pub fn new() -> Scalp {
        Scalp {
            c_bid_q_vec: Vec::with_capacity(40),
            c_bid_q_avg: 0,
            c_ask_q_vec: Vec::with_capacity(40),
            c_ask_q_avg: 0,
        }
    }

    fn compute_price(&Quotation { units: u, nano: n}: &Quotation, shift: i32) -> Quotation {
        match n + 01_0000000 * shift {
            100_0000000 => Quotation { units: u + 1, nano: 0 },
            -01_0000000 => Quotation { units: u - 1, nano: 99_0000000 },
            n => Quotation { units: u, nano: n },
        }
    }
    fn compute_lots_quantity(
        &Quotation { units: p_m, nano: q_m }: &Quotation,
        &Quotation { units: p_p, nano: q_p }: &Quotation) -> i64 {
        let nanos_p = p_p * 100 + q_p as i64 / 1_0000000;
        let nanos_m = p_m * 100 + q_m as i64 / 1_0000000;
        return nanos_m / nanos_p
    }
}

impl Strategy for Scalp {
    fn analyze_ob(&mut self, orders: &GetOrderBookResponse, state: &State) -> Action {
        use Signal::*;

        let ratio = |ask: i64, bid: i64| -> Signal {
            match ask as f64 / bid as f64 {
                val if val > 3.0 => BuyBid,
                val if val < 0.35 => BuyAsk,
                _ => Hold,
            }
        };
        let order_book = |s: &mut Scalp, ask: i64, bid: i64| -> Signal {
            let ask_a = s.c_ask_q_avg / ask > 3;
            s.c_ask_q_vec.push(ask);
            s.c_ask_q_avg = s.c_ask_q_vec.iter().sum::<i64>() / s.c_ask_q_vec.len() as i64;
            let bid_a = s.c_bid_q_avg / bid > 3;
            s.c_bid_q_vec.push(bid);
            s.c_bid_q_avg = s.c_bid_q_vec.iter().sum::<i64>() / s.c_bid_q_vec.len() as i64;
            match (ask_a, bid_a) {
                (true, false) => BuyAsk,
                (false, true) => BuyBid,
                _ => Hold,
            }
        };

        let bids = &orders.bids;
        let asks = &orders.asks;

        let ask_q = asks[0].quantity;
        let bid_q = bids[0].quantity;
        return match (order_book(self, ask_q, bid_q), ratio(ask_q, bid_q)) {
            (BuyBid, BuyBid) => {
                if let State::InPosition(pos) = state {
                    Action::Close(pos.price_in.clone(), pos.lots, {
                        if pos.direction == OrderDirection::Buy
                        { OrderDirection::Sell } else { OrderDirection::Buy }
                    })
                } else {
                    Action::Hold
                }
            },
            (BuyAsk, BuyAsk) => {
                if let State::Seeking(mv) = state {
                    let price = Scalp::compute_price(asks[0].price.as_ref().unwrap(), 0);
                    let lots = Scalp::compute_lots_quantity(&price, &mv);
                    Action::Open(price, lots, OrderDirection::Sell)
                } else {
                    Action::Hold
                }
            },
            _ => Action::Hold
        }
    }
    fn get_take_profit(&mut self, p: Quotation, l: i64) -> (Quotation, i64, OrderDirection) {
        (Scalp::compute_price(&p, 1), l, OrderDirection::Sell)
    }
    fn get_settings(&self) -> Settings {
        Settings {
            account_id: String::from("2157068285"),
            ticker: String::from("TRUR"),
            uid: String::from("e2d0dbac-d354-4c36-a5ed-e5aae42ffc76"),
            figi: String::from("BBG000000001"),
            class_code: String::from("TQTF"),
            trading_time: vec![
                (10, 0, 18, 40)
            ],
            data_type: AnalysisType::OrderBook(10),
            fee_rate: Quotation { units: 0, nano: 0 },
            tax_rate: Quotation { units: 0, nano: 13_0000000 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_quantity() {
        assert_eq!(
            Scalp::compute_lots_quantity(&Quotation { units: 50, nano: 00_0000000 }, &Quotation { units: 5, nano: 90_0000000 }),
            8
        );
    }
    #[test]
    fn correct_price() {
        assert_eq!(
            Scalp::compute_price(&Quotation { units: 5, nano: 90_0000000 }, 1),
            Quotation { units: 5, nano: 91_0000000 }
        );
        assert_eq!(
            Scalp::compute_price(&Quotation { units: 5, nano: 90_0000000 }, -1),
            Quotation { units: 5, nano: 89_0000000 }
        );
        assert_eq!(
            Scalp::compute_price(&Quotation { units: 5, nano: 99_0000000 }, 1),
            Quotation { units: 6, nano: 00_0000000 }
        );
        assert_eq!(
            Scalp::compute_price(&Quotation { units: 5, nano: 00_0000000 }, -1),
            Quotation { units: 4, nano: 99_0000000 }
        );
    }
}
