//! Structured conversation commands and the Rust client adapter interface.
#[cfg(test)]
use serde_json::json;
use serde_json::{Map, Value};
use std::future::Future;

/// A failure that ends the conversation, worded for the transcript.
pub type Failure = &'static str;

/// Configuration for starting a native client.
#[derive(Debug, Clone)]
pub struct ChatInit {
    #[cfg(test)]
    pub provider: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub resume_id: Option<String>,
}

impl ChatInit {
    #[cfg(test)]
    pub fn to_value(&self) -> Value {
        let mut init = json!({"type":"init","provider":self.provider,"program":self.program,"args":self.args,"cwd":self.cwd});
        if let Some(id) = &self.resume_id {
            init["resume_id"] = json!(id);
        }
        init
    }
}

/// One decision a person can give a native request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Accept,
    AcceptSession,
    Decline,
    Cancel,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::AcceptSession => "accept_session",
            Self::Decline => "decline",
            Self::Cancel => "cancel",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "accept" => Self::Accept,
            "accept_session" => Self::AcceptSession,
            "decline" => Self::Decline,
            "cancel" => Self::Cancel,
            _ => return None,
        })
    }
}

/// Answers to a native question, by question id.
pub type Answers = Map<String, Value>;

/// A validated structured conversation command.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatCommand {
    Message {
        id: String,
        content: String,
    },
    Steer {
        id: String,
        content: String,
    },
    Approval {
        request_id: String,
        decision: Decision,
        answers: Option<Answers>,
    },
    Interrupt,
    Model {
        model: Option<String>,
        effort: Option<String>,
    },
    Permission {
        mode: String,
    },
    Shutdown,
}

fn has_control(value: &str) -> bool {
    value.chars().any(|c| (c as u32) < 0x20)
}

impl ChatCommand {
    /// Parse and validate a conversation command.
    pub fn parse(value: &Value) -> Result<Self, &'static str> {
        let Some(object) = value.as_object() else {
            return Err("Expected a structured chat object.");
        };
        let text = |key: &str| object.get(key).and_then(Value::as_str);
        match text("type") {
            Some(kind @ ("message" | "steer")) => {
                let (Some(id), Some(content)) = (text("id"), text("content")) else {
                    return Err("Invalid or oversized chat message.");
                };
                if id.is_empty()
                    || id.chars().count() > 128
                    || has_control(id)
                    || content.trim().is_empty()
                    || content.contains('\0')
                    || content.len() > 256 * 1024
                {
                    return Err("Invalid or oversized chat message.");
                }
                let (id, content) = (id.to_owned(), content.to_owned());
                Ok(if kind == "message" {
                    Self::Message { id, content }
                } else {
                    Self::Steer { id, content }
                })
            }
            Some("approval") => {
                let request_id = text("request_id")
                    .filter(|id| id.chars().count() <= 256)
                    .ok_or("Invalid approval response.")?;
                let decision = text("decision")
                    .and_then(Decision::parse)
                    .ok_or("Invalid approval response.")?;
                let answers = match object.get("answers") {
                    None => None,
                    Some(Value::Object(answers))
                        if answers.len() <= 16
                            && answers.iter().all(|(key, values)| {
                                key.chars().count() <= 256
                                    && values.as_array().is_some_and(|values| {
                                        values.len() <= 32
                                            && values.iter().all(|value| {
                                                value.as_str().is_some_and(|v| v.len() <= 8192)
                                            })
                                    })
                            }) =>
                    {
                        Some(answers.clone())
                    }
                    Some(_) => return Err("Invalid native question answers."),
                };
                Ok(Self::Approval {
                    request_id: request_id.to_owned(),
                    decision,
                    answers,
                })
            }
            Some("model") => {
                let model = match object.get("model") {
                    None => None,
                    Some(Value::String(model))
                        if !model.trim().is_empty()
                            && model.chars().count() <= 128
                            && !has_control(model) =>
                    {
                        Some(model.clone())
                    }
                    Some(_) => return Err("Invalid model selection."),
                };
                let effort = match object.get("effort") {
                    None => None,
                    Some(Value::String(effort))
                        if (1..=16).contains(&effort.len())
                            && effort.bytes().all(|b| b.is_ascii_lowercase()) =>
                    {
                        Some(effort.clone())
                    }
                    Some(_) => return Err("Invalid reasoning effort."),
                };
                if model.is_none() && effort.is_none() {
                    return Err("Choose a model or a thinking depth.");
                }
                Ok(Self::Model { model, effort })
            }
            Some("permission") => match text("mode") {
                Some(mode @ ("ask" | "plan" | "accept_edits" | "danger")) => Ok(Self::Permission {
                    mode: mode.to_owned(),
                }),
                _ => Err("Unsupported permission mode."),
            },
            Some("interrupt") => Ok(Self::Interrupt),
            Some("shutdown") => Ok(Self::Shutdown),
            _ => Err("Unsupported chat control message."),
        }
    }
}

/// One structured conversation's client, behind the internal protocol.
///
/// Each method returns once its command has been taken; what follows
/// arrives as events from [`ClientAdapter::next_event`]. A `Failure` from any
/// of them ends the conversation.
pub trait ClientAdapter: Send + Sized + 'static {
    /// The process this conversation owns, for resource reporting.
    fn pid(&self) -> Option<u32>;
    /// Start the client: the `init` line.
    fn start(&mut self, init: &ChatInit) -> impl Future<Output = Result<(), Failure>> + Send;
    fn message(
        &mut self,
        id: String,
        content: String,
    ) -> impl Future<Output = Result<(), Failure>> + Send;
    fn steer(
        &mut self,
        id: String,
        content: String,
    ) -> impl Future<Output = Result<(), Failure>> + Send;
    fn approve(
        &mut self,
        request_id: String,
        decision: Decision,
        answers: Option<Answers>,
    ) -> impl Future<Output = Result<(), Failure>> + Send;
    fn interrupt(&mut self) -> impl Future<Output = Result<(), Failure>> + Send;
    fn set_model(
        &mut self,
        model: Option<String>,
        effort: Option<String>,
    ) -> impl Future<Output = Result<(), Failure>> + Send;
    fn set_permission_mode(
        &mut self,
        mode: String,
    ) -> impl Future<Output = Result<(), Failure>> + Send;
    /// The next event, unvalidated. `None` once the client has nothing more
    /// to say. Must be safe to drop mid-way: the caller selects over it.
    fn next_event(&mut self) -> impl Future<Output = Option<Result<Value, Failure>>> + Send;
    /// Stop the client and everything it started.
    fn shutdown(self) -> impl Future<Output = ()> + Send;
}

/// Hand a parsed command to the adapter. `Shutdown` is the caller's to act on.
pub async fn dispatch<A: ClientAdapter>(
    adapter: &mut A,
    command: ChatCommand,
) -> Result<(), Failure> {
    match command {
        ChatCommand::Message { id, content } => adapter.message(id, content).await,
        ChatCommand::Steer { id, content } => adapter.steer(id, content).await,
        ChatCommand::Approval {
            request_id,
            decision,
            answers,
        } => adapter.approve(request_id, decision, answers).await,
        ChatCommand::Interrupt => adapter.interrupt().await,
        ChatCommand::Model { model, effort } => adapter.set_model(model, effort).await,
        ChatCommand::Permission { mode } => adapter.set_permission_mode(mode).await,
        ChatCommand::Shutdown => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_commands_are_refused() {
        let parse = |value: Value| ChatCommand::parse(&value);
        assert_eq!(
            parse(json!({"type":"message","id":"a","content":"hi"})),
            Ok(ChatCommand::Message {
                id: "a".into(),
                content: "hi".into()
            })
        );
        assert_eq!(
            parse(json!({"type":"message","id":"a","content":"   "})),
            Err("Invalid or oversized chat message.")
        );
        assert_eq!(
            parse(json!({"type":"approval","request_id":"r","decision":"acceptForSession"})),
            Err("Invalid approval response.")
        );
        assert_eq!(
            parse(
                json!({"type":"approval","request_id":"r","decision":"accept","answers":{"q":"x"}})
            ),
            Err("Invalid native question answers.")
        );
        assert_eq!(
            parse(json!({"type":"model"})),
            Err("Choose a model or a thinking depth.")
        );
        assert_eq!(
            parse(json!({"type":"model","effort":"High"})),
            Err("Invalid reasoning effort.")
        );
        assert_eq!(
            parse(json!({"type":"permission","mode":"bypass"})),
            Err("Unsupported permission mode.")
        );
        assert_eq!(
            parse(json!({"type":"dance"})),
            Err("Unsupported chat control message.")
        );
        assert_eq!(parse(json!("x")), Err("Expected a structured chat object."));
        assert_eq!(
            parse(json!({"type":"interrupt"})),
            Ok(ChatCommand::Interrupt)
        );
    }
}
