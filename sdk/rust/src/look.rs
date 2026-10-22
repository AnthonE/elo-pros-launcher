//! What a call answered: an answer, a refusal, or "could not look".
//!
//! The three are different claims and this module never merges them. A
//! reader that answers "nothing" when it could not look has invented a fact
//! about a player: a ticket read that times out is not "does not own", and an
//! explorer that is down is not an empty inventory.

use serde::de::DeserializeOwned;
use serde_json::Value;

/// One call's outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Look<T> {
    /// The server answered, and this is the answer.
    Answer(T),
    /// The server said no, in its own words (a 4xx with `{"error": …}`), or
    /// this SDK refused before asking (`status` 0) because the server would.
    Refused(Refusal),
    /// Nobody could tell: unreachable, timed out, a 5xx, a body that is not
    /// JSON or not the expected shape, or the server itself saying it could
    /// not read (`reachable: false`). Ask again later; never read it as "no".
    CouldNotLook(String),
}

/// A refusal: the HTTP status, the server's sentence, and its whole body.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    /// The HTTP status, or 0 when this SDK refused before sending anything.
    pub status: u16,
    /// What the server said (`error`), in its own plain words.
    pub error: String,
    /// The whole refusal body, `Null` when there was none.
    pub body: Value,
}

impl<T> Look<T> {
    /// `"answer"`, `"refused"` or `"could_not_look"` — the same word the
    /// Python and JavaScript SDKs use.
    pub fn kind(&self) -> &'static str {
        match self {
            Look::Answer(_) => "answer",
            Look::Refused(_) => "refused",
            Look::CouldNotLook(_) => "could_not_look",
        }
    }

    /// The answer, or None for a refusal or could-not-look.
    pub fn answer(self) -> Option<T> {
        match self {
            Look::Answer(t) => Some(t),
            _ => None,
        }
    }

    /// The answer by reference.
    pub fn as_answer(&self) -> Option<&T> {
        match self {
            Look::Answer(t) => Some(t),
            _ => None,
        }
    }

    pub fn is_answer(&self) -> bool {
        matches!(self, Look::Answer(_))
    }

    pub fn is_refused(&self) -> bool {
        matches!(self, Look::Refused(_))
    }

    pub fn is_could_not_look(&self) -> bool {
        matches!(self, Look::CouldNotLook(_))
    }

    /// Change the answer, keeping a refusal or could-not-look as it is.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Look<U> {
        match self {
            Look::Answer(t) => Look::Answer(f(t)),
            Look::Refused(r) => Look::Refused(r),
            Look::CouldNotLook(w) => Look::CouldNotLook(w),
        }
    }

    /// Chain a step that can itself refuse or fail to look.
    pub fn and_then<U>(self, f: impl FnOnce(T) -> Look<U>) -> Look<U> {
        match self {
            Look::Answer(t) => f(t),
            Look::Refused(r) => Look::Refused(r),
            Look::CouldNotLook(w) => Look::CouldNotLook(w),
        }
    }
}

impl Refusal {
    /// A refusal this SDK makes before sending anything (`status` 0).
    pub fn here(error: impl Into<String>) -> Refusal {
        Refusal {
            status: 0,
            error: error.into(),
            body: Value::Null,
        }
    }
}

fn said(body: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| body.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

/// Sort one HTTP response into a [`Look`] — the rule every call shares, and
/// the same rule the Python and JavaScript SDKs apply:
///
/// 1. a body that is not JSON → could not look;
/// 2. a 5xx → could not look (the server's `why` or `error` when it gave one);
/// 3. any other non-2xx → refused, with the server's `error` (or `detail`);
/// 4. a 2xx whose body says `reachable: false` → could not look, with its
///    `why` or `error`: the server answered that it could not read;
/// 5. a 2xx that is not the expected shape → could not look;
/// 6. otherwise the answer.
pub fn classify<T: DeserializeOwned>(status: u16, body: &str) -> Look<T> {
    let value: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return Look::CouldNotLook(format!(
                "the server answered {status} with a body that is not JSON"
            ))
        }
    };
    if status >= 500 {
        let why = said(&value, &["why", "error"])
            .unwrap_or_else(|| format!("the server answered {status}"));
        return Look::CouldNotLook(why);
    }
    if !(200..300).contains(&status) {
        let error = said(&value, &["error", "why", "detail"])
            .unwrap_or_else(|| format!("the server answered {status}"));
        return Look::Refused(Refusal {
            status,
            error,
            body: value,
        });
    }
    if value.get("reachable") == Some(&Value::Bool(false)) {
        let why = said(&value, &["why", "error"])
            .unwrap_or_else(|| "the server could not look".to_string());
        return Look::CouldNotLook(why);
    }
    match serde_json::from_value(value) {
        Ok(t) => Look::Answer(t),
        Err(e) => Look::CouldNotLook(format!("the answer is not in a shape this SDK reads: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_outcomes_stay_apart() {
        let a: Look<Value> = classify(200, r#"{"ok":true}"#);
        assert!(a.is_answer());
        let r: Look<Value> = classify(404, r#"{"error":"no ask from that wallet"}"#);
        assert_eq!(
            r,
            Look::Refused(Refusal {
                status: 404,
                error: "no ask from that wallet".into(),
                body: serde_json::json!({"error": "no ask from that wallet"}),
            })
        );
        let c: Look<Value> = classify(503, r#"{"entitled":null,"why":"the chain read failed"}"#);
        assert_eq!(c, Look::CouldNotLook("the chain read failed".into()));
        let h: Look<Value> = classify(502, "<html>bad gateway</html>");
        assert!(h.is_could_not_look());
        let u: Look<Value> = classify(200, r#"{"reachable":false,"error":"timeout"}"#);
        assert_eq!(u, Look::CouldNotLook("timeout".into()));
    }

    #[test]
    fn a_shape_it_cannot_read_is_not_an_answer() {
        let x: Look<u64> = classify(200, r#"{"not":"a number"}"#);
        assert!(x.is_could_not_look());
        assert_eq!(x.kind(), "could_not_look");
    }
}
