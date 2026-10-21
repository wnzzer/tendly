//! Test-only JSONL transport for synthetic supervisor fixtures.
use crate::chat_adapter::*;
use serde_json::{Value, json};
use std::time::Duration;
use tendly_runtime::process::OwnedGroup;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
/// Test transport for synthetic JSONL supervisor fixtures.
pub struct FixtureAdapter {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    buffer: Vec<u8>,
    failed: bool,
}

impl FixtureAdapter {
    pub fn new(mut child: tokio::process::Child) -> Result<Self, crate::ApiError> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| crate::ApiError::conflict("Cannot open native chat input"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| crate::ApiError::conflict("Cannot open native chat output"))?;
        Ok(Self {
            child,
            stdin,
            stdout,
            buffer: Vec::new(),
            failed: false,
        })
    }
    async fn write(&mut self, line: Value, failure: Failure) -> Result<(), Failure> {
        match tokio::time::timeout(
            Duration::from_secs(3),
            self.stdin.write_all(format!("{line}\n").as_bytes()),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            _ => Err(failure),
        }
    }
    /// The next complete line already read, as an event.
    fn take_line(&mut self) -> Option<Result<Value, Failure>> {
        let end = self.buffer.iter().position(|b| *b == b'\n')?;
        if end > 256 * 1024 {
            return Some(Err("Native chat event exceeded the size limit"));
        }
        let line: Vec<_> = self.buffer.drain(..=end).collect();
        Some(serde_json::from_slice::<Value>(&line).map_err(
            |_| "Native chat returned an unsupported event; update the installed bridge/client",
        ))
    }
}

const INPUT_UNAVAILABLE: Failure = "Native chat input is unavailable";

impl ClientAdapter for FixtureAdapter {
    fn pid(&self) -> Option<u32> {
        self.child.id()
    }
    async fn start(&mut self, init: &ChatInit) -> Result<(), Failure> {
        self.write(
            init.to_value(),
            "Native chat input closed during initialization",
        )
        .await
    }
    async fn message(&mut self, id: String, content: String) -> Result<(), Failure> {
        self.write(
            json!({"type":"message","id":id,"content":content}),
            INPUT_UNAVAILABLE,
        )
        .await
    }
    async fn steer(&mut self, id: String, content: String) -> Result<(), Failure> {
        self.write(
            json!({"type":"steer","id":id,"content":content}),
            INPUT_UNAVAILABLE,
        )
        .await
    }
    async fn approve(
        &mut self,
        request_id: String,
        decision: Decision,
        answers: Option<Answers>,
    ) -> Result<(), Failure> {
        let mut line =
            json!({"type":"approval","request_id":request_id,"decision":decision.as_str()});
        if let Some(answers) = answers {
            line["answers"] = Value::Object(answers);
        }
        self.write(line, INPUT_UNAVAILABLE).await
    }
    async fn interrupt(&mut self) -> Result<(), Failure> {
        self.write(json!({"type":"interrupt"}), INPUT_UNAVAILABLE)
            .await
    }
    async fn set_model(
        &mut self,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<(), Failure> {
        let mut line = json!({"type":"model"});
        if let Some(model) = model {
            line["model"] = json!(model);
        }
        if let Some(effort) = effort {
            line["effort"] = json!(effort);
        }
        self.write(line, INPUT_UNAVAILABLE).await
    }
    async fn set_permission_mode(&mut self, mode: String) -> Result<(), Failure> {
        self.write(json!({"type":"permission","mode":mode}), INPUT_UNAVAILABLE)
            .await
    }
    async fn next_event(&mut self) -> Option<Result<Value, Failure>> {
        if self.failed {
            return None;
        }
        loop {
            if let Some(event) = self.take_line() {
                self.failed = event.is_err();
                return Some(event);
            }
            if self.buffer.len() > 256 * 1024 {
                self.failed = true;
                return Some(Err("Native chat event exceeded the size limit"));
            }
            let mut bytes = [0u8; 8192];
            // Cancel-safe: a read that is dropped has consumed nothing.
            match self.stdout.read(&mut bytes).await {
                Ok(0) => return None,
                Ok(count) => self.buffer.extend_from_slice(&bytes[..count]),
                Err(_) => {
                    self.failed = true;
                    return Some(Err("Native chat output closed unexpectedly"));
                }
            }
        }
    }
    async fn shutdown(mut self) {
        let mut group = OwnedGroup::new(self.child.id());
        let _ = tokio::time::timeout(
            Duration::from_millis(250),
            self.stdin.write_all(b"{\"type\":\"shutdown\"}\n"),
        )
        .await;
        // A dedicated, positively identified process group contains only this bridge
        // and its native CLI descendants. Never signal the host or unrelated Agents.
        // Windows has no polite request, so there the bridge gets the time it had to
        // shut down on its own after the message above before its tree is stopped.
        #[cfg(windows)]
        let _ = tokio::time::timeout(Duration::from_millis(1600), self.child.wait()).await;
        group.terminate();
        // Keep the leader unreaped until its owned group has been cleaned up.
        // Waiting for only the Node PID can leave TERM-ignoring descendants.
        #[cfg(unix)]
        tokio::time::sleep(Duration::from_millis(1600)).await;
        group.finish();
        if tokio::time::timeout(Duration::from_secs(2), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(1), self.child.wait()).await;
        }
    }
}
