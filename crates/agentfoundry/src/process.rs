use crate::{Failure, Result};
use std::{
    io::Read,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Bound both pipes and lifetime; never treat a truncated result as proof.
pub fn capture(command: &mut Command, limit: usize, timeout: Duration) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            Failure::new(
                if error.kind() == std::io::ErrorKind::NotFound {
                    "MISSING_CAPABILITY"
                } else {
                    "IO_ERROR"
                },
                error.to_string(),
            )
        })?;
    let oversized = Arc::new(AtomicBool::new(false));
    let reader = |stream: Option<Box<dyn Read + Send>>, flag: Arc<AtomicBool>| {
        std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            if let Some(stream) = stream {
                stream.take(limit as u64 + 1).read_to_end(&mut bytes)?;
            }
            if bytes.len() > limit {
                flag.store(true, Ordering::Release);
            }
            Ok(bytes)
        })
    };
    let stdout = reader(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        oversized.clone(),
    );
    let stderr = reader(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        oversized.clone(),
    );
    let started = Instant::now();
    let mut failure = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                failure = Some(Failure::new("IO_ERROR", error.to_string()));
                let _ = child.kill();
                break child.wait()?;
            }
        }
        if oversized.load(Ordering::Acquire) || started.elapsed() >= timeout {
            failure = Some(Failure::new(
                if oversized.load(Ordering::Acquire) {
                    "INPUT_LIMIT_EXCEEDED"
                } else {
                    "IO_ERROR"
                },
                "subprocess exceeded output/lifetime limit; no complete proof captured",
            ));
            let _ = child.kill();
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    while !stdout.is_finished() || !stderr.is_finished() {
        if started.elapsed() >= timeout {
            return Err(Failure::new(
                "IO_ERROR",
                "subprocess descendants retained output pipes; complete capture unavailable",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let out = stdout
        .join()
        .map_err(|_| Failure::new("INTERNAL_ERROR", "stdout reader failed"))??;
    let err = stderr
        .join()
        .map_err(|_| Failure::new("INTERNAL_ERROR", "stderr reader failed"))??;
    if let Some(error) = failure {
        return Err(error);
    }
    if out.len() > limit || err.len() > limit {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "subprocess output exceeded limit; truncated proof rejected",
        ));
    }
    Ok(Output {
        status,
        stdout: out,
        stderr: err,
    })
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn output_overflow_is_not_success() {
        let mut cmd = Command::new("printf");
        cmd.arg("123456789");
        assert_eq!(
            capture(&mut cmd, 4, Duration::from_secs(1))
                .unwrap_err()
                .code,
            "INPUT_LIMIT_EXCEEDED"
        );
    }
    #[test]
    fn timeout_kills_direct_child() {
        let mut cmd = Command::new("sleep");
        cmd.arg("10");
        assert!(capture(&mut cmd, 1024, Duration::from_millis(20)).is_err());
    }
    #[test]
    fn inherited_pipe_does_not_extend_capture_deadline() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 1 &"]);
        let started = Instant::now();
        assert_eq!(
            capture(&mut cmd, 1024, Duration::from_millis(50))
                .unwrap_err()
                .code,
            "IO_ERROR"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
