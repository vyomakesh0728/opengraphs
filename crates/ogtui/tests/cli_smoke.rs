use prost::Message;
use serde_json::Value;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        let unique = NEXT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("ogtui-cli-smoke-{nanos}-{unique}"));
        fs::create_dir_all(&path).expect("create temp test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, PartialEq, Message)]
struct Event {
    #[prost(double, tag = "1")]
    wall_time: f64,
    #[prost(int64, tag = "2")]
    step: i64,
    #[prost(string, optional, tag = "3")]
    file_version: Option<String>,
    #[prost(message, optional, tag = "5")]
    summary: Option<Summary>,
}

#[derive(Clone, PartialEq, Message)]
struct Summary {
    #[prost(message, repeated, tag = "1")]
    value: Vec<SummaryValue>,
}

#[derive(Clone, PartialEq, Message)]
struct SummaryValue {
    #[prost(string, tag = "1")]
    tag: String,
    #[prost(float, optional, tag = "2")]
    simple_value: Option<f32>,
}

fn ogtui<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_ogtui"))
        .args(args)
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .output()
        .expect("run ogtui")
}

fn assert_success(output: &Output) -> String {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be utf8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr should be utf8");
    assert!(
        output.status.success(),
        "expected success, got {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout,
        stderr
    );
    stdout
}

fn assert_failure(output: &Output) -> String {
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be utf8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr should be utf8");
    assert!(
        !output.status.success(),
        "expected failure\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    stderr
}

fn masked_crc32c(data: &[u8]) -> u32 {
    let crc = crc32c::crc32c(data);
    ((crc >> 15) | (crc << 17)).wrapping_add(0xa282_ead8)
}

fn write_record(mut file: &File, data: &[u8]) {
    let len = (data.len() as u64).to_le_bytes();
    file.write_all(&len).expect("write record length");
    file.write_all(&masked_crc32c(&len).to_le_bytes())
        .expect("write length crc");
    file.write_all(data).expect("write record data");
    file.write_all(&masked_crc32c(data).to_le_bytes())
        .expect("write data crc");
}

fn write_tfevents_file(path: &Path, samples: &[(i64, &str, f32)]) {
    let file = File::create(path).expect("create event file");
    for (step, tag, value) in samples {
        let event = Event {
            wall_time: *step as f64,
            step: *step,
            file_version: None,
            summary: Some(Summary {
                value: vec![SummaryValue {
                    tag: (*tag).to_string(),
                    simple_value: Some(*value),
                }],
            }),
        };
        write_record(&file, &event.encode_to_vec());
    }
}

fn create_run(root: &Path, project: &str, run_id: &str, samples: &[(i64, &str, f32)]) -> PathBuf {
    let run_dir = root.join(project).join(run_id);
    fs::create_dir_all(&run_dir).expect("create run directory");
    write_tfevents_file(&run_dir.join("events.out.tfevents.test"), samples);
    run_dir
}

fn sample_run(root: &Path) -> PathBuf {
    create_run(
        root,
        "alpha",
        "demo-run",
        &[
            (1, "train/loss", 1.25),
            (2, "train/loss", 0.75),
            (2, "sys/gpu_util", 91.0),
        ],
    )
}

#[test]
fn top_level_help_mentions_graph_alias_and_subcommands() {
    let stdout = assert_success(&ogtui(["--help"]));
    assert!(stdout.contains("OpenGraphs CLI + TUI"));
    assert!(stdout.contains("Usage: ogtui [OPTIONS] [COMMAND]"));
    assert!(stdout.contains("list     List entities"));
    assert!(stdout.contains("search   Search entities"));
    assert!(stdout.contains("aliases: --graphs"));
}

#[test]
fn run_help_mentions_graph_alias_and_runtime_values() {
    let stdout = assert_success(&ogtui(["run", "--help"]));
    assert!(stdout.contains("Usage: ogtui run [OPTIONS] <FILE>"));
    assert!(stdout.contains("--graph <GRAPH>"));
    assert!(stdout.contains("aliases: --graphs"));
    assert!(stdout.contains("[possible values: local, modal]"));
}

#[test]
fn graphs_alias_is_accepted_before_graph_validation_runs() {
    let stderr = assert_failure(&ogtui(["run", "demo.py", "--graphs", "not-json"]));
    assert!(stderr.contains("parsing --graph JSON"));
    assert!(!stderr.contains("unexpected argument '--graphs'"));
}

#[test]
fn list_projects_text_reports_run_counts() {
    let temp = TestDir::new();
    let root = temp.path().to_str().expect("temp path should be utf8");
    sample_run(temp.path());
    fs::create_dir_all(temp.path().join("beta")).expect("create empty project");

    let stdout = assert_success(&ogtui(["list", "projects", "--path", root]));

    assert!(stdout.contains("- alpha (1)"));
    assert!(stdout.contains("- beta (0)"));
}

#[test]
fn list_runs_json_reports_real_run_summary() {
    let temp = TestDir::new();
    let root = temp.path().to_str().expect("temp path should be utf8");
    sample_run(temp.path());

    let stdout = assert_success(&ogtui([
        "--json",
        "list",
        "runs",
        "--path",
        root,
        "--project",
        "alpha",
        "--status",
        "running",
    ]));
    let payload: Value = serde_json::from_str(&stdout).expect("parse list runs json");
    let runs = payload["runs"].as_array().expect("runs array");

    assert_eq!(payload["count"].as_u64(), Some(1));
    assert_eq!(runs[0]["id"].as_str(), Some("demo-run"));
    assert_eq!(runs[0]["status"].as_str(), Some("running"));
    assert_eq!(runs[0]["metric_count"].as_u64(), Some(2));
    assert_eq!(runs[0]["event_count"].as_u64(), Some(3));
    assert_eq!(runs[0]["max_step"].as_i64(), Some(2));
}

#[test]
fn list_metrics_and_system_metrics_filter_tags() {
    let temp = TestDir::new();
    let root = temp.path().to_str().expect("temp path should be utf8");
    sample_run(temp.path());

    let metrics_stdout = assert_success(&ogtui([
        "list",
        "metrics",
        "--path",
        root,
        "--project",
        "alpha",
        "--run",
        "demo-run",
    ]));
    assert!(metrics_stdout.contains("- sys/gpu_util"));
    assert!(metrics_stdout.contains("- train/loss"));

    let system_stdout = assert_success(&ogtui([
        "--json",
        "list",
        "system-metrics",
        "--path",
        root,
        "--project",
        "alpha",
        "--run",
        "demo-run",
    ]));
    let payload: Value =
        serde_json::from_str(&system_stdout).expect("parse system metrics response");
    let metrics = payload["metrics"].as_array().expect("metrics array");

    assert_eq!(payload["count"].as_u64(), Some(1));
    assert_eq!(metrics[0].as_str(), Some("sys/gpu_util"));
}

#[test]
fn get_run_json_reports_latest_metric_values() {
    let temp = TestDir::new();
    let root = temp.path().to_str().expect("temp path should be utf8");
    sample_run(temp.path());

    let stdout = assert_success(&ogtui([
        "--json",
        "get",
        "run",
        "--path",
        root,
        "--project",
        "alpha",
        "--run",
        "demo-run",
    ]));
    let payload: Value = serde_json::from_str(&stdout).expect("parse get run json");

    assert_eq!(payload["run"]["id"].as_str(), Some("demo-run"));
    assert_eq!(
        payload["latest_metrics"]["train/loss"]["step"].as_f64(),
        Some(2.0)
    );
    assert_eq!(
        payload["latest_metrics"]["train/loss"]["value"].as_f64(),
        Some(0.75)
    );
    assert_eq!(
        payload["latest_metrics"]["sys/gpu_util"]["value"].as_f64(),
        Some(91.0)
    );
}
