use wasabi::usb::descriptor::ConfigurationDescriptor;
use wasabi::usb::descriptor::DeviceDescriptor;
use wasabi::usb::descriptor::EndpointDescriptor;
use wasabi::usb::descriptor::InterfaceDescriptor;
use wasabi::usb::descriptor::descriptors;
use wasabi::usb::hid::key_for;
use wasabi::usb::hid::keys;

/// The QEMU USB keyboard's device descriptor, as the controller returned it.
const KEYBOARD_DEVICE: &[u8] = &[
    0x12, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x40, 0x27, 0x06, 0x01, 0x00, 0x00, 0x01,
    0x01, 0x02, 0x00, 0x01,
];

/// Its configuration: header, interface, HID descriptor and one endpoint.
const KEYBOARD_CONFIGURATION: &[u8] = &[
    0x09, 0x02, 0x22, 0x00, 0x01, 0x01, 0x08, 0xa0, 0x32, // configuration
    0x09, 0x04, 0x00, 0x00, 0x01, 0x03, 0x01, 0x01, 0x00, // interface
    0x09, 0x21, 0x11, 0x01, 0x00, 0x01, 0x22, 0x3f, 0x00, // HID
    0x07, 0x05, 0x81, 0x03, 0x08, 0x00, 0x07, // endpoint
];

pub fn the_device_descriptor_says_which_keyboard_it_is() {
    let descriptor = DeviceDescriptor::from_bytes(KEYBOARD_DEVICE)
        .expect("the descriptor should parse");
    assert_eq!(descriptor.usb_version, 0x0200);
    assert_eq!(descriptor.class, 0x00);
    assert_eq!(descriptor.max_packet_size, 64);
    assert_eq!(descriptor.vendor_id, 0x0627);
    assert_eq!(descriptor.product_id, 0x0001);
    assert_eq!(descriptor.number_of_configurations, 1);
}

pub fn a_truncated_descriptor_is_rejected() {
    assert!(DeviceDescriptor::from_bytes(&KEYBOARD_DEVICE[..10]).is_none());
    assert!(ConfigurationDescriptor::from_bytes(&KEYBOARD_CONFIGURATION[..5]).is_none());
    assert!(InterfaceDescriptor::from_bytes(&KEYBOARD_CONFIGURATION[9..12]).is_none());
    assert!(EndpointDescriptor::from_bytes(&KEYBOARD_CONFIGURATION[27..30]).is_none());
}

pub fn the_configuration_holds_the_interface_and_the_endpoint() {
    let configuration = ConfigurationDescriptor::from_bytes(KEYBOARD_CONFIGURATION)
        .expect("the configuration should parse");
    assert_eq!(configuration.total_length, 34);
    assert_eq!(configuration.number_of_interfaces, 1);
    assert_eq!(configuration.configuration_value, 1);
    assert_eq!(configuration.max_power, 0x32);

    let mut interface = None;
    let mut endpoint = None;
    for raw in descriptors(KEYBOARD_CONFIGURATION) {
        match raw.kind {
            4 => interface = InterfaceDescriptor::from_bytes(raw.bytes),
            5 => endpoint = EndpointDescriptor::from_bytes(raw.bytes),
            _ => {}
        }
    }

    let interface = interface.expect("an interface descriptor");
    assert_eq!(interface.interface_number, 0);
    assert_eq!(interface.class, 0x03); // HID
    assert_eq!(interface.subclass, 0x01); // boot interface
    assert_eq!(interface.protocol, 0x01); // keyboard
    assert_eq!(interface.number_of_endpoints, 1);

    let endpoint = endpoint.expect("an endpoint descriptor");
    assert_eq!(endpoint.address, 0x81);
    assert!(endpoint.direction_in());
    assert_eq!(endpoint.number(), 1);
    assert_eq!(endpoint.transfer_type(), 3); // interrupt
    assert_eq!(endpoint.max_packet_size, 8);
    assert_eq!(endpoint.interval, 7);
}

pub fn a_broken_length_ends_the_walk() {
    // The first descriptor claims thirteen bytes but only nine follow.
    let broken = &[0x0d, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(descriptors(broken).count(), 0);
}

pub fn usage_codes_become_characters() {
    assert_eq!(key_for(0x04, false), Some('a'));
    assert_eq!(key_for(0x04, true), Some('A'));
    assert_eq!(key_for(0x1e, false), Some('1'));
    assert_eq!(key_for(0x1e, true), Some('!'));
    assert_eq!(key_for(0x2c, false), Some(' '));
    assert_eq!(key_for(0x28, false), Some('\n'));
    assert_eq!(key_for(0x00, false), None);
    assert_eq!(key_for(0x68, false), None);
}

pub fn a_report_carries_the_keys_that_are_held() {
    // Shift held, then the letter b, then nothing else.
    let report = [0x02, 0x00, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00];
    let decoded = keys(&report);
    assert_eq!(decoded[0], Some('B'));
    assert!(decoded[1..].iter().all(Option::is_none));

    // A short report has nothing to say.
    assert!(keys(&[0x00, 0x00]).iter().all(Option::is_none));
}
