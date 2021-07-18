use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, sqlx::Type, PartialEq)]
#[serde(into = "i64")]
#[sqlx(transparent)]
pub struct Time(pub DateTime<Utc>);

impl Time {
    pub fn now() -> Self { Self(Utc::now()) }
}

impl From<DateTime<Utc>> for Time {
    fn from(time:DateTime<Utc>) -> Self { Self(time) }
}

impl From<Time> for DateTime<Utc> {
    fn from(time: Time) -> Self { time.0 }
}

impl From<Time> for i64 {
    fn from(time: Time) -> i64 { time.0.timestamp_millis() }
}

pub fn ms_to_time(ms: f64) -> Time {
    (Utc::now() + Duration::milliseconds(ms.ceil() as i64)).into()
}