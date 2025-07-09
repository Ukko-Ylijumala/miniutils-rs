// Copyright (c) 2025 Mikko Tanner. All rights reserved.

use miniutils::SysInfo;
use std::{thread, time::Duration};

fn main() {
    let si: SysInfo = SysInfo::new();
    eprintln!("Static system information:");
    eprintln!("{:?}", si.data);

    loop {
        si.print();
        thread::sleep(Duration::from_secs(2));
    }
}
