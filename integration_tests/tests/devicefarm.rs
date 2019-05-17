#![cfg(feature = "devicefarm")]

extern crate rusoto_core;
extern crate rusoto_devicefarm;

use rusoto_core::Region;
use rusoto_devicefarm::{DeviceFarm, DeviceFarmClient, ListDevicesRequest};

#[test]
pub fn should_list_devices() {
    let client = DeviceFarmClient::new(Region::UsWest2);
    let request = ListDevicesRequest::default();

    client.list_devices(request).sync().unwrap();
}