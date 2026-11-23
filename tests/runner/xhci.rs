use wasabi::xhci::CompletionCode;
use wasabi::xhci::Portsc;
use wasabi::xhci::Speed;

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
