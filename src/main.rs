#![windows_subsystem = "windows"]

use yttt::host_launcher::{ProcessRole, process_role, run_host_process};

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    match process_role(args.iter()) {
        ProcessRole::Desktop => {
            yttt::ui::app::run(yttt::config::profile::AppProfile::production());
        }
        ProcessRole::Host => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("yttt-host")
                .build()
                .expect("failed to initialize Host runtime");
            if let Err(error) = runtime.block_on(run_host_process(args)) {
                eprintln!("yttt Host failed: {error}");
                std::process::exit(1);
            }
        }
    }
}
