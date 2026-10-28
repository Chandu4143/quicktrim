//! QuickTrim entry point.
//!
//! Usage:
//!   quicktrim [VIDEO]
//!   quicktrim --help
//!
//! With no argument the window opens empty and the user picks a file. With a
//! path the file starts loading immediately, which also makes "Open with"
//! registrations and drag-onto-icon work without extra code.

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use std::path::PathBuf;

use quicktrim::app::App;
use quicktrim::MainWindow;
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    let mut video: Option<PathBuf> = None;

    for argument in std::env::args_os().skip(1) {
        let text = argument.to_string_lossy();
        match text.as_ref() {
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            "--version" | "-V" => {
                println!("quicktrim {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other if other.starts_with('-') => {
                eprintln!("quicktrim: unknown option “{other}”\n");
                print_help();
                std::process::exit(2);
            }
            _ if video.is_none() => video = Some(PathBuf::from(argument)),
            _ => {
                eprintln!("quicktrim: opening one video at a time");
            }
        }
    }

    let ui = MainWindow::new()?;
    let app = App::new(ui.as_weak());
    app.start(&ui);

    if let Some(path) = video {
        app.open(path);
    }

    ui.run()
}

fn print_help() {
    println!(
        "QuickTrim {version}\n\
         \n\
         Trim a video without touching the original.\n\
         \n\
         Usage:\n  \
         quicktrim [VIDEO]\n\
         \n\
         Options:\n  \
         -h, --help       Show this help\n  \
         -V, --version    Show the version\n\
         \n\
         Shortcuts:\n  \
         Space            Play or pause\n  \
         Left / Right     Skip 5 seconds (Shift: 1 second)\n  \
         I / O            Set the start / end from the playhead\n  \
         Ctrl+S           Save a copy\n  \
         M                Mute\n",
        version = env!("CARGO_PKG_VERSION")
    );
}
