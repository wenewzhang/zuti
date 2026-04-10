use std::env;

use zuti::disk::get_pool_devices;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <poolname>", args[0]);
        std::process::exit(1);
    }

    let poolname = &args[1];

    match get_pool_devices(poolname) {
        Ok(devices) => {
            for device in devices {
                println!("{}", device.name);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
