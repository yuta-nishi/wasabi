/// The type byte that starts every descriptor (USB 2.0, 9.4).
pub const DEVICE: u8 = 1;
pub const CONFIGURATION: u8 = 2;
pub const STRING: u8 = 3;
pub const INTERFACE: u8 = 4;
pub const ENDPOINT: u8 = 5;

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
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

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LENGTH {
            return None;
        }
        Some(Self {
            usb_version: u16_at(bytes, 2),
            class: bytes[4],
            subclass: bytes[5],
            protocol: bytes[6],
            max_packet_size: bytes[7],
            vendor_id: u16_at(bytes, 8),
            product_id: u16_at(bytes, 10),
            device_version: u16_at(bytes, 12),
            number_of_configurations: bytes[17],
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

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LENGTH {
            return None;
        }
        Some(Self {
            total_length: u16_at(bytes, 2),
            number_of_interfaces: bytes[4],
            configuration_value: bytes[5],
            attributes: bytes[7],
            max_power: bytes[8],
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

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LENGTH {
            return None;
        }
        Some(Self {
            interface_number: bytes[2],
            alternate_setting: bytes[3],
            number_of_endpoints: bytes[4],
            class: bytes[5],
            subclass: bytes[6],
            protocol: bytes[7],
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

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LENGTH {
            return None;
        }
        Some(Self {
            address: bytes[2],
            attributes: bytes[3],
            max_packet_size: u16_at(bytes, 4),
            interval: bytes[6],
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
