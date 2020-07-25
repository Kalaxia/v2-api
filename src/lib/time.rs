use chrono::{DateTime, Utc};

#[derive(Clone, Copy, Debug, serde::Serialize, sqlx::Type)]
#[serde(into = "i64")]
#[sqlx(transparent)]
pub struct Time(pub DateTime<Utc>);

impl Time {
    pub fn now() -> Self { Self(Utc::now()) }
}

impl From<DateTime<Utc>> for Time {
    fn from(time:DateTime<Utc>) -> Self { Self(time) }
}

impl Into<DateTime<Utc>> for Time {
    fn into(self) -> DateTime<Utc> { self.0 }
}

impl Into<i64> for Time {
    fn into(self) -> i64 { self.0.timestamp_millis() }
}