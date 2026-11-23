use wasabi::pci::Command;

pub fn command_fields_are_named() {
    let mut command = Command::new(0);
    command.set_memory_space(1);
    command.set_bus_master(1);
    assert_eq!(command.raw(), 0b110);
    assert_eq!(command.memory_space(), 1);
    assert_eq!(command.bus_master(), 1);
    assert_eq!(command.io_space(), 0);
}
