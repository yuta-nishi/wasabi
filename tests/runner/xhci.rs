use wasabi::pci::Command;
use wasabi::xhci::CompletionCode;
use wasabi::xhci::EndpointContext1;
use wasabi::xhci::Portsc;
use wasabi::xhci::SlotContext0;
use wasabi::xhci::SlotContext1;
use wasabi::xhci::Speed;
use wasabi::xhci::TrbType;

pub fn portsc_fields_come_out_of_the_word() {
    // A device that has just been plugged in: attached, link still training.
    let portsc = Portsc::new(0x0002_0ee1);
    assert_eq!(portsc.connected(), 1);
    assert_eq!(portsc.enabled(), 0);
    assert_eq!(portsc.speed(), 3);
    assert_eq!(portsc.link_state(), 7);
    assert_eq!(portsc.connect_changed(), 1);
}

pub fn a_reset_port_reads_as_enabled() {
    let portsc = Portsc::new(0x0000_0e03);
    assert_eq!(portsc.connected(), 1);
    assert_eq!(portsc.enabled(), 1);
    assert_eq!(portsc.speed(), 3);
    assert_eq!(portsc.link_state(), 0);
    assert_eq!(portsc.reset_changed(), 0);
}

pub fn speed_maps_to_the_slot_context_value() {
    assert_eq!(Speed::from_portsc(3), Some(Speed::High));
    assert_eq!(Speed::High.psi(), 3);
    assert_eq!(Speed::High.max_packet_size(), 64);
    assert_eq!(Speed::Full.max_packet_size(), 8);
    assert_eq!(Speed::Super.max_packet_size(), 512);
    assert_eq!(Speed::from_portsc(0), None);
}

pub fn completion_codes_carry_names() {
    assert_eq!(CompletionCode::from_raw(1), CompletionCode::Success);
    assert_eq!(CompletionCode::from_raw(5), CompletionCode::TrbError);
    assert_eq!(CompletionCode::from_raw(99), CompletionCode::Other(99));
}

pub fn trb_types_have_names() {
    assert_eq!(TrbType::from_raw(33), TrbType::CommandCompletion);
    assert_eq!(TrbType::from_raw(34), TrbType::PortStatusChange);
    assert_eq!(TrbType::from_raw(23), TrbType::NoOpCommand);
    assert_eq!(TrbType::from_raw(63), TrbType::Other(63));
}

pub fn context_words_use_the_specification_positions() {
    let mut slot0 = SlotContext0::new(0);
    slot0.set_speed(Speed::High.psi());
    slot0.set_context_entries(1);
    // Speed is bits 23..20, context entries are bits 31..27.
    assert_eq!(slot0.raw(), (3 << 20) | (1 << 27));

    let mut slot1 = SlotContext1::new(0);
    slot1.set_root_hub_port(5);
    // The root hub port is bits 23..16.
    assert_eq!(slot1.raw(), 5 << 16);

    let mut endpoint1 = EndpointContext1::new(0);
    endpoint1.set_max_packet(64);
    endpoint1.set_ep_type(4);
    endpoint1.set_cerr(3);
    // CErr is bits 2..1, the type bits 5..3, the packet size bits 31..16.
    assert_eq!(endpoint1.raw(), (64 << 16) | (4 << 3) | (3 << 1));
}

pub fn pci_command_fields_are_named() {
    let mut command = Command::new(0);
    command.set_memory_space(1);
    command.set_bus_master(1);
    assert_eq!(command.raw(), 0b110);
    assert_eq!(command.memory_space(), 1);
    assert_eq!(command.bus_master(), 1);
    assert_eq!(command.io_space(), 0);
}
