use gelf::{Logger, Message, Level};
use chrono::Utc;
use std::io::Write;

pub trait Loggable {
    fn to_log_message(&self) -> String;
}

#[cfg(feature="graylog")]
pub fn log(level: Level, message: &str, full_message: &str, metadata: Vec<(&str, String)>, logger: &Option<Logger>) {
    if let Some(log) = logger {
        let mut message = Message::new(String::from(message));
        message.set_full_message(String::from(full_message));
        message.set_level(level);
    
        for (key, value) in metadata {
            message.set_metadata(String::from(key), value).ok();
        }
    
        log.log_message(message);

        return;
    }
    print_log(level, full_message);
}

#[cfg(not(feature="graylog"))]
pub fn log(level: Level, message: &str, full_message: &str, metadata: Vec<(&str, String)>, logger: &Option<Logger>) {
    print_log(level, full_message);
}

fn print_log(level: Level, full_message: &str) {
    let writer = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open("logs/current.log");

    let date = Utc::now().to_string();

    match writer {
        Ok(mut file) => { write!(file, "[{}] app.{}: {}\n", date, level.to_rust().to_string().to_uppercase(), full_message); },
        Err(_err) => { println!("[{}] app.{}: {}", date, level.to_rust().to_string().to_uppercase(), full_message); }
    };
}