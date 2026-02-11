use crate::driver::UartService;
use glenda::interface::DriverService;
use glenda::protocol::device::DeviceNode;

impl<'a> DriverService for UartService<'a> {
    fn init(&mut self, _node: DeviceNode) {
        unimplemented!()
    }
    fn enable(&mut self) {}
    fn disable(&mut self) {}
}
