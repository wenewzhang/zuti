use std::env;

use zuti::disk::find_disk_by_id;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <device>", args[0]);
        eprintln!("Example: {} sdb", args[0]);
        std::process::exit(1);
    }

    let device = &args[1];

    match find_disk_by_id(device) {
        Ok(long_id) => {
            println!("{}", long_id);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
