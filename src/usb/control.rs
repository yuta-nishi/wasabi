/// Builds the eight bytes a setup stage carries for a standard request
/// (USB 2.0, 9.3).
///
/// `request_type` 0x80 is device-to-host, standard, aimed at the device;
/// `request` 0x06 is `GET_DESCRIPTOR`.
pub fn get_descriptor(descriptor_type: u8, index: u16, length: u16) -> u64 {
    let request_type = 0x80u64;
    let request = 0x06u64;
    let value = (u16::from(descriptor_type) << 8) | index;
    request_type | (request << 8) | (u64::from(value) << 16) | (u64::from(length) << 48)
}
