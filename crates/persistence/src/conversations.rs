use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tendly_domain::InteractionMode;

fn content_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

/// The lines of a message after the list of attached files the composer
/// writes above it, if there is one.
fn without_attachment_header(text: &str) -> impl Iterator<Item = &str> {
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|line| {
        line.starts_with("Attached file") && line.ends_with("in this workspace:")
    }) {
        lines.next();
        while lines
            .peek()
            .is_some_and(|line| line.starts_with("- ") || line.trim().is_empty())
        {
            lines.next();
        }
    }
    lines
}

/// A session name taken from the message that opened it.
///
/// Both clients used to leave a summary in their transcripts and no longer do —
/// there is not one `"type":"summary"` line left across the local Claude
/// history — so the first thing the user asked for is the only description of a
/// session that exists anywhere. It is what `claude --resume` falls back to for
/// the same reason.
///
/// Attachment paths and a slash command are stripped: a session called
/// `/model` describes every session. Returns `None` when nothing legible is
/// left, which keeps the created-at name rather than inventing one.
fn derived_title(text: &str) -> Option<String> {
    let body = without_attachment_header(text)
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("@") && !line.starts_with('/'))?;
    // Count characters, not bytes: the limit is about how wide the sidebar row
    // reads, and CJK is where a byte limit would cut a name to a third.
    let mut title: String = body.chars().take(60).collect();
    if body.chars().count() > 60 {
        title.push('…');
    }
    Some(title).filter(|value| !value.is_empty())
}

/// One line of what the user said, for a session list: the attachment header
/// Tendly writes above a message is dropped, whitespace is folded, and the
/// line is cut by characters so CJK keeps its share of the width.
pub fn preview_line(text: &str) -> Option<String> {
    let body = without_attachment_header(text)
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut line: String = body.chars().take(80).collect();
    if body.chars().count() > 80 {
        line.push('…');
    }
    Some(line).filter(|value| !value.is_empty())
}

pub struct StoredConversation {
    pub events: Vec<Value>,
    pub truncated: bool,
}
/// One page of a conversation, oldest first. `older` is the cursor for the
/// page before it: the first sequence this page holds, while any event is
/// older than that.
#[derive(Debug)]
pub struct ConversationPage {
    pub events: Vec<Value>,
    pub older: Option<u64>,
    /// Set for a conversation stored before history was kept in full, whose
    /// start was trimmed away.
    pub truncated: bool,
}
/// One place a search found: the event, what it belongs to, and the words
/// around the match.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub seq: u64,
    /// `message` or `tool`.
    pub kind: String,
    /// The message's or the call's id: the row it is shown in.
    pub id: String,
    pub snippet: String,
}

/// The match with a little on either side, on one line, cut on characters.
pub fn snippet_of(text: &str, query: &str) -> String {
    const AROUND: usize = 40;
    let chars: Vec<char> = text
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let needle: Vec<char> = query.trim().chars().map(lower).collect();
    let at = (!needle.is_empty() && needle.len() <= chars.len())
        .then(|| {
            (0..=chars.len() - needle.len()).find(|&i| {
                needle
                    .iter()
                    .enumerate()
                    .all(|(k, n)| lower(chars[i + k]) == *n)
            })
        })
        .flatten()
        .unwrap_or(0);
    let start = at.saturating_sub(AROUND);
    let end = (at + needle.len() + AROUND).min(chars.len());
    let mut snippet: String = chars[start..end].iter().collect();
    snippet = snippet.trim().to_owned();
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < chars.len() {
        snippet.push('…');
    }
    snippet
}

/// How far back a finished message's deltas are looked for when where they
/// began is not known -- after a restart, mid-message.
const DELTA_REACH: u64 = 500;
pub enum MessageSubmission {
    New(Value),
    Duplicate,
    Conflict,
}

impl Store {
    pub fn set_interaction_mode(&self, id: SessionId, mode: InteractionMode) -> Result<bool> {
        Ok(self.connection.lock().expect("sqlite lock").execute(
            "UPDATE sessions SET interaction_mode=?1,updated_at=?2 WHERE id=?3 AND status IN ('stopped','failed') AND (provider!='terminal' OR ?1='pty')",
            params![if mode == InteractionMode::Structured {"structured"} else {"pty"}, Utc::now().to_rfc3339(), id.to_string()],
        )? == 1)
    }

    pub fn set_native_conversation_id(&self, id: SessionId, native_id: &str) -> Result<bool> {
        Ok(self.connection.lock().expect("sqlite lock").execute(
            "UPDATE sessions SET provider_session_id=?1 WHERE id=?2 AND interaction_mode='structured'",
            params![native_id,id.to_string()],
        )? == 1)
    }

    /// A confirmed provider-local switch starts a fresh native context. Never
    /// carry credentials/environment from the previous account to its successor.
    pub fn switch_session_configuration(
        &self,
        id: SessionId,
        profile_id: Option<Uuid>,
        model: Option<String>,
    ) -> Result<bool> {
        self.switch_session_configuration_with_effort(id, profile_id, model, None)
    }

    pub fn switch_session_configuration_with_effort(
        &self,
        id: SessionId,
        profile_id: Option<Uuid>,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<bool> {
        let mut connection = self.connection.lock().expect("sqlite lock");
        let tx = connection.transaction()?;
        let session = tx.query_row(
            &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1"),
            [id.to_string()],
            session_row,
        )?;
        if !matches!(
            session.status,
            SessionStatus::Stopped | SessionStatus::Failed
        ) || session.provider == ProviderKind::Terminal
        {
            return Ok(false);
        }
        let mut snapshot = profile_id
            .map(|pid| {
                tx.query_row(
                    &format!("SELECT {PROFILE_COLUMNS} FROM endpoint_profiles WHERE id=?1"),
                    [pid.to_string()],
                    profile_row,
                )
            })
            .transpose()?;
        if snapshot
            .as_ref()
            .is_some_and(|p| p.provider != session.provider)
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if model.is_some() || effort.is_some() {
            let p = snapshot.get_or_insert_with(|| EndpointProfile {
                id: Uuid::new_v4(),
                provider: session.provider,
                name: "Session configuration".into(),
                endpoint_url: None,
                model: None,
                permission_mode: "native".into(),
                secret_ref: None,
                proxy_url: None,
                effort: None,
                model_aliases: Default::default(),
                models: Vec::new(),
                api: None,
                native_config: None,
                environment: Default::default(),
                created_at: Utc::now(),
            });
            if let Some(model) = model {
                p.model = Some(model);
            }
            if let Some(effort) = effort {
                p.effort = Some(effort);
            }
        }
        let environment = snapshot
            .as_ref()
            .map(|p| p.environment.clone())
            .unwrap_or_default();
        tendly_domain::validate_environment(&environment)
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        let encoded = snapshot
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(conversion_error)?;
        tx.execute("UPDATE sessions SET endpoint_profile_id=?1,endpoint_snapshot=?2,environment=?3,provider_session_id=NULL,native_source_id=NULL,native_config_dir=NULL,resume_source_id=NULL,resume_configuration_revision=NULL,configuration_revision=configuration_revision+1,updated_at=?4,error=NULL WHERE id=?5",
            params![profile_id.map(|v|v.to_string()),encoded,serde_json::to_string(&environment).map_err(conversion_error)?,Utc::now().to_rfc3339(),id.to_string()])?;
        append(
            &tx,
            id,
            json!({"type":"configuration","id":Uuid::new_v4().to_string(),"profile_name":snapshot.as_ref().map(|p|p.name.as_str()).unwrap_or("Isolated configuration"),"text":"A new native conversation will use this configuration. The conversation above goes to it with the next message."}),
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Point a stopped session at another checkout of its workspace's
    /// repository -- a worktree on another branch -- or back at the workspace
    /// itself with `None`. The next start is a different process in a different
    /// directory, so like an endpoint change it is a new native context: the
    /// revision is bumped, the native id cleared, and a boundary recorded.
    /// Returns false when the session is not stopped.
    pub fn set_session_checkout(
        &self,
        id: SessionId,
        path: Option<&str>,
        branch: Option<&str>,
        label: &str,
    ) -> Result<bool> {
        let mut connection = self.connection.lock().expect("sqlite lock");
        let tx = connection.transaction()?;
        let session = tx.query_row(
            &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id=?1"),
            [id.to_string()],
            session_row,
        )?;
        if !matches!(
            session.status,
            SessionStatus::Stopped | SessionStatus::Failed
        ) {
            return Ok(false);
        }
        tx.execute(
            "UPDATE sessions SET checkout_path=?1,checkout_branch=?2,provider_session_id=NULL,native_source_id=NULL,native_config_dir=NULL,resume_source_id=NULL,resume_configuration_revision=NULL,configuration_revision=configuration_revision+1,updated_at=?3,error=NULL WHERE id=?4",
            params![path, branch, Utc::now().to_rfc3339(), id.to_string()],
        )?;
        append(
            &tx,
            id,
            json!({"type":"configuration","id":Uuid::new_v4().to_string(),"profile_name":label,"text":"The session now works in this checkout. A new native conversation starts there, and the conversation above goes to it with the next message."}),
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn conversation(&self, id: SessionId) -> Result<StoredConversation> {
        let conn = self.connection.lock().expect("sqlite lock");
        let (truncated, anchors) = conn
            .query_row(
                "SELECT truncated,anchors FROM conversation_meta WHERE session_id=?1",
                [id.to_string()],
                |r| Ok((r.get::<_, bool>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
            .unwrap_or((false, "{}".into()));
        let mut statement = conn.prepare(
            "SELECT event_json FROM conversation_events WHERE session_id=?1 ORDER BY seq",
        )?;
        let mut events = statement
            .query_map([id.to_string()], |r| {
                serde_json::from_str::<Value>(&r.get::<_, String>(0)?).map_err(conversion_error)
            })?
            .collect::<Result<Vec<_>>>()?;
        let anchors: std::collections::BTreeMap<String, Value> =
            serde_json::from_str(&anchors).map_err(conversion_error)?;
        for event in anchors.into_values() {
            if !events
                .iter()
                .any(|existing| existing["seq"] == event["seq"])
            {
                events.push(event);
            }
        }
        events.sort_by_key(|event| event["seq"].as_u64().unwrap_or(0));
        Ok(StoredConversation { events, truncated })
    }

    /// The latest `limit` events, or the `limit` before `before`, oldest first.
    /// The latest page also carries the events the current state is read
    /// from -- controls and pending approvals -- when they are older than it.
    pub fn conversation_page(
        &self,
        id: SessionId,
        before: Option<u64>,
        limit: usize,
    ) -> Result<ConversationPage> {
        let conn = self.connection.lock().expect("sqlite lock");
        let session = id.to_string();
        let (truncated, anchors) = conn
            .query_row(
                "SELECT truncated,anchors FROM conversation_meta WHERE session_id=?1",
                [&session],
                |r| Ok((r.get::<_, bool>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
            .unwrap_or((false, "{}".into()));
        let mut events: Vec<Value> = {
            let mut stmt = conn.prepare(
                "SELECT event_json FROM conversation_events WHERE session_id=?1 AND seq<?2 ORDER BY seq DESC LIMIT ?3",
            )?;
            stmt.query_map(
                params![session, before.map_or(i64::MAX, |b| b as i64), limit as i64],
                |r| {
                    serde_json::from_str::<Value>(&r.get::<_, String>(0)?).map_err(conversion_error)
                },
            )?
            .collect::<Result<_>>()?
        };
        events.reverse();
        let first = events.first().and_then(|e| e["seq"].as_u64());
        let older = match first {
            Some(first) => conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM conversation_events WHERE session_id=?1 AND seq<?2)",
                    params![session, first as i64],
                    |r| r.get::<_, bool>(0),
                )?
                .then_some(first),
            None => None,
        };
        if before.is_none() {
            let anchors: std::collections::BTreeMap<String, Value> =
                serde_json::from_str(&anchors).map_err(conversion_error)?;
            let start = first.unwrap_or(u64::MAX);
            let mut carried: Vec<Value> = anchors
                .into_values()
                .filter(|event| event["seq"].as_u64().is_some_and(|seq| seq < start))
                .collect();
            carried.sort_by_key(|event| event["seq"].as_u64().unwrap_or(0));
            carried.dedup_by_key(|event| event["seq"].as_u64());
            carried.extend(events);
            events = carried;
        }
        Ok(ConversationPage {
            events,
            older,
            truncated,
        })
    }
    /// Where `query` appears in the whole history -- what was said, and the
    /// calls made and what they answered -- newest first, once per message or
    /// call, at most `limit`. Streamed pieces are not searched: the finished
    /// message holds the same words. The query is matched as written, not as
    /// a pattern, and without regard to case for Latin letters.
    pub fn search_conversation(
        &self,
        id: SessionId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let query = query.trim();
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let escaped: String = query
            .chars()
            .flat_map(|c| match c {
                '\\' | '%' | '_' => vec!['\\', c],
                c => vec![c],
            })
            .collect();
        let pattern = format!("%{escaped}%");
        let conn = self.connection.lock().expect("sqlite lock");
        let mut stmt = conn.prepare(
            "SELECT seq,event_json FROM conversation_events WHERE session_id=?1 \
             AND json_extract(event_json,'$.type') IN ('message','tool') \
             AND coalesce(json_extract(event_json,'$.delta'),0)=0 \
             AND (json_extract(event_json,'$.text') LIKE ?2 ESCAPE '\\' \
               OR json_extract(event_json,'$.activity') LIKE ?2 ESCAPE '\\' \
               OR json_extract(event_json,'$.name') LIKE ?2 ESCAPE '\\') \
             ORDER BY seq DESC",
        )?;
        let mut rows = stmt.query(params![id.to_string(), pattern])?;
        let mut seen = std::collections::HashSet::new();
        let mut hits = Vec::new();
        while let Some(row) = rows.next()? {
            let seq: u64 = row.get(0)?;
            let event: Value =
                serde_json::from_str(&row.get::<_, String>(1)?).map_err(conversion_error)?;
            let (Some(kind), Some(owner)) = (event["type"].as_str(), event["id"].as_str()) else {
                continue;
            };
            if !seen.insert((kind.to_owned(), owner.to_owned())) {
                continue;
            }
            let lowered = query.to_lowercase();
            let text = ["text", "activity", "name"]
                .iter()
                .filter_map(|field| event[*field].as_str())
                .find(|text| text.to_lowercase().contains(&lowered))
                .unwrap_or_default();
            hits.push(SearchHit {
                seq,
                kind: kind.to_owned(),
                id: owner.to_owned(),
                snippet: snippet_of(text, query),
            });
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }
    /// The events after `after`, or `None` when there are more than `limit` --
    /// a view that far behind is better sent the latest page -- or when
    /// `after` is past the last event, which no view of this one can hold.
    pub fn conversation_after(
        &self,
        id: SessionId,
        after: u64,
        limit: usize,
    ) -> Result<Option<Vec<Value>>> {
        let conn = self.connection.lock().expect("sqlite lock");
        let next: u64 = conn
            .query_row(
                "SELECT next_seq FROM conversation_meta WHERE session_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(1);
        if after >= next {
            return Ok(None);
        }
        let mut stmt = conn.prepare(
            "SELECT event_json FROM conversation_events WHERE session_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3",
        )?;
        let events: Vec<Value> = stmt
            .query_map(
                params![id.to_string(), after as i64, limit as i64 + 1],
                |r| {
                    serde_json::from_str::<Value>(&r.get::<_, String>(0)?).map_err(conversion_error)
                },
            )?
            .collect::<Result<_>>()?;
        Ok((events.len() <= limit).then_some(events))
    }

    /// What was said before the latest fresh native context, written out for
    /// the first message sent into it, or `None` once that context has been
    /// spoken to, when it never began, or when nothing was said before it.
    ///
    /// An endpoint or checkout change starts the client over with nothing, and
    /// the two clients' native histories cannot be moved between homes and
    /// endpoints. The words can: they go ahead of the next message, in front of
    /// the client only -- the stored message, and so the view, stays as typed.
    /// Tool calls are left out, and a `/clear` ends what is carried. The most
    /// recent messages within `budget` bytes are kept.
    pub fn carried_transcript(&self, id: SessionId, budget: usize) -> Result<Option<String>> {
        let conn = self.connection.lock().expect("sqlite lock");
        let session = id.to_string();
        let Some(boundary) = conn
            .query_row(
                "SELECT max(seq) FROM conversation_events WHERE session_id=?1 AND json_extract(event_json,'$.type')='configuration'",
                [&session],
                |r| r.get::<_, Option<i64>>(0),
            )?
        else {
            return Ok(None);
        };
        let spoken: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_events WHERE session_id=?1 AND seq>?2 AND json_extract(event_json,'$.type') IN ('message','cleared') AND coalesce(json_extract(event_json,'$.role'),'user')='user')",
            params![session, boundary],
            |r| r.get(0),
        )?;
        if spoken {
            return Ok(None);
        }
        let mut stmt = conn.prepare(
            "SELECT event_json FROM conversation_events WHERE session_id=?1 AND seq<?2 \
             AND json_extract(event_json,'$.type') IN ('message','configuration','cleared') ORDER BY seq",
        )?;
        let events = stmt
            .query_map(params![session, boundary], |r| {
                serde_json::from_str::<Value>(&r.get::<_, String>(0)?).map_err(conversion_error)
            })?
            .collect::<Result<Vec<_>>>()?;
        // Streamed pieces add up to a message and the finished one replaces
        // them, as the view reads them. A fresh context may reuse message IDs.
        let mut said: Vec<(bool, String)> = Vec::new();
        let mut index = std::collections::HashMap::<String, usize>::new();
        for event in &events {
            match event["type"].as_str() {
                Some("cleared") => {
                    said.clear();
                    index.clear();
                }
                Some("configuration") => index.clear(),
                _ => {
                    let (Some(owner), Some(text)) = (event["id"].as_str(), event["text"].as_str())
                    else {
                        continue;
                    };
                    let user = event["role"].as_str() == Some("user");
                    let delta = event["delta"].as_bool() == Some(true);
                    match index.get(owner) {
                        Some(&at) if said[at].0 == user => {
                            if delta {
                                said[at].1.push_str(text);
                            } else {
                                said[at].1 = text.to_owned();
                            }
                        }
                        _ => {
                            index.insert(owner.to_owned(), said.len());
                            said.push((user, text.to_owned()));
                        }
                    }
                }
            }
        }
        let blocks: Vec<String> = said
            .iter()
            .filter(|(_, text)| !text.trim().is_empty())
            .map(|(user, text)| {
                format!(
                    "{}:\n{}",
                    if *user { "User" } else { "Assistant" },
                    text.trim()
                )
            })
            .collect();
        if blocks.is_empty() {
            return Ok(None);
        }
        let mut kept = Vec::new();
        let mut used = 0;
        for block in blocks.iter().rev() {
            if used + block.len() + 2 > budget {
                break;
            }
            used += block.len() + 2;
            kept.push(block.as_str());
        }
        kept.reverse();
        if kept.is_empty() {
            return Ok(None);
        }
        let omitted = if kept.len() < blocks.len() {
            "Earlier messages did not fit and are left out.\n\n"
        } else {
            ""
        };
        Ok(Some(format!(
            "[The conversation so far in this session, from before it moved to a new endpoint or checkout. Take it as context; do not redo what is already done.]\n\n{omitted}{}\n\n[End of the earlier conversation. The new message follows.]\n\n",
            kept.join("\n\n")
        )))
    }

    pub fn append_conversation_event(&self, id: SessionId, event: Value) -> Result<Value> {
        let mut conn = self.connection.lock().expect("sqlite lock");
        let tx = conn.transaction()?;
        let event = append(&tx, id, event)?;
        // A finished message keeps its place but not its pieces (`compact_deltas`).
        if event["type"] == "message"
            && let (Some(message), Some(seq)) = (event["id"].as_str(), event["seq"].as_u64())
        {
            let key = (id.to_string(), message.to_owned());
            let mut streaming = self.streaming.lock().expect("streaming lock");
            if event.get("delta") == Some(&Value::Bool(true)) {
                if streaming.len() >= 4096 && !streaming.contains_key(&key) {
                    streaming.clear();
                }
                streaming.entry(key).or_insert(seq);
            } else {
                // Unknown after a restart: only the latest rows are searched.
                let from = streaming
                    .remove(&key)
                    .unwrap_or(seq.saturating_sub(DELTA_REACH));
                drop(streaming);
                compact_deltas(&tx, id, message, from, seq)?;
            }
        }
        tx.commit()?;
        Ok(event)
    }

    pub fn chat_submission(
        &self,
        id: SessionId,
        message_id: &str,
        content: &str,
    ) -> Result<Option<bool>> {
        let hash:Option<String>=self.connection.lock().expect("sqlite lock").query_row("SELECT content_hash FROM conversation_submissions WHERE session_id=?1 AND message_id=?2",params![id.to_string(),message_id],|r|r.get(0)).optional()?;
        Ok(hash.map(|hash| hash == content_hash(content)))
    }

    /// Persist the accepted message and idempotency receipt atomically, before
    /// dispatch. An unknown transport outcome is never automatically replayed.
    pub fn submit_chat_message(
        &self,
        id: SessionId,
        message_id: &str,
        content: &str,
    ) -> Result<MessageSubmission> {
        self.submit_chat_message_as(id, message_id, content, false)
    }
    /// `steered` marks a message written into a turn already running, so the
    /// transcript can say it was read mid-turn rather than as a turn of its own.
    pub fn submit_chat_message_as(
        &self,
        id: SessionId,
        message_id: &str,
        content: &str,
        steered: bool,
    ) -> Result<MessageSubmission> {
        let mut conn = self.connection.lock().expect("sqlite lock");
        let tx = conn.transaction()?;
        let hash = content_hash(content);
        let previous:Option<String>=tx.query_row("SELECT content_hash FROM conversation_submissions WHERE session_id=?1 AND message_id=?2",params![id.to_string(),message_id],|r|r.get(0)).optional()?;
        if let Some(previous) = previous {
            return Ok(if previous == hash {
                MessageSubmission::Duplicate
            } else {
                MessageSubmission::Conflict
            });
        }
        tx.execute("INSERT INTO conversation_submissions(session_id,message_id,content_hash) VALUES (?1,?2,?3)",params![id.to_string(),message_id,hash])?;
        let mut message =
            json!({"type":"message","id":message_id,"role":"user","text":content,"delta":false});
        if steered {
            message["steered"] = json!(true);
        }
        let event = append(&tx, id, message)?;
        tx.commit()?;
        Ok(MessageSubmission::New(event))
    }
}

pub(super) fn append(
    tx: &rusqlite::Transaction<'_>,
    id: SessionId,
    mut event: Value,
) -> Result<Value> {
    if !event.is_object() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    tx.execute(
        "INSERT OR IGNORE INTO conversation_meta(session_id) VALUES (?1)",
        [id.to_string()],
    )?;
    let seq: u64 = tx.query_row(
        "SELECT next_seq FROM conversation_meta WHERE session_id=?1",
        [id.to_string()],
        |r| r.get(0),
    )?;
    event["seq"] = json!(seq);
    let encoded = serde_json::to_string(&event).map_err(conversion_error)?;
    if encoded.len() > 256 * 1024 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    tx.execute("INSERT INTO conversation_events(session_id,seq,event_json,byte_count) VALUES (?1,?2,?3,?4)",params![id.to_string(),seq,encoded,encoded.len()])?;
    tx.execute(
        "UPDATE conversation_meta SET next_seq=?1 WHERE session_id=?2",
        params![seq + 1, id.to_string()],
    )?;
    // The display window may rotate while a tool waits for input. Keep a
    // separate bounded set of exact native approval cards and current controls.
    let kind = event["type"].as_str().unwrap_or("");
    // The first thing asked names the session. Only a title this code wrote
    // itself is replaced, so a rename stays put, and only the first message
    // counts, so the name does not follow the conversation around.
    // Every message the user sends is the session's latest word, for lists.
    if kind == "message"
        && event["role"].as_str() == Some("user")
        && let Some(text) = event["text"].as_str()
    {
        let raw: String = text.chars().take(400).collect();
        tx.execute(
            "UPDATE sessions SET preview=?1 WHERE id=?2",
            params![raw, id.to_string()],
        )?;
    }
    if kind == "message"
        && event["role"].as_str() == Some("user")
        && let Some(title) = event["text"].as_str().and_then(derived_title)
    {
        tx.execute(
            "UPDATE sessions SET title=?1,title_source='derived' WHERE id=?2 AND title_source='auto'",
            params![title, id.to_string()],
        )?;
    }
    if matches!(
        kind,
        "ready"
            | "turn"
            | "exit"
            | "configuration"
            | "usage"
            | "settings"
            | "approval"
            | "approval_resolved"
    ) {
        let encoded_anchors: String = tx.query_row(
            "SELECT anchors FROM conversation_meta WHERE session_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        let mut anchors: std::collections::BTreeMap<String, Value> =
            serde_json::from_str(&encoded_anchors).map_err(conversion_error)?;
        if kind == "configuration" {
            anchors.clear();
        }
        if kind == "exit" {
            anchors.retain(|key, _| !key.starts_with("approval:"));
        }
        if kind == "approval_resolved" {
            if let Some(request_id) = event["id"].as_str() {
                anchors.remove(&format!("approval:{request_id}"));
            }
        } else if kind == "approval" {
            let request_id = event["id"].as_str().ok_or(rusqlite::Error::InvalidQuery)?;
            anchors.insert(format!("approval:{request_id}"), event.clone());
            if anchors
                .keys()
                .filter(|key| key.starts_with("approval:"))
                .count()
                > 32
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
        } else if kind == "settings" {
            // The model and permission chips are built from these, and a
            // long session trims them away like any other event. The list of
            // models and the current choice can arrive separately, so the
            // last of each is kept, as pruneChatEvents does in the browser.
            let lists_models = event["models"]
                .as_array()
                .is_some_and(|models| !models.is_empty());
            let key = if lists_models {
                "settings:models"
            } else {
                "settings"
            };
            anchors.insert(key.into(), event.clone());
        } else {
            anchors.insert(kind.into(), event.clone());
        }
        tx.execute(
            "UPDATE conversation_meta SET anchors=?1 WHERE session_id=?2",
            params![
                serde_json::to_string(&anchors).map_err(conversion_error)?,
                id.to_string()
            ],
        )?;
    }
    Ok(event)
}

/// The deltas a message streamed, once the message is whole: the first stays,
/// emptied, so the message keeps the place it began in among the tool calls
/// made while it streamed; the rest are deleted, since the final message
/// carries the text. Nine in ten stored events were deltas.
fn compact_deltas(
    tx: &rusqlite::Transaction<'_>,
    id: SessionId,
    message: &str,
    from: u64,
    before: u64,
) -> Result<()> {
    let session = id.to_string();
    let rows: Vec<(u64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT seq,event_json FROM conversation_events WHERE session_id=?1 AND seq>=?2 AND seq<?3 \
             AND json_extract(event_json,'$.type')='message' AND json_extract(event_json,'$.delta')=1 \
             AND json_extract(event_json,'$.id')=?4 ORDER BY seq",
        )?;
        stmt.query_map(params![session, from, before, message], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<Result<_>>()?
    };
    let Some(((first, json), rest)) = rows.split_first() else {
        return Ok(());
    };
    let mut place: Value = serde_json::from_str(json).map_err(conversion_error)?;
    place["text"] = json!("");
    let encoded = serde_json::to_string(&place).map_err(conversion_error)?;
    tx.execute(
        "UPDATE conversation_events SET event_json=?1,byte_count=?2 WHERE session_id=?3 AND seq=?4",
        params![encoded, encoded.len(), session, first],
    )?;
    if let Some((last, _)) = rest.last() {
        let mut stmt = tx.prepare(
            "DELETE FROM conversation_events WHERE session_id=?1 AND seq>?2 AND seq<=?3 \
             AND json_extract(event_json,'$.type')='message' AND json_extract(event_json,'$.delta')=1 \
             AND json_extract(event_json,'$.id')=?4",
        )?;
        stmt.execute(params![session, first, last, message])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_preview_is_what_was_said_without_the_attachment_header() {
        assert_eq!(
            preview_line(
                "Attached files in this workspace:\n- a.png\n- b.png\n\n  look at\n these "
            )
            .as_deref(),
            Some("look at these")
        );
        assert_eq!(
            preview_line("Attached file in this workspace:\n- a.png"),
            None
        );
        assert_eq!(preview_line(&"字".repeat(90)).unwrap().chars().count(), 81);
    }

    #[test]
    fn a_title_is_what_was_said_without_the_attachment_header() {
        assert_eq!(
            derived_title(
                "Attached file in this workspace:\n- .tendly-files/shot.png\n\n这里ui也有问题"
            )
            .as_deref(),
            Some("这里ui也有问题")
        );
        assert_eq!(
            derived_title("Attached files in this workspace:\n- a.png\n- b.png"),
            None
        );
    }

    #[test]
    fn the_first_message_after_an_endpoint_change_carries_what_was_said_before_it() {
        let path = std::env::temp_dir().join(format!("tendly-carried-{}.db", Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        let workspace = store.create_workspace("Fixture", "/fixture").unwrap();
        let id = store
            .create_session(workspace.id, ProviderKind::ClaudeCode, "Claude session")
            .unwrap()
            .id;
        let append = |event: Value| store.append_conversation_event(id, event).unwrap();
        append(json!({"type":"message","id":"old","role":"user","text":"cleared away"}));
        append(json!({"type":"cleared"}));
        // No boundary yet: the native context is the one that heard all this.
        assert_eq!(store.carried_transcript(id, 4096).unwrap(), None);
        append(json!({"type":"message","id":"u","role":"user","text":"fix the retry backoff"}));
        append(json!({"type":"message","id":"a","role":"assistant","text":"Look","delta":true}));
        append(json!({"type":"message","id":"a","role":"assistant","text":"ing","delta":true}));
        append(json!({"type":"tool","id":"t","name":"Bash","status":"completed","text":"ls"}));
        append(
            json!({"type":"message","id":"a","role":"assistant","text":"It is 50 ms in src/net.rs."}),
        );
        assert!(store.switch_session_configuration(id, None, None).unwrap());

        let carried = store.carried_transcript(id, 4096).unwrap().unwrap();
        assert!(
            carried
                .contains("User:\nfix the retry backoff\n\nAssistant:\nIt is 50 ms in src/net.rs.")
        );
        assert!(
            !carried.contains("cleared away"),
            "a /clear ends what is carried"
        );
        assert!(!carried.contains("Bash") && !carried.contains("Looking"));
        // Only the latest turns that fit, and saying so.
        let short = store.carried_transcript(id, 40).unwrap().unwrap();
        assert!(short.contains("left out") && !short.contains("fix the retry"));

        // A second change carries both contexts, even where IDs repeat.
        append(json!({"type":"message","id":"u","role":"user","text":"now add a test"}));
        assert_eq!(
            store.carried_transcript(id, 4096).unwrap(),
            None,
            "the new context has been spoken to"
        );
        append(json!({"type":"message","id":"a","role":"assistant","text":"Added."}));
        assert!(store.switch_session_configuration(id, None, None).unwrap());
        let both = store.carried_transcript(id, 4096).unwrap().unwrap();
        assert!(
            both.contains("It is 50 ms") && both.contains("now add a test\n\nAssistant:\nAdded.")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn every_message_the_user_sends_becomes_the_sessions_preview() {
        let path = std::env::temp_dir().join(format!("tendly-preview-{}.db", Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        let workspace = store.create_workspace("Fixture", "/fixture").unwrap();
        let session = store
            .create_session(workspace.id, ProviderKind::ClaudeCode, "Claude Code")
            .unwrap();
        assert_eq!(
            store.get_session(session.id).unwrap().unwrap().preview,
            None
        );
        for text in [
            "first question",
            "Attached file in this workspace:\n- shot.png\n\nwhy is this red",
        ] {
            store
                .append_conversation_event(
                    session.id,
                    json!({"type":"message","id":text,"role":"user","text":text}),
                )
                .unwrap();
        }
        // The agent's reply does not move it.
        store
            .append_conversation_event(
                session.id,
                json!({"type":"message","id":"a","role":"assistant","text":"because"}),
            )
            .unwrap();
        assert_eq!(
            store
                .get_session(session.id)
                .unwrap()
                .unwrap()
                .preview
                .as_deref(),
            Some("why is this red")
        );
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn transcript_receipts_and_sequence_survive_reopen_and_restart_does_not_replay_messages() {
        let path = std::env::temp_dir().join(format!("tendly-conversation-{}.db", Uuid::new_v4()));
        let id;
        {
            let store = Store::open(&path).unwrap();
            let workspace = store.create_workspace("Fixture", "/fixture").unwrap();
            let session = store
                .create_session(workspace.id, ProviderKind::Codex, "Chat")
                .unwrap();
            id = session.id;
            assert_eq!(session.interaction_mode, InteractionMode::Pty);
            assert!(
                store
                    .set_interaction_mode(id, InteractionMode::Structured)
                    .unwrap()
            );
            assert!(matches!(
                store
                    .submit_chat_message(id, "message-1", "literal $PATH")
                    .unwrap(),
                MessageSubmission::New(_)
            ));
            store
                .append_conversation_event(id, json!({"type":"turn","status":"running"}))
                .unwrap();
            store.append_conversation_event(id,json!({"type":"approval","id":"pending","title":"Fixture","text":"Fixture","choices":["accept","decline"]})).unwrap();
            store
                .set_session_status(id, SessionStatus::Running)
                .unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            assert!(matches!(
                store
                    .submit_chat_message(id, "message-1", "literal $PATH")
                    .unwrap(),
                MessageSubmission::Duplicate
            ));
            assert!(matches!(
                store
                    .submit_chat_message(id, "message-1", "changed")
                    .unwrap(),
                MessageSubmission::Conflict
            ));
            store.reconcile_after_restart().unwrap();
            let events = store.conversation(id).unwrap().events;
            assert_eq!(events.len(), 4);
            assert_eq!(events[3]["type"], "exit");
            assert_eq!(events[3]["seq"], 4);
            store.reconcile_after_restart().unwrap();
            assert_eq!(store.conversation(id).unwrap().events.len(), 4);
            assert!(matches!(
                store.get_session(id).unwrap().unwrap().status,
                SessionStatus::Stopped
            ));
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn model_and_permission_settings_ride_on_the_latest_page() {
        let store = Store::open(":memory:").unwrap();
        let workspace = store.create_workspace("Fixture", "/fixture").unwrap();
        let id = store
            .create_session(workspace.id, ProviderKind::Codex, "Chat")
            .unwrap()
            .id;
        store
            .append_conversation_event(
                id,
                json!({"type":"settings","model":"a","models":[{"id":"a"},{"id":"b"}],"permission_modes":["ask","danger"]}),
            )
            .unwrap();
        store
            .append_conversation_event(id, json!({"type":"settings","model":"b"}))
            .unwrap();
        for _ in 0..2001 {
            store
                .append_conversation_event(id, json!({"type":"delta","text":"x"}))
                .unwrap();
        }
        let conversation = store.conversation_page(id, None, 100).unwrap();
        assert_eq!(conversation.older, Some(1904));
        let settings: Vec<_> = conversation
            .events
            .iter()
            .filter(|event| event["type"] == "settings")
            .collect();
        assert_eq!(settings.len(), 2, "both the list and the choice are kept");
        assert_eq!(settings[0]["models"].as_array().unwrap().len(), 2);
        assert_eq!(settings[1]["model"], "b", "in their original order");
    }
    fn chat(store: &Store) -> SessionId {
        let workspace = store.create_workspace("Fixture", "/fixture").unwrap();
        store
            .create_session(workspace.id, ProviderKind::Codex, "Chat")
            .unwrap()
            .id
    }
    fn seqs(events: &[Value]) -> Vec<u64> {
        events.iter().map(|e| e["seq"].as_u64().unwrap()).collect()
    }

    /// History is kept in full; sequences never reset, and an event too large
    /// to keep is refused without taking a sequence.
    #[test]
    fn history_is_kept_in_full_and_sequences_never_reset() {
        let store = Store::open(":memory:").unwrap();
        let id = chat(&store);
        for _ in 0..5000 {
            store
                .append_conversation_event(id, json!({"type":"ready"}))
                .unwrap();
        }
        let transcript = store.conversation(id).unwrap();
        assert!(!transcript.truncated);
        assert_eq!(transcript.events.len(), 5000);
        assert_eq!(transcript.events[0]["seq"], 1);
        assert!(
            store
                .append_conversation_event(
                    id,
                    json!({"type":"error","message":"x".repeat(256*1024)})
                )
                .is_err()
        );
        let next = store
            .append_conversation_event(id, json!({"type":"ready"}))
            .unwrap();
        assert_eq!(next["seq"], 5001);
    }

    /// A finished message keeps its place but not its pieces: the first delta
    /// stays, emptied, where the message began, the rest go, and the final
    /// message carries the text. Everything else is kept as it was.
    #[test]
    fn a_finished_messages_deltas_shrink_to_its_place() {
        let store = Store::open(":memory:").unwrap();
        let id = chat(&store);
        let append = |event: Value| store.append_conversation_event(id, event).unwrap();
        append(json!({"type":"message","id":"u","role":"user","text":"hi"}));
        for piece in ["Hel", "lo ", "the"] {
            append(json!({"type":"message","id":"a","role":"assistant","text":piece,"delta":true}));
        }
        // A tool card emitted while the text was still streaming.
        append(json!({"type":"tool","id":"t","name":"Bash","status":"running","text":"ls"}));
        append(json!({"type":"message","id":"a","role":"assistant","text":"re","delta":true}));
        // Another message's delta is left alone.
        append(json!({"type":"message","id":"b","role":"assistant","text":"other","delta":true}));
        append(
            json!({"type":"message","id":"a","role":"assistant","text":"Hello there","delta":false}),
        );
        let events = store.conversation(id).unwrap().events;
        assert_eq!(seqs(&events), [1, 2, 5, 7, 8]);
        assert_eq!(
            events[1],
            json!({"type":"message","id":"a","role":"assistant","text":"","delta":true,"seq":2})
        );
        assert_eq!(events[2]["type"], "tool");
        assert_eq!(events[3]["text"], "other");
        assert_eq!(events[4]["text"], "Hello there");
        // A message that never streamed is stored as it came.
        append(json!({"type":"message","id":"c","role":"assistant","text":"whole"}));
        assert_eq!(store.conversation(id).unwrap().events.len(), 6);
    }

    /// Search reads the whole history: what was said, and what tools were
    /// called and answered, newest first, once per message or call; never a
    /// streamed piece, and the query is text, not a pattern.
    #[test]
    fn search_finds_messages_and_calls_across_the_whole_history() {
        let store = Store::open(":memory:").unwrap();
        let id = chat(&store);
        let append = |event: Value| store.append_conversation_event(id, event).unwrap();
        append(
            json!({"type":"message","id":"u1","role":"user","text":"Where is the Retry backoff set?"}),
        );
        for _ in 0..1500 {
            append(
                json!({"type":"tool","id":"noise","name":"Bash","status":"completed","text":"nothing here"}),
            );
        }
        append(
            json!({"type":"tool","id":"t1","name":"Grep","status":"running","text":"{\"pattern\":\"retry\"}"}),
        );
        append(
            json!({"type":"tool","id":"t1","name":"Grep","status":"completed","text":"src/net.rs:12: // Retry backoff, in ms"}),
        );
        append(
            json!({"type":"message","id":"a1","role":"assistant","text":"The retr","delta":true}),
        );
        append(
            json!({"type":"message","id":"a1","role":"assistant","text":"The retry backoff is 50 ms, in src/net.rs."}),
        );
        append(
            json!({"type":"message","id":"zh","role":"assistant","text":"重试间隔在网络模块里设置。"}),
        );
        append(json!({"type":"message","id":"pct","role":"user","text":"100% done_now"}));
        let hits = store.search_conversation(id, "retry backoff", 50).unwrap();
        // Newest first; the call once, though two of its events match.
        let found: Vec<(&str, &str)> = hits
            .iter()
            .map(|h| (h.kind.as_str(), h.id.as_str()))
            .collect();
        assert_eq!(
            found,
            [("message", "a1"), ("tool", "t1"), ("message", "u1")]
        );
        assert!(hits[0].snippet.contains("retry backoff is 50 ms"));
        assert!(hits.iter().all(|h| h.seq > 0));
        // Chinese, and text that looks like a pattern, are matched as written.
        assert_eq!(
            store.search_conversation(id, "重试间隔", 50).unwrap()[0].id,
            "zh"
        );
        assert_eq!(store.search_conversation(id, "100%", 50).unwrap().len(), 1);
        assert!(store.search_conversation(id, "done_", 50).unwrap().len() == 1);
        assert!(store.search_conversation(id, "0%d", 50).unwrap().is_empty());
        assert!(store.search_conversation(id, "   ", 50).unwrap().is_empty());
        assert_eq!(store.search_conversation(id, "e", 2).unwrap().len(), 2);
    }

    /// A snippet is the match with a little on either side, cut on characters.
    #[test]
    fn a_snippet_shows_the_match_in_its_context() {
        let text = format!("{}needle{}", "前".repeat(100), "后".repeat(100));
        let snippet = snippet_of(&text, "needle");
        assert!(snippet.starts_with('…') && snippet.ends_with('…'));
        assert!(snippet.contains("needle"));
        assert!(snippet.chars().count() <= 100);
        assert_eq!(
            snippet_of("short needle text", "NEEDLE"),
            "short needle text"
        );
    }

    /// A message that was streaming when the server restarted is compacted
    /// all the same once it finishes: its deltas are found among the latest rows.
    #[test]
    fn a_message_finished_after_a_restart_is_compacted_too() {
        let path =
            std::env::temp_dir().join(format!("tendly-restart-deltas-{}.db", Uuid::new_v4()));
        let id = {
            let store = Store::open(&path).unwrap();
            let id = chat(&store);
            for piece in ["a", "b", "c"] {
                store
                    .append_conversation_event(id, json!({"type":"message","id":"m","role":"assistant","text":piece,"delta":true}))
                    .unwrap();
            }
            id
        };
        let store = Store::open(&path).unwrap();
        store
            .append_conversation_event(
                id,
                json!({"type":"message","id":"m","role":"assistant","text":"abc"}),
            )
            .unwrap();
        let events = store.conversation(id).unwrap().events;
        assert_eq!(seqs(&events), [1, 4]);
        assert_eq!(events[0]["text"], "");
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    /// The latest page, then older pages by cursor, cover the history once;
    /// the latest page carries the state its window would otherwise miss.
    #[test]
    fn history_reads_in_pages_with_the_current_state_on_the_latest() {
        let store = Store::open(":memory:").unwrap();
        let id = chat(&store);
        store
            .append_conversation_event(id, json!({"type":"ready"}))
            .unwrap();
        store.append_conversation_event(id,json!({"type":"approval","id":"pending","title":"Native approval","text":"Exact fixture request","choices":["accept","decline"]})).unwrap();
        for n in 0..250 {
            store
                .append_conversation_event(
                    id,
                    json!({"type":"tool","id":format!("t{n}"),"name":"Bash","status":"completed"}),
                )
                .unwrap();
        }
        let latest = store.conversation_page(id, None, 100).unwrap();
        // The last 100, with the ready and the pending approval ahead of them.
        assert_eq!(latest.events.len(), 102);
        assert_eq!(latest.events[0]["type"], "ready");
        assert_eq!(latest.events[1]["id"], "pending");
        assert_eq!(seqs(&latest.events[2..]), (153..=252).collect::<Vec<_>>());
        assert_eq!(latest.older, Some(153));
        let middle = store.conversation_page(id, latest.older, 100).unwrap();
        assert_eq!(seqs(&middle.events), (53..=152).collect::<Vec<_>>());
        assert_eq!(middle.older, Some(53));
        let first = store.conversation_page(id, middle.older, 100).unwrap();
        assert_eq!(seqs(&first.events), (1..=52).collect::<Vec<_>>());
        assert_eq!(first.older, None);
        // A resolved approval is no longer carried.
        store
            .append_conversation_event(id, json!({"type":"approval_resolved","id":"pending"}))
            .unwrap();
        let latest = store.conversation_page(id, None, 100).unwrap();
        assert!(!latest.events.iter().any(|e| e["type"] == "approval"));
        // What follows a view's last event, unless there is more than it can take.
        let after = store.conversation_after(id, 250, 10).unwrap().unwrap();
        assert_eq!(seqs(&after), [251, 252, 253]);
        assert_eq!(store.conversation_after(id, 100, 10).unwrap(), None);
        assert_eq!(store.conversation_after(id, 253, 10).unwrap(), Some(vec![]));
        assert_eq!(store.conversation_after(id, 254, 10).unwrap(), None);
    }

    #[test]
    fn pending_approvals_and_receipts() {
        let store = Store::open(":memory:").unwrap();
        let id = chat(&store);
        let prompt = "prompt never duplicated into a receipt";
        store.submit_chat_message(id, "receipt", prompt).unwrap();
        let hash: String = store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT content_hash FROM conversation_submissions WHERE message_id='receipt'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hash.len(), 64);
        assert!(!hash.contains(prompt));
        assert_eq!(
            store.chat_submission(id, "receipt", prompt).unwrap(),
            Some(true)
        );
    }

    #[test]
    fn the_first_message_names_the_session_and_a_rename_is_never_overwritten() {
        let path = std::env::temp_dir().join(format!("tendly-title-{}.db", Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        let workspace = store.create_workspace("Fixture", "/fixture").unwrap();
        let user = |text: &str| json!({"type":"message","role":"user","text":text});
        let named = |store: &Store, id| store.get_session(id).unwrap().unwrap().title;

        let first = store
            .create_session(workspace.id, ProviderKind::Codex, "Codex session")
            .unwrap()
            .id;
        // A leading slash command or file mention describes no session, so the
        // first legible line is what names it.
        store
            .append_conversation_event(first, user("/model"))
            .unwrap();
        assert_eq!(named(&store, first), "Codex session");
        store
            .append_conversation_event(first, user("@notes.md\nfix the retry backoff"))
            .unwrap();
        assert_eq!(named(&store, first), "fix the retry backoff");
        // Later messages are the conversation moving on, not a new name.
        store
            .append_conversation_event(first, user("and add a test"))
            .unwrap();
        assert_eq!(named(&store, first), "fix the retry backoff");

        // A name typed by hand outranks anything derived, before or after.
        let second = store
            .create_session(workspace.id, ProviderKind::ClaudeCode, "Claude session")
            .unwrap()
            .id;
        store
            .update_session_title(second, "Release checklist")
            .unwrap();
        store
            .append_conversation_event(second, user("look at the logs"))
            .unwrap();
        assert_eq!(named(&store, second), "Release checklist");

        // Long titles are cut by characters; a byte limit would cut CJK to a third.
        let third = store
            .create_session(workspace.id, ProviderKind::Codex, "Codex session")
            .unwrap()
            .id;
        store
            .append_conversation_event(third, user(&"\u{6d4b}".repeat(80)))
            .unwrap();
        let title = named(&store, third);
        assert_eq!(title.chars().count(), 61);
        assert!(title.ends_with('\u{2026}'));

        // Whitespace alone leaves the created-at name rather than a blank row.
        let fourth = store
            .create_session(workspace.id, ProviderKind::Codex, "Codex session")
            .unwrap()
            .id;
        store
            .append_conversation_event(fourth, user("   \n  "))
            .unwrap();
        assert_eq!(named(&store, fourth), "Codex session");
        let _ = std::fs::remove_file(&path);
    }
}
