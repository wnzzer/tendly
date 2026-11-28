//! What agy's conversations spent, read from its own databases.
//!
//! agy keeps each conversation in `antigravity-cli/conversations/<id>.db`, a
//! SQLite database whose rows hold protocol buffers it publishes no schema
//! for. The fields read here were matched against the totals `agy acp`
//! reports for the same turns:
//!
//! - `steps`: one row per step, in order. A step of type 14 is a message the
//!   person sent; type 15 is a model reply, whose `metadata` holds when it
//!   began (`1.1`, seconds) and ended (`8.1`), its tokens (`9`: `2` input
//!   not from the cache, `5` read from the cache, `3` output, `9` thinking)
//!   and which generation made it (`20.3`, absent for the first).
//! - `gen_metadata`: one row per generation, by that index, naming the model
//!   (`1.21` as agy shows it, `1.19` as it calls it, `1.4.1` as a number)
//!   and how long the call took (`1.11`, seconds and nanoseconds).
//! - `trajectory_metadata_blob`: the conversation's workspace (`1.1`).
//!
//! Anything that does not read as expected is skipped, never guessed.
use crate::usage::{Call, FileState, Outcome, Source, Tokens};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tendly_domain::ProviderKind;

const USER_STEP: i64 = 14;
const MODEL_STEP: i64 = 15;
/// A conversation larger than this is not read: it would be read whole on
/// every change.
const MAX_DATABASE: u64 = 256 * 1024 * 1024;

// --- protocol buffer wire format ---------------------------------------------

/// One field of a message: a number, or the bytes of a nested message,
/// string or packed list.
enum Wire<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}

fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

/// The fields of a message in order, stopping at the first that does not
/// parse.
fn fields(bytes: &[u8]) -> impl Iterator<Item = (u64, Wire<'_>)> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        if at >= bytes.len() {
            return None;
        }
        let key = varint(bytes, &mut at)?;
        let value = match key & 7 {
            0 => Wire::Number(varint(bytes, &mut at)?),
            1 => {
                at = at.checked_add(8).filter(|end| *end <= bytes.len())?;
                Wire::Number(0)
            }
            2 => {
                let len = usize::try_from(varint(bytes, &mut at)?).ok()?;
                let end = at.checked_add(len).filter(|end| *end <= bytes.len())?;
                let value = &bytes[at..end];
                at = end;
                Wire::Bytes(value)
            }
            5 => {
                at = at.checked_add(4).filter(|end| *end <= bytes.len())?;
                Wire::Number(0)
            }
            _ => return None,
        };
        Some((key >> 3, value))
    })
}

/// The first nested message at `path`.
fn message<'a>(bytes: &'a [u8], path: &[u64]) -> Option<&'a [u8]> {
    path.iter().try_fold(bytes, |bytes, field| {
        fields(bytes).find_map(|(number, value)| match value {
            Wire::Bytes(inner) if number == *field => Some(inner),
            _ => None,
        })
    })
}
fn number(bytes: &[u8], path: &[u64]) -> Option<u64> {
    let (last, parents) = path.split_last()?;
    fields(message(bytes, parents)?).find_map(|(field, value)| match value {
        Wire::Number(n) if field == *last => Some(n),
        _ => None,
    })
}
fn text<'a>(bytes: &'a [u8], path: &[u64]) -> Option<&'a str> {
    std::str::from_utf8(message(bytes, path)?)
        .ok()
        .filter(|text| !text.is_empty())
}

// --- naming the model --------------------------------------------------------

/// The name a model's price is listed under, from how agy shows it: "Gemini
/// 4 Neon (High)" is `gemini-4-neon`, "Claude Opus 5.5 (Low)" is
/// `claude-opus-5-5`, as Anthropic spells its models.
fn shown_model(shown: &str) -> Option<String> {
    let name = shown
        .rsplit_once(" (")
        .map_or(shown, |(model, _)| model)
        .trim();
    if name.is_empty() {
        return None;
    }
    let mut id = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    if id.starts_with("claude-") {
        id = id.replace('.', "-");
    }
    Some(id)
}

/// The same from what agy calls a model: `gemini-3.8-flash-n` and
/// `gemini-4-argon-trial` are its builds of `gemini-3.8-flash` and
/// `gemini-4-argon`, and `claude-sonnet-5-5-low` is Claude Sonnet 5.5 at low.
fn called_model(called: &str) -> String {
    let mut id = called.to_ascii_lowercase();
    for suffix in [
        "-n", "-trial", "-low", "-medium", "-high", "-xhigh", "-tiered",
    ] {
        if let Some(rest) = id.strip_suffix(suffix) {
            id = rest.to_owned();
        }
    }
    id
}

/// The model each of agy's model numbers turned out to be, wherever it was
/// named: a generation that names none is known by its number alone.
fn numbered() -> &'static Mutex<HashMap<u64, Arc<str>>> {
    static NUMBERED: OnceLock<Mutex<HashMap<u64, Arc<str>>>> = OnceLock::new();
    NUMBERED.get_or_init(Default::default)
}

/// A generation's model and how long its call took.
struct Generation {
    model: Option<Arc<str>>,
    number: Option<u64>,
    duration_ms: Option<u64>,
}

fn generation(data: &[u8]) -> Option<Generation> {
    let record = message(data, &[1])?;
    let model_number = number(record, &[4, 1]);
    let model = text(record, &[21])
        .and_then(shown_model)
        .or_else(|| text(record, &[19]).map(called_model))
        .map(Arc::<str>::from);
    if let (Some(model), Some(known)) = (&model, model_number) {
        numbered()
            .lock()
            .expect("agy model numbers")
            .insert(known, model.clone());
    }
    let duration_ms = number(record, &[11, 1])
        .map(|seconds| seconds * 1000 + number(record, &[11, 2]).unwrap_or(0) / 1_000_000);
    Some(Generation {
        model,
        number: model_number,
        duration_ms,
    })
}

// --- reading a conversation --------------------------------------------------

/// Every message and model call in one of agy's conversations.
pub fn read(path: &Path, source: &Source, state: &mut FileState) {
    if std::fs::metadata(path).map_or(true, |m| m.len() > MAX_DATABASE) {
        return;
    }
    let Some(conversation) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return;
    };
    let Ok(database) = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return;
    };
    let _ = database.busy_timeout(Duration::from_millis(250));
    let _ = read_database(&database, conversation, source, state);
}

fn read_database(
    database: &Connection,
    conversation: &str,
    source: &Source,
    state: &mut FileState,
) -> rusqlite::Result<()> {
    let cwd: Arc<str> = database
        .query_row(
            "SELECT data FROM trajectory_metadata_blob LIMIT 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .ok()
        .and_then(|data| {
            let uri = text(&data, &[1, 1])?.to_owned();
            url::Url::parse(&uri).ok()?.to_file_path().ok()
        })
        .map(|path| Arc::from(path.to_string_lossy().as_ref()))
        .unwrap_or_else(crate::usage::empty);
    let mut generations: HashMap<i64, Generation> = HashMap::new();
    let mut rows = database.prepare("SELECT idx, data FROM gen_metadata")?;
    for row in rows.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<Vec<u8>>>(1)?))
    })? {
        let (index, data) = row?;
        if let Some(generation) = data.as_deref().and_then(generation) {
            generations.insert(index, generation);
        }
    }
    let conversation: Arc<str> = Arc::from(conversation);
    let mut steps = database.prepare(
        "SELECT idx, step_type, metadata, length(error_details) FROM steps \
         WHERE step_type IN (?1, ?2) ORDER BY idx",
    )?;
    for step in steps.query_map([USER_STEP, MODEL_STEP], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<Vec<u8>>>(2)?,
            row.get::<_, Option<i64>>(3)?,
        ))
    })? {
        let (index, kind, metadata, error) = step?;
        let Some(metadata) = metadata else { continue };
        let Some(at) = number(&metadata, &[1, 1]).and_then(|at| i64::try_from(at).ok()) else {
            continue;
        };
        let call = |model: Arc<str>, tokens: Tokens, user: bool| Call {
            at,
            provider: ProviderKind::Gemini,
            model,
            conversation: conversation.clone(),
            cwd: cwd.clone(),
            home_session: source.home_session.clone(),
            dedupe: Some(Arc::from(format!("agy:{conversation}:{index}"))),
            tokens,
            user,
            duration_ms: None,
            outcome: Outcome::Unlogged,
        };
        if kind == USER_STEP {
            state
                .calls
                .push(call(crate::usage::empty(), Tokens::default(), true));
            continue;
        }
        let Some(usage) = message(&metadata, &[9]) else {
            continue;
        };
        let count = |field: u64| number(usage, &[field]).unwrap_or(0);
        let thinking = count(9);
        let tokens = Tokens {
            input: count(2),
            cache_read: count(5),
            cache_write_5m: 0,
            cache_write_1h: 0,
            output: count(3) + thinking,
            reasoning: thinking,
        };
        if tokens.total() == 0 {
            continue;
        }
        let made_by = number(&metadata, &[20, 3])
            .and_then(|n| i64::try_from(n).ok())
            .unwrap_or(0);
        let generation = generations.get(&made_by);
        let model = generation
            .and_then(|g| g.model.clone())
            .or_else(|| {
                let number = generation
                    .and_then(|g| g.number)
                    .or_else(|| number(usage, &[1]))?;
                numbered()
                    .lock()
                    .expect("agy model numbers")
                    .get(&number)
                    .cloned()
            })
            .unwrap_or_else(|| Arc::from("gemini"));
        let mut call = call(model, tokens, false);
        call.outcome = if error.unwrap_or(0) > 0 {
            Outcome::Failed
        } else {
            Outcome::Answered
        };
        if call.outcome == Outcome::Answered {
            call.duration_ms = generation.and_then(|g| g.duration_ms).or_else(|| {
                let ended = i64::try_from(number(&metadata, &[8, 1])?).ok()?;
                u64::try_from(ended - at).ok().map(|seconds| seconds * 1000)
            });
        }
        state.calls.push(call);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field as the wire writes it.
    fn put(out: &mut Vec<u8>, field: u64, value: &Value) {
        let varint = |out: &mut Vec<u8>, mut n: u64| loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        };
        match value {
            Value::Number(n) => {
                varint(out, field << 3);
                varint(out, *n);
            }
            Value::Bytes(bytes) => {
                varint(out, (field << 3) | 2);
                varint(out, bytes.len() as u64);
                out.extend_from_slice(bytes);
            }
        }
    }
    enum Value {
        Number(u64),
        Bytes(Vec<u8>),
    }
    fn msg(entries: &[(u64, Value)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (field, value) in entries {
            put(&mut out, *field, value);
        }
        out
    }
    use Value::{Bytes as B, Number as N};

    #[test]
    fn nested_fields_are_read_by_path() {
        let usage = msg(&[(2, N(13336)), (3, N(217)), (9, N(169))]);
        let meta = msg(&[
            (1, B(msg(&[(1, N(1_795_607_100))]))),
            (9, B(usage)),
            (20, B(msg(&[(1, B(b"x".to_vec())), (3, N(2))]))),
        ]);
        assert_eq!(number(&meta, &[1, 1]), Some(1_795_607_100));
        assert_eq!(number(&meta, &[9, 2]), Some(13336));
        assert_eq!(number(&meta, &[20, 3]), Some(2));
        assert_eq!(number(&meta, &[9, 5]), None);
        assert_eq!(text(&meta, &[20, 1]), Some("x"));
        // A truncated message reads as far as it goes.
        assert_eq!(number(&meta[..3], &[9, 2]), None);
    }

    #[test]
    fn models_are_named_as_their_prices_are_listed() {
        assert_eq!(
            shown_model("Gemini 4 Neon (High)").as_deref(),
            Some("gemini-4-neon")
        );
        assert_eq!(
            shown_model("Claude Opus 5.5 (Low)").as_deref(),
            Some("claude-opus-5-5")
        );
        assert_eq!(
            shown_model("GPT-OSS 120B (Medium)").as_deref(),
            Some("gpt-oss-120b")
        );
        assert_eq!(called_model("gemini-3.8-flash-n"), "gemini-3.8-flash");
        assert_eq!(called_model("gemini-4-argon-trial"), "gemini-4-argon");
        assert_eq!(called_model("claude-sonnet-5-5-low"), "claude-sonnet-5-5");
        assert_eq!(called_model("gemini-pro-default"), "gemini-pro-default");
    }

    #[test]
    fn a_conversation_database_gives_its_messages_and_calls() {
        let dir = std::env::temp_dir().join(format!("tendly-agy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("conv-1.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE steps (idx integer primary key, step_type integer, metadata blob, error_details blob);
             CREATE TABLE gen_metadata (idx integer primary key, data blob);
             CREATE TABLE trajectory_metadata_blob (id text primary key, data blob);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO trajectory_metadata_blob VALUES ('main', ?1)",
            [msg(&[(1, B(msg(&[(1, B(b"file:///work".to_vec()))])))])],
        )
        .unwrap();
        let generation = |shown: Option<&str>, number: u64| {
            let mut record = vec![
                (4, B(msg(&[(1, N(number))]))),
                (11, B(msg(&[(1, N(5)), (2, N(500_000_000))]))),
            ];
            if let Some(shown) = shown {
                record.push((21, B(shown.as_bytes().to_vec())));
            }
            msg(&[(1, B(msg(&record)))])
        };
        db.execute(
            "INSERT INTO gen_metadata VALUES (0, ?1)",
            [generation(Some("Gemini 4 Neon (High)"), 1532)],
        )
        .unwrap();
        db.execute(
            "INSERT INTO gen_metadata VALUES (1, ?1)",
            [generation(None, 1532)],
        )
        .unwrap();
        let at = |seconds: u64| msg(&[(1, N(seconds))]);
        let reply = |start: u64, made_by: Option<u64>, input: u64| {
            let mut meta = vec![
                (1, B(at(start))),
                (8, B(at(start + 6))),
                (
                    9,
                    B(msg(&[
                        (1, N(1532)),
                        (2, N(input)),
                        (3, N(217)),
                        (5, N(11279)),
                        (9, N(169)),
                    ])),
                ),
            ];
            if let Some(made_by) = made_by {
                meta.push((20, B(msg(&[(3, N(made_by))]))));
            }
            msg(&meta)
        };
        db.execute(
            "INSERT INTO steps VALUES (0, 14, ?1, NULL)",
            [msg(&[(1, B(at(1_795_607_100)))])],
        )
        .unwrap();
        db.execute(
            "INSERT INTO steps VALUES (1, 15, ?1, NULL)",
            [reply(1_795_607_100, None, 13336)],
        )
        .unwrap();
        db.execute("INSERT INTO steps VALUES (2, 132, ?1, NULL)", [msg(&[])])
            .unwrap();
        db.execute(
            "INSERT INTO steps VALUES (3, 15, ?1, x'0a01')",
            [reply(1_795_607_200, Some(1), 2357)],
        )
        .unwrap();
        drop(db);

        let source = crate::usage::test_source(ProviderKind::Gemini);
        let mut state = crate::usage::test_state();
        read(&path, &source, &mut state);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(state.calls.len(), 3);
        assert!(state.calls[0].user && &*state.calls[0].conversation == "conv-1");
        let first = &state.calls[1];
        assert_eq!((&*first.model, first.at), ("gemini-4-neon", 1_795_607_100));
        // `file:///work` names a path only where paths need no drive.
        if cfg!(unix) {
            assert_eq!(&*first.cwd, "/work");
        }
        assert_eq!(
            (
                first.tokens.input,
                first.tokens.cache_read,
                first.tokens.output,
                first.tokens.reasoning
            ),
            (13336, 11279, 386, 169)
        );
        assert_eq!(
            (first.outcome, first.duration_ms),
            (Outcome::Answered, Some(5500))
        );
        // A generation that names no model is known by its number.
        let second = &state.calls[2];
        assert_eq!(
            (&*second.model, second.outcome),
            ("gemini-4-neon", Outcome::Failed)
        );
        assert_ne!(first.dedupe, second.dedupe);
    }
}
