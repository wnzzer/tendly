//! Finding the clients' transcripts and reading them, each file only from
//! where it was last read, into de-duplicated calls.

use super::*;

/// One model call, with its tokens split the way prices are.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub(crate) at: i64,
    pub(crate) provider: ProviderKind,
    pub(crate) model: Arc<str>,
    /// The client's own conversation id.
    pub(crate) conversation: Arc<str>,
    pub(crate) cwd: Arc<str>,
    /// Set when the file sits in a Tendly session's own client home.
    pub(crate) home_session: Option<Arc<str>>,
    /// Claude logs a reply more than once; one key per real call.
    pub(crate) dedupe: Option<Arc<str>>,
    pub(crate) tokens: Tokens,
    /// A message the person sent, rather than a model call: it counts toward
    /// messages and activity, never tokens.
    pub(crate) user: bool,
    /// From the request leaving to the last of the reply arriving, where the
    /// client logs both ends.
    pub(crate) duration_ms: Option<u64>,
    pub(crate) outcome: Outcome,
}

/// Whether a request got its answer, as far as its client's logs tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The client logs no failed request (Codex), so its answers prove no
    /// rate; and a reply the person stopped is not the endpoint failing.
    Unlogged,
    Answered,
    /// The request ended in an error, after whatever retries the client made.
    Failed,
    /// One attempt failed and the client tried again: not a call of its own.
    Retried,
}

pub(crate) fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

pub(crate) fn timestamp(value: &Value) -> Option<i64> {
    timestamp_ms(value).map(|at| at.div_euclid(1000))
}

pub(crate) fn timestamp_ms(value: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.get("timestamp")?.as_str()?)
        .ok()
        .map(|at| at.timestamp_millis())
}

/// What a file has given so far, and where to continue.
pub(crate) struct FileState {
    len: u64,
    modified: Option<SystemTime>,
    offset: u64,
    pub(crate) calls: Vec<Call>,
    /// For a client that states its model, conversation and directory once,
    /// then counts: what it stated, and its running total.
    pub(crate) stated_model: Arc<str>,
    pub(crate) stated_conversation: Arc<str>,
    pub(crate) stated_cwd: Arc<str>,
    pub(crate) stated_total: u64,
    /// When each request a client logged on its own line left, in
    /// milliseconds, by the id its reply refers back to.
    pub(crate) request_starts: HashMap<Arc<str>, i64>,
}

pub(crate) struct Source {
    provider: ProviderKind,
    file: PathBuf,
    pub(crate) home_session: Option<Arc<str>>,
}

pub(crate) fn empty() -> Arc<str> {
    Arc::from("")
}

/// Read what the file has gained since last time; start over if it shrank.
fn refresh(source: &Source, state: &mut FileState) {
    let Ok(metadata) = std::fs::metadata(&source.file) else {
        return;
    };
    let modified = metadata.modified().ok();
    if metadata.len() == state.len && modified == state.modified {
        return;
    }
    if metadata.len() < state.offset {
        *state = new_state();
    }
    let Ok(mut file) = File::open(&source.file) else {
        return;
    };
    if file.seek(SeekFrom::Start(state.offset)).is_err() {
        return;
    }
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            break;
        };
        // A line still being written waits for the next read.
        if read == 0 || !line.ends_with('\n') {
            break;
        }
        state.offset += read as u64;
        if let Some(client) = crate::adapters::adapter(source.provider) {
            client.read_transcript_line(&line, source, state);
        }
    }
    state.len = metadata.len();
    state.modified = modified;
}

#[cfg(test)]
pub(crate) fn test_source(provider: ProviderKind) -> Source {
    Source {
        provider,
        file: PathBuf::from("test.jsonl"),
        home_session: None,
    }
}
#[cfg(test)]
pub(crate) fn test_state() -> FileState {
    new_state()
}

fn new_state() -> FileState {
    FileState {
        len: 0,
        modified: None,
        offset: 0,
        calls: Vec::new(),
        stated_model: empty(),
        stated_conversation: empty(),
        stated_cwd: empty(),
        stated_total: 0,
        request_starts: HashMap::new(),
    }
}

pub(crate) fn jsonl_files(root: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth > 0 {
            jsonl_files(&path, out, depth - 1);
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

/// Every transcript under the given client homes. A home is a Claude config
/// directory (projects/) or a Codex home (sessions/, archived_sessions/), or both.
fn sources(homes: &[Home]) -> Vec<Source> {
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for (home, session) in homes {
        let home = dunce::canonicalize(home).unwrap_or_else(|_| home.clone());
        if !seen.insert(home.clone()) {
            continue;
        }
        let transcripts = crate::adapters::all().flat_map(|client| {
            client
                .transcripts()
                .iter()
                .map(move |(subdir, depth)| (client.kind(), *subdir, *depth))
        });
        for (provider, subdir, depth) in transcripts {
            let mut files = Vec::new();
            jsonl_files(&home.join(subdir), &mut files, depth);
            sources.extend(files.into_iter().map(|file| Source {
                provider,
                file,
                home_session: session.clone(),
            }));
        }
    }
    sources
}

/// Read state by client and file: two clients keep transcripts under the same
/// directory name (Codex's and Pi's `sessions/`), so one file is read once per
/// client, each from where that client's reader left off.
fn cache() -> &'static Mutex<HashMap<(ProviderKind, PathBuf), FileState>> {
    static CACHE: OnceLock<Mutex<HashMap<(ProviderKind, PathBuf), FileState>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// All calls from every transcript under the homes, de-duplicated.
pub(super) fn collect(homes: &[Home]) -> Vec<Call> {
    let mut cache = cache().lock().expect("usage cache");
    let mut calls = Vec::new();
    let mut seen = HashSet::new();
    for source in sources(homes) {
        let state = cache
            .entry((source.provider, source.file.clone()))
            .or_insert_with(new_state);
        refresh(&source, state);
        for call in &state.calls {
            if let Some(key) = &call.dedupe
                && !seen.insert(key.clone())
            {
                continue;
            }
            calls.push(call.clone());
        }
    }
    calls
}

/// A client home, and the Tendly session it belongs to when it is one's own.
pub(super) type Home = (PathBuf, Option<Arc<str>>);
