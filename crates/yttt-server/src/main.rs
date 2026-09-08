use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match yttt_server::run_from_os_args(std::env::args_os()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("yttt-server: {error}");
            ExitCode::FAILURE
        }
    }
}
