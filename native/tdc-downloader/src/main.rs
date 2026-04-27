mod protocol;
mod runner;
mod state;

use protocol::Capabilities;
use runner::Runner;
use std::env;

fn capabilities() -> Capabilities {
    Capabilities {
        protocol_version: 1,
        transport_ready: true,
        resume_ready: true,
        commands: vec![
            "start_run",
            "pause",
            "resume",
            "stop",
            "refresh_file_reference",
            "refresh_dc_auth",
        ],
        events: vec![
            "run_started",
            "file_started",
            "file_progress",
            "file_completed",
            "file_skipped",
            "file_restarted",
            "transport_pipeline",
            "file_error",
            "transport_window",
            "transport_stall",
            "request_file_reference_refresh",
            "request_dc_auth_refresh",
            "run_summary",
            "fatal_error",
        ],
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let subcommand = args.get(1).map(String::as_str).unwrap_or("");

    match subcommand {
        "capabilities" => {
            println!(
                "{}",
                serde_json::to_string(&capabilities()).expect("serialize capabilities")
            );
        }
        "run" => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build tokio runtime");
            runtime.block_on(async {
                let mut runner = Runner::stdio();
                if let Err(error) = runner.run().await {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            });
        }
        _ => {
            eprintln!("usage: tdc-downloader <capabilities|run>");
            std::process::exit(2);
        }
    }
}
