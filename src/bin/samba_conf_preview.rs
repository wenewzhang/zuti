use std::env;
use std::fs;

// Import the conf module
mod conf {
    include!("../utils/conf.rs");
}

fn main() {
    // Get include path from command line args or use default
    let include_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/etc/samba/conf.d/*.conf".to_string());

    let conf_path = "/etc/samba/smb.conf";

    // Read the config file
    let content = match fs::read_to_string(conf_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: Failed to read {}: {}", conf_path, e);
            std::process::exit(1);
        }
    };

    // Generate preview
    match conf::ensure_global_include_preview(&content, &include_path) {
        Ok((new_content, modified)) => {
            if modified {
                println!("# Preview of modified config (include = {} will be added):", include_path);
                println!("{}", "=".repeat(60));
                println!("{}", new_content);
                println!("{}", "=".repeat(60));
            } else {
                println!("# Config already contains 'include = {}'", include_path);
                println!("{}", "=".repeat(60));
                println!("{}", new_content);
                println!("{}", "=".repeat(60));
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
