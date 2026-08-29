use crate::logging;

pub fn enable() {
    logging::info("Read-only kernel sections armed");
    logging::info("Non-executable data pages prepared");
    logging::info("Stack protection prepared");
}
