use gelf::{Logger, Message, Level};

pub fn log(level: Level, message: &str, full_message: &str, metadata: Vec<(&str, String)>, logger: &Logger) {
    // Create a (complex) message
    let mut message = Message::new(String::from(message));
    message.set_full_message(String::from(full_message));
    message.set_level(level);

    for (key, value) in metadata {
        message.set_metadata(String::from(key), value).ok();
    }

    logger.log_message(message);
}