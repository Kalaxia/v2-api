use gelf::{Logger, Message, Level};

pub fn log(_level: Level, message: &str, full_message: &str, metadata: Vec<(&str, String)>, logger: &Logger) {
    // Create a (complex) message
    let mut message = Message::new(String::from(message));
    message.set_full_message(String::from(full_message));

    for (key, value) in metadata {
        message.set_metadata(String::from(key), value).ok();
    }

    logger.log_message(message);
}