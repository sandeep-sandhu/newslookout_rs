
# Newslookout

[![build](https://github.com/sandeep-sandhu/newslookout_rs/actions/workflows/rust.yml/badge.svg)](https://github.com/sandeep-sandhu/newslookout_rs/actions)


A web scraping platform built for news scanning, powered by Rust. Port of the application of the same name built in python.


## Quick Start
Add this to your Cargo.toml:
[dependencies]
newslookout = "0.1.0"

## Usage

This package is intended to build a full-fledged multi-threaded web scraping solution that runs in batch mode with very little code.

Get started with just a few lines of code, for example:

```
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
```

## Configuration
The entire application is driven by its config file.

There are a few pre-built modules provided for a few websites. These can be readily extended for other websites as required. Refer to the source code of these in the plugins folder.
