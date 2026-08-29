use blackwall_shared::serial as uart;

pub fn init() {
    uart::init();
}

pub fn write_str(text: &str) {
    uart::write_str(text);
}

pub fn line(text: &str) {
    uart::write_line(text);
}

pub fn read_byte() -> Option<u8> {
    uart::read_byte()
}
