use anyhow::{Context, Result};
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use data_obfuscator::cli::{self, Cli, Commands};
use data_obfuscator::obfuscate::{self, run_tokens};
use data_obfuscator::trace::TraceLog;
use data_obfuscator::ui::{self, Stream};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// An error that has already been shown (and logged) in red; `main` only adds a footer.
#[derive(Debug)]
struct Reported;

impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed (details above and in obfuscator.log)")
    }
}
impl std::error::Error for Reported {}

fn main() {
    let code = match try_main() {
        Ok(()) => 0,
        Err(e) => {
            let msg = if e.downcast_ref::<Reported>().is_some() {
                "FAILED - see the messages above and obfuscator.log".to_string()
            } else {
                format!("ERROR: {e:#}")
            };
            eprintln!("{}", ui::red(Stream::Err, &format!("[X] {msg}")));
            1
        }
    };
    std::process::exit(code);
}

fn try_main() -> Result<()> {
    // Honour --no-color / NO_COLOR even when argument parsing itself fails or prints help.
    let early_no_color = std::env::args().any(|a| a == "--no-color");
    ui::init(early_no_color);

    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            if matches!(
                e.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) {
                let stream = if e.use_stderr() { Stream::Err } else { Stream::Out };
                let banner = ui::banner(stream, &ui::local_now());
                if e.use_stderr() {
                    eprintln!("{banner}");
                } else {
                    println!("{banner}");
                }
            }
            e.exit();
        }
    };

    if cli.show_version {
        print!("{}", ui::version_text(Stream::Out));
        return Ok(());
    }

    let Some(command) = cli.command else {
        println!("{}", ui::banner(Stream::Out, &ui::local_now()));
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };

    // Banner first, before anything else is printed.
    eprintln!("{}", ui::banner(Stream::Err, &ui::local_now()));

    match command {
        Commands::Run(args) => {
            let opts = cli::resolve_run(&args)?;
            // Refuse to touch a folder that already holds files unless --overwrite was given.
            let existing = cli::check_output_dir(&opts)?;
            std::fs::create_dir_all(&opts.output_dir)
                .with_context(|| format!("creating {}", opts.output_dir.display()))?;
            let log_path = opts.output_dir.join("obfuscator.log");
            let mut log = TraceLog::open(&log_path, opts.verbose)?;
            log.info(format!(
                "argv: {}",
                std::env::args().collect::<Vec<_>>().join(" ")
            ));
            if existing > 0 {
                log.warn(format!(
                    "--overwrite: {} already holds {existing} file(s) or folder(s); files with the same names will be replaced",
                    opts.output_dir.display()
                ));
            }
            let result = catch_unwind(AssertUnwindSafe(|| obfuscate::run(&opts, &mut log)));
            match result {
                Ok(Ok(art)) => {
                    print_run_footer(&art);
                    Ok(())
                }
                Ok(Err(e)) => {
                    log.error(format!("{e:#}"));
                    Err(anyhow::Error::new(Reported))
                }
                Err(_) => {
                    log.error("run aborted by panic - see FATAL line above");
                    log.error(format!("log flushed to {}", log_path.display()));
                    Err(anyhow::Error::new(Reported))
                }
            }
        }
        Commands::Reverse(args) => {
            let opts = cli::resolve_reverse(&args)?;
            let existing = data_obfuscator::reverse::check_output_dir(&opts.output_dir, opts.overwrite)?;
            std::fs::create_dir_all(&opts.output_dir)
                .with_context(|| format!("creating {}", opts.output_dir.display()))?;
            let log_path = opts.output_dir.join("reverse.log");
            let mut log = TraceLog::open(&log_path, opts.verbose)?;
            log.info(format!(
                "argv: {}",
                std::env::args().collect::<Vec<_>>().join(" ")
            ));
            if existing > 0 {
                log.warn(format!(
                    "--overwrite: {} already holds {existing} file(s) or folder(s); files with the same names will be replaced",
                    opts.output_dir.display()
                ));
            }
            let result = catch_unwind(AssertUnwindSafe(|| data_obfuscator::reverse::reverse(&opts, &mut log)));
            match result {
                Ok(Ok(art)) => {
                    let g = |t: &str| ui::green(Stream::Err, t);
                    eprintln!();
                    eprintln!("{}", g("[OK] Reverse complete. Files written:"));
                    for f in [&art.dataset_path, &art.report_path, &art.log_path] {
                        eprintln!("      {}", g(&f.display().to_string()));
                    }
                    if art.warnings > 0 {
                        eprintln!(
                            "{}",
                            ui::orange(
                                Stream::Err,
                                &format!("[!] {} warning(s) - read them above and in reverse_report.txt: some values may not be fully restored.", art.warnings)
                            )
                        );
                    }
                    eprintln!(
                        "{}",
                        ui::orange(Stream::Err, "[!] The restored dataset holds the real, identifying data again: do not share it.")
                    );
                    Ok(())
                }
                Ok(Err(e)) => {
                    log.error(format!("{e:#}"));
                    Err(anyhow::Error::new(Reported))
                }
                Err(_) => {
                    log.error("reverse aborted by panic - see FATAL line above");
                    log.error(format!("log flushed to {}", log_path.display()));
                    Err(anyhow::Error::new(Reported))
                }
            }
        }
        Commands::Run2(args) => {
            use data_obfuscator::run2;
            let opts = run2::resolve_run2(&args)?;
            let existing = run2::runner::check_output_dir(&opts)?;
            let dir = &opts.base.output_dir;
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            let log_path = dir.join("run2.log");
            let mut log = TraceLog::open(&log_path, opts.base.verbose)?;
            log.info(format!("argv: {}", std::env::args().collect::<Vec<_>>().join(" ")));
            if existing > 0 {
                log.warn(format!(
                    "--overwrite: {} already holds {existing} file(s) or folder(s); files with the same names will be replaced",
                    dir.display()
                ));
            }
            let result = catch_unwind(AssertUnwindSafe(|| run2::run2(&opts, &mut log)));
            match result {
                Ok(Ok(art)) => {
                    let g = |t: &str| ui::green(Stream::Err, t);
                    eprintln!();
                    eprintln!("{}", g("[OK] run2 complete. Files written:"));
                    let mut files: Vec<std::path::PathBuf> = art.masked_files.clone();
                    files.push(art.llm_path.clone());
                    files.extend(art.keymap_path.clone());
                    files.extend([art.summary_path.clone(), art.config_used_path.clone(), art.log_path.clone()]);
                    for f in files {
                        eprintln!("      {}", g(&f.display().to_string()));
                    }
                    if art.warnings > 0 {
                        eprintln!(
                            "{}",
                            ui::orange(
                                Stream::Err,
                                &format!("[!] {} warning(s) - review them above before sharing the masked datasets.", art.warnings)
                            )
                        );
                    }
                    eprintln!(
                        "{}",
                        g("      Share with the LLM: the *_masked files and llm2.txt. Keep private: keymap2.txt, summary2.txt.")
                    );
                    eprintln!(
                        "{}",
                        g("      Later: data-obfuscator reverse --input master.csv --keymap keymap2.txt  (restores the real values).")
                    );
                    Ok(())
                }
                Ok(Err(e)) => {
                    log.error(format!("{e:#}"));
                    Err(anyhow::Error::new(Reported))
                }
                Err(_) => {
                    log.error("run2 aborted by panic - see FATAL line above");
                    log.error(format!("log flushed to {}", log_path.display()));
                    Err(anyhow::Error::new(Reported))
                }
            }
        }
        Commands::Tokens(args) => {
            let (opts, verbose) = cli::resolve_tokens(&args)?;
            std::fs::create_dir_all(&opts.output_dir)
                .with_context(|| format!("creating {}", opts.output_dir.display()))?;
            let log_path = opts.output_dir.join("obfuscator.log");
            let mut log = TraceLog::open(&log_path, verbose)?;
            log.info(format!(
                "argv: {}",
                std::env::args().collect::<Vec<_>>().join(" ")
            ));
            let result = catch_unwind(AssertUnwindSafe(|| {
                run_tokens(&opts.input, &opts.output_dir, &opts.exclude, &mut log)
            }));
            match result {
                Ok(Ok(path)) => {
                    log.success(format!("wrote {}", path.display()));
                    eprintln!(
                        "{}",
                        ui::green(Stream::Err, "[OK] Token estimates complete.")
                    );
                    Ok(())
                }
                Ok(Err(e)) => {
                    log.error(format!("{e:#}"));
                    Err(anyhow::Error::new(Reported))
                }
                Err(_) => {
                    log.error("tokens aborted by panic - see FATAL line above");
                    log.error(format!("log flushed to {}", log_path.display()));
                    Err(anyhow::Error::new(Reported))
                }
            }
        }
    }
}

fn print_run_footer(art: &data_obfuscator::RunArtifacts) {
    let g = |t: &str| ui::green(Stream::Err, t);
    eprintln!();
    eprintln!("{}", g("[OK] Run complete. Files written:"));
    let mut files = vec![
        art.output_dataset.clone(),
        art.llm_path.clone(),
        art.summary_path.clone(),
        art.tokens_path.clone(),
        art.column_report_path.clone(),
        art.config_used_path.clone(),
        art.log_path.clone(),
    ];
    if let Some(k) = &art.keymap_path {
        files.push(k.clone());
    }
    if let Some(k) = &art.keymap_columns_path {
        files.push(k.clone());
    }
    for f in files {
        eprintln!("      {}", g(&f.display().to_string()));
    }
    if art.warnings > 0 {
        eprintln!(
            "{}",
            ui::orange(
                Stream::Err,
                &format!(
                    "[!] {} warning(s) - review them above before sharing the masked dataset.",
                    art.warnings
                )
            )
        );
    }
    eprintln!(
        "{}",
        g("      Next: open llm.txt for ready-to-paste LLM prompts built for this dataset.")
    );
}
