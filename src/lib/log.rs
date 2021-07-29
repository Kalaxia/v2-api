use gelf::{Logger, Message, Level};

#[cfg(feature="graylog")]
pub fn log(level: Level, message: &str, full_message: &str, metadata: Vec<(&str, String)>, logger: &Logger) {
    let mut message = Message::new(String::from(message));
    message.set_full_message(String::from(full_message));
    message.set_level(level);

    for (key, value) in metadata {
        message.set_metadata(String::from(key), value).ok();
    }

    logger.log_message(message);
}

#[cfg(not(feature="graylog"))]
pub fn log(level: Level, message: &str, full_message: &str, metadata: Vec<(&str, String)>, logger: &Logger) {
    println!("app.{}: {}", level.to_rust().to_string().to_uppercase(), full_message);
}