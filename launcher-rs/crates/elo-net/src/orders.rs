//! The rules every order follows — the same as the site's
//! `watchtower/js/orders-core.js`, and held to the same cases
//! (`tests/order_vectors.json`), so the two can never disagree about when an
//! order fires.
//!
//! Prices are ETH per coin. [`step`] is pure: an order, the price now and the
//! time now (seconds) in; `None`, `Fire { side, amount }` or `Done` out, with
//! the order's new running state (a trailing high, a DCA's next slot, the parts
//! done). An order is finished when its state says so.
//!
//! limit · take-profit · stop · trailing-stop · trailing-buy · bracket · dca ·
//! twap — what each does is written once, in `orders-core.js`'s header.

use serde_json::{json, Value};

pub const KINDS: [&str; 8] = ["limit", "take-profit", "stop", "trailing-stop", "trailing-buy", "bracket", "dca", "twap"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct State {
    pub created: Option<f64>,
    pub peak: Option<f64>,
    pub low: Option<f64>,
    pub next: Option<f64>,
    pub done: u32,
    pub skipped: u32,
    pub finished: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub id: u64,
    pub kind: String,
    pub coin: String,
    pub side: Option<String>,
    pub amount: String,
    pub price: Option<f64>,
    pub pct: Option<f64>,
    pub tp: Option<f64>,
    pub stop: Option<f64>,
    pub every_s: Option<f64>,
    pub times: Option<u32>,
    pub over_s: Option<f64>,
    pub parts: Option<u32>,
    pub cap: Option<f64>,
    pub state: State,
    pub paused: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Fire { side: &'static str, amount: f64 },
    Done,
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
}

impl Order {
    pub fn from_json(v: &Value) -> Option<Order> {
        let st = v.get("state").cloned().unwrap_or(Value::Null);
        Some(Order {
            id: v.get("id").and_then(Value::as_u64).unwrap_or(0),
            kind: v.get("kind")?.as_str()?.to_string(),
            coin: v.get("coin")?.as_str()?.to_ascii_uppercase(),
            side: v.get("side").and_then(Value::as_str).map(str::to_string),
            amount: match v.get("amount")? { Value::String(s) => s.clone(), other => other.to_string() },
            price: f(v, "price"), pct: f(v, "pct"), tp: f(v, "tp"), stop: f(v, "stop"),
            every_s: f(v, "every_s"), times: f(v, "times").map(|x| x as u32),
            over_s: f(v, "over_s"), parts: f(v, "parts").map(|x| x as u32), cap: f(v, "cap"),
            state: State {
                created: f(&st, "created"), peak: f(&st, "peak"), low: f(&st, "low"), next: f(&st, "next"),
                done: f(&st, "done").unwrap_or(0.0) as u32, skipped: f(&st, "skipped").unwrap_or(0.0) as u32,
                finished: st.get("finished").and_then(Value::as_bool).unwrap_or(false),
            },
            paused: v.get("paused").and_then(Value::as_str).map(str::to_string),
        })
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({"id": self.id, "kind": self.kind, "coin": self.coin, "amount": self.amount});
        let o = v.as_object_mut().expect("an object");
        let mut put = |k: &str, x: Option<f64>| { if let Some(x) = x { o.insert(k.into(), json!(x)); } };
        put("price", self.price); put("pct", self.pct); put("tp", self.tp); put("stop", self.stop);
        put("every_s", self.every_s); put("over_s", self.over_s); put("cap", self.cap);
        put("times", self.times.map(f64::from)); put("parts", self.parts.map(f64::from));
        if let Some(s) = &self.side { o.insert("side".into(), json!(s)); }
        if let Some(p) = &self.paused { o.insert("paused".into(), json!(p)); }
        let s = &self.state;
        let mut st = json!({"done": s.done, "skipped": s.skipped, "finished": s.finished});
        let so = st.as_object_mut().expect("an object");
        for (k, x) in [("created", s.created), ("peak", s.peak), ("low", s.low), ("next", s.next)] {
            if let Some(x) = x { so.insert(k.into(), json!(x)); }
        }
        o.insert("state".into(), st);
        v
    }

    /// Which way it trades.
    pub fn side(&self) -> &'static str {
        match self.kind.as_str() {
            "limit" | "trailing-buy" => "buy",
            "take-profit" | "stop" | "trailing-stop" | "bracket" => "sell",
            _ => if self.side.as_deref() == Some("buy") { "buy" } else { "sell" },
        }
    }

    /// Whole, or the sentence that says why not.
    pub fn validate(&self) -> Result<(), String> {
        if !KINDS.contains(&self.kind.as_str()) {
            return Err(format!("an order is {}", KINDS.join(", ")));
        }
        if self.coin.is_empty() || self.coin == "ETH" {
            return Err("name the coin the order trades against ETH".into());
        }
        if !(self.amount.parse::<f64>().map(|a| a > 0.0).unwrap_or(false)) {
            return Err("the amount is above zero".into());
        }
        let pos = |x: Option<f64>| x.is_some_and(|v| v > 0.0 && v.is_finite());
        match self.kind.as_str() {
            "limit" | "take-profit" | "stop" if !pos(self.price) => return Err("the price is above zero".into()),
            "trailing-stop" | "trailing-buy" if !self.pct.is_some_and(|p| p > 0.0 && p < 100.0) =>
                return Err("the trail is a percent between 0 and 100".into()),
            "bracket" if !(pos(self.tp) && pos(self.stop) && self.tp > self.stop) =>
                return Err("a bracket's take-profit is above its stop".into()),
            "dca" if !(pos(self.every_s) && self.times.unwrap_or(0) >= 1) =>
                return Err("a DCA needs an interval and a number of times".into()),
            "twap" if !(pos(self.over_s) && self.parts.unwrap_or(0) >= 2) =>
                return Err("a TWAP needs a duration and at least two parts".into()),
            _ => {}
        }
        if matches!(self.kind.as_str(), "dca" | "twap") && !matches!(self.side.as_deref(), Some("buy") | Some("sell")) {
            return Err("a DCA or TWAP buys or sells".into());
        }
        if self.cap.is_some() && !pos(self.cap) {
            return Err("the cap is above zero".into());
        }
        Ok(())
    }

    fn cap_ok(&self, price: f64) -> bool {
        match self.cap {
            None => true,
            Some(c) => if self.side.as_deref() == Some("buy") { price <= c } else { price >= c },
        }
    }

    /// One line for a list.
    pub fn describe(&self) -> String {
        let (c, a) = (&self.coin, &self.amount);
        let p = |x: Option<f64>| x.map(fmt_price).unwrap_or_default();
        let what = |side: &str| if side == "buy" { format!("{a} ETH of {c}") } else { format!("{a} {c}") };
        match self.kind.as_str() {
            "limit" => format!("buy {a} ETH of {c} at {} or under", p(self.price)),
            "take-profit" => format!("sell {a} {c} at {} or over", p(self.price)),
            "stop" => format!("sell {a} {c} at {} or under", p(self.price)),
            "trailing-stop" => format!("sell {a} {c} if it falls {}% from its high", self.pct.unwrap_or(0.0)),
            "trailing-buy" => format!("buy {a} ETH of {c} if it rises {}% from its low", self.pct.unwrap_or(0.0)),
            "bracket" => format!("sell {a} {c} at {} or over, or {} or under", p(self.tp), p(self.stop)),
            "dca" => format!("{} {} every {}, {} times", self.side(), what(self.side()), dur(self.every_s.unwrap_or(0.0)),
                             self.times.unwrap_or(0)),
            "twap" => format!("{} {} over {} in {} parts", self.side(), what(self.side()), dur(self.over_s.unwrap_or(0.0)),
                              self.parts.unwrap_or(0)),
            k => k.to_string(),
        }
    }
}

/// A price for a terminal: plain above 1, scientific below.
pub fn fmt_price(p: f64) -> String {
    if p == 0.0 || !p.is_finite() {
        return "—".into();
    }
    if p >= 1.0 { format!("{p:.4}") } else { format!("{p:.3e}") }
}

/// A duration a person reads, as the site writes it.
pub fn dur(s: f64) -> String {
    let short = |x: f64| { let t = format!("{x:.2}"); t.trim_end_matches('0').trim_end_matches('.').to_string() };
    if s >= 86400.0 { format!("{} {}", short(s / 86400.0), if s == 86400.0 { "day" } else { "days" }) }
    else if s >= 3600.0 { format!("{} h", short(s / 3600.0)) }
    else if s >= 60.0 { format!("{} min", short(s / 60.0)) }
    else { format!("{s} s") }
}

/// `30m`, `4h`, `1d`, `90s` (or bare seconds) as seconds.
pub fn parse_dur(s: &str) -> Result<f64, String> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let n: f64 = n.trim().parse().map_err(|_| format!("{s} is not a duration like 30m, 4h or 1d"))?;
    let mul = match unit { "" | "s" => 1.0, "m" | "min" => 60.0, "h" => 3600.0, "d" => 86400.0,
                           _ => return Err(format!("{s} is not a duration like 30m, 4h or 1d")) };
    if !(n > 0.0) {
        return Err("a duration is above zero".into());
    }
    Ok(n * mul)
}

/// The order a form describes, the way the window's Orders reads it. `a` is the
/// price (limit, take-profit, stop), the trail % (trailing) or the take-profit
/// (bracket); `b` the bracket's stop; `c` the DCA's interval or the TWAP's span
/// (`4h`); `d` the DCA's count or the TWAP's parts; `cap` optional.
#[allow(clippy::too_many_arguments)]
pub fn from_fields(id: u64, kind: &str, coin: &str, side: &str, amount: &str, a: &str, b: &str, c: &str,
                   d: &str, cap: &str) -> Result<Order, String> {
    let num = |s: &str, what: &str| -> Result<Option<f64>, String> {
        let t = s.trim().trim_end_matches('%');
        if t.is_empty() { Ok(None) } else { t.parse().map(Some).map_err(|_| format!("{what} is a number, not {s}")) }
    };
    let mut o = Order { id, kind: kind.to_string(), coin: coin.trim().to_ascii_uppercase(), side: None,
                        amount: amount.trim().to_string(), price: None, pct: None, tp: None, stop: None,
                        every_s: None, times: None, over_s: None, parts: None, cap: num(cap, "the cap")?,
                        state: State::default(), paused: None };
    match kind {
        "limit" | "take-profit" | "stop" => o.price = num(a, "the price")?,
        "trailing-stop" | "trailing-buy" => o.pct = num(a, "the trail")?,
        "bracket" => { o.tp = num(a, "the take-profit")?; o.stop = num(b, "the stop")?; }
        "dca" | "twap" => {
            o.side = Some(side.to_string());
            let span = if c.trim().is_empty() { None } else { Some(parse_dur(c)?) };
            let n = num(d, "the count")?.map(|x| x as u32);
            if kind == "dca" { o.every_s = span; o.times = n; } else { o.over_s = span; o.parts = n; }
        }
        _ => {}
    }
    if !matches!(kind, "dca" | "twap") {
        o.cap = None;
    }
    o.validate()?;
    Ok(o)
}

/// What the order does at `price` and `now`, and its new state.
pub fn step(o: &Order, price: f64, now: f64) -> (Action, State) {
    let mut st = o.state.clone();
    if st.finished {
        return (Action::Done, st);
    }
    if !(price > 0.0) || !price.is_finite() {
        return (Action::None, st);
    }
    if st.created.is_none() {
        st.created = Some(now);
    }
    let amount: f64 = o.amount.parse().unwrap_or(0.0);
    let side = o.side();
    let fire = |mut s: State, amt: f64| { s.finished = true; (Action::Fire { side, amount: amt }, s) };
    match o.kind.as_str() {
        "limit" | "stop" if price <= o.price.unwrap_or(0.0) => fire(st, amount),
        "take-profit" if price >= o.price.unwrap_or(f64::MAX) => fire(st, amount),
        "bracket" if price >= o.tp.unwrap_or(f64::MAX) || price <= o.stop.unwrap_or(0.0) => fire(st, amount),
        "trailing-stop" => {
            let peak = st.peak.unwrap_or(0.0).max(price);
            st.peak = Some(peak);
            if price <= peak * (1.0 - o.pct.unwrap_or(0.0) / 100.0) { fire(st, amount) } else { (Action::None, st) }
        }
        "trailing-buy" => {
            let low = st.low.map_or(price, |l| l.min(price));
            st.low = Some(low);
            if price >= low * (1.0 + o.pct.unwrap_or(0.0) / 100.0) { fire(st, amount) } else { (Action::None, st) }
        }
        "dca" | "twap" => {
            let slots = if o.kind == "dca" { o.times.unwrap_or(0) } else { o.parts.unwrap_or(0) }.max(1);
            let every = if o.kind == "dca" { o.every_s.unwrap_or(0.0) } else { o.over_s.unwrap_or(0.0) / f64::from(slots) };
            let next = st.next.or(st.created).unwrap_or(now);
            if now < next {
                return (Action::None, st);
            }
            st.done += 1;
            st.next = Some(next + every);
            st.finished = st.done >= slots;
            if !o.cap_ok(price) {
                st.skipped += 1;
                return (Action::None, st);
            }
            let per = if o.kind == "dca" { amount } else { amount / f64::from(slots) };
            (Action::Fire { side, amount: per }, st)
        }
        _ => (Action::None, st),
    }
}

/// ETH per coin from `/api/pairs`: the pair quoted in ETH, or the pair quoted
/// in another coin times that coin's own ETH price. Our chain only.
pub fn price_eth(pairs: &[Value], coin: &str) -> Option<f64> {
    let up = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_ascii_uppercase();
    let rows: Vec<&Value> = pairs.iter()
        .filter(|p| p.get("chain").and_then(Value::as_str).is_none_or(|c| c == "rh")).collect();
    let price = |p: &Value| p.get("price").and_then(Value::as_f64);
    let coin = coin.to_ascii_uppercase();
    if let Some(d) = rows.iter().find(|p| up(p, "base") == coin && up(p, "quote") == "ETH") {
        return price(d);
    }
    let via = rows.iter().find(|p| up(p, "base") == coin)?;
    let q = rows.iter().find(|p| up(p, "base") == up(via, "quote") && up(p, "quote") == "ETH")?;
    Some(price(via)? * price(q)?)
}

/// An amount as text a quote accepts: no exponent, no trailing zeros.
pub fn amount_text(x: f64) -> String {
    let t = format!("{x:.12}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors() -> Value {
        serde_json::from_str(include_str!("../tests/order_vectors.json")).expect("the cases parse")
    }

    #[test]
    fn every_shared_case_passes() {
        let v = vectors();
        for c in v["cases"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let mut o = Order::from_json(&c["order"]).unwrap();
            assert!(o.validate().is_ok(), "{name}: refused a good order: {:?}", o.validate());
            for t in c["ticks"].as_array().unwrap() {
                let (act, st) = step(&o, t["price"].as_f64().unwrap(), t["now"].as_f64().unwrap());
                o.state = st;
                let want = t["action"].as_str().unwrap();
                let got = match &act { Action::None => "none", Action::Fire { .. } => "fire", Action::Done => "done" };
                assert_eq!(got, want, "{name} at {}", t["now"]);
                if let (Action::Fire { amount, .. }, Some(w)) = (&act, t.get("amount")) {
                    let w: f64 = w.as_str().unwrap().parse().unwrap();
                    assert!((amount - w).abs() < 1e-9, "{name}: fired {amount}, want {w}");
                }
            }
        }
    }

    #[test]
    fn every_shared_refusal_is_refused() {
        for o in vectors()["invalid"].as_array().unwrap() {
            let order = Order::from_json(o);
            assert!(order.map_or(true, |x| x.validate().is_err()), "accepted {o}");
        }
    }

    #[test]
    fn eth_prices_go_through_the_reserve() {
        let v = vectors();
        let pairs = v["price_eth"]["pairs"].as_array().unwrap();
        for (k, want) in v["price_eth"]["expect"].as_object().unwrap() {
            let got = price_eth(pairs, k);
            match want.as_f64() {
                Some(w) => assert!((got.unwrap() - w).abs() < 1e-18, "{k}"),
                None => assert!(got.is_none(), "{k}: priced a coin from another chain"),
            }
        }
    }

    #[test]
    fn a_form_reads_as_the_cli_does() {
        let o = from_fields(1, "dca", "elo", "buy", "0.01", "", "", "4h", "6", "0.00000002").unwrap();
        assert_eq!((o.every_s, o.times, o.cap), (Some(14400.0), Some(6), Some(2e-8)));
        assert!(from_fields(1, "bracket", "ELO", "", "1", "1", "2", "", "", "").is_err());
        assert!(from_fields(1, "limit", "ELO", "", "1", "", "", "", "", "").is_err());
        assert_eq!(parse_dur("90").unwrap(), 90.0);
        assert!(parse_dur("soon").is_err());
    }

    #[test]
    fn an_order_round_trips_through_json() {
        let o = Order::from_json(&json!({"id": 4, "kind": "dca", "coin": "elo", "side": "buy", "amount": "0.01",
            "every_s": 3600, "times": 5, "cap": 1e-8, "state": {"done": 2, "next": 99.0}})).unwrap();
        assert_eq!(Order::from_json(&o.to_json()).unwrap(), o);
        assert_eq!(o.coin, "ELO");
        assert_eq!(o.describe(), "buy 0.01 ETH of ELO every 1 h, 5 times");
    }
}
