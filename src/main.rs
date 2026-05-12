use std::fs;
use std::process::ExitCode;

use anyhow::{Context, Result, bail, ensure};
use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};

use gha_container_proof::{
    ActionPlanInput, CheckWorkflowOptions, JobPlanInput, OutputFormat, ProbeInput, RunnerOs,
    apply_strict, render_receipt, run_check_workflow, run_plan_action, run_plan_job, run_probe,
};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Classify GitHub Actions job containers and Docker actions with Docker CLI probe \
             receipts — offline by default."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Receipt format.
    #[arg(long, global = true, value_enum, default_value = "text")]
    format: OutputFormat,

    /// Write the receipt to this path instead of stdout.
    #[arg(long, global = true, value_name = "PATH")]
    output: Option<Utf8PathBuf>,

    /// Treat warnings as failures.
    #[arg(long, global = true)]
    strict: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Scan workflow YAML for job containers and Docker action surfaces.
    CheckWorkflow(CheckWorkflowArgs),
    /// Classify a rendered job-container request.
    PlanJob(PlanJobArgs),
    /// Classify a Docker action request.
    PlanAction(PlanActionArgs),
    /// Optionally use the Docker CLI to verify runtime availability and image behavior.
    Probe(ProbeArgs),
}

#[derive(Debug, Args)]
struct CheckWorkflowArgs {
    /// Repository root. Used to discover `.github/workflows/*.yml` when --workflow is absent.
    #[arg(long, value_name = "DIR", default_value = ".")]
    repo: Utf8PathBuf,

    /// Workspace root used to resolve local action references such as `./actions/build`.
    #[arg(long, value_name = "DIR")]
    workspace: Option<Utf8PathBuf>,

    /// One or more workflow YAML paths. Repeatable.
    #[arg(long, value_name = "PATH")]
    workflow: Vec<Utf8PathBuf>,

    /// Runner OS for the workflow's runs-on classification.
    #[arg(long, value_enum, default_value = "linux")]
    runner_os: RunnerOs,
}

#[derive(Debug, Args)]
struct PlanJobArgs {
    /// Job id this container belongs to.
    #[arg(long, value_name = "JOB", default_value = "job-container")]
    job_id: String,

    /// Runner OS for the host runner.
    #[arg(long, value_enum, default_value = "linux")]
    runner_os: RunnerOs,

    /// `runs-on` labels (repeat or newline-separate).
    #[arg(long = "runs-on", value_name = "LABEL")]
    runs_on: Vec<String>,

    /// Container image.
    #[arg(long, value_name = "IMAGE")]
    container: Option<String>,

    /// KEY=VALUE env entries (repeatable).
    #[arg(long = "env", value_name = "KEY=VALUE")]
    env: Vec<String>,

    /// Port mappings, repeatable.
    #[arg(long = "port", value_name = "PORT")]
    ports: Vec<String>,

    /// Volume mappings, repeatable.
    #[arg(long = "volume", value_name = "VOLUME")]
    volumes: Vec<String>,

    /// Raw Docker options string. May contain literal `--` flags; quote the
    /// whole value so the shell passes it to clap as one argument.
    #[arg(
        long,
        value_name = "OPTIONS",
        default_value = "",
        allow_hyphen_values = true
    )]
    options: String,

    /// Whether container.credentials.username was provided.
    #[arg(long = "credentials-username", default_value_t = false)]
    credentials_username: bool,

    /// Whether container.credentials.password was provided.
    #[arg(long = "credentials-password", default_value_t = false)]
    credentials_password: bool,
}

#[derive(Debug, Args)]
struct PlanActionArgs {
    /// Action reference (e.g. `./actions/build`, `docker://alpine:3`).
    #[arg(long, value_name = "REF")]
    action_ref: String,

    /// Step id, when known.
    #[arg(long, value_name = "ID")]
    step_id: Option<String>,

    /// Local action directory (used to load `action.yml`).
    #[arg(long, value_name = "DIR")]
    action_path: Option<Utf8PathBuf>,

    /// Override `runs.using`.
    #[arg(long, value_name = "USING")]
    using: Option<String>,

    /// Override `runs.image`.
    #[arg(long, value_name = "IMAGE")]
    image: Option<String>,

    /// Override `runs.entrypoint`.
    #[arg(long, value_name = "PATH")]
    entrypoint: Option<String>,

    /// Override `runs.pre-entrypoint`.
    #[arg(long = "pre-entrypoint", value_name = "PATH")]
    pre_entrypoint: Option<String>,

    /// Override `runs.post-entrypoint`.
    #[arg(long = "post-entrypoint", value_name = "PATH")]
    post_entrypoint: Option<String>,

    /// Args string preserved verbatim into the receipt; may be a single
    /// shell-quoted string or repeated `--args` entries.
    #[arg(long, value_name = "ARGS", allow_hyphen_values = true)]
    args: Vec<String>,

    /// KEY=VALUE env entries (repeatable).
    #[arg(long = "env", value_name = "KEY=VALUE")]
    env: Vec<String>,
}

#[derive(Debug, Args)]
struct ProbeArgs {
    /// Docker image to probe.
    #[arg(long, value_name = "IMAGE")]
    image: String,

    /// Runner OS for compatibility classification.
    #[arg(long, value_enum, default_value = "linux")]
    runner_os: RunnerOs,

    /// Tools to probe via `docker run --rm <image> <tool> --version`.
    #[arg(long = "tool", value_name = "TOOL")]
    tools: Vec<String>,

    /// Raw commands to probe via `docker run --rm <image> sh -c <command>`.
    #[arg(long = "command", value_name = "COMMAND", allow_hyphen_values = true)]
    commands: Vec<String>,

    /// Allow probe to pull images via `docker pull` when not present locally.
    #[arg(long = "allow-pull")]
    allow_pull: bool,

    /// Override the Docker CLI binary path. Tests use this to point at a
    /// fake docker; production runs leave it unset so `which docker` is used.
    #[arg(long = "docker-bin", value_name = "PATH")]
    docker_bin: Option<Utf8PathBuf>,
}

fn main() -> ExitCode {
    match run() {
        Ok(success) => {
            if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool> {
    let cli = Cli::parse();

    let mut receipt = match &cli.command {
        Command::CheckWorkflow(args) => {
            ensure!(
                args.repo.exists(),
                "--repo must point at an existing directory"
            );
            let workspace = args.workspace.clone().unwrap_or_else(|| args.repo.clone());
            let options = CheckWorkflowOptions {
                repo_root: args.repo.clone(),
                workspace,
                workflows: args.workflow.clone(),
                runner_os: args.runner_os,
            };
            run_check_workflow(&options)?
        }
        Command::PlanJob(args) => run_plan_job(&JobPlanInput {
            job_id: args.job_id.clone(),
            runner_os: args.runner_os,
            runs_on: expand_lines(&args.runs_on),
            container_image: args.container.clone(),
            env: parse_env_pairs(&args.env)?,
            ports: expand_lines(&args.ports),
            volumes: expand_lines(&args.volumes),
            options: args.options.clone(),
            credentials_username_present: args.credentials_username,
            credentials_password_present: args.credentials_password,
            location: None,
        })?,
        Command::PlanAction(args) => run_plan_action(&ActionPlanInput {
            action_ref: args.action_ref.clone(),
            step_id: args.step_id.clone(),
            action_path: args.action_path.clone(),
            using: args.using.clone(),
            image: args.image.clone(),
            entrypoint: args.entrypoint.clone(),
            pre_entrypoint: args.pre_entrypoint.clone(),
            post_entrypoint: args.post_entrypoint.clone(),
            args: expand_lines(&args.args),
            env: parse_env_pairs(&args.env)?,
            location: None,
        })?,
        Command::Probe(args) => run_probe(&ProbeInput {
            image: args.image.clone(),
            runner_os: args.runner_os,
            tools: expand_lines(&args.tools),
            commands: args.commands.clone(),
            allow_pull: args.allow_pull,
            docker_bin: args.docker_bin.clone(),
        })?,
    };

    if cli.strict {
        apply_strict(&mut receipt);
    }

    let rendered = render_receipt(&receipt, cli.format)?;
    if let Some(output) = &cli.output {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {parent}"))?;
        }
        fs::write(output, &rendered).with_context(|| format!("writing {output}"))?;
    } else {
        print!("{rendered}");
    }

    let ok = receipt.is_success(cli.strict);
    if !ok {
        eprintln!(
            "gha-container-proof: {} failed, {} warned (strict={})",
            receipt.summary.failed, receipt.summary.warnings, cli.strict
        );
    }
    Ok(ok)
}

fn expand_lines(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| value.lines())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_env_pairs(pairs: &[String]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for raw in expand_lines(pairs) {
        let Some((key, value)) = raw.split_once('=') else {
            bail!("expected KEY=VALUE, got `{raw}`");
        };
        out.push((key.trim().to_owned(), value.to_owned()));
    }
    Ok(out)
}
