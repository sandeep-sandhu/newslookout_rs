// file: bin.rs
// Purpose: Application using the newslookout package
// Description: Starts the app.

extern crate newslookout; // not needed since Rust edition 2018

use std::env;
use newslookout::run_app;

fn main() {
    if env::args().len() < 2 {
        println!("Usage: newslookout_app <config_file>");
        panic!("Provide config file as parameter in the command line, (need 2 parameters, got {})",
               env::args().len()
        );
    }

    let configfile = env::args().nth(1).unwrap();

    run_app(configfile);
}

