/// The type byte that starts every descriptor (USB 2.0, 9.4).
pub const DEVICE: u8 = 1;
pub const CONFIGURATION: u8 = 2;
pub const STRING: u8 = 3;
pub const INTERFACE: u8 = 4;
pub const ENDPOINT: u8 = 5;

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// One descriptor as it appears inside a configuration buffer.
pub struct Raw<'a> {
    pub kind: u8,
    pub bytes: &'a [u8],
}

/// Walks the descriptors of a configuration, each starting with its length and
/// its type (USB 2.0, 9.4).
///
/// A length that cannot be true (smaller than the two bytes it takes to describe
/// itself, or longer than what is left) ends the walk.
pub fn descriptors(mut bytes: &[u8]) -> impl Iterator<Item = Raw<'_>> {
    core::iter::from_fn(move || {
        if bytes.len() < 2 {
            return None;
        }
        let length = usize::from(bytes[0]);
        if length < 2 || length > bytes.len() {
            bytes = &[];
            return None;
        }
        let (head, rest) = bytes.split_at(length);
        bytes = rest;
        Some(Raw {
            kind: head[1],
            bytes: head,
        })
    })
}

/// What the device says it is (USB 2.0, 9.6.1).
#[derive(Clone, Copy, Default, Debug)]
pub struct DeviceDescriptor {
    pub usb_version: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub max_packet_size: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub device_version: u16,
    pub number_of_configurations: u8,
}

impl DeviceDescriptor {
    pub const LENGTH: usize = 18;
    const USB_VERSION: usize = 2;
    const CLASS: usize = 4;
    const SUBCLASS: usize = 5;
    const PROTOCOL: usize = 6;
    const MAX_PACKET_SIZE: usize = 7;
    const VENDOR_ID: usize = 8;
    const PRODUCT_ID: usize = 10;
    const DEVICE_VERSION: usize = 12;
    const NUMBER_OF_CONFIGURATIONS: usize = 17;

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LENGTH {
            return None;
        }
        Some(Self {
            usb_version: u16_at(bytes, Self::USB_VERSION),
            class: bytes[Self::CLASS],
            subclass: bytes[Self::SUBCLASS],
            protocol: bytes[Self::PROTOCOL],
            max_packet_size: bytes[Self::MAX_PACKET_SIZE],
            vendor_id: u16_at(bytes, Self::VENDOR_ID),
            product_id: u16_at(bytes, Self::PRODUCT_ID),
            device_version: u16_at(bytes, Self::DEVICE_VERSION),
            number_of_configurations: bytes[Self::NUMBER_OF_CONFIGURATIONS],
        })
    }
}

/// A configuration and everything it contains (USB 2.0, 9.6.3).
#[derive(Clone, Copy, Default, Debug)]
pub struct ConfigurationDescriptor {
    pub total_length: u16,
    pub number_of_interfaces: u8,
    pub configuration_value: u8,
    pub attributes: u8,
    pub max_power: u8,
}

impl ConfigurationDescriptor {
    pub const LENGTH: usize = 9;
    const TOTAL_LENGTH: usize = 2;
    const NUMBER_OF_INTERFACES: usize = 4;
    const CONFIGURATION_VALUE: usize = 5;
    const ATTRIBUTES: usize = 7;
    const MAX_POWER: usize = 8;

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LENGTH {
            return None;
        }
        Some(Self {
            total_length: u16_at(bytes, Self::TOTAL_LENGTH),
            number_of_interfaces: bytes[Self::NUMBER_OF_INTERFACES],
            configuration_value: bytes[Self::CONFIGURATION_VALUE],
            attributes: bytes[Self::ATTRIBUTES],
            max_power: bytes[Self::MAX_POWER],
        })
    }
}

/// One function of a configuration (USB 2.0, 9.6.5): a keyboard is a HID
/// interface, class 3.
#[derive(Clone, Copy, Default, Debug)]
pub struct InterfaceDescriptor {
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub number_of_endpoints: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

impl InterfaceDescriptor {
    pub const LENGTH: usize = 9;
    const INTERFACE_NUMBER: usize = 2;
    const ALTERNATE_SETTING: usize = 3;
    const NUMBER_OF_ENDPOINTS: usize = 4;
    const CLASS: usize = 5;
    const SUBCLASS: usize = 6;
    const PROTOCOL: usize = 7;

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LENGTH {
            return None;
        }
        Some(Self {
            interface_number: bytes[Self::INTERFACE_NUMBER],
            alternate_setting: bytes[Self::ALTERNATE_SETTING],
            number_of_endpoints: bytes[Self::NUMBER_OF_ENDPOINTS],
            class: bytes[Self::CLASS],
            subclass: bytes[Self::SUBCLASS],
            protocol: bytes[Self::PROTOCOL],
        })
    }
}

/// Where a function moves data (USB 2.0, 9.6.6).
#[derive(Clone, Copy, Default, Debug)]
pub struct EndpointDescriptor {
    pub address: u8,
    pub attributes: u8,
    pub max_packet_size: u16,
    pub interval: u8,
}

impl EndpointDescriptor {
    pub const LENGTH: usize = 7;
    const ADDRESS: usize = 2;
    const ATTRIBUTES: usize = 3;
    const MAX_PACKET_SIZE: usize = 4;
    const INTERVAL: usize = 6;

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LENGTH {
            return None;
        }
        Some(Self {
            address: bytes[Self::ADDRESS],
            attributes: bytes[Self::ATTRIBUTES],
            max_packet_size: u16_at(bytes, Self::MAX_PACKET_SIZE),
            interval: bytes[Self::INTERVAL],
        })
    }

    /// 0 control, 1 isochronous, 2 bulk, 3 interrupt.
    pub fn transfer_type(&self) -> u8 {
        self.attributes & 0b11
    }

    pub fn direction_in(&self) -> bool {
        self.address & 0x80 != 0
    }

    pub fn number(&self) -> u8 {
        self.address & 0x0f
    }
}
