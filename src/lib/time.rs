use chrono::{DateTime, Duration, NaiveDateTime, Utc};
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

impl From<i64> for Time {
    fn from(timestamp: i64) -> Self {
        let naive = NaiveDateTime::from_timestamp(timestamp, 0);
        
        Self(DateTime::from_utc(naive, Utc))
    }
}

pub fn ms_to_time(ms: f64) -> Time {
    (Utc::now() + Duration::milliseconds(ms.ceil() as i64)).into()
}