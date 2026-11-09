//! SheetShrink owns its application entry point and shares the existing execution engine.
fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    if std::env::args_os().len() == 1
        || std::env::args_os().any(|argument| argument == "--web")
        || std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("observatory"))
    {
        return web();
    }
    excel_shrink_cli::run_sheetshrink()
}

#[cfg(windows)]
fn web() -> anyhow::Result<()> {
    use clap::Parser;
    use std::path::PathBuf;
    #[derive(Parser)]
    #[command(
        name = "sheetshrink",
        about = "SheetShrink configuration and detailed work Observatory"
    )]
    struct Options {
        #[arg(long)]
        web: bool,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long, num_args = 1..)]
        input: Vec<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 2)]
        workers: usize,
        #[arg(long, default_value_t = 2)]
        max_active_workbooks: usize,
        #[arg(long, default_value_t = 16 * 1024 * 1024)]
        parallel_range_chunk_size: usize,
        #[arg(long)]
        reserve_mib: Option<u64>,
        #[arg(long)]
        recursive: bool,
    }
    let skip = if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("observatory"))
    {
        2
    } else {
        1
    };
    let arguments = std::env::args_os()
        .take(1)
        .chain(std::env::args_os().skip(skip));
    let options = Options::parse_from(arguments);
    preparation_web::serve_sheetshrink(
        preparation_web::Defaults {
            inputs: options.input,
            output: options.output,
            workers: options.workers,
            recursive: options.recursive,
            reserve_mib: options.reserve_mib,
            source_manifest: None,
            reuse_from: None,
            max_active_workbooks: Some(options.max_active_workbooks),
            parallel_range_chunk_size: Some(options.parallel_range_chunk_size),
        },
        options.port,
        |request, context| {
            excel_shrink_cli::observed_cleanup::run(
                excel_shrink_cli::observed_cleanup::Options {
                    inputs: request.inputs,
                    output: request
                        .output
                        .ok_or_else(|| anyhow::anyhow!("Output missing"))?,
                    recursive: request.recursive,
                    threads: request.workers,
                    max_active_workbooks: request.max_active_workbooks.unwrap_or(2),
                    parallel_range_chunk_size: request
                        .parallel_range_chunk_size
                        .unwrap_or(16 * 1024 * 1024),
                },
                context,
            )
        },
    )
}
