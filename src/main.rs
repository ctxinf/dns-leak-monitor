mod capture;
mod config;
mod parse;
mod store;
mod web;

use arc_swap::ArcSwap;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;

fn main() {
    let cfg_path = config_path_from_args();
    let cfg = match config::Config::load(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fatal: {e}");
            std::process::exit(1);
        }
    };

    let exclude = Arc::new(ArcSwap::from_pointee(cfg.normalized_exclude()));
    eprintln!(
        "config loaded: interface={} db={} bind={} retention={}d exclude={} entries",
        cfg.interface,
        cfg.db_path,
        cfg.bind,
        cfg.retention_days,
        exclude.load().len()
    );

    // Writer owns the primary connection; open it before capture so schema
    // exists by the time the web thread opens read-only.
    let conn = match store::open(&cfg.db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fatal: {e}");
            std::process::exit(1);
        }
    };

    let (tx, rx) = mpsc::channel::<capture::Record>();

    std::thread::spawn(move || store::writer(conn, rx));

    {
        let db_path = cfg.db_path.clone();
        let days = cfg.retention_days;
        std::thread::spawn(move || store::pruner(db_path, days));
    }

    {
        let bind = cfg.bind.clone();
        let db_path = cfg.db_path.clone();
        std::thread::spawn(move || {
            if let Err(e) = web::run(&bind, &db_path) {
                eprintln!("fatal: {e}");
                std::process::exit(1);
            }
        });
    }

    {
        let interface = cfg.interface.clone();
        let exclude = exclude.clone();
        std::thread::spawn(move || {
            if let Err(e) = capture::run(&interface, exclude, tx) {
                eprintln!("fatal: {e}");
                std::process::exit(1); // systemd Restart=on-failure takes over
            }
        });
    }

    // Main thread handles signals. SIGHUP hot-swaps the exclude list;
    // interface/bind/db_path changes require a restart.
    let mut signals = Signals::new([SIGHUP, SIGTERM, SIGINT]).expect("signal setup");
    for sig in signals.forever() {
        match sig {
            SIGHUP => match config::Config::load(&cfg_path) {
                Ok(new_cfg) => {
                    let list = new_cfg.normalized_exclude();
                    eprintln!("SIGHUP: exclude list reloaded ({} entries)", list.len());
                    if new_cfg.interface != cfg.interface
                        || new_cfg.bind != cfg.bind
                        || new_cfg.db_path != cfg.db_path
                    {
                        eprintln!("SIGHUP: interface/bind/db_path changed — restart required, ignoring");
                    }
                    exclude.store(Arc::new(list));
                }
                Err(e) => eprintln!("SIGHUP: reload failed, keeping old exclude list: {e}"),
            },
            _ => {
                eprintln!("shutting down");
                std::process::exit(0);
            }
        }
    }
}

fn config_path_from_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--config" || a == "-c" {
            if let Some(p) = args.next() {
                return PathBuf::from(p);
            }
        }
    }
    PathBuf::from("/etc/dns-leak-monitor/config.toml")
}
