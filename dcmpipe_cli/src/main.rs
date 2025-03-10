use std::process;

use clap::Parser;

use crate::app::archiveapp::ArchiveApp;
use crate::app::browseapp::BrowseApp;
#[cfg(feature = "index")]
use crate::app::indexapp::IndexApp;
use crate::app::printapp::PrintApp;
use crate::app::scanapp::ScanApp;
use crate::app::CommandApplication;
use crate::args::{Arguments, Command};

mod app;
mod args;

fn main() {
    let mut app: Box<dyn CommandApplication> = make_app();
    if let Err(e) = app.run() {
        eprintln!("Error: {:?}", e);
        process::exit(1);
    }
}

fn make_app() -> Box<dyn CommandApplication> {
    let args: Arguments = Arguments::parse();

    match args.command {
        Command::Print(args) => Box::new(PrintApp::new(args)),
        Command::Browse(args) => Box::new(BrowseApp::new(args)),
        Command::Scan(args) => Box::new(ScanApp::new(args)),
        #[cfg(feature = "index")]
        Command::Index(args) => Box::new(IndexApp::new(args)),
        Command::Archive(args) => Box::new(ArchiveApp::new(args)),
    }
}
